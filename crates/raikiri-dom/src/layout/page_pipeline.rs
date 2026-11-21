use super::box_tree::BoxTree;
use crate::document::ancestors::{AncestorCursor, ParentRelation};
use crate::node::BoxPropertiesRef;
mod candidates;
mod cell_rows;
mod column_first_piece;
pub(crate) mod column_projection;
mod column_replaced;
mod external_float_gap;
pub(super) mod float_resumption;
mod footnotes;
mod fragment_sources;
mod grid_rows;
mod leading_inline;
mod leading_margins;
mod line_ranges;
mod link_events;
mod margin_clearance;
pub(super) mod normal_block_shifts;
mod oversized_edges;
mod page_start;
mod pagination_state;
mod positioned_flow;
mod sibling_adjacency;
mod sibling_shifts;
mod source_walk;
mod table_cell_fragments;
mod transposed_flow;
use candidates::{CandidateCollector, PageCandidate, has_display_none_descendant};
use column_projection::{ParagraphCache, ParagraphProjection, ProjectionWork};
use footnotes::FootnoteAreas;
use fragment_sources::{
    PageFragmentSource, drop_uncrossed_body, number_fragments, overflow_clip_sources,
    place_sources, restore_crossing_tops,
};
pub(crate) use link_events::page_fragment_events_from_pages;
use pagination_state::{FlowEnv, PaginationState};
use source_walk::{WalkedSources, walk_sources};

use super::ifc::boxes::{IfcBox, IfcBoxKind, find_oof_anchor, first_anchors};
use super::ifc::inline_boxes::atomic_nodes;
use super::ifc::text_lines::{IfcTextLine, TextLineIndex, single_unplaced_line};
use super::table::headers::HeaderRepeats;
use super::taffy_adapter::CollapsibleMarginSet;
use super::*;
use crate::node::FormattingContext;
use crate::page_projection::records::OverflowClipSource;
use raikiri_layout_style::values::{BoxSizing, MaybeResolve, Position, ResolveOrZero};
use raikiri_traits::{PaintInsets, PaintRect};
use std::cell::Cell;
use std::collections::BTreeMap;

type ProjectedSlices = (
    Vec<PageFragment>,
    Vec<ProjectedTextRoot>,
    BTreeMap<NodeId, OverflowClipSource>,
    Vec<PlacementOverflowClipSource>,
    ParagraphCache,
    usize,
);

// Flex traversal skips Contents boxes when globally ordering effective
// items; preserve their stored offsets relative to the visited parent.
fn pagination_child_parent_y(document: &Document, parent: usize, child: usize, raw_y: f32) -> f32 {
    let mut child_parent_y = raw_y;
    for id in document.ancestors(child).take_while(|&id| id != parent) {
        child_parent_y += document.pagination_local_y(id);
    }
    child_parent_y
}

#[derive(Clone, Copy, PartialEq)]
struct OverflowClipLimit {
    end: f32,
    positioned_end: f32,
    // A nested positioned box cannot open another hidden start page under
    // the clip that already bounds its positioned ancestor.
    positioned_started: bool,
}

impl OverflowClipLimit {
    const UNBOUNDED: Self = Self {
        end: f32::INFINITY,
        positioned_end: f32::INFINITY,
        positioned_started: false,
    };

    fn for_positioned(self) -> Self {
        Self {
            end: self.positioned_end,
            ..self
        }
    }
}

#[derive(Clone, Copy)]
struct ColumnContainingBlocks {
    absolute_outside: bool,
    fixed_outside: bool,
}

impl ColumnContainingBlocks {
    fn descendants(
        self,
        document: &Document,
        id: usize,
        computed: &raikiri_style::ComputedValues,
    ) -> Self {
        Self {
            absolute_outside: self.absolute_outside
                && !super::fixed::establishes_absolute_containing_block(document, id, computed),
            fixed_outside: self.fixed_outside
                && !super::fixed::establishes_fixed_containing_block_for_layout(
                    document, id, computed,
                ),
        }
    }
}

#[derive(Clone, Copy)]
struct ContentOverflowContext {
    parent_y: f32,
    in_columns: Option<ColumnContainingBlocks>,
    inside_positioned_flow: bool,
    flow_clip: OverflowClipLimit,
    absolute_clip: OverflowClipLimit,
    fixed_clip: OverflowClipLimit,
}

impl ContentOverflowContext {
    fn update_containing_blocks(
        &mut self,
        document: &Document,
        id: usize,
        computed: &raikiri_style::ComputedValues,
    ) {
        let clip = self.flow_clip.for_positioned();
        if clip != self.absolute_clip
            && super::fixed::establishes_absolute_containing_block(document, id, computed)
        {
            self.absolute_clip = clip;
        }
        if clip != self.fixed_clip
            && super::fixed::establishes_fixed_containing_block_for_layout(document, id, computed)
        {
            self.fixed_clip = clip;
        }
    }
}

struct ContentOverflow {
    end: f32,
    hidden_start_y: Option<f32>,
}

/// How boxes outside the paginated flow reach pages: their lowest block-end
/// edge and the latest start of a wholly clipped positioned flow. This
/// includes the body's own border box, nested positioned boxes, and overflow
/// of a monolithic box. Coordinates are measured from the body after
/// pagination moved the flow.
///
/// Page-fixed boxes repeat and running elements leave the flow, so their
/// subtrees do not extend the document. Overflow clips bound normal content,
/// while a positioned box bypasses clips below its containing block.
/// Inline elements are located by their paragraph's lines, which take part
/// in pagination directly, so only the boxes inside them are measured.
///
/// The in-flow content of a multicol container is projected into columns,
/// so its unfragmented layout position says nothing about the page it paints
/// on. Where a column layout cut that content into fragments, the fragments
/// are measured instead: a box that cannot break and is taller than its
/// column overflows it (CSS Fragmentation 3, 4.4). A positioned box in the
/// columns whose containing block is the container or one of its
/// ancestors is not projected, though: it paints at its laid-out position
/// and is measured there.
fn content_overflow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    root_flow_offset: f32,
    page_area_height: f32,
) -> ContentOverflow {
    use raikiri_style::property::OverflowValue;
    // The body's overflow, or the root's, clips at the viewport, which
    // stays within the paginated flow.
    if super::viewport_overflow_source(document, cascade)
        .is_some_and(|id| cascade.computed[id].overflow.y != OverflowValue::Visible)
    {
        return ContentOverflow {
            end: f32::NEG_INFINITY,
            hidden_start_y: None,
        };
    }
    // The records below each column fragment, and the container records
    // placed against their own node.
    let fragments = document.layout_fragments();
    let mut fragment_children = HashMap::<usize, Vec<usize>>::new();
    let mut fragment_roots = HashMap::<usize, Vec<usize>>::new();
    for (index, fragment) in fragments.iter().enumerate() {
        match fragment.parent {
            Some(parent) => fragment_children.entry(parent).or_default().push(index),
            None => fragment_roots
                .entry(fragment.node_id)
                .or_default()
                .push(index),
        }
    }
    let mut end = f32::NEG_INFINITY;
    let mut hidden_start_y: Option<f32> = None;
    // Within columns, track independently whether absolute and fixed boxes
    // still take their containing blocks from outside the column flow.
    let mut stack = vec![(
        body_id,
        ContentOverflowContext {
            parent_y: 0.0,
            in_columns: None,
            inside_positioned_flow: false,
            flow_clip: OverflowClipLimit::UNBOUNDED,
            absolute_clip: OverflowClipLimit::UNBOUNDED,
            fixed_clip: OverflowClipLimit::UNBOUNDED,
        },
    )];
    while let Some((node_id, mut context)) = stack.pop() {
        let parent_y = context.parent_y;
        let in_columns = context.in_columns;
        let node = &document.nodes[node_id];
        if !node.is_in_document()
            || node.is_display_none()
            || node.is_unrendered_html_element()
            || node.kind() != NodeKind::Element
        {
            continue;
        }
        // An inline element of a paragraph is placed by the paragraph's lines,
        // which take part in pagination directly, and the boxes inside it are
        // located from the paragraph's root, so only its children are walked,
        // in the root's coordinate space.
        let computed = &cascade.computed[node_id];
        if node.flags.contains(NodeFlags::IN_IFC_SUBTREE) {
            let in_columns =
                in_columns.map(|outside| outside.descendants(document, node_id, computed));
            // An inline keeps its paragraph's coordinates, but can still be
            // the containing block of a positioned descendant.
            context.update_containing_blocks(document, node_id, computed);
            context.in_columns = in_columns;
            stack.extend(node.children.iter().map(|&child| (child, context)));
            continue;
        }
        if super::fixed::is_page_fixed(document, cascade, node_id)
            || matches!(
                document.nodes[node_id].position(),
                PositionValue::Running(_)
            )
            || computed.float == FloatValue::Footnote
        {
            // Footnote areas are page-local, so their retained paragraph
            // overflow must not extend the document's normal-flow pages.
            continue;
        }
        // In columns only positioned boxes whose containing blocks are
        // outside are measured. Still visit a column-flow box's descendants:
        // a fixed descendant may use a containing block outside the columns.
        // A `display: contents` element generates no positioned box, so its
        // children stay in the column flow.
        let positioned = matches!(
            document.nodes[node_id].position(),
            PositionValue::Absolute | PositionValue::Fixed
        ) && document.nodes[node_id].display() != DisplayValue::Contents;
        let in_columns = match in_columns {
            Some(outside) if positioned => {
                let escapes = if document.nodes[node_id].position() == &PositionValue::Fixed {
                    outside.fixed_outside
                } else {
                    outside.absolute_outside
                };
                if escapes { None } else { Some(outside) }
            }
            other => other,
        };
        context.inside_positioned_flow |= positioned;
        // Layout places some boxes without their relative offset, which
        // the painter adds to the box and its subtree.
        let paint_offset = painted_relative_offset_y(document, cascade, body_id, node_id);
        // OOF locations are relative to their containing-block owner, which
        // may differ from the DOM parent used by this traversal.
        let y = if document.oof_coordinates.contains_key(&node_id) {
            current_abs_y(document, node_id) + paint_offset
        } else {
            parent_y + node.layout.location.y + paint_offset
        };
        let mut clip = match document.nodes[node_id].position() {
            PositionValue::Absolute if positioned => context.absolute_clip,
            PositionValue::Fixed if positioned => context.fixed_clip,
            _ => context.flow_clip,
        };
        if positioned && in_columns.is_none() && clip.end.is_finite() {
            // Keep a hidden start as a point, including for a zero-height
            // box. Artificial extent could cross a nearby page boundary.
            // Keep the actual clip for its descendants;
            // a fixed descendant may still escape through another CB stream.
            if y.is_finite() && y >= clip.end && !clip.positioned_started {
                hidden_start_y = Some(hidden_start_y.map_or(y, |start| start.max(y)));
            }
            clip.positioned_started = true;
        }
        let bottom = if node_id == body_id {
            block_border_box_end(
                document,
                cascade,
                body_id,
                root_flow_offset,
                page_area_height,
                false,
                None,
            )
        } else if in_columns.is_none() && node.layout.size.height > 0.0 {
            y + node.layout.size.height
        } else {
            f32::NEG_INFINITY
        };
        if bottom.is_finite() && y < clip.end {
            end = end.max(bottom.min(clip.end));
        }
        // A `display: contents` element generates no box and clips nothing,
        // as in painting.
        let clips = node_id != body_id && element_clips_overflow(document, cascade, node_id);
        if clips {
            let paint_end = clip_block_end(document, cascade, node_id, y);
            let positioned_end = paint_end.min(y + node.layout.size.height);
            clip.positioned_end = clip.positioned_end.min(positioned_end);
            // A clip's outset extends ordinary fragmented flow, but paint
            // overflow alone does not add pages to a positioned flow.
            clip.end = clip.end.min(if context.inside_positioned_flow {
                positioned_end
            } else {
                paint_end
            });
        }
        // Generated block content and overflowing lines have no source box
        // to visit. Measure retained paragraph flow even when an authored
        // height or max-height limits its ordinary containing box. The lines
        // of a vertical paragraph stack across a flow paginated along y, so
        // their block extent says nothing about how far down it reaches.
        if in_columns.is_none()
            && y < clip.end
            && node.multicol.is_none()
            && (document.context.pagination_on_transposed_axes
                || !node
                    .ifc_writing_mode()
                    .is_some_and(|mode| mode.is_vertical()))
            && let Some(lines) = node.ifc.as_ref().and_then(|root| root.lines.as_ref())
        {
            end = end.max(
                (y + node.layout.border.top + node.layout.padding.top + lines.shifted_height())
                    .min(clip.end),
            );
        }
        if in_columns.is_none() && y < clip.end {
            let roots = fragment_roots.get(&node_id).map_or(&[][..], Vec::as_slice);
            end = end.max(
                fragment_tree_end(
                    fragments,
                    &fragment_children,
                    roots,
                    y,
                    |node_id| element_clips_overflow(document, cascade, node_id),
                    |node_id| painted_relative_offset_y(document, cascade, body_id, node_id),
                )
                .min(clip.end),
            );
        }
        let in_columns = if let Some(outside) = in_columns {
            Some(outside.descendants(document, node_id, computed))
        } else if node.multicol.is_some() {
            Some(ColumnContainingBlocks {
                absolute_outside: true,
                fixed_outside: true,
            })
        } else {
            None
        };
        context.parent_y = y;
        context.in_columns = in_columns;
        context.flow_clip = clip;
        context.update_containing_blocks(document, node_id, computed);
        stack.extend(node.children.iter().map(|&child| (child, context)));
    }
    ContentOverflow {
        end,
        hidden_start_y,
    }
}

/// The overflow of `node_id` on the axes pagination runs along: the physical
/// x axis then is the block axis of a vertical document, handed to the
/// paginator on transposed axes (see [`transposed_flow`]).
fn pagination_overflow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> raikiri_style::property::OverflowXY {
    let overflow = cascade.computed[node_id].overflow;
    if document.context.pagination_on_transposed_axes {
        raikiri_style::property::OverflowXY::new(overflow.y, overflow.x)
    } else {
        overflow
    }
}

/// Whether the element hides block-axis overflow when painted. A cell that
/// spans a collapsed table row or column clips on both axes.
fn element_clips_overflow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    crate::paint_rules::clips_collapsed_table_span(document, node_id)
        || !matches!(
            pagination_overflow(document, cascade, node_id).y,
            OverflowValue::Visible
        ) && crate::paint_rules::clips_element_overflow(document, cascade, node_id)
}

/// The block offset at which the overflow clip of `node_id` hides what its
/// content paints, for a border box that starts at `y`: the border box's own
/// edge, or the farther one that `overflow-clip-margin` names on an
/// `overflow: clip` axis. A clip edge inside the border box (a padding box
/// clip) still counts as the border box's edge.
fn clip_block_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    y: f32,
) -> f32 {
    let layout = &document.nodes[node_id].layout;
    let (border, padding) = (layout.border, layout.padding);
    let geometry = crate::paint_rules::OverflowClipGeometry::with_overflow(
        &cascade.computed[node_id],
        pagination_overflow(document, cascade, node_id),
        PaintRect::new(0.0, y, layout.size.width, layout.size.height),
        PaintInsets::new(border.top, border.right, border.bottom, border.left),
        PaintInsets::new(padding.top, padding.right, padding.bottom, padding.left),
        crate::paint_rules::is_replaced_element(document, node_id),
    );
    let geometry = if crate::paint_rules::clips_collapsed_table_span(document, node_id) {
        geometry.within_border_box()
    } else {
        geometry
    };
    y + layout.size.height + geometry.block_end_outset()
}

/// Where the clips of `owner` and of the boxes around it that keep their
/// height hide the content inside them now, in the coordinates of placed
/// candidates: the nearest of their block-end clip edges at the places
/// pagination has moved them to. A box whose height follows its content
/// grows with what is pushed down inside it, so its clip ends nothing here.
/// The clips above an absolutely positioned or fixed box do not cut it.
fn moved_clip_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    owner: usize,
    root_flow_offset: f32,
) -> f32 {
    let mut end = f32::INFINITY;
    let mut next = Some(owner);
    while let Some(id) = next.filter(|&id| id != body_id) {
        if element_clips_overflow(document, cascade, id) && !height_follows_content(document, id) {
            let top = current_abs_y(document, id) + root_flow_offset;
            end = end.min(clip_block_end(document, cascade, id, top));
        }
        // The walk stops after an absolutely positioned or fixed box.
        let positioned = matches!(
            document.nodes[id].position(),
            PositionValue::Absolute | PositionValue::Fixed
        );
        next = document.parent_of(id).filter(|_| !positioned);
    }
    end
}

/// Whether a forced break at the start (`before`) or end of `node_id`, at
/// `at` in candidate coordinates, falls inside a box that clips its
/// overflow, has a height its content cannot grow (`height`, `max-height` or
/// size containment; a `min-height` alone lets it grow) and whose border box
/// ends on the page of the break. Both are judged where the normal flow puts
/// them, without relative offsets. Chromium prints the rest of such a box
/// in one piece: the forced breaks in it break nothing. A box that runs past
/// the end of the page breaks there as any other; a clip margin past its
/// border box does not count. A break at the first or last box of a box is a
/// break before or after that box, so it is not inside it. The clips above
/// an absolutely positioned or fixed box do not hold it.
fn clip_holds_breaks(
    document: &super::context::LayoutSession<'_>,
    env: &FlowEnv<'_>,
    node_id: usize,
    before: bool,
    at: f32,
) -> bool {
    let FlowEnv {
        cascade,
        body_id,
        page_origins,
        root_flow_offset,
        ..
    } = *env;
    let positioned = |id: usize| {
        matches!(
            document.nodes[id].position(),
            PositionValue::Absolute | PositionValue::Fixed
        )
    };
    let mut edge = node_id;
    while let Some(parent) = document.parent_of(edge).filter(|&id| id != body_id)
        && !renders_edge_pseudo(cascade, parent, before)
        && ordered_edge_child(document, cascade, parent, before)
            .is_some_and(|child| is_descendant_or_self(document, child, edge))
    {
        edge = parent;
    }
    let at = at - accumulated_relative_offset_y(document, cascade, body_id, node_id);
    let page = if before {
        page_origins.page_index_for_y(at)
    } else {
        page_origins.page_index_for_end(at)
    };
    let page_end = page_origins.origin(page) + page_origins.step_at(page);
    let mut next = document.parent_of(edge).filter(|_| !positioned(edge));
    while let Some(id) = next.filter(|&id| id != body_id) {
        if element_clips_overflow(document, cascade, id)
            && height_is_capped(document, id)
            && current_abs_y(document, id) + root_flow_offset
                - accumulated_relative_offset_y(document, cascade, body_id, id)
                + document.nodes[id].layout.size.height
                <= page_end + 0.001
        {
            return true;
        }
        next = document.parent_of(id).filter(|_| !positioned(id));
    }
    false
}

/// The first (`before`) or last box of `parent` in the order pagination
/// visits its children: a flex or grid container visits its items in
/// `order`-modified order, a block container in source order (see
/// [`page_edge_child`]).
fn ordered_edge_child(
    document: &Document,
    cascade: &CascadeResult,
    parent: usize,
    before: bool,
) -> Option<usize> {
    if !matches!(
        document.nodes[parent].formatting_context,
        FormattingContext::Flex | FormattingContext::Grid
    ) {
        return page_edge_child(document, cascade, parent, before, 0)
            .ok()
            .flatten();
    }
    let order = super::page::pagination_child_order_borrowed(document, cascade, parent);
    let is_item = |&child: &usize| {
        let node = &document.nodes[child];
        node.is_in_document()
            && !node.is_unrendered_html_element()
            && node.display() != DisplayValue::None
            && !matches!(
                node.position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
            && match &node.data {
                crate::node::NodeData::Text(text) => !text.text_content.trim().is_empty(),
                _ => true,
            }
    };
    if before {
        order.iter().copied().find(is_item)
    } else {
        order.iter().rev().copied().find(is_item)
    }
}

/// The lowest edge painted by the fragments under `roots`, placed at the
/// block offset `y`.
///
/// A fragment paints at its parent's origin plus its own offset, and a
/// fragmentainer clip, intersected with the clips of the enclosing
/// fragments, hides what extends past its column. A fragment whose element
/// `clips_overflow` also hides what its descendants paint past its own box.
/// The relative offset that layout left to the painter, `painted_offset` of
/// the element, moves the fragment with its clip and everything inside it; the
/// roots are placed at `y`, which has the offset of their elements already.
fn fragment_tree_end(
    fragments: &[crate::fragment::LayoutFragment],
    fragment_children: &HashMap<usize, Vec<usize>>,
    roots: &[usize],
    y: f32,
    clips_overflow: impl Fn(usize) -> bool,
    painted_offset: impl Fn(usize) -> f32,
) -> f32 {
    let mut end = f32::NEG_INFINITY;
    let mut pending: Vec<(usize, f32, f32)> = roots
        .iter()
        .map(|&index| (index, y, f32::INFINITY))
        .collect();
    while let Some((index, parent_origin, inherited_clip_end)) = pending.pop() {
        let rect = fragments[index].rect;
        let parent_origin = if fragments[index].parent.is_some() {
            parent_origin + painted_offset(fragments[index].node_id)
        } else {
            parent_origin
        };
        let origin = parent_origin + rect.y;
        let clip_end = fragments[index]
            .fragmentainer_clip
            .map_or(inherited_clip_end, |clip| {
                if clip.clip_y {
                    inherited_clip_end.min(parent_origin + clip.rect.y + clip.rect.height)
                } else {
                    inherited_clip_end
                }
            });
        let bottom = (origin + rect.height).min(clip_end);
        if rect.height > 0.0 && bottom.is_finite() {
            end = end.max(bottom);
        }
        let clip_end = if clips_overflow(fragments[index].node_id) {
            bottom
        } else {
            clip_end
        };
        pending.extend(
            fragment_children
                .get(&index)
                .into_iter()
                .flatten()
                .map(|&child| (child, origin, clip_end)),
        );
    }
    end
}

/// The block offset that layout applied to a relatively positioned box.
/// It moves the box but not the flow around it (CSS 2.1
/// §9.4.3), and fragmentation happens before it (CSS Fragmentation 3 §5.5).
fn layout_relative_offset_y(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> f32 {
    layout_relative_offset(document, cascade, body_id, node_id).1
}

/// The inline and block offsets that layout applied to a relatively
/// positioned box, as [`layout_relative_offset_y`] takes the block one.
fn layout_relative_offset(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> (f32, f32) {
    // A `display: contents` element generates no box to offset.
    // Layout places a sticky box without its offset.
    if !matches!(document.nodes[node_id].position(), PositionValue::Relative)
        || matches!(
            document.nodes[node_id].display(),
            DisplayValue::Contents | DisplayValue::None
        )
        || relative_offset_is_painted(document, cascade, Some(body_id), node_id)
    {
        return (0.0, 0.0);
    }
    // Percentages resolve against the containing block's height when it
    // is definite, and otherwise act as `auto` (CSS 2.1 §9.3.2). Layout
    // takes `left` over `-right` and `top` over `-bottom` in every writing
    // mode.
    let insets = super::relative::layout_relative_insets(
        document,
        cascade,
        node_id,
        super::relative::Precedence::LeftTop,
    );
    (insets.0.unwrap_or(0.0), insets.1.unwrap_or(0.0))
}

/// The relative offsets that layout applied to `node_id` and to its
/// ancestors through the document element. They all moved the box away from its
/// normal-flow position.
fn accumulated_relative_offset_y(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> f32 {
    document
        .inclusive_ancestors(node_id)
        .scan(false, |finished, id| {
            if *finished {
                return None;
            }
            *finished = id == body_id;
            Some(id)
        })
        .map(|id| layout_relative_offset_y(document, cascade, body_id, id))
        .sum()
}

/// The unpaginated top of `candidate` where the flow puts it, without the
/// relative offset of its own box. Sibling margins are measured between
/// such tops and bottoms, as an offset only paints a box elsewhere; the
/// offsets of the ancestors move the siblings alike.
fn flow_raw_y(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    candidate: &PageCandidate,
) -> f32 {
    candidate.raw_y - layout_relative_offset_y(document, cascade, body_id, candidate.node_id)
}

/// The block offset of a relatively positioned box that layout leaves to
/// the painter: that of the body, of floats and of table parts, and of boxes
/// placed by manual column placement. The painter takes `top` over
/// `-bottom`, resolved as [`super::relative::used_relative_offset`] does.
fn painted_relative_offset_y(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> f32 {
    if !relative_offset_is_painted(document, cascade, Some(body_id), node_id) {
        return 0.0;
    }
    super::relative::used_relative_offset_for_layout(document, cascade, node_id).1
}

/// How far down the painter moves `node_id` by its own relative or sticky
/// offset, when layout places the box without it (see
/// [`relative_offset_is_painted`]): a float, a table part, a sticky box, a box
/// the columns of a multicol container place. Chromium fragments such a box
/// where the offset puts it, as it does a relatively positioned box in normal
/// flow, which layout already places with its offset.
pub(crate) fn painter_relative_shift_y(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> f32 {
    let node = &document.nodes[node_id];
    // A `display: contents` element has no box for the painter to move.
    if node.kind() != NodeKind::Element
        || matches!(node.display(), DisplayValue::Contents | DisplayValue::None)
        || !relative_offset_is_painted(document, cascade, None, node_id)
    {
        return 0.0;
    }
    super::relative::used_relative_offset_for_layout(document, cascade, node_id).1
}

/// Whether layout leaves the relative offset of `node_id` to the painter.
fn relative_offset_is_painted(
    document: &Document,
    cascade: &CascadeResult,
    body_id: Option<usize>,
    node_id: usize,
) -> bool {
    if !matches!(
        document.nodes[node_id].position(),
        PositionValue::Relative | PositionValue::Sticky
    ) {
        return false;
    }
    super::relative::placed_without_relative_offset_for_layout(document, cascade, body_id, node_id)
        || document.nodes[node_id].needs_relative_block_paint_offset()
}

/// The block-end edge of the root's border box in page coordinates.
///
/// Pagination moves descendants and paragraph lines after the native layout
/// pass. Measure their committed used extents for an auto-sized root, retaining
/// the root's authored height, min/max bounds, border and padding. Its position
/// comes from the laid-out box, including non-collapsing root margins.
///
/// When counting overflow pages, an auto-sized root without block-end edges
/// contributes no extra page beyond those its content already reached.
/// `whole_box` also includes that content when extending the root's own box.
/// `flow_end_cap`, below the top of the root's border box, is where its
/// in-flow content ends when a page edge cut the margin it ends with.
fn block_border_box_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    flow_root: usize,
    root_flow_offset: f32,
    page_area_height: f32,
    whole_box: bool,
    flow_end_cap: Option<f32>,
) -> f32 {
    let computed = &cascade.computed[flow_root];
    // Percentages resolve against the root's used height only when it is
    // definite; otherwise they behave as `auto`. The root's own percentages
    // resolve against the initial containing block, the page area.
    let parent = document.taffy_parent_of(flow_root);
    let root_height = if parent == document.nodes.initial_containing_block_key() {
        Some(page_area_height)
    } else {
        parent
            .and_then(|parent| document.nodes.layout_node(parent))
            .and_then(|parent| {
                if document.context.pagination_on_transposed_axes {
                    // Physical width percentages use the containing block's
                    // content width, now the transposed layout's height.
                    Some(parent.layout.content_box_height())
                } else {
                    parent.percent_basis_height
                }
            })
    };
    let used = |value: &ComputedLengthPercentageOrAuto| {
        match value {
            ComputedLengthPercentageOrAuto::Px(value) => Some(*value),
            ComputedLengthPercentageOrAuto::Percent(_)
            | ComputedLengthPercentageOrAuto::Math(_)
            | ComputedLengthPercentageOrAuto::Calc(_) => {
                root_height.and_then(|basis| used_computed_length_percentage_or_auto(value, basis))
            }
            _ => None,
        }
        .map(|value| value.max(0.0))
    };
    let root = &document.nodes[flow_root];
    let layout = &root.layout;
    let edges_top = layout.padding.top + layout.border.top;
    let edges_bottom = layout.padding.bottom + layout.border.bottom;
    let top = root_flow_offset + current_abs_y(document, flow_root);
    let border_box_end = |content: f32| {
        top + if matches!(
            computed.box_sizing,
            raikiri_style::property::BoxSizing::BorderBox
        ) {
            content.max(edges_top + edges_bottom)
        } else {
            content + edges_top + edges_bottom
        }
    };
    let min_height = used(&computed.min_height).unwrap_or(0.0);
    // Text of the root's own paragraph is located by its lines, which are
    // not measured here, but a block-level `::before` or `::after` between
    // or after them takes room of its own: the paragraph then ends where
    // its content does. Like the root's in-flow children, that content
    // counts past the root's `height` and `max-height`: the root's overflow
    // is visible. That overflow is not part of the whole box.
    let generated_end = root
        .ifc
        .as_ref()
        .filter(|root| !whole_box && !root.generated_blocks.is_empty())
        .and_then(|root| root.lines.as_ref())
        .map(|lines| top + edges_top + lines.shifted_height() + edges_bottom)
        .unwrap_or(f32::NEG_INFINITY);
    // A box that lays out vertical lines on its own axes keeps its physical
    // height in its vertical size mapping; its `height` is the horizontal
    // mapping, the block size. A vertical box in a horizontal flow is read
    // by the former. A transposed cascade has no such mapping left.
    let physical_height = super::bridge::vertical_logical_size(cascade, flow_root)
        .map_or(&computed.height, |size| &size.height);
    // A definite height, even zero, fixes the border box; content
    // overflowing it is measured on its own.
    if let Some(height) = used(physical_height) {
        let height = used(&computed.max_height).map_or(height, |max| height.min(max));
        return border_box_end(height.max(min_height)).max(generated_end);
    }
    let min_end = if min_height > 0.0 {
        border_box_end(min_height)
    } else {
        f32::NEG_INFINITY
    };
    let max_end = used(&computed.max_height).map_or(f32::INFINITY, border_box_end);
    // Without block-end edges or a `min-height` an auto-height body ends
    // with its content, which the flow already paginated.
    let edges_end = edges_bottom > 0.0 || min_height > 0.0;
    if root.is_ifc_root() {
        let lines_end = root
            .ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
            .filter(|_| whole_box || edges_end)
            .map_or(f32::NEG_INFINITY, |lines| {
                (top + edges_top + lines.shifted_height() + edges_bottom).min(max_end)
            });
        return min_end.max(generated_end).max(lines_end);
    }
    if !edges_end && !whole_box {
        return min_end;
    }
    // In-flow children are in page coordinates already, with the margins
    // between them collapsed, so the content ends at the block-end margin
    // edge of the last one; block-end edges or a `min-height` keep that
    // margin, negative or not, inside the root. A body that establishes a
    // formatting context of its own also encloses its floats, including
    // those inside its in-flow blocks that are not formatting contexts
    // themselves.
    // The children of a `display: contents` element take part as the root's
    // own, and a relative offset moves a box without moving the flow.
    let encloses_floats = holds_floats(document, cascade, flow_root);
    let mut content_end = top + edges_top;
    // The border-box end of the last child with a border box of its own,
    // and that child.
    let mut last_flow_end = None;
    // Where the margin box of the last child ends in the committed flow.
    // Pagination may have cut a margin that crossed a page edge, which the
    // margins themselves do not show.
    let mut last_margin_end = f32::INFINITY;
    // The margins of the empty children after the last child with a border
    // box of its own, which collapse through them (CSS 2 §8.3.1). The block-end
    // margins of that child join them where they adjoin the end of the flow.
    let mut trailing = CollapsibleMarginSet::ZERO;
    let block_children = root.formatting_context == FormattingContext::Block;
    // Popped in document order, each with the flow offset of its parent's
    // coordinates and whether it is nested in one of the root's blocks,
    // where only floats count. Boxes inside an inline formatting context
    // are placed in the coordinates of its root, so its inlines add no
    // offset.
    let mut children: Vec<(usize, f32, bool)> = root
        .children
        .iter()
        .rev()
        .map(|&child| (child, top, false))
        .collect();
    while let Some((child, offset, nested)) = children.pop() {
        let node = &document.nodes[child];
        let child_computed = &cascade.computed[child];
        if !node.is_in_document() || node.is_display_none() || node.kind() != NodeKind::Element {
            continue;
        }
        let layout = &node.layout;
        let in_ifc = document
            .parent_of(child)
            .is_some_and(|parent| in_inline_context(document, parent));
        if document.nodes[child].display() == DisplayValue::Contents {
            let offset = if in_ifc {
                offset
            } else {
                offset + layout.location.y
            };
            children.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|&grandchild| (grandchild, offset, nested)),
            );
            continue;
        }
        if !matches!(
            document.nodes[child].position(),
            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
        ) {
            continue;
        }
        if super::page::is_floating_box_for_pagination(document, cascade, child) {
            // The painter adds a float's relative offset; its layout box is
            // where the float sits in the flow. A footnote leaves the flow
            // for its page's footnote area.
            let end = offset + layout.location.y + layout.size.height + layout.margin.bottom;
            let footnote = child_computed.float == FloatValue::Footnote;
            if encloses_floats && !footnote && end.is_finite() {
                content_end = content_end.max(end);
            }
            continue;
        }
        let relative_shift = layout_relative_offset_y(document, cascade, flow_root, child);
        if encloses_floats && !holds_floats(document, cascade, child) {
            let offset = if in_ifc {
                offset
            } else {
                // The marker moves the box's paint and flow advance, while
                // escaped float exclusions retain their normal coordinates.
                offset + layout.location.y
                    - relative_shift
                    - super::ifc::outside_marker::box_leading(document, child)
            };
            children.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|&grandchild| (grandchild, offset, true)),
            );
        }
        if nested {
            continue;
        }
        let mut border_end = offset + layout.location.y - relative_shift + layout.size.height;
        // The parts of an anonymous table are children of the root, which
        // encloses the table's own box too: the table ends below its last
        // part by the half of the collapsed border that hangs under it.
        if let Some(table) = document
            .anonymous_table_of(child)
            .filter(|&table| document.table_parts_boundary(table) == flow_root)
        {
            let table = &document.nodes[table].layout;
            border_end = border_end.max(offset + table.location.y + table.size.height);
        }
        // A page edge may have cut the margin of a box the flow follows, down
        // to where the flow inside the root ends (see `flow_end_cap`); no box
        // ends above its border box.
        let end = flow_end_cap.map_or(border_end + layout.margin.bottom, |cap| {
            (border_end + layout.margin.bottom).min((top + cap).max(border_end))
        });
        if end.is_finite() {
            // A reverse column flex container's source order need not end
            // with its lowest item. Its item margins never collapse.
            if root.formatting_context == FormattingContext::Flex {
                content_end = content_end.max(end);
            }
            last_margin_end = end;
            if block_children
                && child_computed.clear == ClearValue::None
                && collapses_through(document, cascade, child, layout.size.height)
            {
                let (positive, negative) = empty_box_margin_extremes(document, cascade, child);
                trailing = trailing
                    .collapse_with_margin(positive)
                    .collapse_with_margin(negative);
            } else {
                last_flow_end = Some((border_end, child));
                trailing = CollapsibleMarginSet::ZERO;
            }
        }
    }
    // Without block-end edges the last child's block-end margin collapses
    // through the end of the root, unless the root is a formatting context of
    // its own; a `min-height` leaves it out as well. Margins that collapsed
    // through every child joined the root's block-start margin when nothing
    // separates them, so they lie outside the root too.
    let encloses_margins = edges_bottom > 0.0 || encloses_floats;
    if let Some((end, last)) = last_flow_end {
        let end = if encloses_margins {
            // The block-end margins of the last child's own last boxes that
            // adjoin its end collapse with its own one.
            // Margins never collapse between flex or grid items.
            let (positive, negative) = if block_children {
                adjoining_margin_extremes(document, cascade, last, false, None)
            } else {
                let margin = document.nodes[last].layout.margin.bottom;
                (margin.max(0.0), margin.min(0.0))
            };
            let trailing = trailing
                .collapse_with_margin(positive)
                .collapse_with_margin(negative);
            (end + trailing.resolve()).min(last_margin_end)
        } else {
            end
        };
        content_end = content_end.max(end);
    } else if encloses_margins && (edges_top > 0.0 || encloses_floats) {
        content_end = content_end.max((top + edges_top + trailing.resolve()).min(last_margin_end));
    }
    // `max-height` caps the auto height, measured as `box-sizing` says;
    // `min-height` still wins over it.
    let content_height = content_end - (top + edges_top);
    let auto_end = top + edges_top + content_height + edges_bottom;
    min_end.max(auto_end.min(max_end))
}

/// Whether pagination may extend the auto height of `node_id` around its
/// committed content.
fn grows_with_paginated_content(
    document: &super::context::LayoutSession<'_>,
    node_id: usize,
) -> bool {
    let node = &document.nodes[node_id];
    let block_axis = document.block_axis(node_id);
    let ordinary_block = node.formatting_context == FormattingContext::Block
        && matches!(
            node.display(),
            DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
        );
    let flex = node.formatting_context == FormattingContext::Flex;
    // A row-flex item can have a used stretch height even when its
    // auto-sized flex line must still grow around paginated content.
    let assigned_height = node.percent_basis_height.is_some()
        && (block_axis.aspect_ratio().is_some()
            || node.layout_position() == Position::Absolute
            || document.taffy_parent_of(node_id).is_some_and(|parent| {
                let parent_node = &document.nodes[parent];
                match parent_node.formatting_context {
                    FormattingContext::Flex => parent_node.percent_basis_height.is_some(),
                    FormattingContext::Grid => {
                        // A percentage height resolves against the grid area
                        // once the tracks are settled, so the grid has
                        // assigned it even where the row sizes to the item.
                        parent_node.percent_basis_height.is_some()
                            || !grid_rows_size_to_content(document, parent)
                            || matches!(
                                block_axis.height().expand(),
                                ExpandedDimension::Percent(_) | ExpandedDimension::Calc(_)
                            )
                    }
                    _ => false,
                }
            }));
    node.kind() == NodeKind::Element
        && !node.is_display_none()
        && (ordinary_block || flex)
        && !assigned_height
        && node.multicol.is_none()
        && !node.size_containment.height
        && (block_axis.height().is_auto() || definite_content_height(document, node_id).is_none())
}

/// Whether `node_id` is a row or wrapping column flex container whose inline
/// axis is the axis the pages of `root` break along: its height there is its
/// inline size. A wrapping column's inline size takes its items' intrinsic
/// contributions rather than the lines they make, so an orthogonal item can be
/// larger than its line and overflow it (CSS Flexbox 1 §9.9.2). A row's inline
/// size is its main size, which items that cannot shrink below their flex base
/// size overflow, and a wrapping row's `min-content` size is that of its
/// largest item alone (CSS Flexbox 1 §9.9.1).
fn orthogonal_flex_container(
    document: &super::context::LayoutSession<'_>,
    root: usize,
    node_id: usize,
) -> bool {
    let style = document.pagination_style(node_id);
    document.nodes[node_id].formatting_context == FormattingContext::Flex
        && (style.flex_direction_is_row() || style.flex_wraps())
        && document.nodes[node_id].used_writing_mode.is_vertical()
            != document.nodes[root].used_writing_mode.is_vertical()
}

/// Whether `node_id` is a flex container with an item whose top or bottom
/// margin is negative, which can end the container above the bottom of an
/// item.
fn has_negative_item_margin(document: &super::context::LayoutSession<'_>, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    node.formatting_context == FormattingContext::Flex
        && node.children.iter().any(|&child| {
            let margin = document.nodes[child].layout.margin;
            margin.top < 0.0 || margin.bottom < 0.0
        })
}

/// How far the content of each flex item or flex container that
/// [`update_paginated_block_heights`] may extend already reaches past the
/// box's block-end edge before pagination moves anything, by node id.
///
/// A flex item's used height comes from the flex algorithm, which may clamp
/// or shrink it below its content; that content overflows the item rather
/// than extending it. So does the content of a flex container in two cases.
/// Its height comes from its items' outer sizes, so a negative margin can end
/// it above the bottom of an item, which then overflows it. The height of a
/// row or wrapping column whose inline axis runs along the pages is its inline
/// size, which its items can overflow (see [`orthogonal_flex_container`]).
/// Only the growth pagination adds is enclosed. Other boxes are left
/// out: their auto height is the one the extension measures.
fn unpaginated_content_overflow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    root: usize,
    page_area_height: f32,
) -> HashMap<usize, f32> {
    document
        .preorder(root)
        .filter(|&node_id| {
            (orthogonal_flex_container(document, root, node_id)
                || has_negative_item_margin(document, node_id)
                || document.taffy_parent_of(node_id).is_some_and(|parent| {
                    document.nodes[parent].formatting_context == FormattingContext::Flex
                }))
                && grows_with_paginated_content(document, node_id)
        })
        .filter_map(|node_id| {
            let end = block_border_box_end(
                document,
                cascade,
                node_id,
                0.0,
                page_area_height,
                true,
                None,
            );
            let overflow =
                end - current_abs_y(document, node_id) - document.nodes[node_id].layout.size.height;
            (overflow.is_finite() && overflow > 0.0).then_some((node_id, overflow))
        })
        .collect()
}

/// Scale the used width of each box with a percentage `width` by the ratio
/// of the inline size of the page it starts on to the first page's.
///
/// Source layout resolves the percentage against the first page's inline
/// size (`content_width` when the first page has none).
fn scale_percent_widths(
    document: &mut Document,
    cascade: &CascadeResult,
    page_widths: &[f32],
    content_width: f32,
    page_origins: &PageOrigins,
    control: &PageLayoutControl<'_>,
) -> Result<(), LayoutError> {
    if page_widths.is_empty() {
        return Ok(());
    }
    let base_width = page_widths
        .first()
        .copied()
        .filter(|width| width.is_finite() && *width > 0.0)
        .unwrap_or(content_width);
    if base_width.is_finite() && base_width > 0.0 {
        for node_id in 0..document.nodes.len() {
            control.check_aborted()?;
            if !matches!(
                cascade.computed[node_id].width,
                ComputedLengthPercentageOrAuto::Percent(_)
            ) {
                continue;
            }
            let page_index = page_origins.page_index_for_y(current_abs_y(document, node_id));
            let Some(target_width) = page_widths
                .get(page_index as usize)
                .copied()
                .filter(|width| width.is_finite() && *width > 0.0)
            else {
                continue;
            };
            let scale = target_width / base_width;
            if scale.is_finite() && (scale - 1.0).abs() > 0.0001 {
                document.nodes[node_id].layout.size.width *= scale;
            }
        }
    }
    Ok(())
}

/// Let the table boxes around the rows that pagination moved follow them,
/// given where each row sat in its parent before pagination.
///
/// A row moves within its parent with the page it opens, and with the rows
/// before it in the same table that opened one.
fn follow_moved_table_rows(
    document: &mut Document,
    row_origins: Vec<(usize, f32)>,
    table_headers: &HeaderRepeats,
) {
    let moved: Vec<(usize, f32)> = row_origins
        .into_iter()
        .filter_map(|(part, origin)| {
            let delta = document.pagination_local_y(part) - origin;
            (delta > 0.001).then_some((part, delta))
        })
        .collect();
    let (moved_captions, moved_table_rows): (HashMap<usize, f32>, HashMap<usize, f32>) = moved
        .into_iter()
        .partition(|&(part, _)| document.nodes[part].display() == DisplayValue::TableCaption);
    super::table::row_moves::mark_split_lines(document, &moved_table_rows);
    super::table::row_moves::follow_moved_rows(
        document,
        &moved_table_rows,
        &moved_captions,
        |table| {
            table_headers
                .headers
                .values()
                .any(|header| header.table == table && !header.footer)
        },
    );
}

/// Extend auto-height block and flex boxes around their committed paginated
/// content, from descendants to ancestors. Page shifts may have been inherited
/// from a parallel flow rather than introduced at the box's own break.
/// Content that `overflow` says already overflowed a box before pagination
/// keeps overflowing it by the same amount.
fn update_paginated_block_heights(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    root: usize,
    page_area_height: f32,
    overflow: &HashMap<usize, f32>,
    flow_ends: &HashMap<usize, f32>,
) {
    let order: Vec<_> = document.preorder(root).collect();
    for node_id in order.into_iter().rev() {
        if !grows_with_paginated_content(document, node_id) {
            continue;
        }
        let end = block_border_box_end(
            document,
            cascade,
            node_id,
            0.0,
            page_area_height,
            true,
            flow_ends.get(&node_id).copied(),
        );
        let height =
            end - current_abs_y(document, node_id) - overflow.get(&node_id).copied().unwrap_or(0.0);
        if height.is_finite() && height > document.nodes[node_id].layout.size.height {
            document.nodes[node_id].layout.size.height = height;
            document.mark_pagination_extent_dirty(node_id);
        }
    }
}

/// The column flex containers that already hold content on a page.
///
/// Row flex suppresses local named-flow ownership, but its content still
/// occupies the enclosing physical page. The nearest column context of each
/// visited source node is cached, so each source edge is resolved once per
/// layout pass.
struct ColumnContexts {
    /// The page each column context was last occupied on.
    occupied: HashMap<usize, u32>,
    cache: Vec<Option<Option<usize>>>,
}

impl ColumnContexts {
    fn new(node_count: usize) -> Self {
        Self {
            occupied: HashMap::new(),
            cache: vec![None; node_count],
        }
    }

    fn enclosing(
        &mut self,
        node_id: usize,
        document: &Document,
        cascade: &CascadeResult,
    ) -> Option<usize> {
        let mut walk = AncestorCursor::new(Some(node_id), ParentRelation::Source);
        let mut visited = Vec::new();
        let mut context = None;
        while let Some(current) = walk.current() {
            if let Some(cached) = self.cache[current] {
                context = cached;
                break;
            }
            visited.push(current);
            walk.advance(document);
            let Some(parent) = walk.current() else {
                break;
            };
            let computed = &cascade.computed[parent];
            if document.nodes[parent].formatting_context == FormattingContext::Flex
                && matches!(
                    computed.flex_direction,
                    FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
                )
            {
                context = Some(parent);
                break;
            }
        }
        for node in visited {
            self.cache[node] = Some(context);
        }
        context
    }

    /// Whether `context` or a column context enclosing it holds content on
    /// `page`.
    fn is_occupied(
        &mut self,
        context: usize,
        page: u32,
        document: &Document,
        cascade: &CascadeResult,
    ) -> bool {
        let mut current = Some(context);
        while let Some(id) = current {
            if self.occupied.get(&id) == Some(&page) {
                return true;
            }
            current = self.enclosing(id, document, cascade);
        }
        false
    }

    /// Mark `context` and every column context enclosing it as holding
    /// content on `page`.
    fn occupy(
        &mut self,
        mut context: Option<usize>,
        page: u32,
        document: &Document,
        cascade: &CascadeResult,
    ) {
        while let Some(id) = context {
            if self.occupied.insert(id, page) == Some(page) {
                break;
            }
            context = self.enclosing(id, document, cascade);
        }
    }

    /// Mark the column contexts enclosing `node_id` as holding content on
    /// `page`.
    fn occupy_enclosing(
        &mut self,
        node_id: usize,
        page: u32,
        document: &Document,
        cascade: &CascadeResult,
    ) {
        let context = self.enclosing(node_id, document, cascade);
        self.occupy(context, page, document, cascade);
    }
}

/// The last box that took the current page name in the body's flow or in a
/// named flex context.
///
/// Named class-A boxes at one source coordinate form one page transition.
/// This coalesces zero-height named runs such as a/b/c/d/e without creating
/// one blank page per zero-height box.
#[derive(Default)]
struct LastNamedBox {
    /// The source coordinate of the box when it is named.
    raw_y: Option<f32>,
    context: Option<usize>,
    was_zero_height: bool,
    had_display_none_descendant: bool,
}

impl LastNamedBox {
    fn new(
        document: &Document,
        candidate: &PageCandidate,
        node_id: usize,
        raw_y: f32,
        height: f32,
    ) -> Self {
        Self {
            raw_y: candidate.context.is_named().then_some(raw_y),
            context: candidate.named_flex_context,
            was_zero_height: candidate.context.is_named() && height <= 0.001,
            had_display_none_descendant: candidate.context.is_named()
                && has_display_none_descendant(document, node_id),
        }
    }

    /// Whether a named box at `raw_y` in `context` joins the page transition
    /// of this zero-height named box instead of starting its own.
    fn shares_transition(&self, context: Option<usize>, raw_y: f32) -> bool {
        context == self.context
            && self.was_zero_height
            && !self.had_display_none_descendant
            && self
                .raw_y
                .is_some_and(|previous| (raw_y - previous).abs() <= 0.001)
    }

    /// Whether `candidate` is a named box of the page flow, or of a flex
    /// item's named flow, that joins the page transition of this zero-height
    /// named box.
    fn joins_transition(&self, candidate: &PageCandidate) -> bool {
        candidate.context.is_named()
            && (candidate.context.is_direct_body_element()
                || candidate.named_flex_context.is_some())
            && self.shares_transition(candidate.named_flex_context, candidate.raw_y)
    }
}

/// Whether the fixed-height box `node_id`, placed at `top`, fits on the
/// physical paper of a page that starts at `page_top` and has page border or
/// padding.
///
/// Page border and padding reduce the fragmentainer's content height, but
/// must not make such a box manufacture a second page merely to expose its
/// overflow into the page decoration area.
fn fits_decorated_paper(
    document: &super::context::LayoutSession<'_>,
    node_id: usize,
    insets: PageContentInsets,
    page_top: f32,
    paper_height: f32,
    top: f32,
    height: f32,
) -> bool {
    (insets.top > 0.0 || insets.right > 0.0 || insets.bottom > 0.0 || insets.left > 0.0)
        && top >= page_top
        && top + height <= page_top + paper_height + 0.001
        && has_fixed_height(document, node_id)
}

/// Whether `node_id` has a definite, non-negative `height` length.
fn has_fixed_height(document: &super::context::LayoutSession<'_>, node_id: usize) -> bool {
    style_dimension_length(document.block_axis(node_id).height()).is_some_and(|value| value >= 0.0)
}

/// Whether a box that took its page position holds content in the columns
/// around it: it takes part in the flow's page names and is named itself, or
/// is a rendered leaf with a block size.
fn box_occupies_column(
    context: &candidates::BoxContext,
    name_participates_in_flow: bool,
    height: f32,
) -> bool {
    name_participates_in_flow
        && (context.is_named() || (context.is_rendered_leaf() && height > 0.0))
}

/// The flow end of a candidate box placed at `effective_y`, or `None` when
/// the box has no height or no usable position and so cannot run on past
/// its start page.
///
/// A box ending exactly at a page edge belongs to the preceding page.
/// [`f32::EPSILON`] is too small at ordinary CSS coordinates and rounds away,
/// so callers map the end through the half-open interval directly:
/// ceil(end / step) - 1. The box remains visible on every slice it
/// intersects; line-level splitting is a later pass.
fn candidate_flow_end(effective_y: f32, height: f32) -> Option<f32> {
    if height > 0.0 && effective_y.is_finite() && effective_y >= 0.0 {
        let end = (effective_y + height).max(effective_y);
        (end.is_finite() && end > 0.0).then_some(end)
    } else {
        None
    }
}

/// Record the pages a placed candidate occupies.
///
/// Its start `page` takes the current page name and counts toward the page
/// reach. When the box runs on to `end_page`, the pages in between count too
/// and continue the page name.
fn record_candidate_pages(
    reach: &mut PageReach,
    page_names: &mut PageNames,
    control: &PageLayoutControl<'_>,
    page: u32,
    end_page: Option<u32>,
    in_flow: bool,
) -> Result<(), LayoutError> {
    page_names.name_page(page);
    reach.include(page, in_flow);
    match end_page {
        Some(end_page) => record_run_on(reach, page_names, control, page, end_page, in_flow),
        None => Ok(()),
    }
}

/// Break the lines of every paragraph again for a page-specific
/// containing-block width.
///
/// Pagination can change the page geometry after the first layout pass. This
/// helper refreshes only line breaking, leaving taffy's already computed box
/// geometry intact, so a page-aware painter can use the correct line breaks for
/// the page it is about to paint. It is intentionally separate from
/// [`layout_single_page`] because callers must opt into this narrow
/// post-pagination operation.
///
/// Each paragraph is broken at the content width of the nearest ancestor
/// with an authored width, otherwise at `max_advance`.
#[doc(hidden)]
pub fn relayout_text_for_width(document: &mut Document, cascade: &CascadeResult, max_advance: f32) {
    let mut session = super::context::LayoutSession::with_cascade(document, cascade);
    session.page_projection.clear();
    crate::layout::ifc::flow::rebreak_roots(&mut session, cascade, max_advance);
}

/// Lay out a Document on one A4 page (or the specified PageBox).
///
/// Text is laid out by the inline engine with the fonts the Document holds
/// ([`Document::set_font_collection`]). A Document that was given no fonts
/// takes the process-wide layer of the installed fonts
/// ([`crate::system_font_collection`]) on its first layout.
///
/// # In-place changes
/// - Bridge computed values to the node fields and pass overrides layout
///   reads, and assign the paragraphs of the inline engine
/// - Create a definite layout-only page-area ICB and lay out the document
///   element and its ordinary descendants through Taffy
/// - Store each box's used geometry in Node.layout
///
/// # Errors
/// - `LayoutError::Internal` — no `<body>` element found (fragment parses
///   are not supported yet) or an internal taffy error
/// - `LayoutError::IfcUnsupported` / `LayoutError::IfcLimitExceeded` — a
///   paragraph the inline engine cannot lay out, or one over its limits
/// - `LayoutError::FragmentLimitExceeded` — fragment or break-flow work budget exhausted
///
/// # Current non-goals
/// - Calling this repeatedly on one Document is safe (per-pass state is
///   cleared each time), but incremental recomputation is planned for later.
/// - Consumer PageBox overrides will be handled by future per-page PageBox support.
/// - Fragment parses (without `<body>`) will be supported later.
#[doc(hidden)]
pub fn layout_single_page(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<(), LayoutError> {
    let mut session = super::context::LayoutSession::with_cascade(document, cascade);
    layout_single_page_with_table_projection(&mut session, cascade, page_box, false, false)
}

#[cfg(test)]
pub(super) fn layout_single_page_checked(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<(), LayoutError> {
    let mut session = super::context::LayoutSession::with_cascade(document, cascade);
    layout_single_page_with_table_projection(&mut session, cascade, page_box, false, false)
}

/// A single-page layout in `session`, which stays open afterwards so that a
/// test can read what the pass left in the document through the same views
/// the pass used.
#[cfg(test)]
pub(super) fn layout_single_page_in_session(
    session: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<(), LayoutError> {
    layout_single_page_with_table_projection(session, cascade, page_box, false, false)
}

/// The error of a layout that nested deeper than the limit, if the session
/// recorded one. The boxes it left unlaid are zero-sized stand-ins, which the
/// cache must not keep.
fn take_recursion_failure(
    document: &mut super::context::LayoutSession<'_>,
) -> Result<(), LayoutError> {
    if let Some(node) = document.context.recursion.exceeded_at.take() {
        document.layout_dirty = true;
        return Err(LayoutError::Internal {
            message: format!(
                "layout nested more than {} levels deep at node {node}",
                document.context.recursion.limit
            ),
        });
    }
    Ok(())
}

/// The error a layout pass recorded in the session while taffy ran: a
/// layout that nested too deeply, a rejected table or an exhausted fragment
/// budget.
fn take_layout_failure(
    document: &mut super::context::LayoutSession<'_>,
) -> Result<(), LayoutError> {
    take_recursion_failure(document)?;
    if let Some(message) = document.context.table_layout_error.take() {
        // A rejected pass must not cache its temporary zero-sized fallback.
        document.layout_dirty = true;
        return Err(LayoutError::Internal { message });
    }
    if document.fragment_tree.limit_exceeded {
        return Err(LayoutError::FragmentLimitExceeded {
            limit: document.fragment_tree.limit,
        });
    }
    Ok(())
}

fn layout_single_page_with_table_projection(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    page_box: PageBox,
    project_explicit_table_parts: bool,
    paged: bool,
) -> Result<(), LayoutError> {
    super::validate_layout_depth(document)?;
    document.page_projection.clear();
    // At this observation-side entry point, synchronize membership.
    // `mark_in_document_flags` is an idempotent no-op when flags_dirty=false, so
    // this is effectively free after sink.finish(). It protects consumers that
    // skip cascade and call layout directly after post-parse mutation
    // (`Document::append_*`, etc.).
    //
    // Contract: cascade takes `&D: Dom` and cannot mutate, so its caller must
    // synchronize flags (automatic synchronization occurs only through parse.finish()).
    // Layout synchronizes here to ensure that stale flags never reach at least
    // the layout / paint stages.
    document.mark_in_document_flags();
    crate::image_resolve::resolve_inline_svg_intrinsic_sizes(document);
    crate::image_resolve::resolve_canvas_intrinsic_sizes(document);
    crate::image_resolve::resolve_default_object_sizes(document);
    if document.layout_cascade_generation != Some(cascade.generation()) {
        // Computed Grid/Flex style can change without a DOM tree mutation. Do
        // not let Taffy's per-node cache or resolved Grid rows survive that
        // cascade transition.
        document.layout_dirty = true;
    }
    document.layout_cascade_generation = None;
    // Counter snapshots shared with a previous element cascade cannot serve
    // this layout or the projection and paint that follow it.
    if document
        .counter_snapshot_cache
        .get()
        .is_some_and(|(generation, _)| *generation != cascade.element_generation())
    {
        document.counter_snapshot_cache.take();
    }
    if document
        .compact_counter_snapshot_cache
        .get()
        .is_some_and(|(generation, _)| *generation != cascade.element_generation())
    {
        document.compact_counter_snapshot_cache.take();
    }
    // Fonts are only read when a document is laid out: one that was given
    // none takes the installed fonts here, not when it is created.
    if document.ifc.is_none() {
        document.set_font_collection(crate::fonts::system_font_collection());
    }

    document.fragment_tree.clear();
    document.column_rules.clear();
    document.sticky_offsets.clear();
    for state in document
        .nodes
        .iter_mut()
        .filter_map(crate::node::Node::written_state)
    {
        state.multicol_groups.clear();
        state.multicol_rows.clear();
    }

    let (icb, available_space, content_width, content_height, body_id) = document
        .with_style_preparation_batch(|document| {
            // Step 1: bridge the computed values to the node fields and pass overrides.
            super::bridge::apply_computed_to_style_in_session(document, cascade)?;
            if paged {
                if let Some(breaks) = &document.clearance_breaks {
                    let breaks = breaks.applied_to(cascade);
                    clear_floats_at_forced_breaks(document, &breaks);
                } else {
                    clear_floats_at_forced_breaks(document, cascade);
                }
            } else if !document.forced_break_clearance.is_empty()
                || !document.forced_break_line_clearance.is_empty()
                || !document.forced_break_float_floor.is_empty()
                || !document.forced_break_line_float_floor.is_empty()
            {
                document.forced_break_clearance.clear();
                document.forced_break_line_clearance.clear();
                document.forced_break_float_floor.clear();
                document.forced_break_line_float_floor.clear();
                document.layout_dirty = true;
            }
            document.table_objects.project_explicit_parts = project_explicit_table_parts;

            // Step 2: resolve the paper/content split before shaping.  Text wrapping
            // uses the content width, not the outer paper width.
            let margins = page_margins(cascade, page_box);
            let insets = page_content_insets(cascade, page_box);
            // The page border and padding inset the page area, the initial
            // containing block, on every side.
            let content_width = insets.page_area_width(margins, page_box);
            let content_height = insets.page_area_height(margins, page_box);

            // Step 3: <body> lookup
            let body_id = find_body(document).ok_or_else(|| LayoutError::Internal {
                message: "no <body> element found (fragment parse not supported yet)".to_string(),
            })?;
            document.propagate_viewport_overflow(cascade, body_id);
            prepare_authored_block_widths(document, cascade, content_width);
            if let Some(state) = document.ifc.as_mut() {
                state.page_width = Some(content_width);
            }
            document.fixed_page_area = Some((content_width, content_height));
            document.fixed_static_page_origins.clear();
            let icb = super::initial_containing_block::prepare(
                document,
                cascade,
                content_width,
                content_height,
            );
            super::quirks_fill::prepare(document, cascade, body_id, content_width, content_height);
            let available_space = Size {
                width: AvailableSpace::Definite(content_width),
                height: AvailableSpace::Definite(content_height),
            };
            document.begin_oof_layout();
            Ok::<_, LayoutError>((icb, available_space, content_width, content_height, body_id))
        })?;
    crate::layout::dispatch::compute_root_layout_with_content_limits(
        document,
        protocol::NodeId::from(icb),
        available_space,
    );
    take_layout_failure(document)?;
    super::fixed::correct_root_oof_auto_margins(
        document,
        icb,
        Size {
            width: content_width,
            height: content_height,
        },
    );
    document.finalize_oof_layout();
    // After Step 5, check semantic parent-child geometry invariants and
    // replace any invalid subtree with the deterministic fallback (zero).
    // `set_unrounded_layout` sanitizes non-finite values; this check validates
    // parent-child relationships above that layer. See the
    // `enforce_layout_invariants` documentation. Events enter the session's
    // warning buffer along with Steps 1 and 2; Step 6 drains it.
    enforce_layout_invariants(document, body_id);
    super::multicol::record_out_of_flow_boxes_on_column_lines(document, cascade);
    take_layout_failure(document)?;
    super::sticky::resolve_offsets(document, cascade);
    document.fragment_tree.finalize();
    document.column_rules = crate::column_rules::prepare(document, cascade)?;

    // Step 6: replay buffered LayoutWarn events.
    //
    // The session warning buffer has events from bridge calls, Taffy's
    // `set_unrounded_layout`, and `enforce_layout_invariants`.
    //
    // No external caller can supply an observer yet — `layout_single_page`'s
    // signature is a dom→paint boundary (`raikiri-paint` and `raikiri` both
    // call it directly) and adding a parameter, or a new `_with_observer`
    // sibling, is a decision for that wall rather than this task. `observer`
    // is therefore always `None` today, so this always falls back to the
    // same `eprintln!` shape `fonts.rs` uses when uncalled with an observer —
    // but the integration logic is real and ready for a future `_with_observer`
    // sibling to wire an observer through with no further refactor.
    let mut observer: LayoutWarnObserver<'_> = None;
    for event in document.context.layout_warnings.drain(..) {
        emit_layout_warn(&mut observer, event);
    }
    document.layout_cascade_generation = Some(cascade.generation());

    Ok(())
}

/// One page in the block-flow pagination result.
///
/// `content_origin_y` is measured in the single laid-out document's body
/// coordinate space.  A page-aware painter subtracts it before adding the
/// page's physical top margin.  Keeping the source coordinate here means a
/// later scene can select a page without cloning or relaying out the DOM.
#[derive(Debug, Clone, PartialEq)]
#[doc(hidden)]
pub struct PageSlice {
    /// Zero-based page number.
    pub page_index: u32,
    /// Body-content y coordinate at which this page begins.
    pub content_origin_y: f32,
    /// Named page selected by the first class-A box on this page.
    pub page_name: Option<String>,
}

/// Optional image resolution before paginating a document.
#[doc(hidden)]
pub struct PageImageOptions<'a> {
    /// Resolves the intrinsic sizes of rendered images.
    pub resolver: &'a dyn ReplacedResolver,
    /// Base URL for relative image sources.
    pub base_url: Option<&'a url::Url>,
}

/// Borrowed settings for [`layout_pages_with_options`].
#[derive(Default)]
#[doc(hidden)]
pub struct PageLayoutOptions<'a> {
    /// Per-page content heights for horizontal pagination.
    /// Empty keeps the fixed page height. Beyond a nonempty schedule, repeat
    /// its last height after normalizing invalid entries to the fixed height.
    /// Vertical pagination instead uses
    /// [`Self::page_widths`] for its block-axis page advances.
    pub page_steps: &'a [f32],
    /// Per-page content widths; empty keeps the fixed page width.
    /// Horizontal pagination scales percentage widths relative to the first
    /// scheduled width, leaving missing entries unchanged. For a vertical
    /// root, these widths provide the successive block-axis page advances,
    /// repeating the last normalized width beyond a nonempty schedule.
    pub page_widths: &'a [f32],
    /// Image resolution performed before layout, when supplied.
    pub images: Option<PageImageOptions<'a>>,
    /// Caller-owned limits and cancellation state; absent uses
    /// [`PageLayoutControl::default`]. The caller can observe truncation
    /// through the same control's [`PageLayoutControl::page_limit_reached`].
    pub control: Option<&'a PageLayoutControl<'a>>,
}

/// Controls page production and cancellation in the paginator.
///
/// The default matches [`raikiri_traits::RenderLimits::default`]. Pass `None`
/// to [`Self::new`] only when the caller intentionally wants unbounded pages.
#[doc(hidden)]
pub struct PageLayoutControl<'a> {
    max_pages: Option<u32>,
    abort_check: Option<&'a dyn Fn() -> bool>,
    truncate_at_page_limit: bool,
    page_limit_reached: Cell<bool>,
}

impl<'a> PageLayoutControl<'a> {
    /// Create a paginator control with the supplied page limit.
    pub fn new(max_pages: Option<u32>) -> Self {
        Self {
            max_pages,
            abort_check: None,
            truncate_at_page_limit: false,
            page_limit_reached: Cell::new(false),
        }
    }

    /// Create a provisional paginator for page-geometry discovery.
    ///
    /// When a page limit is set and pagination reaches it, layout stops and
    /// returns the page prefix. Call [`Self::page_limit_reached`] to distinguish
    /// that partial result from a complete layout. A final scheduled pass
    /// should use [`Self::new`] so excess pages are rejected.
    pub fn for_geometry_discovery(max_pages: Option<u32>) -> Self {
        Self {
            max_pages,
            abort_check: None,
            truncate_at_page_limit: true,
            page_limit_reached: Cell::new(false),
        }
    }

    /// Check for cancellation while collecting and producing page candidates.
    pub fn with_abort_check(mut self, abort_check: &'a dyn Fn() -> bool) -> Self {
        self.abort_check = Some(abort_check);
        self
    }

    /// Return whether the most recent paginator call returned a partial page
    /// prefix because it reached this control's limit.
    pub fn page_limit_reached(&self) -> bool {
        self.page_limit_reached.get()
    }

    fn check_aborted(&self) -> Result<(), LayoutError> {
        if self.abort_check.is_some_and(|check| check()) {
            Err(LayoutError::Aborted)
        } else {
            Ok(())
        }
    }

    fn check_page_index(&self, page_index: u32) -> Result<(), LayoutError> {
        self.check_aborted()?;
        let actual = u64::from(page_index) + 1;
        if let Some(limit) = self.max_pages
            && actual > u64::from(limit)
        {
            return Err(LayoutError::PageLimitExceeded { limit, actual });
        }
        Ok(())
    }

    /// `page_index` bounded by the page limit, for per-page records that a
    /// discovery pass truncating at the limit would otherwise grow without
    /// bound.
    fn capped_page_index(&self, page_index: u32) -> u32 {
        self.max_pages
            .map_or(page_index, |limit| page_index.min(limit.saturating_sub(1)))
    }

    /// The last page to produce when pagination reached `reach_max`: that
    /// page, or, once the page limit stopped pagination, the last page the
    /// limit allows (the one before `max_pages`).
    fn last_page(&self, reach_max: u32) -> u32 {
        if self.page_limit_reached.get() {
            self.max_pages.unwrap_or(0).saturating_sub(1)
        } else {
            reach_max
        }
    }

    /// Whether pagination may go on to `page_index`. A control that truncates
    /// at the page limit records that it was reached and stops pagination
    /// with `false` instead of failing.
    fn within_page_limit(&self, page_index: u32) -> Result<bool, LayoutError> {
        match self.check_page_index(page_index) {
            Ok(()) => Ok(true),
            Err(LayoutError::PageLimitExceeded { .. }) if self.truncate_at_page_limit => {
                self.page_limit_reached.set(true);
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn check_discovery_page_index(&self, page_index: u32) -> Result<(), LayoutError> {
        self.within_page_limit(page_index).map(|_| ())
    }
}

impl Default for PageLayoutControl<'_> {
    fn default() -> Self {
        Self::new(raikiri_traits::RenderLimits::default().max_document_pages)
    }
}

struct PageOrigins {
    fixed_step: f32,
    scheduled_steps: Vec<f32>,
    scheduled_origins: Vec<f32>,
}

impl PageOrigins {
    fn new(page_steps: &[f32], fixed_step: f32) -> Self {
        let scheduled_steps: Vec<_> = page_steps
            .iter()
            .map(|step| {
                if step.is_finite() && *step > 0.0 {
                    *step
                } else {
                    fixed_step
                }
            })
            .collect();
        let mut scheduled_origins = Vec::with_capacity(scheduled_steps.len() + 1);
        scheduled_origins.push(0.0);
        for step in &scheduled_steps {
            let origin = scheduled_origins.last().copied().unwrap_or(0.0) + step;
            scheduled_origins.push(origin);
        }
        // Newly discovered pages continue the last known page area.
        let fixed_step = scheduled_steps.last().copied().unwrap_or(fixed_step);
        Self {
            fixed_step,
            scheduled_steps,
            scheduled_origins,
        }
    }

    fn step_at(&self, page_index: u32) -> f32 {
        self.scheduled_steps
            .get(page_index as usize)
            .copied()
            .unwrap_or(self.fixed_step)
    }

    fn origin(&self, page_index: u32) -> f32 {
        let index = page_index as usize;
        if self.scheduled_steps.is_empty() {
            return page_index as f32 * self.fixed_step;
        }
        if let Some(origin) = self.scheduled_origins.get(index) {
            return *origin;
        }
        let scheduled_pages = self.scheduled_steps.len();
        let scheduled_end = self.scheduled_origins[scheduled_pages];
        scheduled_end + (index - scheduled_pages) as f32 * self.fixed_step
    }

    fn page_index_for_y(&self, y: f32) -> u32 {
        if !y.is_finite() || y <= 0.0 {
            return 0;
        }
        if self.scheduled_steps.is_empty() {
            let tail_page = (f64::from(y) / f64::from(self.fixed_step)).floor();
            return self.correct_page_index_for_y(self.page_index_after_prefix(tail_page), y);
        }
        let scheduled_pages = self.scheduled_steps.len();
        let scheduled_end = self.scheduled_origins[scheduled_pages];
        if y < scheduled_end {
            let index = self.scheduled_origins[..=scheduled_pages]
                .partition_point(|origin| *origin <= y)
                .saturating_sub(1);
            return self.correct_page_index_for_y(index as u32, y);
        }
        let extra =
            ((f64::from(y) - f64::from(scheduled_end)) / f64::from(self.fixed_step)).floor();
        self.correct_page_index_for_y(self.page_index_after_prefix(extra), y)
    }

    /// The page that a flow position `y` falls on, or `None` when `y` is not
    /// a usable flow position (not finite, or above the first page).
    fn page_index_in_flow(&self, y: f32) -> Option<u32> {
        (y.is_finite() && y >= 0.0).then(|| self.page_index_for_y(y))
    }

    /// The page whose start is the first page edge inside a margin that
    /// starts at `margin_start`, at most `last_page`. An edge within 0.001
    /// of `margin_start` counts as inside the margin, unless the margin
    /// follows a forced break: the forced break at its start is not an edge
    /// inside it.
    fn first_edge_in_margin(
        &self,
        margin_start: f32,
        follows_forced_break: bool,
        last_page: u32,
    ) -> u32 {
        let edge_slack = if follows_forced_break { 0.001 } else { -0.001 };
        (self.page_index_for_y(margin_start + edge_slack) + 1).min(last_page)
    }

    /// The page a box of block size `height` whose top is at `y` starts on.
    /// A box with no block size at a page's end still fits on that page, so
    /// it does not open the next one: a forced break after it would
    /// otherwise leave it alone on a page.
    fn start_page(&self, y: f32, height: f32) -> u32 {
        if height <= 0.0 {
            self.page_index_for_end(y)
        } else {
            self.page_index_for_y(y)
        }
    }

    /// The page a fixed-height block is pushed to when its first line, the
    /// text of `text_height` at `text_y`, would straddle the end of
    /// `current_page` (see [`first_line_overflows`]), or `None` when the block
    /// stays. The block moves at least to the page after `current_page`.
    fn first_line_push_page(
        &self,
        text_y: f32,
        text_height: f32,
        block_y: f32,
        current_page: u32,
    ) -> Option<u32> {
        first_line_overflows(
            text_y,
            text_height,
            block_y,
            self.origin(current_page),
            self.step_at(current_page),
        )
        .then(|| {
            let natural_page = self.page_index_for_y(text_y + text_height);
            forced_target_page(current_page, natural_page)
        })
    }

    /// Where a box starts when an unforced break falls in the collapsed
    /// margin above it at the edge that opens `natural_page`: its border-box
    /// top.
    ///
    /// A forced break that already coincides with that edge sits before the
    /// margin, which therefore stays on the new page above the box's
    /// `margin_top`. Otherwise the margin, which starts at `margin_start`,
    /// breaks at the first page edge inside it, and the rest of it is
    /// truncated together with the blank pages it would fill.
    fn truncated_margin_target(
        &self,
        natural_page: u32,
        forced_before: bool,
        margin_top: f32,
        margin_start: impl FnOnce() -> f32,
        follows_forced_break: bool,
    ) -> f32 {
        if forced_before {
            // The break opens the page at the top of the margin, which may lie
            // on a page before the box's. A margin that runs past the end of
            // that page is cut at that end, and the box starts the next page
            // (Chromium).
            let break_page = self
                .page_starting_at_or_after(margin_start())
                .min(natural_page);
            let top = self.origin(break_page) + margin_top;
            let next = self.origin(break_page.saturating_add(1));
            if break_page >= natural_page {
                self.origin(natural_page) + margin_top
            } else if top >= next - 0.001 {
                next
            } else {
                top
            }
        } else {
            self.origin(self.first_edge_in_margin(
                margin_start(),
                follows_forced_break,
                natural_page,
            ))
        }
    }

    /// The page a box at flow position `effective_y` falls on when it starts
    /// a page transition from `current_page`, or `current_page` when the
    /// position is outside the flow.
    ///
    /// Preceding content that overflowed past the current page (a tall box,
    /// or floats it contains) may have put the box partway down a later page,
    /// which the flow has reached by `flow_max`. A forced break or a page-type
    /// change (`forced_transition`) sits after all of that content, so it opens
    /// the page after the one the box's margin box starts on, below its
    /// block-start `margin_top`, unless that margin box is already at the top
    /// of a page. A margin box on a page the flow has not reached opens the
    /// page it starts on.
    fn transition_natural_page(
        &self,
        effective_y: f32,
        margin_top: f32,
        current_page: u32,
        flow_max: u32,
        forced_transition: bool,
    ) -> u32 {
        let Some(page) = self.page_index_in_flow(effective_y) else {
            return current_page;
        };
        // A forced break sits at the top of the margin box. A negative margin
        // puts that top below the border box, past content that runs on
        // beyond the border box's page. A positive one that runs over a page
        // edge leaves the break on the page the margin box starts on: the
        // margin follows the break, which cuts it at the end of the page the
        // break opens.
        let margin_box_y = effective_y - margin_top;
        let break_page = self
            .page_index_in_flow(margin_box_y)
            .map_or(page, |at| if margin_top > 0.0 { at } else { at.max(page) });
        if !forced_transition {
            page
        } else if break_page > current_page && flow_max >= break_page && margin_box_y.is_finite() {
            self.page_starting_at_or_after(margin_box_y)
        } else {
            // Content that ends at a page edge reaches no further than the
            // page before it, and the break opens the page at that edge.
            break_page
        }
    }

    /// The page that starts at flow position `y` (within 0.001), or else the
    /// page after the one `y` falls on. A position above the first page falls
    /// on the first page.
    fn page_starting_at_or_after(&self, y: f32) -> u32 {
        let page = self.page_index_for_y(y.max(0.0));
        if (y - self.origin(page)).abs() <= 0.001 {
            page
        } else {
            page.saturating_add(1)
        }
    }

    /// The last page that a box of block size `height` whose top is at `y`
    /// runs on, or `None` when it cannot run on past its start page (see
    /// [`candidate_flow_end`]).
    fn end_page(&self, y: f32, height: f32) -> Option<u32> {
        candidate_flow_end(y, height).map(|end| self.page_index_for_end(end))
    }

    fn page_index_for_end(&self, end: f32) -> u32 {
        if !end.is_finite() || end <= 0.0 {
            return 0;
        }
        if self.scheduled_steps.is_empty() {
            let tail_page = (f64::from(end) / f64::from(self.fixed_step)).ceil() - 1.0;
            return self
                .correct_page_index_for_end(self.page_index_after_prefix(tail_page.max(0.0)), end);
        }
        let scheduled_pages = self.scheduled_steps.len();
        let index = self.scheduled_origins[1..].partition_point(|origin| *origin < end);
        if index < scheduled_pages {
            return self.correct_page_index_for_end(index as u32, end);
        }
        let scheduled_end = self.scheduled_origins[scheduled_pages];
        let extra =
            ((f64::from(end) - f64::from(scheduled_end)) / f64::from(self.fixed_step)).ceil() - 1.0;
        self.correct_page_index_for_end(self.page_index_after_prefix(extra.max(0.0)), end)
    }

    fn correct_page_index_for_y(&self, page_index: u32, y: f32) -> u32 {
        let page_end = self.page_end(page_index);
        let previous_page_end = page_index
            .checked_sub(1)
            .map(|previous| self.page_end(previous))
            .unwrap_or(f32::NEG_INFINITY);
        if previous_page_end <= y && y < page_end {
            page_index
        } else {
            self.first_page_ending_after(y)
        }
    }

    fn correct_page_index_for_end(&self, page_index: u32, end: f32) -> u32 {
        let page_end = self.page_end_with_rounding_tolerance(page_index);
        let previous_page_end = page_index
            .checked_sub(1)
            .map(|previous| self.page_end_with_rounding_tolerance(previous))
            .unwrap_or(f32::NEG_INFINITY);
        if previous_page_end < end && end <= page_end {
            page_index
        } else {
            self.first_page_ending_at_or_after(end)
        }
    }

    fn page_end(&self, page_index: u32) -> f32 {
        self.origin(page_index.saturating_add(1))
    }

    fn page_end_with_rounding_tolerance(&self, page_index: u32) -> f32 {
        let page_end = self.page_end(page_index);
        let next = page_end.next_up();
        if next - page_end < self.step_at(page_index) {
            next
        } else {
            page_end
        }
    }

    fn first_page_ending_after(&self, y: f32) -> u32 {
        self.first_page_matching_end(y, |origins, page_index, y| origins.page_end(page_index) > y)
    }

    fn first_page_ending_at_or_after(&self, end: f32) -> u32 {
        self.first_page_matching_end(end, |origins, page_index, end| {
            origins.page_end_with_rounding_tolerance(page_index) >= end
        })
    }

    fn first_page_matching_end(
        &self,
        value: f32,
        matches: impl Fn(&Self, u32, f32) -> bool,
    ) -> u32 {
        let mut low = 0_u64;
        let mut high = u64::from(u32::MAX) + 1;
        while low < high {
            let middle = low + (high - low) / 2;
            if matches(self, middle as u32, value) {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        low.min(u64::from(u32::MAX)) as u32
    }

    fn page_index_after_prefix(&self, tail_page: f64) -> u32 {
        let scheduled_pages = u32::try_from(self.scheduled_steps.len()).unwrap_or(u32::MAX);
        let available = u32::MAX - scheduled_pages;
        if tail_page >= f64::from(available) {
            u32::MAX
        } else {
            scheduled_pages + tail_page as u32
        }
    }
}

/// Layout a document and return neutral per-page fragment snapshots.
///
/// This is the `raikiri-dom` producer-facing geometry API corresponding to
/// fulgur's `PaginationGeometryTable`.  The existing [`layout_pages`] API is
/// unchanged; this convenience wrapper runs it and projects its post-layout
/// DOM coordinates into page-local [`PageFragmentItem`] records.
///
/// The geometry pass clips a box at page boundaries. Shaped text placements
/// additionally carry a neutral line range based on each line's CSS-px center;
/// the pass does not expose the shaping engine or re-shape the text. Fixed-
/// positioned subtrees use their existing post-layout geometry as complete
/// per-page repeat records. Eligible first table-header groups likewise use
/// complete copies on their table's committed pages; footers remain in flow.
/// The projection is deterministic
/// and keeps source-node identity stable, so a consumer can select
/// continuation lines without re-running pagination.
#[cfg(test)]
pub(crate) fn layout_page_fragments(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<Vec<PageFragment>, LayoutError> {
    let slices = layout_pages(document, cascade, page_box)?;
    Ok(page_fragments_from_slices(
        document, cascade, page_box, &slices,
    ))
}

/// Resolve one producer-owned page metadata record from a page cascade.
///
/// The returned `content_box.x/y` is the physical page-local offset for the
/// page's content-relative item rectangles. This helper keeps the conversion
/// in `raikiri-dom` so consumers do not recompute page margins or insets.
pub(crate) fn resolve_page_fragment_geometry(
    cascade: &CascadeResult,
    page_box: PageBox,
    page_index: u32,
) -> PageFragmentPageGeometry {
    let margins = page_margins(cascade, page_box);
    let content_insets = page_content_insets(cascade, page_box);
    let content_width = content_insets.page_area_width(margins, page_box);
    let content_height = content_insets.page_area_height(margins, page_box);
    let content_box = PageFragmentRect::new(
        margins.left + content_insets.left,
        margins.top + content_insets.top,
        content_width,
        content_height,
    );
    let margins = PageFragmentInsets::new(margins.top, margins.right, margins.bottom, margins.left);
    let content_insets = PageFragmentInsets::new(
        content_insets.top,
        content_insets.right,
        content_insets.bottom,
        content_insets.left,
    );
    let orientation = if page_box.width > page_box.height {
        PageFragmentOrientation::Landscape
    } else {
        PageFragmentOrientation::Portrait
    };
    PageFragmentPageGeometry::new(
        page_index,
        page_box,
        margins,
        content_insets,
        content_box,
        orientation,
    )
}

/// Whether a text node with the lines `owned` has glyphs on `line` or on a
/// later line of its paragraph.
fn reaches_line(owned: Option<&IfcTextLines>, line: usize) -> bool {
    owned
        .and_then(|owned| owned.lines.last())
        .is_some_and(|last| last.line >= line)
}

/// Block-start and block-end edges of every line of the ifc paragraph a text
/// node belongs to, from the text node's own first line. A paragraph whose
/// text sits in inline elements, or in several text nodes, has lines that no
/// single text node owns. A text node outside an ifc paragraph has none.
fn paragraph_line_bounds(
    document: &Document,
    node_id: usize,
    text_lines: &TextLineIndex,
) -> Option<Vec<(f32, f32)>> {
    let owned = text_lines.get(node_id)?;
    let first_top = positioned_owned_text_line_bounds(document, owned)?
        .first()?
        .0;
    Some(
        ifc_root_line_bounds(document, owned.root)?
            .into_iter()
            .map(|(top, bottom)| (top - first_top, bottom - first_top))
            .collect(),
    )
}

/// Block-start and block-end edges of every line of the ifc paragraph rooted
/// at `root`, as they sit in the document, or `None` when the root has no
/// positioned lines.
fn ifc_root_line_bounds(document: &Document, root: usize) -> Option<Vec<(f32, f32)>> {
    let line_count = document
        .ifc_layout_node(root)?
        .ifc
        .as_ref()?
        .lines
        .as_ref()?
        .lines
        .len();
    let paragraph = IfcTextLines {
        root,
        width: 0.0,
        lines: (0..line_count)
            .map(|line| IfcTextLine {
                line,
                top: 0.0,
                bottom: 0.0,
            })
            .collect(),
    };
    positioned_owned_text_line_bounds(document, &paragraph)
}

/// Whether the floats before the block `block_id` put the first line of its
/// paragraph below themselves. `owned` holds the lines of the text that begins
/// the paragraph, which must own its first line, and `offset` is how far that
/// line sits below the top of the block's content (more than a rounding
/// error). In a block of the normal flow whose lines can move, the floats
/// before it are the one thing that leaves room above the first line: a block
/// child before the line, or a float of its own that moves with it, would
/// keep the lines from moving.
fn floats_put_first_line_down(
    document: &Document,
    block_id: usize,
    owned: Option<&IfcTextLines>,
    offset: f32,
) -> bool {
    owned.is_some_and(|owned| owned.lines.first().is_some_and(|line| line.line == 0))
        && offset > 0.5
        && is_plain_block_box(document, block_id)
}

/// The height of the first line of the paragraph the text node `node_id`
/// belongs to, or `None` when the node does not own that line (a line of an
/// image comes before its first text, say) or it has no position.
fn first_owned_line_height(
    document: &Document,
    node_id: usize,
    text_lines: &TextLineIndex,
) -> Option<f32> {
    let owned = text_lines.get(node_id)?;
    let first_line = IfcTextLines {
        root: owned.root,
        width: owned.width,
        lines: vec![*owned.lines.first().filter(|line| line.line == 0)?],
    };
    positioned_owned_text_line_bounds(document, &first_line)?
        .first()
        .map(|&(top, bottom)| bottom - top)
}

/// A break moved up between a paragraph's lines: the first line that moves,
/// how far it and the lines after it move, and the page they move to.
type LineBreakMove = (usize, f32, u32);

/// The widows a paragraph of `total_lines` lines can have next to `orphans`.
/// A paragraph too short for both constraints keeps its orphans and gives up
/// the widows it cannot have: its last piece takes the lines the orphans leave
/// it (CSS Fragmentation 3 §3.3 lets the UA ignore a constraint it cannot
/// meet, and Chromium keeps the orphans). A paragraph breaks between its
/// lines exactly when it has more than `orphans` of them.
fn usable_widows(total_lines: usize, orphans: usize, widows: usize) -> usize {
    widows.min(total_lines.saturating_sub(orphans)).max(1)
}

/// The lines that a page gives up to the next one when the padding and border
/// below its last line do not fit, and it holds `lines` lines of the paragraph.
/// The break the edges ask for follows `widows` lines after it and `orphans`
/// before it when the page can have both; a page that cannot ignores the two
/// and gives up its last line, as Chromium does.
fn edge_break_lines(lines: usize, orphans: usize, widows: usize) -> usize {
    if lines.saturating_sub(widows) >= orphans {
        widows
    } else {
        1
    }
}

/// The break that keeps the last `widows` lines of a paragraph together at
/// the top of the page after `current_page` (CSS Fragmentation 3 §3.3).
/// `line_metrics` holds each line's top and bottom relative to the top of
/// the paragraph's first line, which sits at `effective_y`, and has at least
/// `widows` lines. Returns `None` when those lines would not fit on that page
/// either, so the natural break stays, or when the break would not move them
/// down.
fn widows_break_on_next_page(
    line_metrics: &[(f32, f32)],
    effective_y: f32,
    current_page: u32,
    widows: usize,
    page_origins: &PageOrigins,
) -> Option<LineBreakMove> {
    let total_lines = line_metrics.len();
    Some(total_lines - widows)
        .filter(|&line| {
            // Widows that cannot fit on the next page would not be kept
            // together there either; the natural break stays.
            let moved_height = line_metrics[total_lines - 1].1 - line_metrics[line].0;
            moved_height <= page_origins.step_at(current_page.saturating_add(1)) + 0.001 && line > 0
        })
        .map(|line| {
            let target_y = page_origins.origin(current_page.saturating_add(1));
            (
                line,
                target_y - (effective_y + line_metrics[line].0),
                current_page.saturating_add(1),
            )
        })
        .filter(|(_, shift, _)| shift.is_finite() && *shift > 0.0)
}

/// The lines of a paragraph that one page holds.
struct LinePiece {
    /// The page the lines are on.
    page: u32,
    /// The first of the lines.
    first: usize,
    /// How many lines there are.
    count: usize,
    /// How far the first line moved down to start the page, when it did.
    push: Option<f32>,
}

/// Where each page of a paragraph starts, and the lines it holds: a line that
/// crosses a page end starts the next page, and the lines after it move down
/// with it. `forced` names the lines that start a page whether they cross the
/// end of the one before or not: `(line, page)`. The last line carries
/// `tail_edges`, the padding and border that close the paragraph below it: it
/// crosses a page end when they do, and starts the next page with them.
fn line_pieces(
    line_metrics: &[(f32, f32)],
    effective_y: f32,
    current_page: u32,
    page_origins: &PageOrigins,
    forced: &[(usize, u32)],
    tail_edges: f32,
) -> Vec<LinePiece> {
    let mut pieces: Vec<LinePiece> = Vec::new();
    let mut shift = 0.0_f32;
    let mut page = current_page;
    for (line, &(line_top, line_bottom)) in line_metrics.iter().enumerate() {
        let top = effective_y + line_top + shift;
        let edges = if line + 1 == line_metrics.len() {
            tail_edges
        } else {
            0.0
        };
        let bottom = effective_y + line_bottom + shift + edges;
        // The flow itself may carry the line onto a later page.
        page = page.max(page_origins.page_index_for_y(top + 0.001));
        let mut push = None;
        if line > 0 {
            let next = page.saturating_add(1);
            let crosses = top > page_origins.origin(page) + 0.001
                && bottom > page_origins.origin(next) + 0.001;
            let target = match forced.iter().find(|&&(first, _)| first == line) {
                Some(&(_, forced_page)) => {
                    (page_origins.origin(forced_page) - top > 0.001).then_some(forced_page)
                }
                None => crosses.then_some(next),
            };
            if let Some(target) = target {
                page = target;
                let delta = page_origins.origin(page) - top;
                shift += delta;
                push = Some(delta);
            }
        }
        match pieces.last_mut() {
            Some(piece) if piece.page == page => piece.count += 1,
            _ => pieces.push(LinePiece {
                page,
                first: line,
                count: 1,
                push,
            }),
        }
    }
    pieces
}

/// The moves that keep every line of a paragraph whole on one page, each page
/// ending with the lines that fit it and the next starting with the line that
/// does not, and that give the pages after a break the `widows` lines it
/// needs, or as many of them as the page before can spare and still keep
/// `orphans` lines.
///
/// `line_metrics` and `effective_y` are as for [`widows_break_on_next_page`];
/// the first line is on `current_page`.
///
/// A line that crosses a page end without starting the next page moves to the
/// top of that page, and the lines after it move with it. The widows are
/// counted on the pages as they come out: a page that a moved line starts
/// holds fewer lines than the page grid has room for. The first break that
/// leaves fewer lines after it than the widows ask for, and whose page can
/// spare a line and keep its orphans, moves up; a break whose page cannot stays
/// where it falls. The lines the page before gives up are laid out again from
/// the top of the page after it, so the lines that then cross its end start the
/// page after that. When the widows are more lines than a page holds, Chromium
/// repairs only the break from its first fragment. A whole-block move has
/// already consumed that fragment when `fit.first_fragment` is false.
///
/// `tail_edges` is the padding and border below the last line. They stay with
/// the lines at the end of the last page: when they do not fit under it, the
/// page gives up the lines of an edge break to the next one (see
/// [`edge_break_lines`]), which holds them with the edges. The breaks the lines
/// themselves need are repaired first, and the edges are fitted to the lines as
/// they come out.
fn paragraph_line_moves(
    line_metrics: &[(f32, f32)],
    effective_y: f32,
    current_page: u32,
    fit: &ParagraphLineFit,
    page_origins: &PageOrigins,
) -> Vec<LineBreakMove> {
    let (Some(&(_, last_bottom)), total) = (line_metrics.last(), line_metrics.len()) else {
        return Vec::new();
    };
    let (orphans, widows, tail_edges, first_fragment) =
        (fit.orphans, fit.widows, fit.tail_edges, fit.first_fragment);
    let kept_widows = usable_widows(total, orphans, widows);
    let mut pieces = line_pieces(
        line_metrics,
        effective_y,
        current_page,
        page_origins,
        &[],
        0.0,
    );
    // Judge the declared widows, not the reduced count a short paragraph can
    // spare. An impossible count is relaxed only at the opening fragment.
    let widows_fit = pieces.last().is_some_and(|last| {
        let first_widow = total - widows.clamp(1, total);
        last_bottom - line_metrics[first_widow].0 <= page_origins.step_at(last.page) + 0.001
    });
    let mut breaks: Vec<(usize, u32)> = pieces
        .iter()
        .zip(pieces.iter().skip(1))
        .enumerate()
        .find_map(|(index, (before, after))| {
            let remaining = total - after.first;
            let moved = kept_widows
                .saturating_sub(remaining)
                .min(before.count.saturating_sub(orphans));
            (moved > 0 && ((first_fragment && index == 0) || widows_fit))
                .then_some((after.first - moved, after.page))
        })
        .into_iter()
        .collect();
    if !breaks.is_empty() {
        pieces = line_pieces(
            line_metrics,
            effective_y,
            current_page,
            page_origins,
            &breaks,
            0.0,
        );
    }
    if tail_edges > 0.0 {
        let closed = line_pieces(
            line_metrics,
            effective_y,
            current_page,
            page_origins,
            &breaks,
            tail_edges,
        );
        // The edges start a page of their own when they do not fit under the
        // last line.
        if let Some(last) = pieces.last().filter(|_| closed.len() > pieces.len()) {
            let moved = edge_break_lines(last.count, orphans, widows);
            breaks.push((total - moved, last.page.saturating_add(1)));
            pieces = line_pieces(
                line_metrics,
                effective_y,
                current_page,
                page_origins,
                &breaks,
                tail_edges,
            );
        } else {
            pieces = closed;
        }
    }
    pieces
        .iter()
        .filter_map(|piece| Some((piece.first, piece.push?, piece.page)))
        .filter(|&(_, delta, _)| delta.is_finite() && delta > 0.0)
        .collect()
}

/// The moves that keep every line of a paragraph inside an absolutely
/// positioned box whole on one page: a line that crosses a page end starts the
/// next page, and the lines after it move down with it. The box's first line
/// moves like any other: the page breaks between the box's top edge and its
/// first line. A line taller than the page it would start stays where it is,
/// as it fits no page.
///
/// The breaks between the lines keep `widows` as in the flow (see
/// [`paragraph_line_moves`]), while `orphans` never moves the first line: the
/// box cannot start its lines on a later page. A paragraph with a line taller
/// than a page breaks only where its lines cross the page ends.
///
/// `line_metrics` and `effective_y` are as for [`widows_break_on_next_page`].
fn positioned_line_moves(
    line_metrics: &[(f32, f32)],
    effective_y: f32,
    page_origins: &PageOrigins,
    orphans: i32,
    widows: i32,
) -> Vec<LineBreakMove> {
    let Some(&(first_top, first_bottom)) = line_metrics.first() else {
        return Vec::new();
    };
    let top = effective_y + first_top;
    let mut page = page_origins.page_index_for_y(top + 0.001);
    let next = page.saturating_add(1);
    let crosses = top > page_origins.origin(page) + 0.001
        && effective_y + first_bottom > page_origins.origin(next) + 0.001;
    let delta = page_origins.origin(next) - top;
    let mut moves = Vec::new();
    let mut start = effective_y;
    if crosses
        && first_bottom - first_top <= page_origins.step_at(next) + 0.001
        && delta.is_finite()
        && delta > 0.0
    {
        moves.push((0, delta, next));
        start += delta;
        page = next;
    }
    let fit = ParagraphLineFit::measure(
        line_metrics,
        start,
        page_origins.origin(page),
        page_origins.page_end(page),
        orphans,
        widows,
        0.0,
    );
    let kept = paragraph_line_moves(line_metrics, start, page, &fit, page_origins);
    // A line that runs over the end of the page it lands on, as it is taller
    // than that page, fits no page: the lines then break only where they
    // cross.
    if leaves_a_line_across(line_metrics, start, &kept, page_origins) {
        moves.extend(crossing_line_moves(line_metrics, start, page_origins));
    } else {
        moves.extend(kept);
    }
    moves
}

/// Whether a line crosses the end of the page it starts on once `moves`
/// have moved the lines down, or is taller than that page.
fn leaves_a_line_across(
    line_metrics: &[(f32, f32)],
    effective_y: f32,
    moves: &[LineBreakMove],
    page_origins: &PageOrigins,
) -> bool {
    let mut shift = 0.0_f32;
    line_metrics
        .iter()
        .enumerate()
        .any(|(line, &(line_top, line_bottom))| {
            shift += moves
                .iter()
                .filter(|&&(moved, _, _)| moved == line)
                .map(|&(_, delta, _)| delta)
                .sum::<f32>();
            let top = effective_y + line_top + shift;
            let page = page_origins.page_index_for_y(top + 0.001);
            line_bottom - line_top > page_origins.step_at(page) + 0.001
                || top > page_origins.origin(page) + 0.001
                    && effective_y + line_bottom + shift
                        > page_origins.origin(page.saturating_add(1)) + 0.001
        })
}

/// The moves that start a page with each line after the first that crosses a
/// page end, and with the lines after it, except a line that fits no page.
fn crossing_line_moves(
    line_metrics: &[(f32, f32)],
    effective_y: f32,
    page_origins: &PageOrigins,
) -> Vec<LineBreakMove> {
    let mut shift = 0.0_f32;
    let mut moves = Vec::new();
    for (line, &(line_top, line_bottom)) in line_metrics.iter().enumerate().skip(1) {
        let top = effective_y + line_top + shift;
        let bottom = effective_y + line_bottom + shift;
        let page = page_origins.page_index_for_y(top + 0.001);
        let next = page.saturating_add(1);
        let crosses =
            top > page_origins.origin(page) + 0.001 && bottom > page_origins.origin(next) + 0.001;
        let delta = page_origins.origin(next) - top;
        if crosses
            && line_bottom - line_top <= page_origins.step_at(next) + 0.001
            && delta.is_finite()
            && delta > 0.0
        {
            moves.push((line, delta, next));
            shift += delta;
        }
    }
    moves
}

/// Whether `node_id` is an absolutely positioned box that no fixed box
/// holds. A fixed box repeats on every page, and what it holds does not break.
fn is_unfixed_absolute_box(document: &Document, node_id: usize) -> bool {
    let fixed = |id: usize| {
        document.nodes[id].kind() == NodeKind::Element
            && !document.nodes[id].generates_no_box()
            && document.nodes[id].position() == &PositionValue::Fixed
    };
    document.nodes[node_id].kind() == NodeKind::Element
        && !document.nodes[node_id].generates_no_box()
        && document.nodes[node_id].position() == &PositionValue::Absolute
        && !document.ancestors(node_id).any(fixed)
}

/// A forced page break between two lines of a paragraph, and where the
/// content that follows it starts the next page.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ForcedLineBreak {
    /// The first line after the break.
    line: usize,
    /// How far above that line the content that starts the next page begins:
    /// the border-box top of a box that the break is before.
    lead: f32,
    /// The margin that stays above that content, at the top of the next page.
    gap: f32,
}

/// The forced page breaks that the block-level `::before` and `::after` of
/// the paragraph `root` ask for between its lines, in line order. A
/// `break-*: always` counts only where it breaks pages.
fn forced_line_breaks(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
) -> Vec<ForcedLineBreak> {
    let lines = document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref());
    let mut breaks: Vec<_> = lines
        .into_iter()
        .flat_map(|lines| {
            lines
                .forced_breaks
                .iter()
                // Generated content of a multicol container sits in its
                // columns, where a page value breaks nothing.
                .filter(|forced| {
                    document.nodes[root].multicol.is_none()
                        && page_break_is_forced(document, cascade, root, forced.value)
                })
                .filter_map(move |forced| {
                    let lead = match forced.box_top {
                        Some(top) => lines.lines.get(forced.line)?.block_offset() - top,
                        None => 0.0,
                    };
                    Some(ForcedLineBreak {
                        line: forced.line,
                        lead: lead.max(0.0),
                        gap: forced.gap,
                    })
                })
        })
        .collect();
    merge_forced_line_breaks(&mut breaks);
    breaks
}

/// Puts `breaks` in line order, and the breaks before the same line into one
/// that keeps the larger lead and gap.
fn merge_forced_line_breaks(breaks: &mut Vec<ForcedLineBreak>) {
    breaks.sort_by_key(|forced| forced.line);
    breaks.dedup_by(|later, earlier| {
        let same_line = later.line == earlier.line;
        if same_line {
            earlier.lead = earlier.lead.max(later.lead);
            earlier.gap = earlier.gap.max(later.gap);
        }
        same_line
    });
}

/// The moves that forced page breaks between the lines of a paragraph ask
/// for, in line order: each moves the line after its break, and the lines
/// after that, down so that what the break is before starts the next page,
/// below the margin `gap` it keeps there. `breaks` are as for
/// [`forced_line_breaks`], and `line_metrics` and `effective_y` as for
/// [`widows_break_on_next_page`]. A break that has no line on either side
/// stays at the edge of the paragraph, and a page edge that already falls
/// before what the break is before is the break.
fn forced_line_moves(
    breaks: &[ForcedLineBreak],
    line_metrics: &[(f32, f32)],
    effective_y: f32,
    page_origins: &PageOrigins,
) -> Vec<LineBreakMove> {
    let mut moved = 0.0;
    let mut moves = Vec::new();
    for &ForcedLineBreak { line, lead, gap } in breaks {
        if line == 0 || line >= line_metrics.len() {
            continue;
        }
        let top = |line: usize| effective_y + line_metrics[line].0 + moved;
        let bottom = |line: usize| effective_y + line_metrics[line].1 + moved;
        // Each line belongs to the page that holds its center (see
        // `line_center_on_page`).
        let page_of =
            |line: usize| page_origins.page_index_for_y((top(line) + bottom(line)) * 0.5 + 0.001);
        // What starts the next page is a box above the line when there is
        // one, even if the line itself is already on the next page.
        let starts = top(line) - lead;
        let previous = page_of(line - 1);
        let starts_page = if lead > 0.0 {
            page_origins.page_index_for_y(starts + 0.001)
        } else {
            page_of(line)
        };
        if starts_page > previous {
            continue;
        }
        let page = previous.saturating_add(1);
        let shift = page_origins.origin(page) + gap - starts;
        if shift.is_finite() && shift > 0.0 {
            moves.push((line, shift, page));
            moved += shift;
        }
    }
    moves
}

/// Moves line `line` of the paragraph `root` and every line after it down by
/// `shift`, and grows the root's box by `growth` to hold them.
pub(in crate::layout) fn shift_ifc_lines_from(
    document: &mut Document,
    root: usize,
    line: usize,
    shift: f32,
    growth: f32,
) {
    let node = document.table_layout_node_mut(root);
    if let Some(lines) = node.ifc.as_mut().and_then(|ifc| ifc.lines.as_mut()) {
        lines.shifts.push(line, shift);
        node.layout.size.height += growth;
        document.mark_pagination_extent_dirty(root);
    }
}

/// How far the border box of the paragraph `root` grows when its lines from
/// `shift` on move down by `shift`. A box whose height follows its content
/// grows by the whole shift. A box with a definite height keeps it, and the
/// moved lines overflow it. A `min-height` or `max-height` box grows only
/// as far as its content end passes the bounds (CSS 2.1 §10.7). A flex or
/// grid item stretched to its line or row has room below its content, which
/// lines moving down take first.
pub(in crate::layout) fn lines_shift_growth(
    document: &super::context::LayoutSession<'_>,
    root: usize,
    shift: f32,
) -> f32 {
    let growth = if height_follows_content(document, root) {
        shift
    } else if lines_move_grows_box(document, root) {
        content_growth(document, root, shift).unwrap_or(0.0)
    } else {
        return 0.0;
    };
    let in_flex_or_grid = document
        .taffy_parent_of(root)
        .is_some_and(|parent| document.nodes[parent].display().is_flex_or_grid_container());
    if shift > 0.0 && in_flex_or_grid {
        stretched_content_growth(document, root, shift).map_or(growth, |room| growth.min(room))
    } else {
        growth
    }
}

/// How far the border box of the paragraph `root`, whose height can grow with
/// its content, grows when the content from its lines on moves down by
/// `shift`: `None` for a paragraph without lines.
fn content_growth(
    document: &super::context::LayoutSession<'_>,
    root: usize,
    shift: f32,
) -> Option<f32> {
    let bottom = content_bottom(document, root)?;
    let layout = &document.nodes[root].layout;
    let (height, _) = grown_height(
        layout.size.height,
        border_box_height_bounds(document, root),
        layout.padding.bottom + layout.border.bottom,
        (bottom, bottom + shift),
    );
    Some(height - layout.size.height)
}

/// How far the paragraph `root`, a flex or grid item, has to grow to hold its
/// content when the content from its lines on moves down by `shift`: the
/// room a stretch left below its content takes the move first.
fn stretched_content_growth(
    document: &super::context::LayoutSession<'_>,
    root: usize,
    shift: f32,
) -> Option<f32> {
    let bottom = content_bottom(document, root)?;
    let layout = &document.nodes[root].layout;
    let end = bottom + shift + layout.padding.bottom + layout.border.bottom;
    Some((end - layout.size.height).max(0.0))
}

/// Where the content of the paragraph `root` ends from the top of its border
/// box: its last line, or a block child after it. `None` for a paragraph
/// without lines.
fn content_bottom(document: &super::context::LayoutSession<'_>, root: usize) -> Option<f32> {
    let node = document.ifc_layout_node(root)?;
    let ifc = node.ifc.as_ref()?;
    let lines = ifc.lines.as_ref()?;
    let last = lines.lines.len().checked_sub(1)?;
    let layout = &node.layout;
    // The block children that follow the last line end the content too, and
    // follow the moved lines by the whole shift.
    let blocks_end = ifc
        .boxes
        .iter()
        .filter(|b| b.kind == IfcBoxKind::Block)
        .map(|b| {
            let child = &document.nodes[b.node].layout;
            pagination_child_parent_y(document, root, b.node, document.pagination_local_y(b.node))
                + child.size.height
        })
        .fold(f32::NEG_INFINITY, f32::max);
    Some(
        (layout.padding.top
            + layout.border.top
            + lines.line_top(last)
            + lines.lines[last].block_size())
        .max(blocks_end),
    )
}

/// Block edges of a text node's lines after the paragraph's column ranges
/// have moved them from their unfragmented shaping offsets.
#[cfg(test)]
fn positioned_text_line_bounds(document: &Document, node_id: usize) -> Option<Vec<(f32, f32)>> {
    let owned = document.ifc_text_lines(node_id)?;
    positioned_owned_text_line_bounds(document, &owned)
}

/// Where the text `node_id`, placed by positioned coordinates, starts now.
/// Text of the inline layout rooted at `ifc_root` starts at its first line's
/// current position; other text starts at its own box's top.
fn positioned_text_start_y(
    document: &Document,
    text_lines: &TextLineIndex,
    node_id: usize,
    ifc_root: Option<usize>,
) -> f32 {
    let Some(root) = ifc_root else {
        return current_abs_y(document, node_id);
    };
    let owned = text_lines
        .get(node_id)
        .expect("text candidate has indexed ownership");
    current_abs_y(document, root) + text_first_line_offset(document, root, owned)
}

/// Places the text `node_id` at `y` in the paginated flow. When the text
/// moves its inline layout root, `moved_root` holds that root and the text's
/// offset in it, and the root goes to the matching position. Otherwise a
/// later text of the inline layout rooted at `text_ifc_root`, whose root already
/// moved, sends its lines where the flow puts them and the lines after
/// follow, because an earlier box of the paragraph may have grown.
fn place_text_candidate(
    document: &mut Document,
    text_lines: &TextLineIndex,
    node_id: usize,
    moved_root: Option<(usize, f32)>,
    text_ifc_root: Option<usize>,
    y: f32,
) {
    if let Some((root, offset)) = moved_root {
        materialize_y(document, root, y - offset);
    } else if let Some(root) = text_ifc_root
        && let Some(owned) = text_lines.get(node_id)
    {
        let current = current_abs_y(document, root) + text_first_line_offset(document, root, owned);
        follow_moved_ifc_text(document, root, node_id, y - current, text_lines);
    }
    materialize_y(document, node_id, y);
}

/// Moves a text candidate that `place_text_candidate` already placed down by
/// `delta` when a page boundary before it pushes it to a later page. The text
/// of an inline layout paragraph is painted from its root, so when
/// `moved_root` holds that root, the root carries the movement. Otherwise a
/// later text of the inline layout rooted at `text_ifc_root`, whose root
/// already moved, moves its own lines and the ones after alone. Only the
/// moved box's own position changes; content that follows it is moved by the
/// flow shift the caller applies to later candidates.
fn shift_text_candidate(
    document: &mut Document,
    text_lines: &TextLineIndex,
    node_id: usize,
    moved_root: Option<usize>,
    text_ifc_root: Option<usize>,
    delta: f32,
) {
    let moved = moved_root.unwrap_or(node_id);
    document.translate_pagination_y(moved, delta);
    if moved_root.is_none()
        && let Some(root) = text_ifc_root
    {
        follow_moved_ifc_text(document, root, node_id, delta, text_lines);
    }
}

/// Returns the first owned IFC line top, preferring its positioned bounds,
/// then its indexed line record, and finally zero when neither is available.
fn first_owned_text_line_top(owned: &IfcTextLines, positioned: Option<&[(f32, f32)]>) -> f32 {
    positioned
        .and_then(|lines| lines.first().map(|line| line.0))
        .or_else(|| owned.lines.first().map(|line| line.top))
        .unwrap_or(0.0)
}

fn text_first_line_offset(document: &Document, root: usize, owned: &IfcTextLines) -> f32 {
    let layout = document
        .ifc_layout_node(root)
        .expect("valid paragraph root")
        .layout;
    let positioned = positioned_owned_text_line_bounds(document, owned);
    layout.border.top + layout.padding.top + first_owned_text_line_top(owned, positioned.as_deref())
}

fn positioned_owned_text_line_bounds(
    document: &Document,
    owned: &IfcTextLines,
) -> Option<Vec<(f32, f32)>> {
    let root = document.ifc_layout_node(owned.root)?.ifc.as_ref()?;
    let root_lines = root.lines.as_ref()?;
    let fragments = root.multicol_fragments.as_deref();
    let mut fragment_cursor = 0;
    Some(
        owned
            .lines
            .iter()
            .map(|line| {
                // Both line owners and fragments are ordered by source line.
                let fragment = fragments.and_then(|fragments| {
                    while fragments
                        .get(fragment_cursor)
                        .is_some_and(|fragment| fragment.line_end <= line.line)
                    {
                        fragment_cursor += 1;
                    }
                    fragments.get(fragment_cursor).filter(|fragment| {
                        fragment.line_start <= line.line && line.line < fragment.line_end
                    })
                });
                let offset = fragment
                    .and_then(|fragment| {
                        root_lines
                            .lines
                            .get(fragment.line_start)
                            .map(|first| fragment.y - first.block_offset())
                    })
                    .unwrap_or(0.0);
                let top = root_lines.line_top(line.line)
                    - crate::layout::ifc::outside_marker::fit_leading(
                        document, owned.root, line.line,
                    );
                let height =
                    crate::layout::ifc::outside_marker::fit_height(&root_lines.lines[line.line]);
                (top + offset, top + height + offset)
            })
            .collect(),
    )
}

/// Project an already-paginated document using one fixed geometry for all pages.
///
/// This compatibility entry point remains valid for fixed-page callers. New
/// page-aware callers should use [`page_fragments_from_slices_with_page_geometry`]
/// so each page carries its producer-resolved metadata.
#[cfg(test)]
pub(crate) fn page_fragments_from_slices(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
) -> Vec<PageFragment> {
    let geometries: Vec<_> = slices
        .iter()
        .map(|slice| resolve_page_fragment_geometry(cascade, page_box, slice.page_index))
        .collect();
    page_fragments_from_slices_with_page_geometry(document, cascade, page_box, slices, &geometries)
}

/// The page placements of [`project_slices`].
#[cfg(test)]
pub(crate) fn page_fragments_from_slices_with_page_geometry(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
    page_geometries: &[PageFragmentPageGeometry],
) -> Vec<PageFragment> {
    project_slices(document, cascade, page_box, slices, page_geometries).0
}

/// Project slices using one resolved geometry record for each page, and
/// also return the paragraphs laid out by the inline engine, in document
/// order, at their content-box origins in the shared flow space.
///
/// `page_geometries` is producer-owned resolved metadata. A missing page index
/// falls back to `page_box` and the supplied cascade for compatibility, but a
/// page-aware caller should provide every emitted page explicitly. Item
/// rectangles remain relative to each page's `content_box` origin; consumers
/// add `content_box.x/y` exactly once when placing them on the physical page.
#[cfg(test)]
pub(crate) fn project_slices(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
    page_geometries: &[PageFragmentPageGeometry],
) -> (
    Vec<PageFragment>,
    Vec<ProjectedTextRoot>,
    BTreeMap<NodeId, OverflowClipSource>,
    Vec<PlacementOverflowClipSource>,
) {
    let (pages, roots, clips, placements, _, _) = project_slices_with_control(
        document,
        cascade,
        page_box,
        slices,
        page_geometries,
        &PageLayoutControl::default(),
    )
    .expect("test projection stays within its work cap");
    (pages, roots, clips, placements)
}

/// The part of a box of height `height` whose top is at flow offset `top`
/// that lies on the page spanning `[page_start, page_end)`, as its top
/// relative to the page and its height, or `None` when the box misses it.
///
/// The paginator moves a box to a page origin, but its offset is read back
/// as a sum over its ancestors, which can round one ulp either way. Past
/// 8192px one ulp is 2^-10 px, so an overlap of one ulp at either edge of a
/// box that crosses it would leave a sliver of the box on the other page.
/// Such an overlap is rounding, as for the paginator's page ends: the page
/// that keeps the box also keeps the part in the band, so no part of the box
/// is lost. A box that lies within one ulp of both sides of an edge stays
/// whole on the page it starts on, so that it is never dropped from both.
fn flow_page_overlap(top: f32, height: f32, page_start: f32, page_end: f32) -> Option<(f32, f32)> {
    let bottom = top + height;
    let starts_after =
        top >= page_end || (bottom.next_down() > page_end && top.next_up() >= page_end);
    let ends_before =
        bottom <= page_start || (top < page_start && bottom.next_down() <= page_start);
    if starts_after || ends_before {
        return None;
    }
    let visible_top = if top.next_up() >= page_start {
        top
    } else {
        page_start
    };
    let visible_bottom = if bottom.next_down() <= page_end {
        bottom
    } else {
        page_end
    };
    Some((
        (visible_top - page_start).max(0.0),
        (visible_bottom - visible_top).max(0.0),
    ))
}

fn build_page_fragments(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
    page_geometries: &[PageFragmentPageGeometry],
) -> (Vec<PageFragment>, FlowAxis) {
    let fallback_geometry = resolve_page_fragment_geometry(cascade, page_box, 0);
    let mut ordered_slices: Vec<&PageSlice> = slices.iter().collect();
    ordered_slices.sort_by(|left, right| {
        left.page_index
            .cmp(&right.page_index)
            .then_with(|| left.content_origin_y.total_cmp(&right.content_origin_y))
    });
    // Stable sorting retains the first input geometry for duplicate indices.
    // Both streams then advance only forward, including for sparse indices.
    let mut ordered_geometries: Vec<_> = page_geometries.iter().collect();
    ordered_geometries.sort_by(|left, right| {
        #[cfg(test)]
        tests::GEOMETRY_COMPARISONS.with(|count| count.set(count.get() + 1));
        left.page_index.cmp(&right.page_index)
    });
    let mut remaining_geometries = ordered_geometries.as_slice();
    let mut pages: Vec<PageFragment> = ordered_slices
        .iter()
        .map(|slice| {
            while let Some(geometry) = remaining_geometries.first() {
                #[cfg(test)]
                tests::GEOMETRY_COMPARISONS.with(|count| count.set(count.get() + 1));
                if geometry.page_index >= slice.page_index {
                    break;
                }
                remaining_geometries = &remaining_geometries[1..];
            }
            let geometry = remaining_geometries
                .first()
                .filter(|geometry| {
                    #[cfg(test)]
                    tests::GEOMETRY_COMPARISONS.with(|count| count.set(count.get() + 1));
                    geometry.page_index == slice.page_index
                })
                .copied()
                .copied()
                .unwrap_or_else(|| fallback_geometry.with_page_index(slice.page_index));
            PageFragment::with_page_geometry(
                slice.page_index,
                geometry,
                slice.content_origin_y,
                slice.page_name.clone(),
            )
        })
        .collect();

    // Pages of a root whose blocks stack on vertical axes cut the flow along
    // x; every placement below is made in flow space, where the flow runs
    // down, and turned back to layout space on its page.
    let flow_axis =
        pages_follow_x_axis(document, cascade).map_or(FlowAxis::Y, |right_to_left| FlowAxis::X {
            right_to_left,
            width: pages.first().map_or(0.0, |page| page.content_box.width),
            page_width: 0.0,
        });

    // Each page's slice of the shared flow space: from its origin to the next
    // page's origin, or its content extent along the flow for the last page.
    for (page_slot, page) in pages.iter_mut().enumerate() {
        page.flow_axis = match flow_axis {
            FlowAxis::X {
                right_to_left,
                width,
                ..
            } => FlowAxis::X {
                right_to_left,
                width,
                page_width: page.content_box.width,
            },
            FlowAxis::Y => FlowAxis::Y,
        };
        let page_start = page.content_origin_y;
        let extent = match flow_axis {
            FlowAxis::Y => page.content_box.height,
            FlowAxis::X { .. } => page.content_box.width,
        };
        let page_end = ordered_slices
            .get(page_slot + 1)
            .map(|next| next.content_origin_y)
            .filter(|next| next.is_finite() && *next > page_start)
            .unwrap_or(page_start + extent);
        page.flow_range = (page_start.is_finite() && page_end.is_finite() && page_end > page_start)
            .then_some((page_start, page_end));
    }

    (pages, flow_axis)
}

pub(crate) fn project_slices_with_control(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
    page_geometries: &[PageFragmentPageGeometry],
    control: &PageLayoutControl<'_>,
) -> Result<ProjectedSlices, LayoutError> {
    let (mut pages, flow_axis) =
        build_page_fragments(document, cascade, page_box, slices, page_geometries);

    let mut work = ProjectionWork::new(
        control,
        document.fragment_tree.work_limit(document.nodes.len()),
    );
    work.check()?;
    let text_roots = Vec::new();
    // A root with `display: none` generates no boxes, so its pages carry no
    // fragments, text, or links.
    let root_generates_box = BoxPropertiesRef::from_computed(cascade.root_element_computed())
        .display()
        != DisplayValue::None;
    let Some(body_id) = find_body(document).filter(|_| root_generates_box) else {
        return Ok((
            pages,
            text_roots,
            BTreeMap::new(),
            Vec::new(),
            ParagraphCache::new(),
            work.remaining(),
        ));
    };

    let WalkedSources {
        mut nodes,
        mut text_roots,
        placement_overflow_clips,
        paragraph_cache,
        repeated_placement_nodes,
    } = walk_sources(document, cascade, body_id, &mut work)?;
    keep_painted_shifts_after_flow_start(&pages, flow_axis, &mut nodes, &mut text_roots);
    restore_crossing_tops(document, &mut nodes, &pages);

    place_sources(document, &nodes, &mut pages, flow_axis, &mut work)?;

    drop_uncrossed_body(document, &mut pages, body_id);
    number_fragments(document, &mut pages);

    let clip_sources = overflow_clip_sources(document, cascade, &nodes, &repeated_placement_nodes);

    Ok((
        pages,
        text_roots,
        clip_sources,
        placement_overflow_clips,
        paragraph_cache,
        work.remaining(),
    ))
}

/// Limits the painted shifts of `nodes` and `text_roots` so that a box the
/// painter moves up stops at the start of the flow. Nothing comes before the
/// first page, so a box moved above it stays on that page, as layout placed
/// it, and overflows its top.
fn keep_painted_shifts_after_flow_start(
    pages: &[PageFragment],
    flow_axis: FlowAxis,
    nodes: &mut [PageFragmentSource],
    text_roots: &mut [ProjectedTextRoot],
) {
    let Some(start) = pages
        .iter()
        .filter_map(|page| page.flow_range.map(|(start, _)| start))
        .reduce(f32::min)
        .filter(|_| flow_axis == FlowAxis::Y)
    else {
        return;
    };
    let limit = |shift: f32, y: f32| shift.max(start.min(y) - y);
    for node in nodes {
        node.painted_shift = limit(node.painted_shift, node.abs_y);
    }
    for root in text_roots {
        root.painted_shift = limit(root.painted_shift, root.y);
    }
}

/// Group page snapshots into a deterministic NodeId-ordered geometry table.
///
/// `pages` is normally the ordered snapshot returned by
/// [`layout_page_fragments`] or [`page_fragments_from_slices`]. The result
/// normalizes each item to its containing page and sorts node fragments by
/// page/fragment index, while preserving the producer's `is_repeat` flag.
#[cfg(test)]
pub(crate) fn page_fragment_geometry_table(pages: &[PageFragment]) -> PageFragmentGeometryTable {
    let mut table = PageFragmentGeometryTable::new();
    for page in pages {
        for item in &page.items {
            // The page snapshot is authoritative for this placement. Normalize
            // manually assembled snapshots as well as producer output so the
            // node-centric table always retains the page index required by a
            // fulgur-compatible consumer.
            let item = item.clone().with_page_index(page.page_index);
            let geometry = table
                .entry(item.node_id)
                .or_insert_with(|| PageFragmentGeometry::new(item.node_id, item.is_repeat));
            debug_assert_eq!(geometry.is_repeat, item.is_repeat); // cov:ignore: one producer cannot mix split and repeat records.
            geometry.fragments.push(item);
        }
    }
    for geometry in table.values_mut() {
        geometry
            .fragments
            .sort_by_key(|item| (item.page_index, item.fragment_index));
    }
    table
}

fn line_range_for_page(
    line_metrics: &[(f32, f32)],
    text_abs_y: f32,
    page_start: f32,
    page_end: f32,
) -> Option<PageFragmentLineRange> {
    let mut first = None;
    let mut end = 0_u32;
    for (index, (line_top, line_bottom)) in line_metrics.iter().copied().enumerate() {
        #[cfg(test)]
        tests::LINE_CENTER_EVALUATIONS.with(|count| count.set(count.get() + 1));
        let top = text_abs_y + line_top;
        let bottom = text_abs_y + line_bottom;
        if !line_center_on_page(top, bottom, page_start, page_end) {
            continue;
        }
        let index = index as u32;
        first.get_or_insert(index);
        end = index.saturating_add(1);
    }
    first.map(|start| PageFragmentLineRange::new(start, end))
}

/// Whether a line from `top` to `bottom` in the shared flow space belongs to
/// the page whose slice runs from `page_start` to `page_end`: the page that
/// holds the line's center. A line therefore belongs to exactly one page.
pub(crate) fn line_center_on_page(top: f32, bottom: f32, page_start: f32, page_end: f32) -> bool {
    let center = (top + bottom) * 0.5;
    center.is_finite() && center >= page_start - 0.001 && center < page_end - 0.001
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

/// Give the box after each forced page break clearance past the floats
/// before it.
///
/// The box starts a new page, which the floats placed before the break do not
/// reach. Layout places the whole flow in one pass and pagination then moves
/// the box down to its page, so without clearance the earlier floats would
/// still narrow the box and push its own floats aside on the new page
/// (CSS Fragmentation 3, 3.1: content after a forced break goes to the next
/// fragmentainer). A change of named page between siblings forces a break
/// too (CSS Paged Media 3, 4.1).
fn clear_floats_at_forced_breaks<'name>(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &'name CascadeResult,
) {
    let has_named_pages = super::page::has_named_pages(cascade);
    let pagination_on_transposed_axes = document.context.pagination_on_transposed_axes;
    let in_flow = |document: &Document, id: usize| {
        let node = &document.nodes[id];
        node.kind() == NodeKind::Element
            && node.is_in_document()
            && !node.is_unrendered_html_element()
            && !matches!(
                document.nodes[id].display(),
                DisplayValue::None | DisplayValue::Contents
            )
            && !matches!(
                document.nodes[id].position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
    };
    let is_float =
        |document: &Document, id: usize| is_floating_box_for_pagination(document, cascade, id);
    // The in-flow and floating child boxes of `parent`, and its nonempty text
    // as `None`, with the children of a `display: contents` element standing
    // in its place. With `static_absolute`, the absolutely positioned boxes
    // left at their static position are among them too.
    let child_boxes = |document: &Document, parent: usize, static_absolute: bool| {
        let mut boxes = Vec::new();
        // `None` stands for the `::before` content of a flattened element.
        // The children are taken in the order pagination visits them, which
        // follows `order` in a flex or grid container.
        let mut pending: Vec<Option<usize>> = pagination_child_order(document, cascade, parent)
            .into_iter()
            .rev()
            .map(Some)
            .collect();
        while let Some(item) = pending.pop() {
            let Some(child) = item else {
                boxes.push(None);
                continue;
            };
            let node = &document.nodes[child];
            if node.kind() == NodeKind::Text {
                if node.is_in_document()
                    && super::page::text_is_page_content(
                        BoxTree::for_layout(document),
                        cascade,
                        child,
                    )
                {
                    boxes.push(None);
                }
            } else if node.kind() == NodeKind::Element
                && node.is_in_document()
                && !node.is_unrendered_html_element()
                && (document.nodes[child].display() == DisplayValue::Contents
                    || document.nodes[child].display().is_inline_flow()
                        && in_flow(document, child)
                        && !is_float(document, child)
                        && !is_monolithic_for_pagination_with_transposed_axes(
                            document,
                            cascade,
                            child,
                            pagination_on_transposed_axes,
                        ))
            {
                // An inline box is content of its parent's line, so its own
                // break properties do not apply, but pagination still breaks
                // at the blocks inside it. One without children still makes
                // a line of its own.
                if node.children.is_empty() && document.nodes[child].display().is_inline_flow() {
                    boxes.push(None);
                }
                pending.extend(node.children.iter().rev().map(|&child| Some(child)));
                if crate::generated_content::is_in_flow_generated_text(
                    cascade,
                    child,
                    raikiri_style::PseudoElem::Before,
                ) {
                    pending.push(None);
                }
            } else if static_absolute
                && node.kind() == NodeKind::Element
                && node.is_in_document()
                && !node.is_unrendered_html_element()
                && document.nodes[child].display() != DisplayValue::None
                && is_static_position_absolute(document, cascade, child)
            {
                boxes.push(Some(child));
            } else if in_flow(document, child) {
                // An atomic inline box, a replaced element among them, is
                // content of its parent's line, which pagination does not
                // break around or inside.
                let inline_level =
                    document.nodes[child].display().is_inline_level() && !is_float(document, child);
                boxes.push((!inline_level).then_some(child));
            }
        }
        boxes
    };
    // The float that a forced break at the end of the content of `id` sends
    // to the next page, when only floats follow that break inside it: it
    // starts that page, as a float right after a forced break among the
    // boxes after `id` would. A float that a break further in sends there
    // starts the page in its place, and the floats after it line up beside
    // it. A box that holds its floats, or that ends with generated content,
    // has none. `memo` keeps each box's result, so nested boxes are each
    // looked at once.
    let trailing_page_start_float = |document: &super::context::LayoutSession<'_>,
                                     memo: &mut HashMap<usize, Option<usize>>,
                                     mut id: usize| {
        // The boxes walked into, each with the float after its last block,
        // if any.
        let mut chain = Vec::new();
        let mut found = None;
        for _ in 0..128 {
            if let Some(&known) = memo.get(&id) {
                found = known;
                break;
            }
            let boxes = if is_monolithic_for_pagination_with_transposed_axes(
                document,
                cascade,
                id,
                pagination_on_transposed_axes,
            ) || holds_floats_in_preparation(document, cascade, id)
                || renders_edge_pseudo(cascade, id, false)
            {
                Vec::new()
            } else {
                child_boxes(document, id, false)
            };
            let last = boxes
                .iter()
                .rposition(|item| item.is_none_or(|item| !is_float(document, item)));
            let Some(Some(block)) = last.map(|last| boxes[last]) else {
                memo.insert(id, None);
                break;
            };
            let float = last.and_then(|last| boxes.get(last + 1).copied().flatten());
            if float.is_some() && !page_edge_constraint(document, cascade, block, false).0 {
                memo.insert(id, None);
                break;
            }
            chain.push((id, float));
            id = block;
        }
        for (id, float) in chain.into_iter().rev() {
            found = found.or(float);
            memo.insert(id, found);
        }
        found
    };
    // The page value a box ends with: that of its last in-flow content, down
    // to the deepest box, or `inherited` without one. A monolithic box does
    // not pass up a descendant's. `trailing` keeps each box's own result, so
    // nested boxes are each looked at once.
    let end_page_name = |document: &Document,
                         trailing: &mut HashMap<usize, Option<&'name str>>,
                         id: usize,
                         inherited: Option<&'name str>| {
        if !has_named_pages {
            return inherited;
        }
        let mut chain = Vec::new();
        let mut id = id;
        let mut name = loop {
            if let Some(name) = trailing.get(&id) {
                break *name;
            }
            chain.push(id);
            if is_monolithic_for_pagination_with_transposed_axes(
                document,
                cascade,
                id,
                pagination_on_transposed_axes,
            ) {
                break None;
            }
            let last = child_boxes(document, id, false)
                .into_iter()
                .rev()
                .find(|child| child.is_none_or(|child| !is_float(document, child)));
            match last {
                Some(Some(child)) => id = child,
                _ => break None,
            }
        };
        for &id in chain.iter().rev() {
            if page_applies_to_box(document, id) {
                name = name.or_else(|| selected_page_name(cascade, id));
            }
            trailing.insert(id, name);
            #[cfg(test)]
            super::page::record_page_name_work(3);
        }
        name.or(inherited)
    };
    let mut trailing = HashMap::new();
    // Whether a box with the page value `name` after content that ended with
    // `previous` starts a new page.
    let changes_page = |previous: &Option<Option<&str>>, name: &Option<&str>| {
        previous.as_ref().is_some_and(|previous| previous != name)
    };
    // Walk the boxes with the page value each inherits. Floats are no part of
    // the page flow, and their insides are not walked.
    let mut cleared = Vec::new();
    document.context.float_resumption_candidates.clear();
    document.context.float_break_closers.clear();
    let mut cleared_lines = Vec::new();
    let mut cleared_to_float_floor = Vec::new();
    let mut cleared_lines_to_float_floor = Vec::new();
    let mut trailing_floats = HashMap::new();
    let mut pending = vec![(0_usize, None::<&str>, false)];
    while let Some((parent, inherited, at_page_start_float)) = pending.pop() {
        let mut after_forced_break = false;
        // Generated prefixes are not arena children. Before content and
        // inside markers take the parent's page before its child boxes.
        let generated_prefix =
            crate::generated_content::has_page_flow_generated_prefix(cascade, parent);
        // Whether the box that comes first among the parent's boxes starts
        // right at the float that began the page: the parent took the forced
        // break, so a break before that box, which CSS Fragmentation 3 (3.1)
        // propagates up to the parent's start edge, is the same break. Generated
        // content of the parent comes before that box, so it is a break of its
        // own then.
        let mut page_start_inherited = at_page_start_float && !generated_prefix;
        let mut float_after_break = false;
        let mut page_start_float = None;
        // Whether the float that started the page took its own break-before
        // or a change of named page there, and not just the break of the box
        // before it.
        let mut float_own_break = false;
        // The last in-flow block among the parent's boxes: when the parent
        // is a paragraph of the inline engine, the lines after it are laid
        // out from its bottom.
        let mut last_block = None;
        // Whether that block ends with a float that took the forced break
        // at its end and started the new page.
        let mut block_ends_with_page_start_float = false;
        let mut previous_page_name = generated_prefix.then_some(inherited);
        for item in child_boxes(document, parent, true) {
            let Some(child) = item else {
                // Text takes the parent's page and starts the new page itself.
                // Its lines have no element to give clearance to, so the
                // paragraph starts them below the earlier floats after the
                // block that ends right before the break.
                if let Some(block) = last_block.take()
                    && (after_forced_break || changes_page(&previous_page_name, &inherited))
                {
                    if std::mem::take(&mut block_ends_with_page_start_float) {
                        cleared_lines_to_float_floor.push(block);
                    } else {
                        cleared_lines.push(block);
                    }
                }
                block_ends_with_page_start_float = false;
                after_forced_break = false;
                float_after_break = false;
                float_own_break = false;
                page_start_float = None;
                page_start_inherited = false;
                previous_page_name = Some(inherited);
                continue;
            };
            if is_static_position_absolute(document, cascade, child) {
                if !has_named_pages {
                    continue;
                }
                // An absolutely positioned box left at its static position
                // keeps the named pages of its in-flow content in the page
                // flow, as pagination does, while its own `page` value and
                // its unnamed content take no part. Its content is in a
                // formatting context of its own, so only the boxes after it
                // start below the earlier floats. A monolithic box passes no
                // page name on.
                let last = (!is_monolithic_for_pagination_with_transposed_axes(
                    document,
                    cascade,
                    child,
                    pagination_on_transposed_axes,
                ))
                .then(|| {
                    child_boxes(document, child, false)
                        .into_iter()
                        .rev()
                        .find(|item| item.is_none_or(|item| !is_float(document, item)))
                })
                .flatten()
                .flatten();
                if let Some(last) = last {
                    let name = end_page_name(document, &mut trailing, last, inherited);
                    if name != inherited {
                        previous_page_name = Some(name);
                    }
                }
                continue;
            }
            if is_float(document, child) {
                let carries_flow_break = after_forced_break && !float_after_break;
                page_start_inherited = false;
                // The first float right after a forced break goes to the new
                // page, so it clears the earlier page's floats too; the floats
                // after it line up beside it. The break still carries on to
                // the next in-flow box, which pagination would otherwise leave
                // on the earlier page. A float's own forced break before it
                // moves it and the boxes after it to the next page the same
                // way. Its break after it is ignored. A float is no part of the
                // page flow (CSS Page 3, section 4.1): its page name changes no
                // page and is not the page before the box after it, which is
                // compared with the box before the float, or names the first
                // page when there is none.
                if page_break_is_forced(
                    document,
                    cascade,
                    child,
                    cascade.computed[child].break_before,
                ) {
                    after_forced_break = true;
                    float_after_break = false;
                    float_own_break = true;
                    // The page change is at the float, not right after the
                    // block before it, so the lines after that block stay where
                    // they are.
                    last_block = None;
                }
                if after_forced_break && !float_after_break {
                    document.record_pass_override(
                        child,
                        raikiri_layout_style::OwnedPropertyOverride::clear(ClearValue::Both),
                    );
                    cleared.push(child);
                    float_after_break = true;
                    if carries_flow_break {
                        page_start_float = Some(child);
                    }
                }
                continue;
            }
            // A float right after the break started the new page, so the box
            // after it lines up beside that float instead of below it.
            let starts_at_inherited = std::mem::take(&mut page_start_inherited);
            let beside_page_start_float =
                std::mem::take(&mut float_after_break) || starts_at_inherited;
            let float_took_own_break = std::mem::take(&mut float_own_break);
            let resumed_float = page_start_float.take();
            last_block = Some(child);
            let start_page_name = if has_named_pages {
                match propagated_start_page_name_with_transposed_axes(
                    document,
                    cascade,
                    child,
                    inherited,
                    pagination_on_transposed_axes,
                ) {
                    (true, name) => name,
                    (false, _) => inherited, // cov:ignore: child_boxes only yields visible boxes with propagated page names.
                }
            } else {
                None
            };
            let named_break = changes_page(&previous_page_name, &start_page_name);
            let breaks_before = page_break_is_forced(
                document,
                cascade,
                child,
                cascade.computed[child].break_before,
            );
            let own_break = named_break || breaks_before;
            let forced = after_forced_break || own_break;
            // Floats on either side belong to the earlier page, so a
            // one-sided clear is widened too. A break before the box
            // coincides with the break of the box before the float that
            // started the page, but not with the float's own break.
            let at_page_start_float = (forced || starts_at_inherited)
                && beside_page_start_float
                && !named_break
                && !(breaks_before && float_took_own_break)
                && !document.nodes[parent].used_writing_mode.is_vertical();
            // A break after the last box inside this one is carried out to
            // the next box, as the paginator does.
            after_forced_break = page_edge_constraint(document, cascade, child, false).0;
            // A float after that break inside this box started the new page:
            // the boxes after this one line up beside it, as beside a float
            // right after a break among them, and the lines after it start
            // below the floats before that float only.
            block_ends_with_page_start_float = false;
            if after_forced_break
                && !document.nodes[parent].used_writing_mode.is_vertical()
                && let Some(float) =
                    trailing_page_start_float(document, &mut trailing_floats, child)
            {
                float_after_break = true;
                page_start_float = Some(float);
                block_ends_with_page_start_float = true;
                // The parents are walked outside in, so the first box found
                // for a float is the outermost one.
                document
                    .context
                    .float_break_closers
                    .entry(float)
                    .or_insert(child);
            }
            previous_page_name = Some(end_page_name(document, &mut trailing, child, inherited));
            // child_boxes represents inline content as None; floats and static-position
            // absolute boxes have already been handled above.
            let own_page_name = selected_page_name(cascade, child).or(inherited);
            // Pagination does not break inside a monolithic box.
            if !is_monolithic_for_pagination_with_transposed_axes(
                document,
                cascade,
                child,
                pagination_on_transposed_axes,
            ) {
                pending.push((child, own_page_name, at_page_start_float));
            }
            if forced && at_page_start_float {
                // Layout clears the floats before the one that starts the
                // page, not that float.
                cleared_to_float_floor.push(child);
                if let Some(float) = resumed_float {
                    document
                        .context
                        .float_resumption_candidates
                        .push((float, child));
                }
            } else if forced {
                document.record_pass_override(
                    child,
                    raikiri_layout_style::OwnedPropertyOverride::clear(ClearValue::Both),
                );
                cleared.push(child);
            }
        }
        // Generated `::after` content ends the parent's lines like text.
        if let Some(block) = last_block
            && (after_forced_break || changes_page(&previous_page_name, &inherited))
            && crate::generated_content::is_in_flow_generated_text(
                cascade,
                parent,
                raikiri_style::PseudoElem::After,
            )
        {
            if block_ends_with_page_start_float {
                cleared_lines_to_float_floor.push(block);
            } else {
                cleared_lines.push(block);
            }
        }
    }
    cleared.sort_unstable();
    cleared_lines.sort_unstable();
    cleared_to_float_floor.sort_unstable();
    cleared_lines_to_float_floor.sort_unstable();
    if cleared != document.forced_break_clearance
        || cleared_lines != document.forced_break_line_clearance
        || cleared_to_float_floor != document.forced_break_float_floor
        || cleared_lines_to_float_floor != document.forced_break_line_float_floor
    {
        document.forced_break_clearance = cleared;
        document.forced_break_line_clearance = cleared_lines;
        document.forced_break_float_floor = cleared_to_float_floor;
        document.forced_break_line_float_floor = cleared_lines_to_float_floor;
        document.layout_dirty = true;
    }
}

/// Whether `value`, the `break-after` of the box `node_id`, forces a page
/// break after it, before any float rule. A spanner's `break-after` sits
/// between it and what follows: when that is column content, the break
/// falls in the columns, so Chromium ignores a page value there. It breaks
/// the page before another spanner, or after the last spanner of the
/// multicol container where the container's own edge follows. A spanner
/// inside another box, such as an enclosing spanner, has no column content
/// after it within that box.
pub(super) fn page_break_after_is_forced(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    value: BreakBetween,
) -> bool {
    if !page_break_is_forced(document, cascade, node_id, value) {
        return false;
    }
    // `column-span: all` outside a multicol container spans nothing.
    if !super::is_spanner_in(document, cascade, node_id)
        || !document
            .layout_ancestors(node_id)
            .any(|id| document.nodes[id].multicol.is_some())
    {
        return true;
    }
    // Source order: column text after a spanner in an inline formatting
    // context is no layout child of its own. A wrapper that generates no box
    // passes the edge on to its parent, and the first content inside one
    // after the spanner is what follows it. Inside a box of its own, such as
    // an enclosing spanner, what follows is no column content, so the page
    // value breaks the page; at the end of such a box the break passes on to
    // the box's own edge. A plain wrapper that layout leaves out ends in the
    // columns.
    let mut current = node_id;
    loop {
        let parent = document.parent_of(current);
        // Generated content after the last child is not among the children
        // but follows the spanner too.
        let next = parent.and_then(|parent| {
            first_flow_child(document, cascade, parent, Some(current))
                .map(Some)
                .or_else(|| generated_on_side(cascade, parent, false).then_some(None))
                .map(|next| (parent, next))
        });
        if let Some((parent, next)) = next {
            return !lays_out_in_columns(document, parent)
                || next.is_some_and(|next| starts_with_spanner(document, cascade, next));
        }
        match parent {
            Some(parent)
                if cascade.computed[parent].display == DisplayValue::Contents
                    || (document.nodes[parent].multicol.is_none()
                        && document.nodes[parent].display() != DisplayValue::Contents) =>
            {
                current = parent
            }
            _ => return parent.is_none_or(|parent| document.nodes[parent].multicol.is_some()),
        }
    }
}

/// The spanner right before the spanner `node_id` among the children of
/// their parent, with nothing that the flow lays out between the two. The
/// lines of a paragraph keep no box before a spanner among them, so a break
/// after the first one is found from here.
fn spanner_before(document: &Document, cascade: &CascadeResult, node_id: usize) -> Option<usize> {
    if !super::is_spanner_in(document, cascade, node_id) {
        return None;
    }
    let parent = document.parent_of(node_id)?;
    let children = &document.nodes[parent].children;
    let index = children.iter().position(|&child| child == node_id)?;
    let previous = children[..index]
        .iter()
        .rev()
        .copied()
        .find(|&child| lays_out_in_flow(document, cascade, child))?;
    super::is_spanner_in(document, cascade, previous).then_some(previous)
}

/// Whether the children of `id` are laid out in the columns of a multicol
/// container: `id` is that container, or a box that layout leaves out on the
/// way up to it.
fn lays_out_in_columns(document: &Document, id: usize) -> bool {
    document
        .inclusive_layout_ancestors(id)
        .find(|&id| {
            let node = &document.nodes[id];
            node.multicol.is_some() || node.display() != DisplayValue::Contents
        })
        .is_some_and(|id| document.nodes[id].multicol.is_some())
}

/// The first child of `parent`, after `after` when it is given, that the
/// flow lays out (see [`lays_out_in_flow`]).
fn first_flow_child(
    document: &Document,
    cascade: &CascadeResult,
    parent: usize,
    after: Option<usize>,
) -> Option<usize> {
    let children = &document.nodes[parent].children;
    let start = after.map_or(0, |after| {
        children
            .iter()
            .position(|&child| child == after)
            .map_or(children.len(), |index| index + 1)
    });
    children[start..]
        .iter()
        .copied()
        .find(|&child| lays_out_in_flow(document, cascade, child))
}

/// Whether the flow lays out `child`: it is neither hidden nor out of flow,
/// nor text of white space that its style collapses away, nor a wrapper that
/// generates no box and holds no such child or generated content.
fn lays_out_in_flow(document: &Document, cascade: &CascadeResult, child: usize) -> bool {
    let node = &document.nodes[child];
    node.is_in_document()
        && node.display() != DisplayValue::None
        && !matches!(
            node.position(),
            PositionValue::Absolute | PositionValue::Fixed
        )
        && match &node.data {
            crate::node::NodeData::Text(text) => {
                !crate::generated_content::is_collapsed_white_space(
                    &text.text_content,
                    cascade.computed[child].effective_white_space_collapse,
                )
            }
            _ => true,
        }
        && (cascade.computed[child].display != DisplayValue::Contents
            || generated_on_side(cascade, child, true)
            || generated_on_side(cascade, child, false)
            || first_flow_child(document, cascade, child, None).is_some())
}

/// Whether the content that starts at `node`, looking into wrappers that
/// generate no box, is a spanner.
fn starts_with_spanner(document: &Document, cascade: &CascadeResult, node: usize) -> bool {
    super::is_spanner_in(document, cascade, node)
        || (cascade.computed[node].display == DisplayValue::Contents
            && !generated_on_side(cascade, node, true)
            && first_flow_child(document, cascade, node, None)
                .is_some_and(|first| starts_with_spanner(document, cascade, first)))
}

/// Whether `value`, the `break-after` of the box `node_id`, forces a page
/// break after it. A float has no break after it: Chromium keeps the content
/// after a float with it whatever the float names. The `break-before` of a
/// float still applies, as the reftests for floats under CSS2 pagination
/// (`float-page-break-inside-avoid-5-print.html` and its siblings) expect.
fn box_forces_break_after(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    value: BreakBetween,
) -> bool {
    page_break_after_is_forced(document, cascade, node_id, value)
        && (matches!(cascade.computed[node_id].float, FloatValue::None)
            || !is_floating_box_for_pagination(document, cascade, node_id))
}

pub(super) fn page_break_is_forced(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    value: BreakBetween,
) -> bool {
    if !value.may_force_page_break() {
        return false;
    }
    // A break inside a multicol container falls in its column context.
    // Chromium also ignores a page value there when it prints: the content
    // after it stays in the columns and on the page. A spanner leaves the
    // columns of its own container, and so does everything inside it, so
    // only a container above that one turns the page value into a column
    // break. A float or an out-of-flow box spans nothing, and neither does
    // a spanner with a box between it and the container, such as a
    // `flow-root`: only boxes that layout leaves out, `display: contents` and
    // the plain wrappers it lays out like it, let a spanner reach the columns.
    let mut spanned = super::is_spanner_in(document, cascade, node_id);
    for id in document.layout_ancestors(node_id) {
        let node = &document.nodes[id];
        if node.multicol.is_some() {
            if !spanned {
                return false;
            }
            spanned = false;
        } else if node.display() != DisplayValue::Contents {
            spanned = false;
        }
        spanned |= super::is_spanner_in(document, cascade, id);
    }
    true
}

/// Whether `node_id` starts (`before`) or ends the block flow of the table
/// cell around it: each box from it up to the cell is the first or last box
/// of the next one up, which stacks its children in block flow. Floats,
/// positioned boxes and empty `display: contents` boxes beside it do not
/// count, as for [`page_edge_child`], and generated content does. The cell
/// must sit in a row of its own element, which takes the break, and must not
/// be a box that pagination does not break through, such as a cell in an
/// orthogonal writing mode.
///
/// A forced break there lies at the edge of the cell, not inside it, so it
/// goes to the row (CSS Fragmentation 3, 3.1): see [`row_takes_cell_break`].
fn at_table_cell_edge(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    before: bool,
) -> bool {
    if !is_block_level_flow_box(document.nodes[node_id].display()) {
        return false;
    }
    let mut id = node_id;
    loop {
        let parent = document.ancestors(id).find(|&ancestor| {
            document.nodes[ancestor].display() != DisplayValue::Contents
                || document.nodes[ancestor].has_before_or_after_content
        });
        let Some(parent) = parent.filter(|&parent| {
            page_edge_child(document, cascade, parent, before, 0) == Ok(Some(id))
        }) else {
            return false;
        };
        if document.nodes[parent].display() == DisplayValue::TableCell {
            return !generated_on_side(cascade, parent, before)
                && !is_monolithic_for_pagination_with_transposed_axes(
                    document, cascade, parent, false,
                )
                && document
                    .parent_of(parent)
                    .is_some_and(|row| document.nodes[row].display() == DisplayValue::TableRow);
        }
        if !passes_edge_breaks(document, cascade, parent, before) {
            return false;
        }
        id = parent;
    }
}

/// Whether a forced break at the start (`before`) or the end of the block
/// flow of one of the cells of `row` passes to the row, which then breaks
/// before or after itself (see [`at_table_cell_edge`]).
fn row_takes_cell_break(
    document: &Document,
    cascade: &CascadeResult,
    row: usize,
    before: bool,
) -> bool {
    document.nodes[row].display() == DisplayValue::TableRow
        && document.nodes[row].children.iter().any(|&cell| {
            if document.nodes[cell].display() != DisplayValue::TableCell
                || generated_on_side(cascade, cell, before)
                || is_monolithic_for_pagination_with_transposed_axes(document, cascade, cell, false)
            {
                return false;
            }
            let mut edge = page_edge_child(document, cascade, cell, before, 0);
            while let Ok(Some(id)) = edge {
                let node = &document.nodes[id];
                if node.kind() != NodeKind::Element || !is_block_level_flow_box(node.display()) {
                    return false;
                }
                let computed = &cascade.computed[id];
                if before
                    && (page_break_is_forced(document, cascade, id, computed.break_before)
                        || starts_with_forced_spanner(document, cascade, id))
                    || !before
                        && box_forces_break_after(document, cascade, id, computed.break_after)
                {
                    return true;
                }
                if !passes_edge_breaks(document, cascade, id, before) {
                    return false;
                }
                edge = page_edge_child(document, cascade, id, before, 0);
            }
            false
        })
}

/// Whether `node_id` forces a page break before (`before`) or after itself
/// in the page flow: its own forced break, unless it lies at the edge of a
/// table cell, or one that a row takes from the edge of one of its cells.
fn forces_page_break(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    before: bool,
) -> bool {
    let computed = &cascade.computed[node_id];
    let own = if before {
        page_break_is_forced(document, cascade, node_id, computed.break_before)
            || starts_with_forced_spanner(document, cascade, node_id)
    } else {
        box_forces_break_after(document, cascade, node_id, computed.break_after)
    };
    // Break properties apply to boxes in the flow: an absolutely or fixed
    // positioned box breaks no page before or after itself (Chromium).
    (own && !at_table_cell_edge(document, cascade, node_id, before)
        || row_takes_cell_break(document, cascade, node_id, before))
        && !is_out_of_flow_positioned(document, node_id)
}

/// Whether `node_id` is a multicol container whose first box is a spanner
/// that forces a page break before itself. No column content comes before
/// that break, so it falls before the container, which moves to the next
/// page whole (Chromium). `page_edge_child` looks through boxes that
/// generate no box; one with generated content starts with that content.
fn starts_with_forced_spanner(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    document.nodes[node_id].multicol.is_some()
        && !generated_on_side(cascade, node_id, true)
        && page_edge_child(document, cascade, node_id, true, 0)
            .ok()
            .flatten()
            .is_some_and(|child| {
                document.nodes[child].kind() == NodeKind::Element
                    && document.nodes[child].display() != DisplayValue::Contents
                    && page_break_is_forced(
                        document,
                        cascade,
                        child,
                        cascade.computed[child].break_before,
                    )
            })
}

/// Whether `id` is a block container whose first and last boxes start and
/// end its own block flow, so that a forced break at them passes to it.
/// Generated content on that side of its boxes keeps the break inside it,
/// and no break reaches out of a box that pagination does not break through.
fn passes_edge_breaks(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
    before: bool,
) -> bool {
    breaks_through_block_flow(document, cascade, id) && !generated_on_side(cascade, id, before)
}

/// Whether `id` stacks its children in block flow that pagination breaks
/// through, so that a forced break between them breaks the page. A block
/// with inline content beside its block children still does.
pub(super) fn breaks_through_block_flow(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
) -> bool {
    matches!(
        document.nodes[id].display(),
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
    ) && document.nodes[id].multicol.is_none()
        && !is_monolithic_for_pagination_with_transposed_axes(document, cascade, id, false)
}

/// Whether `id` has generated content in the flow before (`before`) or after
/// its boxes. An inside list marker comes before them too.
pub(super) fn generated_on_side(cascade: &CascadeResult, id: usize, before: bool) -> bool {
    if before {
        crate::generated_content::has_page_flow_generated_prefix(cascade, id)
    } else {
        crate::generated_content::is_in_flow_generated_text(
            cascade,
            id,
            raikiri_style::PseudoElem::After,
        )
    }
}

// Find the first/last generated child box, eliding Contents without cloning
// wide child lists. Nonempty text and generated content remain edge barriers.
fn page_edge_child(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
    before: bool,
    depth: usize,
) -> Result<Option<usize>, ()> {
    if depth >= 128 {
        return Err(());
    }
    let children = &document.nodes[id].children;
    for offset in 0..children.len() {
        let child = children[if before {
            offset
        } else {
            children.len() - 1 - offset
        }];
        let node = &document.nodes[child];
        if !node.is_in_document()
            || node.is_unrendered_html_element()
            || document.nodes[child].display() == DisplayValue::None
            || matches!(
                document.nodes[child].position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
            || is_floating_box_for_pagination(document, cascade, child)
        {
            continue;
        }
        if let crate::node::NodeData::Text(text) = &node.data {
            // White space that the style preserves still makes a line.
            if crate::generated_content::is_collapsed_white_space(
                &text.text_content,
                cascade.computed[child].effective_white_space_collapse,
            ) {
                continue;
            }
            return Ok(Some(child));
        }
        // An inline box has no block edges of its own: a block inside it at
        // its edge is at the paragraph's edge, as it is for an element with
        // no box. Padding, border or margin on its inline sides take room on
        // a line around the block, which then is not at the edge, as do a
        // line break and a replaced element, which are content of a line.
        let display = document.nodes[child].display();
        if (display == DisplayValue::Contents
            || (display == DisplayValue::Inline
                && !has_inline_side_edges(cascade, child)
                && !makes_line_content(document, child)))
            && node.kind() == NodeKind::Element
            && !node.has_before_or_after_content
        {
            if let Some(descendant) = page_edge_child(document, cascade, child, before, depth + 1)?
            {
                return Ok(Some(descendant));
            }
            continue;
        }
        return Ok(Some(child));
    }
    Ok(None)
}

/// Whether inline content of the paragraph `root` follows its block `block`:
/// the end of an inline box around the block, or an inline box or text after
/// it. Chromium closes such a block with a line of its own, empty as it may
/// be, which a forced break after the block carries to the next page along
/// with the paragraph's bottom padding and border.
fn line_follows_paragraph_block(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    block: usize,
) -> bool {
    // A line closes after the block when an inline box around it ends or
    // inline content follows it, but only if no box between the block and
    // the paragraph keeps the break inside itself.
    let mut follows = false;
    let mut id = block;
    while id != root {
        // A break inside a box out of the flow, a float or an atomic inline
        // stays inside that box: no line of the paragraph follows it.
        let node = &document.nodes[id];
        let display = node.display();
        let Some(parent) = document.parent_of(id) else {
            return false;
        };
        if matches!(
            node.position(),
            PositionValue::Absolute | PositionValue::Fixed
        ) || is_floating_box_for_pagination(document, cascade, id)
            || !(display == DisplayValue::Inline
                || display == DisplayValue::Contents
                || is_block_level_flow_box(display))
        {
            return false;
        }
        follows = follows
            || (id != block && display == DisplayValue::Inline)
            || inline_content_follows(document, cascade, parent, id);
        id = parent;
    }
    follows
}

/// Whether a later child of `parent` than `id` puts content on a line.
fn inline_content_follows(
    document: &Document,
    cascade: &CascadeResult,
    parent: usize,
    id: usize,
) -> bool {
    let children = &document.nodes[parent].children;
    let after = children
        .iter()
        .position(|&child| child == id)
        .map_or(0, |at| at + 1);
    children[after..]
        .iter()
        .any(|&child| makes_inline_content(document, cascade, child, 0))
}

/// Whether `id` puts content on a line of its paragraph: text that does not
/// collapse away, or an inline-level box in the flow.
fn makes_inline_content(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
    depth: usize,
) -> bool {
    let node = &document.nodes[id];
    if depth >= 128
        || !node.is_in_document()
        || node.is_unrendered_html_element()
        || node.display() == DisplayValue::None
        || matches!(
            node.position(),
            PositionValue::Absolute | PositionValue::Fixed
        )
        || is_floating_box_for_pagination(document, cascade, id)
    {
        return false;
    }
    if let crate::node::NodeData::Text(text) = &node.data {
        return !crate::generated_content::is_collapsed_white_space(
            &text.text_content,
            cascade.computed[id].effective_white_space_collapse,
        );
    }
    match node.display() {
        DisplayValue::Contents => node
            .children
            .iter()
            .any(|&child| makes_inline_content(document, cascade, child, depth + 1)),
        display => node.kind() == NodeKind::Element && !is_block_level_flow_box(display),
    }
}

/// Whether the inline element `id` is content of a line by itself: a line
/// break opportunity (`br`, `wbr`) or a replaced element.
fn makes_line_content(document: &Document, id: usize) -> bool {
    document.nodes[id]
        .tag_name()
        .is_some_and(|tag| tag.eq_ignore_ascii_case("br") || tag.eq_ignore_ascii_case("wbr"))
        || crate::layout::page::is_replaced_for_pagination(document, id)
}

/// Whether the inline box `id` has padding, border or margin on its left or
/// right side.
fn has_inline_side_edges(cascade: &CascadeResult, id: usize) -> bool {
    use raikiri_style::resolve::{ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrCalc};
    let style = &cascade.computed[id];
    let padding =
        |side: &ComputedLengthPercentageOrCalc| *side != ComputedLengthPercentageOrCalc::Px(0.0);
    let margin =
        |side: &ComputedLengthPercentageOrAuto| *side != ComputedLengthPercentageOrAuto::Px(0.0);
    padding(&style.padding.left)
        || padding(&style.padding.right)
        || margin(&style.margin.left)
        || margin(&style.margin.right)
        || style.border.left.width().0 != 0.0
        || style.border.right.width().0 != 0.0
}

// Preserve source-node context when resolving Always: a column-local forced
// edge must not become a page break merely because its wrapper is outside it.
fn page_edge_constraint(
    document: &Document,
    cascade: &CascadeResult,
    mut id: usize,
    before: bool,
) -> (bool, bool) {
    let mut forced = false;
    let mut avoided = false;
    for _ in 0..128 {
        let computed = &cascade.computed[id];
        if document.nodes[id].display() != DisplayValue::Contents {
            let edge = if before {
                computed.break_before
            } else {
                computed.break_after
            };
            forced |= if before {
                page_break_is_forced(document, cascade, id, edge)
            } else {
                box_forces_break_after(document, cascade, id, edge)
            };
            avoided |= matches!(edge, BreakBetween::Avoid | BreakBetween::AvoidPage);
        }
        if renders_edge_pseudo(cascade, id, before)
            || !matches!(
                document.nodes[id].display(),
                DisplayValue::Block
                    | DisplayValue::FlowRoot
                    | DisplayValue::ListItem
                    | DisplayValue::Contents
            )
        {
            break;
        }
        let Ok(Some(child)) = page_edge_child(document, cascade, id, before, 0) else {
            break;
        };
        if document.nodes[child].kind() == NodeKind::Text {
            break;
        }
        id = child;
    }
    (forced, avoided)
}

pub(crate) fn page_name_to_owned(name: Option<&str>) -> Option<String> {
    name.map(|name| {
        #[cfg(test)]
        super::page::record_page_name_work(2);
        name.to_owned()
    })
}

pub(crate) fn selected_page_name(cascade: &CascadeResult, node_id: usize) -> Option<&str> {
    match cascade.page_values.get(node_id) {
        Some(raikiri_style::property::PageValue::Named(name)) => Some(name.0.as_str()),
        _ => None,
    }
}

/// Layout a document once and produce ordered page slices.
///
/// The current fragmentainer handles ordinary block-flow children of
/// `<body>`.  It honors forced `page` breaks (including the legacy aliases
/// already normalized by the style cascade), coalesces adjacent forced
/// breaks, and keeps an empty explicitly-created page.  A block that is taller
/// than a page is exposed on each intersecting slice; splitting its internal
/// line/child fragments is deliberately left to the next fragmentation pass.
/// Direct tables additionally keep a row whose cell boxes would cross a page
/// together by moving that row to the next fragmentainer. Ordinary horizontal
/// direct-body tables repeat their first header group, reserving its height
/// before continuation rows and keeping the initial header with the first row.
/// Headers taller than a quarter page, spanning rows, bottom captions, fixed
/// header descendants, header page boundaries, and multicolumn tables retain
/// ordinary fragmentation. Repeated footers and cell-internal breaks remain
/// outside this pass.
/// Column-direction flex containers likewise keep a direct flex item together;
/// row-direction flex, wrapping-line, and intrinsic-item fragmentation remain
/// outside this pass. A single explicit-column grid likewise keeps direct
/// non-spanning items together; multi-column placement and spanning items are
/// outside this pass.
/// For an ordinary direct-body block whose complete text layout fits within one
/// fragmentainer, `orphans` / `widows` can move the block intact when the
/// natural split would leave too few line boxes on either side. Oversized or
/// nested inline formatting contexts remain outside this pass.
///
/// This is intentionally separate from [`layout_single_page`]: existing
/// callers retain the single-page contract while paged callers get a real
/// per-page result and the same post-layout DOM as the scene builder.
#[doc(hidden)]
pub fn layout_pages(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_options(document, cascade, page_box, PageLayoutOptions::default())
}

/// Layout a document into pages with borrowed geometry, image, and control settings.
///
/// Image resolution preserves the document's decoded image state for later
/// layout and painting. A hidden root does not ask the resolver for images.
#[doc(hidden)]
pub fn layout_pages_with_options(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    options: PageLayoutOptions<'_>,
) -> Result<Vec<PageSlice>, LayoutError> {
    let default_control;
    let control = match options.control {
        Some(control) => control,
        None => {
            default_control = PageLayoutControl::default();
            &default_control
        }
    };
    if let Some(images) = options.images {
        control.check_page_index(0)?;
        super::validate_layout_depth(document)?;
        document.page_projection.clear();
        document.mark_in_document_flags();
        resolve_document_images(document, cascade, images.resolver, images.base_url)?;
    }
    layout_pages_core(
        document,
        cascade,
        page_box,
        options.page_steps,
        options.page_widths,
        control,
    )
}

/// Resolve the intrinsic sizes of the document's `<img>` elements. A root
/// with `display: none` generates no boxes, so none of its images are
/// rendered and the resolver is not asked for them.
fn resolve_document_images(
    document: &mut Document,
    cascade: &CascadeResult,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
) -> Result<(), LayoutError> {
    if BoxPropertiesRef::from_computed(cascade.root_element_computed()).display()
        == DisplayValue::None
    {
        return Ok(());
    }
    match base_url {
        Some(base_url) => {
            crate::image_resolve::resolve_images_with_base(document, resolver, Some(base_url))
        }
        None => crate::image_resolve::resolve_images(document, resolver),
    }
    .map_err(LayoutError::Resolver)
}

fn layout_pages_core(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    control: &PageLayoutControl<'_>,
) -> Result<Vec<PageSlice>, LayoutError> {
    let mut resumptions = float_resumption::Plan::default();
    let mut unpaged_plain_columns = false;
    let mut spanning_origins = HashMap::new();
    let mut page_filled_origins = HashMap::new();
    // Each container whose columns are filled page by page may take one
    // more attempt, to fill them from where the walk moved it.
    let mut attempts = 0;
    // Floats moved whole to a later page take one more attempt to lay the
    // content beside them out without them.
    while attempts < 3 + page_filled_origins.len() + usize::from(resumptions.has_pushed_floats()) {
        attempts += 1;
        let next = layout_pages_attempt(
            document,
            cascade,
            page_box,
            page_steps,
            page_widths,
            control,
            PageAttempt {
                unpaged_plain_columns,
                resumptions: &resumptions,
                spanning_origins: &spanning_origins,
                page_filled_origins: &page_filled_origins,
            },
        )?;
        if !next.moved_columns
            && next.resumptions == resumptions
            && next.spanning_origins == spanning_origins
            && next.page_filled_origins == page_filled_origins
        {
            return Ok(next.slices);
        }
        unpaged_plain_columns |= next.moved_columns;
        resumptions = next.resumptions;
        spanning_origins = next.spanning_origins;
        page_filled_origins = next.page_filled_origins;
    }
    // cov:ignore: defensive retry-budget exhaustion rejects non-converging pagination instead of returning unstable slices.
    Err(LayoutError::Internal {
        message: "page float resumption did not stabilize".to_string(),
    })
}

struct PageAttempt<'a> {
    unpaged_plain_columns: bool,
    resumptions: &'a float_resumption::Plan,
    spanning_origins: &'a HashMap<usize, f32>,
    page_filled_origins: &'a HashMap<usize, f32>,
}

struct PageAttemptResult {
    slices: Vec<PageSlice>,
    moved_columns: bool,
    resumptions: float_resumption::Plan,
    spanning_origins: HashMap<usize, f32>,
    page_filled_origins: HashMap<usize, f32>,
}

/// Lay out and break `document` into pages once, retaining the reasons to
/// retry from source layout. Moving columns filled page by page leaves the
/// slices empty; a changed spanning origin requests a new group plan.
/// With `unpaged_plain_columns`, plain columns are not filled page by page.
fn layout_pages_attempt(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    control: &PageLayoutControl<'_>,
    attempt: PageAttempt<'_>,
) -> Result<PageAttemptResult, LayoutError> {
    control.page_limit_reached.set(false);
    control.check_page_index(0)?;
    let mut session = super::context::LayoutSession::with_cascade(document, cascade);
    session.context.unpaged_plain_columns = attempt.unpaged_plain_columns;
    session.context.float_resumptions = attempt.resumptions.clone();
    session.context.spanning_page_origins = attempt.spanning_origins.clone();
    session.context.page_filled_origins = attempt.page_filled_origins.clone();
    if !attempt.resumptions.is_empty()
        || !attempt.spanning_origins.is_empty()
        || !attempt.page_filled_origins.is_empty()
    {
        session.layout_dirty = true;
    }
    // A root with `display: none` generates no boxes, so there is nothing to
    // lay out or break into later pages: the document prints as a single
    // blank page.
    if BoxPropertiesRef::from_computed(cascade.root_element_computed()).display()
        == DisplayValue::None
    {
        // Fragments and a projection of an earlier layout no longer apply.
        session.fragment_tree.clear();
        session.column_rules.clear();
        session.page_projection.clear();
        session.outside_marker_leading = Default::default();
        session.fixed_static_page_origins.clear();
        return Ok(PageAttemptResult {
            slices: vec![PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None,
            }],
            moved_columns: false,
            resumptions: float_resumption::Plan::default(),
            spanning_origins: HashMap::new(),
            page_filled_origins: HashMap::new(),
        });
    }
    layout_single_page_with_table_projection(&mut session, cascade, page_box, true, true)?;
    control.check_page_index(0)?;

    let body_id = super::initial_containing_block::root_element(&session).ok_or_else(|| {
        LayoutError::Internal {
            message: "no document element found".to_string(),
        }
    })?;
    let margins = page_margins(cascade, page_box);
    let insets = page_content_insets(cascade, page_box);
    let slices = match transposed_flow::FlowAxes::of_root(&session, cascade) {
        None => paginate_flow(
            &mut session,
            cascade,
            body_id,
            PageFlowGeometry {
                page_box,
                margins,
                insets,
            },
            page_steps,
            page_widths,
            control,
        )?,
        // The pages of a document whose root lays out its blocks on vertical
        // axes follow the physical x axis: each page takes the width of its
        // page area from the flow. The paginator breaks along y, so it runs
        // on the transposed document. The pages keep the inline size of the
        // first page area.
        Some(axes) => {
            super::multicol::release_unspanned_groups(&mut session);
            transposed_flow::TransposedDocument::with_transposed(
                &mut session,
                body_id,
                axes,
                insets.page_area_width(margins, page_box),
                |session, transposed| {
                    let transposed_cascade = transposed.cascade(cascade);
                    paginate_flow(
                        session,
                        &transposed_cascade,
                        body_id,
                        PageFlowGeometry {
                            page_box: axes.page_box(page_box),
                            margins: axes.margins(margins),
                            insets: axes.insets(insets),
                        },
                        page_widths,
                        &[],
                        control,
                    )
                },
            )?
        }
    };
    // The pages were cut from layouts run since the first pass, which take the
    // same limit.
    take_recursion_failure(&mut session)?;
    let resumptions = float_resumption::Plan::collect(&session, cascade);
    Ok(PageAttemptResult {
        slices,
        moved_columns: session.context.page_filled_columns_moved,
        resumptions,
        spanning_origins: session.context.spanning_page_origins.clone(),
        page_filled_origins: session.context.page_filled_origins.clone(),
    })
}

/// The direction in which the pages of `document` continue, when they follow
/// the physical x axis: `Some(true)` when each page continues to the left of
/// the one before it, as `vertical-rl` does, and `Some(false)` when it
/// continues to the right, as `vertical-lr` does. `None` when the pages
/// follow y.
///
/// A page then holds the slice of the flow from its
/// [`PageSlice::content_origin_y`] along that axis: for `vertical-rl`, the
/// boxes whose physical x lies between `-content_origin_y` and the page
/// area width minus it.
#[doc(hidden)]
pub fn pages_follow_x_axis(document: &Document, cascade: &CascadeResult) -> Option<bool> {
    transposed_flow::FlowAxes::of_root(document, cascade)
        .map(transposed_flow::FlowAxes::right_to_left)
}

/// The page box, margins and insets a flow is paginated in.
#[derive(Clone, Copy)]
struct PageFlowGeometry {
    page_box: PageBox,
    margins: PageMargins,
    insets: PageContentInsets,
}

/// The page type pagination assigns to each page the flow reaches.
struct PageNames {
    /// Whether this cascade can produce a named page during this pagination run.
    records_names: bool,
    /// The page type of the flow at the candidate being placed.
    current: Option<String>,
    /// The page type the enclosing flow set last; an unnamed candidate
    /// outside a named flex context compares its own against it.
    outer: Option<String>,
    by_page: Vec<Option<String>>,
}

impl PageNames {
    fn new(first: Option<String>) -> Self {
        #[cfg(test)]
        super::page::record_page_name_work(5);
        Self {
            records_names: true,
            by_page: vec![first.clone()],
            outer: first.clone(),
            current: first,
        }
    }

    fn unnamed() -> Self {
        Self {
            records_names: false,
            current: None,
            outer: None,
            by_page: Vec::new(),
        }
    }

    /// Returns whether content already placed in `candidate`'s flow lets a
    /// named page change open a new page, and the page type `candidate`
    /// compares its own against to detect that change. `saw_child` tells
    /// whether any earlier box was placed. Also records the page type an
    /// enclosing flow sets for later candidates.
    fn comparison(
        &mut self,
        candidate: &PageCandidate,
        saw_child: bool,
        column_contexts: &mut ColumnContexts,
        document: &Document,
        cascade: &CascadeResult,
        page: u32,
    ) -> (bool, Option<String>) {
        if !candidate.context.page_name_in_flow() {
            return (false, self.current.clone());
        }
        match candidate.named_flex_context {
            Some(context) if candidate.context.deferred_named_break_after() => {
                // Empty enclosing column boxes do not open an anonymous
                // page before the row's established deferred boundary.
                let seen = column_contexts.is_occupied(context, page, document, cascade)
                    || self.current.is_some();
                self.outer = candidate.page_name.clone();
                (saw_child && seen, self.current.clone())
            }
            Some(context)
                if candidate.context.is_named()
                    || candidate.context.is_text()
                    || candidate.context.is_rendered_leaf() =>
            {
                // Nested column flows share the physical page opened by
                // their preceding sibling, including an inner flow's end.
                // A previously seen anonymous type is still a page type.
                let seen = column_contexts.is_occupied(context, page, document, cascade)
                    || self.current.is_some();
                (saw_child && seen, self.current.clone())
            }
            Some(_) => (false, candidate.page_name.clone()),
            None => {
                let previous = if candidate.context.is_named() {
                    self.current.clone()
                } else {
                    self.outer.clone()
                };
                if !candidate.context.is_float_descendant() {
                    self.outer = candidate.page_name.clone();
                }
                (saw_child, previous)
            }
        }
    }

    /// Give `page` the current page type.
    fn name_page(&mut self, page: u32) {
        if !self.records_names {
            debug_assert!(self.current.is_none() && self.outer.is_none());
            return;
        }
        let index = page as usize;
        if self.by_page.len() <= index {
            self.by_page.resize(index + 1, None);
        }
        self.by_page[index].clone_from(&self.current);
    }

    /// Name the pages after `from` through `end`, where content on page
    /// `from` runs on. Pages no earlier content reached take the current
    /// page type; with `overwrite`, so do those that already have one.
    fn continue_onto(&mut self, from: u32, end: u32, overwrite: bool) {
        if !self.records_names {
            debug_assert!(self.current.is_none() && self.outer.is_none());
            return;
        }
        let end = end as usize;
        if self.by_page.len() <= end {
            self.by_page.resize(end + 1, self.current.clone());
        }
        if overwrite && end > from as usize {
            for name in &mut self.by_page[from as usize + 1..=end] {
                name.clone_from(&self.current);
            }
        }
    }
}

/// The last pages pagination has reached.
#[derive(Default)]
struct PageReach {
    /// The last page any box reached.
    max: u32,
    /// The last page the normal flow (floats included) reached, apart from
    /// absolutely positioned boxes and footnotes, which paint in parallel
    /// flows.
    flow_max: u32,
}

impl PageReach {
    fn include(&mut self, page: u32, in_flow: bool) {
        self.max = self.max.max(page);
        if in_flow {
            self.flow_max = self.flow_max.max(page);
        }
    }

    /// Include `page`, a page a box reaches past the one it was placed on,
    /// once the page limit allows it.
    fn include_checked(
        &mut self,
        control: &PageLayoutControl<'_>,
        page: u32,
        in_flow: bool,
    ) -> Result<(), LayoutError> {
        control.check_discovery_page_index(page)?;
        self.include(page, in_flow);
        Ok(())
    }
}

/// A box that starts on `start_page` runs on through `end_page`. The pages
/// it continues onto keep the page type it started on until a later box
/// names another, even when an enclosing box running onto them named them
/// first; a box outside the normal flow names only pages no earlier box
/// named.
fn record_run_on(
    reach: &mut PageReach,
    page_names: &mut PageNames,
    control: &PageLayoutControl<'_>,
    start_page: u32,
    end_page: u32,
    in_flow: bool,
) -> Result<(), LayoutError> {
    reach.include_checked(control, end_page, in_flow)?;
    page_names.continue_onto(start_page, control.capped_page_index(end_page), in_flow);
    Ok(())
}

fn reset_inline_x_for_page_transition(
    document: &mut Document,
    node_id: usize,
    page_transition: bool,
    is_direct_body_element: bool,
) {
    // A direct inline child starts its next page fragment at the inline
    // origin instead of keeping its source-order position from the first pass.
    if page_transition
        && is_direct_body_element
        && document.nodes[node_id].display().is_inline_flow()
    {
        document.nodes[node_id].layout.location.x = 0.0;
    }
}

/// The page advance used where no page has a scheduled height: the page
/// area's `content_height`, or the `paper_height` (at least 1) when the page
/// area is empty. A page with margins consuming the entire paper still needs
/// a finite cursor for forced breaks. No valid page box reaches this case in
/// normal CSS, but the fallback keeps the API panic-free for direct callers.
fn fixed_page_step(content_height: f32, paper_height: f32) -> f32 {
    if content_height.is_finite() && content_height > 0.0 {
        content_height
    } else {
        paper_height.max(1.0)
    }
}

/// Propagate a spanning owner's growth only while its ancestors can absorb it.
fn grow_spanning_column_ancestors(
    document: &mut super::context::LayoutSession<'_>,
    owner: usize,
    delta: f32,
    body_id: usize,
    sibling_shifts: &mut sibling_shifts::SiblingShifts,
    fragment_edits: &mut FragmentEdits,
    column_work: &mut ProjectionWork<'_, '_>,
) -> Result<(), LayoutError> {
    let mut child = owner;
    let mut walk = AncestorCursor::new(document.layout_parent_of(owner), ParentRelation::Layout);
    while let Some(parent) = walk.current() {
        if document.nodes[child].layout_position() == Position::Absolute
            || document.pagination_style(child).float().is_floated()
        {
            break;
        }
        // A paragraph's line shifts follow its boxes' current positions.
        if document.nodes[parent].ifc.is_some() {
            sibling_shifts.apply(document, fragment_edits, parent);
        }
        let _ = follow_moved_ifc_block(document, child, delta);
        sibling_shifts.record(document, parent, child, delta, column_work)?;
        if !document.block_axis(parent).height().is_auto() {
            break;
        }
        document.nodes[parent].layout.size.height += delta;
        document.mark_pagination_extent_dirty(parent);
        fragment_edits.grow(parent, delta);
        if parent == body_id {
            break; // cov:ignore: the body has a pinned viewport height and exits at the preceding fixed-height boundary.
        }
        child = parent;
        walk.advance(document);
    }
    Ok(())
}

/// Breaks the page before the edges that follow the first `placed` candidates
/// and have not been judged yet (see [`PaginationState::break_before_edges`]).
/// Returns whether the page limit stopped the flow.
fn break_before_edges(
    state: &mut PaginationState,
    document: &mut super::context::LayoutSession<'_>,
    env: &FlowEnv<'_>,
    edges_breaks: &mut std::iter::Peekable<std::vec::IntoIter<candidates::EdgesBreak>>,
    placed: usize,
) -> Result<bool, LayoutError> {
    while let Some(edges_break) = edges_breaks.next_if(|found| found.before <= placed) {
        if let Err(halt) = state.break_before_edges(document, env, &edges_break) {
            halt.finish()?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Break the laid-out flow of `document` into pages of `geometry` along y.
/// `page_steps` and `page_widths` give the content height and width of each
/// page area, as for [`layout_pages_with_options`].
#[allow(clippy::too_many_arguments)]
fn paginate_flow(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    geometry: PageFlowGeometry,
    page_steps: &[f32],
    page_widths: &[f32],
    control: &PageLayoutControl<'_>,
) -> Result<Vec<PageSlice>, LayoutError> {
    let PageFlowGeometry {
        page_box,
        margins,
        insets,
    } = geometry;
    // Keep the scheduled inline size identical to the first layout pass.
    let content_width = insets.page_area_width(margins, page_box);
    let content_height = insets.page_area_height(margins, page_box);
    let mut unpaginated_overflow =
        unpaginated_content_overflow(document, cascade, body_id, content_height);
    let positioned_owner_heights = positioned_flow::owner_heights(document);
    let fixed_page_step = fixed_page_step(content_height, page_box.height);
    // A page-size/margin change can make the fragmentainer height vary by
    // page.  The default API supplies no schedule and therefore follows the
    // fixed first-page height used historically.
    let page_step = fixed_page_step;
    let page_origins = PageOrigins::new(page_steps, fixed_page_step);
    let page_step_at = |page_index: u32| page_origins.step_at(page_index);
    let page_origin = |page_index: u32| page_origins.origin(page_index);
    let page_index_for_y = |y: f32| page_origins.page_index_for_y(y);
    leading_margins::truncate_flow(document, cascade, body_id, &page_origins, control)?;
    let root_flow_offset = 0.0;

    // Page heights are known here, after source layout has established each
    // owner's position. Retain page-local column placements before collecting
    // paragraph and fragment projections, including the changed following flow.
    let work_limit = document.fragment_tree.work_limit(document.nodes.len());
    let mut column_work = ProjectionWork::with_remaining(
        control,
        work_limit,
        work_limit.saturating_sub(document.fragment_tree.break_flow_work_used),
    );
    // Each owner's position walks its ancestors; find it once per owner
    // rather than once per comparison.
    let mut spanning_owners: Vec<(f32, usize)> = document
        .nodes
        .iter()
        .enumerate()
        .filter(|&(id, node)| {
            !node.multicol_groups.is_empty()
                || node.multicol.is_some()
                    && document
                        .children_through_contents(id)
                        .iter()
                        .any(|&child| super::is_spanner_in(document, cascade, child))
        })
        .map(|(id, _)| (current_abs_y(document, id), id))
        .collect();
    spanning_owners.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Fragment records change for every owner; they are installed together
    // once all owners are paginated.
    let mut fragment_edits = FragmentEdits::new(&document.fragment_tree);
    // So are the moves of the boxes after each owner, which are applied to a
    // parent's children before they are read.
    let mut sibling_shifts = sibling_shifts::SiblingShifts::default();
    let mut leading_spanning_origins = Vec::new();
    for (_, owner) in spanning_owners {
        let source_y = root_flow_offset
            + current_abs_y(document, owner)
            + sibling_shifts.offset(document, owner);
        sibling_shifts.apply(document, &mut fragment_edits, owner);
        let owner_y = document
            .context
            .spanning_page_origins
            .get(&owner)
            .or_else(|| document.context.page_filled_origins.get(&owner))
            .copied()
            .unwrap_or(source_y);
        let tracked = document.context.spanning_page_origins.contains_key(&owner)
            || (lineless_leading_piece_end(document, cascade, body_id, owner).is_some()
                && document
                    .children_through_contents(owner)
                    .iter()
                    .any(|&child| document.nodes[child].column_span_all));
        if tracked {
            leading_spanning_origins.push((owner, owner_y));
        }
        let delta = paginate_spanning_columns(
            document,
            &mut fragment_edits,
            owner,
            owner_y,
            &|y| {
                let index = page_index_for_y(y);
                (index, page_origin(index), page_step_at(index))
            },
            &mut column_work,
        )?;
        if delta == 0.0 {
            continue;
        }
        grow_spanning_column_ancestors(
            document,
            owner,
            delta,
            body_id,
            &mut sibling_shifts,
            &mut fragment_edits,
            &mut column_work,
        )?;
    }
    sibling_shifts.apply_all(document, &mut fragment_edits);
    fragment_edits.finish(document, &mut column_work)?;
    document.fragment_tree.break_flow_work_used = work_limit - column_work.remaining();
    document.fragment_tree.finalize();
    document.column_rules = crate::column_rules::prepare(document, cascade)?;

    // Where the columns of each container were filled page by page: from
    // where an earlier attempt's walk moved it, when it did, else from the
    // position it has now. The flag tells the first case.
    let refilled: Vec<_> = document
        .context
        .page_filled_columns
        .iter()
        .map(
            |&owner| match document.context.page_filled_origins.get(&owner) {
                Some(&origin) => (owner, origin - root_flow_offset, true),
                None => (owner, current_abs_y(document, owner), false),
            },
        )
        .collect();
    let roots = candidates::FlowRoots::new(document, body_id);
    let mut candidates = Vec::new();
    let mut text_lines = TextLineIndex::default();
    let mut collector = CandidateCollector::new(
        document,
        cascade,
        roots,
        page_step,
        &mut candidates,
        &mut text_lines,
        control,
    );
    collector.collect_flow(root_flow_offset)?;
    let (clip_edges, edges_breaks, preceding_bottoms) = collector.into_outputs();
    let mut footer_tail_ends = HashMap::<usize, f32>::new();
    for candidate in &candidates {
        let edges = candidate.context.item_tail_edges();
        if !candidate.context.is_table_row() || edges <= 0.0 {
            continue;
        }
        if let Some(footer) = footer_group_of_row(document, candidate.node_id) {
            let tail_end = candidate.raw_y + candidate.height + edges;
            footer_tail_ends
                .entry(footer)
                .and_modify(|end| *end = end.max(tail_end))
                .or_insert(tail_end);
        }
    }
    let mut table_headers = HeaderRepeats::prepare(
        document,
        cascade,
        roots.contents,
        page_step,
        |id| root_flow_offset + current_abs_y(document, id),
        |id| {
            super::page::is_monolithic_for_pagination_with_transposed_axes(
                document,
                cascade,
                id,
                document.context.pagination_on_transposed_axes,
            )
        },
    );
    for (owner, tail_end) in footer_tail_ends {
        if let Some(footer) = table_headers.headers.get_mut(&owner) {
            // An avoided group may be represented by its first row. Both
            // ends include root_flow_offset; subtract the entire footer so
            // its remaining rows are not counted as additional closing edges.
            let footer_end = footer.source_y + footer.height;
            footer.closing_edges = (tail_end - footer_end).max(0.0);
        }
    }
    let avoided_breaks = AvoidedBreaks::collect(
        document,
        cascade,
        &candidates,
        &table_headers,
        roots,
        content_width,
        root_flow_offset,
    );
    // A footer that starts a run of boxes tied by avoided breaks moves to
    // the next page with them (see `place_table_footer`).
    for (&group, footer) in table_headers
        .headers
        .iter_mut()
        .filter(|(_, group)| group.footer)
    {
        let first_row = document
            .children_through_contents(group)
            .into_iter()
            .find(|&row| document.nodes[row].display() == DisplayValue::TableRow);
        if let Some(run) = first_row.and_then(|row| avoided_breaks.run_ends.get(&row)) {
            footer.tied_run = Some((run.end - (footer.source_y + footer.height), run.firm));
        }
    }
    let table_row_origins = table_row_origins(document, &candidates);
    let page_names = if super::page::has_named_pages(cascade) {
        PageNames::new(
            page_name_to_owned(selected_page_name(cascade, body_id)).or_else(|| {
                super::page::first_page_name_with_transposed_axes(
                    document,
                    cascade,
                    document.context.pagination_on_transposed_axes,
                )
            }),
        )
    } else {
        PageNames::unnamed()
    };
    let mut state = PaginationState::new(
        table_headers,
        page_names,
        ColumnContexts::new(document.nodes.len()),
    );
    state.track_outer_owners(&positioned_owner_heights);
    state.sibling_margins.preceding_bottoms = preceding_bottoms;
    state.margin_rooms = truncatable_margin_rooms(document, cascade, body_id, &edges_breaks);
    let env = FlowEnv {
        cascade,
        body_id,
        control,
        page_origins: &page_origins,
        text_lines: &text_lines,
        roots,
        root_flow_offset,
        content_width,
        avoided_breaks: &avoided_breaks,
        insets,
        page_box,
        clip_edges: &clip_edges,
    };
    state.prepare_float_gaps(document, cascade, body_id, &candidates);
    normal_block_shifts::begin(document, cascade, body_id);
    let candidate_result = (|| -> Result<(), LayoutError> {
        let mut candidates = candidates.into_iter();
        let mut edges_breaks = edges_breaks.into_iter().peekable();
        let mut done = 0;
        let mut halted = false;
        // Each candidate is placed while the page limit lets pagination reach the
        // page the flow is on. The page breaks before the edges of a block that
        // follow the candidates before them, when they do not fit.
        while let Some(mut candidate) = candidates.next()
            && control.within_page_limit(state.current_page)?
        {
            state
                .pending_break
                .leave_scopes(document, candidate.node_id);
            if break_before_edges(&mut state, document, &env, &mut edges_breaks, done)? {
                halted = true;
                break;
            }
            if let Err(halt) =
                state.break_before_content_after_forced_break(document, &env, Some(&candidate))
            {
                halt.finish()?;
                halted = true;
                break;
            }
            done += 1;
            state.rebase_fragmented_cell_content(document, &mut candidate);
            let Some(entry) = state.begin_candidate(document, &env, &candidate)? else {
                continue;
            };
            let placed = if candidate.context.is_text() {
                state.place_text(document, &env, &candidate, &entry)
            } else {
                state.place_box(document, &env, &candidate, &entry)
            };
            if let Err(halt) = placed {
                halt.finish()?;
                halted = true;
                break;
            }
        }
        if !halted {
            break_before_edges(&mut state, document, &env, &mut edges_breaks, done)?;
            if let Err(halt) = state.break_before_content_after_forced_break(document, &env, None) {
                halt.finish()?;
            }
        }
        state.subtree_shifts.finish_rows(document);
        Ok(())
    })();
    document.finish_normal_block_shifts();
    candidate_result?;
    // A block whose margin above its edges a page edge truncated ends that
    // much above the margin box of its last child, as content overflowing it
    // would.
    for (&node_id, &cut) in &state.margin_cuts {
        *unpaginated_overflow.entry(node_id).or_default() += cut;
    }
    if !positioned_owner_heights.is_empty() {
        // Resolve ancestor-dependent positions from normal-flow growth before
        // positioned fragmentation and the table-box geometry finalizers.
        // First-row break gaps added by those finalizers do not contribute
        // to this positioning basis, as with a percentage top beside a table.
        update_paginated_block_heights(
            document,
            cascade,
            body_id,
            content_height,
            &unpaginated_overflow,
            &state.kept_margin_ends,
        );
    }
    if let Err(halt) = state.keep_positioned_lines(document, &env, &positioned_owner_heights) {
        halt.finish()?;
    }

    // Replan all groups of a leading spanner owner from source geometry when
    // the outer flow accepted a different origin than the planner used.
    for (owner, planned_y) in leading_spanning_origins {
        let accepted_y = root_flow_offset + current_abs_y(document, owner);
        if (accepted_y - planned_y).abs() > 1e-3 {
            document
                .context
                .spanning_page_origins
                .insert(owner, accepted_y);
        }
    }

    // A container the walk moved, as a forced break does, has its columns
    // filled for another place. The first one is filled again from where the
    // walk put it, on the next attempt; the containers after it move with it
    // and wait for their own attempt. A container that moves away from such
    // a place too has the document laid out without filling any columns.
    let moved = refilled.iter().find_map(|&(owner, planned_y, replanned)| {
        let y = current_abs_y(document, owner);
        let tolerance = 1e-3_f32.max(y.abs() * 1e-6);
        ((y - planned_y).abs() > tolerance).then_some((owner, y, replanned))
    });
    if let Some((owner, y, replanned)) = moved {
        document.context.page_filled_columns_moved = replanned;
        if !replanned {
            document
                .context
                .page_filled_origins
                .insert(owner, root_flow_offset + y);
        }
        return Ok(Vec::new());
    }
    place_repeated_table_footers(document, &mut state.table_headers, &page_origins);
    follow_moved_table_rows(document, table_row_origins, &state.table_headers);
    super::table::headers::finish_boxes(document, &state.table_headers);
    update_paginated_block_heights(
        document,
        cascade,
        body_id,
        content_height,
        &unpaginated_overflow,
        &state.kept_margin_ends,
    );
    state.table_headers.index();
    document.table_objects.headers = std::mem::take(&mut state.table_headers);

    state.reach_overflow_pages(document, &env, content_height)?;

    scale_percent_widths(
        document,
        cascade,
        page_widths,
        content_width,
        &page_origins,
        control,
    )
    .and_then(|()| control.check_discovery_page_index(state.reach.max))?;
    note_fixed_static_page_origins(document, cascade, &page_origins);
    Ok(page_slices(
        control.last_page(state.reach.max),
        &page_origins,
        &state.page_names,
    ))
}

/// Notes where the page begins that holds the static position of each fixed
/// box placed there along the block axis, when that page is not the first.
/// The paint draws such a box from that page on, at the same place on each,
/// as Chromium does; a box placed by an inset repeats on every page instead.
fn note_fixed_static_page_origins(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    page_origins: &PageOrigins,
) {
    if document.context.pagination_on_transposed_axes {
        return;
    }
    let origins: Vec<(usize, f32)> = document
        .oof_coordinates
        .keys()
        .copied()
        .filter(|&id| {
            let node = &document.nodes[id];
            node.kind() == NodeKind::Element
                && node.is_in_document()
                && node.display() != DisplayValue::None
                && node.position() == &PositionValue::Fixed
                && inset_is_auto(&cascade.computed[id].top)
                && inset_is_auto(&cascade.computed[id].bottom)
        })
        .filter_map(|id| {
            let origin =
                page_origins.origin(page_origins.page_index_for_y(current_abs_y(document, id)));
            (origin > 0.0).then_some((id, origin))
        })
        .collect();
    document.fixed_static_page_origins.extend(origins);
}

/// The pages from the first through `last_page`, each starting at its origin
/// in the flow and carrying the page type `page_names` gave it, if any.
fn page_slices(
    last_page: u32,
    page_origins: &PageOrigins,
    page_names: &PageNames,
) -> Vec<PageSlice> {
    (0..=last_page)
        .map(|page_index| PageSlice {
            page_index,
            content_origin_y: page_origins.origin(page_index),
            page_name: page_names
                .by_page
                .get(page_index as usize)
                .cloned()
                .flatten(),
        })
        .collect()
}

/// Where each table row and caption among `candidates` sits in its parent
/// before pagination; the table boxes around the ones that moved follow them
/// once every one is placed.
fn table_row_origins(document: &Document, candidates: &[PageCandidate]) -> Vec<(usize, f32)> {
    candidates
        .iter()
        .filter(|candidate| {
            candidate.context.is_table_row() || candidate.context.is_table_caption()
        })
        .map(|candidate| {
            let row = candidate.node_id;
            (row, document.pagination_local_y(row))
        })
        .collect()
}

/// The first row of a table without a caption above its grid carries its
/// offset below the table's top: when the row starts the next page, the
/// table's spacing and borders above it move with it.
///
/// Returns that offset for each such first row.
fn table_row_leads(
    document: &Document,
    cascade: &CascadeResult,
    candidates: &[PageCandidate],
    root_flow_offset: f32,
) -> HashMap<usize, f32> {
    let mut table_row_leads = HashMap::<usize, f32>::new();
    if candidates
        .iter()
        .any(|candidate| candidate.context.is_table_row())
    {
        let candidate_y: HashMap<usize, f32> = candidates
            .iter()
            .map(|candidate| (candidate.node_id, candidate.raw_y))
            .collect();
        // Each table's first row in visual order: the table layout places a
        // footer group after the body groups wherever it is in the source.
        let mut table_first_rows = HashMap::<usize, (usize, f32)>::new();
        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.context.is_table_row())
        {
            if let Some(table) = document.table_of_row(candidate.node_id) {
                let first = table_first_rows
                    .entry(table)
                    .or_insert((candidate.node_id, candidate.raw_y));
                if candidate.raw_y < first.1 {
                    *first = (candidate.node_id, candidate.raw_y); // cov:ignore: rows are collected in visual order, so a later row never starts higher.
                } // cov:ignore: closes the visual-order guard above.
            }
        }
        for (table, (row, row_y)) in table_first_rows {
            if !rows_start_the_grid(document, cascade, table)
                || starts_with_loose_cells(document, table)
            {
                continue;
            }
            let table_y = candidate_y
                .get(&table)
                .copied()
                .unwrap_or_else(|| root_flow_offset + current_abs_y(document, table));
            table_row_leads.insert(row, (row_y - table_y).max(0.0));
        }
    }
    table_row_leads
}

/// The footer group of the table that the row `row` belongs to, when the row
/// is in one. A footer group around the table itself is not the row's.
fn footer_group_of_row(document: &Document, row: usize) -> Option<usize> {
    let boundary = document
        .table_of_row(row)
        .map(|table| document.table_parts_boundary(table));
    document
        .inclusive_ancestors(row)
        .take_while(|&id| {
            Some(id) != boundary
                && !matches!(
                    document.nodes[id].display(),
                    DisplayValue::Table | DisplayValue::InlineTable
                )
        })
        .find(|&id| document.nodes[id].display() == DisplayValue::TableFooterGroup)
}

/// Whether the anonymous table `table` starts with cells outside any row,
/// which table fixup puts into an anonymous row of their own before the rows.
fn starts_with_loose_cells(document: &Document, table: usize) -> bool {
    document
        .anonymous_table_parts(table)
        .and_then(|parts| parts.first())
        .is_some_and(|&first| document.nodes[first].display() == DisplayValue::TableCell)
}

/// Whether the rows of `table` start the table's grid: nothing comes before
/// the first row.
///
/// A caption above the grid stays on the current page, one inside a
/// `display: contents` child of the table included; an absolutely positioned
/// one is out of the table's flow. Content outside the rows, wrapped in an
/// anonymous row, may come before the first row.
fn rows_start_the_grid(document: &Document, cascade: &CascadeResult, table: usize) -> bool {
    let top_caption = document
        .children_through_contents(table)
        .iter()
        .any(|&child| {
            document.nodes[child].display() == DisplayValue::TableCaption
                && cascade.computed[child].caption_side
                    == raikiri_style::property::CaptionSideValue::Top
                && document.nodes[child].layout_position() != Position::Absolute
        });
    !(top_caption
        || document.nodes[table].children.iter().any(|&child| {
            document.nodes[child].display() != DisplayValue::TableRow
                && document.anonymous_table_cells(child).next().is_some()
        })
        || document.anonymous_table_cells(table).next().is_some())
}

/// The block end, before pagination, of the first row of each plain block
/// table among `candidates` whose rows start its grid, below a header that
/// repeats: the part of the table that a box tied to the one before it
/// cannot be broken away from. That is the row up to the first break that
/// splits it, or the whole row when none does. A first row in a row group
/// that avoids breaks inside it takes the whole group along (`row_groups`).
fn first_row_ends(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    candidates: &[PageCandidate],
    table_headers: &HeaderRepeats,
    row_groups: &HashMap<usize, (f32, f32)>,
    body_id: usize,
) -> HashMap<usize, f32> {
    // Each table's first row in visual order, with the block end of the row:
    // the table layout places a footer group after the body groups wherever
    // it is in the source.
    let mut first_rows = HashMap::<usize, (usize, f32)>::new();
    let mut row_tops = HashMap::<usize, f32>::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.context.is_table_row())
    {
        // The rows of an inline table, nested in a cell or not, are not rows
        // of a block table: the plain-table filter below drops that table.
        // A header that repeats goes with the first row below it.
        if table_headers.owner(candidate.node_id).is_some() {
            continue;
        }
        if let Some(table) = document.table_of_row(candidate.node_id) {
            row_tops.insert(candidate.node_id, candidate.raw_y);
            let end = candidate.raw_y + candidate.height;
            first_rows
                .entry(table)
                .and_modify(|first| {
                    if end < first.1 {
                        *first = (candidate.node_id, end);
                    }
                })
                .or_insert((candidate.node_id, end));
        }
    }
    // Cells outside any row: table fixup puts the ones that start an
    // anonymous table into an anonymous row, which ends where the last of
    // them does.
    let row_tables: Vec<usize> = candidates
        .iter()
        .filter_map(|candidate| {
            let table = document.anonymous_table_of(candidate.node_id)?;
            (starts_with_loose_cells(document, table)
                && document.anonymous_table_parts(table)?.first() == Some(&candidate.node_id))
            .then_some(table)
        })
        .collect();
    if !row_tables.is_empty() {
        let ends: HashMap<usize, f32> = candidates
            .iter()
            .map(|candidate| (candidate.node_id, candidate.raw_y + candidate.height))
            .collect();
        for table in row_tables {
            let parts = document.anonymous_table_parts(table).unwrap_or_default();
            let end = parts
                .iter()
                .take_while(|&&part| document.nodes[part].display() == DisplayValue::TableCell)
                .filter_map(|part| ends.get(part).copied())
                .fold(f32::NEG_INFINITY, f32::max);
            first_rows.insert(table, (parts[0], end));
        }
    }
    first_rows
        .into_iter()
        .filter(|&(table, (row, end))| {
            end.is_finite()
                && is_plain_table_box(document, table)
                && rows_start_the_grid(document, cascade, table)
                && !forced_break_before_first_row(document, cascade, table, row)
        })
        .map(|(table, (row, end))| {
            // A row that a break can split holds only its part before the
            // first such break (see `table_cell_fragments::first_split_end`).
            // A table that repeats its footer keeps its rows whole.
            let split = (!row_groups.contains_key(&row)
                && document.nodes[row].display() == DisplayValue::TableRow
                && !has_footer_group(document, table))
            .then(|| table_cell_fragments::first_split_end(document, cascade, body_id, row))
            .flatten();
            let end = match (split, row_tops.get(&row)) {
                (Some(split), Some(&top)) => (top + split).min(end),
                _ => end,
            };
            let group_end = row_groups
                .get(&row)
                .map_or(end, |&(group_end, _)| group_end);
            // An anonymous table has no box of its own among the candidates:
            // its first part stands for it, and so does that part's own
            // `break-inside`, since the table has none.
            let key = document
                .anonymous_table_parts(table)
                .and_then(|parts| parts.first().copied())
                .unwrap_or(table);
            (key, end.max(group_end))
        })
        .collect()
}

/// Whether a forced page break comes before `row`, the first row of `table`:
/// its own `break-before` or one it takes from its cells (see
/// [`forces_page_break`]), or that of the row group around it. The page edge
/// constraint of the table does not look inside it, so the box before the
/// table would be tied to a row that starts a page of its own.
fn forced_break_before_first_row(
    document: &Document,
    cascade: &CascadeResult,
    table: usize,
    row: usize,
) -> bool {
    let boundary = document.table_parts_boundary(table);
    forces_page_break(document, cascade, row, true)
        || document
            .parent_of(row)
            .filter(|&group| group != boundary)
            .is_some_and(|group| {
                page_break_is_forced(
                    document,
                    cascade,
                    group,
                    cascade.computed[group].break_before,
                )
            })
}

/// Whether `node_id` is a block-level table that stacks in the normal flow of
/// a horizontal block container.
fn is_plain_table_box(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    node.kind() == NodeKind::Element
        && node.display() == DisplayValue::Table
        && matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative
        )
        && !node.used_writing_mode.is_vertical()
}

/// Whether `caption` is a caption of a block table that stacks in plain block
/// flow up to `body_id`, outside any table cell, float and out-of-flow box,
/// and in no flex or grid item but one alone in its container: its lines, and
/// those of the blocks inside it, can be broken between at a page end like
/// those of a paragraph, and the rows and captions after it follow the moved
/// lines.
pub(super) fn is_flow_table_caption(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    caption: usize,
    body_id: usize,
) -> bool {
    caption_table(document, caption)
        .is_some_and(|table| placed_in_flow_to_body(document, cascade, table, body_id, true, false))
}

/// The block table of `caption` when `caption` is a horizontal table caption
/// of a table that [`is_plain_table_box`] takes.
fn caption_table(document: &Document, caption: usize) -> Option<usize> {
    (document.nodes[caption].display() == DisplayValue::TableCaption
        && !document.nodes[caption].used_writing_mode.is_vertical())
    .then(|| document.layout_parent_of(caption))
    .flatten()
    .filter(|&table| is_plain_table_box(document, table))
}

/// A table row group with `break-inside: avoid` starts on the next page
/// when it would cross the current one. Its first row carries the block
/// end of the group's rows up to the first forced break between them (a
/// forced break already splits the group).
///
/// Returns, for each such first row, that block end and the row's table
/// lead.
fn avoided_row_groups(
    document: &Document,
    cascade: &CascadeResult,
    candidates: &[PageCandidate],
    table_headers: &HeaderRepeats,
    table_row_leads: &HashMap<usize, f32>,
) -> HashMap<usize, (f32, f32)> {
    let mut avoided_row_groups = HashMap::<usize, (f32, f32)>::new();
    // A header group repeated on each page is placed by the repetition
    // path; any other header group avoids breaks like a body group.
    let avoiding_row_group = |group: usize| {
        (matches!(
            document.nodes[group].display(),
            DisplayValue::TableRowGroup | DisplayValue::TableFooterGroup
        ) || (document.nodes[group].display() == DisplayValue::TableHeaderGroup
            && !table_headers.headers.contains_key(&group)))
            && avoids_page_break_inside(cascade.computed[group].break_inside)
    };
    if candidates.iter().any(|candidate| {
        candidate.context.is_table_row()
            && document
                .parent_of(candidate.node_id)
                .is_some_and(avoiding_row_group)
    }) {
        let mut group_first_rows = HashMap::<usize, Option<usize>>::new();
        // The last row seen in each row group: rows of a nested table are
        // visited between two rows of the outer group.
        let mut previous_rows = HashMap::<Option<usize>, usize>::new();
        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.context.is_table_row())
        {
            let row = candidate.node_id;
            let forced_before = forces_page_break(document, cascade, row, true)
                || previous_rows
                    .insert(document.parent_of(row), row)
                    .is_some_and(|previous| forces_page_break(document, cascade, previous, false));
            let Some(group) = document
                .parent_of(row)
                .filter(|&group| avoiding_row_group(group))
            else {
                continue;
            };
            let entry = group_first_rows.entry(group);
            let first = match entry {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    let lead = table_row_leads.get(&row).copied().unwrap_or(0.0);
                    avoided_row_groups.insert(row, (candidate.raw_y + candidate.height, lead));
                    slot.insert(Some(row));
                    continue;
                }
                std::collections::hash_map::Entry::Occupied(slot) => *slot.get(),
            };
            let Some(first) = first else {
                continue;
            };
            if forced_before {
                group_first_rows.insert(group, None);
                continue;
            }
            if let Some((end, _)) = avoided_row_groups.get_mut(&first) {
                *end = end.max(candidate.raw_y + candidate.height);
            }
        }
    }
    avoided_row_groups
}

/// The painted end of an avoided run and an optional direct-flow alternative
/// whose content fits before the closing edges of its last box.
struct AvoidedRunEnd {
    end: f32,
    /// Whether the page holds no break inside the run at a better place than
    /// before it (see [`HeldPart::firm`]): the run then moves to the next
    /// page even when it does not fit there.
    firm: bool,
    closing: Option<ClosingRun>,
    /// Whether the run starts inside the first box of the run (see
    /// [`run_break_start`]), at a box that the page may break inside: when
    /// that box crosses the page end itself, the break inside it is the later
    /// one, and the run does not move.
    breaks_inside_start: bool,
    /// The end of a shorter run that starts at the same box, and whether it
    /// is firm: when the longer run does not move, the shorter one still
    /// moves on its own where it crosses the page end.
    shorter: Option<(f32, bool)>,
}

/// Normal-flow spans from a direct sibling run's first border-box top.
struct ClosingRun {
    height: f32,
    content_height: f32,
}

impl ClosingRun {
    /// Closing edges can displace the run when only those edges overflow.
    /// Once the run reaches the next page, its last box can move alone.
    fn moves(&self, fit: &BoxFit) -> bool {
        self.content_height.is_finite()
            && self.content_height > 0.0
            && fit.flow_y + self.content_height <= fit.page_end + 0.001
            && self.content_height + fit.margin_top.max(0.0) <= fit.next_page_step + 0.001
            && avoided_box_moves(
                fit.flow_y,
                fit.margin_top,
                fit.flow_y + self.height,
                fit.page_start,
                fit.page_end,
            )
    }
}

/// The box before which the page takes the last break that the block
/// `node_id` holds between boxes of its own, when the block starts a run of
/// boxes tied by avoided breaks: Chromium breaks there, inside the first box
/// of the run, rather than before it, as it takes the latest of equal
/// breaks. The boxes of the block are searched from the last: a break inside
/// a box comes after the one before it, and a break between two boxes that
/// avoided breaks tie is no place to break. Blocks of blocks and flex
/// containers of one column are searched, and a table breaks before its last
/// row (see [`last_row_break`]); a paragraph, a box that avoids breaks inside
/// it, a multicol container and any other box hold no break here.
fn run_break_start(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<usize> {
    // A table that avoids breaks inside it still takes the breaks forced
    // between its rows (see `last_row_break`).
    if is_plain_table_box(document, node_id)
        && (may_break_inside(document, cascade, node_id)
            || avoids_page_break_inside(cascade.computed[node_id].break_inside)
                && !clips_at_bounded_height(document, cascade, node_id)
                && !is_monolithic_for_pagination_with_transposed_axes(
                    document,
                    cascade,
                    node_id,
                    document.context.pagination_on_transposed_axes,
                ))
    {
        return last_row_break(document, cascade, node_id);
    }
    if !may_break_inside(document, cascade, node_id) {
        return None;
    }
    if !stacks_boxes(document, cascade, node_id) {
        return None;
    }
    let boxes = flow_boxes(document, cascade, node_id);
    for (index, &current) in boxes.iter().enumerate().rev() {
        if let Some(inner) = run_break_start(document, cascade, current) {
            return Some(inner);
        }
        let &before = index.checked_sub(1).and_then(|index| boxes.get(index))?;
        if !boxes_are_tied(document, cascade, before, current) {
            // A table that moves whole starts the run as a block does.
            return (is_plain_block_box(document, current)
                || is_plain_table_box(document, current))
            .then_some(current);
        }
    }
    None
}

/// Whether the box `node_id` stacks the boxes of its flow one below another
/// as a block of blocks does: a plain block that lays out no lines and no
/// columns, or a flex container of one column.
fn stacks_boxes(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let node = &document.nodes[node_id];
    let computed = &cascade.computed[node_id];
    let column_flex = is_plain_container_box(document, node_id)
        && node.display() == DisplayValue::Flex
        && computed.flex_direction == FlexDirectionValue::Column
        && computed.flex_wrap == FlexWrapValue::NoWrap;
    (is_plain_block_box(document, node_id) || column_flex)
        && !node.is_ifc_root()
        && node.multicol.is_none()
}

/// The in-flow boxes of the flow of the box `node_id`, in pagination order,
/// past floats and through `display: contents` elements.
fn flow_boxes(document: &Document, cascade: &CascadeResult, node_id: usize) -> Vec<usize> {
    let mut boxes = Vec::new();
    for child in pagination_child_order(document, cascade, node_id)
        .into_iter()
        .rev()
    {
        if !is_floating_box_for_pagination(document, cascade, child) {
            push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
        }
    }
    boxes.reverse();
    boxes
}

/// The box whose flow holds the box `node_id`, past `display: contents`
/// elements.
fn flow_parent(document: &Document, node_id: usize) -> Option<usize> {
    let mut parent = document.nodes[node_id].parent?;
    while document.nodes[parent].display() == DisplayValue::Contents {
        parent = document.nodes[parent].parent?;
    }
    Some(parent)
}

/// Whether the page holds no break inside the part of the box `outer` from
/// the top of the box `start` in it (see [`run_break_start`]) to its end:
/// neither inside `start` nor inside or between the boxes after it.
fn holds_no_break_from(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    outer: usize,
    start: usize,
) -> bool {
    let next = |&id: &usize| next_flow_box(document, cascade, id);
    box_holds_no_break(document, cascade, body_id, start)
        && std::iter::successors(Some(start), |&id| flow_parent(document, id))
            .take_while(|&id| id != outer)
            .all(|current| {
                std::iter::successors(next(&current), next)
                    .all(|id| box_holds_no_break(document, cascade, body_id, id))
            })
}

/// Whether the page may break inside the box `node_id`: it neither avoids
/// breaks inside it, nor is monolithic, nor clips its overflow at a height of
/// its own, which Chromium treats as monolithic too.
fn may_break_inside(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    !avoids_page_break_inside(cascade.computed[node_id].break_inside)
        && !clips_at_bounded_height(document, cascade, node_id)
        && !is_monolithic_for_pagination_with_transposed_axes(
            document,
            cascade,
            node_id,
            document.context.pagination_on_transposed_axes,
        )
}

/// The part of the table `table` before which the page breaks when the table
/// starts a run of boxes tied by avoided breaks, as in Chromium: the part
/// that the table ends with in visual order, a caption below the rows, its
/// footer group or its last row, together with the parts before it that it
/// may not break from. Walking back from the last row, a break between two
/// rows that is avoided by the rows or their row groups, or that falls
/// inside a row group (or a table) that avoids breaks inside it, ties the
/// rows together, unless a break is forced there. `None` when the table
/// moves whole: the walk reaches its first row.
fn last_row_break(document: &Document, cascade: &CascadeResult, table: usize) -> Option<usize> {
    let avoids_inside = |id: usize| avoids_page_break_inside(cascade.computed[id].break_inside);
    let table_avoids_inside = avoids_inside(table);
    // Each row with the row group that holds it, if any, in visual order.
    let rows = body_rows(document, table);
    let &(last, last_group) = rows.last()?;
    // A caption below the rows is the last part, unless a break it may not
    // take ties it to the caption or row above it: walking back over the
    // captions, the first one that the page may break before starts the last
    // part; when every one is tied, they go with the rows tied to the last
    // row. A table that avoids breaks inside it ties them too, unless a break
    // is forced there.
    let tied = |edges: &[(usize, bool)]| {
        let (forced, avoided) =
            edges
                .iter()
                .fold((false, false), |(forced, avoided), &(id, before)| {
                    let (own_forced, own_avoided) =
                        page_edge_constraint(document, cascade, id, before);
                    (
                        forced || own_forced || forces_page_break(document, cascade, id, before),
                        avoided || own_avoided,
                    )
                });
        !forced && (avoided || table_avoids_inside)
    };
    let footer_rows = footer_rows(document, table);
    let captions = bottom_captions(document, table, last);
    for (index, &caption) in captions.iter().enumerate().rev() {
        let mut edges = vec![(caption, true)];
        match index.checked_sub(1).map(|above| captions[above]) {
            Some(above) => edges.push((above, false)),
            // The footer group lies between the rows and the captions.
            None => match footer_rows.last() {
                Some(&footer_row) => {
                    edges.push((footer_row, false));
                    edges.extend(document.parent_of(footer_row).map(|group| (group, false)));
                }
                None => {
                    edges.push((last, false));
                    edges.extend(last_group.map(|group| (group, false)));
                }
            },
        }
        if !tied(&edges) {
            return Some(caption);
        }
    }
    // A footer group is the last part on its own, unless a break it may not
    // take ties it to the last row: then it goes with the rows tied to that
    // row.
    if let Some(&footer_row) = footer_rows.first() {
        let footer = document.parent_of(footer_row).unwrap_or(table);
        let mut edges = vec![(footer, true), (footer_row, true), (last, false)];
        edges.extend(last_group.map(|group| (group, false)));
        if !tied(&edges) {
            return Some(footer_row);
        }
    }
    tied_rows_start(document, cascade, &rows, table_avoids_inside)
}

/// When `start` is the first row of a repeated footer group, which starts a
/// run of boxes tied by avoided breaks, the first of the body rows that the
/// last one may not break from (see [`tied_rows_start`]), and how far below
/// that row's top the footer ends. The footer moves on with the run only
/// when it fits below the rows; when it does not, the page breaks before
/// those rows instead, as in Chromium, which closes the page with a copy of
/// the footer.
fn footer_guard(
    document: &Document,
    cascade: &CascadeResult,
    start: usize,
) -> Option<(usize, f32)> {
    let footer = document
        .parent_of(start)
        .filter(|&group| document.nodes[group].display() == DisplayValue::TableFooterGroup)?;
    let table = document.table_of_row(start)?;
    let rows = body_rows(document, table);
    let avoids_inside = avoids_page_break_inside(cascade.computed[table].break_inside);
    let row = tied_rows_start(document, cascade, &rows, avoids_inside)
        .or_else(|| rows.first().map(|&(row, _)| row))?;
    let footer_end = current_abs_y(document, footer) + document.nodes[footer].layout.size.height;
    Some((row, footer_end - current_abs_y(document, row)))
}

/// The rows of the footer group of the table `table`, in order.
fn footer_rows(document: &Document, table: usize) -> Vec<usize> {
    document
        .children_through_contents(table)
        .into_iter()
        .find(|&child| document.nodes[child].display() == DisplayValue::TableFooterGroup)
        .map(|footer| {
            document
                .children_through_contents(footer)
                .into_iter()
                .filter(|&row| document.nodes[row].display() == DisplayValue::TableRow)
                .collect()
        })
        .unwrap_or_default()
}

/// The captions of the table `table` below its rows, whose last row is
/// `last`, from top to bottom: the captions that start at or below the row's
/// end.
fn bottom_captions(document: &Document, table: usize, last: usize) -> Vec<usize> {
    let top = |id: usize| current_abs_y(document, id);
    let mut captions: Vec<usize> = document
        .children_through_contents(table)
        .into_iter()
        .filter(|&child| document.nodes[child].display() == DisplayValue::TableCaption)
        .filter(|&caption| {
            top(caption) >= top(last) + document.nodes[last].layout.size.height - 0.001
        })
        .collect();
    captions.sort_by(|&a, &b| top(a).total_cmp(&top(b)));
    captions
}

/// The lowest caption of the table `table` below its rows (see
/// [`bottom_captions`]).
fn bottom_caption(document: &Document, table: usize, last: usize) -> Option<usize> {
    bottom_captions(document, table, last).last().copied()
}

/// The first of the `rows` (each with the row group that holds it, if any,
/// in visual order) that the last row may not break from: walking back from
/// the last row, a break between two rows that the rows or their row groups
/// avoid, that falls inside a row group that avoids breaks inside it, or any
/// break when `all_avoided`, ties the two rows together unless a break is
/// forced there. `None` when the walk reaches the first row.
fn tied_rows_start(
    document: &Document,
    cascade: &CascadeResult,
    rows: &[(usize, Option<usize>)],
    all_avoided: bool,
) -> Option<usize> {
    let avoids_inside = |id: usize| avoids_page_break_inside(cascade.computed[id].break_inside);
    rows.windows(2)
        .rev()
        .find(|pair| {
            let ((before, before_group), (after, after_group)) = (pair[0], pair[1]);
            let mut edges = vec![(before, false), (after, true)];
            if before_group != after_group {
                edges.extend(before_group.map(|group| (group, false)));
                edges.extend(after_group.map(|group| (group, true)));
            }
            let (forced, avoided) =
                edges
                    .into_iter()
                    .fold((false, false), |(forced, avoided), (id, before_edge)| {
                        let (own_forced, own_avoided) =
                            page_edge_constraint(document, cascade, id, before_edge);
                        (
                            forced
                                || own_forced
                                || forces_page_break(document, cascade, id, before_edge),
                            avoided || own_avoided,
                        )
                    });
            let avoided = avoided
                || all_avoided
                || (before_group == after_group && after_group.is_some_and(avoids_inside));
            forced || !avoided
        })
        .map(|pair| pair[1].0)
}

/// The rows of the table `table` outside its header and footer groups, each
/// with the row group that holds it, if any, in visual order.
fn body_rows(document: &Document, table: usize) -> Vec<(usize, Option<usize>)> {
    let mut rows = Vec::new();
    for child in document.children_through_contents(table) {
        match document.nodes[child].display() {
            DisplayValue::TableRowGroup => rows.extend(
                document
                    .children_through_contents(child)
                    .into_iter()
                    .filter(|&row| document.nodes[row].display() == DisplayValue::TableRow)
                    .map(|row| (row, Some(child))),
            ),
            DisplayValue::TableRow => rows.push((child, None)),
            _ => {}
        }
    }
    rows.sort_by(|&(a, _), &(b, _)| {
        current_abs_y(document, a).total_cmp(&current_abs_y(document, b))
    });
    rows
}

/// Where a box directly in the body breaks when it starts a run of boxes
/// tied by avoided breaks (see [`run_break_start`]). An anonymous table,
/// which wraps rows that are its parts, breaks before its last row when the
/// rows before it leave a break there.
fn body_table_break_start(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    table: usize,
) -> Option<usize> {
    let Some(parts) = document.anonymous_table_parts(table) else {
        return run_break_start(document, cascade, table);
    };
    // The rows among the parts, and the rows of the row groups among them
    // with their group.
    let rows: Vec<(usize, Option<usize>)> = parts
        .iter()
        .flat_map(|&part| match document.nodes[part].display() {
            DisplayValue::TableRow => vec![(part, None)],
            DisplayValue::TableRowGroup => document
                .children_through_contents(part)
                .into_iter()
                .filter(|&row| document.nodes[row].display() == DisplayValue::TableRow)
                .map(|row| (row, Some(part)))
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    tied_rows_start(document, cascade, &rows, false)
}

/// Whether the table `table` starts a run of boxes tied by avoided breaks
/// with the box after it, from its last part (see [`last_row_break`]) or
/// whole. A table whose last part forces a break after it, which ends the
/// table's page, starts none.
fn table_starts_run(document: &Document, cascade: &CascadeResult, table: usize) -> bool {
    if let Some(parts) = document.anonymous_table_parts(table) {
        return parts
            .last()
            .is_none_or(|&last| !forces_page_break(document, cascade, last, false));
    }
    body_rows(document, table)
        .last()
        .is_none_or(|&(last, group)| {
            if let Some(caption) = bottom_caption(document, table, last) {
                return !forces_page_break(document, cascade, caption, false);
            }
            let footer_rows = footer_rows(document, table);
            if let Some(&footer_row) = footer_rows.last() {
                let footer = document.parent_of(footer_row).unwrap_or(table);
                // A break forced between the footer's rows ends its first
                // part on the page; the rest starts the next page on its own.
                let forced_inside = footer_rows.windows(2).any(|pair| {
                    forces_page_break(document, cascade, pair[0], false)
                        || forces_page_break(document, cascade, pair[1], true)
                });
                return !forced_inside
                    && !forces_page_break(document, cascade, footer_row, false)
                    && !forces_page_break(document, cascade, footer, false);
            }
            !forces_page_break(document, cascade, last, false)
                && !group.is_some_and(|group| {
                    document
                        .children_through_contents(group)
                        .into_iter()
                        .rev()
                        .find(|&row| document.nodes[row].display() == DisplayValue::TableRow)
                        == Some(last)
                        && forces_page_break(document, cascade, group, false)
                })
        })
}

/// The next in-flow box after the box `node_id` in the flow of its parent,
/// past floats and through `display: contents` elements on either side.
fn next_flow_box(document: &Document, cascade: &CascadeResult, node_id: usize) -> Option<usize> {
    let boxes = flow_boxes(document, cascade, flow_parent(document, node_id)?);
    let index = boxes.iter().position(|&id| id == node_id)?;
    boxes.get(index + 1).copied()
}

/// Where the blocks of the document start the runs of boxes tied by avoided
/// breaks that they open with the box after them (see [`run_break_start`]),
/// found once for each block, with the next in-flow box of each box found
/// once for each flow.
#[derive(Default)]
pub(super) struct RunBreakStarts {
    next: HashMap<usize, Option<usize>>,
    starts: HashMap<usize, Option<usize>>,
}

impl RunBreakStarts {
    /// The next in-flow box after `node_id` in the flow of its parent (see
    /// [`next_flow_box`]).
    fn next_flow_box(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
    ) -> Option<usize> {
        if !self.next.contains_key(&node_id)
            && let Some(parent) = flow_parent(document, node_id)
        {
            let boxes = flow_boxes(document, cascade, parent);
            for (index, &id) in boxes.iter().enumerate() {
                self.next.insert(id, boxes.get(index + 1).copied());
            }
        }
        *self.next.entry(node_id).or_insert(None)
    }

    /// The box where the block `block` starts the run it opens with the box
    /// after it, if avoided breaks tie the two.
    fn start(
        &mut self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        block: usize,
    ) -> Option<usize> {
        if let Some(&start) = self.starts.get(&block) {
            return start;
        }
        let start = self
            .next_flow_box(document, cascade, block)
            .filter(|&next| {
                document.nodes[next].kind() == NodeKind::Element
                    && boxes_are_tied(document, cascade, block, next)
            })
            .and_then(|_| run_break_start(document, cascade, block));
        self.starts.insert(block, start);
        start
    }

    /// Whether the box `node_id` is where a block around it, below the body,
    /// starts the run of boxes tied by avoided breaks that it opens with the
    /// box after it: the page may break before it then, so the walk looks at
    /// it.
    pub(super) fn starts_break(
        &mut self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
        node_id: usize,
    ) -> bool {
        document
            .ancestors(node_id)
            .take_while(|&ancestor| ancestor != body_id)
            .filter(|&ancestor| stacks_boxes(document, cascade, ancestor))
            .any(|ancestor| self.start(document, cascade, ancestor) == Some(node_id))
    }
}

/// Connected class-A sibling runs have a legal break before their first
/// box. Pagination moves a run when it fits in one page, or when only the
/// closing edges of its last lone box overflow. Otherwise it relaxes the
/// run's avoidance constraints so an oversized run still makes progress.
///
/// Returns the block end of each run, including its last box's block-end
/// margin, keyed by its first box. A box inside a block that starts a run among
/// the boxes of its parent, or holds one at the start of its own boxes, is
/// keyed the same way, by the end its candidate carries (`avoided_unit`).
fn avoided_run_ends(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    candidates: &[PageCandidate],
    table_headers: &HeaderRepeats,
    row_groups: &HashMap<usize, (f32, f32)>,
    body_id: usize,
    content_width: f32,
) -> HashMap<usize, AvoidedRunEnd> {
    let first_row_ends = first_row_ends(
        document,
        cascade,
        candidates,
        table_headers,
        row_groups,
        body_id,
    );
    // The last box of each run, keyed by the run's first box, and the margin
    // below it.
    let mut run_lasts = HashMap::<usize, (&PageCandidate, f32)>::new();
    // The runs with a box before their last that holds a break of its own
    // (see `box_holds_no_break`), keyed by their first box.
    let mut soft_runs = std::collections::HashSet::<usize>::new();
    let mut previous: Option<&PageCandidate> = None;
    let mut run_start: Option<usize> = None;
    for candidate in candidates {
        if !(candidate.context.is_direct_body_element()
            || document.nodes[candidate.node_id].kind() == NodeKind::Text
                && BoxTree::for_layout(document)
                    .parent(candidate.node_id, BoxParentScope::Document)
                    == Some(body_id))
        {
            continue;
        }
        // The parts of an anonymous table after its first, which stands for
        // the table, lie inside it.
        if document
            .anonymous_table_of(candidate.node_id)
            .and_then(|table| document.anonymous_table_parts(table))
            .is_some_and(|parts| parts.first() != Some(&candidate.node_id))
        {
            continue;
        }
        let computed = &cascade.computed[candidate.node_id];
        // Floats and positioned boxes form parallel flows. Their placement
        // does not interrupt a connected run of normal-flow siblings.
        if is_floating_box_for_pagination(document, cascade, candidate.node_id)
            || matches!(
                document.nodes[candidate.node_id].position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
        {
            continue;
        }
        let eligible = candidate.context.is_direct_body_element()
            && (document.nodes[candidate.node_id]
                .display()
                .is_block_flex_or_grid_container()
                || matches!(
                    document.nodes[candidate.node_id].display(),
                    DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
                ))
            && !matches!(
                document.nodes[candidate.node_id].position(),
                PositionValue::Running(_)
            );
        // A table joins a run by its first row, which stays with the box
        // before it, and starts one by its last part, which stays with the
        // box after it (see `last_row_break`).
        let ends_run = !eligible
            && candidate.context.is_direct_body_element()
            && first_row_ends.contains_key(&candidate.node_id);
        if !eligible && !ends_run {
            previous = None;
            run_start = None;
            continue;
        }
        // An anonymous table, keyed by its first part, ends with its last.
        let connected = previous.is_some_and(|prior| {
            let before = document
                .anonymous_table_of(prior.node_id)
                .and_then(|table| document.anonymous_table_parts(table))
                .and_then(|parts| parts.last().copied())
                .unwrap_or(prior.node_id);
            boxes_are_tied(document, cascade, before, candidate.node_id)
        });
        // A paragraph in the run that breaks between its lines is a better
        // place to break than before the run: the run starts after it, and
        // the paragraph keeps its last lines with the boxes after it.
        let splits_lines = |id: usize| paragraph_may_split_lines(document, cascade, id, false);
        if connected && previous.is_some_and(|prior| splits_lines(prior.node_id)) {
            run_start = None;
        } else if let Some(prior) = previous.filter(|_| connected) {
            let start = *run_start.get_or_insert(prior.node_id);
            // The first box is told apart below, once it is known where in
            // it the run starts.
            if prior.node_id != start
                && !box_holds_no_break(document, cascade, body_id, prior.node_id)
            {
                soft_runs.insert(start);
            }
            let margin_bottom =
                used_computed_length_percentage_or_auto(&computed.margin.bottom, content_width)
                    .unwrap_or(0.0)
                    .max(0.0);
            run_lasts.insert(start, (candidate, margin_bottom));
        } else {
            run_start = None;
        }
        // The key of an anonymous table is its first part.
        let table = document
            .anonymous_table_of(candidate.node_id)
            .unwrap_or(candidate.node_id);
        if ends_run && !table_starts_run(document, cascade, table) {
            previous = None;
            run_start = None;
        } else {
            // A table that breaks inside it starts a run of its own from
            // its last part; one that moves whole goes on with the run.
            if ends_run && body_table_break_start(document, cascade, table).is_some() {
                run_start = None;
            }
            previous = Some(candidate);
        }
    }
    let mut ends: HashMap<usize, AvoidedRunEnd> = run_lasts
        .into_iter()
        .map(|(run_first, (last, margin_bottom))| {
            // The last box of the run stays with the box before it only as far
            // as the page has to hold of it.
            let (head, firm) = match first_row_ends.get(&last.node_id) {
                // A table that avoids breaks inside it stays whole.
                Some(&row_end) => {
                    if avoids_page_break_inside(cascade.computed[last.node_id].break_inside) {
                        (None, true)
                    } else {
                        (Some(row_end - last.raw_y), false)
                    }
                }
                None => {
                    let held = avoided_run_head(document, cascade, body_id, last.node_id, true);
                    (held.end, held.firm)
                }
            };
            // A first box that holds breaks between boxes of its own starts
            // the run at the last of them (see `run_break_start`).
            // An anonymous table stands for itself by its first part.
            let first_box = document.anonymous_table_of(run_first).unwrap_or(run_first);
            let start = body_table_break_start(document, cascade, first_box).unwrap_or(run_first);
            let firm = firm
                && !soft_runs.contains(&run_first)
                && holds_no_break_from(document, cascade, body_id, run_first, start);
            let (end, whole) = match head {
                Some(head) if head < last.height - 0.001 => (last.raw_y + head, false),
                // Content that runs out past a box that avoids breaks
                // inside it.
                Some(head) if head > last.height + 0.001 => {
                    (last.raw_y + head.max(last.height + margin_bottom), true)
                }
                _ => (last.raw_y + last.height + margin_bottom, true),
            };
            // Only direct sibling runs gain this alternative. A run inside
            // a wrapper still uses its existing fragment-level constraints.
            let closing = whole
                .then(|| {
                    lone_box_with_floats(document, cascade, body_id, last.node_id, false, false)
                })
                .flatten()
                .filter(|lone| lone.tail > 0.001)
                .map(|lone| {
                    let flow_top = |id| {
                        current_abs_y(document, id)
                            - accumulated_relative_offset_y(document, cascade, body_id, id)
                    };
                    let span = flow_top(last.node_id) - flow_top(start) + last.height;
                    ClosingRun {
                        height: span,
                        content_height: span - lone.tail + lone.float_reach,
                    }
                });
            let breaks_inside_start =
                start != run_first && may_break_inside(document, cascade, start);
            (
                start,
                AvoidedRunEnd {
                    end,
                    firm,
                    closing,
                    breaks_inside_start,
                    shorter: None,
                },
            )
        })
        .collect();
    // A box that holds a run of its own at its start, or starts a run among
    // the boxes of its parent, carries where that run ends (see
    // `CandidateCollector`): it moves with the run when that crosses a page.
    for candidate in candidates {
        if let Some(unit) = candidate.context.avoided_unit() {
            let end = candidate.raw_y + unit.end;
            let firm = unit.firm;
            let breaks_inside_start = unit.breaks_inside;
            let unit = unit.end;
            ends.entry(candidate.node_id)
                .and_modify(|known| {
                    // The known run stays the longer one on a tie.
                    let mut runs = [(known.end, known.firm), (end, firm)];
                    runs.sort_by(|a, b| b.0.total_cmp(&a.0));
                    let [longer, shorter] = runs;
                    (known.end, known.firm) = longer;
                    if shorter.0 < longer.0 - 0.001
                        && known.shorter.is_none_or(|known| known.0 < shorter.0)
                    {
                        known.shorter = Some(shorter);
                    }
                    if let Some(closing) = &mut known.closing {
                        closing.height = closing.height.max(unit);
                        closing.content_height = closing.content_height.max(unit);
                    }
                })
                .or_insert(AvoidedRunEnd {
                    end,
                    firm,
                    closing: None,
                    breaks_inside_start,
                    shorter: None,
                });
        }
    }
    // A run that starts at a repeated footer keeps the rows above it from
    // breaking away from the footer (see `footer_guard`).
    let guards: Vec<(usize, f32)> = ends
        .keys()
        .filter(|&&start| table_headers.is_footer_source(start))
        .filter_map(|&start| footer_guard(document, cascade, start))
        .collect();
    for (row, offset) in guards {
        if let Some(candidate) = candidates.iter().find(|candidate| candidate.node_id == row) {
            ends.entry(row).or_insert(AvoidedRunEnd {
                end: candidate.raw_y + offset,
                firm: false,
                closing: None,
                breaks_inside_start: true,
                shorter: None,
            });
        }
    }
    ends
}

/// How far below the border-box top of a box the run of boxes tied by
/// avoided breaks ends that the box starts, and whether that run moves to
/// the next page even when it does not fit there (see [`HeldPart::firm`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RunUnit {
    pub(super) end: f32,
    pub(super) firm: bool,
    /// Whether the run starts at this box inside the first box of the run,
    /// and the page may break inside this box (see
    /// [`AvoidedRunEnd::breaks_inside_start`]).
    pub(super) breaks_inside: bool,
}

/// Whether the page holds no break inside the box `node_id` of a run of
/// boxes tied by avoided breaks that is better than the one before the run:
/// the part of it that [`avoided_run_head`] finds is firm and takes the
/// whole box, or the box has nothing in its flow to break between. A box of
/// the run with such a break inside it takes that break before the run moves
/// on, as Chromium takes the latest of equal breaks.
fn box_holds_no_break(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> bool {
    // A box with no content in its flow has no place inside it to break.
    if !document.nodes[node_id].is_ifc_root()
        && last_flow_child(document, cascade, node_id).is_none()
    {
        return true;
    }
    let held = avoided_run_head(document, cascade, body_id, node_id, false);
    held.firm
        && held
            .end
            .is_none_or(|end| end >= document.nodes[node_id].layout.size.height - 0.001)
}

/// The part of a block that the page cannot break away from the box before
/// it (see [`avoided_run_head`]).
struct HeldPart {
    /// How far below the border-box top of the block the part ends, or `None`
    /// when it is the whole block.
    end: Option<f32>,
    /// Whether the part reaches across an avoided break between two boxes
    /// inside the block.
    tied: bool,
    /// Whether no break inside the part is one the page could take: it ends
    /// with lines held for `orphans`, or with a box that avoids breaks inside
    /// it or that no break goes through. A part found otherwise, such as the
    /// whole of an empty block or the first row of a table, may hold breaks
    /// that Chromium takes instead of the one before the run.
    firm: bool,
}

/// How far below the border-box top of the block `node_id` the part of it ends
/// that the page cannot break away from the box before it, when an avoided
/// break ties the two, and whether that part holds boxes that avoided breaks
/// tie to each other. `end` is `None` when the part is the whole box.
///
/// A box tied to the one before it can still be broken inside: the page ends
/// within it, and the box before it stays with what the page holds of it
/// (CSS Break 3 §4.4). A paragraph that breaks between its lines holds the
/// first `orphans` of them, and one with no more lines than `orphans` moves
/// whole, so it holds all of them; what follows the last held line, such as the rest of a box taller than
/// its lines, flows on to the next page, but not the padding and border that
/// close the box right below it, which would be alone there. The first box of
/// a block of boxes holds the same, and with it the boxes after it that
/// avoided breaks tie to it, when `join_ties` is set. A table holds its first
/// row (see [`held_table_end`]), and a flex container that lays its items
/// out in rows the first piece of their content. A box that avoids breaks
/// inside it is kept whole, with the content that runs out past it (see
/// [`avoided_box_extent`]). A box that is not a plain block or table, a
/// paragraph that cannot move its lines down inside it, a block that moves
/// whole with its closing edges (see [`lone_box_tail`]), and anything with a
/// break, a float or generated content at its start is kept whole too.
/// Whether no break inside the part held beats the one before the run is
/// told by [`HeldPart::firm`].
fn avoided_run_head(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    join_ties: bool,
) -> HeldPart {
    let mut current = node_id;
    let mut offset = 0.0;
    let mut tied = false;
    // Whether `current` was reached across an avoided break, which then
    // belongs to the run that holds the part, not to the box.
    let mut joined = false;
    // What a clipped block holds is not drawn past its edge, so the part held
    // does not reach beyond it.
    let mut clip_end = f32::INFINITY;
    loop {
        if element_clips_overflow(document, cascade, current) {
            clip_end = clip_end.min(offset + clip_block_end(document, cascade, current, 0.0));
        }
        match held_step(
            document, cascade, body_id, node_id, current, joined, join_ties,
        ) {
            HeldStep::Whole { firm } if tied => {
                return HeldPart {
                    end: Some((offset + document.nodes[current].layout.size.height).min(clip_end)),
                    tied,
                    firm,
                };
            }
            HeldStep::Whole { firm } => {
                return HeldPart {
                    end: None,
                    tied,
                    firm,
                };
            }
            HeldStep::End { end, firm } => {
                return HeldPart {
                    end: Some((offset + end).min(clip_end)),
                    tied,
                    firm,
                };
            }
            HeldStep::Into {
                next,
                delta,
                across_tie,
            } => {
                offset += delta;
                current = next;
                tied |= across_tie;
                joined = across_tie;
            }
        }
    }
}

/// Where the part of the block `current` that the page holds lies, found from
/// the block that starts `node_id` (see [`avoided_run_head`]).
enum HeldStep {
    /// The block in full, `firm` as for [`HeldPart`].
    Whole { firm: bool },
    /// Up to `end` below the block's border-box top, `firm` as for
    /// [`HeldPart`].
    End { end: f32, firm: bool },
    /// In the box `next`, which starts `delta` below the block's border-box
    /// top, and which an avoided break ties to the box before it when
    /// `across_tie`.
    Into {
        next: usize,
        delta: f32,
        across_tie: bool,
    },
}

/// One step of [`avoided_run_head`] at the block `current`.
fn held_step(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    current: usize,
    joined: bool,
    join_ties: bool,
) -> HeldStep {
    let node = &document.nodes[current];
    let is_table = is_plain_table_box(document, current);
    if avoids_page_break_inside(cascade.computed[current].break_inside) {
        // A break forced inside the box still breaks it (see
        // `forced_break_inside`): only the content before it is held.
        let page_name = std::iter::once(current)
            .chain(document.ancestors(current))
            .find_map(|id| selected_page_name(cascade, id));
        if let Some(end) = forced_break_inside(
            document,
            cascade,
            body_id,
            current,
            page_name,
            document.context.pagination_on_transposed_axes,
            f32::INFINITY,
        ) {
            return HeldStep::End { end, firm: true };
        }
        let extent = avoided_box_extent(document, cascade, body_id, current);
        return if extent > node.layout.size.height + 0.001 {
            HeldStep::End {
                end: extent,
                firm: true,
            }
        } else {
            HeldStep::Whole { firm: true }
        };
    }
    if box_is_unbreakable(
        document,
        cascade,
        current,
        document.context.pagination_on_transposed_axes,
    ) {
        return HeldStep::Whole { firm: true };
    }
    // A flex container that lays its items out in rows holds the first
    // piece of their content, which no break goes through.
    if node.multicol.is_none()
        && !node.has_before_or_after_content
        && lays_out_in_flex_rows(document, cascade, current)
        && let Some(end) = first_piece_end(document, cascade, body_id, current, 0)
    {
        return HeldStep::End { end, firm: true };
    }
    if node.multicol.is_some()
        || node.has_before_or_after_content
        || !(is_table || is_plain_block_box(document, current))
    {
        return HeldStep::Whole { firm: false };
    }
    // The breaks at the start of the box that ties it to the one before it
    // belong to the run, not to the boxes inside it.
    if current != node_id && !joined {
        let (forced, avoided) = page_edge_constraint(document, cascade, current, true);
        if forced || avoided {
            return HeldStep::Whole { firm: false };
        }
    }
    // A table holds its first row, or only the part of it before the first
    // break that splits it.
    if is_table {
        return held_table_end(document, cascade, current).map_or(
            HeldStep::Whole { firm: false },
            |end| HeldStep::End {
                end: first_row_split_end(document, cascade, body_id, current)
                    .map_or(end, |split| split.min(end)),
                firm: false,
            },
        );
    }
    // A block whose box is followed by closing edges alone moves whole
    // rather than leave them on the next page (see `lone_box_tail`), and
    // takes the box before it along.
    if lone_box_tail(document, cascade, body_id, current, false).is_some_and(|tail| tail > 0.001) {
        return HeldStep::End {
            end: node.layout.size.height,
            firm: false,
        };
    }
    if node.is_ifc_root() {
        return held_lines_end(document, cascade, current).map_or(
            HeldStep::Whole { firm: false },
            |end| HeldStep::End { end, firm: true },
        );
    }
    // A block of boxes holds its first box, and the boxes after it that
    // avoided breaks tie to it.
    let mut first = None;
    let mut last = None;
    let mut boxes = Vec::new();
    'children: for &child in &node.children {
        if is_floating_box_for_pagination(document, cascade, child) {
            // A float at the start is part of the part held; one after the
            // first box lies in a flow of its own and ties nothing.
            if first.is_none() {
                return HeldStep::Whole { firm: false };
            }
            continue;
        }
        boxes.clear();
        push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
        for &next in boxes.iter().rev() {
            let Some(before) = last else {
                first = Some(next);
                last = Some(next);
                if !join_ties {
                    break 'children;
                }
                continue;
            };
            // A table that moves whole holds the box after it as a block
            // does (see `last_row_break`).
            if (is_plain_block_box(document, before)
                || (ends_a_run_as_table(document, cascade, before)
                    && table_starts_run(document, cascade, before)
                    && run_break_start(document, cascade, before).is_none()))
                && !takes_no_block_space(document, next)
                && boxes_are_tied(document, cascade, before, next)
                && (is_plain_block_box(document, next)
                    || ends_a_run_as_table(document, cascade, next))
            {
                last = Some(next);
            } else {
                break 'children;
            }
        }
    }
    let (Some(first), Some(last)) = (first, last) else {
        return HeldStep::Whole { firm: false };
    };
    if takes_no_block_space(document, first) {
        return HeldStep::Whole { firm: false };
    }
    HeldStep::Into {
        next: last,
        delta: normal_flow_y(document, cascade, body_id, last),
        across_tie: last != first,
    }
}

/// How far below the border-box top of the paragraph `current` the part of it
/// ends that the page holds with the box before it (see [`held_step`]), or
/// `None` when the paragraph is held whole.
fn held_lines_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    current: usize,
) -> Option<f32> {
    let lines_node = document.ifc_layout_node(current)?;
    let ifc = lines_node.ifc.as_ref()?;
    let lines = ifc.lines.as_ref()?;
    // Block boxes between the lines are breakable places too, and the lines of
    // a multicol container sit in its columns.
    if !lines.block_line_starts.is_empty() || ifc.multicol_fragments.is_some() {
        return None;
    }
    let total = lines.lines.len();
    let computed = &cascade.computed[current];
    let breaks_between = total > computed.orphans.max(1) as usize;
    // A paragraph that cannot move its lines down inside it breaks where they
    // naturally fall, or moves whole when that break leaves too few of them on
    // a side: the box before it cannot tell which, so it stays with the whole
    // paragraph.
    if breaks_between && !lines_can_shift_inside(document, cascade, current) {
        return None;
    }
    let held = if breaks_between {
        usize::try_from(computed.orphans)
            .unwrap_or(1)
            .clamp(1, total)
    } else {
        total
    };
    let line = lines.lines.get(held.checked_sub(1)?)?;
    let held_end = lines_node.layout.border.top
        + lines_node.layout.padding.top
        + line.block_offset()
        + line.block_size();
    // The padding and border below the last line of a paragraph held whole stay
    // with it, as they would be alone on the next page.
    let closing_edges = lines_node.layout.padding.bottom + lines_node.layout.border.bottom;
    let whole =
        !breaks_between && held_end + closing_edges >= lines_node.layout.size.height - 0.001;
    Some(if whole {
        lines_node.layout.size.height
    } else {
        held_end
    })
}

/// How far below the border-box top of the plain block table `table` the part
/// of it ends that the page cannot break away from the box before it: its
/// first row, or the row group around that row when the group avoids breaks
/// inside it. A row that a break can split holds less (see
/// [`first_row_split_end`]). `None` when the table is held whole, which
/// is also the case when it may repeat a header group or something comes
/// before its first row.
fn held_table_end(document: &Document, cascade: &CascadeResult, table: usize) -> Option<f32> {
    let children = document.children_through_contents(table);
    if !rows_start_the_grid(document, cascade, table) {
        return None;
    }
    let row = first_table_row(document, &children)?;
    if forced_break_before_first_row(document, cascade, table, row) {
        return None;
    }
    // A forced break between the rows of a group splits it: the rows before
    // the break are what the page holds.
    let held = document
        .parent_of(row)
        .filter(|&group| {
            group != table && avoids_page_break_inside(cascade.computed[group].break_inside)
        })
        .map_or(row, |group| {
            let mut held = row;
            let rows = document
                .children_through_contents(group)
                .into_iter()
                .filter(|&sibling| document.nodes[sibling].display() == DisplayValue::TableRow);
            for next in rows.skip_while(|&sibling| sibling != row).skip(1) {
                if forces_page_break(document, cascade, held, false)
                    || forces_page_break(document, cascade, next, true)
                {
                    break;
                }
                held = next;
            }
            held
        });
    Some(
        current_abs_y(document, held) + document.nodes[held].layout.size.height
            - current_abs_y(document, table),
    )
}

/// Whether the table `table` has a footer group, whose copy it may repeat
/// below a row that breaks: such a table keeps its rows whole.
fn has_footer_group(document: &Document, table: usize) -> bool {
    document
        .children_through_contents(table)
        .iter()
        .any(|&child| document.nodes[child].display() == DisplayValue::TableFooterGroup)
}

/// The row of the table whose children are `children` that comes first in
/// the flow, in or out of a row group.
fn first_table_row(document: &Document, children: &[usize]) -> Option<usize> {
    children
        .iter()
        .flat_map(|&child| {
            let in_group = matches!(
                document.nodes[child].display(),
                DisplayValue::TableRowGroup | DisplayValue::TableFooterGroup
            );
            let inside = if in_group {
                document.children_through_contents(child)
            } else {
                Vec::new()
            };
            std::iter::once(child).chain(inside)
        })
        .filter(|&row| document.nodes[row].display() == DisplayValue::TableRow)
        .map(|row| (row, current_abs_y(document, row)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(row, _)| row)
}

/// How far below the border-box top of the plain block table `table` the
/// first break that splits its first row can go (see
/// [`table_cell_fragments::first_split_end`]). `None` when no break splits
/// that row, or when the row group around it avoids breaks inside it.
fn first_row_split_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    table: usize,
) -> Option<f32> {
    let row = first_table_row(document, &document.children_through_contents(table))?;
    if has_footer_group(document, table)
        || document.parent_of(row).is_some_and(|group| {
            group != table && avoids_page_break_inside(cascade.computed[group].break_inside)
        })
    {
        return None;
    }
    let split = table_cell_fragments::first_split_end(document, cascade, body_id, row)?;
    Some(current_abs_y(document, row) - current_abs_y(document, table) + split)
}

/// Whether `node_id` is a table that ends a run of boxes tied by avoided
/// breaks: one whose first row is the part the page holds with the box before
/// it (see [`held_table_end`]). Any other table is left out of the run.
fn ends_a_run_as_table(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    is_plain_table_box(document, node_id) && held_table_end(document, cascade, node_id).is_some()
}

/// Whether an avoided break (`break-after` of the box `before` or
/// `break-before` of the box `after`) ties the two boxes, with no forced break
/// at either: neither one of their own nor one that a change of the used page
/// value between them forces (see [`super::page::page_names_meet`]).
fn boxes_are_tied(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    before: usize,
    after: usize,
) -> bool {
    let (forced_after, avoided_after) = page_edge_constraint(document, cascade, before, false);
    let (forced_before, avoided_before) = page_edge_constraint(document, cascade, after, true);
    !forced_after
        && !forced_before
        && (avoided_after || avoided_before)
        && (!document.style_views.names_pages(cascade)
            || super::page::page_names_meet(
                document,
                cascade,
                before,
                after,
                document.context.pagination_on_transposed_axes,
            ))
}

/// Whether an avoided break ties the block `node_id` to the box before it in
/// the flow of its parent (see [`boxes_are_tied`]). A float or an
/// out-of-flow box between the two is not counted.
fn tied_to_box_before(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let children = document.nodes[node_id]
        .parent
        .map(|parent| pagination_child_order(document, cascade, parent))
        .unwrap_or_default();
    let position = children.iter().position(|&child| child == node_id);
    children[..position.unwrap_or(0)]
        .iter()
        .rev()
        .copied()
        .find(|&child| {
            let node = &document.nodes[child];
            node.kind() == NodeKind::Element
                && !node.is_display_none()
                && is_block_level_flow_box(node.display())
                && !is_floating_box_for_pagination(document, cascade, child)
                && !matches!(
                    node.position(),
                    PositionValue::Absolute | PositionValue::Fixed
                )
        })
        .is_some_and(|before| boxes_are_tied(document, cascade, before, node_id))
}

/// The boxes of a flow that avoid a page break inside them or between them,
/// and the runs of margins that pass through empty blocks, found once from its
/// candidates.
#[derive(Default)]
struct AvoidedBreaks {
    /// Clearance facts captured before pagination changes float positions.
    clearance_free: margin_clearance::ClearanceFree,
    /// The block end of each run of boxes joined by avoided breaks, keyed by
    /// the box that starts it among the boxes of its parent or holds it at the
    /// start of its own (see [`avoided_run_ends`]).
    run_ends: HashMap<usize, AvoidedRunEnd>,
    /// How far below its table's top edge the first row of each table
    /// starts (see [`table_row_leads`]).
    row_leads: HashMap<usize, f32>,
    /// The block end and table lead of each avoided table row group, keyed
    /// by its first row (see [`avoided_row_groups`]).
    row_groups: HashMap<usize, (f32, f32)>,
    /// The block that ends the run of margins around each empty block whose
    /// margins collapse through it, keyed by the empty block (see
    /// [`margin_run_ends`]).
    margin_run_ends: HashMap<usize, MarginRunEnd>,
}

/// The block that ends the run of margins around each empty block of
/// `candidates`, whose margins collapse through it: the next block among its
/// siblings, which the empty block and every other empty block before it lead
/// to. A negative margin on that block pulls its top above the empty block, so
/// a page edge in the run is found from where that block starts.
///
/// Floats and out-of-flow subtrees do not end the run. Text, a block whose
/// clearance is not proven absent, or a block that a forced break or a page
/// name change reaches ends the search without a result, and so does a block
/// that something other than empty blocks
/// separates from the empty one, or that a forced break follows: the margin
/// stays the empty block's own.
fn margin_run_ends(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    candidates: &[PageCandidate],
    clearance_free: &margin_clearance::ClearanceFree,
) -> HashMap<usize, MarginRunEnd> {
    let tree = BoxTree::for_layout(document);
    let mut adjacency = sibling_adjacency::SiblingAdjacency::default();
    // Remove parallel subtrees once, rather than walking the same float run
    // again for every empty block before it.
    let candidates: Vec<_> = candidates
        .iter()
        .filter(|candidate| participates_in_margin_adjacency(document, cascade, candidate))
        .collect();
    let joins_run = |candidate: &PageCandidate| {
        let participates = participates_in_margin_adjacency(document, cascade, candidate);
        collapses_with_previous_block(document, candidate, participates) && {
            let computed = &cascade.computed[candidate.node_id];
            (computed.clear == ClearValue::None || clearance_free.contains(candidate.node_id))
                && matches!(
                    computed.break_before,
                    BreakBetween::Auto | BreakBetween::Avoid | BreakBetween::AvoidPage
                )
        }
    };
    let mut ends = HashMap::new();
    // From the last candidate on, so that the end of the run an empty block
    // leads to is known when the empty block before it asks.
    for (index, candidate) in candidates.iter().enumerate().rev() {
        let node_id = candidate.node_id;
        if !joins_run(candidate)
            || !collapses_through(document, cascade, node_id, candidate.height)
            || !no_forced_break_inside_or_after(document, cascade, node_id)
        {
            continue;
        }
        // The empty block has no content, so what follows it and its empty
        // descendants is the next block.
        let Some(next) = candidates[index + 1..]
            .iter()
            .find(|next| !is_descendant_or_self(document, next.node_id, node_id))
        else {
            continue;
        };
        if !joins_run(next)
            || next.page_name != candidate.page_name
            || tree.parent(next.node_id, BoxParentScope::Document)
                != tree.parent(node_id, BoxParentScope::Document)
            || !adjacency.adjoins(document, cascade, next.node_id, node_id)
        {
            continue;
        }
        let end = ends.get(&next.node_id).copied().unwrap_or(MarginRunEnd {
            node_id: next.node_id,
            raw_y: flow_raw_y(document, cascade, body_id, next),
            height: next.height,
        });
        ends.insert(node_id, end);
    }
    ends
}

/// Whether no break is forced after `node_id` or after a box inside it: a
/// forced break after the last box inside it comes after `node_id` as well.
fn no_forced_break_inside_or_after(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let mut pending = vec![node_id];
    while let Some(id) = pending.pop() {
        // A positioned box keeps the breaks inside it to itself.
        if is_out_of_flow_positioned(document, id) {
            continue;
        }
        if !matches!(
            cascade.computed[id].break_after,
            BreakBetween::Auto | BreakBetween::Avoid | BreakBetween::AvoidPage
        ) {
            return false;
        }
        pending.extend(document.nodes[id].children.iter().copied());
    }
    true
}

/// How far below the border-box top of the block `node_id`, in normal flow,
/// the first break forced inside it falls, when one does: the end of the
/// content before it. Only the block flow inside the block is looked
/// through, as a break inside a flex, grid or table box, an atomic inline or
/// a box no page break goes through stays inside that box, and floats and
/// out-of-flow boxes take their breaks with them. A forced break before the
/// first box at the start of the block or after the last one at its end is a
/// break before or after the block itself (CSS Fragmentation 3 §4.1), so it
/// is left out. A block-level box inside that names a page other than
/// `page_name`, the one the block is on, forces a break before it too (CSS
/// Paged Media 3 §6.3).
///
/// A box that avoids a break inside it still breaks there: Chromium keeps it
/// on its page when the content before that break fits there, and moves it
/// only when that content crosses the page end.
///
/// A break inside a box in the block that clips its overflow, has a height
/// its content cannot grow and ends no further below the border-box top of
/// the block than `page_end`, breaks nothing (see [`clip_holds_breaks`]).
fn forced_break_inside(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    page_name: Option<&str>,
    transposed: bool,
    page_end: f32,
) -> Option<f32> {
    if !looks_through_for_breaks(document, cascade, node_id, transposed) {
        return None;
    }
    let flow_top = |id: usize| {
        current_abs_y(document, id) - accumulated_relative_offset_y(document, cascade, body_id, id)
    };
    let top = flow_top(node_id);
    let held = |id: usize, before: bool| {
        // A break at the first or last box of a box is a break before or
        // after that box.
        let mut edge = id;
        while let Some(parent) = document.parent_of(edge).filter(|&parent| parent != node_id)
            && !renders_edge_pseudo(cascade, parent, before)
            && ordered_edge_child(document, cascade, parent, before)
                .is_some_and(|child| is_descendant_or_self(document, child, edge))
        {
            edge = parent;
        }
        document
            .ancestors(edge)
            .take_while(|&id| id != node_id)
            .any(|id| {
                element_clips_overflow(document, cascade, id)
                    && height_is_capped(document, id)
                    && flow_top(id) + document.nodes[id].layout.size.height - top
                        <= page_end + 0.001
            })
    };
    // Walk the boxes in flow order, so that the first forced break found is
    // the first one in the flow; with negative margins a later break can sit
    // above an earlier one. An entry marked `true` is the end of a box,
    // where its `break-after` applies.
    let mut pending: Vec<(usize, bool)> = document.nodes[node_id]
        .children
        .iter()
        .rev()
        .map(|&id| (id, false))
        .collect();
    while let Some((id, after)) = pending.pop() {
        let node = &document.nodes[id];
        if after {
            if !at_block_edge(document, cascade, id, node_id, false) && !held(id, false) {
                return Some(flow_top(id) + node.layout.size.height - top);
            }
            continue;
        }
        if !node.is_in_document()
            || node.is_unrendered_html_element()
            || node.kind() != NodeKind::Element
            || node.display() == DisplayValue::None
            || matches!(
                node.position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
            || is_floating_box_for_pagination(document, cascade, id)
        {
            continue;
        }
        let breaks = !node.display().is_inline_level() && node.display() != DisplayValue::Contents;
        if breaks {
            let computed = &cascade.computed[id];
            let names_other_page =
                selected_page_name(cascade, id).is_some_and(|name| Some(name) != page_name);
            if (page_break_is_forced(document, cascade, id, computed.break_before)
                || names_other_page)
                && !at_block_edge(document, cascade, id, node_id, true)
                && !held(id, true)
            {
                return Some(flow_top(id) - node.layout.margin.top - top);
            }
            if page_break_after_is_forced(document, cascade, id, computed.break_after) {
                pending.push((id, true));
            }
        }
        if matches!(
            node.display(),
            DisplayValue::Contents | DisplayValue::Inline
        ) || looks_through_for_breaks(document, cascade, id, transposed)
        {
            pending.extend(node.children.iter().rev().map(|&child| (child, false)));
        }
    }
    None
}

/// Whether the breaks inside the box `node_id` are breaks of the block flow
/// around it: it lays its children out in block flow and a page break can go
/// through it.
fn looks_through_for_breaks(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    transposed: bool,
) -> bool {
    is_plain_block_box(document, node_id)
        && !is_monolithic_for_pagination_with_transposed_axes(
            document, cascade, node_id, transposed,
        )
}

/// Whether the box `id` inside the block `block` is the first box at its
/// start (`before`) or the last at its end, through every box between them.
/// The `::before` box of the flow of a box between them comes before its
/// first box, and its `::after` box after its last (see
/// [`renders_edge_pseudo`]).
fn at_block_edge(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
    block: usize,
    before: bool,
) -> bool {
    std::iter::successors(Some(id), |&id| document.parent_of(id))
        .take_while(|&id| id != block)
        .all(|id| {
            document
                .parent_of(id)
                .filter(|&parent| !renders_edge_pseudo(cascade, parent, before))
                .and_then(|parent| page_edge_child(document, cascade, parent, before, 0).ok())
                .flatten()
                .is_some_and(|edge| edge == id || document.ancestors(edge).any(|up| up == id))
        })
}

/// Where a box and its page stand when the box is tested for moving to the
/// next page.
#[derive(Clone, Copy)]
struct BoxFit {
    /// The top of the box's border box, with the relative offsets of the box
    /// and its ancestors.
    effective_y: f32,
    /// The same position in normal-flow coordinates, without those offsets.
    flow_y: f32,
    margin_top: f32,
    trailing_margin: f32,
    /// How far below the top of the box that moves with it (see
    /// [`leading_holder`]) the box starts, and that box's own block-start
    /// margin. Zero and `margin_top` when the box moves alone.
    holder_lead: f32,
    holder_margin_top: f32,
    /// The same for a box that avoids a break inside it and begins the plain
    /// blocks with padding or border above it: how far below the top of the
    /// outermost of them the box starts, and the block-start margin of that
    /// block (see `avoided_holder` in `begin_box`). Zero and `margin_top` when
    /// the box is judged alone.
    avoided_lead: f32,
    avoided_margin_top: f32,
    /// The flow positions at which the current page starts and ends.
    page_start: f32,
    page_end: f32,
    /// The block size of the page after the current one.
    next_page_step: f32,
}

/// The ways, besides a forced break, in which a box moves to the next page.
struct BoxOverflow {
    /// The box avoids breaks inside it and would cross the page edge, or it
    /// ends its parent and the parent's closing edges would break away from it.
    page: bool,
    /// The box is one that no page break goes through (see
    /// [`box_is_unbreakable`]) and moves to the next page for that, which is
    /// also counted in `page`. It does so even as the first box of the flow
    /// when a margin above it keeps it from starting the page.
    unbreakable: bool,
    /// The box moves with its leading piece and takes the plain blocks it
    /// begins along: a column with its leading replaced or size-contained
    /// piece, or a row flex container or grid with the first piece of its
    /// content. Unlike an unbreakable box, it does not move as the first box
    /// of the flow.
    leading_piece_with_holder: bool,
    /// The box avoids a break inside it (`break-inside`) and moves to the next
    /// page for that, which is also counted in `page`. A line of generated
    /// text that is kept whole, or a box that moves for the edges that close
    /// its parent, is not this.
    moved_for_avoided_break: bool,
    /// The box is a flex container that moves whole for what follows its
    /// items (see [`flex_margins_move`]), which is also counted in `page`. It
    /// takes the blocks it begins along, with the padding and border they
    /// keep above it.
    moved_for_flex_margins: bool,
    /// The box starts a run of boxes joined by avoided breaks that would
    /// cross the page edge.
    avoided_run: bool,
    /// The box is a table row that would cross the page edge.
    table_row: bool,
    /// The box is a table caption with no break between its lines that would
    /// cross the page edge.
    table_caption: bool,
    /// The box is a table caption that moves to the next page alone, with the
    /// edges that close its table's parent, though it fits this page.
    tail_caption: bool,
    /// How far below its table's top edge the box starts when it is the
    /// first row of a table, or zero.
    table_row_lead: f32,
    /// The table lead of the avoided row group that starts at the box and
    /// moves whole, if any.
    row_group_lead: Option<f32>,
    /// The box is a flex item that would cross the page edge.
    flex_item: bool,
    /// The box is a grid item that would cross the page edge.
    grid_item: bool,
}

impl BoxOverflow {
    /// Whether the box moves to the next page for any of these reasons. The
    /// table lead is no reason: it only places a first row that moves.
    fn moves_box(&self) -> bool {
        self.page
            || self.avoided_run
            || self.table_row
            || self.table_caption
            || self.row_group_lead.is_some()
            || self.flex_item
            || self.grid_item
    }
}

impl AvoidedBreaks {
    /// Finds the avoided breaks of `candidates`, whose flow starts at
    /// `root_flow_offset` and whose class-A boxes sit in the contents box of
    /// `roots`.
    fn collect(
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        candidates: &[PageCandidate],
        table_headers: &HeaderRepeats,
        roots: candidates::FlowRoots,
        content_width: f32,
        root_flow_offset: f32,
    ) -> Self {
        let row_leads = table_row_leads(document, cascade, candidates, root_flow_offset);
        let row_groups =
            avoided_row_groups(document, cascade, candidates, table_headers, &row_leads);
        let run_ends = avoided_run_ends(
            document,
            cascade,
            candidates,
            table_headers,
            &row_groups,
            roots.contents_box,
            content_width,
        );
        let clearance_free =
            margin_clearance::ClearanceFree::collect(document, cascade, candidates);
        let margin_run_ends = margin_run_ends(
            document,
            cascade,
            roots.element,
            candidates,
            &clearance_free,
        );
        Self {
            clearance_free,
            run_ends,
            row_leads,
            row_groups,
            margin_run_ends,
        }
    }

    /// The block that ends the run of margins around `node_id`, an empty block
    /// whose margins collapse through it, if the run goes on to another block.
    fn margin_run_end(&self, node_id: usize) -> Option<MarginRunEnd> {
        self.margin_run_ends.get(&node_id).copied()
    }

    /// How the box of `candidate`, placed as `fit` says, moves to the next
    /// page. `trailing_flex_children` remembers the trailing item of each
    /// flex container it looks up.
    #[allow(clippy::too_many_arguments)]
    fn measure(
        &self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
        candidate: &PageCandidate,
        fit: &BoxFit,
        table_headers: &HeaderRepeats,
        trailing_flex_children: &mut TrailingFlexChildren,
    ) -> BoxOverflow {
        let node_id = candidate.node_id;
        let raw_y = candidate.raw_y;
        let height = candidate.height;
        let BoxFit {
            effective_y,
            flow_y,
            margin_top,
            trailing_margin,
            holder_lead,
            holder_margin_top,
            avoided_lead,
            avoided_margin_top,
            page_start,
            page_end,
            next_page_step,
        } = *fit;
        let avoided_run = self.run_ends.get(&node_id).is_some_and(|end| {
            let moves = |(run_end, firm): (f32, bool)| {
                avoided_run_moves(
                    run_end - raw_y,
                    firm,
                    margin_top,
                    effective_y,
                    page_start,
                    page_end,
                    next_page_step,
                )
            };
            !(end.breaks_inside_start && flow_y + height > page_end + 0.001)
                && (moves((end.end, end.firm))
                    || end.shorter.is_some_and(moves)
                    || end
                        .closing
                        .as_ref()
                        .is_some_and(|closing| closing.moves(fit)))
        });
        let page_box_end = flow_y + height + trailing_margin;
        let unplaced_line = candidate.context.unplaced_line();
        // A replaced element is unbreakable whole.
        let unbreakable = candidate.context.is_unbreakable()
            && unbreakable_box_moves(
                flow_y - holder_lead,
                holder_margin_top,
                flow_y,
                page_box_end - height + candidate.context.unbreakable_extent(height),
                page_start,
                page_end,
            );
        // A paragraph of one line that no text places, of generated text or
        // atomic inlines alone, keeps the edges that follow that line (see
        // `unplaced_line_extent`), and moves with the blocks it begins when
        // they cross the page end. It moves only past content before it on
        // the page (see `follows_box_on_page`), when the edges above its line
        // cross the page end too, or when the line, with no edges above it,
        // crosses the page end itself: below nothing but margins and edges
        // that end on the page, the page breaks between the edges and the
        // line instead (see
        // `PaginationState::break_before_unplaced_line`), as Chromium does.
        let unbreakable = unbreakable
            || unplaced_line.is_some_and(|line| {
                let top = flow_y - holder_lead;
                top > page_start + 0.001
                    && starts_on_page_and_runs_past_it(
                        top,
                        flow_y + unplaced_line_extent(document, node_id, line, height),
                        page_start,
                        page_end,
                    )
                    && ((line.start <= 0.5 && flow_y + line.end_within(height) > page_end + 0.001)
                        || flow_y + line.start > page_end + 0.001
                        || follows_box_on_page(document, cascade, body_id, node_id, page_start))
            });
        let avoids_break_inside =
            box_avoids_page_break_inside(document, node_id, cascade.computed[node_id].break_inside);
        // Content that runs out past the end of a box whose height is held
        // below it, and that nothing clips, has to fit as well: a break
        // through it is a break inside the box.
        let avoided_end = avoids_break_inside.then(|| {
            flow_y
                + external_float_gap::fit_height(document, candidate)
                    .max(avoided_box_extent(document, cascade, body_id, node_id))
                + trailing_margin
        });
        // The blocks that a box which avoids a break inside it begins, with
        // the padding and border they keep above it, are what starts a page.
        let avoided_moves = |end: f32| {
            avoided_box_moves(
                flow_y - avoided_lead,
                avoided_margin_top,
                end,
                page_start,
                page_end,
            )
        };
        // A break avoided before the box is no better than one avoided inside
        // it: Chromium takes the later of the two and splits the box.
        let avoided_end = avoided_end.filter(|_| !candidate.context.avoids_break_before());
        // Only the content before a break forced inside the box has to fit,
        // which can reach below the box's own end (a later child with a
        // negative margin pulls the end up).
        let moves_for_avoided_end = avoided_end.is_some_and(|end| {
            avoided_moves(
                forced_break_inside(
                    document,
                    cascade,
                    body_id,
                    node_id,
                    candidate.page_name.as_deref(),
                    document.context.pagination_on_transposed_axes,
                    page_end - flow_y,
                )
                .map_or(end, |offset| flow_y + offset),
            )
        });
        let page = unbreakable || moves_for_avoided_end;
        // A box inside another that moves with its run lengthens the boxes
        // around it, as one that moves for a break it avoids does.
        let page = page || (avoided_run && !candidate.context.is_direct_body_element());
        // A flex container with an item that ends on this page but whose
        // block-end margin runs past the page end, or whose single line is
        // closed by edges that do, moves to the next page whole, with the
        // blocks it begins.
        let moved_for_flex_margins = flex_margins_move(document, cascade, body_id, candidate, fit);
        let page = page || moved_for_flex_margins;
        // An ordinary table row that would cross the current page moves to
        // the next one whole when a cell of it breaks before its content, as
        // Chromium does; otherwise it breaks there with its cells (see
        // `table_cell_fragments::row_splits`). A row whose cells would need
        // lines moved that cannot move inside them, or one of a table that
        // repeats a header or footer, still moves whole.
        // A row that follows a repeated header must fit beside the header's
        // copy on the next page; otherwise moving it gains nothing.
        // A row of a table that repeats its footer leaves room for the
        // footer's copy below it, on this page and on the next.
        let repeated_header_height = table_headers.repeated_height_before(node_id);
        let repeated_footer_height = table_headers.repeated_height_after(node_id);
        let table_row_lead = self.row_leads.get(&node_id).copied().unwrap_or(0.0);
        // The last row of a table moves to the next page with the edges that
        // close the table when they do not fit after it.
        let item_tail = item_tail_move(candidate, fit);
        let page = page
            || (item_tail
                && !candidate.context.is_table_row()
                && !candidate.context.is_flex_item());
        let row_group = self.row_groups.get(&node_id).copied();
        // An avoided tail group may split after moving to a shorter page.
        // Its first row and the repeated groups must fit, not the whole group.
        let tail_fits = table_row_lead + height + repeated_header_height + repeated_footer_height
            <= next_page_step + 0.001;
        let cell_text_fits = candidate.context.is_table_row()
            && table_cell_fragments::fitting(
                document, cascade, node_id, flow_y, page_start, page_end,
            );
        let table_row = candidate.context.is_table_row()
            && ((item_tail && tail_fits)
                || (!cell_text_fits
                    && table_row_moves(
                        flow_y,
                        height + repeated_footer_height,
                        table_row_lead + height + repeated_header_height + repeated_footer_height,
                        page_start,
                        page_end,
                        next_page_step,
                    )
                    && !(repeated_header_height + repeated_footer_height <= 0.001
                        && table_cell_fragments::row_splits(
                            document, cascade, body_id, node_id, flow_y, page_end,
                        )))
                || spanned_row_restarts(
                    document,
                    cascade,
                    (body_id, candidate.page_name.as_deref()),
                    node_id,
                    flow_y,
                    height,
                    page_start,
                ));
        let row_group_lead = row_group
            .filter(|&(end, _)| {
                (item_tail && tail_fits)
                    || row_group_moves(flow_y, end - raw_y, page_start, page_end)
            })
            .map(|(_, lead)| lead);
        // A column flex item breaks across the page end as a block does,
        // between its lines or boxes. An earlier item moves whole only for
        // what keeps it whole: the padding and border that close a paragraph
        // of one line, which stay with that line, or a single line that no
        // text places and that crosses the page end itself. The trailing item may extend into
        // the containing flex box's continuation at the page edge.
        let flex_item_is_last = candidate.context.is_flex_item()
            && trailing_flex_children.is_trailing(document, cascade, node_id);
        let item_overflow = || {
            item_moves_past_page_end(
                effective_y,
                height,
                margin_top,
                page_start,
                page_end,
                next_page_step,
            )
        };
        let item_kept_whole = || {
            let layout = &document.nodes[node_id].layout;
            let one_line = document
                .ifc_layout_node(node_id)
                .and_then(|node| node.ifc.as_ref())
                .and_then(|ifc| ifc.lines.as_ref())
                .is_some_and(|lines| lines.lines.len() == 1 && lines.block_line_starts.is_empty());
            if one_line && layout.padding.bottom + layout.border.bottom > 0.001 {
                return item_overflow();
            }
            !document.context.pagination_on_transposed_axes
                && self::unplaced_line(document, cascade, node_id)
                    .filter(|line| !line.atomic)
                    .is_some_and(|line| {
                        item_moves_past_page_end(
                            effective_y,
                            line.end_within(height),
                            margin_top,
                            page_start,
                            page_end,
                            next_page_step,
                        )
                    })
        };
        let unplaced_item = unplaced_item_moves(document, cascade, candidate, fit);
        let flex_item = candidate.context.is_flex_item()
            && (item_tail || (!flex_item_is_last && item_kept_whole()) || unplaced_item);
        let grid_item = candidate.context.is_grid_item() && (item_overflow() || unplaced_item);
        // A caption moves to the next page whole unless a break between its
        // lines leaves enough of them on either side (orphans and widows): a
        // break between boxes beats one that leaves too few. It stays when it
        // already starts its page or is taller than the next one. A caption
        // that stays in plain block flow has its lines broken between by the
        // paragraph line pass (see `is_flow_table_caption`); elsewhere it
        // stays as it was laid out.
        let table_caption = candidate.context.is_table_caption()
            && item_overflow()
            && (avoids_break_inside
                || !caption_lines_split_well(document, cascade, node_id, flow_y, page_end));
        BoxOverflow {
            page,
            unbreakable,
            leading_piece_with_holder: false,
            moved_for_avoided_break: avoids_break_inside && moves_for_avoided_end,
            moved_for_flex_margins,
            avoided_run,
            table_row,
            table_caption,
            tail_caption: candidate.context.is_table_caption() && item_tail,
            table_row_lead,
            row_group_lead,
            flex_item,
            grid_item,
        }
    }
}

// Pagination adjusts selected boxes after taffy has produced one normal
// flow layout.  Keep the arena's parent relation so a descendant can be
// materialized relative to an already-shifted ancestor instead of being
// shifted twice.
fn current_abs_y(document: &Document, node_id: usize) -> f32 {
    if let Some(origin) = document.layout_origin_of(node_id) {
        return origin.y;
    } // cov:ignore: callers supply existing layout keys; missing keys would panic in the fallback's arena indexing.
    current_abs_y_fallback(document, node_id) // cov:ignore: valid keys return above; the defensive walk is tested directly.
}

// Keep the defensive walk testable without bypassing the coordinate-owner fast path.
fn current_abs_y_fallback(document: &Document, node_id: usize) -> f32 {
    let mut id = document.ifc_source_owner(node_id);
    let mut y = if id != node_id {
        document
            .ifc_layout_node(node_id)
            .expect("valid anonymous root")
            .layout
            .location
            .y
    } else {
        0.0
    };
    let mut guard = 0_usize;
    let mut walk = AncestorCursor::new(Some(id), ParentRelation::Layout);
    while guard <= document.nodes.len() {
        y += document.pagination_local_y(id);
        // A box of an inline engine paragraph is located from the root,
        // not from the inline elements around it.
        let Some(parent_id) = document.parent_of(id).and_then(|_| {
            walk.advance(document);
            walk.current()
        }) else {
            break;
        };
        id = parent_id;
        guard += 1;
        if !document.is_source_node(id) {
            break; // cov:ignore: defensive stop at a generated layout parent; source boxes sit under source parents.
        }
    }
    y
}

/// Whether something that `parent` holds renders before `child` in its flow:
/// a float beside a paragraph, a block, or text. It already sits on this page,
/// so moving `parent` would take it along. An absolutely or fixed positioned
/// box is not part of that flow: the page does not break around it, so it
/// does not keep `parent` on its page, and one that `parent` contains goes
/// with it (Chromium). An out-of-flow `child` itself never moves `parent`.
fn renders_before_child(document: &Document, parent: usize, child: usize) -> bool {
    if sibling_renders(document, child)
        && document.nodes[child].layout_position() == Position::Absolute
    {
        return true;
    }
    // Every sibling comes before `child` when the parent does not list it. One
    // pass that stops at `child` or at the first sibling that renders in the
    // flow.
    document.nodes[parent]
        .layout_children()
        .iter()
        .take_while(|&&sibling| sibling != child)
        .any(|&sibling| renders_in_flow(document, sibling))
}

/// Whether the child `node` of a box renders something in its parent's flow:
/// it renders and is not positioned out of the flow, or it is a
/// `display: contents` element whose children do.
fn renders_in_flow(document: &Document, node: usize) -> bool {
    #[cfg(test)]
    tests::PARENT_CHILD_STEPS.with(|count| count.set(count.get() + 1));
    let box_node = &document.nodes[node];
    if !sibling_renders(document, node) {
        return false;
    }
    if box_node.kind() == NodeKind::Element && box_node.display() == DisplayValue::Contents {
        return box_node
            .children
            .iter()
            .any(|&child| renders_in_flow(document, child));
    }
    box_node.layout_position() != Position::Absolute
}

/// Whether a child of a box renders something: an element that is in the
/// document and has a box, or text that is not only white space.
fn sibling_renders(document: &Document, sibling: usize) -> bool {
    let node = &document.nodes[sibling];
    match node.kind() {
        NodeKind::Element => {
            node.is_in_document()
                && !node.is_unrendered_html_element()
                && node.display() != DisplayValue::None
        }
        _ => node
            .text_content()
            .is_some_and(|text| !text.trim().is_empty()),
    }
}

/// The outermost box that `block` starts, together with the ancestors that
/// render nothing before it: moving that box moves `block` without leaving the
/// ancestors' starts behind on the earlier page.
///
/// An ancestor without top border or padding begins where its child does, and
/// is left with an empty start otherwise. One with a top border or padding, or
/// one that keeps the margin of its first child inside it, is a plain block
/// box that begins its page below `page_start`: the page breaks
/// before it rather than between those edges and `block`, which would leave
/// them alone at the foot of the page (CSS Break 3 §4.1), unless clearance
/// puts the child below its margin after the edges. One that starts at
/// `page_start` gains nothing by moving, and stops the walk. A
/// `display: contents` ancestor has no box and is passed over.
fn first_box_ancestor_at_start(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
    page_start: f32,
) -> usize {
    let block_top = current_abs_y(document, block);
    let mut child_top = block_top;
    let mut unusual_coordinates = has_unusual_coordinates(document, block);
    let mut moved = block;
    let mut child = block;
    // The nearest box at or below `child`. A `display: contents` element has
    // no box, so the coordinates of the box inside it are relative to the
    // box around it, and the walk carries them past it unchanged.
    let mut box_child = block;
    for parent in document
        .layout_ancestors(block)
        .take_while(|&id| id != body_id)
    {
        // A float moves apart from the boxes around it.
        if renders_before_child(document, parent, child)
            || is_floating_box_for_pagination(document, cascade, child)
        {
            break;
        }
        child = parent;
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        // Flow candidates follow their layout parents; carry the current top
        // upward and fall back for out-of-flow or inline formatting coordinates.
        let parent_top = if unusual_coordinates {
            current_abs_y(document, parent)
        } else if document.oof_coordinates.contains_key(&parent)
            || document.nodes[parent]
                .flags
                .contains(NodeFlags::IN_IFC_SUBTREE)
        {
            unusual_coordinates = true;
            current_abs_y(document, parent)
        } else {
            child_top - document.pagination_local_y(box_child)
        };
        let child_start = child_top;
        // Where the flow puts the child below the parent's edges, without the
        // relative offset that only paints it elsewhere.
        let child_flow_start =
            child_start - layout_relative_offset_y(document, cascade, body_id, box_child);
        // The margin above the child, with those that collapse up through it.
        let child_margin = collapsed_margin_above(document, cascade, box_child).max(0.0);
        box_child = parent;
        child_top = parent_top;
        let layout = &document.nodes[parent].layout;
        let has_edges = layout.border.top != 0.0 || layout.padding.top != 0.0;
        // A box that establishes an independent formatting context keeps the
        // margin of its first child inside it (CSS 2 §8.3.1): that margin
        // does not adjoin a break before the box, so it goes along as its
        // edges would, whether it is positive or negative.
        let keeps_child_margin = (child_start - parent_top).abs() > 0.01
            && !margins_may_collapse_through(document, cascade, parent);
        let moves_with_block = if has_edges || keeps_child_margin {
            // The page breaks before a relative offset moves the box, so the
            // ancestor starts the page at its place in the normal flow. A
            // child that clearance put further down than its margin leaves a
            // gap after the edges, and the page breaks in that gap (Chromium).
            let content_top = parent_top + layout.border.top + layout.padding.top;
            is_plain_block_box(document, parent)
                && child_flow_start - content_top <= child_margin + 0.01
                && parent_top - accumulated_relative_offset_y(document, cascade, body_id, parent)
                    > page_start + 0.001
        } else {
            (parent_top - child_start).abs() <= 0.01
        };
        if !moves_with_block {
            break;
        }
        moved = parent;
    }
    moved
}

/// The outermost box that `node_id`, a box that avoids a break inside it,
/// begins with nothing of its own above it, and how far below that box's top
/// `node_id` starts in the flow: `(holder, lead)`. The walk passes the plain
/// blocks that begin where their child does, as [`leading_holder`] does
/// without edges, and also those that establish an independent formatting
/// context and keep the margin of their first child inside them (CSS 2
/// §8.3.1), when they begin below `page_start`: that margin does not adjoin a
/// break before them, so it moves with the box. A negative one leaves the
/// box above the holder's top, and the lead negative. Border or padding
/// above the child stops the walk, as does anything else before it, and a
/// box placed out of flow or by an inline formatting context.
fn avoided_box_holder(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    page_start: f32,
) -> (usize, f32) {
    let flow_top = |id: usize| {
        current_abs_y(document, id) - accumulated_relative_offset_y(document, cascade, body_id, id)
    };
    let mut holder = node_id;
    let mut child = node_id;
    // The nearest box at or below `child`: a `display: contents` element has
    // no box of its own.
    let mut box_child = node_id;
    for ancestor in document
        .layout_ancestors(node_id)
        .take_while(|&id| id != body_id)
    {
        let starts_row = starts_single_row(document, cascade, ancestor, box_child);
        if renders_before_child(document, ancestor, child) && !starts_row {
            break;
        }
        child = ancestor;
        if document.nodes[ancestor].display() == DisplayValue::Contents {
            continue;
        }
        let layout = &document.nodes[ancestor].layout;
        if !(is_plain_block_box(document, ancestor) || starts_row)
            || document.nodes[ancestor].is_multicol_container()
            || is_floating_box_for_pagination(document, cascade, ancestor)
            || has_unusual_coordinates(document, box_child)
            || has_unusual_coordinates(document, ancestor)
            || layout.border.top != 0.0
            || layout.padding.top != 0.0
        {
            break;
        }
        // A box begins where its child is drawn, as for `leading_holder`:
        // a relative offset on the way leaves the box apart from it. A kept
        // margin is measured in the flow, with the margins that collapse up
        // through the child into it.
        let begins_with_child =
            (current_abs_y(document, box_child) - current_abs_y(document, ancestor)).abs() <= 0.01;
        let keeps_child_margin = !begins_with_child
            && (flow_top(box_child)
                - flow_top(ancestor)
                - collapsed_margin_above(document, cascade, box_child))
            .abs()
                <= 0.01
            && !margins_may_collapse_through(document, cascade, ancestor)
            && flow_top(ancestor) > page_start + 0.001;
        if !(begins_with_child || keeps_child_margin) {
            break;
        }
        holder = ancestor;
        box_child = ancestor;
    }
    (
        holder,
        lead_below_ancestor(document, cascade, body_id, node_id, holder),
    )
}

/// Whether the lead to a contained box consists only of cloned leading
/// edges along a static, margin-free chain. Zero-edge holders are transparent.
/// Keeping that owner is useful even for a tall box: a page that already
/// starts with the cloned edges gains nothing by moving the child alone.
/// Slice edges, margins, other gaps, relative placement and replaced boxes
/// keep their existing holder-fit policy.
fn contained_cloned_leading_holder(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    holder: usize,
    lead: f32,
) -> bool {
    if is_replaced_for_pagination(document, node_id)
        || !size_contained_for_pagination(BoxTree::for_layout(document), node_id)
    {
        return false;
    }
    let mut child = node_id;
    let mut repeated = 0.0;
    let mut reached_holder = false;
    for parent in document.layout_ancestors(node_id) {
        if reached_holder {
            break;
        }
        let node = &document.nodes[child];
        let owner = &document.nodes[parent];
        if !document.is_source_node(parent)
            || !matches!(node.position(), PositionValue::Static)
            || !matches!(owner.position(), PositionValue::Static)
            || node.layout.margin.top != 0.0
        {
            return false;
        }
        let edges = owner.layout.border.top + owner.layout.padding.top;
        if (edges > 0.0
            && cascade.computed[parent].box_decoration_break
                != raikiri_style::property::BoxDecorationBreak::Clone)
            || (document.pagination_local_y(child) - edges).abs() > 0.001
        {
            return false;
        }
        repeated += edges;
        reached_holder = parent == holder;
        child = parent;
    }
    reached_holder && repeated > 0.0 && (lead - repeated).abs() <= 0.001
}

/// The outermost box that moves to the next page together with `node_id`, a
/// box that no page break goes through or that avoids one inside it, when it
/// is the first thing in the plain blocks around it: the ancestors that render
/// nothing before it. And how far below the top of that box `node_id` starts,
/// which is the padding, the border and the margin that its ancestors keep
/// above it.
///
/// A page does not break between such an ancestor's start and its first child
/// (CSS Break 3 §4.1), so the ancestor starts the next page with the child
/// rather than leaving its start, and a fixed height, a background, a padding
/// or a border, behind on this one. An ancestor that is not a plain block of
/// the normal flow, that lays its children out in several columns, or that is
/// placed by an inline formatting context or out of flow, is left to the box
/// alone; the walk stops below it. Two exceptions move with the box: a flex
/// or grid container that lays out a single row the box begins (see
/// [`starts_single_row`]), whose whole row Chromium moves when the page
/// cannot break before the box, and a multicol container of a single column,
/// which follows the page as block flow does.
///
/// With `edges` false the walk stops below the first ancestor that keeps
/// something above the box, so the holder starts where the box does and the
/// distance is zero. A caller that cannot land the box below such edges asks
/// for this.
///
/// Where the box begins its ancestors does not depend on the page it falls on,
/// so the holder is the same when the candidates are collected and when the
/// box moves. Whether a holder that already starts its page is worth moving is
/// for the caller to judge from its top.
///
/// Every other ancestor the walk passes is a plain block of the normal flow
/// that is not a multicol container, so the only boxes beside the box that
/// move with it are the other items of such a row.
fn leading_holder(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    edges: bool,
) -> (usize, f32) {
    leading_holder_with(
        document,
        cascade,
        body_id,
        node_id,
        (edges, true),
        renders_before_child,
    )
}

/// The walk of [`leading_holder`]. With `through_rows` false it stops below
/// every container that lays boxes beside one another, as for a break forced
/// inside such a row, which waits for the row rather than moving it.
fn leading_holder_with(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    (edges, through_rows): (bool, bool),
    mut renders_before: impl FnMut(&Document, usize, usize) -> bool,
) -> (usize, f32) {
    if has_unusual_coordinates(document, node_id) {
        return (node_id, 0.0);
    }
    let top = current_abs_y(document, node_id);
    let mut holder = node_id;
    let mut child = node_id;
    // The nearest box at or below `child`: a `display: contents` element has
    // no box of its own.
    let mut box_child = node_id;
    for ancestor in document
        .layout_ancestors(node_id)
        .take_while(|&id| id != body_id)
    {
        // The items of a row sit beside the one that holds the box, not
        // before it.
        let starts_row = through_rows && starts_single_row(document, cascade, ancestor, box_child);
        if renders_before(document, ancestor, child) && !starts_row {
            break;
        }
        child = ancestor;
        if document.nodes[ancestor].display() == DisplayValue::Contents {
            continue;
        }
        box_child = ancestor;
        if !(is_plain_block_box(document, ancestor) || starts_row)
            || (document.nodes[ancestor].is_multicol_container()
                && !(through_rows && is_single_column(document, ancestor)))
            || is_floating_box_for_pagination(document, cascade, ancestor)
            || has_unusual_coordinates(document, ancestor)
            || if edges {
                current_abs_y(document, ancestor) > top + 0.01
            } else {
                (current_abs_y(document, ancestor) - top).abs() > 0.01
            }
        {
            break;
        }
        holder = ancestor;
    }
    let lead = if edges {
        flow_distance(document, cascade, body_id, node_id, holder).max(0.0)
    } else {
        0.0
    };
    (holder, lead)
}

/// Whether the multicol container `id` lays its content out in one column
/// of its whole width, which follows the page as block flow does.
fn is_single_column(document: &Document, id: usize) -> bool {
    document.nodes[id]
        .multicol
        .is_some_and(|style| style.count == Some(1) && style.width.is_none())
}

/// Whether `item` is an item of the flex or grid container `container` at
/// the top of a single row the container lays out beside it: a row flex
/// container that does not wrap, or a grid of several columns, with `item`
/// at the top of its content box. A box that begins such an item begins the
/// row, and Chromium moves the whole row, and so the container, when the
/// page cannot break before the box.
fn starts_single_row(
    document: &Document,
    cascade: &CascadeResult,
    container: usize,
    item: usize,
) -> bool {
    let node = &document.nodes[container];
    let computed = &cascade.computed[container];
    let row = match node.display() {
        DisplayValue::Flex => {
            matches!(
                computed.flex_direction,
                FlexDirectionValue::Row | FlexDirectionValue::RowReverse
            ) && computed.flex_wrap == FlexWrapValue::NoWrap
        }
        DisplayValue::Grid => node.grid_column_count > 1,
        _ => false,
    };
    let layout = &node.layout;
    row && matches!(
        node.position(),
        PositionValue::Static | PositionValue::Relative
    ) && !node.used_writing_mode.is_vertical()
        && document.layout_parent_of(item) == Some(container)
        && (document.nodes[item].layout.location.y - layout.border.top - layout.padding.top).abs()
            <= 0.01
}

/// The block size of what a box that begins the blocks from its parent up to
/// `holder` lies below inside them: the border and padding of those blocks and
/// the margins between their edges, as the layout resolved them. A margin
/// collapses with the ones next to it until a border or padding separates it,
/// or a block that establishes an independent formatting context (which
/// keeps the margin of its first child inside it), so the largest of each such
/// run counts, as in the normal flow. The margin above `holder` itself, and
/// any that collapses through its top, lie outside it.
///
/// This is what the box's distance from `holder` (see [`leading_holder`]) is
/// when nothing else comes before it. The walk of `leading_holder` cannot see
/// what an anonymous block holds, so a larger distance means something is
/// there that a break may fall after.
fn leading_space_between(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    holder: usize,
) -> f32 {
    let mut total = 0.0;
    let mut margin = document.nodes[node_id].layout.margin.top.max(0.0);
    for ancestor in document.layout_ancestors(node_id) {
        if document.nodes[ancestor].display() == DisplayValue::Contents {
            continue;
        }
        let layout = &document.nodes[ancestor].layout;
        let edges = layout.border.top + layout.padding.top;
        if edges > 0.0 || !margins_may_collapse_through(document, cascade, ancestor) {
            total += margin + edges;
            margin = 0.0;
        }
        if ancestor == holder {
            break;
        }
        margin = margin.max(layout.margin.top);
    }
    total
}

/// The outermost box that `node_id` begins, and how far below the top of that
/// box `node_id` starts: `Some((holder, lead))`. Only the border and padding
/// of the boxes between them, and the margins between those, may lie above
/// `node_id` (see [`leading_holder`] and [`leading_space_between`]; a hundredth of
/// a pixel absorbs the rounding of the two). The result is `None` when
/// `node_id` begins no such box, or when anything else lies before it: the
/// walk of `leading_holder` cannot see what an anonymous block holds, and that
/// is a place to break at.
fn lead_below_edges(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<(usize, f32)> {
    holder_below_edges(document, cascade, body_id, node_id).filter(|&(_, lead)| lead > 0.0)
}

/// As [`lead_below_edges`], but the boxes between `node_id` and its holder
/// may also have no border or padding at its top, so that `node_id` starts
/// where its holder does.
fn holder_below_edges(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<(usize, f32)> {
    let (holder, lead) = leading_holder(document, cascade, body_id, node_id, true);
    (holder != node_id && lead <= leading_space_between(document, cascade, node_id, holder) + 0.01)
        .then_some((holder, lead))
}

/// The margin above the border box of `holder`: the one that the margins
/// adjoining its top collapse into (CSS 2 §8.3.1). A margin that collapses up
/// through `holder` from a box inside it, as at the start of the document
/// where the margin of the first block collapses through `body`, lies above
/// the border box as the holder's own margin would.
fn collapsed_margin_above(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    holder: usize,
) -> f32 {
    let (positive, negative) = adjoining_margin_extremes(document, cascade, holder, true, None);
    positive + negative
}

/// Whether the position of `node_id` does not follow its layout parent's in
/// the block flow: it is placed out of flow, or by an inline formatting
/// context.
fn has_unusual_coordinates(document: &Document, node_id: usize) -> bool {
    document.oof_coordinates.contains_key(&node_id)
        || document.ifc_source_owner(node_id) != node_id
        || document.nodes[node_id]
            .flags
            .contains(NodeFlags::IN_IFC_SUBTREE)
}

/// Whether `node_id` is out of flow with a block-start or block-end inset.
/// Insets come straight from the computed values: nothing overrides them,
/// and the cascade of a transposed pagination carries them on its axes.
fn has_explicit_block_axis_inset(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    if document.nodes[node_id].kind() != NodeKind::Element {
        return false;
    }
    let computed = &cascade.computed[node_id];
    matches!(
        document.nodes[node_id].position(),
        PositionValue::Absolute | PositionValue::Fixed
    ) && (!inset_is_auto(&computed.top) || !inset_is_auto(&computed.bottom))
}

/// Whether `node_id` is out of flow and placed by a block-start inset of a
/// fixed length alone: its place follows the top of its containing block, not
/// the lines of the paragraph it sits in nor the height of the box that holds
/// it.
fn is_top_anchored_out_of_flow(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    has_explicit_block_axis_inset(document, cascade, node_id) && {
        let computed = &cascade.computed[node_id];
        // A percentage resolves against the height of the containing block,
        // which the moved lines can grow, and so can a `calc()` that has one.
        let top_is_a_length = matches!(&computed.top, ComputedLengthPercentageOrAuto::Px(_));
        top_is_a_length && inset_is_auto(&computed.bottom)
    }
}

/// How an out-of-flow box of a paragraph is placed when the lines of the
/// paragraph move down inside it, and what moves the box along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutOfFlowFollow {
    /// A block-start inset of a fixed length places it: the lines do not
    /// matter.
    TopAnchored,
    /// Without insets along the block axis it keeps the place of its anchor
    /// on a line, and moves with that line.
    OnLine,
    /// A `bottom`, or a `top` that is a percentage, places it against the
    /// padding box of the paragraph, whose height the moved lines grow.
    WithHeight,
    /// An ancestor's height places it after normal-flow growth settles.
    DeferredAncestorHeight,
}

/// Whether a size follows the height of the containing block: it is a
/// percentage, a `calc()` that holds one, or `stretch`, which fills it.
fn follows_height(length: impl Into<Dimension>) -> bool {
    match length.into().expand() {
        ExpandedDimension::Percent(_) | ExpandedDimension::Stretch => true,
        ExpandedDimension::Calc(calc) => calc.has_percentage(),
        _ => false,
    }
}

/// How the out-of-flow box `node`, one of the boxes of the paragraph `root`,
/// follows lines that move down, or `None` when it would need more than that
/// to stay where its insets put it: its size depends on the containing block's
/// height, both insets place it, its ancestor cannot be resolved after normal
/// flow, or it does not follow the static position of a line.
fn out_of_flow_follow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    root: usize,
    node: usize,
) -> Option<OutOfFlowFollow> {
    if is_top_anchored_out_of_flow(document, cascade, node) {
        return Some(OutOfFlowFollow::TopAnchored);
    }
    let ifc = document.ifc_layout_node(root)?.ifc.as_ref()?;
    if ifc.writing_mode != shodo::geometry::WritingMode::HorizontalTb
        || document.context.pagination_on_transposed_axes
        || document.nodes[node].position() != &PositionValue::Absolute
    {
        return None;
    }
    let style = document.physical_style(node);
    // A size that follows the height of the containing block moves its end
    // when the paragraph grows.
    if follows_height(style.size().height)
        || follows_height(style.min_size().height)
        || follows_height(style.max_size().height)
    {
        return None;
    }
    let inset = style.inset();
    match (inset.top.is_auto(), inset.bottom.is_auto()) {
        (true, true) => document
            .oof_coordinates
            .get(&node)
            .is_some_and(|coordinate| coordinate.follow_static_y)
            .then_some(OutOfFlowFollow::OnLine),
        (false, false) => None,
        _ => {
            if document
                .oof_coordinates
                .get(&node)
                .is_some_and(|coordinate| coordinate.owner == root)
            {
                Some(OutOfFlowFollow::WithHeight)
            } else {
                positioned_flow::follows_ancestor_height(document, root, node)
                    .then_some(OutOfFlowFollow::DeferredAncestorHeight)
            }
        }
    }
}

fn in_inset_out_of_flow_subtree(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    document.inclusive_ancestors(node_id).any(|id| {
        document.nodes[id].kind() == NodeKind::Element
            && (matches!(document.nodes[id].position(), PositionValue::Fixed)
                || has_explicit_block_axis_inset(document, cascade, id))
    })
}

fn has_explicit_positioned_block_ancestor(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    document
        .inclusive_ancestors(document.ifc_source_owner(node_id))
        .any(|id| has_explicit_block_axis_inset(document, cascade, id))
}

fn materialize_y(document: &mut Document, node_id: usize, desired_y: f32) -> IfcBoxShifts {
    if !desired_y.is_finite() {
        return IfcBoxShifts::default(); // cov:ignore: callers pass finite page positions.
    }
    let actual_y = current_abs_y(document, node_id);
    let delta = desired_y - actual_y;
    if delta.is_finite() {
        document.translate_pagination_y(node_id, delta);
        if document.is_source_node(node_id) {
            return follow_moved_ifc_block(document, node_id, delta);
        }
    } // cov:ignore: only source boxes are materialized during pagination.
    IfcBoxShifts::default()
}

/// The lines of the text `text` of the paragraph `root` start a later
/// page: they, the lines after them and the paragraph's boxes below their
/// top move down by `delta`.
fn follow_moved_ifc_text(
    document: &mut Document,
    root: usize,
    text: usize,
    delta: f32,
    text_lines: &TextLineIndex,
) {
    if delta == 0.0 || !delta.is_finite() {
        return;
    }
    let Some((owned, first)) = text_lines
        .get(text)
        .and_then(|owned| Some((owned, owned.lines.first().copied()?)))
    else {
        return; // cov:ignore: callers pass texts indexed with at least one line.
    };
    let old_top = text_first_line_offset(document, root, owned);
    let Some(ifc) = document.table_layout_node_mut(root).ifc.as_mut() else {
        return; // cov:ignore: the paragraph root of an indexed text carries its inline layout.
    };
    let Some(lines) = ifc.lines.as_mut() else {
        return; // cov:ignore: the paragraph root of an indexed text carries its line records.
    };
    lines.shifts.push(first.line, delta);
    let boxes: Vec<(usize, IfcBoxKind)> = ifc.boxes.iter().map(|b| (b.node, b.kind)).collect();
    let last_line_end = lines.lines.last().map(|line| {
        line.block_offset() + line.block_size() + lines.shifts.of(lines.lines.len() - 1)
    });
    for &(b, kind) in &boxes {
        // A float before the text in the source is a page candidate of its
        // own and was placed in the paginated flow before the text was.
        if kind == IfcBoxKind::Float && precedes_in_tree(document, b, text) {
            continue;
        }
        if document.pagination_local_y(b) >= old_top - 0.01 {
            document.translate_pagination_y(b, delta);
        }
    }
    // The root holds the lines that moved, and the floats of the paragraph
    // when it establishes a formatting context of its own.
    let node = &document.nodes[root];
    let contains_floats = matches!(
        node.display(),
        DisplayValue::FlowRoot
            | DisplayValue::InlineBlock
            | DisplayValue::TableCell
            | DisplayValue::TableCaption
    ) || matches!(
        node.position(),
        PositionValue::Absolute | PositionValue::Fixed
    );
    let floats_end = boxes
        .iter()
        .filter(|&&(_, kind)| contains_floats && kind == IfcBoxKind::Float)
        .map(|&(b, _)| {
            let layout = document.nodes[b].layout;
            document.pagination_local_y(b) + layout.size.height + layout.margin.bottom
        })
        .fold(f32::NEG_INFINITY, f32::max);
    let layout = &mut document.table_layout_node_mut(root).layout;
    if let Some(end) = last_line_end {
        let height = (layout.border.top
            + layout.padding.top
            + end
            + layout.padding.bottom
            + layout.border.bottom)
            .max(floats_end + layout.padding.bottom + layout.border.bottom);
        layout.size.height = layout.size.height.max(height);
        document.mark_pagination_extent_dirty(root);
    }
}

/// Whether `a` comes before `b` in tree order. An ancestor comes before its
/// descendants.
pub(crate) fn precedes_in_tree(document: &Document, a: usize, b: usize) -> bool {
    let a_chain: Vec<usize> = document.inclusive_ancestors(a).collect();
    let mut b_child = None;
    for id in document.inclusive_ancestors(b) {
        if let Some(position) = a_chain.iter().position(|&ancestor| ancestor == id) {
            let Some(b_child) = b_child else {
                // `b` is `a` or one of its ancestors.
                return false;
            };
            let Some(a_child) = position.checked_sub(1).map(|index| a_chain[index]) else {
                // `a` is an ancestor of `b`.
                return true;
            };
            let children = &document.nodes[id].children;
            let index = |child| children.iter().position(|&c| c == child);
            return index(a_child) < index(b_child);
        }
        b_child = Some(id);
    }
    false
}

/// A block child of a paragraph laid out by the inline engine was moved
/// by `delta` (to a later page): the lines after it and the paragraph's
/// boxes after it in source order move with it, as the content that
/// follows a block follows it in the block's formatting context. Lines
/// and boxes before it stay where they are. A later box that a negative
/// margin pulls up beside the block still follows it. Returns the IFC boxes
/// considered and which of them it shifted.
fn follow_moved_ifc_block(document: &mut Document, node_id: usize, delta: f32) -> IfcBoxShifts {
    if delta == 0.0 || !delta.is_finite() || document.nodes[node_id].in_ifc_subtree() {
        return IfcBoxShifts::default();
    }
    // An anonymous table cell lays the boxes it wraps out as a paragraph of
    // its own, though it is no source parent of theirs.
    let Some(root) = document
        .anonymous_cell_of(node_id)
        .or_else(|| document.layout_parent_of(node_id))
    else {
        return IfcBoxShifts::default(); // cov:ignore: a block child of an inline-engine paragraph has a layout parent.
    };
    let layout = document.nodes[node_id].layout;
    let old_top = document.pagination_local_y(node_id) - delta;
    let old_bottom = old_top + layout.size.height;
    follow_ifc_block(document, root, node_id, old_top, old_bottom, delta)
}

/// The content after `block`, a block child of the paragraph `root` that
/// spanned `old_top..old_bottom`, moves down by a finite, nonzero
/// `delta`: the lines after
/// it and the paragraph's boxes after it in source order. A box inside
/// `block` that moves to a later page takes the rest of the paragraph
/// along this way even though `block` itself stays.
#[derive(Default)]
struct IfcBoxShifts {
    /// Every IFC box considered while following the moved block.
    boxes: Vec<usize>,
    /// IFC boxes whose positions were shifted by this pass.
    shifted: Vec<usize>,
}

fn follow_ifc_block(
    document: &mut Document,
    root: usize,
    block: usize,
    old_top: f32,
    old_bottom: f32,
    delta: f32,
) -> IfcBoxShifts {
    follow_ifc_block_with_oof(document, root, block, old_top, old_bottom, delta, None)
}

fn follow_ifc_block_with_oof(
    document: &mut Document,
    root: usize,
    block: usize,
    old_top: f32,
    old_bottom: f32,
    delta: f32,
    allowed_oof: Option<&HashSet<usize>>,
) -> IfcBoxShifts {
    let Some(ifc) = document.table_layout_node_mut(root).ifc.as_mut() else {
        return IfcBoxShifts::default();
    };
    let Some(lines) = ifc.lines.as_mut() else {
        return IfcBoxShifts::default(); // cov:ignore: a paragraph whose block child moves carries its line records.
    };
    let Some(&(_, first_after)) = lines
        .block_line_starts
        .iter()
        .find(|(child, _)| *child == block)
    else {
        return IfcBoxShifts::default();
    };
    lines.shifts.push(first_after, delta);
    // The root's boxes are kept in source order. A later box in flow
    // follows the block from as high as the block's top, which covers a
    // negative top margin but not a box in a later column of the same
    // paragraph. An absolutely positioned box is placed against the
    // root, so one follows the block only from below its old bottom.
    let boxes: Vec<(usize, IfcBoxKind, bool)> = ifc
        .boxes
        .iter()
        .scan(false, |after, b| {
            let later = *after;
            *after |= b.node == block;
            Some((b.node, b.kind, later))
        })
        .collect();
    let mut shifts = IfcBoxShifts::default();
    for (b, kind, later) in boxes {
        shifts.boxes.push(b);
        let y = document.pagination_local_y(b);
        let follows = if kind == IfcBoxKind::OutOfFlow {
            y >= old_bottom - 0.01 && allowed_oof.is_none_or(|allowed| allowed.contains(&b))
        } else {
            later && y >= old_top - 0.01
        };
        if follows {
            document.translate_pagination_y(b, delta);
            shifts.shifted.push(b);
        }
    }
    shifts
}

fn is_descendant_or_self(document: &Document, node_id: usize, ancestor_id: usize) -> bool {
    if document.anonymous_table_cell(ancestor_id).is_some() {
        // An anonymous cell is no source parent of the boxes it wraps.
        return node_id == ancestor_id
            || wrapping_anonymous_cell(document, node_id) == Some(ancestor_id);
    }
    document.is_ancestor_or_self(ancestor_id, node_id)
}

fn has_footnote_ancestor(cascade: &CascadeResult, node_id: usize, document: &Document) -> bool {
    document.ancestors(node_id).any(|id| {
        cascade
            .computed
            .get(id)
            .is_some_and(|computed| matches!(computed.float, FloatValue::Footnote))
    })
}

/// Like [`layout_single_page`], but first resolve intrinsic sizes of
/// replaced elements such as `<img>` through `resolver`.
///
/// # Errors
/// As for [`layout_single_page`], plus this function’s own
/// `LayoutError::Resolver`: abort the pre-pass as soon as `resolver.resolve()`
/// returns `Err`, wrap that error in `LayoutError::Resolver`, and return
/// without running taffy layout. The `ReplacedResolver` contract makes `Err`
/// terminal; consumers request placeholder degradation by returning
/// `Ok(ResolvedIntrinsic::new(intrinsic, ResolveDisposition::Fallback { .. }))` instead.
/// See the documentation for `crate::image_resolve::resolve_images`.
///
/// # Execution order (essential)
/// Call [`Document::mark_in_document_flags`] before `resolve_images`.
/// `resolve_images` skips `<img>` in inert subtrees (such as descendants of
/// `<template>`) using membership flags. If flags are stale, it may fetch a URL
/// that should not be fetched. [`layout_single_page`] synchronizes flags too,
/// but only **after** this pre-pass, which would be too late.
/// `mark_in_document_flags` is O(1) when `flags_dirty == false`,
/// so calling it twice has no meaningful cost.
#[doc(hidden)]
pub fn layout_single_page_with_resolver(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
) -> Result<(), LayoutError> {
    layout_single_page_with_resolver_and_base_url(document, cascade, page_box, resolver, None)
}

/// [`layout_single_page_with_resolver`] with document-relative image URLs.
#[doc(hidden)]
pub fn layout_single_page_with_resolver_and_base_url(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
) -> Result<(), LayoutError> {
    super::validate_layout_depth(document)?;
    document.page_projection.clear();
    // See “Execution order” above — this must precede `resolve_images`, whose
    // membership gate reads the flags this refreshes.
    document.mark_in_document_flags();
    resolve_document_images(document, cascade, resolver, base_url)?;
    layout_single_page(document, cascade, page_box)
}

/// Whether `id` holds in-flow content that keeps its margins from collapsing
/// through it (CSS 2 §8.3.1). A line box counts, including one holding only
/// generated content such as an inside marker or `::before`, and so does a
/// block-level `::before` or `::after` box with padding, border or height,
/// or one the paragraph placed as a box of its own, as with clearance.
/// Out-of-flow children do not count, text and inline children count only
/// through their lines, and an empty block child whose own margins collapse
/// through it does not count.
fn has_in_flow_content(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    id: usize,
) -> bool {
    // On transposed axes the paragraph's width is its inline size, while the
    // generated boxes keep their physical styles.
    let vertical = document.context.pagination_on_transposed_axes;
    let has_paragraph_content = document.ifc_layout_node(id).is_some_and(|node| {
        let width = node.layout.size.width;
        node.ifc.as_ref().is_some_and(|ifc| {
            ifc.generated_blocks
                .iter()
                .any(|block| block.separates_margins(width, vertical))
                || ifc
                    .lines
                    .as_ref()
                    .is_some_and(|lines| !lines.lines.is_empty() || !lines.placed_blocks.is_empty())
        })
    });
    if has_paragraph_content {
        return true;
    }
    let mut boxes = Vec::new();
    for &child in &document.nodes[id].children {
        push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
    }
    boxes.into_iter().any(|child| {
        let node = &document.nodes[child];
        let display = document.nodes[child].display();
        if node.kind() == NodeKind::Text || display.is_inline_level() {
            // Text and inline-level boxes are content only through the lines
            // they make, which the line check above has already counted.
            false
        } else {
            !margins_may_collapse_through(document, cascade, child)
                || node.layout.size.height > 0.0
                || has_in_flow_content(document, cascade, child)
        }
    })
}

/// Whether the margins of `owner`, an empty box after a forced break, reach
/// the block-start margin of `node_id` through the block-end margin of
/// `previous`, its preceding sibling box: `previous` is `owner` itself or an
/// ancestor whose boxes down to `owner` are all empty, so that the margins
/// collapse through each of them (CSS 2 §8.3.1), and no generated box lies
/// between `previous` and `node_id` to keep the margins apart.
fn forced_margin_reaches(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    owner: usize,
    previous: usize,
    node_id: usize,
) -> bool {
    if generated_box_separates(document, node_id, previous) {
        return false;
    }
    let mut id = owner;
    while id != previous {
        match BoxTree::for_layout(document).parent(id, BoxParentScope::Document) {
            Some(parent)
                if collapses_through(
                    document,
                    cascade,
                    parent,
                    document.nodes[parent].layout.size.height,
                ) =>
            {
                id = parent;
            }
            _ => return false,
        }
    }
    true
}

/// The block-end margins of `ancestor` and of the boxes inside it down to
/// `last`, when `last` ends each of them with no padding, border or used
/// height to keep its block-end margin from theirs (CSS 2 §8.3.1).
fn ends_with_bottom_margin_of(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    ancestor: usize,
    last: usize,
) -> Option<(f32, f32)> {
    let mut extremes = (0.0f32, 0.0f32);
    let mut id = last;
    while id != ancestor {
        let parent = BoxTree::for_layout(document).parent(id, BoxParentScope::Document)?;
        let layout = &document.nodes[parent].layout;
        if layout.border.bottom + layout.padding.bottom > 0.0
            || !margins_may_collapse_through(document, cascade, parent)
            || !document.block_axis(parent).height().is_auto()
            || last_flow_box(document, cascade, parent) != Some(id)
        {
            return None;
        }
        let margin = layout.margin.bottom;
        extremes = join_margin_extremes(extremes, (margin.max(0.0), margin.min(0.0)));
        id = parent;
    }
    Some(extremes)
}

/// Whether a `::before` or `::after` box of a `display: contents` element
/// between `previous` and `node_id`, its following sibling box, keeps their
/// margins apart. Such a box has no node of its own: it is a block or inline
/// content in the paragraph of the shared box parent (see
/// [`has_in_flow_content`]).
fn generated_box_separates(
    document: &super::context::LayoutSession<'_>,
    node_id: usize,
    previous: usize,
) -> bool {
    use raikiri_style::PseudoElem;

    let separates = |element: usize, pseudo: PseudoElem| {
        let id = crate::generated_content::generated_node_id(element, pseudo) as u64;
        let vertical = document.context.pagination_on_transposed_axes;
        BoxTree::for_layout(document)
            .parent(element, BoxParentScope::Document)
            .and_then(|parent| document.ifc_layout_node(parent))
            .is_some_and(|node| {
                let width = node.layout.size.width;
                // A block box the line loop placed holds lines or clearance,
                // and inline content puts text on a line; either keeps the
                // margins apart, like the content that `has_in_flow_content`
                // counts.
                node.ifc.as_ref().is_some_and(|ifc| {
                    ifc.generated_blocks
                        .iter()
                        .any(|block| block.start == id && block.separates_margins(width, vertical))
                        || ifc.lines.as_ref().is_some_and(|lines| {
                            lines.placed_blocks.iter().any(|placed| placed.node == id)
                                || lines.lines.iter().any(|line| {
                                    super::ifc::text_lines::glyph_owners(line)
                                        .contains(&(id as usize))
                                })
                        })
                })
            })
    };
    let is_contents = |id: usize| {
        document.nodes[id].kind() == NodeKind::Element
            && document.nodes[id].display() == DisplayValue::Contents
    };
    // Walks the boxes before `id` in reverse order, the generated boxes of
    // `display: contents` elements included, until `previous`.
    fn scan(
        document: &Document,
        id: usize,
        previous: usize,
        is_contents: &dyn Fn(usize) -> bool,
        separates: &dyn Fn(usize, PseudoElem) -> bool,
    ) -> Option<bool> {
        if id == previous {
            return Some(false);
        }
        if !is_contents(id) {
            return None;
        }
        if separates(id, PseudoElem::After) {
            return Some(true);
        }
        document.nodes[id]
            .children
            .iter()
            .rev()
            .find_map(|&child| scan(document, child, previous, is_contents, separates))
            .or_else(|| separates(id, PseudoElem::Before).then_some(true))
    }
    let mut current = node_id;
    for parent in document.ancestors(node_id) {
        let siblings = &document.nodes[parent].children;
        let index = siblings
            .iter()
            .position(|&id| id == current)
            .unwrap_or(siblings.len());
        if let Some(found) = siblings[..index]
            .iter()
            .rev()
            .find_map(|&id| scan(document, id, previous, &is_contents, &separates))
        {
            return found;
        }
        // `previous` shares the box parent of `node_id`, so the walk finds it
        // before it leaves the `display: contents` ancestors.
        if !is_contents(parent) || separates(parent, PseudoElem::Before) {
            return is_contents(parent);
        }
        current = parent;
    }
    false // cov:ignore: the document node is never a `display: contents` element, so the walk returns before it runs out of parents
}

/// Where the collapsed margin between `previous` and `node_id` starts in the
/// page flow, given the previous sibling's bottom and the top of `node_id`'s
/// border box.
///
/// Only the collapsed margin is truncated, not clearance or other space in
/// the gap. Both positive and negative adjoining margins contribute to its
/// width, even when an empty sibling places its border above that margin.
fn collapsed_margin_start(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    previous: usize,
    node_id: usize,
    previous_bottom: f32,
    border_box_top: f32,
) -> f32 {
    let (previous_positive, previous_negative) =
        adjoining_margin_extremes(document, cascade, previous, false, None);
    let (current_positive, current_negative) =
        adjoining_margin_extremes(document, cascade, node_id, true, None);
    let collapsed =
        previous_positive.max(current_positive) + previous_negative.min(current_negative);
    previous_bottom.max(border_box_top - collapsed)
}

/// The largest positive margin and the most negative one (zero when there is
/// none) among the block-start (`start`) or block-end margin of `id` and the
/// margins of its first or last in-flow child that adjoin it. When that child
/// is empty and its margins collapse through it, the margins of the next box
/// inward adjoin the edge too (the boxes after it at the block start, the
/// boxes before it at the block end), and so on until a box that does not
/// collapse through. Their sum is the one margin they collapse into (CSS 2
/// §8.3.1).
fn adjoining_margin_extremes(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    id: usize,
    start: bool,
    clearance_free: Option<&margin_clearance::ClearanceFree>,
) -> (f32, f32) {
    let layout = &document.nodes[id].layout;
    let margin = if start {
        layout.margin.top
    } else {
        layout.margin.bottom
    };
    let own = (margin.max(0.0), margin.min(0.0));
    let edges = if start {
        layout.border.top + layout.padding.top
    } else {
        layout.border.bottom + layout.padding.bottom
    };
    // The block-end margin of the last child adjoins the box's own only when
    // the box takes its height from its content (CSS 2 §8.3.1).
    if edges > 0.0
        || !margins_may_collapse_through(document, cascade, id)
        || (!start && !document.block_axis(id).height().is_auto())
    {
        return own;
    }
    let mut boxes = Vec::new();
    for &child in document.nodes[id].children.iter().rev() {
        push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
    }
    // The boxes in the order the edge meets them: `boxes` runs from the last
    // in-flow box to the first.
    if start {
        boxes.reverse();
    }
    let (mut positive, mut negative) = own;
    for (index, child) in boxes.into_iter().enumerate() {
        let child_layout = &document.nodes[child].layout;
        // A box with used clearance may have been put below a float, which
        // keeps its margin from adjoining this edge once an earlier box is
        // there. Use facts from before pagination moved the floats.
        let clears = cascade.computed[child].clear != ClearValue::None
            && !clearance_free.is_some_and(|facts| facts.contains(child));
        if document.nodes[child].kind() != NodeKind::Element
            || document.nodes[child].display().is_inline_level()
            || (index > 0 && clears)
        {
            break;
        }
        // The child's margin adjoins only when its border box touches this
        // box's content edge, which a used height or a line before it would
        // prevent, and so would a `max-height` that ends this box above the
        // end of a child with content. An empty child that its margins
        // collapse through may sit below that edge and still adjoin it.
        // Contact is measured in the flow, before relative offsets: an offset
        // moves the box without moving its margins (CSS 2 §9.4.3).
        let child_y = normal_flow_y(document, cascade, id, child);
        let empty = collapses_through(document, cascade, child, child_layout.size.height);
        let touches = if start {
            child_y <= 0.001
        } else {
            let child_end = child_y + child_layout.size.height;
            child_end >= layout.size.height - 0.001
                && (empty || child_end <= layout.size.height + 0.001)
        };
        if !touches {
            break;
        }
        // An empty box lets its margins collapse through it, so all of them,
        // and those of the boxes inside it, meet the margin of the box after
        // it, which adjoins this edge too.
        if !clears && empty {
            let (child_positive, child_negative) =
                empty_box_margin_extremes(document, cascade, child);
            positive = positive.max(child_positive);
            negative = negative.min(child_negative);
            continue;
        }
        let (child_positive, child_negative) =
            adjoining_margin_extremes(document, cascade, child, start, clearance_free);
        positive = positive.max(child_positive);
        negative = negative.min(child_negative);
        break;
    }
    (positive, negative)
}

/// The largest positive margin and the most negative one (zero when there is
/// none) among the block-start and block-end margins of `id`, a box that
/// [`collapses_through`], and of the boxes inside it, which are empty as well:
/// they all collapse into one margin.
fn empty_box_margin_extremes(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    id: usize,
) -> (f32, f32) {
    let margin = document.nodes[id].layout.margin;
    let mut positive = margin.top.max(margin.bottom).max(0.0);
    let mut negative = margin.top.min(margin.bottom).min(0.0);
    let mut boxes = Vec::new();
    for &child in &document.nodes[id].children {
        push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
    }
    // Text and inline-level boxes make no margin of their own here.
    for child in boxes.into_iter().filter(|&child| {
        document.nodes[child].kind() == NodeKind::Element
            && !document.nodes[child].display().is_inline_level()
    }) {
        let (child_positive, child_negative) = empty_box_margin_extremes(document, cascade, child);
        positive = positive.max(child_positive);
        negative = negative.min(child_negative);
    }
    (positive, negative)
}

/// The outer block extents `(top, bottom)` of the floats that come before `id`
/// among its siblings, in the coordinates of their parent, the ones `id`'s
/// lines were broken against. The floats before a `display: contents` box
/// that holds `id` are not among its siblings, and so are not listed.
fn preceding_float_extents(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
) -> Vec<(f32, f32)> {
    let parent = document.layout_parent_of(id);
    let siblings = parent.map_or(&[][..], |parent| &document.nodes[parent].children[..]);
    let position = siblings
        .iter()
        .position(|&sibling| sibling == id)
        .unwrap_or(0);
    siblings[..position]
        .iter()
        // A footnote is placed on a page of its own and takes no room from
        // the lines.
        .filter(|&&sibling| {
            is_floating_box_for_pagination(document, cascade, sibling)
                && float_side_index(cascade.computed[sibling].float).is_some()
                && document.layout_parent_of(sibling) == parent
        })
        .map(|&float| float_outer_extent(document, float))
        .collect()
}

/// The block-axis extent `(top, bottom)` of the margin box of the float `id`,
/// in the coordinates of its parent.
fn float_outer_extent(document: &Document, id: usize) -> (f32, f32) {
    let layout = &document.nodes[id].layout;
    (
        document.pagination_local_y(id) - layout.margin.top,
        document.pagination_local_y(id) + layout.size.height + layout.margin.bottom,
    )
}

/// Whether moving the box `id`, whose border box starts at `top`, down by
/// `delta` leaves every line below it where it belongs.
///
/// Each line was broken against the floats `floats` (their outer block
/// extents, in the coordinates `top` is in), so a line may only move when no
/// float's lower edge falls between its old and its new place: the line would
/// keep the room it was broken for, or the one it had been given, beside a
/// float that is no longer there. A paragraph whose lines leave a gap, since a
/// float put one below itself, stays too, as does one whose lines sit beside
/// floats that are not among `floats`. A paragraph that mixes in block
/// children or generated blocks is not looked into, since the blocks leave
/// gaps of their own. Neither is a box, `id` or one below it, that is not a
/// plain block, or clears a float.
fn lines_stay_put_when_moved(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
    top: f32,
    delta: f32,
    floats: &[(f32, f32)],
) -> bool {
    // A box of a formatting context of its own is placed beside the floats as
    // a whole, not line by line, and one in vertical writing breaks its lines
    // along the other axis.
    if !margins_may_collapse_through(document, cascade, id)
        || document.nodes[id].used_writing_mode.is_vertical()
    {
        return false;
    }
    if let Some(ifc) = document
        .ifc_layout_node(id)
        .and_then(|node| node.ifc.as_ref())
    {
        let layout = &document.nodes[id].layout;
        let content_top = top + layout.border.top + layout.padding.top;
        return ifc.generated_blocks.is_empty()
            && ifc.lines.as_ref().is_none_or(|lines| {
                lines.placed_blocks.is_empty()
                    && lines.block_line_starts.is_empty()
                    && lines_keep_their_room(lines, content_top, delta, floats)
            });
    }
    let mut boxes = Vec::new();
    for &child in document.nodes[id].children.iter().rev() {
        push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
    }
    // A box that clears a float or sits beside it in a formatting context of
    // its own may have been put below the float as well.
    boxes.into_iter().all(|child| {
        cascade.computed[child].clear == ClearValue::None
            && document.layout_parent_of(child) == Some(id)
            && lines_stay_put_when_moved(
                document,
                cascade,
                child,
                top + document.pagination_local_y(child),
                delta,
                floats,
            )
    })
}

/// Whether each of `lines`, whose content box starts at `content_top`, follows
/// the line before it and sits beside the same floats of `floats` when moved
/// down by `delta` as where it is. Lines that sit beside floats that `floats`
/// does not hold fail it.
fn lines_keep_their_room(
    lines: &crate::layout::ifc::root::IfcLines,
    content_top: f32,
    delta: f32,
    floats: &[(f32, f32)],
) -> bool {
    let beside = |top: f32, bottom: f32, float: (f32, f32)| {
        top < float.1 - 0.001 && float.0 < bottom - 0.001
    };
    let mut end = 0.0;
    let mut beside_a_float = false;
    let steady = lines.lines.iter().all(|line| {
        let follows = line.block_offset() <= end + 0.001;
        end = line.block_offset() + line.block_size();
        let top = content_top + line.block_offset();
        let bottom = top + line.block_size();
        follows
            && floats.iter().all(|&float| {
                let was = beside(top, bottom, float);
                beside_a_float |= was;
                was == beside(top + delta, bottom + delta, float)
            })
    });
    steady && (beside_a_float || !lines.beside_floats)
}

/// How much the margins of the first in-flow children of `id` that adjoin its
/// block-start margin, the boxes after an empty first child among them, change
/// the margin it keeps at a forced break: the one margin they collapse into,
/// less the margin of `id` itself.
fn collapsed_start_margin_gain(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    id: usize,
    clearance_free: &margin_clearance::ClearanceFree,
) -> f32 {
    let (positive, negative) =
        adjoining_margin_extremes(document, cascade, id, true, Some(clearance_free));
    positive + negative - document.nodes[id].layout.margin.top
}

/// Whether the block-start and block-end margins of `id`, a box with block
/// size `block_size`, collapse through it (CSS 2 §8.3.1): it has no block
/// size, it is a block box that establishes no independent formatting
/// context, and it has no in-flow content.
fn collapses_through(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    id: usize,
    block_size: f32,
) -> bool {
    block_size <= 0.0
        && margins_may_collapse_through(document, cascade, id)
        && !has_in_flow_content(document, cascade, id)
}

/// Whether `id` is a block box (a list item included) that establishes no
/// independent formatting context, the only kind whose margins can collapse
/// through it when it is empty (CSS 2 §8.3.1).
fn margins_may_collapse_through(document: &Document, cascade: &CascadeResult, id: usize) -> bool {
    let computed = &cascade.computed[id];
    let visible = |overflow| matches!(overflow, OverflowValue::Visible | OverflowValue::Clip);
    matches!(
        document.nodes[id].display(),
        DisplayValue::Block | DisplayValue::ListItem
    ) && visible(computed.overflow.x)
        && visible(computed.overflow.y)
        && document.nodes[id].multicol.is_none()
        && !document.nodes[id].roots_block_context()
        && !establishes_containment_context(computed)
        && !super::page::establishes_orthogonal_flow(document, cascade, id)
}

#[cfg(test)]
mod tests;

/// The heights of an auto-height box after a descendant moved: see
/// [`grown_auto_height`].
struct GrownHeight {
    /// The used border-box height.
    height: f32,
    /// The height the flow after the box follows, before and after the move.
    flow_heights: (f32, f32),
    /// The change to the furthest same-BFC descendant float end.
    float_extent_delta: f32,
    /// Whether `max-height` keeps the box from holding its moved content.
    capped: bool,
    /// How far the flow after the box moves for the bottom margin of its
    /// last box beyond the change of its height: the margin collapses
    /// through the bottom of a box without bottom padding or border only
    /// while its height is that of its content, and a `min-height` that the
    /// moved content leaves behind lets it through (see
    /// [`escaped_end_margin_shift`]).
    margin_shift: f32,
}

/// Whether the height of `block` can grow with its content: it is `auto`, or a
/// percentage against an indefinite containing block, which behaves as `auto`
/// (CSS 2.1 §10.5).
fn height_can_grow(document: &super::context::LayoutSession<'_>, block: usize) -> bool {
    document.block_axis(block).height().is_auto()
        || definite_content_height(document, block).is_none()
}

/// Whether the box of the paragraph `block` can grow when its lines move
/// down: its height can grow with its content (see [`height_can_grow`]) and
/// no size containment ignores that content.
pub(in crate::layout) fn lines_move_grows_box(
    document: &super::context::LayoutSession<'_>,
    block: usize,
) -> bool {
    height_can_grow(document, block) && !document.nodes[block].size_containment.height
}

/// The `min-height` and `max-height` of `block` as border-box heights (CSS 2.1
/// §10.7); `max` is infinite when nothing caps the height.
pub(in crate::layout) fn border_box_height_bounds(
    document: &super::context::LayoutSession<'_>,
    block: usize,
) -> (f32, f32) {
    let layout = &document.nodes[block].layout;
    let block_axis = document.block_axis(block);
    let vertical_edges =
        layout.padding.top + layout.padding.bottom + layout.border.top + layout.border.bottom;
    let box_sizing_edges = if document.pagination_style(block).box_sizing() == BoxSizing::ContentBox
    {
        vertical_edges
    } else {
        0.0
    };
    // A percentage bound resolves against a containing block of definite
    // height (CSS 2.1 §10.7).
    let containing_height = BoxTree::for_layout(document)
        .parent(block, BoxParentScope::Document)
        .and_then(|containing| definite_content_height(document, containing));
    // One that does not resolve behaves as `0` for `min-height` and as
    // `none` for `max-height`.
    let bound = |value: LengthPercentageAuto, auto: f32| {
        resolve_length_percentage_auto(value, containing_height)
            .map_or(auto, |value| value + box_sizing_edges)
    };
    let min = bound(block_axis.min_height(), 0.0);
    let max = bound(block_axis.max_height(), f32::INFINITY).max(min);
    (min, max)
}

/// The border-box height of a box that was `old` tall, with `min` and `max`
/// as its height bounds and `edges_below` of bottom padding and border, once
/// its content end moves from `before` to `after`, and whether `max-height`
/// cut the grown height short.
pub(in crate::layout) fn grown_height(
    old: f32,
    (min, max): (f32, f32),
    edges_below: f32,
    (before, after): (f32, f32),
) -> (f32, bool) {
    let bounded = old <= min + 0.001 || old >= max - 0.001;
    let unconstrained = if bounded { before + edges_below } else { old };
    let delta = after - before;
    let grown = unconstrained + delta;
    let height = if delta < 0.0 {
        grown.clamp(min, max).min(old)
    } else {
        grown.clamp(min, max).max(old)
    };
    (height, grown > max + 0.001)
}

/// The used border-box height of `parent` after its child `moved` (already
/// moved by `flow_delta`, or grown by `size_delta` while the in-flow content
/// inside it moved by `flow_delta`) and the in-flow children after it shift
/// down by `flow_delta`, clamped by `min-height` and `max-height` (CSS 2.1
/// §10.7). A fixed-height box keeps its height, but can still pass a changed
/// float extent to an ancestor that holds those floats. `None` when the
/// movement does not affect tracked content. `lines_moved` tells whether the
/// lines of `parent`'s paragraph after `moved` already carry the shift.
///
/// A box that holds its floats grows to their margin-box end; a box that
/// shares its BFC with an ancestor passes the float-end change to that root.
/// The flow after a non-BFC box follows only its in-flow content (CSS 2.1
/// §10.6.3). `ifc_boxes` records paragraph boxes moved by the IFC
/// block-following pass and those that stayed in place, whose float ends
/// must not be projected as ordinary later siblings.
#[allow(clippy::too_many_arguments)]
fn grown_auto_height(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    parent: usize,
    moved: usize,
    flow_delta: f32,
    size_delta: f32,
    float_delta: f32,
    lines_moved: bool,
    body_id: usize,
    ifc_boxes: &IfcBoxShifts,
) -> Option<GrownHeight> {
    let node = &document.nodes[parent];
    let can_grow = height_can_grow(document, parent);
    let layout = &node.layout;
    let (min, max) = border_box_height_bounds(document, parent);
    let old = layout.size.height;
    // The content extent before and after the shift, from the in-flow
    // children's margin boxes when bottom padding or border, or a formatting
    // context of the box's own, keeps the last child's margin inside it
    // (CSS 2.1 §8.3.1).
    let edges_below = layout.padding.bottom + layout.border.bottom;
    // A formatting context of the box's own also holds its floats (CSS 2.1
    // §10.6.7).
    let contains_floats = holds_floats(document, cascade, parent);
    let contains_margins = edges_below > 0.0 || contains_floats;
    // The boxes after the moved one already sit at their new places when
    // the container moved them along with it.
    let siblings_moved = moves_later_siblings(document, parent);
    // The content's end before and after the shift: `flow` for the edge
    // the flow after the box follows, `size` for the box's border box.
    let mut flow = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    let mut size = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    let mut float_extent = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    let mut shifted = false;
    let mut float_shifted = false;
    // Each child with the ends of its flow and border box, before and after.
    let mut child_ends: Vec<(usize, [(f32, f32); 2])> = Vec::new();
    // The ends of `moved` and of the children after it, before and after
    // the shift, from their current `end`. `moved_by` is how far `moved`'s
    // end moved.
    let shift = |child: usize, end: f32, moved_by: f32, shifted: &mut bool| {
        if child == moved {
            *shifted = true;
            (end - moved_by, end)
        } else if *shifted && (ifc_boxes.shifted.contains(&child) || siblings_moved) {
            (end - flow_delta, end)
        } else if *shifted {
            (end, end + flow_delta)
        } else {
            (end, end)
        }
    };
    let shift_float = |child: usize, end: f32, moved_by: f32, shifted: &mut bool| {
        if child == moved {
            *shifted = true;
            (end - moved_by, end)
        } else if *shifted && (ifc_boxes.shifted.contains(&child) || siblings_moved) {
            (end - flow_delta, end)
        } else if *shifted && ifc_boxes.boxes.contains(&child) {
            (end, end)
        } else if *shifted {
            (end, end + flow_delta)
        } else {
            (end, end)
        }
    };
    let extend = |range: &mut (f32, f32), (before, after): (f32, f32)| {
        range.0 = range.0.max(before);
        range.1 = range.1.max(after);
    };
    // The box whose margin box ends the content, for the last child: the
    // margin below it ends the content too when the box keeps it inside, and
    // may pull the end up (see `flow_end_box`).
    let end_box = |child: usize| {
        flow_end_box(document, cascade, child).map(|(end, (positive, negative))| {
            (
                end,
                if contains_margins {
                    positive + negative
                } else {
                    0.0
                },
            )
        })
    };
    if let Some([cached_flow, cached_size, cached_float]) = normal_block_shifts::extents(
        document,
        parent,
        moved,
        flow_delta,
        size_delta,
        float_delta,
        contains_floats,
        end_box,
    ) {
        flow = cached_flow;
        size = cached_size;
        float_extent = cached_float;
        shifted = true;
        float_shifted = true;
    } else {
        // Contents and non-offset inline wrappers contribute their children to
        // this box's layout, matching the layout-parent walk. A grid's items
        // follow one another in the order of their rows.
        let ordered = pagination_child_order_borrowed(document, cascade, parent);
        // The children of `display: contents` boxes found so far, last first:
        // they come before the rest of `ordered`.
        let mut unwrapped = Vec::new();
        let mut rest = ordered.iter();
        while let Some(child) = unwrapped.pop().or_else(|| rest.next().copied()) {
            #[cfg(test)]
            tests::record_normal_sibling_work(2, 1);
            let child_node = &document.nodes[child];
            if document.nodes[child].display() == DisplayValue::Contents
                || !document.contributes_layout_offset(child)
            {
                unwrapped.extend(child_node.children.iter().rev().copied());
                continue;
            }
            if child_node.kind() == NodeKind::Text
                || document.nodes[child].display() == DisplayValue::None
                || child_node.layout_position() == Position::Absolute
            {
                continue;
            }
            if child == moved {
                // Later siblings move with an in-flow block even when that block
                // has no float of its own.
                float_shifted = true;
            }
            let child_layout = &child_node.layout;
            let top = pagination_child_parent_y(
                document,
                parent,
                child,
                document.pagination_local_y(child),
            );
            if is_floating_box_for_pagination(document, cascade, child) {
                if matches!(
                    cascade.computed[child].float,
                    FloatValue::Left | FloatValue::Right
                ) {
                    let moved_by = if child == moved {
                        float_delta
                    } else {
                        flow_delta
                    };
                    let ends = shift_float(
                        child,
                        top + child_layout.size.height + child_layout.margin.bottom,
                        moved_by,
                        &mut float_shifted,
                    );
                    extend(&mut float_extent, ends);
                    shifted |= child == moved;
                }
                continue;
            }
            let mut descendant_float_extent = (f32::NEG_INFINITY, f32::NEG_INFINITY);
            visit_same_bfc_floats_in_subtree(
                document,
                cascade,
                body_id,
                child,
                top,
                |float, end| {
                    let already_followed = ifc_boxes
                        .shifted
                        .iter()
                        .any(|&box_id| is_descendant_or_self(document, float, box_id));
                    let is_ifc_box = ifc_boxes
                        .boxes
                        .iter()
                        .any(|&box_id| is_descendant_or_self(document, float, box_id));
                    let ends = if child == moved {
                        (end - float_delta, end)
                    } else if float_shifted && (already_followed || siblings_moved) {
                        (end - flow_delta, end)
                    } else if float_shifted && is_ifc_box {
                        (end, end)
                    } else if float_shifted {
                        (end, end + flow_delta)
                    } else {
                        (end, end)
                    };
                    extend(&mut descendant_float_extent, ends);
                },
            );
            if descendant_float_extent.0.is_finite() {
                extend(&mut float_extent, descendant_float_extent);
            }
            let relative_offset = layout_relative_offset_y(document, cascade, body_id, child);
            let size_end = top + child_layout.size.height - relative_offset;
            let size_ends = shift(child, size_end, size_delta, &mut shifted);
            let flow_ends = if contains_floats {
                size_ends
            } else {
                let end = top + child_layout.size.height - relative_offset;
                shift(child, end, flow_delta, &mut shifted)
            };
            child_ends.push((child, [flow_ends, size_ends]));
        }
        // Only the margin box of the box that ends the flow ends the content
        // (CSS 2 §10.6.3), with its bottom margin, which may be negative,
        // below it; what the boxes before it reach overflows the box.
        let end = child_ends.last().map(|&(last, last_ends)| {
            end_box(last)
                .and_then(|(end, margin)| {
                    child_ends
                        .iter()
                        .rfind(|&&(child, _)| child == end)
                        .map(|&(_, ends)| (ends, margin))
                })
                .unwrap_or_else(|| {
                    let margin = if contains_margins {
                        end_margin_through(document, cascade, last)
                    } else {
                        0.0
                    };
                    (last_ends, margin)
                })
        });
        if let Some(([flow_ends, size_ends], margin)) = end {
            extend(&mut flow, (flow_ends.0 + margin, flow_ends.1 + margin));
            extend(&mut size, (size_ends.0 + margin, size_ends.1 + margin));
        }
    }
    if contains_floats {
        extend(&mut flow, float_extent);
        extend(&mut size, float_extent);
    }
    // The lines of a paragraph laid out around block children end the
    // content too; those after the moved box move with it.
    if let Some(lines) = document
        .ifc_layout_node(parent)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        && let Some(last) = lines.lines.len().checked_sub(1)
        && let Some(&(_, first_after)) = lines
            .block_line_starts
            .iter()
            .find(|(block, _)| *block == moved)
    {
        let bottom = layout.padding.top
            + layout.border.top
            + lines.line_top(last)
            + lines.lines[last].block_size();
        let ends = match (last >= first_after, lines_moved) {
            (false, _) => (bottom, bottom),
            (true, true) => (bottom - flow_delta, bottom),
            (true, false) => (bottom, bottom + flow_delta),
        };
        extend(&mut flow, ends);
        extend(&mut size, ends);
    }
    // The in-flow content also ends the box.
    extend(&mut size, flow);
    // The grown height, and whether `max-height` cut it short.
    let grow = |old: f32, ends: (f32, f32)| grown_height(old, (min, max), edges_below, ends);
    let (in_flow_height, capped) = if can_grow {
        grow(old, flow)
    } else {
        (old, false)
    };
    let float_extent_delta = if float_shifted && float_extent.0.is_finite() {
        float_extent.1 - float_extent.0
    } else {
        0.0
    };
    // Without a `min-height` the height is always that of the content.
    let margin_shift = if can_grow
        && !contains_margins
        && !capped
        && border_box_height_bounds(document, parent).0 > 0.0
    {
        escaped_end_margin_shift(document, cascade, parent, (old, in_flow_height), flow)
    } else {
        0.0
    };
    shifted.then(|| GrownHeight {
        height: if can_grow {
            grow(old, size).0.max(in_flow_height)
        } else {
            old
        },
        flow_heights: (old, in_flow_height),
        float_extent_delta,
        capped,
        margin_shift,
    })
}

/// How far the flow after the block `parent`, with no bottom padding or
/// border, moves for the bottom margin of its last box when its height goes
/// from `heights.0` to `heights.1` and the end of its content from
/// `ends.0` to `ends.1`: that margin collapses through the bottom of the
/// block, and moves the flow after it, only while the block's height is
/// that of its content, which `min-height` may make taller, as in Chromium.
/// It then collapses with the margins after the block, its own bottom margin
/// and the top margins of the box after it.
fn escaped_end_margin_shift(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    parent: usize,
    heights: (f32, f32),
    ends: (f32, f32),
) -> f32 {
    let Some((_, (positive, negative))) = last_flow_box(document, cascade, parent)
        .and_then(|last| flow_end_box(document, cascade, last))
    else {
        return 0.0;
    };
    let (after_positive, after_negative) = margin_after_box(document, cascade, parent);
    let margin = positive.max(after_positive) + negative.min(after_negative)
        - (after_positive + after_negative);
    let escapes = |height: f32, end: f32| (height - end).abs() <= CONTENT_END_TOLERANCE;
    match (escapes(heights.0, ends.0), escapes(heights.1, ends.1)) {
        // A positive margin that starts to collapse through would reach past
        // the page end, where a break truncates it, so only a negative one
        // moves the flow.
        (false, true) => margin.min(0.0),
        // The margin the layout let through, of either sign, no longer is.
        (true, false) => -margin,
        _ => 0.0,
    }
}

/// The gap after the block `previous`, `gap` in the layout, once a `min-height`
/// holds the end of its content above its bottom: the bottom margin of its
/// last box, which the layout let through when the content reached that
/// bottom, then stays inside the block (see [`escaped_end_margin_shift`]),
/// and only the margins after the block are left between it and the box
/// after it.
fn escape_free_gap(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    previous: usize,
    gap: f32,
) -> f32 {
    let layout = &document.nodes[previous].layout;
    if gap <= 0.0
        || document.nodes[previous].kind() != NodeKind::Element
        || layout.padding.bottom + layout.border.bottom > 0.0
        || border_box_height_bounds(document, previous).0 <= 0.0
    {
        return gap;
    }
    // The margin still collapses through while the content reaches the
    // bottom, as `escaped_end_margin_shift` measures it.
    // A relative offset of that box does not move its margin.
    let reaches_bottom = |last: usize| {
        normal_flow_bottom(document, cascade, previous, last)
            >= layout.size.height - CONTENT_END_TOLERANCE
    };
    if last_flow_box(document, cascade, previous).is_none_or(reaches_bottom) {
        return gap;
    }
    let (positive, negative) = margin_after_box(document, cascade, previous);
    gap.min(positive + negative)
}

/// The extremes of the margins that collapse below the box `node`: its own
/// bottom margin and the margins that adjoin it from the boxes that follow
/// it in the flow, `display: contents` ones looked into. Empty boxes there
/// let all of their margins through to the next box, whose top margins
/// adjoin too (see [`adjoining_margin_extremes`]). When no box follows, the
/// margin reaches past the end of the parent: through a `display: contents`
/// one, and through the bottom of a block that takes its height from its
/// content, whose own bottom margin joins it (CSS 2 §8.3.1).
fn margin_after_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node: usize,
) -> (f32, f32) {
    let bottom = document.nodes[node].layout.margin.bottom;
    let (mut positive, mut negative) = (bottom.max(0.0), bottom.min(0.0));
    let mut current = node;
    while let Some(parent) = document.parent_of(current) {
        let mut boxes = Vec::new();
        for &sibling in document.nodes[parent]
            .children
            .iter()
            .rev()
            .take_while(|&&sibling| sibling != current)
        {
            push_in_flow_boxes_rev(document, cascade, sibling, &mut boxes);
        }
        for next in boxes.into_iter().rev() {
            // Text and inline-level boxes make a line, which no margin
            // crosses.
            if document.nodes[next].kind() != NodeKind::Element
                || document.nodes[next].display().is_inline_level()
            {
                return (positive, negative);
            }
            let empty = cascade.computed[next].clear == ClearValue::None
                && collapses_through(
                    document,
                    cascade,
                    next,
                    document.nodes[next].layout.size.height,
                );
            let (next_positive, next_negative) = if empty {
                empty_box_margin_extremes(document, cascade, next)
            } else {
                adjoining_margin_extremes(document, cascade, next, true, None)
            };
            positive = positive.max(next_positive);
            negative = negative.min(next_negative);
            if !empty {
                return (positive, negative);
            }
        }
        if document.nodes[parent].kind() != NodeKind::Element {
            break;
        }
        if document.nodes[parent].display() != DisplayValue::Contents {
            let layout = &document.nodes[parent].layout;
            if layout.padding.bottom + layout.border.bottom > 0.0
                || !margins_may_collapse_through(document, cascade, parent)
                || !document.block_axis(parent).height().is_auto()
                || border_box_height_bounds(document, parent).0 > 0.0
            {
                break;
            }
            let margin = layout.margin.bottom;
            positive = positive.max(margin);
            negative = negative.min(margin);
        }
        current = parent;
    }
    (positive, negative)
}

/// The furthest left or right float end in `branch`, in `parent`'s flow
/// coordinates. Traversal stops at an independent BFC so its floats stay
/// represented by its border box.
#[cfg(test)]
fn same_bfc_float_end_in_subtree(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    branch: usize,
    branch_top: f32,
) -> f32 {
    let mut end = f32::NEG_INFINITY;
    visit_same_bfc_floats_in_subtree(
        document,
        cascade,
        body_id,
        branch,
        branch_top,
        |_, float_end| end = end.max(float_end),
    );
    end
}

/// Whether `node_id` has an element among its children.
fn has_element_child(document: &Document, node_id: usize) -> bool {
    document.nodes[node_id]
        .children
        .iter()
        .any(|&id| document.nodes[id].kind() == NodeKind::Element)
}

/// Whether an element below `branch` is a left or right float that the walk
/// of its subtree can reach, which does not go into a box that holds its own
/// floats. The answer reads the styles of the boxes it reaches and nothing
/// else, so it may be `true` for a float that the walk skips for another
/// reason, but is never `false` for one that it visits.
fn has_float_below(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    branch: usize,
) -> bool {
    // The stack only allocates for a branch with nested elements.
    let mut nested = Vec::new();
    let mut parent = branch;
    loop {
        for &id in &document.nodes[parent].children {
            #[cfg(test)]
            tests::FLOAT_PREFILTER_VISITS.with(|count| count.set(count.get() + 1));
            let node = &document.nodes[id];
            if node.kind() != NodeKind::Element {
                continue;
            }
            if matches!(
                cascade.computed[id].float,
                FloatValue::Left | FloatValue::Right
            ) {
                return true;
            }
            // A `display: contents` element has no box, so its children stand
            // in its parent's. Any other box that holds its floats keeps them
            // from the branch, and the style of the box decides that: a flex
            // or grid item below the branch has its container in between,
            // which the walk never goes through.
            if has_element_child(document, id)
                && (node.display() == DisplayValue::Contents
                    || !holds_floats_by_style(document, cascade, id))
            {
                nested.push(id);
            }
        }
        match nested.pop() {
            Some(next) => parent = next,
            None => return false,
        }
    }
}

/// Visits each left or right float end in `branch`, in `parent`'s flow
/// coordinates. Traversal stops at an independent BFC so its floats stay
/// represented by its border box. Returns false if any reached float end
/// is nonfinite; the remaining finite ends are still visited.
fn visit_same_bfc_floats_in_subtree(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    branch: usize,
    branch_top: f32,
    mut visit: impl FnMut(usize, f32),
) -> bool {
    // A box without an element in it has no float to visit, which the walk
    // below would find out only after the style queries about the box.
    if document.nodes[branch].kind() != NodeKind::Element || !has_element_child(document, branch) {
        return true;
    }
    if holds_floats(document, cascade, branch)
        || (matches!(cascade.computed[branch].float, FloatValue::Footnote)
            && is_floating_box_for_pagination(document, cascade, branch))
        // cov:ignore: abs/fixed are caught above; running() is normalized by cascade.
        || !matches!(
            document.nodes[branch].position(),
            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
        )
    {
        return true;
    }
    // A box with no float below it that the walk reaches has nothing to
    // visit, and the style queries of the walk below are skipped. A box that
    // holds its floats itself is rejected above, so its subtree is not read.
    if !has_float_below(document, cascade, branch) {
        return true;
    }
    #[cfg(test)]
    tests::FLOAT_SUBTREE_WALKS.with(|count| count.set(count.get() + 1));
    let mut finite = true;
    let branch_offset = branch_top
        - layout_relative_offset_y(document, cascade, body_id, branch)
        - if document.contributes_layout_offset(branch) {
            super::ifc::outside_marker::box_leading(document, branch)
        } else {
            document.pagination_local_y(branch)
        };
    let mut pending: Vec<(usize, f32)> = document.nodes[branch]
        .children
        .iter()
        .rev()
        .map(|&child| (child, branch_offset))
        .collect();
    while let Some((child, offset)) = pending.pop() {
        let node = &document.nodes[child];
        if node.kind() != NodeKind::Element || !node.is_in_document() {
            continue;
        }
        let layout = &node.layout;
        if node.display() == DisplayValue::None {
            continue;
        }
        if node.display() == DisplayValue::Contents {
            let child_offset = if document.contributes_layout_offset(child) {
                offset + document.pagination_local_y(child)
            } else {
                offset
            };
            pending.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|&descendant| (descendant, child_offset)),
            );
            continue;
        }
        if matches!(cascade.computed[child].float, FloatValue::Footnote)
            && is_floating_box_for_pagination(document, cascade, child)
        {
            continue;
        }
        if matches!(
            document.nodes[child].position(),
            PositionValue::Absolute | PositionValue::Fixed
        ) {
            continue;
        }
        if matches!(
            cascade.computed[child].float,
            FloatValue::Left | FloatValue::Right
        ) {
            let bottom = offset
                + document.pagination_local_y(child)
                + layout.size.height
                + layout.margin.bottom;
            if bottom.is_finite() {
                visit(child, bottom);
            } else {
                finite = false;
            }
            continue;
        }
        if holds_floats(document, cascade, child) {
            continue;
        }
        let relative_offset = layout_relative_offset_y(document, cascade, body_id, child);
        let child_offset = if document.contributes_layout_offset(child) {
            offset + document.pagination_local_y(child)
                - relative_offset
                - super::ifc::outside_marker::box_leading(document, child)
        } else {
            offset - relative_offset
        };
        pending.extend(
            node.children
                .iter()
                .rev()
                .map(|&descendant| (descendant, child_offset)),
        );
    }
    finite
}

/// The content-box height of `block` when it is definite: a length, or a
/// percentage of a definite containing block, the initial containing block
/// being definite (CSS 2.1 §10.5). `stretch` fills a definite containing block
/// as a percentage does, and is `auto` otherwise (CSS Sizing 4 §5.1).
fn definite_content_height(
    document: &super::context::LayoutSession<'_>,
    block: usize,
) -> Option<f32> {
    let node = &document.nodes[block];
    let height = document.block_axis(block).height().expand();
    let definite = if matches!(
        height,
        ExpandedDimension::Percent(_) | ExpandedDimension::Calc(_) | ExpandedDimension::Stretch
    ) {
        BoxTree::for_layout(document)
            .parent(block, BoxParentScope::Document)
            .is_none_or(|containing| {
                document.nodes[containing].kind() == NodeKind::Document
                    || definite_content_height(document, containing).is_some()
            })
    } else {
        matches!(height, ExpandedDimension::Length(_))
    };
    definite.then(|| node.layout.content_box_height())
}

/// Whether `node_id` is a block box that takes no block space, so that only
/// margins lie between the boxes on either side of it.
fn takes_no_block_space(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    let layout = &node.layout;
    node.kind() == NodeKind::Element
        && matches!(
            document.nodes[node_id].display(),
            DisplayValue::Block | DisplayValue::ListItem | DisplayValue::FlowRoot
        )
        && layout.size.height <= 0.001
        && layout.padding.top + layout.padding.bottom + layout.border.top + layout.border.bottom
            <= 0.0
}

/// Pushes `node_id` onto `boxes` when it is an in-flow box, or else the
/// in-flow boxes that a `display: contents` element lifts into its parent,
/// last first.
fn push_in_flow_boxes_rev(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    boxes: &mut Vec<usize>,
) {
    match sibling_adjacency::flow_box(document, cascade, node_id) {
        sibling_adjacency::FlowBox::Skip | sibling_adjacency::FlowBox::Phantom => {}
        sibling_adjacency::FlowBox::Contents => {
            for &id in document.nodes[node_id].children.iter().rev() {
                push_in_flow_boxes_rev(document, cascade, id, boxes);
            }
        }
        sibling_adjacency::FlowBox::Box => boxes.push(node_id),
    }
}

/// The items that sit beside `source`, or beside a box that holds it, on a
/// flex line or grid row and come after it: a break forced after `source`
/// does not fall before them, but waits for what follows the line or row
/// (CSS Flexbox 1 §10, CSS Grid 1 §12: a break between items of one line or
/// row is taken after it). Items of a later line or row, out-of-flow boxes
/// and the anonymous items of text are not among them.
fn later_line_peers(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    source: usize,
) -> Vec<usize> {
    let mut peers = Vec::new();
    let mut item = source;
    while let Some(container) = document.layout_parent_of(item) {
        let node = &document.nodes[container];
        if !node.used_writing_mode.is_vertical() {
            match node.display() {
                DisplayValue::Flex | DisplayValue::InlineFlex => {
                    break_flex_line_peers(document, cascade, body_id, container, item, &mut peers);
                }
                DisplayValue::Grid | DisplayValue::InlineGrid => {
                    grid_row_peers(document, container, item, &mut peers);
                }
                _ => {}
            }
        }
        item = container;
    }
    peers
}

/// The in-flow element children of `container` after `item`.
fn later_in_flow_items(document: &Document, container: usize, item: usize) -> Vec<usize> {
    in_flow_items(document, container)
        .into_iter()
        .skip_while(|&child| child != item)
        .skip(1)
        .collect()
}

/// The in-flow element children of `container`.
fn in_flow_items(document: &Document, container: usize) -> Vec<usize> {
    document.nodes[container]
        .children
        .iter()
        .copied()
        .filter(|&child| {
            let node = &document.nodes[child];
            node.kind() == NodeKind::Element
                && !matches!(node.display(), DisplayValue::None | DisplayValue::Contents)
                && !matches!(
                    node.position(),
                    PositionValue::Absolute | PositionValue::Fixed
                )
        })
        .collect()
}

/// Push the later items of the row flex container `container` that share
/// the flex line of its item `item`: every item when the container does not
/// wrap, and otherwise those on the line of `item` (see [`flex_line_of`]).
/// The lines of a container that wraps in reverse stack upwards, above the
/// line of `item`, so all of its later items stay with it, as Chromium prints
/// them.
fn break_flex_line_peers(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    container: usize,
    item: usize,
    peers: &mut Vec<usize>,
) {
    let computed = &cascade.computed[container];
    if !matches!(
        computed.flex_direction,
        FlexDirectionValue::Row | FlexDirectionValue::RowReverse
    ) {
        return;
    }
    let later = later_in_flow_items(document, container, item);
    if computed.flex_wrap == FlexWrapValue::Wrap {
        let line = flex_line_of(
            document,
            cascade,
            body_id,
            container,
            in_flow_items(document, container),
            item,
        );
        peers.extend(later.into_iter().filter(|id| line.contains(id)));
    } else {
        peers.extend(later);
    }
}

/// Push the later items of the grid `container` that start within the rows
/// its item `item` spans.
fn grid_row_peers(document: &Document, container: usize, item: usize, peers: &mut Vec<usize>) {
    let rows = &document.nodes[container].grid_item_rows;
    let row_of = |id: usize| rows.iter().find(|row| row.item == id);
    // An item the grid placed no row for has no row to share.
    let span = row_of(item);
    peers.extend(
        later_in_flow_items(document, container, item)
            .into_iter()
            .filter(|&later| {
                span.zip(row_of(later))
                    .is_some_and(|(span, row)| row.start >= span.start && row.start < span.end)
            }),
    );
}

/// A forced break after a box, kept attached to that box until its
/// descendants have been visited. Applying it to the first flex child would
/// turn a break-after on the flex container into an extra blank page.
#[derive(Default)]
struct PendingBreak {
    source: Option<usize>,
    /// The boxes beside the source on its flex line or grid row, which the
    /// break does not fall before (see [`later_line_peers`]).
    line_peers: Vec<usize>,
    /// Where the box that forced the break ends in the flow, its end margin
    /// left out.
    source_end: f32,
    /// Whether the source is the last box at the end of the innermost box in
    /// `enclosing`, so that its break is that box's break too.
    ends_enclosing: bool,
    /// The breaks forced after boxes that hold the source, outermost first.
    /// Each still falls after its own box once the break inside it has been
    /// taken.
    enclosing: Vec<EnclosingBreak>,
    /// The nearest absolutely positioned box around the source, if any: a
    /// break forced inside one falls before the next box inside it only and
    /// never reaches the flow outside (Chromium).
    scope: Option<usize>,
    /// The breaks still pending when a break was forced inside a positioned
    /// box that does not hold their sources, each with that box: they fall
    /// after it once the flow leaves it.
    suspended: Vec<(usize, PendingBreak)>,
}

/// A pending break forced after a box that holds the source of a later one
/// (see [`PendingBreak::note_after`]).
struct EnclosingBreak {
    source: usize,
    line_peers: Vec<usize>,
    source_end: f32,
    ends_enclosing: bool,
    scope: Option<usize>,
}

impl PendingBreak {
    /// Whether the pending break falls before `node_id`, which is outside the
    /// box that forced it.
    ///
    /// A break forced after an item of a flex line or grid row does not fall
    /// between that item and the later items of the same line or row, which
    /// sit beside it: it waits for the box after the line, as browsers print
    /// it.
    fn applies_to(&self, document: &Document, node_id: usize) -> bool {
        self.source.is_some_and(|source| {
            in_break_scope(document, self.scope, node_id)
                && !is_descendant_or_self(document, node_id, source)
                && !self
                    .line_peers
                    .iter()
                    .any(|&peer| is_descendant_or_self(document, node_id, peer))
        })
    }

    /// Take the pending break before `node_id`, which it applies to. The
    /// break forced after a box that holds the source is the same break when
    /// the source ends that box, or when that box ends before `node_id`; a
    /// float after the source takes both. Otherwise it is pending again, as
    /// it falls after its own box.
    fn take(&mut self, document: &Document, node_id: usize) {
        if !self.applies_to(document, node_id) {
            return;
        }
        self.source = None;
        self.line_peers.clear();
        let mut same_break = self.ends_enclosing;
        while let Some(outer) = self.enclosing.pop() {
            if !same_break && is_descendant_or_self(document, node_id, outer.source) {
                self.restore(outer);
                break;
            }
            same_break = outer.ends_enclosing;
        }
    }

    fn restore(&mut self, outer: EnclosingBreak) {
        self.source = Some(outer.source);
        self.line_peers = outer.line_peers;
        self.source_end = outer.source_end;
        self.ends_enclosing = outer.ends_enclosing;
        self.scope = outer.scope;
    }

    /// Drop the breaks forced inside an absolutely positioned box that
    /// `node_id` lies outside of: they break nothing after that box. The
    /// break forced after a box around that one, or one that was pending
    /// before the flow entered that box, is pending again.
    fn leave_scopes(&mut self, document: &Document, node_id: usize) {
        loop {
            if self.source.is_some() && !in_break_scope(document, self.scope, node_id) {
                self.source = None;
                self.line_peers.clear();
                while let Some(outer) = self.enclosing.pop() {
                    if in_break_scope(document, outer.scope, node_id) {
                        self.restore(outer);
                        break;
                    }
                }
            }
            if self
                .suspended
                .last()
                .is_none_or(|&(scope, _)| in_break_scope(document, Some(scope), node_id))
            {
                return;
            }
            // A break restored here may lie in a positioned box that the
            // node is outside of too: the next round drops it and restores
            // the one before it.
            if let Some((_, outer)) = self.suspended.pop()
                && self.source.is_none()
            {
                let suspended = std::mem::take(&mut self.suspended);
                *self = outer;
                self.suspended = suspended;
            }
        }
    }

    /// Take the pending break forced after a block inside `node_id` with the
    /// line that closes the paragraph of `node_id` after that block. The
    /// breaks forced after the boxes that end before that line go with it;
    /// a break forced after `node_id` or a box around it is pending again,
    /// since the line separates it from the one taken.
    fn take_by_closing_line(&mut self, document: &Document, node_id: usize) {
        self.source = None;
        self.line_peers.clear();
        while let Some(outer) = self.enclosing.pop() {
            if is_descendant_or_self(document, node_id, outer.source) {
                self.restore(outer);
                break;
            }
        }
    }

    /// Make `node_id` the source of the pending break when it forces a break
    /// after itself, unless that source is already the node itself. A break
    /// forced after a box inside the source falls before the next box in the
    /// source, and the source's own break waits for the box after the
    /// source; Chromium breaks twice.
    fn note_after(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        body_id: usize,
        node_id: usize,
        deferred_named_break_after: bool,
        held: impl FnOnce() -> bool,
    ) {
        // A forced break held by a clip box around the node breaks nothing;
        // a change of page name after it still applies.
        let breaks_after = (forces_page_break(document, cascade, node_id, false) && !held())
            || deferred_named_break_after;
        if !breaks_after || self.source == Some(node_id) {
            return;
        }
        if let Some(source) = self
            .source
            .filter(|&source| is_descendant_or_self(document, node_id, source))
        {
            self.enclosing.push(EnclosingBreak {
                source,
                line_peers: std::mem::take(&mut self.line_peers),
                source_end: self.source_end,
                ends_enclosing: self.ends_enclosing,
                scope: self.scope,
            });
        } else if let Some(scope) = break_scope(document, node_id)
            .filter(|&scope| self.source.is_some() && self.scope != Some(scope))
        {
            // A break that has not fallen yet, outside the positioned box
            // around the node, falls after that box.
            let suspended = std::mem::take(&mut self.suspended);
            let outer = std::mem::take(self);
            self.suspended = suspended;
            self.suspended.push((scope, outer));
        } else {
            // A source that does not hold the node is replaced, as are the
            // boxes around it that end before the node.
            self.enclosing
                .retain(|outer| is_descendant_or_self(document, node_id, outer.source));
        }
        // Only floats and out-of-flow boxes, which take no break of the flow,
        // may follow a box at the end of the one around it (CSS
        // Fragmentation 3 §4.1).
        self.ends_enclosing = self
            .enclosing
            .last()
            .is_some_and(|outer| at_block_edge(document, cascade, node_id, outer.source, false));
        self.source = Some(node_id);
        self.line_peers = later_line_peers(document, cascade, body_id, node_id);
        self.scope = break_scope(document, node_id);
    }
}

/// The nearest absolutely or fixed positioned box around `node_id`, which
/// keeps the breaks forced inside it to itself.
fn break_scope(document: &Document, node_id: usize) -> Option<usize> {
    document
        .layout_ancestors(node_id)
        .find(|&id| is_out_of_flow_positioned(document, id))
}

/// Whether a break forced inside `scope` (the flow outside every positioned
/// box when `None`) may fall before `node_id`.
fn in_break_scope(document: &Document, scope: Option<usize>, node_id: usize) -> bool {
    scope.is_none_or(|scope| node_id != scope && is_descendant_or_self(document, node_id, scope))
}

/// The outermost absolutely positioned box around `node_id`, or `node_id`
/// itself, when the flow puts it: it is left at its static position in block
/// flow, with no positioned box around it placed by an inset or fixed, and
/// no paragraph lays it out among its lines (a box there goes with them).
/// A flex or grid container, or a table part, places its positioned
/// children itself.
fn static_positioned_root(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<usize> {
    let positioned = std::iter::once(node_id)
        .chain(document.layout_ancestors(node_id))
        .filter(|&id| is_out_of_flow_positioned(document, id))
        .last()?;
    let parent = document.layout_parent_of(positioned)?;
    let in_lines = document
        .ifc_layout_node(parent)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .is_some_and(|lines| !lines.lines.is_empty());
    (document.nodes[positioned].position() == &PositionValue::Absolute
        && !in_inset_out_of_flow_subtree(document, cascade, node_id)
        && !in_lines
        && !is_flex_grid_or_table_display(document.nodes[parent].display())
        && !moves_reach_cell_row(document, positioned))
    .then_some(positioned)
}

/// The positioned boxes after the box `source` in document order and before
/// `next` (up to the end of the document when `None`), outside both: the
/// boxes around `next` are passed through, and the boxes inside a positioned
/// box or one that generates no box are left out. None when `next` does not
/// come after `source`, as a flex item that `order` moves may not.
fn positioned_boxes_after(document: &Document, source: usize, next: Option<usize>) -> Vec<usize> {
    let after_subtree = |mut id: usize| loop {
        let parent = document.parent_of(id)?;
        let siblings = &document.nodes[parent].children;
        // Searched from the end: the boxes after a break are near it.
        if let Some(&sibling) = siblings
            .iter()
            .rposition(|&child| child == id)
            .and_then(|index| siblings.get(index + 1))
        {
            return Some(sibling);
        }
        id = parent;
    };
    let mut found = Vec::new();
    let mut current = after_subtree(source);
    while let Some(id) = current {
        if Some(id) == next {
            return found;
        }
        current = if next.is_some_and(|next| document.is_ancestor_or_self(id, next)) {
            document.nodes[id].children.first().copied()
        } else if is_out_of_flow_positioned(document, id) {
            found.push(id);
            after_subtree(id)
        } else if document.nodes[id].display() == DisplayValue::None {
            after_subtree(id)
        } else {
            document.nodes[id]
                .children
                .first()
                .copied()
                .or_else(|| after_subtree(id))
        };
    }
    if next.is_some() { Vec::new() } else { found }
}

/// Whether `node_id` generates an absolutely or fixed positioned box. An
/// element with `display: contents` generates no box, so its children stay
/// in the flow around it.
pub(super) fn is_out_of_flow_positioned(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    matches!(
        node.position(),
        PositionValue::Absolute | PositionValue::Fixed
    ) && !matches!(node.display(), DisplayValue::Contents | DisplayValue::None)
}

/// The last flex item placed in each named flex context, keyed by context:
/// its block size and the page it was placed on.
#[derive(Default)]
struct PrecedingFlexItems {
    by_context: HashMap<usize, (f32, u32)>,
}

impl PrecedingFlexItems {
    /// Record a flex item of `context` with `height` on `page`. Returns
    /// whether the item it replaces had a block size and sat on that same
    /// page.
    fn record(&mut self, context: usize, height: f32, page: u32) -> bool {
        self.by_context.insert(context, (height, page)).is_some_and(
            |(previous_height, previous_page)| previous_height > 0.0 && previous_page == page,
        )
    }
}

/// Whether a box whose margin box starts at `margin_box_y` starts a page after
/// `current_page`, or at `page_start`, the start of `current_page`.
///
/// This matters when the page height changes after page zero: the
/// fixed-height pass may have considered the box to be on the current page,
/// while the scheduled pass places it exactly at the next page origin. A
/// border box that lands exactly on a page origin only because the preceding
/// collapsed margin filled the previous page has not opened that page: the
/// break still moves it, and the margin after the forced break is kept. A
/// margin box that starts part-way down a later page follows content that
/// runs on to that page, so it has not opened the page either.
fn margin_box_starts_page(
    page_origins: &PageOrigins,
    margin_box_y: f32,
    current_page: u32,
    page_start: f32,
) -> bool {
    let margin_box_page = page_origins
        .page_index_in_flow(margin_box_y)
        .unwrap_or(current_page);
    (margin_box_page > current_page
        && (margin_box_y - page_origins.origin(margin_box_page)).abs() <= 0.001)
        || (margin_box_y.is_finite() && (margin_box_y - page_start).abs() <= 0.001)
}

/// The page that a box which has to start a new page lands on: the page after
/// `current_page`, or `natural_page` when the box's position already falls on
/// a later page.
fn forced_target_page(current_page: u32, natural_page: u32) -> u32 {
    current_page.saturating_add(1).max(natural_page)
}

/// Where a box that crosses a page transition is placed, given the
/// `target_y` top of the page it moves to.
///
/// A forced break sits before the box's margin box, so a positive block-start
/// `margin_top` stays on the new page, and a negative one may pull the box back
/// above the page top (the `underflow-from-next-page` case from CSS Break).
/// A table's first row keeps the table's top edge, `table_row_lead`, above it.
/// An unforced move truncates the margin; a moved row group or table row keeps
/// its lead from the table's top edge. The box's `relative_offset` moves with
/// it to the page it starts on, after a forced break too: the offset only
/// paints the box away from its place in the flow.
///
/// `row_group_lead` is the lead of a row group that moves as a whole, if any.
fn transition_target_y(
    target_y: f32,
    margin_top: f32,
    relative_offset: f32,
    table_row_lead: f32,
    forced_transition: bool,
    row_group_lead: Option<f32>,
    table_row_overflow: bool,
) -> f32 {
    let underflow_y = target_y + margin_top;
    if forced_transition && margin_top < 0.0 && underflow_y < target_y {
        underflow_y + relative_offset
    } else if forced_transition {
        target_y + margin_top.max(0.0) + table_row_lead + relative_offset
    } else if let Some(lead) = row_group_lead {
        target_y + lead + relative_offset
    } else if table_row_overflow {
        target_y + table_row_lead + relative_offset
    } else {
        target_y + relative_offset
    }
}

/// Where a page transition places a box, and how far that moves the box,
/// and with it the flow after it, and the line it starts.
struct TransitionLanding {
    /// Where the box's border box starts after the move (see
    /// [`transition_target_y`]).
    target_y: f32,
    /// How far the box itself moves down, and the flow after it with it.
    node_delta: f32,
    /// How far the box's border box, at its place in the flow without its
    /// relative offset, would move down to the top of its page.
    shift_delta: f32,
}

impl TransitionLanding {
    /// The landing of a box that was materialized at `effective_y`, which
    /// includes its `relative_offset`, and moves to the page that starts at
    /// `page_top`.
    ///
    /// Both distances are the additional movement for this boundary, since the
    /// box already sits at `effective_y`. The line shift is measured from the
    /// box's place in the flow, without the offset.
    fn new(
        page_top: f32,
        effective_y: f32,
        margin_top: f32,
        relative_offset: f32,
        forced_transition: bool,
        overflow: &BoxOverflow,
    ) -> Self {
        let target_y = transition_target_y(
            page_top,
            margin_top,
            relative_offset,
            overflow.table_row_lead,
            forced_transition,
            overflow.row_group_lead,
            overflow.table_row,
        );
        Self {
            target_y,
            node_delta: target_y - effective_y,
            shift_delta: page_top - (effective_y - relative_offset),
        }
    }

    /// Whether both distances are usable numbers, so that the box can move.
    fn is_finite(&self) -> bool {
        self.node_delta.is_finite() && self.shift_delta.is_finite()
    }
}

/// Whether the edge at `page_top` of page `natural_page` falls inside the
/// margin between a previous sibling whose border box ends at
/// `previous_bottom` and a box whose border box starts at `effective_y`.
///
/// A border box ending exactly at the break still has an adjoining margin on
/// the following page. An empty box that a forced break placed at the page
/// top, which `follows_forced_break` reports, leaves its collapsed margin
/// after the break; a later, unforced page edge inside that margin still
/// falls in it.
fn page_edge_falls_in_margin(
    natural_page: u32,
    page_top: f32,
    previous_bottom: f32,
    effective_y: f32,
    follows_forced_break: bool,
) -> bool {
    let after_forced_break = follows_forced_break && (previous_bottom - page_top).abs() <= 0.001;
    natural_page > 0
        && effective_y > page_top + 0.001
        && previous_bottom <= page_top + 0.001
        && !after_forced_break
}

/// Whether a table row of `height` at `flow_y` crosses `page_end` and moves
/// whole to the next page, whose block size is `next_page_step`.
///
/// `needed` is the block size the row takes on the next page: its lead from
/// the page top, its height, and the copy of any repeated header above it.
fn table_row_moves(
    flow_y: f32,
    height: f32,
    needed: f32,
    page_start: f32,
    page_end: f32,
    next_page_step: f32,
) -> bool {
    needed <= next_page_step + 0.001
        && starts_on_page_and_runs_past_it(flow_y, flow_y + height, page_start, page_end)
}

/// Whether the table row `row`, of `height` at `flow_y`, starts on the page
/// before the one that starts at `page_start`, beside a cell of an earlier
/// row that spans it, and moves to that page whole because a cell of its own
/// breaks before any of its content there. Chromium then starts the row's
/// own cells at the top of the next page and keeps the row's height, while
/// the spanning cell goes on from where it broke. The flow reaches a row
/// beside a spanning cell only after that cell's content, which may have
/// taken it to the next page already.
fn spanned_row_restarts(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    (body_id, page_name): (usize, Option<&str>),
    row: usize,
    flow_y: f32,
    height: f32,
    page_start: f32,
) -> bool {
    if document.context.pagination_on_transposed_axes
        || flow_y >= page_start - 0.001
        || flow_y + height <= page_start + 0.001
    {
        return false;
    }
    let beside_span = document
        .ancestors(row)
        .find(|&id| {
            matches!(
                document.nodes[id].display(),
                DisplayValue::Table | DisplayValue::InlineTable
            )
        })
        .is_some_and(|table| {
            super::table::row_moves::row_beside_spanning_cell(document, table, row)
        });
    let cells: Vec<usize> = document.nodes[row]
        .children
        .iter()
        .copied()
        .filter(|&cell| {
            document.nodes[cell].display() == DisplayValue::TableCell
                && sibling_renders(document, cell)
        })
        .collect();
    let row_top = cells
        .iter()
        .map(|&cell| document.pagination_local_y(cell))
        .fold(f32::INFINITY, f32::min);
    beside_span
        && cells.iter().any(|&cell| {
            !super::table::row_moves::spans_rows(document, cell)
                && first_content_breaks_before(
                    document,
                    cascade,
                    (body_id, page_name),
                    cell,
                    flow_y + document.pagination_local_y(cell) - row_top,
                    page_start,
                )
        })
}

/// Whether the first content in flow of `block`, whose border box starts at
/// `top`, is a box that no break goes through, as it avoids one or is
/// unbreakable, and that runs past `page_end`: nothing of the block's content
/// goes before the break. Chromium starts the row on the next page also when
/// that box is taller than the page and breaks inside it there. A break
/// forced inside the box leaves only the content before it to fit.
fn first_content_breaks_before(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    (body_id, page_name): (usize, Option<&str>),
    block: usize,
    top: f32,
    page_end: f32,
) -> bool {
    let first = document.nodes[block]
        .children
        .iter()
        .copied()
        .find(|&child| {
            sibling_renders(document, child)
                && !matches!(
                    document.nodes[child].position(),
                    PositionValue::Absolute | PositionValue::Fixed
                )
                && !is_floating_box_for_pagination(document, cascade, child)
        });
    first.is_some_and(|first| {
        let node = &document.nodes[first];
        if node.kind() != NodeKind::Element || node.display().is_inline_level() {
            return false;
        }
        let top = top + document.pagination_local_y(first);
        if box_avoids_page_break_inside(document, first, cascade.computed[first].break_inside)
            || box_is_unbreakable(document, cascade, first, false)
        {
            // Only the content before a break forced inside the box has to
            // fit, as for any box that avoids a break inside it.
            let end = forced_break_inside(
                document,
                cascade,
                body_id,
                first,
                page_name,
                false,
                page_end - top,
            )
            .map_or(top + node.layout.size.height, |offset| top + offset);
            return top < page_end && end > page_end + 0.001;
        }
        // Content that a box clipped at a height of its own lets overflow
        // past its end takes no room, so the box has to reach the page end
        // itself for its content to start the row on the next page.
        (!clips_at_bounded_height(document, cascade, first)
            || top + node.layout.size.height > page_end + 0.001)
            && first_content_breaks_before(
                document,
                cascade,
                (body_id, page_name),
                first,
                top,
                page_end,
            )
    })
}

/// Whether a run of `run_height` that avoids breaks inside it, starting at
/// `effective_y` below a block-start `margin_top`, moves whole to the next
/// page, whose block size is `next_page_step`.
///
/// A run of sibling blocks joined by `break-before` or `break-after: avoid`
/// stays intact when its block size, through the last box's trailing margin,
/// would cross the current page. It moves when it fits the next page, or,
/// when it is `firm`, whether or not it fits: no break inside a firm run is
/// better than the one before it (see [`HeldPart::firm`]), so Chromium takes
/// that one, and the run breaks inside on the next page.
fn avoided_run_moves(
    run_height: f32,
    firm: bool,
    margin_top: f32,
    effective_y: f32,
    page_start: f32,
    page_end: f32,
    next_page_step: f32,
) -> bool {
    run_height.is_finite()
        && run_height > 0.0
        && (firm || run_height + margin_top.max(0.0) <= next_page_step)
        && effective_y > page_start
        && effective_y < page_end
        && effective_y + run_height > page_end
}

/// Whether the box `node_id`, whose `break-inside` is `break_inside`, avoids
/// page breaks inside it.
///
/// `break-inside` does not apply to inline boxes (CSS Fragmentation 3 §3.2):
/// the block boxes inside an avoiding inline box break as they would without
/// it. An atomic inline, such as an inline table, is a single box in its line
/// and still moves as a whole.
fn box_avoids_page_break_inside(
    document: &Document,
    node_id: usize,
    break_inside: raikiri_style::property::BreakInside,
) -> bool {
    avoids_page_break_inside(break_inside)
        && (!document.nodes[node_id].display().is_inline_flow()
            || page_applies_to_box(document, node_id))
}

/// Whether the box `node_id` is replaced content, a full size containment box
/// or an orthogonal flow in a flow paginated along y, laid out as a
/// block-level box: no page break goes through it, so it moves to the next
/// page whole when it would cross the page end, as if it said
/// `break-inside: avoid`. It stays where it is only at the top of its page
/// (CSS Fragmentation 3 section 5.1.1, CSS Writing Modes 4 section 7.3,
/// Chromium).
///
/// A box with inline outer display is left out: it takes part in the lines of
/// its paragraph, which move it (see [`unplaced_line`]), and moving it on its
/// own as well would shift it twice. So is a positioned box taken out of the
/// flow: its offsets place it in its containing block, and moving it would
/// push the boxes that follow it.
fn box_is_unbreakable(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    pagination_on_transposed_axes: bool,
) -> bool {
    let node = &document.nodes[node_id];
    matches!(
        node.position(),
        PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
    ) && !node.display().is_inline_level()
        && (is_replaced_for_pagination(document, node_id)
            || size_contained_for_pagination(BoxTree::for_layout(document), node_id)
            || (!pagination_on_transposed_axes
                && super::page::establishes_orthogonal_flow(document, cascade, node_id)))
}

/// The single line of a paragraph that no text node places, as the block
/// that holds it sees it.
#[derive(Clone, Copy)]
struct UnplacedBlockLine {
    /// How far below the border-box top of the block the line starts.
    start: f32,
    /// How far below the border-box top of the block the line ends.
    end: f32,
    /// Whether the line holds atomic inlines and no text: no page break goes
    /// through it, as through the box of one.
    atomic: bool,
    /// Whether the block child of the paragraph laid out right above the
    /// line sits in no inline box and does not avoid a break after it, unless
    /// it forces one. The
    /// page then breaks between that block and the line, as before a line of
    /// text there, instead of keeping the paragraph whole (see
    /// [`unplaced_line_after_block`]).
    after_block: bool,
}

impl UnplacedBlockLine {
    /// Where the line ends below the border-box top of its block, of block
    /// size `height`: a block shorter than its line ends where the block does.
    fn end_within(self, height: f32) -> f32 {
        self.end.min(height)
    }
}

/// Whether the content box of the block `node_id`, of block size `height`,
/// ends at its unplaced `line`, or above it: no height of the block's own goes
/// on below the line.
fn unplaced_line_ends_content(
    document: &Document,
    node_id: usize,
    line: UnplacedBlockLine,
    height: f32,
) -> bool {
    let layout = &document.nodes[node_id].layout;
    height - layout.padding.bottom - layout.border.bottom - line.end <= 0.01
}

/// How far below the border-box top of the block `node_id`, of block size
/// `height`, its unplaced `line` keeps the block whole. The padding and border
/// below the line go with it when the content box ends at the line: no break
/// falls between a line and the edges after it, so Chromium moves the block
/// whole when they cross the page end. A box taller than its line ends where
/// the box does, and its edges follow that end, not the line.
fn unplaced_line_extent(
    document: &Document,
    node_id: usize,
    line: UnplacedBlockLine,
    height: f32,
) -> f32 {
    if unplaced_line_ends_content(document, node_id, line, height) {
        height
    } else {
        line.end_within(height)
    }
}

/// The line of the in-flow block `node_id` when its paragraph is a single line
/// that no text node places: generated text, a glyphless line, or atomic
/// inlines alone (see [`single_unplaced_line`]).
///
/// No text candidate moves such a line, so its block moves whole when the
/// line would cross a page edge, as a line of text does. Only a horizontal
/// paragraph is measured: the lines of a vertical one are columns.
fn unplaced_line(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<UnplacedBlockLine> {
    paragraph_unplaced_line(document, cascade, node_id).filter(|line| !line.after_block)
}

/// The single unplaced line of the in-flow block `node_id` (see
/// [`unplaced_line`]) when a block child of the paragraph is laid out above
/// it. Such a line does not keep the paragraph whole: Chromium breaks the
/// page between the block and the line, as before a line of text there, and
/// moves the line on, or truncates the block-end margin of the block above
/// it at the page edge (see `PaginationState::break_before_line_after_block`).
fn unplaced_line_after_block(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<UnplacedBlockLine> {
    paragraph_unplaced_line(document, cascade, node_id).filter(|line| line.after_block)
}

/// The single unplaced line of the in-flow block `node_id`, whatever comes
/// above it (see [`unplaced_line`] and [`unplaced_line_after_block`]).
fn paragraph_unplaced_line(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<UnplacedBlockLine> {
    let node = &document.nodes[node_id];
    if !node.is_ifc_root()
        || !is_block_level_flow_box(node.display())
        || node.used_writing_mode.is_vertical()
        || !matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
        )
        || is_floating_box_for_pagination(document, cascade, node_id)
    {
        return None;
    }
    let unplaced = single_unplaced_line(document, cascade, node_id)?;
    let ifc_node = document.ifc_layout_node(node_id)?;
    let layout = ifc_node.layout;
    let after_block =
        block_above_line(document, node_id, unplaced.line.line).is_some_and(|block| {
            let (forced, avoided) = page_edge_constraint(document, cascade, block, false);
            block_outside_inline(document, node_id, block) && (forced || !avoided)
        });
    Some(UnplacedBlockLine {
        start: layout.border.top + layout.padding.top + unplaced.line.top,
        end: layout.border.top + layout.padding.top + unplaced.line.bottom,
        atomic: unplaced.atomic,
        after_block,
    })
}

/// The block child of the paragraph `root` laid out last above its line
/// `line`, if any.
fn block_above_line(document: &Document, root: usize, line: usize) -> Option<usize> {
    document
        .ifc_layout_node(root)?
        .ifc
        .as_ref()?
        .lines
        .as_ref()?
        .block_line_starts
        .iter()
        .rev()
        .find(|&&(_, after)| after <= line)
        .map(|&(block, _)| block)
}

/// The paragraph whose single unplaced line follows the block that ends with
/// `source`, a box that forces a page break after it (see
/// [`unplaced_line_after_block`]). No text candidate takes the break before
/// such a line, so the line takes it: Chromium starts the next page with it,
/// as with a line of text there. A float of the paragraph after `source`
/// takes the break first, so the line is left to follow it.
fn line_after_forced_break(
    document: &Document,
    cascade: &CascadeResult,
    source: usize,
) -> Option<usize> {
    // The paragraph may hold `source` inside blocks that end with it, each
    // of them a paragraph of its own.
    for root in document.ancestors(source) {
        if !document.nodes[root].is_ifc_root() {
            continue;
        }
        if unplaced_line_after_block(document, cascade, root).is_some()
            && let Some(unplaced) = single_unplaced_line(document, cascade, root)
            && let Some(block) = block_above_line(document, root, unplaced.line.line)
            && document.is_ancestor_or_self(block, source)
            && (block == source || at_block_edge(document, cascade, source, block, false))
        {
            return (!float_after(document, cascade, root, source)).then_some(root);
        }
        if !at_block_edge(document, cascade, source, root, false) {
            return None;
        }
    }
    None
}

/// Whether a float of the paragraph `root` follows `source` in it.
fn float_after(document: &Document, cascade: &CascadeResult, root: usize, source: usize) -> bool {
    document
        .preorder(root)
        .skip_while(|&id| id != source)
        .filter(|&id| !document.is_ancestor_or_self(source, id))
        .any(|id| is_floating_box_for_pagination(document, cascade, id))
}

/// Whether the block child `block` of the paragraph `root` sits in no inline
/// box of it, in the tree of elements: the boxes of the paragraph place the
/// block straight in the root. Chromium keeps a block inside an inline box
/// together with the line after it, as part of the line of that inline box.
fn block_outside_inline(document: &Document, root: usize, block: usize) -> bool {
    std::iter::successors(document.nodes[block].parent, |&id| {
        document.nodes[id].parent
    })
    .take_while(|&ancestor| ancestor != root)
    .all(|ancestor| !document.nodes[ancestor].display().is_inline_level())
}

/// How far below the border-box top of the block `node_id` the piece that
/// its single unplaced line keeps whole ends (see [`unplaced_line`]): the
/// line with the padding and border that close the block right after it (see
/// [`unplaced_line_extent`]). No text candidate moves such a line, and no
/// break falls between it and those edges, so Chromium moves them on
/// together, as it moves a paragraph of one line of text with its edges.
/// `None` when the block holds anything besides that line, such as a block
/// box laid out among or after its lines.
fn unplaced_line_piece(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<(UnplacedBlockLine, f32)> {
    let ifc_node = document.ifc_layout_node(node_id)?;
    let ifc = ifc_node.ifc.as_ref()?;
    let lines = ifc.lines.as_ref()?;
    if !ifc.generated_blocks.is_empty()
        || !lines.placed_blocks.is_empty()
        || !lines.block_line_starts.is_empty()
    {
        return None;
    }
    let height = document.nodes[node_id].layout.size.height;
    unplaced_line(document, cascade, node_id)
        .map(|line| (line, unplaced_line_extent(document, node_id, line, height)))
}

/// Whether the flex or grid item `candidate`, placed as `fit` says, moves to
/// the next page for its single unplaced line (see [`unplaced_line_piece`]):
/// the item starts on the page below its top, and the line with the edges
/// that close the item after it runs past the page end. A line of text in
/// the same place is moved by its text candidate, with those edges; Chromium
/// moves a line of generated text or atomic inlines alone the same way,
/// whether or not the item is the last one, and also when the piece is
/// taller than the next page.
fn unplaced_item_moves(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    candidate: &PageCandidate,
    fit: &BoxFit,
) -> bool {
    let node_id = candidate.node_id;
    (candidate.context.is_flex_item() || candidate.context.is_grid_item())
        && !document.context.pagination_on_transposed_axes
        // An item that already starts its page gains nothing by moving.
        && fit.flow_y - fit.margin_top.max(0.0) > fit.page_start + 0.001
        && unplaced_line_piece(document, cascade, node_id).is_some_and(|(line, piece)| {
            fit.flow_y + line.start < fit.page_end && fit.flow_y + piece > fit.page_end + 0.001
        })
}

/// The deepest box [`first_piece_end`] looks into.
const FIRST_PIECE_DEPTH: usize = 8;

/// How far below the border-box top the first piece of the content of the box
/// `node_id` ends: the least block size from the top of the box that the page
/// breaks after, with its content as unbreakable as a line is. That is the end
/// of the first `orphans` lines of a paragraph, the bottom of a box that is
/// monolithic or avoids breaks inside it, and for a block, a flex container or
/// a grid, the first piece of its first box or, for the last two, of the item
/// whose first piece ends first. `None` for a box whose content is not read
/// this way: one with text of its own beside boxes, a float, generated content,
/// columns, a table, a vertical flow, or nothing at all, which can be cut
/// anywhere.
fn first_piece_end(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    depth: usize,
) -> Option<f32> {
    let node = &document.nodes[node_id];
    // A box in the orthogonal flow of its parent is monolithic, and its
    // block size along the parent's axis is the whole of it.
    let orthogonal_item = depth > 0 && establishes_orthogonal_flow(document, cascade, node_id);
    // A box inside the one asked about whose content is a single line that
    // no text places keeps that line and the edges after it together, its
    // generated text included.
    let unplaced = (depth > 0 && depth <= FIRST_PIECE_DEPTH && node.kind() == NodeKind::Element)
        .then(|| unplaced_line_piece(document, cascade, node_id))
        .flatten();
    if depth > FIRST_PIECE_DEPTH
        || node.kind() != NodeKind::Element
        || (node.used_writing_mode.is_vertical() && !orthogonal_item)
        || node.multicol.is_some()
        || (node.has_before_or_after_content && unplaced.is_none())
        || !matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative
        )
    {
        return None;
    }
    let computed = &cascade.computed[node_id];
    let height = node.layout.size.height;
    if avoids_page_break_inside(computed.break_inside)
        || is_monolithic_for_pagination_with_transposed_axes(document, cascade, node_id, false)
    {
        return Some(height);
    }
    if let Some((_, piece)) = unplaced {
        return Some(piece);
    }
    if node.is_ifc_root() {
        let ifc_node = document.ifc_layout_node(node_id)?;
        let lines = ifc_node.ifc.as_ref()?.lines.as_ref()?;
        // Block boxes between the lines are places to break at too.
        if !lines.block_line_starts.is_empty() || lines.lines.is_empty() {
            return None;
        }
        let kept = (computed.orphans.max(1) as usize).min(lines.lines.len()) - 1;
        let layout = ifc_node.layout;
        return Some(
            layout.border.top
                + layout.padding.top
                + lines.line_top(kept)
                + lines.lines[kept].block_size(),
        );
    }
    let stacks_every_box = matches!(node.display(), DisplayValue::Flex | DisplayValue::Grid);
    if !stacks_every_box
        && !matches!(
            node.display(),
            DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
        )
    {
        return None;
    }
    let mut boxes = Vec::new();
    for &child in &node.children {
        if is_floating_box_for_pagination(document, cascade, child) {
            return None;
        }
        let start = boxes.len();
        push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
        boxes[start..].reverse();
        // A block is judged by its first box; what follows it breaks away
        // from it at a better place.
        if !stacks_every_box && !boxes.is_empty() {
            break;
        }
    }
    // A box with nothing in it can be cut anywhere.
    if boxes.is_empty() {
        return None;
    }
    // The top line of a flex container cannot be cut where an item that
    // starts at its top and that no break goes through is still running, so
    // the container moves whole. Its first piece ends where the last of the
    // first pieces of those items does, not the first. An item that starts
    // lower is pushed on alone instead (those on the lines below, among
    // them). A cuttable lineless item contributes only the minimal piece
    // that keeps leading edges with it; an unread item adds no bound.
    if lays_out_in_flex_rows(document, cascade, node_id) {
        let layout = node.layout;
        let content_top = layout.border.top + layout.padding.top;
        return boxes
            .iter()
            .filter_map(|&child| {
                // The page breaks before the relative offset moves the box.
                let top = normal_flow_y(document, cascade, body_id, child);
                (top <= content_top + 0.001).then_some(top)?;
                let piece = first_piece_end(document, cascade, body_id, child, depth + 1).or_else(
                    || {
                        // A cuttable lineless item still keeps the edges above
                        // it when its content begins at or past the page end.
                        lineless_piece_end(document, cascade, body_id, child, depth + 1)
                            .map(|(end, _)| end)
                    },
                )?;
                Some(top + piece)
            })
            .reduce(f32::max);
    }
    let examined = if stacks_every_box { boxes.len() } else { 1 };
    let mut least = f32::INFINITY;
    for &child in &boxes[..examined] {
        // The page breaks before the relative offset moves the box.
        let end = normal_flow_y(document, cascade, body_id, child)
            + first_piece_end(document, cascade, body_id, child, depth + 1)?;
        least = least.min(end);
    }
    least.is_finite().then_some(least)
}

/// Whether `node_id` is a flex container that lays its items out in rows
/// across the page.
fn lays_out_in_flex_rows(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    document.nodes[node_id].display() == DisplayValue::Flex
        && matches!(
            cascade.computed[node_id].flex_direction,
            FlexDirectionValue::Row | FlexDirectionValue::RowReverse
        )
}

/// How far below the border-box top of the flex container or grid `node_id`
/// the first piece of its content ends, when it has padding or border above
/// its items, or lays them out in rows (see [`first_piece_end`]). Such a
/// container moves whole with its first piece even where nothing is above it,
/// as the items of its top line all begin at its top.
fn leading_piece_end(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    let layout = &document.nodes[node_id].layout;
    (layout.border.top + layout.padding.top > 0.0
        || lays_out_in_flex_rows(document, cascade, node_id))
    .then(|| first_piece_end(document, cascade, body_id, node_id, 0))
    .flatten()
}

/// The extent given to a box with nothing to keep together, which the page
/// can be cut through anywhere: just enough for a box that begins at the page
/// end to count as running past it.
const CUT_ANYWHERE_PIECE: f32 = 0.01;

/// How much room for content a box needs, past its padding and border, to
/// count as having a height of its own.
const HEIGHT_EPSILON: f32 = 0.001;

/// How far below the border-box top of the wrapper `node_id` the first
/// piece of its content ends, when padding or border sit above that content,
/// on `node_id` or on a box it begins, and the first boxes inside it, down to
/// the one that decides, hold no line to keep with those edges. A box that
/// avoids breaks inside it or that no break goes through ends where it does.
/// A box that holds no box of its own is one piece with its edges when it has
/// no height of its own, and is cut anywhere after the edges above it when it
/// has; one with no edges at all only counts when it begins past the page end
/// (see [`first_piece_end`]). Lines of text are left to the rules for lines,
/// but a paragraph of one line that no text places ends with that line (see
/// [`unplaced_line`]), and a box with no edges above it is left to the rules
/// for boxes that avoid a break. An eligible atomic inline keeps its entire
/// box with that piece.
fn lineless_leading_piece_end(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    lineless_piece_end(document, cascade, body_id, node_id, 0)
        .filter(|&(_, has_edges)| has_edges)
        .map(|(end, _)| {
            if document.nodes[node_id].display() == DisplayValue::InlineBlock {
                document.nodes[node_id].layout.size.height
            } else {
                end
            }
        })
}

/// Whether the root box can keep its leading edges with block content.
/// This does not change the block classification used for margin or run rules.
fn has_block_leading_content(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    node.kind() == NodeKind::Element
        && matches!(
            node.display(),
            DisplayValue::Block
                | DisplayValue::FlowRoot
                | DisplayValue::ListItem
                | DisplayValue::InlineBlock
        )
        && matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
        )
        && !node.used_writing_mode.is_vertical()
}

/// The end of the first piece of the block-like root `node_id` that holds no line
/// of text, from its border-box top, and whether padding or border sit above the
/// content that decides it (see [`lineless_leading_piece_end`]). The root has
/// no piece when it holds no box and has no edges at all, and a box deeper
/// down that holds no box is cut anywhere unless it has edges or a height.
fn lineless_piece_end(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    depth: usize,
) -> Option<(f32, bool)> {
    lineless_piece(document, cascade, body_id, node_id, depth)
        .map(|piece| (piece.end, piece.has_edges))
}

/// The first piece of a block that holds no line of text (see
/// [`lineless_piece_end`]).
#[derive(Clone, Copy)]
struct LinelessPiece {
    /// Where the piece ends, from the border-box top of the block.
    end: f32,
    /// Whether padding or border sit above the content that decides it.
    has_edges: bool,
    /// Whether the piece ends with a box that no break goes through.
    monolithic: bool,
}

fn lineless_piece(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    depth: usize,
) -> Option<LinelessPiece> {
    if depth > FIRST_PIECE_DEPTH
        || !(if depth == 0 {
            has_block_leading_content(document, node_id)
        } else {
            is_plain_block_box(document, node_id)
                && !document.nodes[node_id].is_multicol_container()
                && !is_floating_box_for_pagination(document, cascade, node_id)
        })
    {
        return None;
    }
    let node = &document.nodes[node_id];
    // A forced break before a box inside takes the page break there: moving
    // the block would only push that break on to one more page.
    if depth > 0 && forces_page_break(document, cascade, node_id, true) {
        return None;
    }
    // A paragraph of one line that no text places, of generated text or
    // atomic inlines alone, is kept whole by that line (see
    // `unplaced_line`): no break falls between the edges above it and the
    // line, as none falls above a box that no break goes through.
    // A box with a height of its own below the line is left to the rules for
    // its own box.
    let unplaced = if depth > 0 {
        unplaced_line(document, cascade, node_id).filter(|&line| {
            unplaced_line_ends_content(document, node_id, line, node.layout.size.height)
        })
    } else {
        None
    };
    if node.has_before_or_after_content && unplaced.is_none() {
        return None;
    }
    // The box the walk starts at is judged by the rules of its own.
    if depth > 0 {
        if is_monolithic_for_pagination_with_transposed_axes(document, cascade, node_id, false) {
            return Some(LinelessPiece {
                end: node.layout.size.height,
                has_edges: false,
                monolithic: true,
            });
        }
        if avoids_page_break_inside(cascade.computed[node_id].break_inside) {
            // A box that forces a break inside it splits there instead.
            return (!has_mandatory_break_inside(document, cascade, node_id)).then_some(
                LinelessPiece {
                    end: node.layout.size.height,
                    has_edges: false,
                    monolithic: false,
                },
            );
        }
        if let Some(line) = unplaced {
            return Some(LinelessPiece {
                end: line.end_within(node.layout.size.height),
                has_edges: false,
                monolithic: true,
            });
        }
    }
    if node.is_ifc_root() {
        return None;
    }
    let mut first = None;
    for &child in &node.children {
        if is_floating_box_for_pagination(document, cascade, child) {
            return None;
        }
        let mut boxes = Vec::new();
        push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
        if let Some(&box_id) = boxes.last() {
            first = Some(box_id);
            break;
        }
    }
    let layout = &node.layout;
    let edges_above = layout.border.top + layout.padding.top;
    let Some(child) = first else {
        // The box holds no box of its own, so the edges above its content are
        // the whole of what comes first. A box with nothing but edges (no
        // height of its own) goes as one piece, bottom edges included, as
        // browsers do; a box with room for content can be cut anywhere after
        // the edges above it, and a box with no edges at all is cut anywhere.
        let edges_below = layout.border.bottom + layout.padding.bottom;
        if depth == 0 && edges_above + edges_below == 0.0 {
            return None;
        }
        let has_room = layout.size.height - edges_above - edges_below > HEIGHT_EPSILON;
        // An inline-block that has room for content fragments across pages
        // (css/printing/fragmented-inline-block-002-print), so its edges do
        // not take the whole box to the next page.
        if depth == 0 && has_room && node.display() == DisplayValue::InlineBlock {
            return None;
        }
        let end = if has_room {
            edges_above + CUT_ANYWHERE_PIECE
        } else {
            layout.size.height.max(CUT_ANYWHERE_PIECE)
        };
        return Some(LinelessPiece {
            end,
            has_edges: edges_above > 0.0,
            monolithic: false,
        });
    };
    // The page breaks before the relative offset moves the box.
    let top = normal_flow_y(document, cascade, body_id, child);
    let piece = lineless_piece(document, cascade, body_id, child, depth + 1)?;
    // A box that no break goes through and that runs out past the bottom of
    // a block whose height is held below its content is visible overflow of
    // that block there, which the page cuts rather than moving the block on
    // for it. A box that only avoids a break inside it still takes the block
    // along, as Chromium prints it.
    let end = top + piece.end;
    Some(LinelessPiece {
        end: if piece.monolithic {
            end.min(layout.size.height)
        } else {
            end
        },
        // A box that establishes an independent formatting context keeps a
        // positive margin of its first child inside it (CSS 2 §8.3.1), which
        // a break before the child cannot truncate: it stays above the
        // content as edges would. Columns start a fragmentation context of
        // their own, which truncates the margin at their top instead.
        has_edges: piece.has_edges
            || edges_above > 0.0
            || (top > 0.01
                && node.multicol.is_none()
                && !margins_may_collapse_through(document, cascade, node_id)),
        monolithic: piece.monolithic,
    })
}

/// Whether a box with padding or border above its content moves to the next
/// page, so that those edges do not stay behind alone: the box starts on the
/// page, but the first piece of its content (see [`first_piece_end`], which
/// `leading_piece_end` and `leading_block_piece_end` are the end of, from the
/// top of the box) runs past the page end. A break between the edges and the
/// content is a worse one (class C) than the break before the box (class A,
/// CSS Break 3 §4.1), as browsers choose; a box that its page starts with would
/// break from the edges on any page.
///
/// The box moves even when the piece is taller than the next page: browsers
/// start the box on the next page then too, and the piece goes on below its
/// edges, splitting there if it can be split, or going on to the page after
/// if it is monolithic.
fn leading_edges_move(candidate: &PageCandidate, fit: &BoxFit) -> bool {
    let BoxFit {
        flow_y,
        margin_top,
        page_start,
        page_end,
        ..
    } = *fit;
    let moves =
        |end: f32| avoided_box_moves(flow_y, margin_top, flow_y + end, page_start, page_end);
    candidate.context.leading_piece_end().is_some_and(moves)
        || candidate
            .context
            .leading_block_piece_end()
            .is_some_and(moves)
}

/// Whether `break_inside` avoids page breaks inside its box.
fn avoids_page_break_inside(break_inside: raikiri_style::property::BreakInside) -> bool {
    matches!(
        break_inside,
        raikiri_style::property::BreakInside::Avoid
            | raikiri_style::property::BreakInside::AvoidPage
    )
}

/// Whether a box at `flow_y` that avoids breaks inside it, with its
/// block-start `margin_top`, moves whole to the next page: it starts on the
/// page spanning `page_start..page_end` and ends at `box_end` past that
/// page's end.
///
/// A box whose margin box is already at the top of its page would break
/// inside on any page, so moving it would only leave a blank page.
fn avoided_box_moves(
    flow_y: f32,
    margin_top: f32,
    box_end: f32,
    page_start: f32,
    page_end: f32,
) -> bool {
    flow_y - margin_top.max(0.0) > page_start + 0.001
        && starts_on_page_and_runs_past_it(flow_y, box_end, page_start, page_end)
}

/// Whether a box that no page break goes through (see [`box_is_unbreakable`])
/// moves to the next page: it starts on this page and runs past its end, and
/// does not start the page.
///
/// Unlike a block that avoids a break inside it, which stays where its margin
/// box starts the page, such a box stays only where its border box does.
/// Browsers truncate the margin at a break between pages but keep the one at
/// the start of the first page, which leaves a box with a margin there to
/// move on even when nothing precedes it.
fn unbreakable_box_moves(
    top: f32,
    margin_top: f32,
    flow_y: f32,
    box_end: f32,
    page_start: f32,
    page_end: f32,
) -> bool {
    // The first page starts at the flow origin; a margin above the box is
    // cut by any break after it.
    let top = if page_start > 0.001 {
        top - margin_top.max(0.0)
    } else {
        top
    };
    top > page_start + 0.001
        && starts_on_page_and_runs_past_it(flow_y, box_end, page_start, page_end)
}

/// Whether a box ending the blocks around it with `edges` of bottom padding
/// and border below it moves to the next page, so that those edges do not
/// break away from it.
///
/// When the last box of a block fits its page but the block's closing edges
/// do not, a break between the two would leave the edges alone at the top of
/// the next page. A break between boxes (a class A break point) is preferred
/// to that one (class C, CSS Break 3 §4.1), as browsers do, so the last box
/// moves along with the edges, even when the next page cannot hold the box
/// with all of them: the edges that still do not fit there start the page
/// after it (see `PaginationState::break_before_edges`).
/// The next page must hold the box itself, or moving it gains nothing; and a
/// box that its page starts with would break from the edges on any page.
///
/// A block that holds one box has the edges at its own end: its last
/// `closing_edges_inside` of block size follow that box, and the page breaks
/// before the block rather than between the box and them (see
/// [`lone_box_tail`]).
///
/// The first box of a run of boxes tied by avoided breaks that ends its block
/// moves with the `closing_edges_span` of block size that the rest of the run
/// takes below it, so that the run keeps together (see
/// [`avoid_chain_before_last_box`]).
///
/// A block that holds one box may be tied by an avoided break to the box before
/// it. The run of boxes those breaks tie cannot be kept together once its first
/// box starts its page and the block still does not fit after it, so the
/// avoided break gives way and the page breaks before the block, as browsers
/// do. While the first box sits further down the page, the run moves as a
/// whole instead, and the block does not move on its own.
fn closing_edges_move(candidate: &PageCandidate, fit: &BoxFit) -> bool {
    let edges = candidate.context.closing_edges();
    let inside = candidate.context.closing_edges_inside();
    let span = candidate.context.closing_edges_span();
    let reach = candidate.context.closing_edges_float_reach();
    let BoxFit {
        flow_y,
        margin_top,
        page_start,
        page_end,
        next_page_step,
        ..
    } = *fit;
    let box_end = flow_y + candidate.height + span;
    edges + inside > 0.0
        && box_end - inside + reach <= page_end + 0.001
        && candidate.height + span - inside + reach <= next_page_step + 0.001
        && candidate
            .context
            .lone_run_head_offset()
            .is_none_or(|offset| flow_y - offset <= page_start + 0.001)
        && avoided_box_moves(flow_y, margin_top, box_end + edges, page_start, page_end)
}

/// Where the lines in the flow of the box `node_id` end, from the top of its
/// border box, in normal-flow positions: its own last line, or the lowest end
/// of the lines inside a box in its flow. A box that clips its overflow keeps
/// them. `None` when the flow holds no lines.
fn lines_end_in_flow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    if element_clips_overflow(document, cascade, node_id) {
        return None;
    }
    if document.nodes[node_id].is_ifc_root() {
        return content_bottom(document, node_id);
    }
    let mut end: Option<f32> = None;
    let mut stack: Vec<usize> = document.nodes[node_id]
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(child) = stack.pop() {
        let node = &document.nodes[child];
        if node.kind() != NodeKind::Element || node.is_display_none() {
            continue;
        }
        // The children of a box without one of its own are in the flow
        // that holds it.
        if node.display() == DisplayValue::Contents {
            stack.extend(node.children.iter().rev().copied());
            continue;
        }
        let style = document.physical_style(child);
        if style.float().is_floated() || style.position().is_out_of_flow() {
            continue;
        }
        if let Some(child_end) = lines_end_in_flow(document, cascade, body_id, child) {
            // A relative offset moves the box after the page breaks.
            let top = pagination_child_parent_y(
                document,
                node_id,
                child,
                document.pagination_local_y(child),
            ) - layout_relative_offset_y(document, cascade, body_id, child);
            let child_end = top + child_end;
            end = Some(end.map_or(child_end, |end: f32| end.max(child_end)));
        }
    }
    end
}

/// Whether the lines of the box that the closing edges of the candidate follow
/// (the box itself, or the last of the run it starts) run past the end of that
/// box and of the page that holds it. Such a box breaks inside its
/// overflowing content, as Chromium does: it stays on its page, and the
/// padding and border that close the block around it go on to the next page
/// alone (see [`closing_edges_move`]).
fn lines_cross_page_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    candidate: &PageCandidate,
    fit: &BoxFit,
) -> bool {
    let last = candidate
        .context
        .closing_edges_last_box()
        .unwrap_or(candidate.node_id);
    let box_end = fit.flow_y + candidate.height + candidate.context.closing_edges_span();
    lines_end_in_flow(document, cascade, body_id, last).is_some_and(|end| {
        let overflow = end - document.nodes[last].layout.size.height;
        overflow > 0.001 && box_end + overflow > fit.page_end + 0.001
    })
}

/// Whether the last box of a run of boxes tied by avoided breaks moves to the
/// next page with the edges that close its block, the avoided break before it
/// giving way.
///
/// The run moves to the next page as one when the edges do not fit after it
/// (see [`closing_edges_move`]), and that happens before the walk reaches the
/// last box. When the edges still do not fit after the box, the run has moved
/// as far on as it goes, and the page breaks before the box instead, as
/// browsers do: a break between boxes beats one between the box and the edges
/// even when it is avoided. The box must end within its own page, and the next
/// page must hold it.
fn run_tail_move(candidate: &PageCandidate, fit: &BoxFit) -> bool {
    tail_edges_move(
        candidate.context.run_tail_edges(),
        candidate.context.closing_edges_float_reach(),
        candidate,
        fit,
    )
}

/// Whether the last item of a flex container that stacks its items, or the
/// last row of a table, moves to the next page with the `item_tail_edges` that
/// close the container, so that those edges do not break away from it: the
/// break between items or rows is the better one (see [`tail_edges_move`]).
fn item_tail_move(candidate: &PageCandidate, fit: &BoxFit) -> bool {
    tail_edges_move(candidate.context.item_tail_edges(), 0.0, candidate, fit)
}

/// Whether `node_id` is a flex container that [`flex_margins_move`] may move
/// whole: one with an item that has a block-end margin, or with padding or a
/// border that closes it below its items.
fn has_flex_trailing_space(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    node.formatting_context == FormattingContext::Flex
        && !node.used_writing_mode.is_vertical()
        && (node.layout.padding.bottom + node.layout.border.bottom > 0.001
            || node.layout_children().iter().any(|&item| {
                crate::layout::page::is_in_flow_flex_child_for_pagination(document, node_id, item)
                    && document.nodes[item].layout.margin.bottom > 0.001
            }))
}

/// Whether the flex container of `candidate`, placed as `fit` says, moves to
/// the next page whole because what follows an item that ends on this page
/// runs past the page end. Chromium does not truncate a flex item's block-end
/// margin at a page break as block flow does: when an item ends on the page
/// but its margin runs past the page end, the page breaks before the
/// container, which moves with all its items (in a column or a wrapping row
/// too, not just the item). The margin counts only as far as the container's
/// content box reaches. The padding and border that close the container move
/// it too when its items all end on the page and they follow a single line:
/// a row that does not wrap, or a column of one item. An item counts where
/// it stands in the flow, without its relative offset. Like a box that avoids
/// a break inside it, the container goes with the blocks it begins, and stays
/// where they start the page or where what has to fit does not fit the next
/// page either.
fn flex_margins_move(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    candidate: &PageCandidate,
    fit: &BoxFit,
) -> bool {
    let node_id = candidate.node_id;
    let node = &document.nodes[node_id];
    if node.formatting_context != FormattingContext::Flex
        || document.context.pagination_on_transposed_axes
        || node.used_writing_mode.is_vertical()
    {
        return false;
    }
    let BoxFit {
        flow_y,
        avoided_lead,
        avoided_margin_top,
        page_start,
        page_end,
        next_page_step,
        ..
    } = *fit;
    let layout = &node.layout;
    let border_box_end = flow_y + candidate.height;
    let content_end = border_box_end - layout.padding.bottom - layout.border.bottom;
    let style = document.pagination_style(node_id);
    let row = style.flex_direction_is_row();
    // A break forced between the items of a column or between the lines of
    // a wrapping row starts a page of its own, so only the items before it
    // decide whether the container moves. In a wrapping row, a break forced
    // before or after an item waits for the end of its flex line.
    let lines_break = !row || style.flex_wraps();
    let mut forced = false;
    let mut break_after = false;
    let mut items = 0;
    let mut items_end = flow_y;
    // An item starts a new flex line of a wrapping row when it starts back
    // before the end of the item before it along the main axis, as in
    // `flex_line_of`; the items of a line can sit at different heights.
    let computed = &cascade.computed[node_id];
    let forward = (computed.flex_direction == FlexDirectionValue::Row)
        == (computed.direction == raikiri_style::property::Direction::Ltr);
    let mut previous_span: Option<(f32, f32)> = None;
    let mut end = flow_y;
    for item in crate::layout::page::pagination_child_order(document, cascade, node_id) {
        if !crate::layout::page::is_in_flow_flex_child_for_pagination(document, node_id, item) {
            continue;
        }
        let computed = &cascade.computed[item];
        let item_layout = &document.nodes[item].layout;
        let top = flow_y + item_layout.location.y
            - layout_relative_offset_y(document, cascade, body_id, item);
        let x = item_layout.location.x - layout_relative_offset(document, cascade, body_id, item).0;
        let span = (
            x - item_layout.margin.left,
            x + item_layout.size.width + item_layout.margin.right,
        );
        let starts_line = !row
            || previous_span.is_none_or(|(start, end)| {
                if forward {
                    span.0 < end - 0.001
                } else {
                    span.1 > start + 0.001
                }
            });
        previous_span = Some(span);
        if lines_break
            && items > 0
            && starts_line
            && (break_after || page_break_is_forced(document, cascade, item, computed.break_before))
        {
            forced = true;
            break;
        }
        let bottom = top + item_layout.size.height;
        items += 1;
        items_end = items_end.max(bottom);
        if bottom <= page_end + 0.001 {
            end = end.max((bottom + item_layout.margin.bottom).min(content_end));
        }
        break_after |=
            lines_break && page_break_is_forced(document, cascade, item, computed.break_after);
    }
    let single_line = if row { !style.flex_wraps() } else { items == 1 };
    if single_line && !forced && items_end <= page_end + 0.001 && end >= content_end - 0.001 {
        end = border_box_end;
    }
    end > page_end + 0.001
        && end - (flow_y - avoided_lead) <= next_page_step + 0.001
        && avoided_box_moves(
            flow_y - avoided_lead,
            avoided_margin_top,
            end,
            page_start,
            page_end,
        )
}

/// Whether a box that ends within its page moves to the next page because the
/// `edges` that follow it do not fit there, when the next page can hold it and
/// it does not start its own page. The floats that reach `float_reach` below
/// the box must end within the page too, or they keep the box where it is.
fn tail_edges_move(edges: f32, float_reach: f32, candidate: &PageCandidate, fit: &BoxFit) -> bool {
    let BoxFit {
        flow_y,
        margin_top,
        page_start,
        page_end,
        next_page_step,
        ..
    } = *fit;
    edges > 0.0
        && flow_y + candidate.height + float_reach <= page_end + 0.001
        && candidate.height + float_reach <= next_page_step + 0.001
        && avoided_box_moves(
            flow_y,
            margin_top,
            flow_y + candidate.height + edges,
            page_start,
            page_end,
        )
}

/// The block size of the bottom padding and border of the block `node_id`
/// that close it below its last box, when they follow that box: the block
/// stacks its boxes in block flow (see [`block_flow_closing_edges`]), or is a
/// paragraph that ends with a block child after its lines (see
/// [`ends_with_block_child`]), and its content ends at that box (see
/// [`edges_below_last_box`]).
fn closing_edges_of(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    block_flow_closing_edges(document, cascade, body_id, node_id).or_else(|| {
        (!document.nodes[node_id].used_writing_mode.is_vertical()
            && ends_with_block_child(document, node_id))
        .then(|| edges_below_last_box(document, cascade, body_id, node_id))
        .flatten()
    })
}

/// The bottom padding and border of the block `node_id`, when its
/// `min-height` leaves room below its last box in the flow, where
/// [`closing_edges_of`] finds no edges that follow that box: moved content may
/// still grow the block past that room during pagination, and the edges then
/// follow the box (see [`min_height_edges_move`]). A `height` or `max-height`
/// that ends the box where it ends leaves no such room.
fn min_height_room_edges(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    let node = &document.nodes[node_id];
    let edges = node.layout.padding.bottom + node.layout.border.bottom;
    let shaped = !node.used_writing_mode.is_vertical()
        && ((node.kind() == NodeKind::Element && moves_later_siblings(document, node_id))
            || ends_with_block_child(document, node_id));
    (edges > 0.0
        && shaped
        && !height_follows_content(document, node_id)
        && !height_is_capped(document, node_id))
    .then(|| flow_content_overflow(document, cascade, body_id, node_id))
    .flatten()
    .filter(|&overflow| overflow < -CONTENT_END_TOLERANCE)
    .map(|_| edges)
}

/// Whether the bottom padding and border that close the block of set height
/// around `candidate` still follow it: the block ends where it ends, so once
/// pagination moved content in it down and the box now ends at or past the
/// block's end, the edges are above the box, and do not move it.
fn capped_edges_follow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    root_flow_offset: f32,
    candidate: &PageCandidate,
    fit: &BoxFit,
) -> bool {
    candidate.context.capped_edges_block().is_none_or(|block| {
        let block_end = current_abs_y(document, block)
            - accumulated_relative_offset_y(document, cascade, body_id, block)
            + root_flow_offset
            + document.nodes[block].layout.size.height;
        let margin = end_margin_through(document, cascade, candidate.node_id);
        fit.flow_y + candidate.height + margin < block_end - 0.001
    })
}

/// Whether a box that ends a block whose `min-height` left room below it
/// moves to the next page with the bottom padding and border of that block,
/// as [`tail_edges_move`] moves a box with the edges that follow it. That room
/// is gone once pagination moved the content above the box down far enough:
/// the block then grows with its content, and its edges follow the box, as in
/// Chromium. While the room is left, the page breaks inside it instead.
fn min_height_edges_move(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    root_flow_offset: f32,
    candidate: &PageCandidate,
    fit: &BoxFit,
) -> bool {
    let Some((block, edges)) = candidate.context.min_height_edges() else {
        return false;
    };
    let layout = &document.nodes[block].layout;
    let content_end = current_abs_y(document, block)
        - accumulated_relative_offset_y(document, cascade, body_id, block)
        + root_flow_offset
        + layout.size.height
        - layout.padding.bottom
        - layout.border.bottom;
    // The block grew to the margin box of the box as the box lays it out,
    // with its own bottom margin. The edges follow the margin that ends its
    // flow, which a margin of its last child collapsing through it may pull
    // up further.
    let own_margin = document.nodes[candidate.node_id].layout.margin.bottom;
    fit.flow_y + candidate.height + own_margin >= content_end - 0.001 && {
        let end_margin = if collapsed_empty_box(document, cascade, candidate.node_id) {
            0.0
        } else {
            end_margin_through(document, cascade, candidate.node_id)
        };
        tail_edges_move(edges + end_margin, 0.0, candidate, fit)
    }
}

/// Whether `node_id` is an empty box that its margins collapse through and
/// that stays with the box before it. Chromium leaves no break before such a
/// box: it does not move on with the edges that close its block, and a page
/// edge truncates the margins that collapse through it. A box of auto height
/// is one, and so is a box of set height zero once a margin other than zero
/// collapses into its top (see [`margin_collapses_into_top`]). Without one,
/// Chromium breaks before such a box as before any other: it moves on with
/// the edges, and keeps its bottom margin whole above them.
pub(super) fn collapsed_empty_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    collapses_as_empty_box(document, cascade, node_id)
        && (document.block_axis(node_id).height().is_auto()
            || (margin_collapses_into_top(document, cascade, node_id)
                && !splits_margin(document, cascade, node_id)))
}

/// Whether `node_id` is an empty element whose margins collapse through it,
/// with no clearance or forced break to keep them apart, whatever its height
/// says.
fn collapses_as_empty_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let node = &document.nodes[node_id];
    let computed = &cascade.computed[node_id];
    node.kind() == NodeKind::Element
        && node.layout.size.height.abs() <= 0.001
        // Clearance keeps the margins from collapsing through it, and a
        // forced break before or after it keeps them whole.
        && computed.clear == ClearValue::None
        && !page_break_is_forced(document, cascade, node_id, computed.break_before)
        && !page_break_after_is_forced(document, cascade, node_id, computed.break_after)
        && collapses_through(document, cascade, node_id, 0.0)
}

/// Whether a margin other than zero collapses into the top of the empty box
/// `node_id`: its own top margin, or the bottom margin of the box before it
/// in the flow, collapsed with those of the empty boxes between the two. A
/// `display: contents` element lends the boxes inside it to that flow, and
/// a forced break or text between the two keeps them apart. Chromium keeps
/// such a box behind even when the margins cancel out.
fn margin_collapses_into_top(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    document.nodes[node_id].layout.margin.top != 0.0 || margin_above_box(document, cascade, node_id)
}

/// Whether one of the margins that end the flow above the box `node_id`
/// is other than zero: the bottom margin of the box before it, collapsed
/// with those of the empty boxes between the two (see
/// [`margin_collapses_into_top`]).
fn margin_above_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let mut boxes = Vec::new();
    let mut current = node_id;
    while let Some(parent) = document.parent_of(current) {
        // Walked from the end, the box is reached past only the few that
        // follow it, as it usually ends its block.
        for &sibling in document.nodes[parent]
            .children
            .iter()
            .rev()
            .skip_while(|&&child| child != current)
            .skip(1)
        {
            boxes.clear();
            push_in_flow_boxes_rev(document, cascade, sibling, &mut boxes);
            for &id in &boxes {
                if document.nodes[id].kind() != NodeKind::Element
                    || page_break_after_is_forced(
                        document,
                        cascade,
                        id,
                        cascade.computed[id].break_after,
                    )
                {
                    return false;
                }
                if !collapses_as_empty_box(document, cascade, id) {
                    return non_zero_margin(adjoining_margin_extremes(
                        document, cascade, id, false, None,
                    ));
                }
                if non_zero_margin(empty_box_margin_extremes(document, cascade, id)) {
                    return true;
                }
            }
        }
        if document.nodes[parent].display() != DisplayValue::Contents {
            break;
        }
        current = parent;
    }
    false
}

/// The box whose margin box ends the flow of its parent when `last`, the
/// last box in that flow, is followed by nothing, and the margin below it:
/// the last box before the empty boxes that end the flow and stay with it
/// (see [`collapsed_empty_box`]), whose bottom margin collapses with all of
/// theirs into one, or `last` itself with its own bottom margin collapsed
/// with those inside it (see [`end_margin_through`]). The margin comes as
/// the extremes of the margins that collapse into it, the largest positive
/// and the most negative. `None` when no box that takes part in the flow
/// comes before those empty boxes.
fn flow_end_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    last: usize,
) -> Option<(usize, (f32, f32))> {
    if !collapsed_empty_box(document, cascade, last) {
        return Some((
            last,
            adjoining_margin_extremes(document, cascade, last, false, None),
        ));
    }
    // The block whose flow `last` ends: a `display: contents` element lends
    // the boxes inside it to the flow of its parent.
    let mut block = document.parent_of(last)?;
    while document.nodes[block].display() == DisplayValue::Contents {
        block = document.parent_of(block)?;
    }
    // The empty boxes that end the flow, `last` first, and the box before
    // them. Walked from the end, `last` is reached past only the few that
    // follow it outside the flow.
    let mut run = Vec::new();
    let mut before = None;
    // The children still to visit, last first, through the `display:
    // contents` elements on the way. The first box in the flow met so is
    // `last` when the flow ends with it, as it does for the callers.
    let mut pending: Vec<usize> = document.nodes[block].children.clone();
    while let Some(child) = pending.pop() {
        if !takes_part_in_flow(document, cascade, child) {
            continue;
        }
        if document.nodes[child].display() == DisplayValue::Contents {
            pending.extend(document.nodes[child].children.iter().copied());
            continue;
        }
        if !collapses_as_empty_box(document, cascade, child) {
            before = Some(child);
            break;
        }
        run.push(child);
    }
    run.first().filter(|&&first| first == last)?;
    // Which of them stay with the box before them, found from the first on:
    // each one adds its margins to those that collapse into the tops of the
    // boxes after it (see `collapsed_empty_box`).
    let mut stays = vec![false; run.len()];
    let mut above = run
        .last()
        .is_some_and(|&first| margin_above_box(document, cascade, first));
    for (index, &child) in run.iter().enumerate().rev() {
        stays[index] = document.block_axis(child).height().is_auto()
            || ((document.nodes[child].layout.margin.top != 0.0 || above)
                && !splits_margin(document, cascade, child));
        above |= non_zero_margin(empty_box_margin_extremes(document, cascade, child));
    }
    // The last box that does not stay ends the flow, with the margins of
    // those after it collapsed into its own.
    let (mut positive, mut negative) = (0.0_f32, 0.0_f32);
    let end = run
        .iter()
        .zip(&stays)
        .find_map(|(&child, &stays)| {
            (!stays).then_some(child).or_else(|| {
                let (child_positive, child_negative) =
                    empty_box_margin_extremes(document, cascade, child);
                positive = positive.max(child_positive);
                negative = negative.min(child_negative);
                None
            })
        })
        .or(before)?;
    let (end_positive, end_negative) =
        adjoining_margin_extremes(document, cascade, end, false, None);
    Some((
        end,
        (positive.max(end_positive), negative.min(end_negative)),
    ))
}

/// Whether margin extremes hold a margin other than zero.
fn non_zero_margin((positive, negative): (f32, f32)) -> bool {
    positive > 0.0 || negative < 0.0
}

/// [`closing_edges_of`] for a block that stacks its boxes in block flow and
/// holds no lines. The body is such a block as well.
fn block_flow_closing_edges(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    let node = &document.nodes[node_id];
    (node.kind() == NodeKind::Element
        && !node.used_writing_mode.is_vertical()
        && moves_later_siblings(document, node_id))
    .then(|| edges_below_last_box(document, cascade, body_id, node_id))
    .flatten()
}

/// How close, in pixels, the end of the flow content of a block of set height
/// must come to the end of its content box to count as reaching it. Half a
/// pixel absorbs the rounding of line boxes, as the edges of a paragraph's
/// lines do (see `PaginationState::keep_lines_together`).
const CONTENT_END_TOLERANCE: f32 = 0.5;

/// How far the bottom padding and border of the block `node_id` reach below
/// its last box in the flow, when they come right after it, so that a break
/// between the two would leave the edges alone at the top of the next page.
///
/// That is so when its height is that of its content, and also when a set
/// `height`, `min-height` or `max-height` ends its content box where its flow
/// content ends or above that end: Chromium then moves the last box to the
/// next page with the edges, as it does in a block of auto height, and the
/// block keeps the height its constraints give it. The content reaches past
/// the content box by its overflow, which the edges then cover in part, so
/// only the rest of them counts. A taller box leaves room between its last
/// box and the edges, where the page breaks instead (see
/// [`content_ends_above_edges`]).
fn edges_below_last_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    let node = &document.nodes[node_id];
    let edges = node.layout.padding.bottom + node.layout.border.bottom;
    if height_follows_content(document, node_id) {
        return Some(edges);
    }
    let overflow = flow_content_overflow(document, cascade, body_id, node_id)?;
    let rest = edges - overflow.max(0.0);
    (overflow >= -CONTENT_END_TOLERANCE && rest > 0.0).then_some(rest)
}

/// Whether the block `node_id` passes the closing edges of the blocks it ends
/// on to its own last box: its height is that of its content, or its
/// `min-height` and `max-height` leave it the height of its content all the
/// same, short of both. A set `height` ends the box where it ends whatever
/// its content does, and so does a size constraint that its content reaches,
/// so the edges below it follow its own border box instead, as in Chromium.
fn passes_closing_edges(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> bool {
    if height_follows_content(document, node_id) {
        return true;
    }
    let height = document.nodes[node_id].layout.size.height;
    let (min, max) = border_box_height_bounds(document, node_id);
    document.block_axis(node_id).height().is_auto()
        && height > min + 0.01
        && height < max - 0.01
        && flow_content_overflow(document, cascade, body_id, node_id)
            .is_some_and(|overflow| overflow.abs() <= CONTENT_END_TOLERANCE)
}

/// How far the flow content of the block `node_id` ends below the end of its
/// content box, or above it when negative, at normal-flow positions: the end
/// of its lines and the block children among them for a paragraph, and the
/// end of the margin box of its last child in the flow otherwise, or the top
/// of the content box when it has none. `None` when size containment ignores
/// the content, or a `display: contents` child ends the flow, whose boxes are
/// not measured here.
fn flow_content_overflow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    let node = &document.nodes[node_id];
    if node.size_containment.height {
        return None;
    }
    let layout = &node.layout;
    let content_top = layout.border.top + layout.padding.top;
    let content_end = layout.size.height - layout.padding.bottom - layout.border.bottom;
    let flow_end = if node.is_ifc_root() {
        content_top
            + document
                .ifc_layout_node(node_id)?
                .ifc
                .as_ref()?
                .lines
                .as_ref()?
                .shifted_height()
    } else {
        match last_flow_child(document, cascade, node_id) {
            None => content_top,
            Some(child) if document.nodes[child].display() == DisplayValue::Contents => {
                return None;
            }
            Some(child) => {
                let child_layout = &document.nodes[child].layout;
                child_layout.location.y
                    + child_layout.size.height
                    + end_margin_through(document, cascade, child)
                    - layout_relative_offset_y(document, cascade, body_id, child)
            }
        }
    };
    Some(flow_end - content_end)
}

/// The bottom margin that ends the margin box of the box `node_id` in the
/// flow: its own, collapsed with the margins of the boxes inside it that
/// adjoin its bottom (see [`adjoining_margin_extremes`]).
fn end_margin_through(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> f32 {
    if let Some(below) = margin_below_split_box(document, cascade, node_id) {
        return below;
    }
    let (positive, negative) = adjoining_margin_extremes(document, cascade, node_id, false, None);
    positive + negative
}

/// The part of the collapsed margin around `node_id` that lies below its
/// border box, when `node_id` is an empty box of set height whose margins
/// collapse through it (see [`collapses_as_empty_box`]). The margins of the
/// boxes inside it collapse through its top, so the box sits below them and
/// only what its own bottom margin adds to the margin they collapse into
/// with the margin above it lies below the box (CSS 2 §8.3.1). `None` for
/// any other box.
fn margin_below_split_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<f32> {
    if !splits_margin(document, cascade, node_id) {
        return None;
    }
    let above = margin_before_box(document, cascade, node_id);
    let resolve = |(positive, negative): (f32, f32)| positive + negative;
    let top = join_margin_extremes(
        above,
        adjoining_margin_extremes(document, cascade, node_id, true, None),
    );
    let all = join_margin_extremes(above, empty_box_margin_extremes(document, cascade, node_id));
    Some(resolve(all) - resolve(top))
}

/// Whether `node_id` is an empty box of set height whose margins collapse
/// through it (see [`collapses_as_empty_box`]) with a margin other than zero
/// inside it, which collapses through its top (see
/// [`margin_below_split_box`]). Chromium takes such a box to the next page
/// once the margins above it reach past a page edge: the edge truncates them,
/// but the box keeps its bottom margin whole below it. An empty box with no
/// such margin inside it is truncated with the margins around it instead.
fn splits_margin(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    if document.block_axis(node_id).height().is_auto()
        || !collapses_as_empty_box(document, cascade, node_id)
    {
        return false;
    }
    let mut boxes = Vec::new();
    for &child in &document.nodes[node_id].children {
        push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
    }
    boxes
        .into_iter()
        .any(|id| non_zero_margin(empty_box_margin_extremes(document, cascade, id)))
}

/// Whether `node_id` splits the margin (see [`splits_margin`]) and a
/// positive margin collapses into its top: one of its own or of the boxes
/// inside it, or the margin before it (see [`margin_before_box`]). A page
/// edge right above the box then falls in that margin. The box is checked
/// first, so that ordinary boxes at a page top skip the walk over the
/// siblings before them.
pub(super) fn margin_above_page_edge(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    splits_margin(document, cascade, node_id)
        && (margin_before_box(document, cascade, node_id).0 > 0.0
            || adjoining_margin_extremes(document, cascade, node_id, true, None).0 > 0.0)
}

/// How much further the flow after `node_id` moves when a page edge takes
/// the box to the top of a page, for a box that [`splits_margin`]: the box
/// then keeps its own bottom margin whole below it, where the layout left
/// only what that margin added to the margins above the box.
pub(super) fn split_box_kept_margin(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> f32 {
    if !splits_margin(document, cascade, node_id) {
        return 0.0;
    }
    let below = margin_below_split_box(document, cascade, node_id).unwrap_or(0.0);
    document.nodes[node_id].layout.margin.bottom - below
}

/// The box that splits the margin between `previous` and the box after it
/// in the flow (see [`splits_margin`]), whose siblings before it are
/// `preceding_siblings`, with the extremes of the margins below its border
/// box up to that box: its own bottom margin, and those of the empty boxes
/// after it and of the boxes it ends. The split box is among the empty
/// siblings before the box, or ends the flow of `previous`, or is
/// `previous`.
fn margin_split_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    previous: usize,
    preceding_siblings: &[usize],
) -> Option<(usize, (f32, f32))> {
    let own_bottom = |id: usize| {
        let margin = document.nodes[id].layout.margin.bottom;
        (margin.max(0.0), margin.min(0.0))
    };
    let mut below = (0.0_f32, 0.0_f32);
    let mut current = previous;
    // The boxes before `node_id`, last first, with those that `display:
    // contents` elements lend to the flow.
    let mut boxes = Vec::new();
    'before: for &sibling in preceding_siblings.iter().rev() {
        boxes.clear();
        push_in_flow_boxes_rev(document, cascade, sibling, &mut boxes);
        for &id in &boxes {
            // Text, preserved white space among it, forms a line between
            // the two boxes and keeps their margins apart.
            if document.nodes[id].kind() != NodeKind::Element {
                return None;
            }
            if id != previous && splits_margin(document, cascade, id) {
                return Some((id, join_margin_extremes(below, own_bottom(id))));
            }
            if id == previous || !collapses_as_empty_box(document, cascade, id) {
                current = id;
                break 'before;
            }
            below = join_margin_extremes(below, empty_box_margin_extremes(document, cascade, id));
        }
    }
    loop {
        if splits_margin(document, cascade, current) {
            return Some((current, join_margin_extremes(below, own_bottom(current))));
        }
        below = join_margin_extremes(below, own_bottom(current));
        let layout = &document.nodes[current].layout;
        if layout.padding.bottom + layout.border.bottom > 0.0
            || !document.block_axis(current).height().is_auto()
            || !margins_may_collapse_through(document, cascade, current)
        {
            return None;
        }
        let mut boxes = Vec::new();
        for &child in document.nodes[current].children.iter().rev() {
            push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
        }
        let mut next = None;
        for child in boxes {
            if document.nodes[child].kind() != NodeKind::Element
                || document.nodes[child].display().is_inline_level()
            {
                return None;
            }
            if splits_margin(document, cascade, child)
                || !collapses_as_empty_box(document, cascade, child)
            {
                next = Some(child);
                break;
            }
            below =
                join_margin_extremes(below, empty_box_margin_extremes(document, cascade, child));
        }
        current = next?;
    }
}

/// The extremes of the margin that ends the flow before `node_id`: the
/// bottom margin of the box before it, collapsed with those of the empty
/// boxes between the two (see [`empty_box_margin_extremes`]). A `display:
/// contents` element lends its boxes to the flow of its parent, so the walk
/// goes through those around and before `node_id`. Zero when no box comes
/// before it.
fn margin_before_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> (f32, f32) {
    let mut extremes = (0.0_f32, 0.0_f32);
    let mut current = node_id;
    while let Some(parent) = document.parent_of(current) {
        for &sibling in document.nodes[parent]
            .children
            .iter()
            .rev()
            .skip_while(|&&child| child != current)
            .skip(1)
        {
            if join_margin_before(document, cascade, sibling, &mut extremes) {
                return extremes;
            }
        }
        if document.nodes[parent].display() != DisplayValue::Contents {
            break;
        }
        current = parent;
    }
    extremes
}

/// The extremes of two sets of margins that collapse into one.
fn join_margin_extremes(
    (positive, negative): (f32, f32),
    (other_positive, other_negative): (f32, f32),
) -> (f32, f32) {
    (positive.max(other_positive), negative.min(other_negative))
}

/// Joins the margins that `node_id`, met walking back from a box, adds to
/// the margin above that box (see [`margin_before_box`]). Returns whether
/// the walk ends there: at text that forms a line, preserved white space
/// among it, which keeps the margins apart, or at a box that is not empty.
fn join_margin_before(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    extremes: &mut (f32, f32),
) -> bool {
    match sibling_adjacency::flow_box(document, cascade, node_id) {
        sibling_adjacency::FlowBox::Skip | sibling_adjacency::FlowBox::Phantom => {
            return false;
        }
        sibling_adjacency::FlowBox::Contents => {
            return document.nodes[node_id]
                .children
                .iter()
                .rev()
                .any(|&child| join_margin_before(document, cascade, child, extremes));
        }
        sibling_adjacency::FlowBox::Box if document.nodes[node_id].kind() != NodeKind::Element => {
            return true;
        }
        sibling_adjacency::FlowBox::Box => {}
    }
    let empty = collapses_as_empty_box(document, cascade, node_id);
    let margins = if empty {
        empty_box_margin_extremes(document, cascade, node_id)
    } else {
        adjoining_margin_extremes(document, cascade, node_id, false, None)
    };
    *extremes = join_margin_extremes(*extremes, margins);
    !empty
}

/// The last child of the block `node_id` that takes part in its flow: an
/// element that is neither floated nor absolutely positioned, which may be a
/// `display: contents` one.
fn last_flow_child(document: &Document, cascade: &CascadeResult, node_id: usize) -> Option<usize> {
    document.nodes[node_id]
        .children
        .iter()
        .rev()
        .copied()
        .find(|&child| takes_part_in_flow(document, cascade, child))
}

/// Whether `node_id` is an element that takes part in the flow of its
/// parent: neither floated nor absolutely positioned.
fn takes_part_in_flow(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    node.is_in_document()
        && !node.is_display_none()
        && node.kind() == NodeKind::Element
        && matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
        )
        && !super::page::is_floating_box_for_pagination(document, cascade, node_id)
}

/// The box before the empty boxes that end at `node_id`, and how far above
/// the top of the first of them it ends: empty boxes that establish no
/// formatting context of their own let the margins around them collapse
/// through.
fn before_empty_boxes(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> (usize, f32) {
    // A relative offset paints a box elsewhere but leaves the flow be.
    let flow_top = |id: usize| {
        current_abs_y(document, id) - layout_relative_offset_y(document, cascade, body_id, id)
    };
    let mut current = node_id;
    let mut lift = 0.0_f32;
    loop {
        let Some(parent) = document.parent_of(current).filter(|_| {
            let height = document.nodes[current].layout.size.height;
            height.abs() <= 0.001 && collapses_through(document, cascade, current, 0.0)
        }) else {
            return (current, lift);
        };
        let siblings = &document.nodes[parent].children;
        let Some(before) = siblings
            .iter()
            .position(|&child| child == current)
            .and_then(|index| {
                siblings[..index]
                    .iter()
                    .rev()
                    .copied()
                    .find(|&child| takes_part_in_flow(document, cascade, child))
            })
        else {
            return (current, lift);
        };
        let gap = flow_top(current) - flow_top(before) - document.nodes[before].layout.size.height;
        if !gap.is_finite() || gap < -0.001 {
            return (current, lift);
        }
        lift += gap;
        current = before;
    }
}

/// Whether the paragraph `node_id` is a block container in one column whose
/// content ends with a block child of its own: the block comes after all of
/// its lines, and no other box that takes part in the flow, such as a float
/// or a generated block, comes after it. An absolutely positioned box after
/// it does not count.
fn ends_with_block_child(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    node.kind() == NodeKind::Element
        && matches!(
            node.display(),
            DisplayValue::Block
                | DisplayValue::FlowRoot
                | DisplayValue::ListItem
                | DisplayValue::TableCaption
        )
        && node.multicol.is_none()
        && document
            .ifc_layout_node(node_id)
            .and_then(|node| node.ifc.as_ref())
            .filter(|ifc| ifc.multicol_fragments.is_none())
            .is_some_and(|ifc| {
                let Some(lines) = ifc.lines.as_ref() else {
                    return false;
                };
                let last = ifc
                    .boxes
                    .iter()
                    .rev()
                    .find(|b| b.kind != IfcBoxKind::OutOfFlow);
                last.is_some_and(|last| {
                    last.kind == IfcBoxKind::Block
                        && document.parent_of(last.node) == Some(node_id)
                        && lines.generated_block_starts.is_empty()
                        && lines
                            .block_line_starts
                            .last()
                            .is_some_and(|&(block, after)| {
                                block == last.node && after >= lines.lines.len()
                            })
                })
            })
}

/// Whether the paragraph `block_id` lays out a block after its last line: a
/// block child or a block-level `::before` or `::after` that no line follows,
/// even an empty one such as a clearfix, which Chromium breaks before.
fn block_after_last_line(document: &Document, block_id: usize) -> bool {
    document
        .ifc_layout_node(block_id)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .is_some_and(|lines| {
            let count = lines.lines.len();
            lines
                .block_line_starts
                .iter()
                .chain(&lines.generated_block_starts)
                .any(|&(_, after)| after >= count)
        })
}

/// The block size of the bottom padding and border of the block `node_id`
/// when they follow its flow content: it is a block container or a paragraph
/// whose height is that of its content, or whose set `height`, `min-height`
/// or `max-height` leaves room below its content (see
/// [`content_ends_above_edges`]).
fn content_bottom_edges(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    let node = &document.nodes[node_id];
    let edges = node.layout.padding.bottom + node.layout.border.bottom;
    (node.kind() == NodeKind::Element
        && !node.used_writing_mode.is_vertical()
        && node.multicol.is_none()
        // A set height keeps the place of a closing line in the block.
        && (if edges > 0.0 {
            node.is_ifc_root() || moves_later_siblings(document, node_id)
        } else {
            node.display() != DisplayValue::Contents && document.block_axis(node_id).height().is_auto()
        })
        && (height_follows_content(document, node_id)
            || min_height_left_behind(document, node_id)
            || content_ends_above_edges(document, cascade, body_id, node_id)
            || ends_with_collapsed_empty_box(document, cascade, node_id))
        && (edges > 0.0 || closes_forced_block_with_line(document, cascade, node_id)))
    .then_some(edges)
}

/// Whether the paragraph `root` closes a block among its lines that forces a
/// page break after itself with a line of its own (see
/// [`line_follows_paragraph_block`]). That line starts the page the break
/// opens, empty as it may be, so the paragraph ends there like one whose
/// bottom padding and border the break carries over: a break forced after
/// the paragraph or before the box after it opens one more page (Chromium).
fn closes_forced_block_with_line(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
) -> bool {
    // Only an inline box, or inline content after a block, closes it with a
    // line: a paragraph of blocks alone has none.
    document.nodes[root].children.iter().any(|&child| {
        matches!(
            document.nodes[child].display(),
            DisplayValue::Inline | DisplayValue::Contents
        )
    }) && document.preorder(root).skip(1).any(|id| {
        forces_page_break(document, cascade, id, false)
            && line_follows_paragraph_block(document, cascade, root, id)
    })
}

/// Whether the flow of the block `node_id`, whose height nothing caps, ends
/// with an empty box that stays with the box before it (see
/// [`collapsed_empty_box`]). That box does
/// not move on with the edges below it, so the page breaks between the two
/// instead, even where the block's `min-height` ends at its content.
fn ends_with_collapsed_empty_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    !document.nodes[node_id].is_ifc_root()
        && !height_is_capped(document, node_id)
        && last_flow_box(document, cascade, node_id)
            .is_some_and(|last| collapsed_empty_box(document, cascade, last))
}

/// Whether the block `node_id` of auto `height` and `max-height` has grown
/// to or past its `min-height` with its content, so that its height follows
/// that content as if it had none.
fn min_height_left_behind(document: &super::context::LayoutSession<'_>, node_id: usize) -> bool {
    let block_axis = document.block_axis(node_id);
    let (min_height, _) = border_box_height_bounds(document, node_id);
    block_axis.height().is_auto()
        && block_axis.max_height().is_auto()
        && !document.nodes[node_id].size_containment.height
        && document.nodes[node_id].layout.size.height > min_height - 0.001
}

/// The block size of the margin between the last box in the flow of the
/// block `node_id` and its bottom padding and border: the bottom margin of
/// that box, collapsed with those of the last boxes in its own flow that do
/// not close with edges of their own. A page edge inside it truncates it
/// (CSS Fragmentation 3 §5.2), and the edges begin the next page. Zero for a
/// paragraph, whose lines end its content, and for a block of set `height`.
/// A block that `max-height` holds has no edges that break apart from its
/// content (see [`content_bottom_edges`]). Paired with the block size of the
/// top of that margin that a float the block holds keeps, and with how far
/// below the end of the content the last of the empty boxes that end the
/// flow lies, when a negative margin below it ends the content above it
/// (see [`empty_box_offset`]): Chromium moves that box to the next page
/// once a page edge falls above it, and the edges begin that page too.
fn margin_above_edges(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> (f32, f32, f32) {
    let node = &document.nodes[node_id];
    if node.is_ifc_root()
        || !document.block_axis(node_id).height().is_auto()
        || node.size_containment.height
        || in_flex_or_grid(document, node_id, body_id)
    {
        return (0.0, 0.0, 0.0);
    }
    let layout = &node.layout;
    let Some(last) = last_flow_box(document, cascade, node_id) else {
        return (0.0, 0.0, 0.0);
    };
    // Empty boxes at the end that stay with the box before them leave the
    // margins that collapse through them below that box, where a page edge
    // truncates them as one (see `flow_end_box`).
    let empty_last = last;
    let (last, end_margin) = if collapsed_empty_box(document, cascade, last) {
        match flow_end_box(document, cascade, last) {
            Some((end, (positive, _))) => (end, Some(positive)),
            None => return (0.0, 0.0, 0.0),
        }
    } else {
        (last, None)
    };
    // Where the last of those empty boxes lies below the box before them.
    let empty_offset = end_margin
        .is_some()
        .then(|| empty_box_offset(document, cascade, last, empty_last));
    let content_end = layout.size.height - layout.padding.bottom - layout.border.bottom;
    // A box inside a `display: contents` child is laid out by `node_id`, so
    // the box is located from the origins of the two.
    let last_bottom = current_abs_y(document, last) - current_abs_y(document, node_id)
        + document.nodes[last].layout.size.height
        - layout_relative_offset_y(document, cascade, body_id, last);
    // The space below the last box holds no more than the largest margin
    // that collapses there.
    let mut margin = end_margin.unwrap_or(0.0);
    let mut current = end_margin.is_none().then_some(last);
    while let Some(id) = current {
        let child = &document.nodes[id];
        // The margins of an empty box that a set height makes empty collapse
        // through it, and Chromium keeps them whole below the box before it.
        if child.layout.size.height <= 0.0 {
            return (0.0, 0.0, 0.0);
        }
        margin = margin.max(child.layout.margin.bottom);
        if child.layout.padding.bottom + child.layout.border.bottom > 0.0
            || child.is_ifc_root()
            || !height_follows_content(document, id)
        {
            break;
        }
        current = last_flow_box(document, cascade, id);
    }
    // Content below the margin, such as a float that the block holds, or a
    // used height of its own, leaves no margin above the edges. Room that
    // `min-height` leaves below it does: the margin reaches the edges once
    // moved content outgrows that room, and a cut then takes no more than
    // the block's height above its `min-height` (see
    // `PaginationState::truncate_margin_above_edges`).
    let signed_gap = content_end - last_bottom;
    let gap = signed_gap.max(0.0);
    // A negative margin below the last of those empty boxes may end the
    // content above it: the box still lies that far below the end.
    let reach = empty_offset.map_or(0.0, |offset| (offset - signed_gap).max(0.0));
    let min_height_room = border_box_height_bounds(document, node_id).0 > 0.0
        && border_box_height_bounds(document, node_id).0 >= layout.size.height - 0.001;
    if gap > margin + 0.001 && (!min_height_room || holds_floats(document, cascade, node_id)) {
        return (0.0, 0.0, 0.0);
    }
    let margin_above = gap.min(margin);
    (
        margin_above,
        margin_kept_by_floats(document, cascade, body_id, node_id, last, margin_above),
        reach,
    )
}

/// How far below the box `end` the empty box `empty` lies, that ends the
/// flow after it with the empty boxes between the two: the position of an
/// empty box that the margins collapse through is where its top margin
/// ends (CSS 2 §8.3.1), the margins of `end`, of those between and the
/// margins that adjoin its own top collapsed into one.
fn empty_box_offset(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    end: usize,
    empty: usize,
) -> f32 {
    let (top_positive, top_negative) =
        adjoining_margin_extremes(document, cascade, empty, true, None);
    let (mut positive, mut negative) =
        adjoining_margin_extremes(document, cascade, end, false, None);
    positive = positive.max(top_positive);
    negative = negative.min(top_negative);
    let siblings = document
        .parent_of(empty)
        .map_or(&[][..], |parent| &document.nodes[parent].children[..]);
    for &child in siblings
        .iter()
        .rev()
        .skip_while(|&&child| child != empty)
        .skip(1)
        .take_while(|&&child| child != end)
    {
        if takes_part_in_flow(document, cascade, child) {
            let (child_positive, child_negative) =
                empty_box_margin_extremes(document, cascade, child);
            positive = positive.max(child_positive);
            negative = negative.min(child_negative);
        }
    }
    positive + negative
}

/// The block size of the top of the margin above the edges of the block
/// `node_id`, `margin_above` tall below its last in-flow box `last`, that
/// the floats the block holds keep from a cut, where they lie now. A float
/// that a block formatting context grows to hold keeps the content box down
/// to its bottom. The floats in a box that does not hold them stand out of
/// it and keep nothing.
pub(super) fn margin_kept_by_floats(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    last: usize,
    margin_above: f32,
) -> f32 {
    if !holds_floats(document, cascade, node_id) {
        return 0.0;
    }
    let top = current_abs_y(document, node_id);
    let last_bottom = current_abs_y(document, last) - top + document.nodes[last].layout.size.height
        - layout_relative_offset_y(document, cascade, body_id, last);
    own_floats(document, cascade, node_id)
        .into_iter()
        .map(|float| {
            // Relative offsets of the float and of the boxes between it and
            // the block paint it elsewhere but leave the block's height be.
            let offset: f32 = document
                .inclusive_ancestors(float)
                .take_while(|&id| id != node_id)
                .map(|id| layout_relative_offset_y(document, cascade, body_id, id))
                .sum();
            float_extent(document, float).1 - offset - top - last_bottom
        })
        .fold(0.0_f32, f32::max)
        .min(margin_above)
}

/// The floats that the block formatting context of `node_id` holds: those
/// in its flow, outside the boxes in it that hold their own floats or leave
/// the flow.
fn own_floats(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> Vec<usize> {
    let mut floats = Vec::new();
    let mut pending: Vec<usize> = document.nodes[node_id].children.clone();
    while let Some(id) = pending.pop() {
        let node = &document.nodes[id];
        if node.kind() != NodeKind::Element || node.is_display_none() {
            continue;
        }
        if is_floating_box_for_pagination(document, cascade, id) {
            floats.push(id);
            continue;
        }
        if !matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
        ) || node.display() != DisplayValue::Contents && holds_floats(document, cascade, id)
        {
            continue;
        }
        pending.extend(node.children.iter().copied());
    }
    floats
}

/// The last box in the flow of the block `node_id`: its last flow child (see
/// [`last_flow_child`]), or the last box inside it when that child is a
/// `display: contents` element without a box of its own.
pub(super) fn last_flow_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<usize> {
    let mut last = last_flow_child(document, cascade, node_id)?;
    while document.nodes[last].display() == DisplayValue::Contents {
        last = last_flow_child(document, cascade, last)?;
    }
    Some(last)
}

/// Where the box `previous`, which ends at `bottom`, ends its block-end
/// padding and borders, when a page edge falls inside them or, where it has
/// none, inside those of a box that ends with it: the last in-flow box of a
/// block that stacks its boxes in block flow, or any item of a flex or grid
/// container that ends its content, when the height of the outer box follows
/// them and no margin separates the two, and so on down. Edges that end the
/// outer box after a split inside it begin on the page, so they keep the
/// margin after them, as does an edge between the edges of two boxes. Empty
/// boxes at the end of the flow are passed over to the box before them.
fn split_edges_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    page_origins: &PageOrigins,
    previous: usize,
    bottom: f32,
) -> Option<f32> {
    let body_id = super::page::find_body(document).unwrap_or(document.root);
    let (current, lift) = before_empty_boxes(document, cascade, body_id, previous);
    let split_end = bottom - lift;
    // Far down a long flow the edges are only as exact as `f32` keeps them.
    let tolerance = 0.001_f32.max(bottom.abs() * 4.0 * f32::EPSILON);
    let ends = SplitEdgesEnd {
        document,
        cascade,
        page_origins,
        body_id,
        tolerance,
    };
    ends.split(current, split_end).then_some(split_end)
}

/// What [`split_edges_end`] walks the boxes that end a box with.
struct SplitEdgesEnd<'a, 'b> {
    document: &'a super::context::LayoutSession<'b>,
    cascade: &'a CascadeResult,
    page_origins: &'a PageOrigins,
    body_id: usize,
    tolerance: f32,
}

impl SplitEdgesEnd<'_, '_> {
    /// Whether a page edge falls inside the block-end padding and borders of
    /// `current`, which ends at `end`, or, when `current` has none, inside
    /// those of a box that ends with it. A block that stacks its boxes in block flow ends with its last
    /// in-flow box. A flex or grid container may end with any of its items,
    /// side by side or not: the one that reaches the end of its content.
    fn split(&self, current: usize, end: f32) -> bool {
        let document = self.document;
        let node = &document.nodes[current];
        let own = node.layout.padding.bottom + node.layout.border.bottom;
        // Edges that a page edge does not split begin on the page they end
        // on, after any split inside: they are content of that page.
        if own > self.tolerance {
            let edge = self
                .page_origins
                .origin(self.page_origins.page_index_for_end(end));
            return edge > end - own + self.tolerance && edge < end - self.tolerance;
        }
        if node.is_ifc_root()
            || node.multicol.is_some()
            || node.used_writing_mode.is_vertical()
            || !(height_follows_content(document, current)
                || min_height_left_behind(document, current))
        {
            return false;
        }
        let end = end - own;
        if is_flex_or_grid(document, current) {
            let mut items = Vec::new();
            push_flow_items(document, self.cascade, current, &mut items);
            return items
                .into_iter()
                .any(|item| self.ends_content(current, item) && self.split(item, end));
        }
        last_flow_box(document, self.cascade, current)
            .is_some_and(|last| self.ends_content(current, last) && self.split(last, end))
    }

    /// Whether `child`, with no margin below it, ends where the content of
    /// `current` does: nothing else, such as a float `current` holds or a
    /// longer item beside it, reaches lower.
    fn ends_content(&self, current: usize, child: usize) -> bool {
        let document = self.document;
        let node = &document.nodes[current];
        let content_end =
            node.layout.size.height - node.layout.padding.bottom - node.layout.border.bottom;
        // A relative offset paints a box elsewhere but leaves the flow be.
        let child_bottom = current_abs_y(document, child)
            - layout_relative_offset_y(document, self.cascade, self.body_id, child)
            - current_abs_y(document, current)
            + document.nodes[child].layout.size.height;
        document.nodes[child].layout.margin.bottom.abs() <= self.tolerance
            && (content_end - child_bottom).abs() <= self.tolerance
    }
}

/// The in-flow boxes that `node_id` lays out, in order: its in-flow children,
/// with those of a child that generates no box of its own in its place.
fn push_flow_items(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    items: &mut Vec<usize>,
) {
    for &child in &document.nodes[node_id].children {
        if !takes_part_in_flow(document, cascade, child) {
            continue;
        }
        if document.nodes[child].display() == DisplayValue::Contents {
            push_flow_items(document, cascade, child, items);
        } else {
            items.push(child);
        }
    }
}

/// For each block below `body_id` whose height follows its content, the
/// block size of the margins inside it that a page edge may truncate (CSS
/// Fragmentation 3 §5.2): those between its in-flow children, the margin
/// above its edges (see [`margin_above_edges`]), and those inside the blocks
/// in it whose heights follow their content too. A truncated margin reaches
/// no page of its own: the content and the edges around it reach the pages
/// the block runs on to, and no more than its min-height leaves. Flex and
/// grid items keep their margins whole.
fn truncatable_margin_rooms(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    edges_breaks: &[candidates::EdgesBreak],
) -> HashMap<usize, f32> {
    let mut rooms: HashMap<usize, f32> = edges_breaks
        .iter()
        .map(|found| (found.node_id, found.margin_above - found.margin_kept))
        .filter(|&(_, room)| room > 0.0)
        .collect();
    let order: Vec<_> = document.preorder(body_id).collect();
    // Whether each box lies in a flex or grid container, found top down.
    let mut in_items = vec![false; document.nodes.len()];
    for &node_id in &order {
        in_items[node_id] = document
            .parent_of(node_id)
            .is_some_and(|parent| in_items[parent] || is_flex_or_grid(document, parent));
    }
    for &node_id in order.iter().rev() {
        let follows = node_id != body_id
            && !in_items[node_id]
            && (height_follows_content(document, node_id)
                || min_height_left_behind(document, node_id));
        if !follows {
            rooms.remove(&node_id);
            continue;
        }
        let gaps = margins_between_children(document, cascade, body_id, node_id);
        // The block keeps its min-height whatever the cuts inside it take.
        let (min_height, _) = border_box_height_bounds(document, node_id);
        let room = (rooms.get(&node_id).copied().unwrap_or(0.0) + gaps)
            .min(document.nodes[node_id].layout.size.height - min_height);
        if room <= 0.0 {
            rooms.remove(&node_id);
            continue;
        }
        rooms.insert(node_id, room);
        if let Some(parent) = document.parent_of(node_id) {
            *rooms.entry(parent).or_default() += room;
        }
    }
    rooms
}

/// The block size of the margins between the in-flow children of the block
/// container `node_id`: between the border boxes of each child and the one
/// after it, no more than the larger of the margins that meet there.
fn margins_between_children(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> f32 {
    let node = &document.nodes[node_id];
    if node.kind() != NodeKind::Element
        || node.formatting_context != FormattingContext::Block
        || !moves_later_siblings(document, node_id)
    {
        return 0.0;
    }
    let mut total = 0.0;
    let mut previous: Option<(f32, f32)> = None;
    for &child in &node.children {
        let child_node = &document.nodes[child];
        if !child_node.is_in_document()
            || child_node.is_display_none()
            || child_node.kind() != NodeKind::Element
            || child_node.display() == DisplayValue::Contents
            || !matches!(
                child_node.position(),
                PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
            )
            || super::page::is_floating_box_for_pagination(document, cascade, child)
        {
            continue;
        }
        let layout = &child_node.layout;
        let top = layout.location.y - layout_relative_offset_y(document, cascade, body_id, child);
        if let Some((bottom, margin_bottom)) = previous {
            let margin = margin_bottom.max(layout.margin.top);
            total += (top - bottom).min(margin).max(0.0);
        }
        previous = Some((top + layout.size.height, layout.margin.bottom));
    }
    total
}

/// Whether the block `node_id` lies in an item of a flex or grid container,
/// the body among them, whose fragmentation keeps the margins of the item's
/// content.
fn in_flex_or_grid(
    document: &super::context::LayoutSession<'_>,
    node_id: usize,
    body_id: usize,
) -> bool {
    for id in document.ancestors(node_id) {
        if is_flex_or_grid(document, id) {
            return true;
        }
        if id == body_id {
            break;
        }
    }
    false
}

/// Whether `node_id` is a flex or grid container.
fn is_flex_or_grid(document: &super::context::LayoutSession<'_>, node_id: usize) -> bool {
    matches!(
        document.nodes[node_id].formatting_context,
        FormattingContext::Flex | FormattingContext::Grid
    )
}

/// Whether the block `node_id`, which a set `height`, `min-height` or
/// `max-height` sizes, ends its content box below the end of its flow
/// content (see [`flow_content_overflow`]). The bottom padding and border
/// then close the box on their own and, as in Chromium, start the next page
/// whole when they do not fit below the content box on its page, its
/// fragment taking the rest of the page. A box whose content reaches its end
/// has the edges right below its last box instead, and moves that box along
/// (see [`edges_below_last_box`]).
fn content_ends_above_edges(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> bool {
    flow_content_overflow(document, cascade, body_id, node_id)
        .is_some_and(|overflow| overflow < -CONTENT_END_TOLERANCE)
}

/// Where the block `node_id` sits in its parent along the block axis at its
/// normal-flow position: its layout location, less the relative offset that
/// layout applied.
fn normal_flow_y(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> f32 {
    document.pagination_local_y(node_id)
        - layout_relative_offset_y(document, cascade, body_id, node_id)
}

/// Where the block `node_id` ends in its parent along the block axis at its
/// normal-flow position.
fn normal_flow_bottom(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> f32 {
    normal_flow_y(document, cascade, body_id, node_id) + document.nodes[node_id].layout.size.height
}

/// The block size at the end of the block `node_id` that follows its one box:
/// the bottom padding and border of `node_id` and of the blocks that box ends,
/// and the margin below the box where it does not collapse out of `node_id`.
/// `None` unless `node_id` holds one box in its flow, which holds one in turn,
/// down to a box that the page cannot break inside: one without a box in its
/// flow, a paragraph of one line (or of too few for its orphans and widows
/// where `ancestor_edges_follow`), a box that avoids breaks inside (unless a
/// forced break or a page name below it breaks the page there anyway), or a
/// flex container, grid or table that moves whole (see
/// [`container_moves_whole`]). A float within the block moves with it.
///
/// With no box before it in the block, a break that would leave this tail
/// alone on the next page falls before the block instead (CSS Break 3 §4.1
/// prefers a break between boxes to one between a box and the edges of its
/// parent), as browsers do. A paragraph of more lines breaks between them and
/// leaves the tail to its last lines, which is not this block's to move; but
/// when `ancestor_edges_follow`, the edges of the blocks around this one close
/// after it, a paragraph with too few lines for its orphans and widows cannot
/// be split without breaking them, and the block moves.
fn lone_box_tail(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    ancestor_edges_follow: bool,
) -> Option<f32> {
    lone_box_with_floats(
        document,
        cascade,
        body_id,
        node_id,
        ancestor_edges_follow,
        false,
    )
    .map(|lone| lone.tail)
}

/// A block that holds one box (see [`lone_box_tail`]).
#[derive(Clone, Copy)]
struct LoneBox {
    /// The block size at the end of the block that follows its box.
    tail: f32,
    /// How far below the box the floats beside it reach, or zero.
    float_reach: f32,
}

/// [`lone_box_tail`], along with how far the floats beside the box reach below
/// it. A float that reaches into the padding or border below the content, or
/// past the block, is accepted when `floats_may_reach` is set, so long as it
/// starts within the block: the page edge it reaches is for the caller to
/// weigh.
fn lone_box_with_floats(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    ancestor_edges_follow: bool,
    floats_may_reach: bool,
) -> Option<LoneBox> {
    let mut current = node_id;
    // Where the box ends, from the top of `node_id`.
    let mut content_end = 0.0;
    // The lowest margin-box bottom of the floats beside the box.
    let mut float_bottom = f32::NEG_INFINITY;
    loop {
        let node = &document.nodes[current];
        // Generated boxes are not among the children, and the columns of a
        // multicol container break on their own.
        if node.multicol.is_some() || node.has_before_or_after_content {
            return None;
        }
        // A box that avoids breaks inside moves whole, whatever it holds, but
        // for a forced break or a page name below it: the page breaks there
        // whichever page the box starts.
        if current != node_id
            && avoids_page_break_inside(cascade.computed[current].break_inside)
            && !has_mandatory_break_inside(document, cascade, current)
        {
            break;
        }
        if current != node_id && node.is_ifc_root() {
            let lines = document
                .ifc_layout_node(current)
                .and_then(|node| node.ifc.as_ref())
                .and_then(|ifc| ifc.lines.as_ref())?;
            // Lines too few to leave the orphans before a break and the
            // widows after it keep together, when the closing edges of the
            // blocks around the box are what the page breaks away from: the
            // break before the block that holds them is the one between
            // boxes, as browsers take it. Alone in a block with edges of its
            // own, such lines are split between them instead, as browsers
            // print it.
            let computed = &cascade.computed[current];
            let together = if ancestor_edges_follow {
                lines_keep_together(lines.lines.len(), computed.orphans, computed.widows)
            } else {
                lines.lines.len() <= 1
            };
            if !lines.block_line_starts.is_empty() || !together {
                return None;
            }
            break;
        }
        let mut boxes = Vec::new();
        for &child in &node.children {
            if is_floating_box_for_pagination(document, cascade, child) {
                // A float beside the box moves along with the block.
                let accepted = float_within_box(document, node_id, child)
                    || (floats_may_reach && float_starts_within_box(document, node_id, child));
                if !accepted {
                    return None;
                }
                // The blocks between the one that holds the box and the float
                // may be offset, while `content_end` is at normal-flow
                // positions.
                let offset_below = accumulated_relative_offset_y(document, cascade, body_id, child)
                    - accumulated_relative_offset_y(document, cascade, body_id, node_id);
                float_bottom = float_bottom.max(float_extent(document, child).1 - offset_below);
                continue;
            }
            push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
            if boxes.len() > 1 {
                return None;
            }
        }
        match boxes[..] {
            [] if current != node_id => break,
            [only]
                if block_flow_closing_edges(document, cascade, body_id, current).is_some()
                    && is_plain_block_box(document, only) =>
            {
                // The box sits at its normal-flow position, less the
                // relative offset that layout applied.
                content_end += normal_flow_y(document, cascade, body_id, only);
                current = only;
            }
            // A flex container, grid or table that moves whole is the box that
            // ends the chain. Named pages are assumed, as the walk does not
            // know whether the document has any: a column flex container
            // then moves whole only if its item holds nothing to break at.
            [only]
                if block_flow_closing_edges(document, cascade, body_id, current).is_some()
                    && container_moves_whole(document, cascade, only, true) =>
            {
                content_end += normal_flow_y(document, cascade, body_id, only);
                current = only;
                break;
            }
            _ => return None,
        }
    }
    content_end += document.nodes[current].layout.size.height;
    Some(LoneBox {
        tail: document.nodes[node_id].layout.size.height - content_end,
        float_reach: (float_bottom - (current_abs_y(document, node_id) + content_end)).max(0.0),
    })
}

/// Whether `node_id` is a block box that stacks in the normal flow of a
/// horizontal block container: not a float, positioned away from the flow, or
/// laid out in another writing mode.
fn is_plain_block_box(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    node.kind() == NodeKind::Element
        && matches!(
            node.display(),
            DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
        )
        && matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative
        )
        && !node.used_writing_mode.is_vertical()
}

/// Whether the page breaks inside the box `node_id` at no better place than
/// before it, even with padding or a border around the box it holds, or
/// around boxes that avoided breaks tie together: see [`breaks_only_around`]
/// and [`lone_box_tail`].
fn moves_whole_at_best(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    has_named_pages: bool,
) -> bool {
    breaks_only_around(document, cascade, node_id, has_named_pages)
        || lone_box_tail(document, cascade, body_id, node_id, false).is_some()
}

/// How the last box of a block is tied to the boxes before it by avoided page
/// breaks (`break-after` or `break-before` of `avoid`), in the children of the
/// block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AvoidChain {
    /// The last box is tied to the boxes from the child `first` on, and a
    /// break before `first` is not avoided: the page may break there, and no
    /// box in the run breaks better inside.
    Run { first: usize },
    /// The last box is tied to the boxes before it all the way to the first
    /// box of the block: no break between boxes is left to take.
    Start,
    /// The run of tied boxes starts at the paragraph `first`, which may break
    /// between its lines, and the boxes after it move whole at best: the
    /// page breaks between the paragraph's lines, so that its last lines go
    /// to the next page with the boxes after it and the edges.
    Lines { first: usize },
}

/// The avoided breaks that tie the last box of a block to the boxes before it.
/// `children` and `flow_flags` are the children of the block and, for each, its
/// last box in the flow and whether it floats, and `last` is the index of the
/// last child that has a box in the flow. `names_page` is `None` when the
/// document names no page, and otherwise tells whether a box or anything in it
/// has a named page. The result is `None` when no break before the last box is
/// avoided, or the boxes around it are not plain blocks in the flow, or a named
/// page may start between the boxes the avoided breaks tie: that boundary is
/// mandatory, so the boxes do not move together.
///
/// The edges of the block that fall on the next page alone are kept with its
/// last box by a break before that box (CSS Break 3 §4.1). When the break is
/// avoided, browsers break before the first box of the tied run instead, which
/// leaves it and the boxes after it on the next page, and when the run starts
/// the block they still break before the last box, the avoided break giving way.
fn avoid_chain_before_last_box(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    children: &[usize],
    flow_flags: &[(Option<usize>, bool)],
    last: usize,
    mut names_page: Option<impl FnMut(usize) -> bool>,
) -> Option<AvoidChain> {
    let has_named_pages = names_page.is_some();
    // A block box in the flow that takes block space. Text between the boxes
    // is left to the rules for it. A float lies in a flow of its own: it does
    // not cut the run, and moves along with the boxes it sits among.
    let is_box = |index: usize| {
        let id = children[index];
        let (in_flow, floated) = flow_flags[index];
        in_flow == Some(id)
            && !floated
            && (is_plain_block_box(document, id) || is_plain_container_box(document, id))
            && !takes_no_block_space(document, id)
    };
    if !is_box(last) {
        return None;
    }
    let mut first = last;
    let starts_block = loop {
        // A child that generates no box, such as collapsed white space, and
        // a float are not between the boxes.
        let Some(before) = (0..first)
            .rev()
            .find(|&index| flow_flags[index] != (None, false) && !flow_flags[index].1)
        else {
            break true;
        };
        // An empty block between the boxes ties them as a box does.
        let empty_block = |index: usize| {
            let id = children[index];
            flow_flags[index] == (Some(id), false)
                && is_plain_block_box(document, id)
                && takes_no_block_space(document, id)
        };
        if !is_box(before) && !empty_block(before) {
            return None;
        }
        let (forced_after, avoided_after) =
            page_edge_constraint(document, cascade, children[before], false);
        let (forced_before, avoided_before) =
            page_edge_constraint(document, cascade, children[first], true);
        if forced_after || forced_before || !(avoided_after || avoided_before) {
            break false;
        }
        // A named page on either side may start a new page between the
        // boxes, avoided break or not. Telling that from one name on both
        // sides is not tried: such boxes keep the placement they had before.
        let mut names = |index: usize| {
            names_page
                .as_mut()
                .is_some_and(|names_page| names_page(children[index]))
        };
        if names(before) || names(first) {
            return None;
        }
        first = before;
    };
    if first == last {
        return None;
    }
    // A break inside one of the boxes of the run at a better place than
    // before the run is taken instead.
    let boxes: Vec<usize> = (first..=last)
        .filter(|&index| flow_flags[index] != (None, false) && !flow_flags[index].1)
        .collect();
    // A paragraph with lines too few to break between for its `orphans` and
    // `widows` moves whole too.
    // A run that starts the block has no break before it to take: its
    // paragraph breaks between its lines all the same.
    let moves_whole = |index: usize| {
        let id = children[index];
        moves_whole_at_best(document, cascade, body_id, id, has_named_pages)
            || (!starts_block
                && run_paragraph(document, cascade, id).is_some_and(|paragraph| {
                    paragraph_lines_keep_together(document, cascade, paragraph)
                }))
    };
    let Some(split) = boxes.iter().rposition(|&index| !moves_whole(index)) else {
        return Some(if starts_block {
            AvoidChain::Start
        } else {
            AvoidChain::Run { first }
        });
    };
    // A box after the paragraph whose set height leaves room below its lines
    // breaks in that room: the paragraph keeps its lines then.
    if boxes[split + 1..]
        .iter()
        .any(|&index| room_below_lines(document, children[index]))
    {
        return None;
    }
    // The last paragraph of the run that breaks between its lines takes the
    // break instead, which leaves its last lines with the boxes after it. A
    // paragraph with lines too few to split would move whole without the
    // boxes tied before it, so then the run keeps its placement.
    let paragraph = children[boxes[split]];
    run_paragraph(document, cascade, paragraph)
        .is_some_and(|paragraph| {
            paragraph_may_split_lines(document, cascade, paragraph, split == 0)
        })
        .then_some(AvoidChain::Lines {
            first: boxes[split],
        })
}

/// Whether the paragraph `node_id` may break between its lines: it has lines,
/// does not avoid breaks inside it, has no set height that leaves room below
/// its lines (the page breaks in that room instead), and, unless `few_lines` allows it, has
/// lines enough for `orphans` and `widows` on both sides of a break.
fn paragraph_may_split_lines(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    few_lines: bool,
) -> bool {
    document.nodes[node_id].is_ifc_root()
        && !avoids_page_break_inside(cascade.computed[node_id].break_inside)
        && !room_below_lines(document, node_id)
        && document
            .ifc_layout_node(node_id)
            .and_then(|node| node.ifc.as_ref())
            .and_then(|ifc| ifc.lines.as_ref())
            .is_some_and(|lines| !lines.lines.is_empty())
        && (few_lines || !paragraph_lines_keep_together(document, cascade, node_id))
}

/// The paragraph that a run of tied boxes ends with in the box `node_id`: the
/// box itself, or the paragraph inside plain blocks that hold nothing else in
/// their flow, which breaks as the box does. `None` when there is no such
/// paragraph.
fn run_paragraph(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<usize> {
    let mut current = node_id;
    loop {
        let node = &document.nodes[current];
        if node.is_ifc_root() {
            return Some(current);
        }
        if !is_plain_block_box(document, current) {
            return None;
        }
        let mut boxes = pagination_child_order(document, cascade, current)
            .into_iter()
            .filter(|&child| {
                let child_node = &document.nodes[child];
                child_node.kind() == NodeKind::Element
                    && !child_node.is_display_none()
                    && !is_floating_box_for_pagination(document, cascade, child)
                    && !matches!(
                        child_node.position(),
                        PositionValue::Absolute | PositionValue::Fixed
                    )
            });
        let (Some(only), None) = (boxes.next(), boxes.next()) else {
            return None;
        };
        current = only;
    }
}

/// Whether the paragraph `node_id` has two lines or more to break between for
/// the edges that close the blocks around it, whatever its `orphans`,
/// `widows` and `break-inside` ask. Such a break beats one before the
/// paragraph that an avoided break forbids. A paragraph whose set height
/// leaves room below its lines has the edges after that room instead.
fn paragraph_has_lines_to_split(
    document: &super::context::LayoutSession<'_>,
    node_id: usize,
) -> bool {
    document.nodes[node_id].is_ifc_root()
        && !room_below_lines(document, node_id)
        && document
            .ifc_layout_node(node_id)
            .and_then(|node| node.ifc.as_ref())
            .and_then(|ifc| ifc.lines.as_ref())
            .is_some_and(|lines| lines.lines.len() > 1 && lines.block_line_starts.is_empty())
}

/// Whether the paragraph `node_id` has a set height that leaves room below its
/// last line, where the page may break inside it.
fn room_below_lines(document: &super::context::LayoutSession<'_>, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    if !node.is_ifc_root() || height_follows_content(document, node_id) {
        return false;
    }
    let lines_end = document
        .ifc_layout_node(node_id)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .and_then(|lines| lines.lines.last())
        .map_or(0.0, |line| line.block_offset() + line.block_size());
    let layout = &node.layout;
    let content = layout.size.height
        - layout.padding.top
        - layout.padding.bottom
        - layout.border.top
        - layout.border.bottom;
    content > lines_end + 0.5
}

/// For each child of a block, how far below its border-box top the run of
/// boxes tied by avoided breaks ends that the child starts, when something in
/// the flow of the block lies before the child: the page may break before it,
/// and the run moves to the next page together when it would cross the edge.
/// `children` and `flow_flags` are as for [`avoid_chain_before_last_box`].
///
/// Only block boxes next to each other make a run: a float or text between
/// them ends it. A table joins it by its first row (see
/// [`ends_a_run_as_table`]) and starts the next one by its last part (see
/// [`last_row_break`]), or goes on with it when it moves whole. The run ends
/// where its last box holds the page (see [`avoided_run_head`]).
fn nested_run_units(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    children: &[usize],
    flow_flags: &[(Option<usize>, bool)],
    run_breaks: &mut HashMap<usize, RunUnit>,
) -> (Vec<Option<RunUnit>>, Vec<f32>) {
    // A table joins a run by its first row, which stays with the box before
    // it, and starts one by its last part, which stays with the box after it
    // (see `last_row_break`).
    let is_flow_box = |index: usize| {
        let id = children[index];
        let (in_flow, floated) = flow_flags[index];
        in_flow == Some(id) && !floated && !takes_no_block_space(document, id)
    };
    let is_box = |index: usize| is_flow_box(index) && is_plain_block_box(document, children[index]);
    // A flex container that lays its items out in rows ends a run too, with
    // the first piece of their content (see `held_step`).
    let is_run_end = |index: usize| {
        is_box(index)
            || (is_flow_box(index)
                && (ends_a_run_as_table(document, cascade, children[index])
                    || lays_out_in_flex_rows(document, cascade, children[index])))
    };
    let is_table = |index: usize| {
        is_flow_box(index) && ends_a_run_as_table(document, cascade, children[index])
    };
    // Whether the box at `index` starts a run with the box after it, and
    // whether it is a table that breaks inside it before the run.
    let starts_run = |index: usize| {
        is_box(index) || (is_table(index) && table_starts_run(document, cascade, children[index]))
    };
    let breaking_table = |index: usize| {
        is_table(index) && run_break_start(document, cascade, children[index]).is_some()
    };
    let first_in_flow = flow_flags
        .iter()
        .position(|&(in_flow, _)| in_flow.is_some());
    let mut units = vec![None; children.len()];
    let mut tails = vec![0.0; children.len()];
    let mut finish = |run: Option<(usize, usize)>| {
        let Some((start, last)) = run else {
            return;
        };
        let (start_box, last_box) = (children[start], children[last]);
        let last_height = document.nodes[last_box].layout.size.height;
        let held = avoided_run_head(document, cascade, body_id, last_box, true);
        let margin_bottom = document.nodes[last_box].layout.margin.bottom.max(0.0);
        let head = match held.end {
            Some(head) if head < last_height - 0.001 => head,
            Some(head) => head.max(last_height + margin_bottom),
            None => last_height + margin_bottom,
        };
        let run_end = normal_flow_y(document, cascade, body_id, last_box) + head;
        // Whether the boxes of the run after its first box, up to `end`,
        // hold no break inside them better than the one before the run.
        let holds_no_break = |end: usize| {
            (start + 1..end)
                .filter(|&index| is_flow_box(index))
                .all(|index| box_holds_no_break(document, cascade, body_id, children[index]))
        };
        // A paragraph in the run that breaks between its lines is a better
        // place to break than before the run: the run does not move whole,
        // and the last such paragraph keeps its last lines with the boxes
        // after it instead.
        if let Some(paragraph) = (start..last).rev().find(|&index| {
            is_box(index) && paragraph_may_split_lines(document, cascade, children[index], false)
        }) {
            tails[paragraph] =
                run_end - normal_flow_bottom(document, cascade, body_id, children[paragraph]);
            // The boxes before the paragraph stay tied to its first lines.
            if start < paragraph && Some(start) != first_in_flow {
                let paragraph_box = children[paragraph];
                let height = document.nodes[paragraph_box].layout.size.height;
                let whole = height + document.nodes[paragraph_box].layout.margin.bottom.max(0.0);
                let held = avoided_run_head(document, cascade, body_id, paragraph_box, true);
                let head = held
                    .end
                    .filter(|&head| head < height - 0.001)
                    .unwrap_or(whole);
                units[start] = Some(RunUnit {
                    end: normal_flow_y(document, cascade, body_id, paragraph_box) + head
                        - normal_flow_y(document, cascade, body_id, start_box),
                    firm: held.firm
                        && box_holds_no_break(document, cascade, body_id, start_box)
                        && holds_no_break(paragraph),
                    breaks_inside: false,
                });
            }
            return;
        }
        // A run from the first box has no break before it to move to, unless
        // the run starts inside a table.
        if Some(start) != first_in_flow || breaking_table(start) {
            // A first box that holds breaks between boxes of its own starts
            // the run at the last of them (see `run_break_start`).
            let key = run_break_start(document, cascade, start_box);
            let first = key.unwrap_or(start_box);
            // A start inside the first box, such as a row of a table, is
            // measured from that box, in the coordinates of its parent.
            let start_y = normal_flow_y(document, cascade, body_id, start_box);
            let first_y =
                start_y + current_abs_y(document, first) - current_abs_y(document, start_box);
            let unit = RunUnit {
                end: run_end - first_y,
                firm: held.firm
                    && holds_no_break_from(document, cascade, body_id, start_box, first)
                    && holds_no_break(last),
                breaks_inside: key.is_some() && may_break_inside(document, cascade, first),
            };
            match key {
                // Runs of blocks at more than one level can start at the
                // same box: the longest of them holds the page.
                Some(key) => {
                    run_breaks
                        .entry(key)
                        .and_modify(|known| {
                            *known = if unit.end > known.end { unit } else { *known }
                        })
                        .or_insert(unit);
                }
                None => units[start] = Some(unit),
            }
        }
    };
    let mut run: Option<(usize, usize)> = None;
    let mut previous: Option<usize> = None;
    for index in 0..children.len() {
        // A child that generates no box, such as collapsed white space, and a
        // float, which lies in a flow of its own, are not between the boxes.
        if flow_flags[index].0.is_none() {
            continue;
        }
        let tied_before = previous.filter(|&before| {
            starts_run(before)
                && boxes_are_tied(document, cascade, children[before], children[index])
                && is_run_end(index)
        });
        match (tied_before, run) {
            // A table that breaks inside it ends the run it joins and starts
            // one of its own.
            (Some(before), Some((start, _))) if !breaking_table(before) => {
                run = Some((start, index))
            }
            (Some(before), current) => {
                finish(current);
                run = Some((before, index));
            }
            (None, _) => {
                finish(run.take());
            }
        }
        previous = Some(index);
    }
    finish(run);
    (units, tails)
}

/// The break that keeps a paragraph's last lines with the `edges` of bottom
/// padding and border that close the blocks around it, when the lines fit the
/// page but the edges do not. A break between the last line and the edges
/// (class C) is not taken while one between lines (class B) honors `orphans`
/// and `widows`, as in [`closing_edges_move`]: the page gives up `widows` lines
/// when it keeps its `orphans` after that, and its last line when it cannot
/// (see [`edge_break_lines`]). The edges that do not fit below the lines on the
/// next page start the page after it (see `PaginationState::break_before_edges`);
/// that page only has to hold the lines. `line_metrics`
/// is as for [`widows_break_on_next_page`]. `None` when the lines or the edges
/// do not run past the page end the way that needs, there is only one line,
/// or the next page cannot hold the lines.
fn closing_edges_line_move(
    line_metrics: &[(f32, f32)],
    edges: f32,
    effective_y: f32,
    current_page: u32,
    orphans: usize,
    widows: usize,
    page_origins: &PageOrigins,
) -> Option<LineBreakMove> {
    let page_start = page_origins.origin(current_page);
    let page_end = page_start + page_origins.step_at(current_page);
    let first_top = effective_y + line_metrics.first()?.0;
    let last_bottom = effective_y + line_metrics.last()?.1;
    let fits_lines = first_top >= page_start - 0.001
        && first_top < page_end
        && last_bottom <= page_end + 0.001
        && line_metrics.len() > 1;
    if !fits_lines || last_bottom + edges <= page_end + 0.001 {
        return None;
    }
    widows_break_on_next_page(
        line_metrics,
        effective_y,
        current_page,
        edge_break_lines(line_metrics.len(), orphans, widows),
        page_origins,
    )
}

/// Whether the float `float` lies within the block-axis extent of the content
/// of the box `unit` that holds it, margins included. It then moves along with
/// `unit`, ends where the content does, and reaches no page edge that the
/// content does not: it is no place to break at. A float that reaches into the
/// padding or border below the content, or past the box, is not.
fn float_within_box(document: &Document, unit: usize, float: usize) -> bool {
    let (_, bottom) = float_extent(document, float);
    let unit_layout = &document.nodes[unit].layout;
    let content_bottom = current_abs_y(document, unit) + unit_layout.size.height
        - unit_layout.padding.bottom
        - unit_layout.border.bottom;
    float_starts_within_box(document, unit, float) && bottom <= content_bottom + 0.001
}

/// Whether the float `float` starts at or below the top of the box `unit`
/// that holds it, margins included.
fn float_starts_within_box(document: &Document, unit: usize, float: usize) -> bool {
    float_extent(document, float).0 >= current_abs_y(document, unit) - 0.001
}

/// The top and bottom of the margin box of the float `float`.
fn float_extent(document: &Document, float: usize) -> (f32, f32) {
    let layout = &document.nodes[float].layout;
    let y = current_abs_y(document, float);
    (
        y - layout.margin.top,
        y + layout.size.height + layout.margin.bottom,
    )
}

/// Whether `display` is that of a flex container, a grid or a table laid out
/// as a block-level box: the boxes that [`container_moves_whole`] judges.
fn is_flex_grid_or_table_display(display: DisplayValue) -> bool {
    matches!(
        display,
        DisplayValue::Flex | DisplayValue::Grid | DisplayValue::Table
    )
}

/// Whether `node_id` is a block-level flex container, grid or table that
/// stacks in the normal flow of a horizontal block container: not positioned
/// away from the flow, and not laid out in another writing mode.
fn is_plain_container_box(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    node.kind() == NodeKind::Element
        && is_flex_grid_or_table_display(node.display())
        && matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative
        )
        && !node.used_writing_mode.is_vertical()
}

/// Whether a box below `node_id` forces a page break or starts a named page, so
/// that the page breaks inside `node_id` where that box asks it to.
fn has_mandatory_break_inside(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let mut pending = document.nodes[node_id].children.clone();
    while let Some(id) = pending.pop() {
        let node = &document.nodes[id];
        // Text and elements that generate no box break nothing.
        if node.kind() != NodeKind::Element
            || !node.is_in_document()
            || node.is_unrendered_html_element()
            || node.display() == DisplayValue::None
            // A positioned box keeps the breaks inside it to itself.
            || is_out_of_flow_positioned(document, id)
        {
            continue;
        }
        if forces_page_break(document, cascade, id, true)
            || forces_page_break(document, cascade, id, false)
            || selected_page_name(cascade, id).is_some()
        {
            return true;
        }
        pending.extend_from_slice(&node.children);
    }
    false
}

/// Whether the flex container, grid or table `node_id` can be judged by its
/// children: generated boxes are not among them, and a box inside that forces
/// a page break or starts a named page breaks the page there whichever page
/// the container starts.
fn container_can_be_judged(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let node = &document.nodes[node_id];
    is_plain_container_box(document, node_id)
        && node.multicol.is_none()
        && !node.has_before_or_after_content
        && !is_floating_box_for_pagination(document, cascade, node_id)
        && !has_mandatory_break_inside(document, cascade, node_id)
}

/// Whether the flex container, grid or table `node_id` moves whole to the next
/// page when the closing edges of the blocks around it do not fit after it, as
/// browsers do: the break before it is the one that keeps those edges from
/// being left alone, and the page does not break between its items instead.
///
/// That is so of a grid, of a flex container that lays its items out in one
/// line, and of a table of a single row, whatever their items hold, unless a
/// box inside forces a page break or starts a named page: the page breaks there
/// whichever page the container starts. A flex container that stacks its items,
/// a column of more than one or rows that wrap, and a table with a caption, a
/// header or footer group or more than one row, break between them and are left
/// to the rules for those. A column of one item stacks it in block flow, so that
/// item has to be as unbreakable as a block (see [`breaks_only_around`]). A grid
/// whose rows end exactly at the page end moves whole as well, not only its
/// last row: browsers break before the grid, as they do for one that ends
/// earlier. A grid that runs past the page end breaks between its rows like any
/// other box.
fn container_moves_whole(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    has_named_pages: bool,
) -> bool {
    let node = &document.nodes[node_id];
    if !container_can_be_judged(document, cascade, node_id) {
        return false;
    }
    match node.display() {
        DisplayValue::Table => {
            let elements = |parent: usize| {
                document
                    .children_through_contents(parent)
                    .into_iter()
                    .filter(|&child| document.nodes[child].kind() == NodeKind::Element)
            };
            let mut rows = 0;
            for child in elements(node_id) {
                match document.nodes[child].display() {
                    DisplayValue::TableRow => rows += 1,
                    DisplayValue::TableRowGroup => {
                        for row in elements(child) {
                            if document.nodes[row].display() != DisplayValue::TableRow {
                                return false;
                            }
                            rows += 1;
                        }
                    }
                    _ => return false,
                }
                if rows > 1 {
                    return false;
                }
            }
            true
        }
        DisplayValue::Flex => {
            let computed = &cascade.computed[node_id];
            let row = matches!(
                computed.flex_direction,
                FlexDirectionValue::Row | FlexDirectionValue::RowReverse
            );
            if row && computed.flex_wrap == FlexWrapValue::NoWrap {
                return true;
            }
            // More than one item is enough to know it stacks them.
            let mut items = Vec::new();
            for &child in &node.children {
                push_in_flow_boxes_rev(document, cascade, child, &mut items);
                if items.len() > 1 {
                    return false;
                }
            }
            row || items
                .iter()
                .all(|&item| breaks_only_around(document, cascade, item, has_named_pages))
        }
        _ => true,
    }
}

/// The box representing the final item of a column flex container, a final
/// inner block of its breakable item, a final aligned row of a wrapping row
/// flex container, or the visual last row or unbreakable bottom caption of
/// the table `node_id`. The page breaks between these boxes rather than
/// moving the container whole (see [`container_moves_whole`]).
///
/// The closing edges of the container, and of the blocks it ends, follow that
/// box. Browsers send it to the next page with them when they do not fit after
/// it, and the container stays where it is with the boxes before it. Other
/// members of a selected wrapping row move with its tallest representative.
/// Header and footer groups follow the table's visual order; the first row
/// of an avoided final group can represent the whole group. Rowspans,
/// anonymous cells, unsupported captions and unsupported wrapping flex
/// containers keep their existing fragmentation paths.
fn container_item_tail(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    has_named_pages: bool,
) -> Option<(usize, Option<usize>)> {
    let node = &document.nodes[node_id];
    // A grid moves whole, so only flex containers and tables are left.
    if !matches!(node.display(), DisplayValue::Flex | DisplayValue::Table)
        || !container_can_be_judged(document, cascade, node_id)
        || avoids_page_break_inside(cascade.computed[node_id].break_inside)
        || container_moves_whole(document, cascade, node_id, has_named_pages)
    {
        return None;
    }
    match node.display() {
        DisplayValue::Flex => {
            let computed = &cascade.computed[node_id];
            if computed.flex_direction == FlexDirectionValue::Row
                && computed.flex_wrap == FlexWrapValue::Wrap
            {
                return wrapping_row_item_tail(document, cascade, node_id, has_named_pages)
                    .map(|item| (item, None));
            }
            if !matches!(
                computed.flex_direction,
                FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
            ) || computed.flex_wrap != FlexWrapValue::NoWrap
            {
                return None;
            }
            let items: Vec<usize> = pagination_child_order(document, cascade, node_id)
                .into_iter()
                .filter(|&child| is_in_flow_flex_child_for_pagination(document, node_id, child))
                .collect();
            let (&last, before) = items.split_last()?;
            let item = &document.nodes[last];
            if item.kind() != NodeKind::Element
                || !matches!(item.position(), PositionValue::Static)
                || item.used_writing_mode.is_vertical()
            {
                return None;
            }
            let whole = avoids_page_break_inside(cascade.computed[last].break_inside)
                || matches!(
                    document.block_axis(last).height().expand(),
                    ExpandedDimension::Length(_)
                )
                || breaks_only_around(document, cascade, last, has_named_pages);
            if whole {
                return (!break_between_is_avoided(
                    document,
                    cascade,
                    &before[before.len().saturating_sub(1)..],
                    &[last],
                ))
                .then_some((last, None));
            }
            (computed.flex_direction == FlexDirectionValue::Column)
                .then(|| inner_column_item_tail(document, cascade, last, has_named_pages))
                .flatten()
                .map(|tail| (tail, None))
        }
        _ => {
            // A cell that spans rows ties the rows it spans together, and the
            // table keeps its layout geometry when rows move (see
            // `follow_moved_rows`).
            if super::table::row_moves::has_row_spanning_cell(document, node_id) {
                return None;
            }
            let elements = |parent: usize| {
                document
                    .children_through_contents(parent)
                    .into_iter()
                    .filter(|&child| document.nodes[child].kind() == NodeKind::Element)
                    .filter(|&child| document.nodes[child].display() != DisplayValue::None)
            };
            if document.anonymous_table_cells(node_id).next().is_some() {
                return None;
            }
            // Each visual row or bottom caption, with its row group if any.
            let mut rows: Vec<(usize, Option<usize>)> = Vec::new();
            for child in pagination_child_order(document, cascade, node_id)
                .into_iter()
                .filter(|&child| document.nodes[child].kind() == NodeKind::Element)
            {
                match document.nodes[child].display() {
                    DisplayValue::TableRow => rows.push((child, None)),
                    DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup => {
                        if document.anonymous_table_cells(child).next().is_some() {
                            return None;
                        }
                        // A box in the group that is no row is not a candidate
                        // for the edges, so a table that ends with one has no
                        // last row to move.
                        rows.extend(elements(child).map(|row| (row, Some(child))));
                    }
                    DisplayValue::TableCaption => {
                        if cascade.computed[child].caption_side
                            == raikiri_style::property::CaptionSideValue::Bottom
                        {
                            let caption = &document.nodes[child];
                            if !matches!(caption.position(), PositionValue::Static)
                                || caption.used_writing_mode.is_vertical()
                                || !breaks_only_around(document, cascade, child, has_named_pages)
                            {
                                return None;
                            }
                            rows.push((child, None));
                        }
                    }
                    _ => return None,
                }
            }
            let (&(mut last, group), mut before) = rows.split_last()?;
            let mut fallback = None;
            if let Some(group) = group
                .filter(|&group| avoids_page_break_inside(cascade.computed[group].break_inside))
            {
                // The group's first row represents the whole avoided group.
                // Mandatory breaks were excluded for the table above.
                let first = rows.iter().position(|&(_, held)| held == Some(group))?;
                if let Some(&(previous, _)) = before.last()
                    && before.len() > first
                    && !break_between_is_avoided(document, cascade, &[previous], &[last])
                {
                    fallback = Some(last);
                }
                last = rows[first].0;
                before = &rows[..first];
            }
            // The boxes that start the last row's side of the break and end
            // the other side of it: a group is one of them where the break is
            // the group's own edge.
            let mut next = vec![last];
            let mut previous = Vec::new();
            if let Some(group) = group
                && before.last().is_none_or(|&(_, held)| held != Some(group))
            {
                next.push(group);
            }
            if let Some(&(row, held)) = before.last() {
                previous.push(row);
                previous.extend(held.filter(|&held| group != Some(held)));
            }
            (!break_between_is_avoided(document, cascade, &previous, &next))
                .then_some((last, fallback))
        }
    }
}

/// Whether a `break-after` of one of the boxes `previous` or a `break-before`
/// of one of the boxes `next` avoids the page break between the two sides.
fn break_between_is_avoided(
    document: &Document,
    cascade: &CascadeResult,
    previous: &[usize],
    next: &[usize],
) -> bool {
    next.iter()
        .any(|&next| page_edge_constraint(document, cascade, next, true).1)
        || previous
            .iter()
            .any(|&previous| page_edge_constraint(document, cascade, previous, false).1)
}

/// Whether the caption `caption`, whose box starts at `flow_y` and crosses the
/// page end `page_end`, is left where it is: all of its lines fit the page, or
/// they can be broken between with at least `orphans` lines before the break and
/// `widows` lines after it. A caption of fewer lines than both asks for has no
/// such break. With enough lines, one exists as long as the `orphans` first
/// lines fit the page: the break then goes after as many of them as leave the
/// `widows` last ones. A caption without positioned lines stays: there is
/// nothing to tell where it could be broken.
fn caption_lines_split_well(
    document: &Document,
    cascade: &CascadeResult,
    caption: usize,
    flow_y: f32,
    page_end: f32,
) -> bool {
    let computed = &cascade.computed[caption];
    let orphans = computed.orphans.max(1) as usize;
    let layout = &document.nodes[caption].layout;
    let content_top = flow_y + layout.border.top + layout.padding.top;
    ifc_root_line_bounds(document, caption).is_none_or(|bounds| {
        let count = bounds.len();
        let fitting = bounds
            .iter()
            .take_while(|&&(_, bottom)| content_top + bottom <= page_end + 0.001)
            .count();
        // Every line fits: only the box runs on past the page end, as the room
        // a `height` leaves under the last line, and there is no line to move.
        fitting == count
            || (!lines_keep_together(count, computed.orphans, computed.widows)
                && fitting >= orphans)
    })
}

/// Whether `line_count` lines are too few for any break between them to leave
/// `orphans` lines before it and `widows` lines after it: three lines with the
/// initial two and two. Browsers do not split such lines where a break between
/// boxes is there to take instead.
fn lines_keep_together(line_count: usize, orphans: i32, widows: i32) -> bool {
    line_count < orphans.max(1) as usize + widows.max(1) as usize
}

/// Whether the lines of the paragraph `node_id` are too few to be split by
/// [`lines_keep_together`]. A block box between the lines is a place to break
/// at, so a paragraph with one is not.
fn paragraph_lines_keep_together(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let computed = &cascade.computed[node_id];
    document
        .ifc_layout_node(node_id)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .is_some_and(|lines| {
            lines.block_line_starts.is_empty()
                && lines_keep_together(lines.lines.len(), computed.orphans, computed.widows)
        })
}

/// A tallest member of the final aligned row of a wrapping row flex.
/// Its height represents the entire row for page-fit and ancestor growth.
fn wrapping_row_item_tail(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    container: usize,
    has_named_pages: bool,
) -> Option<usize> {
    let aligned_at_start = |value| {
        matches!(
            value,
            SelfAlignmentValue::Normal
                | SelfAlignmentValue::Stretch
                | SelfAlignmentValue::Start
                | SelfAlignmentValue::FlexStart
        )
    };
    let mut lines: Vec<(f32, f32, Vec<usize>)> = Vec::new();
    for item in pagination_child_order(document, cascade, container)
        .into_iter()
        .filter(|&id| is_in_flow_flex_child_for_pagination(document, container, id))
    {
        let node = &document.nodes[item];
        let values = &cascade.computed[item];
        let aligned = (values.align_self == AlignSelfValue::Auto
            && aligned_at_start(cascade.computed[container].align_items))
            || matches!(values.align_self, AlignSelfValue::Value(value) if aligned_at_start(value));
        let top = node.layout.location.y;
        let bottom = top + node.layout.size.height;
        if !is_plain_block_box(document, item)
            || !matches!(node.position(), PositionValue::Static)
            || node.size_containment.height
            || !aligned
            || node.layout.margin.top != 0.0
            || node.layout.margin.bottom != 0.0
            || !matches!(
                values.min_height,
                ComputedLengthPercentageOrAuto::Auto | ComputedLengthPercentageOrAuto::Px(0.0)
            )
            || values.max_height != ComputedLengthPercentageOrAuto::Auto
            || !top.is_finite()
            || !bottom.is_finite()
            || bottom <= top
        {
            return None;
        }
        if let Some((line_top, line_bottom, members)) = lines.last_mut()
            && (top - *line_top).abs() <= 0.001
        {
            *line_bottom = line_bottom.max(bottom);
            members.push(item);
        } else {
            if lines
                .last()
                .is_some_and(|(_, bottom, _)| top < *bottom - 0.001)
            {
                return None;
            }
            lines.push((top, bottom, vec![item]));
        }
    }
    let (last, previous) = lines.split_last()?;
    let previous = previous.last()?;
    if break_between_is_avoided(document, cascade, &previous.2, &last.2)
        || !last.2.iter().all(|&item| {
            avoids_page_break_inside(cascade.computed[item].break_inside)
                || matches!(
                    document.block_axis(item).height().expand(),
                    ExpandedDimension::Length(_)
                )
                || breaks_only_around(document, cascade, item, has_named_pages)
        })
    {
        return None;
    }
    last.2.iter().copied().max_by(|&left, &right| {
        document.nodes[left]
            .layout
            .size
            .height
            .total_cmp(&document.nodes[right].layout.size.height)
    })
}

/// The innermost final block of a breakable column item whose surrounding
/// plain blocks can grow with it. Only this selected path gains a page
/// candidate; other flex descendants keep their existing placement rules.
/// Clipped boxes and vertical margins retain their existing path.
fn inner_column_item_tail(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    item: usize,
    has_named_pages: bool,
) -> Option<usize> {
    let mut current = item;
    let mut whole = false;
    loop {
        let node = &document.nodes[current];
        if !is_plain_block_box(document, current)
            || !matches!(node.position(), PositionValue::Static)
            || node.used_writing_mode.is_vertical()
            || node.layout.margin.top != 0.0
            || node.layout.margin.bottom != 0.0
            || node.has_before_or_after_content
            || cascade.computed[current].overflow.y
                != raikiri_style::property::OverflowValue::Visible
        {
            return None;
        }
        if whole
            || avoids_page_break_inside(cascade.computed[current].break_inside)
            || (node.is_ifc_root()
                && breaks_only_around(document, cascade, current, has_named_pages))
        {
            return Some(current);
        }
        if !height_follows_content(document, current) || !moves_later_siblings(document, current) {
            return None;
        }
        let mut boxes = Vec::new();
        for child in document.children_through_contents(current) {
            if is_floating_box_for_pagination(document, cascade, child) {
                return None;
            }
            let start = boxes.len();
            push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
            boxes[start..].reverse();
        }
        let (&last, before) = boxes.split_last()?;
        if break_between_is_avoided(
            document,
            cascade,
            &before[before.len().saturating_sub(1)..],
            &[last],
        ) {
            return None;
        }
        // A single-child chain is traversed once; asking whether each
        // wrapper's entire subtree is unbreakable would rescan it quadratically.
        whole = !before.is_empty() && breaks_only_around(document, cascade, last, has_named_pages);
        current = last;
    }
}

/// Whether the page cannot break inside the block `node_id` at a better place
/// than before it: it has no box in its flow, or it has no more lines than
/// `orphans`, so that a break cannot fall between them, or it is a block
/// without padding or border around one box that is as unbreakable, or it
/// holds boxes that are as unbreakable and that avoided breaks tie to each
/// other, so that no break between them is left to take. A flex container, a
/// grid or a table qualifies when it moves whole (see
/// [`container_moves_whole`]), and a float within the block does not count as a
/// place to break at.
fn breaks_only_around(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    has_named_pages: bool,
) -> bool {
    let mut current = node_id;
    loop {
        let node = &document.nodes[current];
        if node.multicol.is_some() {
            return false;
        }
        if is_flex_grid_or_table_display(node.display()) {
            return container_moves_whole(document, cascade, current, has_named_pages);
        }
        if node.is_ifc_root() {
            let computed = &cascade.computed[current];
            return document
                .ifc_layout_node(current)
                .and_then(|node| node.ifc.as_ref())
                .and_then(|ifc| ifc.lines.as_ref())
                .is_some_and(|lines| {
                    // Block boxes between the lines are breakable places too.
                    lines.block_line_starts.is_empty()
                        && lines.lines.len() <= computed.orphans.max(1) as usize
                });
        }
        let mut boxes = Vec::new();
        for &child in &node.children {
            if is_floating_box_for_pagination(document, cascade, child) {
                // A float beside the boxes moves along with the block.
                if !float_within_box(document, node_id, child) {
                    return false;
                }
                continue;
            }
            let start = boxes.len();
            push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
            boxes[start..].reverse();
        }
        let layout = &node.layout;
        let edges =
            layout.padding.top + layout.padding.bottom + layout.border.top + layout.border.bottom;
        match boxes[..] {
            [] => return true,
            // Nothing separates the block from its one box, so it breaks as
            // that box does.
            [only]
                if edges == 0.0
                    && document.nodes[only].kind() == NodeKind::Element
                    && (matches!(
                        document.nodes[only].display(),
                        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
                    ) || is_flex_grid_or_table_display(document.nodes[only].display())) =>
            {
                current = only;
            }
            // The boxes are tied from the first to the last, and the block
            // breaks only where they do, whatever its padding and border: a
            // break between them and its edges is no better than the one
            // before the block.
            [_, _, ..] => {
                return boxes
                    .windows(2)
                    .all(|pair| boxes_are_tied(document, cascade, pair[0], pair[1]))
                    && boxes.iter().all(|&id| {
                        (is_plain_block_box(document, id) || is_plain_container_box(document, id))
                            && !takes_no_block_space(document, id)
                            && breaks_only_around(document, cascade, id, has_named_pages)
                    });
            }
            _ => return false,
        }
    }
}

/// Whether a table row group of `group_height` at `flow_y` that avoids
/// breaks inside it moves whole past `page_end`.
///
/// A row group that is not already at the top of its page moves as a
/// whole, even when it is taller than a page and breaks there.
fn row_group_moves(flow_y: f32, group_height: f32, page_start: f32, page_end: f32) -> bool {
    flow_y.is_finite()
        && flow_y > page_start + 0.001
        && flow_y < page_end
        && flow_y + group_height > page_end
}

/// Whether a flex or grid item of `height` at `effective_y`, with its
/// block-start `margin_top`, crosses `page_end` and can move whole to the
/// next page, whose block size is `next_page_step`.
///
/// An item already at the top of its page, or one taller than the next
/// page, would cross a page edge wherever it went: pushing it only leaves a
/// blank page behind (CSS Fragmentation 3 §4.4 lets monolithic content
/// overflow a fresh fragmentainer). The page top is judged by the margin
/// box, since a block-start margin already at the page top would stay with
/// the item.
fn item_moves_past_page_end(
    effective_y: f32,
    height: f32,
    margin_top: f32,
    page_start: f32,
    page_end: f32,
    next_page_step: f32,
) -> bool {
    let item_can_move =
        effective_y - margin_top.max(0.0) > page_start + 0.001 && height <= next_page_step + 0.001;
    item_can_move
        && effective_y.is_finite()
        && effective_y < page_end
        && effective_y + height > page_end
}

/// Whether a box whose block-start edge is at `start_y` starts on the page
/// spanning `page_start..page_end` and ends at `end_y` past that page's end.
fn starts_on_page_and_runs_past_it(
    start_y: f32,
    end_y: f32,
    page_start: f32,
    page_end: f32,
) -> bool {
    start_y.is_finite() && start_y >= page_start && start_y < page_end && end_y > page_end
}

/// The trailing flex item of each flex container, found once per container.
#[derive(Default)]
struct TrailingFlexChildren {
    by_parent: HashMap<usize, Option<usize>>,
}

impl TrailingFlexChildren {
    /// Whether `node_id` is the trailing item of its flex container, which may
    /// extend into the container's continuation at the page edge.
    fn is_trailing(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
    ) -> bool {
        BoxTree::for_layout(document)
            .parent(node_id, BoxParentScope::Document)
            .is_some_and(|parent_id| {
                let trailing_child = self.by_parent.entry(parent_id).or_insert_with(|| {
                    trailing_flex_child_for_pagination(document, cascade, parent_id)
                });
                *trailing_child == Some(node_id)
            })
    }
}

/// Flow shifts that apply only once pagination has left a subtree or reached
/// a moved line.
#[derive(Default)]
struct DeferredShifts {
    /// Lines of a paragraph that were moved down to a later page, because
    /// they cross a page end or to keep enough widows there. The flow after
    /// the paragraph, and its texts that start on a moved line, take each
    /// shift once they are reached, and are on its page or after it; its texts
    /// before that line stay where they are.
    line_splits: Vec<LineSplit>,
    /// Prefix corrections wait for their owner's first candidate. Moving a
    /// holder can remove a descendant's prefix before earlier siblings visit.
    pending_prefixes: BTreeMap<usize, Vec<PendingPrefix>>,
    /// What the flow after a paragraph gives back once pagination leaves it,
    /// when a text inside it took a whole line shift but its box grew by less:
    /// `(paragraph, correction, page the flow was on before that text)`.
    excess_shifts: Vec<(usize, f32, u32)>,
    /// The page the flow goes back to, at the earliest, at the next candidate
    /// in the flow after such a paragraph: the box ends before its moved lines.
    page_floor: Option<u32>,
}

struct PendingPrefix {
    split: LineSplit,
    ancestor_growth: Vec<(usize, f32)>,
}

/// Lines of a paragraph moved down inside it: see [`DeferredShifts`].
#[derive(Clone, Copy, Debug, PartialEq)]
struct LineSplit {
    /// The paragraph that holds the lines.
    root: usize,
    /// The first line that moved.
    first_moved: usize,
    /// How far the moved lines went down. The paragraph's own later texts and
    /// block children take all of it.
    shift: f32,
    /// How far the paragraph's box grew. The flow after the paragraph takes
    /// this much of the shift: a box that keeps its height holds the moved
    /// lines as overflow.
    growth: f32,
    /// The page the lines moved to.
    page: u32,
    /// Whether the block child laid out right before the first moved line
    /// moved with the lines.
    with_block_before: bool,
}

impl DeferredShifts {
    fn activate_prefixes(&mut self, rank: usize, shifts: &mut SubtreeShifts) {
        while self
            .pending_prefixes
            .first_key_value()
            .is_some_and(|(&first, _)| first <= rank)
        {
            let (_, prefixes) = self
                .pending_prefixes
                .pop_first()
                .expect("a reached prefix exists");
            for prefix in prefixes {
                self.line_splits.push(prefix.split);
                shifts.growth.extend(prefix.ancestor_growth);
            }
        }
    }

    /// Adds to `flow_shift` every pending shift that `node_id` has reached,
    /// and drops them: the corrections of the paragraphs it left, then the
    /// line splits. Returns the last page that a line split it reached moved
    /// lines to. `current_page` is the page the flow is on before `node_id`.
    fn apply_reached(
        &mut self,
        document: &Document,
        text_lines: &TextLineIndex,
        node_id: usize,
        ifc_root: Option<(usize, f32)>,
        current_page: u32,
        flow_shift: &mut f32,
    ) -> Option<u32> {
        let mut reached_page: Option<u32> = None;
        let page_floor = &mut self.page_floor;
        self.excess_shifts
            .retain(|&(root, correction, page_before)| {
                let left = !is_descendant_or_self(document, node_id, root);
                if left {
                    *flow_shift += correction;
                    *page_floor =
                        Some(page_floor.map_or(page_before, |floor| floor.min(page_before)));
                }
                !left
            });
        let excess_shifts = &mut self.excess_shifts;
        self.line_splits.retain(
            |&LineSplit {
                 root,
                 first_moved,
                 shift,
                 growth,
                 page,
                 with_block_before,
             }| {
                let after_paragraph = !is_descendant_or_self(document, node_id, root);
                let reached = after_paragraph
                    || ifc_root.is_some_and(|(text_root, _)| text_root == root)
                        && text_lines
                            .get(node_id)
                            .and_then(|owned| owned.lines.first())
                            .is_some_and(|line| line.line >= first_moved)
                    || follows_line_of_paragraph(
                        document,
                        node_id,
                        root,
                        first_moved,
                        with_block_before,
                    );
                if reached {
                    *flow_shift += if after_paragraph { growth } else { shift };
                    // The flow after the paragraph takes the growth alone.
                    if !after_paragraph && growth != shift {
                        excess_shifts.push((root, growth - shift, current_page));
                    }
                    // A box that kept its height leaves what follows it where
                    // it is, on an earlier page than the moved lines.
                    if !after_paragraph || growth > 0.0 {
                        reached_page = reached_page.max(Some(page));
                    }
                }
                !reached
            },
        );
        reached_page
    }
}

/// Paragraphs laid out by the inline engine that already had a candidate
/// inside them. Only the first candidate of a paragraph may move its root: a
/// later one would move again what the earlier ones placed.
#[derive(Default)]
struct EnteredParagraphs {
    roots: HashSet<usize>,
}

impl EnteredParagraphs {
    /// Registers every paragraph root around `node_id` and returns whether
    /// this is the first candidate inside all of them.
    ///
    /// `ifc_root`, the candidate's own paragraph root, is registered directly
    /// only when it is an anonymous cell; the other roots are the inline
    /// formatting context roots among the ancestors of `node_id`.
    fn enter(
        &mut self,
        document: &Document,
        ifc_root: Option<(usize, f32)>,
        node_id: usize,
    ) -> bool {
        let mut first = ifc_root
            .filter(|&(root, _)| document.is_anonymous_cell(root))
            .is_none_or(|(root, _)| self.roots.insert(root));
        // A block that an anonymous cell lays out among its lines enters
        // the cell's paragraph too, though the cell is none of its source
        // ancestors: the text after it then follows the lines that moved
        // with it instead of moving the whole cell again.
        if let Some(cell) = wrapping_anonymous_cell(document, node_id)
            .filter(|&cell| ifc_root.is_none_or(|(root, _)| root != cell))
            && document
                .ifc_layout_node(cell)
                .is_some_and(|cell| cell.ifc.is_some())
            && !self.roots.insert(cell)
        {
            first = false;
        }
        for id in document.ancestors(node_id) {
            if document.nodes[id].is_ifc_root() && !self.roots.insert(id) {
                first = false;
                // Higher IFC roots were registered when this root was first entered.
                break;
            }
        }
        first
    }
}

/// Paragraph blocks whose leading line or whose orphans and widows move them
/// as a unit to the next page. Each block is checked once, from its first
/// text child, so it is not reflowed again for later lines.
#[derive(Default)]
struct CheckedParagraphs {
    /// Paragraphs whose own edges were checked for a single exterior holder.
    oversized_edges: HashSet<usize>,
    /// Blocks whose first line was checked against the page end.
    block_text: HashSet<usize>,
    /// Blocks whose orphans and widows were checked.
    orphans_widows: HashSet<usize>,
    /// Column paragraph eligibility, including rejections, for this pass.
    column_eligibility: HashMap<usize, bool>,
    /// Runs of lines between the block children of a paragraph that were
    /// checked against the page end: `(paragraph root, first line)`.
    lines_after_blocks: HashSet<(usize, usize)>,
    /// Runs of lines before a block inside an inline box of their paragraph
    /// that were checked against a break before the block: `(paragraph
    /// root, first line)`.
    lines_before_inline_blocks: HashSet<(usize, usize)>,
}

impl CheckedParagraphs {
    /// Whether both checks already ran for `block_id`.
    fn is_fully_checked(&self, block_id: usize) -> bool {
        self.block_text.contains(&block_id) && self.orphans_widows.contains(&block_id)
    }
}

/// The block whose lines hold `candidate`'s text and that the paragraph
/// checks may move as a unit, with whether it is nested in plain block flow
/// rather than a direct child of the flow's contents box.
///
/// The block is `paragraph_root`, the inline layout root of the text's
/// paragraph, or else the text's parent. A block nested in plain block flow
/// uses the same checks as a direct child; line moves propagate only the
/// growth its height constraints allow. A caption of a block table in plain
/// block flow is such a block too (see [`is_flow_table_caption`]): the rows
/// and captions after it follow its moved lines. There is no such block for
/// text placed by positioned insets, for a block whose checks have all run,
/// or for a nested block outside plain block flow or inside an out-of-flow
/// box. A block inside a float is one when only floats break the plain block
/// flow up to the body (see [`is_float_flow_paragraph`]).
fn line_block_parent(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    roots: &candidates::FlowRoots,
    candidate: &PageCandidate,
    paragraph_root: Option<usize>,
    checked_paragraphs: &CheckedParagraphs,
    uses_positioned_coordinates: bool,
) -> Option<(usize, bool)> {
    paragraph_root
        .or(document.parent_of(candidate.node_id))
        .filter(|parent_id| {
            !uses_positioned_coordinates && !checked_paragraphs.is_fully_checked(*parent_id)
        })
        .and_then(|parent_id| {
            if document.layout_parent_of(parent_id)? == roots.contents_box {
                Some((parent_id, false))
            } else {
                let float_paragraph = candidate.context.is_float_descendant()
                    && is_float_flow_paragraph(document, cascade, parent_id, roots.element);
                (float_paragraph
                    || !candidate.context.is_float_descendant()
                        && !candidate.context.is_out_of_flow_descendant()
                        && (is_block_flow_paragraph(document, cascade, parent_id, roots.element)
                            || is_flow_table_caption(document, cascade, parent_id, roots.element)))
                .then_some((parent_id, true))
            }
        })
}

/// The inline child of `block` from which the lines of the paragraph `block`
/// reach the row of the table cell that holds it (see
/// [`moves_reach_cell_row`]), when `block` is that cell or a block inside it
/// whose lines can move down there at a page end, as those of a paragraph in
/// block flow do: the row grows to hold them, the cells beside it keep their
/// places, and the flow after the row follows its growth.
///
/// Every box from the paragraph up to the cell stays in its flow, keeps the
/// moved lines in view (none clips at a set height or ignores its content),
/// and the table is placed in plain block flow up to the body. A table that
/// repeats a header or footer group on each page leaves no room for its
/// copies here, and pagination on transposed axes moves no lines this way.
pub(super) fn cell_flow_inline(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
) -> Option<usize> {
    let node = &document.nodes[block];
    if document.context.pagination_on_transposed_axes
        || node.used_writing_mode.is_vertical()
        || !matches!(
            node.display(),
            DisplayValue::Block
                | DisplayValue::FlowRoot
                | DisplayValue::ListItem
                | DisplayValue::TableCell
        )
    {
        return None;
    }
    let cell = document
        .inclusive_ancestors(block)
        .find(|&id| document.nodes[id].display() == DisplayValue::TableCell)?;
    let (_, _, table) = cell_row_end(document, cell)?;
    let in_view = document
        .inclusive_ancestors(block)
        .take_while(|&id| id != cell)
        .all(|id| {
            holds_moved_lines(document, cascade, id, false)
                && !is_floating_box_for_pagination(document, cascade, id)
                && !matches!(
                    document.nodes[id].position(),
                    PositionValue::Absolute | PositionValue::Fixed
                )
        });
    let repeats_groups = document.nodes[table].children.iter().any(|&child| {
        matches!(
            document.nodes[child].display(),
            DisplayValue::TableHeaderGroup | DisplayValue::TableFooterGroup
        )
    });
    if !in_view
        || repeats_groups
        || document.nodes[cell].size_containment.height
        || !placed_in_block_flow(document, cascade, table, body_id)
    {
        return None;
    }
    node.children
        .iter()
        .copied()
        .find(|&child| document.nodes[child].in_ifc_subtree())
        .filter(|&inline| moves_reach_cell_row(document, inline))
}

/// Where block `block_id` now sits in the flow, given that its text at raw
/// position `text_raw_y` now sits at `text_effective_y`. The block already
/// sits `flow_shift` below its raw position, so this keeps the text's offset
/// inside the block.
fn block_effective_y(
    document: &Document,
    block_id: usize,
    text_raw_y: f32,
    text_effective_y: f32,
    root_flow_offset: f32,
    flow_shift: f32,
) -> f32 {
    let block_raw_y = root_flow_offset + current_abs_y(document, block_id) - flow_shift;
    text_effective_y - (text_raw_y - block_raw_y)
}

/// The paragraph `paragraph_root` with its flex container, when the
/// paragraph is a flex item alone on its line. Such a paragraph cannot move
/// as a unit, but its later lines can move down inside it. Text placed by
/// positioned insets, or inside a float or an out-of-flow box, has none.
fn flex_line_paragraph(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    candidate: &PageCandidate,
    paragraph_root: Option<usize>,
    uses_positioned_coordinates: bool,
) -> Option<(usize, usize)> {
    paragraph_root
        .filter(|_| {
            !uses_positioned_coordinates
                && !candidate.context.is_float_descendant()
                && !candidate.context.is_out_of_flow_descendant()
        })
        .and_then(|root| {
            flex_container_of_item_alone_on_its_line(document, cascade, root, body_id)
                .map(|container| (root, container))
        })
}

/// Whether a block of `block_height` is no taller than a page of `page_step`,
/// give or take 0.001, so that moving it whole to a page can hold it.
fn block_fits_page_step(block_height: f32, page_step: f32) -> bool {
    block_height.is_finite() && block_height <= page_step + 0.001
}

/// Walks the in-flow boxes before `node_id` in the flow below `body_id`, a
/// level at a time: for the parent of the box, then of each ancestor of it
/// that is not the body, `found` sees that parent, the box or ancestor, and
/// the boxes its children before the box or ancestor generate (see [`push_in_flow_boxes_rev`]),
/// nearest first. True as soon as `found` is.
fn any_level_before(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    mut found: impl FnMut(usize, usize, &[usize]) -> bool,
) -> bool {
    let mut node = node_id;
    let mut boxes = Vec::new();
    while node != body_id {
        let Some(parent) = document.parent_of(node) else {
            return false;
        };
        boxes.clear();
        for &child in document.nodes[parent]
            .children
            .iter()
            .take_while(|&&child| child != node)
        {
            push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
        }
        if found(parent, node, &boxes) {
            return true;
        }
        node = parent;
    }
    false
}

/// Whether in-flow content comes before the box `node_id` on the page that
/// starts at `page_start`: a box before it or before an ancestor of it that
/// is not the body (see [`any_level_before`]), empty or not, or the lines of
/// text there, or the generated `::before` box of an ancestor of it, the body
/// too, that ends below the page start or starts on the page, where the line
/// loop of the ancestor's paragraph placed it. Only such content gives the
/// flow a break before `node_id` on the page: below nothing but margins, or
/// the top padding and border of the boxes it begins, a box starts the page's
/// content, as Chromium has no break before it there (CSS Fragmentation 3
/// §4.4). Positions are those of the flow, without relative offsets.
pub(super) fn follows_box_on_page(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    page_start: f32,
) -> bool {
    content_lies_before(document, cascade, body_id, node_id, &|top, bottom| {
        top >= page_start - 0.001 || bottom > page_start + 0.001
    })
}

/// The lowest end, in the flow, of the in-flow content before the box
/// `node_id` (see [`follows_box_on_page`]) that ends at or above `y`, the
/// block-start margin edge of the box: content beside the box, as an earlier
/// flex item, grid item or table cell, ends below it. Content of no height
/// counts up to `border_top` too: an empty box whose margins collapse with
/// those of the box lies at the end of the collapsed margins in the flow,
/// below the margin edge of the box, and Chromium breaks after it all the
/// same. Callers pass the block-start border edge of the box there only when
/// its margin can adjoin content before it (see [`margins_adjoin_in_flow`]),
/// and the margin edge otherwise. `None` when no such content comes before
/// the box.
pub(super) fn content_end_above(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    y: f32,
    border_top: f32,
) -> Option<f32> {
    let end = std::cell::Cell::new(None::<f32>);
    content_lies_before(document, cascade, body_id, node_id, &|top, bottom| {
        if bottom <= y + 0.001 || (bottom <= top + 0.001 && bottom <= border_top + 0.001) {
            end.set(Some(end.get().map_or(bottom, |end| end.max(bottom))));
        }
        false
    });
    end.get()
}

/// Whether `accept` takes the top and bottom in the flow, without relative
/// offsets, of some in-flow content before the box `node_id`: a box before it
/// or before an ancestor of it that is not the body (see
/// [`any_level_before`]), empty or not, or the lines of text there, or the
/// generated `::before` box of an ancestor of it, the body too, where the line
/// loop of the ancestor's paragraph placed it.
fn content_lies_before(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    accept: &dyn Fn(f32, f32) -> bool,
) -> bool {
    let flow_y = |id| {
        current_abs_y(document, id) - accumulated_relative_offset_y(document, cascade, body_id, id)
    };
    let pseudo_lies = |parent: usize, element: usize, pseudo: raikiri_style::PseudoElem| {
        generated_box_lies(document, cascade, &flow_y, parent, element, pseudo, accept)
    };
    any_level_before(
        document,
        cascade,
        body_id,
        node_id,
        |parent, node, boxes| {
            // An element that generates no box of its own, an empty or a
            // `display: contents` one, may still hold generated content,
            // block-level or on the lines of the paragraph, or lay out a
            // phantom line, which is content of the page.
            boxless_content_before(
                document,
                cascade,
                parent,
                node,
                &|child, pseudo| {
                    renders_pseudo(cascade, child, pseudo) && pseudo_lies(parent, child, pseudo)
                },
                &|id| {
                    let top = flow_y(id);
                    accept(top, top)
                },
            ) || boxes.iter().any(|&id| {
                if document.nodes[id].kind() != NodeKind::Element {
                    // Text has no box of its own: its lines tell where it lies.
                    return document
                        .ifc_text_lines(id)
                        .and_then(|owned| {
                            let bounds = positioned_owned_text_line_bounds(document, &owned)?;
                            let root = document.ifc_layout_node(owned.root)?.layout;
                            let origin = flow_y(owned.root) + root.border.top + root.padding.top;
                            let (first, _) = *bounds.first()?;
                            let (_, last) = *bounds.last()?;
                            Some(accept(origin + first, origin + last))
                        })
                        .unwrap_or(false);
                }
                let top = flow_y(id);
                accept(top, top + document.nodes[id].layout.size.height)
            }) || (renders_before_pseudo(cascade, parent)
                && pseudo_lies(parent, parent, raikiri_style::PseudoElem::Before))
        },
    )
}

/// Whether the generated `pseudo` box of `element` has bounds that `accept`
/// takes, as top and bottom in the flow. It lies in the paragraph of
/// `parent`, or of the box around it when `parent` is a `display: contents`
/// element, which has no box: a block-level one where the line loop placed
/// it, an inline one on the lines that hold its text. `flow_y` gives the flow
/// position of a box.
fn generated_box_lies(
    document: &Document,
    cascade: &CascadeResult,
    flow_y: &dyn Fn(usize) -> f32,
    parent: usize,
    element: usize,
    pseudo: raikiri_style::PseudoElem,
    accept: &dyn Fn(f32, f32) -> bool,
) -> bool {
    let mut root = parent;
    while document.nodes[root].kind() == NodeKind::Element
        && document.nodes[root].display() == DisplayValue::Contents
        && let Some(up) = document.parent_of(root)
    {
        root = up;
    }
    let id = crate::generated_content::generated_node_id(element, pseudo);
    let node = document
        .ifc_layout_node(root)
        .unwrap_or(&document.nodes[root]);
    let origin = flow_y(root) + node.layout.border.top + node.layout.padding.top;
    if let Some(lines) = node.ifc.as_ref().and_then(|ifc| ifc.lines.as_ref()) {
        let blocks = lines
            .placed_blocks
            .iter()
            .filter(|block| block.node == id as u64)
            .map(|block| {
                let top = origin + block.border_box.y;
                (top, top + block.border_box.height)
            });
        let line_bounds = lines
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| super::ifc::text_lines::glyph_owners(line).contains(&id))
            .map(|(index, line)| {
                let top = origin + lines.line_top(index);
                (
                    top,
                    top + crate::layout::ifc::outside_marker::fit_height(line),
                )
            });
        let mut bounds = blocks.chain(line_bounds).peekable();
        if bounds.peek().is_some() {
            return bounds.any(|(top, bottom)| accept(top, bottom));
        }
    }
    // An empty `::before` leaves neither a block nor a line: a block-level
    // one whose margins collapse through it, or an inline one that makes no
    // line, which may not even make its element hold a paragraph. Its
    // anonymous block is still a box of no height at the start of the
    // element's content (CSS 2.1 §9.2.1.1, §9.4.2), as an empty `div` is, and
    // Chromium breaks after it as after such a box. The content of a
    // `display: contents` element starts at its first box.
    element == parent
        && pseudo == raikiri_style::PseudoElem::Before
        && holds_block_flow(document.nodes[root].display())
        && crate::generated_content::is_in_flow_generated_text(cascade, element, pseudo)
        && if element == root {
            Some(origin)
        } else {
            first_box_in(document, element).map(flow_y)
        }
        .is_some_and(|start| accept(start, start))
}

/// Whether a box of `display` lays its in-flow children out one below
/// another, in block flow: not a flex or grid container, nor a table or a
/// part of one that holds rows, columns or cells side by side.
fn holds_block_flow(display: DisplayValue) -> bool {
    !display.is_flex_or_grid_container()
        && !matches!(
            display,
            DisplayValue::Table
                | DisplayValue::InlineTable
                | DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup
                | DisplayValue::TableRow
                | DisplayValue::TableColumnGroup
                | DisplayValue::TableColumn
        )
}

/// The first element among the children of the `display: contents` element
/// `element`, or of the `display: contents` elements there, that generates
/// a box.
fn first_box_in(document: &Document, element: usize) -> Option<usize> {
    document.nodes[element].children.iter().find_map(|&child| {
        let node = &document.nodes[child];
        match node.display() {
            _ if node.kind() != NodeKind::Element => None,
            DisplayValue::None => None,
            DisplayValue::Contents => first_box_in(document, child),
            _ => Some(child),
        }
    })
}

/// Whether the block-start margin of the box `node_id` can adjoin content
/// before it in the flow below `body_id`: every ancestor of it up to the
/// body, the body too, lays its children out in block flow (see [`holds_block_flow`]) and
/// in one column. In a flex, grid or table container, or a multi-column
/// one, content before the box may lie beside it instead.
pub(super) fn margins_adjoin_in_flow(document: &Document, body_id: usize, node_id: usize) -> bool {
    let mut node = node_id;
    while let Some(parent) = document.parent_of(node) {
        let holder = &document.nodes[parent];
        if holder.is_multicol_container() || !holds_block_flow(holder.display()) {
            return false;
        }
        if parent == body_id {
            break;
        }
        node = parent;
    }
    true
}

/// Whether generated content before the box `node_id` in the flow below
/// `body_id` runs past `y`: the `::before` of an ancestor of it, the body
/// too, or the generated content of an element before it, or before an
/// ancestor, that has no box of its own (see [`boxless_content_before`]).
/// Such content reaches the pages it runs onto, though no box of the flow
/// is placed there, so a box after it fits from the page it is on.
pub(super) fn generated_content_runs_past(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    y: f32,
) -> bool {
    generated_content_lies_before(document, cascade, body_id, node_id, &|_, bottom| {
        bottom > y + 0.001
    })
}

/// The top and the end, in the flow, of the generated content right before
/// the box `node_id`, the lowest of it, when it ends at or above `y`, a
/// position in the flow above the box, and no other in-flow content before
/// the box ends below it (see [`content_lies_before`]).
/// That is the `::before` of its parent, or the generated content of an
/// element before the box that has no box of its own (see
/// [`boxless_content_before`]); or, when the box begins its parent and its
/// margin collapses through the parent's top (see
/// [`starts_with_child_margin`]), such content before the parent, and so on
/// up. The margin of the box then adjoins it, as one between sibling blocks
/// does.
pub(super) fn generated_content_end_above(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    y: f32,
) -> Option<(f32, f32)> {
    let flow_y = |id| {
        current_abs_y(document, id) - accumulated_relative_offset_y(document, cascade, body_id, id)
    };
    let last = std::cell::Cell::new(None::<(f32, f32)>);
    let record = |top: f32, bottom: f32| {
        if bottom <= y + 0.001 && last.get().is_none_or(|(_, end)| bottom > end) {
            last.set(Some((top, bottom)));
        }
        false
    };
    // Generated content before an ancestor is separated from the box by the
    // ancestor's own edges or margin, unless the box begins the ancestor and
    // its margin collapses through the ancestor's top (see
    // `starts_with_child_margin`): the walk goes up only from such a level.
    any_level_before(
        document,
        cascade,
        body_id,
        node_id,
        |parent, node, boxes| {
            let lies_before = boxless_content_before(
                document,
                cascade,
                parent,
                node,
                &|child, pseudo| {
                    renders_pseudo(cascade, child, pseudo)
                        && generated_box_lies(
                            document, cascade, &flow_y, parent, child, pseudo, &record,
                        )
                },
                &|_| false,
            ) || !boxes.is_empty();
            if renders_before_pseudo(cascade, parent) {
                generated_box_lies(
                    document,
                    cascade,
                    &flow_y,
                    parent,
                    parent,
                    raikiri_style::PseudoElem::Before,
                    &record,
                );
                return true;
            }
            lies_before
                || last.get().is_some()
                || !starts_with_child_margin(document, cascade, body_id, parent, node)
        },
    );
    let (top, end) = last.get()?;
    // Any other in-flow content that ends below it comes between: a box
    // after it whose negative margin pulls the box above that box's end too.
    (!content_lies_before(document, cascade, body_id, node_id, &|_, bottom| {
        bottom > end + 0.001
    }))
    .then_some((top, end))
}

/// Whether the margin of `child`, the first of the in-flow content of
/// `parent`, collapses through the top of `parent`: `parent` is a
/// `display: contents` element, which has no box, or a plain block of the
/// normal flow with no padding or border above, where the first box of
/// `child` starts in the flow, as it does only when the margin is not held
/// inside `parent`.
fn starts_with_child_margin(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    parent: usize,
    child: usize,
) -> bool {
    let node = &document.nodes[parent];
    if node.display() == DisplayValue::Contents {
        return true;
    }
    let flow_y = |id| {
        current_abs_y(document, id) - accumulated_relative_offset_y(document, cascade, body_id, id)
    };
    let child_box = if document.nodes[child].display() == DisplayValue::Contents {
        first_box_in(document, child)
    } else {
        Some(child)
    };
    matches!(node.display(), DisplayValue::Block | DisplayValue::ListItem)
        && is_plain_block_box(document, parent)
        && !node.is_multicol_container()
        && !is_floating_box_for_pagination(document, cascade, parent)
        && node.layout.border.top + node.layout.padding.top <= 0.001
        && child_box.is_some_and(|child| (flow_y(child) - flow_y(parent)).abs() <= 0.001)
}

/// Whether `accept` takes the top and bottom in the flow of some generated
/// content before the box `node_id`: the `::before` of an ancestor of it,
/// the body too, or the generated content of an element before it, or
/// before an ancestor, that has no box of its own (see
/// [`boxless_content_before`]).
fn generated_content_lies_before(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    accept: &dyn Fn(f32, f32) -> bool,
) -> bool {
    let flow_y = |id| {
        current_abs_y(document, id) - accumulated_relative_offset_y(document, cascade, body_id, id)
    };
    any_level_before(document, cascade, body_id, node_id, |parent, node, _| {
        boxless_content_before(
            document,
            cascade,
            parent,
            node,
            &|child, pseudo| {
                renders_pseudo(cascade, child, pseudo)
                    && generated_box_lies(document, cascade, &flow_y, parent, child, pseudo, accept)
            },
            &|_| false,
        ) || (renders_before_pseudo(cascade, parent)
            && generated_box_lies(
                document,
                cascade,
                &flow_y,
                parent,
                parent,
                raikiri_style::PseudoElem::Before,
                accept,
            ))
    })
}

/// Whether an element among the children of `parent` before `node` that
/// generates no box of its own, or among the children of the `display:
/// contents` elements there, holds content: a generated `::before` or
/// `::after` that `pseudo_on_page` accepts, or a phantom line, the line of an
/// empty inline element, whose top `phantom_on_page` accepts. The anonymous
/// block of that line is a box of no height (CSS 2.1 §9.4.2), as an empty
/// `div` is.
fn boxless_content_before(
    document: &Document,
    cascade: &CascadeResult,
    parent: usize,
    node: usize,
    pseudo_on_page: &dyn Fn(usize, raikiri_style::PseudoElem) -> bool,
    phantom_on_page: &dyn Fn(usize) -> bool,
) -> bool {
    let mut pending: Vec<usize> = document.nodes[parent]
        .children
        .iter()
        .copied()
        .take_while(|&child| child != node)
        .collect();
    while let Some(id) = pending.pop() {
        if document.nodes[id].kind() == NodeKind::Element
            && [
                raikiri_style::PseudoElem::Before,
                raikiri_style::PseudoElem::After,
            ]
            .into_iter()
            .any(|pseudo| pseudo_on_page(id, pseudo))
        {
            return true;
        }
        match sibling_adjacency::flow_box(document, cascade, id) {
            sibling_adjacency::FlowBox::Phantom if phantom_on_page(id) => return true,
            sibling_adjacency::FlowBox::Contents => {
                pending.extend_from_slice(&document.nodes[id].children);
            }
            _ => {}
        }
    }
    false
}

/// Whether the element `node_id` has a `pseudo` box of the flow: generated
/// content that is not absolutely positioned. It is not a child in the arena,
/// so walks over the children do not see it.
fn renders_pseudo(
    cascade: &CascadeResult,
    node_id: usize,
    pseudo: raikiri_style::PseudoElem,
) -> bool {
    cascade
        .pseudo
        .get(&(raikiri_style::StyleNodeId::new(node_id as u64), pseudo))
        .is_some_and(|style| pseudo_style_renders(style))
}

/// Whether the element `parent` has a `::before` box of the flow (see
/// [`renders_pseudo`]).
fn renders_before_pseudo(cascade: &CascadeResult, parent: usize) -> bool {
    renders_pseudo(cascade, parent, raikiri_style::PseudoElem::Before)
}

/// Whether the element `id` has a `::before` box of the flow (`before`) or
/// an `::after` one, which stands between the edge of the element on that
/// side and its first or last box (see [`renders_pseudo`]).
fn renders_edge_pseudo(cascade: &CascadeResult, id: usize, before: bool) -> bool {
    let pseudo = if before {
        raikiri_style::PseudoElem::Before
    } else {
        raikiri_style::PseudoElem::After
    };
    renders_pseudo(cascade, id, pseudo)
}

/// Whether a pseudo-element of `style` generates a box of the flow:
/// content that is not absolutely positioned or hidden by `display: none`.
fn pseudo_style_renders(style: &raikiri_style::ComputedValues) -> bool {
    style.display != DisplayValue::None
        && !matches!(
            style.position,
            PositionValue::Absolute | PositionValue::Fixed
        )
        && !style.content.is_empty()
        && !style
            .content
            .iter()
            .any(|part| matches!(part, raikiri_style::property::ContentComponent::None))
}

/// Whether in-flow content comes before the box `node_id` in the flow below
/// `body_id`: a sibling of the box, or of an ancestor of it that is not the
/// body, which generates an in-flow box (see [`any_level_before`]), or
/// the top border or padding of such a block-level ancestor in horizontal
/// writing. The flow can break before
/// such a box, between it and what comes first, as browsers break after those
/// edges too. A box that starts the flow, below nothing but margins, cannot be
/// broken from.
fn follows_flow_content(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> bool {
    any_level_before(document, cascade, body_id, node_id, |parent, _, boxes| {
        !boxes.is_empty()
            || (parent != body_id && {
                let holder = &document.nodes[parent];
                !holder.display().is_inline_level()
                    && !holder.used_writing_mode.is_vertical()
                    && holder.layout.border.top + holder.layout.padding.top > 0.0
            })
    })
}

/// Whether the in-flow text of `height` with page type `page_name` opens a
/// page of another type than `comparison_page_name`, the one the flow
/// compares it against (see [`PageNames::comparison`]). Text that has no
/// height, or that comes before any content of its flow
/// (`name_context_seen`), does not.
fn text_changes_page_name(
    name_context_seen: bool,
    name_participates_in_flow: bool,
    height: f32,
    page_name: &Option<String>,
    comparison_page_name: &Option<String>,
) -> bool {
    name_context_seen
        && height > 0.0
        && name_participates_in_flow
        && page_name != comparison_page_name
}

/// Whether a page type a candidate set inside a flex item's own named flow
/// (`keep_local_page_type`) reaches the enclosing flow: it does once the
/// candidate has moved on to a page after `page_before_candidate`.
fn page_type_leaves_local_flow(
    keep_local_page_type: bool,
    current_page: u32,
    page_before_candidate: u32,
) -> bool {
    !keep_local_page_type || current_page > page_before_candidate
}

/// Whether the in-flow text of `height` hands its page type on to the flow.
/// Text that changed the page type or consumed a pending break already
/// set the page type through the page it opened, and text without height
/// sets none.
fn text_sets_page_name(
    height: f32,
    name_participates_in_flow: bool,
    named_page_change: bool,
    consumes_pending_break: bool,
    leaves_local_flow: bool,
) -> bool {
    height > 0.0
        && name_participates_in_flow
        && !named_page_change
        && !consumes_pending_break
        && leaves_local_flow
}

/// Whether a box with page type `page_name` opens a page of another type
/// than `comparison_page_name`, the one the flow compares it against (see
/// [`PageNames::comparison`]). A box that comes before any content of its
/// flow (`name_context_seen`), or that joins the page transition of a
/// preceding zero-height named box (`joins_last_named`), does not.
fn box_changes_page_name(
    name_context_seen: bool,
    page_name: &Option<String>,
    comparison_page_name: &Option<String>,
    joins_last_named: bool,
) -> bool {
    name_context_seen && page_name != comparison_page_name && !joins_last_named
}

/// Whether the box with `context` hands its page type on to the flow: it
/// does when it takes part in page type transitions and stands for content
/// (a direct child of the body, a named box, a rendered leaf, or a box with
/// a named break deferred past its end), once it opened a page
/// (`page_transition`) or its page type left the local flow of a flex item
/// (`leaves_local_flow`).
fn box_sets_page_name(
    context: &candidates::BoxContext,
    page_transition: bool,
    leaves_local_flow: bool,
) -> bool {
    context.page_name_in_flow()
        && (context.is_direct_body_element()
            || context.is_named()
            || context.is_rendered_leaf()
            || context.deferred_named_break_after())
        && (page_transition || leaves_local_flow)
}

/// Whether the text from `text_y` with `text_height` starts at or below the
/// start of the page that starts at `page_start` and is `page_step` tall and
/// ends past that page's end, while its block, from `block_y`, starts on that
/// page. Such a fixed-height block is pushed to the next page as a unit, as
/// monolithic content is.
fn first_line_overflows(
    text_y: f32,
    text_height: f32,
    block_y: f32,
    page_start: f32,
    page_step: f32,
) -> bool {
    let page_end = page_start + page_step;
    text_y.is_finite()
        && text_height.is_finite()
        && text_y >= page_start
        && block_y >= page_start
        && block_y < page_end
        && text_y + text_height > page_end
}

/// The height of the text that decides whether a fixed-height block of
/// `block_height` whose text starts at `text_y` moves, given the block's top
/// at `block_y` on a page of `page_step`.
///
/// A block that fits a page moves as a unit when the text taken whole, of
/// `text_height`, straddles the page end. A taller block fragments anyway,
/// so only its first line, of the height `first_line` returns, decides
/// whether it starts on the next page, and only when the block's top down to
/// that line fits a page: otherwise moving it gains nothing, as for a first
/// line taller than the page. A block with no resolved height stays.
fn first_line_judged_height(
    block_height: f32,
    page_step: f32,
    block_y: f32,
    text_y: f32,
    text_height: f32,
    first_line: impl FnOnce() -> Option<f32>,
) -> Option<f32> {
    if block_fits_page_step(block_height, page_step) {
        Some(text_height)
    } else {
        first_line().filter(|first_line| {
            block_height.is_finite()
                && block_fits_page_step(text_y - block_y + first_line, page_step)
        })
    }
}

/// Whether the border box of `block` ends where its content does: its height
/// follows the content, or a `min-height` or `max-height` that the content
/// passes or stays below does not set it. A `height` ends it elsewhere.
pub(super) fn box_ends_at_content(
    document: &super::context::LayoutSession<'_>,
    block: usize,
) -> bool {
    if height_follows_content(document, block) {
        return true;
    }
    if !document.block_axis(block).height().is_auto()
        || document.nodes[block].size_containment.height
    {
        return false;
    }
    let (min, max) = border_box_height_bounds(document, block);
    let height = document.nodes[block].layout.size.height;
    height > min + 0.5 && height < max - 0.5
}

/// Whether the box of the paragraph `block_id` leaves the lines that pass its
/// end to pagination: its height follows its lines, or it does not clip them.
/// A box with a set height that clips its block-axis overflow hides the lines
/// past its end, so they stay where they are and add no page.
fn box_leaves_lines_to_pagination(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block_id: usize,
) -> bool {
    height_follows_content(document, block_id)
        || !element_clips_overflow(document, cascade, block_id)
}

/// Whether later lines of the paragraph `block_id` can move down inside it: it
/// is in one column and holds nothing but text, floats, atomic inlines that
/// come after its first text (see [`atomics_follow_text`]), and out-of-flow
/// boxes that stay where their insets put them or follow the moved lines (see
/// [`out_of_flow_follow`]). The atomic inlines on the moved lines move with
/// them (see [`shift_paragraph_lines`]); a float is a page candidate of its
/// own. Its box grows with the moved lines as far as its
/// height allows (see [`lines_shift_growth`]).
fn lines_can_shift_inside(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block_id: usize,
) -> bool {
    document
        .ifc_layout_node(block_id)
        .and_then(|node| node.ifc.as_ref())
        .is_some_and(|ifc| {
            let mut atomics_follow = None;
            ifc.multicol_fragments.is_none()
                && ifc.boxes.iter().all(|b| {
                    b.kind == IfcBoxKind::Float
                        || (b.kind == IfcBoxKind::OutOfFlow
                            && out_of_flow_follow(document, cascade, block_id, b.node).is_some())
                        || (b.kind == IfcBoxKind::Atomic
                            && *atomics_follow.get_or_insert_with(|| {
                                atomics_follow_text(document, cascade, block_id, &ifc.boxes)
                            }))
                })
        })
}

/// Whether every atomic inline among the `boxes` of the paragraph `root` comes
/// after the first of its text in tree order, the order in which its page
/// candidates are placed. The text is what moves the lines, and the boxes
/// before it are placed before it is: a move would leave them behind. The
/// walk stops at the first text or atomic inline, and does not enter the
/// paragraph's boxes, whose content is not its text.
fn atomics_follow_text(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    boxes: &[IfcBox],
) -> bool {
    let boxed: HashMap<usize, bool> = boxes
        .iter()
        .map(|b| (b.node, b.kind == IfcBoxKind::Atomic))
        .collect();
    // Children in the order layout and the page candidates take them, which
    // puts the summary of a `details` first.
    let children = |node: usize| {
        pagination_child_order_borrowed(document, cascade, node)
            .into_owned()
            .into_iter()
            .rev()
    };
    let mut stack: Vec<usize> = children(root).collect();
    while let Some(node) = stack.pop() {
        if let Some(&atomic) = boxed.get(&node) {
            if atomic {
                return false;
            }
            continue;
        }
        match &document.nodes[node].data {
            crate::node::NodeData::Text(text) if !text.text_content.trim().is_empty() => {
                return true;
            }
            // A `display: contents` element lays its children out in its
            // place, so they are walked as the paragraph's own; a
            // `display: none` one, or one that HTML never renders whatever
            // its display, lays out nothing.
            crate::node::NodeData::Element(_)
                if !document.nodes[node].is_display_none()
                    && !document.nodes[node].is_unrendered_html_element() =>
            {
                stack.extend(children(node));
            }
            _ => {}
        }
    }
    true
}

/// Whether the lines of the paragraph `block_id` that follow its block
/// children can move down inside it: no size containment ignores them, it is
/// in one column, and the only boxes it lays out are atomic inlines, floats,
/// block children and out-of-flow boxes. Its box grows with the moved lines
/// as far as its height allows (see [`lines_shift_growth`]): as in Chromium,
/// a `height`, `min-height` or `max-height` does not keep the lines in
/// place, and lines past a set height overflow the box. The block children move with the lines
/// before them (see [`blocks_after_line`]). An out-of-flow box is one that a block-axis
/// inset places away from the lines or that follows them (see
/// [`out_of_flow_follow`]). A float is a page candidate of its own, placed
/// where the layout put it: the lines move, the float does not follow them.
fn lines_can_shift_between_blocks(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block_id: usize,
) -> bool {
    !document.nodes[block_id].size_containment.height
        && content_follows_block_children(document, cascade, block_id)
}

/// Whether the content of the paragraph `block_id` after a block child
/// follows that child down when it moves or grows: the paragraph is in one
/// column, and lays out only atomic inlines, floats, block children and
/// out-of-flow boxes that a block-axis inset places away from the lines or
/// that follow them (see [`out_of_flow_follow`]).
fn content_follows_block_children(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block_id: usize,
) -> bool {
    document
        .ifc_layout_node(block_id)
        .and_then(|node| node.ifc.as_ref())
        .is_some_and(|ifc| {
            ifc.multicol_fragments.is_none()
                && ifc.boxes.iter().all(|b| {
                    matches!(
                        b.kind,
                        IfcBoxKind::Block | IfcBoxKind::Atomic | IfcBoxKind::Float
                    ) || (b.kind == IfcBoxKind::OutOfFlow
                        && (has_explicit_block_axis_inset(document, cascade, b.node)
                            || out_of_flow_follow(document, cascade, block_id, b.node).is_some()))
                })
        })
}

/// Whether all the lines of the paragraph `block_id` can move down inside it
/// although it has block children, because every block child comes after the
/// last line: the block children follow the moved lines when pagination reaches
/// them, and the paragraph's own checks judge all the lines as one run. It is
/// in one column, and apart from those blocks lays out only floats, atomic
/// inlines that come after its first text (see [`atomics_follow_text`]) and
/// move with the lines that hold them, and out-of-flow boxes that stay where
/// their insets put them or follow the moved lines (see
/// [`out_of_flow_follow`]). Its box grows with the moved lines as far as its
/// height allows (see [`lines_shift_growth`]).
fn lines_can_shift_above_blocks(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block_id: usize,
) -> bool {
    document
        .ifc_layout_node(block_id)
        .and_then(|node| node.ifc.as_ref())
        .is_some_and(|ifc| {
            let mut atomics_follow = None;
            ifc.multicol_fragments.is_none()
                && ifc.boxes.iter().all(|b| {
                    matches!(b.kind, IfcBoxKind::Block | IfcBoxKind::Float)
                        || (b.kind == IfcBoxKind::OutOfFlow
                            && out_of_flow_follow(document, cascade, block_id, b.node).is_some())
                        || (b.kind == IfcBoxKind::Atomic
                            && *atomics_follow.get_or_insert_with(|| {
                                atomics_follow_text(document, cascade, block_id, &ifc.boxes)
                            }))
                })
                && ifc.lines.as_ref().is_some_and(|lines| {
                    lines
                        .block_line_starts
                        .iter()
                        .all(|&(_, after)| after >= lines.lines.len())
                })
        })
}

/// The lines of the paragraph `root` that lie between two of its block
/// children, as line indices, for the run that holds the first line of the
/// text `node_id`. `None` when no block child comes before that line: the
/// paragraph's own checks judge such lines.
fn lines_between_blocks(
    document: &Document,
    text_lines: &TextLineIndex,
    node_id: usize,
    root: usize,
) -> Option<std::ops::Range<usize>> {
    let (start, end, _) = line_run(document, text_lines, node_id, root)?;
    Some(start?..end)
}

/// The lines of the paragraph `root` that break with the first line of the
/// text `node_id`, when a block child of the paragraph stands before or
/// after them: they break as a paragraph of their own, apart from the lines
/// on the other side of the block (see [`lines_between_blocks`]), with
/// whether a block child comes before them and whether they are the last
/// lines of the paragraph. `None` when no block child stands among the lines.
fn line_run_among_blocks(
    document: &Document,
    text_lines: &TextLineIndex,
    node_id: usize,
    root: usize,
) -> Option<LineRun> {
    let (start, end, total) = line_run(document, text_lines, node_id, root)?;
    (start.is_some() || end < total).then(|| LineRun {
        lines: start.unwrap_or(0)..end,
        after_block: start.is_some(),
        ends_paragraph: end == total,
    })
}

/// A run of lines of a paragraph between its block children.
pub(super) struct LineRun {
    /// The lines of the run, as indices into the paragraph's lines.
    pub(super) lines: std::ops::Range<usize>,
    /// Whether a block child of the paragraph comes before the run.
    pub(super) after_block: bool,
    /// Whether no block child comes after the run.
    pub(super) ends_paragraph: bool,
}

/// The run of lines of the paragraph `root` that holds the first line of the
/// text `node_id`, between its block children: the line after the block
/// before the run (`None` when no block child comes before it), the line
/// after its last line, and the number of lines in the paragraph.
fn line_run(
    document: &Document,
    text_lines: &TextLineIndex,
    node_id: usize,
    root: usize,
) -> Option<(Option<usize>, usize, usize)> {
    let first = text_lines.get(node_id)?.lines.first()?.line;
    let lines = document
        .ifc_layout_node(root)?
        .ifc
        .as_ref()?
        .lines
        .as_ref()?;
    let total = lines.lines.len();
    let mut start = None;
    let mut end = total;
    for &(_, after) in &lines.block_line_starts {
        if after <= first {
            start = start.max(Some(after));
        } else {
            end = end.min(after);
        }
    }
    Some((start, end, total))
}

/// The margin between the block child of the paragraph `root` that comes
/// right before its line `line` and that line: how far the line starts below
/// the block's border box, when only the block's block-end margin lies
/// between them. The line then sits in that margin as the first box of an
/// anonymous block after the block would, and a page edge inside the margin
/// truncates it there (CSS Fragmentation 3 section 5.2, Chromium). `None`
/// when no block comes right before the line, or when something else, such as
/// a float the line clears, pushes the line further down. `None` too
/// when the paragraph holds floats, which lines moving up leave behind, or
/// is a flex or grid item, whose size the lines do not set alone.
fn margin_above_line(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    root: usize,
    line: usize,
) -> Option<f32> {
    let node = document.ifc_layout_node(root)?;
    let ifc = node.ifc.as_ref()?;
    if ifc.boxes.iter().any(|item| item.kind == IfcBoxKind::Float)
        || edges_break_container(document, root, body_id)
            .flatten()
            .is_some()
    {
        return None;
    }
    let lines = ifc.lines.as_ref()?;
    let block = lines
        .block_line_starts
        .iter()
        .rev()
        .find(|&&(_, after)| after == line)?
        .0;
    // An anonymous box, with no style of its own, is left alone.
    cascade.computed.get(block)?;
    let line_top = node.layout.border.top + node.layout.padding.top + lines.line_top(line);
    // The block sits in the root, maybe inside inline boxes, each placed
    // in the box around it. Relative offsets only paint them elsewhere.
    let mut block_bottom = document.nodes[block].layout.size.height;
    let mut current = block;
    while current != root {
        block_bottom += document.pagination_local_y(current)
            - layout_relative_offset_y(document, cascade, body_id, current);
        current = document.nodes[current].parent?;
    }
    let gap = line_top - block_bottom;
    let (positive, negative) = adjoining_margin_extremes(document, cascade, block, false, None);
    (gap > 0.001 && gap <= positive + negative + 0.01).then_some(gap)
}

/// The block child of the paragraph `root` laid out right after its line
/// `end - 1`, and whether it sits inside an inline box of the paragraph, in
/// the tree of elements.
fn block_after_lines(document: &Document, root: usize, end: usize) -> Option<(usize, bool)> {
    let lines = document
        .ifc_layout_node(root)?
        .ifc
        .as_ref()?
        .lines
        .as_ref()?;
    let &(block, _) = lines
        .block_line_starts
        .iter()
        .find(|&&(_, after)| after == end)?;
    Some((block, in_inline_box(document, root, block)))
}

/// Whether the block `block` of the paragraph `root` sits inside an inline
/// box of the paragraph, in the tree of elements.
fn in_inline_box(document: &Document, root: usize, block: usize) -> bool {
    std::iter::successors(document.nodes[block].parent, |&id| {
        document.nodes[id].parent
    })
    .take_while(|&ancestor| ancestor != root)
    .any(|ancestor| document.nodes[ancestor].display().is_inline_level())
}

/// The blocks laid out among the lines of the paragraph `root`, once each
/// and in order, each with the index of the line after it and whether it
/// sits inside an inline box of the paragraph.
fn paragraph_blocks(document: &Document, root: usize) -> Vec<(usize, usize, bool)> {
    let mut seen = HashSet::new();
    document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref()?.lines.as_ref())
        .map_or_else(Vec::new, |lines| {
            lines
                .block_line_starts
                .iter()
                .filter(|&&(block, _)| seen.insert(block))
                .map(|&(block, after)| (block, after, in_inline_box(document, root, block)))
                .collect()
        })
}

/// The lines of the paragraph `root` before its run of lines from `start`
/// on that count toward the `orphans` of a break among that run, on the page
/// that starts at `page_start`, as their block-start and block-end edges
/// relative to `y`, in order. When the block right before the run sits
/// inside an inline box and the run's first line does not start the page,
/// Chromium counts each block of the paragraph inside an inline box as one
/// line (see [`inline_block_after_lines`]), and every line of the paragraph
/// on the page before the run counts too, back to a block straight in the
/// paragraph, which ends the lines before it; a block that goes on from
/// the page before counts as a line there too, and keeps its top above
/// the page. Blocks end where the line after them starts, and blocks next
/// to each other are one line together. `bounds` holds
/// every line of the paragraph, offset by `y`.
fn inline_lines_before_run(
    document: &Document,
    root: usize,
    start: usize,
    bounds: &[(f32, f32)],
    y: f32,
    page_start: f32,
) -> Vec<(f32, f32)> {
    if inline_block_after_lines(document, root, start).is_none()
        || bounds
            .get(start)
            .is_none_or(|&(top, _)| y + top <= page_start + 0.5)
    {
        return Vec::new();
    }
    let page_top = page_start - y;
    let blocks: Vec<_> = paragraph_blocks(document, root)
        .into_iter()
        .filter(|&(_, after, _)| after <= start)
        .collect();
    // A block straight in the paragraph ends the lines before it.
    let first = blocks
        .iter()
        .filter(|&&(_, _, inline)| !inline)
        .map(|&(_, after, _)| after)
        .max()
        .unwrap_or(0);
    let mut ahead: Vec<(f32, f32)> = bounds.get(first..start).unwrap_or_default().to_vec();
    let mut bottom = None;
    for &(block, after, inline) in blocks.iter().rev() {
        if !inline || after < first {
            break;
        }
        // A block before the run has a line after it, the run's at least.
        let next_top = bounds[after].0;
        // Blocks next to each other are one line together.
        let height = document.nodes[block].layout.size.height;
        match bottom {
            Some(line) if line == after => {
                if let Some(last) = ahead.last_mut() {
                    last.0 -= height;
                }
            }
            _ => ahead.push((next_top - height, next_top)),
        }
        bottom = Some(after);
    }
    ahead.retain(|&(_, bottom)| bottom > page_top + 0.5);
    ahead.sort_by(|a, b| a.0.total_cmp(&b.0));
    ahead
}

/// How far below its border-box top the box `node_id`, which avoids breaks
/// inside it, has to fit on a page: its border box, or the end of its flow
/// content when that runs out past a height held below it and nothing clips
/// it. A break through that content is a break inside the box.
fn avoided_box_extent(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> f32 {
    let node = &document.nodes[node_id];
    let layout = &node.layout;
    if node.size_containment.height || element_clips_overflow(document, cascade, node_id) {
        return layout.size.height;
    }
    let content_top = layout.border.top + layout.padding.top;
    // The margins below the content are cut at a page break, so only the
    // border box of the last child counts.
    let flow_end = if node.is_ifc_root() {
        document
            .ifc_layout_node(node_id)
            .and_then(|lines_node| lines_node.ifc.as_ref())
            .and_then(|ifc| ifc.lines.as_ref())
            .map(|lines| content_top + lines.shifted_height())
    } else {
        last_flow_child(document, cascade, node_id)
            .filter(|&child| document.nodes[child].display() != DisplayValue::Contents)
            .map(|child| {
                let child_layout = &document.nodes[child].layout;
                child_layout.location.y + child_layout.size.height
                    - layout_relative_offset_y(document, cascade, body_id, child)
            })
    };
    flow_end.map_or(layout.size.height, |end| layout.size.height.max(end))
}

/// Whether lines of the paragraph that holds the block `block` come before
/// it: the block is a block child of the paragraph's root, straight or
/// through inline boxes, laid out after one of its lines.
fn lines_before_block(document: &Document, block: usize) -> bool {
    let Some(root) = std::iter::successors(document.nodes[block].parent, |&id| {
        document.nodes[id].parent
    })
    .find(|&id| !document.nodes[id].display().is_inline_level()) else {
        return false;
    };
    document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .and_then(|lines| {
            lines
                .block_line_starts
                .iter()
                .find(|&&(child, _)| child == block)
        })
        .is_some_and(|&(_, after)| after > 0)
}

/// Where the first piece of the content of the block `block` ends (see
/// [`first_piece_end`]), read through a block child that comes before the
/// first line of a paragraph: that child holds the paragraph's first piece.
fn block_first_piece_end(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    block: usize,
) -> Option<f32> {
    first_piece_end(document, cascade, body_id, block, 0).or_else(|| {
        let node = &document.nodes[block];
        if !node.is_ifc_root() {
            return None;
        }
        let lines = document
            .ifc_layout_node(block)?
            .ifc
            .as_ref()?
            .lines
            .as_ref()?;
        let &(leading, _) = lines
            .block_line_starts
            .iter()
            .find(|&&(_, after)| after == 0)?;
        // An anonymous box or one inside an inline box is not read.
        cascade.computed.get(leading)?;
        (document.nodes[leading].parent == Some(block)).then_some(())?;
        Some(
            normal_flow_y(document, cascade, body_id, leading)
                + first_piece_end(document, cascade, body_id, leading, 1)?,
        )
    })
}

/// The block that [`block_after_lines`] finds after the line `end - 1` of
/// the paragraph `root`, when it sits inside an inline box. Chromium lays
/// such a block out on a line of its own in the paragraph, which `orphans`
/// and `widows` count as one line: a page break before the block leaves
/// that one line after it.
fn inline_block_after_lines(document: &Document, root: usize, end: usize) -> Option<usize> {
    block_after_lines(document, root, end)
        .and_then(|(block, in_inline_box)| in_inline_box.then_some(block))
}

/// Whether the lines `run` of the paragraph `root` are the last of its content:
/// no block child comes after them. The run ends where the next block child
/// begins, or at the last line.
fn run_ends_paragraph(document: &Document, root: usize, run: &std::ops::Range<usize>) -> bool {
    document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .is_some_and(|lines| {
            !lines
                .block_line_starts
                .iter()
                .any(|&(_, after)| after >= run.end)
        })
}

/// The first of the lines `line_metrics` (block-start and block-end edges
/// relative to `effective_y`) that crosses the end of the page from
/// `page_start` to `page_end` without starting the next page, if it is not the
/// first thing on its page.
fn straddling_line(
    line_metrics: &[(f32, f32)],
    effective_y: f32,
    page_start: f32,
    page_end: f32,
) -> Option<usize> {
    let line = line_metrics
        .iter()
        .position(|&(_, bottom)| effective_y + bottom > page_end + 0.001)?;
    let top = effective_y + line_metrics[line].0;
    (top > page_start + 0.001 && top < page_end - 0.001).then_some(line)
}

/// The block children of the paragraph `root` that come after its line
/// `line`: moving the lines from `line` on moves them too, so that a block
/// placed later starts from where the lines took it. A block split across
/// columns comes after the line only when its first part does. With
/// `with_block_before`, the block laid out right before line `line` counts
/// too.
fn blocks_after_line(
    document: &Document,
    root: usize,
    line: usize,
    with_block_before: bool,
) -> HashSet<usize> {
    let mut first_after: HashMap<usize, usize> = HashMap::new();
    let starts = document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .into_iter()
        .flat_map(|lines| &lines.block_line_starts);
    for &(block, after) in starts {
        let first = first_after.entry(block).or_insert(after);
        *first = (*first).min(after);
    }
    first_after
        .into_iter()
        .filter_map(|(block, after)| {
            (after > line || (with_block_before && after == line)).then_some(block)
        })
        .collect()
}

/// Whether `node_id` lies in a block child of the paragraph `root` that comes
/// after line `line`, so that moving the lines from `line` on moves it too
/// (see [`blocks_after_line`] for `with_block_before`).
fn follows_line_of_paragraph(
    document: &Document,
    node_id: usize,
    root: usize,
    line: usize,
    with_block_before: bool,
) -> bool {
    let block = std::iter::successors(Some(node_id), |&node| document.layout_parent_of(node))
        .find(|&node| document.layout_parent_of(node) == Some(root));
    document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .zip(block)
        .is_some_and(|(lines, block)| {
            (document.nodes[block].display().is_atomic_inline_level()
                || is_replaced_for_pagination(document, block))
                && atomic_nodes(lines.lines.get(line..).unwrap_or_default()).contains(&block)
                || blocks_after_line(document, root, line, with_block_before).contains(&block)
                // A box out of the flow with no insets along the block axis
                // keeps the static position of its anchor, on a line. Lines
                // only move when every such box is one that
                // `out_of_flow_follow` puts on its line, so its text goes
                // with that line.
                || document
                    .oof_coordinates
                    .get(&block)
                    .is_some_and(|coordinate| coordinate.follow_static_y)
                    && find_oof_anchor(document, lines, block).is_some_and(|(anchor, ..)| anchor >= line)
        })
}

/// How a paragraph that crosses the page end is repaired so that its break
/// honors `orphans` and `widows`.
#[derive(Debug, PartialEq)]
enum LineBreakFix {
    /// The natural break stays.
    Keep,
    /// Lines move down inside the paragraph to the pages the moves name, the
    /// earlier page first.
    MoveLines(Vec<LineBreakMove>),
    /// The whole block moves to the next page.
    MoveBlock,
}

/// How a paragraph's lines fall against the end of the current page, and
/// whether the natural break between them honors `orphans` and `widows`.
struct ParagraphLineFit {
    /// The number of lines the paragraph's text owns.
    total_lines: usize,
    /// Whether no whole-block move has consumed the opening fragment yet.
    first_fragment: bool,
    /// Whether the first line starts on the current page.
    starts_on_page: bool,
    /// The leading lines whose block-end edge is at or before the page end,
    /// up to the first line that is not.
    lines_before_break: usize,
    /// The used `orphans`, at least 1.
    orphans: usize,
    /// The used `widows`, at least 1.
    widows: usize,
    /// Whether the first line starts on a later page than the box that holds it,
    /// which starts below the top of this one: the padding and border above the
    /// line end the page. Set by the caller, which knows the boxes.
    top_edges_end_page: bool,
    /// Whether the paragraph's one line fits the page but its own padding and
    /// border below it do not, and the page can break before the paragraph:
    /// with no break between lines, the line goes with the edges, so the
    /// whole paragraph moves. Set by the caller, which knows the boxes.
    lone_line_moves_with_edges: bool,
    /// Whether the lines need a break that leaves fewer than `orphans`
    /// lines before it or fewer than `widows` lines after it.
    violates_line_constraints: bool,
    /// The padding and border below the last line, which stay with it; 0 for
    /// a paragraph of one line.
    tail_edges: f32,
    /// Whether every line fits the page but the last does not with
    /// `tail_edges` below it.
    tail_overflows: bool,
    /// Whether every line fits the page but the last does not with the edges
    /// below it, a paragraph of one line included.
    edges_overflow: bool,
}

impl ParagraphLineFit {
    /// Whether the lines all fit the page, but not with the padding and border
    /// below the last one, and the page cannot give up the `widows` lines the
    /// edges ask for and keep the `orphans`: a paragraph of one line has no
    /// break between lines, and with more only the last line could go to the
    /// next page, which leaves a break inside the lines that violates one of
    /// the two.
    fn edges_leave_too_few_lines(&self) -> bool {
        self.edges_overflow && self.total_lines.saturating_sub(self.widows) < self.orphans
    }

    /// Whether the break leaves fewer than `orphans` lines on the page, or
    /// all the lines of a paragraph shorter than that do not fit it.
    fn leaves_too_few_orphans(&self) -> bool {
        self.lines_before_break < self.orphans.min(self.total_lines)
    }

    /// How to repair the break between the paragraph's lines on the page
    /// `current_page`. A block that can move whole moves whenever the break
    /// leaves too few orphans, as lines moved down inside it would leave
    /// its top on this page. Else lines move down inside the paragraph when a
    /// line crosses the page end and `can_move_lines` allows it: the line that
    /// crosses each page end starts the next page, and the first break that
    /// leaves too few widows moves up as far as the orphans allow (see
    /// [`paragraph_line_moves`]). Else the whole block moves when
    /// `can_move_block` and the break violates a constraint; otherwise the
    /// natural break stays.
    ///
    /// The block also moves when no line starts on the page because the
    /// padding and border above the lines end it (`top_edges_end_page`): the
    /// break before the block is the one that leaves them with the lines.
    /// So does a paragraph of one line whose own padding and border below it
    /// cross the page end (`lone_line_moves_with_edges`).
    fn fix(
        &self,
        line_metrics: &[(f32, f32)],
        effective_y: f32,
        current_page: u32,
        page_origins: &PageOrigins,
        can_move_block: bool,
        can_move_lines: impl FnOnce() -> bool,
    ) -> LineBreakFix {
        let move_block = can_move_block
            && (self.starts_on_page || self.top_edges_end_page)
            && (self.violates_line_constraints || self.lone_line_moves_with_edges);
        let moves = if !(move_block && self.leaves_too_few_orphans())
            && self.starts_on_page
            && (self.lines_before_break < self.total_lines || self.tail_overflows)
            && can_move_lines()
        {
            paragraph_line_moves(line_metrics, effective_y, current_page, self, page_origins)
        } else {
            Vec::new()
        };
        if !moves.is_empty() {
            LineBreakFix::MoveLines(moves)
        } else if move_block {
            LineBreakFix::MoveBlock
        } else {
            LineBreakFix::Keep
        }
    }

    /// Measures `line_metrics`, the block-start and block-end edges of each
    /// line relative to `effective_y`, against the page from `page_start`
    /// to `page_end`, with the computed `orphans` and `widows`. `edges` is the
    /// padding and border below the last line, which `line_metrics` does not
    /// count and which stay with that line. A paragraph of one line has
    /// nothing to break between, so `tail_edges` stays 0 for it; whether the
    /// edges cross the page end is still recorded in `edges_overflow`, and
    /// the caller moves the box whole at the break before it.
    fn measure(
        line_metrics: &[(f32, f32)],
        effective_y: f32,
        page_start: f32,
        page_end: f32,
        orphans: i32,
        widows: i32,
        edges: f32,
    ) -> Self {
        let total_lines = line_metrics.len();
        let starts_on_page = line_metrics.first().is_some_and(|(line_top, _)| {
            let top = effective_y + line_top;
            top >= page_start - 0.001 && top < page_end
        });
        let lines_before_break = line_metrics
            .iter()
            .take_while(|(_, line_bottom)| effective_y + line_bottom <= page_end + 0.001)
            .count();
        let needs_break = lines_before_break < total_lines;
        let orphans = orphans.max(1) as usize;
        let widows = widows.max(1) as usize;
        let usable = usable_widows(total_lines, orphans, widows);
        let violates_line_constraints = needs_break
            && (lines_before_break < orphans
                || total_lines.saturating_sub(lines_before_break) < usable);
        let tail_edges = if total_lines > 1 && edges > 0.0 {
            edges
        } else {
            0.0
        };
        let edges_overflow = !needs_break
            && line_metrics
                .last()
                .is_some_and(|(_, bottom)| effective_y + bottom + edges > page_end + 0.001);
        let tail_overflows = tail_edges > 0.0 && edges_overflow;
        Self {
            total_lines,
            first_fragment: true,
            starts_on_page,
            lines_before_break,
            orphans,
            widows,
            top_edges_end_page: false,
            lone_line_moves_with_edges: false,
            violates_line_constraints,
            tail_edges,
            tail_overflows,
            edges_overflow,
        }
    }
}

/// The adjoining block margins between siblings, which an unforced page
/// break inside them truncates.
#[derive(Default)]
struct SiblingMargins {
    adjacency: sibling_adjacency::SiblingAdjacency,
    /// Unpaginated border-box bottom of the last in-flow block-level child
    /// seen for each parent, while no other content followed it. The gap to
    /// the next such sibling is their collapsed margin.
    previous_bottoms: HashMap<usize, (usize, f32)>,
    /// For a candidate right after an in-flow block that is not a candidate,
    /// so never recorded in `previous_bottoms`, that block and its
    /// unpaginated border-box bottom.
    preceding_bottoms: HashMap<usize, (usize, f32)>,
    /// The empty box whose block-end margin still adjoins the latest forced
    /// break: the forced-break box itself, or an empty box right after it.
    /// A margin collapsed through it follows the break and is kept.
    forced_break_box: Option<usize>,
    /// The margins of the empty boxes from the latest forced break down to
    /// `forced_break_box`, as Chromium keeps them below the page top and as
    /// the layout collapsed them with the block-end margin of the box
    /// before the break (see `forced_empty_box_shift`).
    forced_margins: Option<((f32, f32), (f32, f32))>,
    /// The top of the page that the latest forced break opened.
    forced_page_top: Option<f32>,
    /// Whether truncating a margin moved a box.
    moved_boxes: bool,
    /// The box that forces a break after it, which a float took to the new
    /// page, paired with that page's index, until the in-flow box after the
    /// float is placed. The forced boundary sits before its margin; the next
    /// page edge truncates it.
    break_taken_by_float: Option<(usize, u32)>,
}

/// A box that starts on `natural_page`, below a page edge that may fall in the
/// margin above it.
struct MarginBreak {
    node_id: usize,
    /// The body element, which ends the walk for relative offsets.
    body_id: usize,
    /// The initial layout proves this clearing box has no used clearance.
    clearance_free: bool,
    /// The unpaginated top of the box.
    raw_y: f32,
    /// The top of the box in the flow so far.
    effective_y: f32,
    /// The page the box starts on while the margin keeps its size.
    natural_page: u32,
    /// Whether a forced break sits before the box.
    forced_before: bool,
    /// The block-start margin of the box.
    margin_top: f32,
    /// Whether the box's margin collapses with the margin of the block
    /// before it.
    collapses_with_previous_block: bool,
    /// Where the margin the box sits in ends, when the box is empty and the
    /// margin goes on to the box after it. While a negative margin there
    /// pulls that box above this one (see `pulled_run_end`), the page edge is
    /// found from where it starts, in place of `effective_y` and
    /// `natural_page`.
    run_end: Option<MarginRunEnd>,
}

/// The box that ends a run of margins, found for an empty box that the
/// margins collapse through: the next block in the flow.
#[derive(Clone, Copy)]
struct MarginRunEnd {
    node_id: usize,
    /// The unpaginated top of the box.
    raw_y: f32,
    /// The block size of the box.
    height: f32,
}

/// How the top of `at.natural_page` truncates the margin between the box
/// `at` and generated content right before it, a block-level `::before` or
/// `::after` that is no candidate of its own (see
/// [`generated_content_end_above`]), as it truncates one between sibling
/// blocks: Chromium breaks the margin there too.
fn truncation_after_generated_content(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    page_origins: &PageOrigins,
    at: &MarginBreak,
) -> Option<MarginTruncation> {
    let page_top = page_origins.origin(at.natural_page);
    // A box that may have clearance keeps its gap, as between siblings; and
    // only a page edge in the margin above the box cuts it.
    if !at.collapses_with_previous_block
        || at.run_end.is_some()
        || !at.clearance_free
            && !matches!(
                cascade.computed[at.node_id].clear,
                raikiri_style::property::ClearValue::None
            )
        || at.natural_page == 0
        || at.effective_y <= page_top + 0.001
    {
        return None;
    }
    // Nothing but margins lies between the generated content and the box:
    // those of both, collapsed, which the page edge must fall in. Generated
    // content at the top of the page, as a table of no height there, starts
    // the page: the margin after it is not at a break.
    let (content_top, previous_bottom) =
        generated_content_end_above(document, cascade, body_id, at.node_id, at.effective_y)?;
    if previous_bottom > page_top + 0.001 || content_top >= page_top - 0.001 {
        return None;
    }
    let target_y = page_origins.truncated_margin_target(
        at.natural_page,
        at.forced_before,
        at.margin_top,
        || previous_bottom,
        false,
    );
    Some(MarginTruncation {
        delta: target_y - at.effective_y,
        underflows: at.forced_before && at.margin_top < 0.0,
    })
}

/// How a page edge truncates the margin above a box.
struct MarginTruncation {
    /// How far the box moves; negative when it moves up.
    delta: f32,
    /// Whether a negative margin after a forced break pulls the box above
    /// the top of the page the break opens. The break still opens that page.
    underflows: bool,
}

impl SiblingMargins {
    /// How the top of `at.natural_page` truncates the margin between the box
    /// `at` and the sibling block before it, or `None` when the page edge
    /// falls outside that margin or no margin separates the two. A box that
    /// may have clearance is left alone, since its gap may hold that space.
    fn truncation_at_break(
        &mut self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        page_origins: &PageOrigins,
        at: &MarginBreak,
    ) -> Option<MarginTruncation> {
        let node_id = at.node_id;
        // Retain a clearing box's gap unless the initial layout proved it
        // contains no clearance. Pagination may already have moved its floats.
        if !at.collapses_with_previous_block
            || !at.clearance_free
                && !matches!(
                    cascade.computed[node_id].clear,
                    raikiri_style::property::ClearValue::None
                )
        {
            return None;
        }
        let (previous, gap) = self.previous_and_gap(document, cascade, node_id, at.raw_y)?;
        if let Some(truncation) =
            self.margin_after_forced_empty_box(document, cascade, page_origins, previous, gap, at)
        {
            return truncation;
        }
        if !(gap.is_finite() && gap > 0.0) {
            return None;
        }
        let float_break_page = self
            .break_taken_by_float
            .filter(|&(source, _)| is_descendant_or_self(document, source, previous))
            .map(|(_, page)| page);
        let taken_by_float = float_break_page.is_some();
        let forced_before = at.forced_before || taken_by_float;
        let follows_forced_break = self.follows_forced_break(document, cascade, previous, node_id);
        // The page edge cuts the whole run of margins, which ends where the
        // box after an empty box starts, so the run is judged from there; the
        // empty box then moves by as much as the box after it, and so do the
        // boxes that follow.
        let pull = self.pulled_run_end(document, cascade, at);
        let run_end = pull.map(|(end, _)| end);
        let pulled = pull.map_or(0.0, |(_, pulled)| pulled);
        let gap = gap - pulled;
        if gap <= 0.0 {
            return None;
        }
        let effective_y = at.effective_y - pulled;
        // Chromium drops the margin after a box whose block-end edges run
        // over a page edge, so the box after it follows the last fragment
        // of the edges directly.
        if let Some(end) = (!forced_before && !follows_forced_break)
            .then(|| split_edges_end(document, cascade, page_origins, previous, effective_y - gap))
            .flatten()
            .filter(|_| self.adjacency.adjoins(document, cascade, node_id, previous))
        {
            return Some(MarginTruncation {
                delta: end - effective_y,
                underflows: false,
            });
        }
        let natural_page = run_end.map_or(at.natural_page, |end| {
            page_origins.start_page(effective_y, end.height)
        });
        let margin_node = run_end.map_or(node_id, |end| end.node_id);
        let page_top = page_origins.origin(natural_page);
        let previous_bottom = effective_y - gap;
        // A margin that ends right at a page edge holds no break there; the
        // first edge it crosses is one page up, when it reaches that far.
        let edge_page = if natural_page > 1
            && (effective_y - page_top).abs() <= 0.001
            && previous_bottom <= page_origins.origin(natural_page - 1) + 0.001
        {
            natural_page - 1
        } else {
            natural_page
        };
        let edge_top = page_origins.origin(edge_page);
        // The float landed where the margin it was placed after collapsed
        // to, so the box after it may already sit at the top of the page,
        // above the margin that the break keeps. Moving the box down moves
        // its lines with it, but they were broken against the floats where
        // the box was, so the box moves only when every line keeps the room
        // the floats leave it.
        let lands_at_page_top = taken_by_float
            && natural_page > 0
            && at.margin_top > 0.0
            && (effective_y - page_top).abs() <= 0.001
            && lines_stay_put_when_moved(
                document,
                cascade,
                node_id,
                document.pagination_local_y(node_id),
                at.margin_top,
                &preceding_float_extents(document, cascade, node_id),
            );
        if !(lands_at_page_top
            || page_edge_falls_in_margin(
                edge_page,
                edge_top,
                previous_bottom,
                effective_y,
                follows_forced_break,
            ))
            // Only a margin between adjoining siblings is truncated; a box
            // that is not a candidate may lie between two nested candidates.
            || !self.adjacency.adjoins(document, cascade, node_id, previous)
        {
            return None;
        }
        let mut target_y = page_origins.truncated_margin_target(
            float_break_page.unwrap_or(edge_page),
            forced_before,
            at.margin_top,
            || {
                collapsed_margin_start(
                    document,
                    cascade,
                    previous,
                    margin_node,
                    previous_bottom,
                    effective_y,
                )
            },
            follows_forced_break,
        );
        // An empty box of set height that the margins inside it pushed past
        // the page edge starts the page and keeps the margin below it.
        if !forced_before
            && !follows_forced_break
            && let Some((split, below)) = margin_split_box(
                document,
                cascade,
                previous,
                self.adjacency.preceding_siblings(document, node_id),
            )
        {
            // Both boxes are measured where the flow put them, without the
            // relative offsets that moved them afterwards.
            let flow_abs_y = |id: usize| {
                current_abs_y(document, id)
                    - accumulated_relative_offset_y(document, cascade, at.body_id, id)
            };
            let split_y = effective_y - (flow_abs_y(margin_node) - flow_abs_y(split));
            if split_y >= edge_top - 0.001 {
                let (positive, negative) =
                    adjoining_margin_extremes(document, cascade, margin_node, true, None);
                target_y = target_y.max(edge_top + below.0.max(positive) + below.1.min(negative));
            }
        }
        if let Some(page) = float_break_page {
            // A relayout may already place the box at the next edge. Keep
            // the margin from the float's forced boundary, not that edge,
            // and never above the page the float's break opens.
            target_y = target_y
                .min(page_origins.page_end(page))
                .max(page_origins.origin(page));
        }
        Some(MarginTruncation {
            delta: target_y - effective_y,
            underflows: forced_before && at.margin_top < 0.0,
        })
    }

    /// The block that ends the run of margins around the empty box `at`, with
    /// how far a negative margin on it pulls its top above the empty box, if
    /// it does. The empty box stands where that block starts: a page edge in
    /// the run cuts it from there. A forced break before the empty box, or one
    /// that reaches its margin, keeps the margin as it is.
    fn pulled_run_end(
        &self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        at: &MarginBreak,
    ) -> Option<(MarginRunEnd, f32)> {
        let end = at.run_end?;
        let pulled = at.raw_y - end.raw_y;
        if at.forced_before
            || pulled <= 0.001
            || !at.collapses_with_previous_block
            || !at.clearance_free
                && !matches!(
                    cascade.computed[at.node_id].clear,
                    raikiri_style::property::ClearValue::None
                )
        {
            return None;
        }
        let (previous, _) = self.gap_after_previous(document, cascade, at.node_id, at.raw_y)?;
        let taken_by_float = self
            .break_taken_by_float
            .is_some_and(|(source, _)| is_descendant_or_self(document, source, previous));
        (!taken_by_float && !self.follows_forced_break(document, cascade, previous, at.node_id))
            .then_some((end, pulled))
    }

    /// Whether a forced break reaches the margin above `node_id`, whose
    /// unpaginated top is `raw_y`: the margin between it and the sibling
    /// block before it still adjoins the latest forced break, or the break was
    /// taken to the new page by a float inside that sibling.
    fn forced_break_reaches(
        &self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        node_id: usize,
        raw_y: f32,
    ) -> bool {
        self.gap_after_previous(document, cascade, node_id, raw_y)
            .is_some_and(|(previous, _)| {
                self.follows_forced_break(document, cascade, previous, node_id)
                    || self.break_taken_by_float.is_some_and(|(source, _)| {
                        is_descendant_or_self(document, source, previous)
                    })
            })
    }

    /// Whether the margin between `previous` and `node_id` still adjoins
    /// the latest forced break.
    fn follows_forced_break(
        &self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        previous: usize,
        node_id: usize,
    ) -> bool {
        self.forced_break_box
            .is_some_and(|owner| forced_margin_reaches(document, cascade, owner, previous, node_id))
    }

    /// The previous in-flow sibling block of `node_id` and the positive gap
    /// between its unpaginated bottom and `raw_y`, the unpaginated top of
    /// `node_id`, if both are known and the gap is finite. Both are measured
    /// where the flow puts the boxes (see `flow_raw_y`). The previous block is
    /// the one recorded for the parent, or, when the box before `node_id` in
    /// the flow of its parent is a block that is not a candidate, that block:
    /// the page edge cuts the margin between the two also inside a block that
    /// no break goes before. A margin that the layout let through the bottom
    /// of the previous block and that no longer is leaves the gap (see
    /// [`escape_free_gap`]).
    fn gap_after_previous(
        &self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        node_id: usize,
        raw_y: f32,
    ) -> Option<(usize, f32)> {
        self.previous_and_gap(document, cascade, node_id, raw_y)
            .filter(|(_, gap)| gap.is_finite() && *gap > 0.0)
    }

    /// The previous block and the gap that [`Self::gap_after_previous`]
    /// finds, whatever the size of the gap.
    fn previous_and_gap(
        &self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        node_id: usize,
        raw_y: f32,
    ) -> Option<(usize, f32)> {
        let recorded = || {
            BoxTree::for_layout(document)
                .parent(node_id, BoxParentScope::Document)
                .and_then(|parent| self.previous_bottoms.get(&parent).copied())
        };
        self.preceding_bottoms
            .get(&node_id)
            .copied()
            .or_else(recorded)
            .map(|(previous, bottom)| {
                (
                    previous,
                    escape_free_gap(document, cascade, previous, raw_y - bottom),
                )
            })
    }

    /// Where the box `at` goes after `previous`, an empty box that a forced
    /// break took to the top of a page, or a box that ends with one, `gap`
    /// below it in the unpaginated flow. The layout collapsed the margins of
    /// the empty box with the bottom margin of the box before the break and
    /// left the box after it only what was over. Chromium truncates the
    /// margin before the break and keeps the margins after it whole below the
    /// page top: those of the empty boxes after the break, collapsed with the
    /// top margin of `at`, up to the next page edge, which cuts them.
    /// `None` when `previous` is not such a box, and `Some(None)` when the
    /// box is already where they put it.
    fn margin_after_forced_empty_box(
        &mut self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        page_origins: &PageOrigins,
        previous: usize,
        gap: f32,
        at: &MarginBreak,
    ) -> Option<Option<MarginTruncation>> {
        // An empty box after the break passes it on (see
        // `pass_on_forced_break`): its margins join the ones kept for the
        // box after it.
        let owner = self.forced_break_box?;
        let page_top = self.forced_page_top?;
        if at.forced_before
            || !gap.is_finite()
            || collapses_as_empty_box(document, cascade, at.node_id)
            || !collapses_as_empty_box(document, cascade, owner)
            || generated_box_separates(document, at.node_id, previous)
            || !self
                .adjacency
                .adjoins(document, cascade, at.node_id, previous)
        {
            return None;
        }
        // The block-end margins of the boxes that `owner` ends join it.
        let enclosing = if owner == previous {
            (0.0, 0.0)
        } else {
            ends_with_bottom_margin_of(document, cascade, previous, owner)?
        };
        let after = join_margin_extremes(
            enclosing,
            adjoining_margin_extremes(document, cascade, at.node_id, true, None),
        );
        // The empty boxes from the break to `owner`, the last one first.
        let mut run = vec![owner];
        for &sibling in self
            .adjacency
            .preceding_siblings(document, owner)
            .iter()
            .rev()
        {
            // Nodes that generate no box, such as collapsed white space, a
            // comment or a box with `display: none`, leave the margins
            // around them adjoining.
            match sibling_adjacency::flow_box(document, cascade, sibling) {
                sibling_adjacency::FlowBox::Skip | sibling_adjacency::FlowBox::Phantom => continue,
                sibling_adjacency::FlowBox::Box
                    if collapses_as_empty_box(document, cascade, sibling) =>
                {
                    run.push(sibling);
                }
                _ => break,
            }
        }
        // Their margins collapse with the one above `at`. A box that splits
        // the margin right after the break keeps the margins inside it above
        // itself instead, which add to the rest (see `splits_margin`).
        let (first, rest) = run.split_last()?;
        let mut kept = rest.iter().fold(after, |kept, &empty| {
            join_margin_extremes(kept, empty_box_margin_extremes(document, cascade, empty))
        });
        let (above_positive, above_negative) = if splits_margin(document, cascade, *first) {
            let own = document.nodes[*first].layout.margin.bottom;
            kept = join_margin_extremes(kept, (own.max(0.0), own.min(0.0)));
            adjoining_margin_extremes(document, cascade, *first, true, None)
        } else {
            kept = join_margin_extremes(kept, empty_box_margin_extremes(document, cascade, *first));
            (0.0, 0.0)
        };
        let target = page_top + kept.0 + kept.1 + above_positive + above_negative;
        // The next page edge is an unforced break, which truncates the
        // margins that reach past it.
        let next_top = page_origins.origin(page_origins.page_index_for_y(page_top) + 1);
        let target = if target > next_top + 0.001 {
            next_top
        } else {
            target
        };
        let delta = target - at.effective_y;
        Some((delta.abs() > 0.001).then_some(MarginTruncation {
            delta,
            // A negative margin pulls the box above the page top, and the
            // box stays on the page that the break opened.
            underflows: target < page_top - 0.001,
        }))
    }

    /// How far the flow after `node_id` moves on, an empty box that a forced
    /// break took to `landed_y` at the top of the page that starts at
    /// `page_top`, from `raw_y` in the unpaginated flow. The layout left the
    /// flow after the box what was over of the margin it collapsed with the
    /// bottom margin of the box before the break. Chromium truncates that
    /// bottom margin and keeps the margins of the empty box whole below the
    /// page top, up to `next_top`, the next page edge, which cuts them. Zero
    /// for any other box.
    fn forced_empty_box_shift(
        &mut self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        node_id: usize,
        raw_y: f32,
        landed_y: f32,
        (page_top, next_top): (f32, f32),
    ) -> f32 {
        self.forced_margins = None;
        if !collapses_as_empty_box(document, cascade, node_id) {
            return 0.0;
        }
        let Some((previous, gap)) = self
            .previous_and_gap(document, cascade, node_id, raw_y)
            .filter(|(_, gap)| gap.is_finite())
        else {
            return 0.0;
        };
        let own = empty_box_margin_extremes(document, cascade, node_id);
        let before = adjoining_margin_extremes(document, cascade, previous, false, None);
        let (positive, negative) = join_margin_extremes(before, own);
        // The layout put the flow after the box where the joined margin ends,
        // `gap` of it above the box. A gap wider than the margin holds
        // something else, such as a float the box went below.
        let left_after = positive + negative - gap;
        if left_after < -0.001 {
            return 0.0;
        }
        let kept = if splits_margin(document, cascade, node_id) {
            // A box that splits the margin sits below the margins inside it
            // and keeps its own block-end margin whole below it (see
            // `splits_margin`).
            let (inner_positive, inner_negative) =
                adjoining_margin_extremes(document, cascade, node_id, true, None);
            inner_positive + inner_negative + document.nodes[node_id].layout.margin.bottom
        } else {
            self.forced_margins = Some((own, (positive, negative)));
            own.0 + own.1
        };
        // The next page edge is an unforced break, which truncates the
        // margins that reach past it.
        (page_top + kept).min(next_top) - (landed_y + left_after)
    }

    /// Ends the run of adjoining siblings at the in-flow text `node_id`:
    /// text between two blocks separates them. The run is that of
    /// `ifc_root`, the inline layout root holding the text, or else that of
    /// the text's parent.
    fn end_run_at_text(&mut self, document: &Document, node_id: usize, ifc_root: Option<usize>) {
        if let Some(parent_id) = ifc_root
            .or_else(|| BoxTree::for_layout(document).parent(node_id, BoxParentScope::Document))
        {
            self.previous_bottoms.remove(&parent_id);
        }
    }

    /// Moves the forced break on to `node_id`, a child of `parent_id`, or
    /// ends it. An empty box right after the forced-break box passes the
    /// break on to the margin after it when its margins collapse through
    /// with that margin (`collapses_through`); any other box ends it.
    fn pass_on_forced_break(
        &mut self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        parent_id: usize,
        node_id: usize,
        collapses_through: bool,
    ) -> f32 {
        let follows_forced_box =
            self.previous_bottoms
                .get(&parent_id)
                .is_some_and(|&(previous, _)| {
                    self.follows_forced_break(document, cascade, previous, node_id)
                });
        let passes_on = follows_forced_box && collapses_through;
        self.forced_break_box = passes_on.then_some(node_id);
        // The margins of the box join those kept below the page top, and
        // the flow after it moves by as much more than the layout let it.
        let forced_margins = self.forced_margins.take();
        let Some((kept, laid_out)) = forced_margins.filter(|_| {
            passes_on
                && collapses_as_empty_box(document, cascade, node_id)
                && !splits_margin(document, cascade, node_id)
        }) else {
            return 0.0;
        };
        let own = empty_box_margin_extremes(document, cascade, node_id);
        let joined = (
            join_margin_extremes(kept, own),
            join_margin_extremes(laid_out, own),
        );
        self.forced_margins = Some(joined);
        let size = |(positive, negative): (f32, f32)| positive + negative;
        (size(joined.0) - size(kept)) - (size(joined.1) - size(laid_out))
    }

    /// Records `bottom`, the unpaginated border-box bottom of `node_id`, as
    /// the previous bottom for the next block-level child of `parent_id`
    /// when `node_id` is a block whose margins collapse with its siblings'
    /// (`collapses`). Any other box ends the run of adjoining siblings, so
    /// the previous bottom of `parent_id` is forgotten.
    fn record_bottom(&mut self, parent_id: usize, node_id: usize, collapses: bool, bottom: f32) {
        if collapses {
            self.previous_bottoms.insert(parent_id, (node_id, bottom));
        } else {
            self.previous_bottoms.remove(&parent_id);
        }
    }
}

/// The outermost box around `float` whose content ends with the forced
/// break after `source` and the floats after it, when its block-end edges
/// reach below `source` and still fit on the page that break leaves.
/// Chromium ends the box on that page and starts the next one with the float
/// beside the flow after the box. Edges that do not fit go to the next page
/// with the float.
fn edges_closed_before_float_break(
    document: &super::context::LayoutSession<'_>,
    page_origins: &PageOrigins,
    source: usize,
    float: usize,
) -> Option<usize> {
    let closer = *document.context.float_break_closers.get(&float)?;
    if !is_descendant_or_self(document, source, closer) {
        return None;
    }
    let source_end = current_abs_y(document, source) + document.nodes[source].layout.size.height;
    let end = current_abs_y(document, closer) + document.nodes[closer].layout.size.height;
    let page = page_origins.page_index_for_y(source_end - 0.001);
    (end > source_end + 0.001 && end <= page_origins.page_end(page) + 0.001).then_some(closer)
}

/// Shifts of the flow that end where a subtree does.
#[derive(Default)]
struct SubtreeShifts {
    /// Ancestors that could not hold a nested box that moved, with the part
    /// of the shift each did not absorb and the page the flow was on.
    ///
    /// A nested box that moved inside an ancestor whose height cannot grow
    /// to hold it overflows that ancestor: the part of the shift the
    /// ancestor did not absorb ends where its subtree does, and the page the
    /// flow was on is restored with it.
    bounded: Vec<(usize, f32, u32)>,
    /// Formatting-context roots that grew around a moved float, with the
    /// growth the flow after them keeps.
    growth: Vec<(usize, f32)>,
    /// Shifts that the flow takes from a child of a box on: each with the
    /// box, the index of the child among its children and the shift. The
    /// children before it keep their places, and the child and all that
    /// follows it move.
    from_box: Vec<(usize, usize, f32)>,
    /// Floats that the lines of their paragraph took down before the float
    /// became a candidate: the float starts where the lines put it.
    line_moved_floats: HashSet<usize>,
    /// Floats that went down to a later page on their own, with how far
    /// they moved past their place in the flow.
    floats_ahead: Vec<(usize, f32)>,
    /// Flex and grid containers with an item that grew while the items
    /// beside it kept their places.
    beside: Vec<BesideGrowth>,
    /// Whether the last [`SubtreeShifts::leave`] took the flow out of an
    /// item that a growth moved into one that it left in place.
    left_later_item: bool,
    /// Whether the last [`SubtreeShifts::leave`] reached a box that moved
    /// down below a float grown inside another float.
    reached_moved_box: bool,
    /// Table rows, or cells outside a table, that forced breaks in their
    /// cells grew, with how far.
    grown_rows: Vec<((usize, usize), f32)>,
    /// Table rows, or cells outside a table, that floats moved in their
    /// cells grew, with how far, and the cells with how far each grew to
    /// hold its floats. A cell's stored height takes that growth in, but a
    /// row's growth is measured from the height the cell was laid out with.
    float_rows: Vec<((usize, usize), f32)>,
    float_cells: Vec<(usize, f32)>,
    /// Boxes that forced breaks inside them grew, with how far in all:
    /// their stored heights leave that growth out.
    grown_boxes: Vec<(usize, f32)>,
    /// Cells beside one that a forced break grew, aligned again within the
    /// part of their row on the page the break leaves, with how far their
    /// content moved up (down when negative). Their content that is still
    /// to be placed moves with it.
    realigned_cells: Vec<(usize, f32)>,
    /// Cells that a page end or a forced break inside them split, which keep
    /// their content at their top.
    split_cells: HashSet<usize>,
}

/// A flex or grid container one of whose items grew because the closing
/// edges of a block in it moved to a later page. The items beside that one
/// stay where they are; those on the rows or lines after it move down with
/// the end of its row, and the flow after the container follows its growth.
struct BesideGrowth {
    container: usize,
    /// The items that begin below the grown one, on later rows or lines.
    later: HashSet<usize>,
    /// The items on the grown one's row or line, with whether each stretches
    /// to the height of the row and the relative offset that layout applied
    /// to it.
    row: Vec<(usize, bool, f32)>,
    /// How far the end of the grown item's row or line moved.
    later_shift: f32,
    /// The later item the flow is in, which carries `later_shift`.
    inside_later: Option<usize>,
    /// How much the container grew.
    growth: f32,
    /// The page the grown item ends on, which the flow after it is on.
    page: u32,
}

impl SubtreeShifts {
    /// Records that `id` grew `added` more, and returns how far it has
    /// grown in all.
    fn grow_box(&mut self, id: usize, added: f32) -> f32 {
        match self
            .grown_boxes
            .iter_mut()
            .find(|(grown_id, _)| *grown_id == id)
        {
            Some((_, grown)) => {
                *grown += added;
                *grown
            }
            None => {
                self.grown_boxes.push((id, added));
                added
            }
        }
    }

    /// How far `id` has grown in all (see [`Self::grow_box`]).
    fn box_growth(&self, id: usize) -> f32 {
        self.grown_boxes
            .iter()
            .find(|(grown_id, _)| *grown_id == id)
            .map_or(0.0, |&(_, grown)| grown)
    }

    /// How far `row` has grown in all, for forced breaks and for moved
    /// floats in its cells.
    fn row_growth(&self, row: (usize, usize)) -> f32 {
        let grown = |rows: &[((usize, usize), f32)]| {
            rows.iter()
                .find(|(id, _)| *id == row)
                .map_or(0.0, |&(_, grown)| grown)
        };
        grown(&self.grown_rows).max(grown(&self.float_rows))
    }

    /// How far the cell `cell` grew to hold the floats that moved in it.
    fn float_growth(&self, cell: usize) -> f32 {
        self.float_cells
            .iter()
            .find(|(id, _)| *id == cell)
            .map_or(0.0, |&(_, grown)| grown)
    }

    /// Records that the cell `cell` of `row` grew `growth` more to hold a
    /// float that moved in it, and returns how much more that grows the
    /// row.
    fn grow_row_for_float(&mut self, row: (usize, usize), cell: usize, growth: f32) -> f32 {
        let total = self.float_growth(cell) + growth;
        self.float_cells.retain(|(id, _)| *id != cell);
        self.float_cells.push((cell, total));
        let before = self.row_growth(row);
        match self.float_rows.iter_mut().find(|(id, _)| *id == row) {
            Some((_, grown)) => *grown = grown.max(total),
            None => self.float_rows.push((row, total)),
        }
        self.row_growth(row) - before
    }

    /// Records that a cell of `row` needs it `growth` taller than it was
    /// laid out for a forced break in it. `row` is the box that holds the
    /// row with the index after it, which tells the anonymous rows of one
    /// table apart.
    fn grow_row(&mut self, row: (usize, usize), growth: f32) {
        match self.grown_rows.iter_mut().find(|(id, _)| *id == row) {
            Some((_, grown)) => *grown = grown.max(growth),
            None => self.grown_rows.push((row, growth)),
        }
    }

    /// A box that moves down by `delta` takes the floats inside it along. A
    /// float there that already went down on its own keeps its place: it
    /// gives back as much of its own move as the box adds, so it does not
    /// move twice, and the formatting context that grew to hold its own move
    /// gives back that much of its growth too.
    fn take_back_floats_ahead(
        &mut self,
        document: &mut super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
        moved: usize,
        delta: f32,
    ) {
        if delta <= 0.0 {
            return;
        }
        let mut taken = Vec::new();
        for (float, ahead) in &mut self.floats_ahead {
            // A float inside another float moves with that one, which gives
            // back its own move.
            let inside = *float != moved
                && document
                    .layout_ancestors(*float)
                    .take_while(|&id| id != moved)
                    .all(|id| !is_floating_box_for_pagination(document, cascade, id))
                && is_descendant_or_self(document, *float, moved);
            if inside {
                let back = ahead.min(delta);
                document.translate_pagination_y(*float, -back);
                *ahead -= back;
                taken.push((*float, back));
            }
        }
        self.floats_ahead.retain(|&(_, ahead)| ahead > 0.001);
        for (float, back) in taken {
            let context = float_context_root(document, cascade, float, body_id);
            let Some(entry) = self
                .growth
                .iter_mut()
                .find(|(grown, growth)| *grown == context && *growth > 0.001)
            else {
                continue;
            };
            let shrink = back.min(entry.1);
            entry.1 -= shrink;
            document.nodes[context].layout.size.height -= shrink;
            document.mark_pagination_extent_dirty(context);
            shrink_ancestors_after_content_change(
                document,
                cascade,
                context,
                -shrink,
                -shrink,
                body_id,
                &IfcBoxShifts::default(),
                None,
            );
        }
    }

    /// How much of the shift of the flow inside the block `node_id` the
    /// block holds: its height kept it from growing that far, so the flow
    /// after it gives that much back once it leaves the block.
    fn held_by(&self, node_id: usize) -> f32 {
        self.bounded
            .iter()
            .filter(|&&(ancestor_id, _, _)| ancestor_id == node_id)
            .map(|&(_, delta, _)| delta)
            .sum()
    }

    /// End the shifts of the subtrees `node_id` is outside of, applying
    /// them to `flow_shift`, and return the page the flow returns to when a
    /// bounded subtree ends.
    ///
    /// Leaving several bounded subtrees at once returns to the page the
    /// earliest of them started from.
    fn leave(
        &mut self,
        document: &mut super::context::LayoutSession<'_>,
        node_id: usize,
        flow_shift: &mut f32,
    ) -> Option<u32> {
        let mut restored_page: Option<u32> = None;
        // Only the boxes around the float's paragraph move with its lines.
        self.floats_ahead.retain(|&(float, _)| {
            let paragraph = document
                .ifc_root_of(float)
                .or_else(|| document.layout_parent_of(float))
                .unwrap_or(float);
            is_descendant_or_self(document, node_id, paragraph)
        });
        self.growth.retain(|&(context, growth)| {
            if is_descendant_or_self(document, node_id, context) {
                return true;
            }
            *flow_shift += growth;
            false
        });
        let mut reached_moved_box = false;
        self.from_box.retain(|&(parent, from, shift)| {
            if reaches_child(document, node_id, parent, from) {
                reached_moved_box |= shift > 0.0;
                *flow_shift += shift;
                return false;
            }
            true
        });
        self.bounded.retain(|&(ancestor_id, delta, flow_page)| {
            if is_descendant_or_self(document, node_id, ancestor_id) {
                return true;
            }
            *flow_shift -= delta;
            restored_page = Some(restored_page.map_or(flow_page, |page| page.min(flow_page)));
            false
        });
        let mut beside_page = None;
        let mut left_later = false;
        let mut finished = Vec::new();
        self.beside.retain_mut(|beside| {
            let inside = is_descendant_or_self(document, node_id, beside.container);
            let item = inside
                .then(|| item_of(document, beside.container, node_id))
                .flatten()
                .filter(|item| beside.later.contains(item));
            if beside.inside_later != item {
                let shift = beside.later_shift;
                if beside.inside_later.take().is_some() {
                    *flow_shift -= shift;
                    if inside && item.is_none() {
                        left_later = true;
                    }
                }
                if let Some(item) = item {
                    beside.inside_later = Some(item);
                    *flow_shift += shift;
                    beside_page = beside_page.max(Some(beside.page));
                }
            }
            if inside {
                return true;
            }
            *flow_shift += beside.growth;
            if beside.growth > 0.001 {
                beside_page = beside_page.max(Some(beside.page));
            }
            finished.push((beside.container, std::mem::take(&mut beside.row)));
            false
        });
        for (container, row) in finished {
            stretch_row_items(document, container, &row);
        }
        // An item that the growth left in place, such as one in the next
        // column of a column flex container, is on the page it was on, which
        // the caller finds from where the flow now is.
        self.left_later_item = left_later;
        self.reached_moved_box = reached_moved_box;
        // The flow after a grown item is on the page the item ends on, past
        // the earlier page that a subtree it leaves goes back to.
        if beside_page.is_some() {
            restored_page = restored_page.max(beside_page);
        }
        restored_page
    }
}

impl SubtreeShifts {
    /// Whether the last [`SubtreeShifts::leave`] took the flow into an item
    /// that a growth left in place, clearing the answer.
    fn take_left_later_item(&mut self) -> bool {
        std::mem::take(&mut self.left_later_item)
    }

    /// Whether the last [`SubtreeShifts::leave`] reached a box that moved
    /// down below a grown float, clearing the answer.
    fn take_reached_moved_box(&mut self) -> bool {
        std::mem::take(&mut self.reached_moved_box)
    }

    /// Stretches the rows of the containers the flow has not left when it
    /// ends, as leaving them would (see [`stretch_row_items`]).
    fn finish_rows(&mut self, document: &mut super::context::LayoutSession<'_>) {
        for beside in self.beside.drain(..) {
            stretch_row_items(document, beside.container, &beside.row);
        }
    }
}

/// The item of the flex or grid `container` that holds `node_id`, which the
/// container lays out, through any `display: contents` boxes.
fn item_of(document: &Document, container: usize, node_id: usize) -> Option<usize> {
    document
        .inclusive_ancestors(node_id)
        .find(|&id| document.taffy_parent_of(id) == Some(container))
}

/// The bottom of the margin box of `item` in the coordinates of `container`.
fn item_bottom_in(document: &Document, container: usize, item: usize) -> f32 {
    let layout = &document.nodes[item].layout;
    pagination_child_parent_y(document, container, item, document.pagination_local_y(item))
        + layout.size.height
        + layout.margin.bottom
}

/// Grows the flex or grid `container` around its `item`, whose margin box
/// ended at `before` and now ends at `after` in the coordinates of the
/// container, while the items beside it keep their places (see
/// [`place_beside_growth`]). The items stretched to the row stretch with it
/// once the flow leaves the container. Its ancestors grow to hold it, and
/// `shifts` records the growth for the flow inside the container and after
/// it (see [`BesideGrowth`]); `page` is the page the item now ends on.
#[allow(clippy::too_many_arguments)]
fn grow_container_beside_item(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    container: usize,
    item: usize,
    ends: (f32, f32),
    shifts: &mut SubtreeShifts,
    (flow_page, page): (u32, u32),
) {
    // Grid areas are where layout put the items, before the growths that
    // moved them down.
    let moved = |id| -> f32 {
        shifts
            .beside
            .iter()
            .filter(|growth| growth.container == container && growth.later.contains(&id))
            .map(|growth| growth.later_shift)
            .sum()
    };
    let Some(placed) = place_beside_growth(
        document, cascade, body_id, container, item, ends, 0.0, moved,
    ) else {
        return;
    };
    if placed.growth > 0.001 {
        grow_ancestors_for_shift(
            document,
            cascade,
            container,
            placed.growth,
            body_id,
            shifts,
            flow_page,
            &IfcBoxShifts::default(),
        );
    }
    shifts.beside.push(BesideGrowth {
        container,
        later: placed.later,
        row: placed.row,
        later_shift: placed.later_shift,
        inside_later: None,
        growth: placed.growth,
        page,
    });
}

/// How [`place_beside_growth`] moved the items of a flex or grid container
/// after one of them grew.
struct BesidePlacement {
    /// The items that begin below the grown one, on later rows or lines.
    later: HashSet<usize>,
    /// The items on the grown one's row or line, with whether each stretches
    /// to the height of the row and the relative offset that layout applied
    /// to it.
    row: Vec<(usize, bool, f32)>,
    /// How far the end of the grown item's row or line moved.
    later_shift: f32,
    /// How much the container grew.
    growth: f32,
}

/// Moves the items of the flex or grid `container` around its `item`, whose
/// margin box ended at `before` and now ends at `after` in the coordinates of
/// the container, while the items beside it keep their places (CSS
/// Fragmentation 3 §4.4: the items that begin below the end of the item's
/// row or line move down with that end), and grows the container to hold
/// its content. `moved` tells how far earlier growths moved an item from the
/// grid area layout gave it, and `item_moved` how far the item itself moved
/// down whole with its growth. `None` when the item's rows have a set size
/// or the end of its row or line stays.
#[allow(clippy::too_many_arguments)]
fn place_beside_growth(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    container: usize,
    item: usize,
    (before, after): (f32, f32),
    item_moved: f32,
    moved: impl Fn(usize) -> f32,
) -> Option<BesidePlacement> {
    let mut later = HashSet::default();
    // Rows are found and measured in normal flow: a relative offset only moves
    // the paint of an item.
    let offset = |document: &super::context::LayoutSession<'_>, id| {
        layout_relative_offset_y(document, cascade, body_id, id)
    };
    let item_offset = offset(document, item);
    let (before, after) = (before - item_offset, after - item_offset);
    let mut row = vec![(
        item,
        stretches_across_row(document, cascade, container, item),
        item_offset,
    )];
    let mut row_end = before;
    let grid_area = grid_item_area(document, container, item);
    if let Some((area, area_end)) = grid_area {
        let area_end = area_end + moved(item);
        // Rows of a set size do not grow, and an item that still fits the
        // rows it spans, stretched or not, leaves them as they are.
        if !area.grows {
            return None;
        }
        row_end = row_end.max(area_end);
    }
    let mut later_end = f32::NEG_INFINITY;
    // Where the margin box of the grown item starts: the items that end
    // above it are on earlier rows or lines, which the growth leaves alone.
    let item_top =
        pagination_child_parent_y(document, container, item, document.pagination_local_y(item))
            - document.nodes[item].layout.margin.top
            - item_offset
            - item_moved;
    let mut placed = Vec::new();
    for child in document.children_through_contents(container) {
        let node = &document.nodes[child];
        let blank_text =
            matches!(&node.data, NodeData::Text(text) if text.text_content.trim().is_empty());
        if blank_text || node.is_display_none() || node.layout_position() == Position::Absolute {
            continue;
        }
        let child_offset = offset(document, child);
        let top = pagination_child_parent_y(
            document,
            container,
            child,
            document.pagination_local_y(child),
        ) - child_offset
            - node.layout.margin.top;
        let bottom = item_bottom_in(document, container, child) - child_offset;
        placed.push(PlacedItem {
            id: child,
            offset: child_offset,
            top,
            bottom,
        });
    }
    let same_line = items_on_line_of(document, cascade, body_id, container, item, &placed);
    let column = column_line_of(document, cascade, container, item, before, &placed);
    for child in placed {
        // The other columns of a multi-line column flex container sit beside
        // the grown item, and its growth moves only its own column.
        if child.id == item
            || column
                .as_ref()
                .is_some_and(|line| !line.contains(&child.id))
        {
            continue;
        }
        // A margin above an item on the same row or line can take it below
        // the end of the grown item; its margin box still starts on the row.
        // An item on the line that ends above the grown item neither ends
        // the row nor stretches across it.
        if !same_line.contains(&child.id) && child.top >= before - 0.001 {
            later.insert(child.id);
            // A stretched row ends below the item in it.
            let area_end = grid_item_area(document, container, child.id)
                .map_or(f32::NEG_INFINITY, |(_, end)| end + moved(child.id));
            later_end = later_end.max(child.bottom).max(area_end);
        } else if child.bottom > item_top + 0.001
            && (grid_area.is_none() || same_line.contains(&child.id))
        {
            // A grid item that spans other rows than the grown item ends
            // with them, not with its row.
            row_end = row_end.max(child.bottom);
            row.push((
                child.id,
                stretches_across_row(document, cascade, container, child.id),
                child.offset,
            ));
        }
    }
    let later_shift = (after - row_end).max(0.0);
    if later_shift <= 0.001 {
        return None;
    }
    for &child in &later {
        document.translate_pagination_y(child, later_shift);
    }
    let end_before = row_end.max(later_end);
    let end_after = row_end.max(after).max(later_end + later_shift);
    let layout = document.nodes[container].layout;
    let growth = if height_can_grow(document, container)
        && !document.nodes[container].size_containment.height
    {
        let (height, _) = grown_height(
            layout.size.height,
            border_box_height_bounds(document, container),
            layout.padding.bottom + layout.border.bottom,
            (end_before, end_after),
        );
        (height - layout.size.height).max(0.0)
    } else {
        0.0
    };
    if growth > 0.001 {
        document.nodes[container].layout.size.height += growth;
        document.mark_pagination_extent_dirty(container);
    }
    Some(BesidePlacement {
        later,
        row,
        later_shift,
        growth,
    })
}

/// The rows that `item` spans in the grid `container`, and where its grid
/// area ends from the top of the border box of the container.
fn grid_item_area(
    document: &super::context::LayoutSession<'_>,
    container: usize,
    item: usize,
) -> Option<(crate::node::GridItemRow, f32)> {
    let node = &document.nodes[container];
    let rows = *node.grid_item_rows.iter().find(|row| row.item == item)?;
    let (_, area) = node.grid_item_areas.iter().find(|&&(id, _)| id == item)?;
    Some((rows, rows.block_start + area.height))
}

/// An in-flow item of a flex or grid container, measured in normal flow
/// from the top of its margin box to the end of its margin box.
struct PlacedItem {
    id: usize,
    offset: f32,
    top: f32,
    bottom: f32,
}

/// The `placed` items on the flex line of `item` when `container` is a
/// multi-line column flex container. The items fill a line in order until one
/// starts over in the block axis beside an item already on it; the grown item
/// is measured by where its margin box ended before it grew (`before`), as the
/// items after it have not moved yet.
fn column_line_of(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    container: usize,
    item: usize,
    before: f32,
    placed: &[PlacedItem],
) -> Option<HashSet<usize>> {
    if !wraps_items_into_columns(document, cascade, container) {
        return None;
    }
    let mut ordered: Vec<&PlacedItem> = placed.iter().collect();
    ordered.sort_by_key(|child| cascade.computed[child.id].order);
    let span = |child: &PlacedItem| {
        let bottom = if child.id == item {
            before
        } else {
            child.bottom
        };
        (child.top, bottom)
    };
    let mut line: Vec<&PlacedItem> = Vec::new();
    let mut lines = Vec::new();
    for child in ordered {
        let (top, bottom) = span(child);
        let starts_over = line.iter().any(|&other| {
            let (other_top, other_bottom) = span(other);
            top < other_bottom - 0.001 && bottom > other_top + 0.001
        });
        if starts_over {
            lines.push(std::mem::take(&mut line));
        }
        line.push(child);
    }
    lines.push(line);
    lines
        .into_iter()
        .find(|line| line.iter().any(|child| child.id == item))
        .map(|line| line.into_iter().map(|child| child.id).collect())
}

/// Whether `container` is a horizontal multi-line column flex container,
/// which puts its items in columns side by side.
fn wraps_items_into_columns(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    container: usize,
) -> bool {
    let style = &cascade.computed[container];
    document.nodes[container].formatting_context == FormattingContext::Flex
        && !document.nodes[container].used_writing_mode.is_vertical()
        && !matches!(
            style.flex_direction,
            FlexDirectionValue::Row | FlexDirectionValue::RowReverse
        )
        && style.flex_wrap != FlexWrapValue::NoWrap
}

/// Which of the `placed` items of `container` are known to share the flex
/// line or grid row of `item`, whatever their place in the block axis: an
/// item aligned to the end of a tall line starts below the end of an item
/// aligned to its start. A single-line row flex container has one line, the
/// items of a multi-line one fill a line in order until one starts over at
/// the inline start of an item already on it, and grid items share the rows
/// of `item` when they span the same ones. The items the result does not claim are placed by where
/// their margin boxes are.
fn items_on_line_of(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    container: usize,
    item: usize,
    placed: &[PlacedItem],
) -> HashSet<usize> {
    let node = &document.nodes[container];
    let mut same: HashSet<usize> = HashSet::default();
    if row_flex_container(document, cascade, item) == Some(container) {
        if cascade.computed[container].flex_wrap == FlexWrapValue::NoWrap {
            same.extend(placed.iter().map(|child| child.id));
        } else {
            same.extend(flex_line_of(
                document,
                cascade,
                body_id,
                container,
                placed.iter().map(|child| child.id).collect(),
                item,
            ));
        }
    } else if let Some(rows) = node.grid_item_rows.iter().find(|row| row.item == item) {
        same.extend(
            node.grid_item_rows
                .iter()
                .filter(|other| (other.start, other.end) == (rows.start, rows.end))
                .map(|other| other.item),
        );
    }
    same
}

/// The items of `items`, children of the multi-line row flex container
/// `container`, that share the flex line of `item`. The items fill a line in
/// `order`, each one further along the main axis than the one before, until
/// one starts back before the end of the previous item: gaps and
/// `justify-content` can leave the items of two lines side by side without
/// overlapping, but never send an item of the same line backwards. Lines are
/// found in normal flow and in the inline axis, which pagination never
/// moves: a relative offset only moves the paint of an item.
fn flex_line_of(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    container: usize,
    mut items: Vec<usize>,
    item: usize,
) -> Vec<usize> {
    items.sort_by_key(|&id| cascade.computed[id].order);
    let computed = &cascade.computed[container];
    let forward = (computed.flex_direction == FlexDirectionValue::Row)
        == (computed.direction == raikiri_style::property::Direction::Ltr);
    let span = |id: usize| {
        let layout = &document.nodes[id].layout;
        let x = layout.location.x - layout_relative_offset(document, cascade, body_id, id).0;
        (
            x - layout.margin.left,
            x + layout.size.width + layout.margin.right,
        )
    };
    let mut line: Vec<usize> = Vec::new();
    let mut lines = Vec::new();
    for id in items {
        let (start, end) = span(id);
        let starts_over = line.last().is_some_and(|&previous| {
            let (previous_start, previous_end) = span(previous);
            if forward {
                start < previous_end - 0.001
            } else {
                end > previous_start + 0.001
            }
        });
        if starts_over {
            lines.push(std::mem::take(&mut line));
        }
        line.push(id);
    }
    lines.push(line);
    lines
        .into_iter()
        .find(|line| line.contains(&item))
        .unwrap_or_default()
}

/// Stretches the items of `row` that stretch to the height of their row or
/// line, in the flex or grid `container`, to the end of the row once the flow
/// has left the container: the items in it that grew for their closing edges
/// have all grown by then, so each stretched item grows only once. An item
/// stretches no further than its `max-height`.
fn stretch_row_items(
    document: &mut super::context::LayoutSession<'_>,
    container: usize,
    row: &[(usize, bool, f32)],
) {
    let end = row
        .iter()
        .map(|&(child, _, offset)| item_bottom_in(document, container, child) - offset)
        .fold(f32::NEG_INFINITY, f32::max);
    for &(child, stretches, offset) in row {
        let layout = document.nodes[child].layout;
        let room = end - (item_bottom_in(document, container, child) - offset);
        if !stretches || room <= 0.001 {
            continue;
        }
        let (_, max) = border_box_height_bounds(document, child);
        document.nodes[child].layout.size.height =
            (layout.size.height + room).min(max).max(layout.size.height);
        document.mark_pagination_extent_dirty(child);
    }
}

/// Whether `item` of the flex or grid `container` stretches to the height of
/// its row or line: the container lays its items out in rows, the item's
/// `align-self`, or the container's `align-items` where that is `auto`, is
/// `normal` or `stretch`, its height is `auto`, and neither of its vertical
/// margins is `auto` (CSS Box Alignment 3 §6.2, CSS Flexbox 1 §9.4).
fn stretches_across_row(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    container: usize,
    item: usize,
) -> bool {
    let stretches = |value| {
        matches!(
            value,
            SelfAlignmentValue::Normal | SelfAlignmentValue::Stretch
        )
    };
    let values = &cascade.computed[item];
    let aligned = (values.align_self == AlignSelfValue::Auto
        && stretches(cascade.computed[container].align_items))
        || matches!(values.align_self, AlignSelfValue::Value(value) if stretches(value));
    let block_axis = document.block_axis(item);
    aligned
        && (document.nodes[container].formatting_context == FormattingContext::Grid
            || row_flex_container(document, cascade, item) == Some(container))
        && block_axis.height().is_auto()
        && !block_axis.margin_top().is_auto()
        && !block_axis.margin_bottom().is_auto()
}

/// The used block-start margin of `node_id`, and the block-end margin that
/// travels with it when deciding whether it fits on its page. The block-end
/// margin of an in-flow box is truncated at the break after it (CSS
/// Fragmentation 3 §5.2), so it does not break the box and counts as zero.
/// A float's margin box stays whole.
fn break_margins(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    content_width: f32,
) -> (f32, f32) {
    let block_axis = document.block_axis(node_id);
    let computed = &cascade.computed[node_id];
    let margin_top = used_style_length_percentage_auto(block_axis.margin_top(), content_width)
        .or_else(|| used_computed_length_percentage_or_auto(&computed.margin.top, content_width))
        .unwrap_or(0.0);
    let trailing_margin = if is_floating_box_for_pagination(document, cascade, node_id) {
        used_style_length_percentage_auto(block_axis.margin_bottom(), content_width)
            .or_else(|| {
                used_computed_length_percentage_or_auto(&computed.margin.bottom, content_width)
            })
            .unwrap_or(0.0)
            .max(0.0)
    } else {
        0.0
    };
    (margin_top, trailing_margin)
}

/// Places the repeated header of the table that `row` belongs to, when no
/// page holds it yet: the header follows the flow shift so far, and the page
/// it lands on records the placement. The rows of a table whose body is all
/// in anonymous rows are not placed on their own, so the table itself places
/// its header.
fn place_first_table_header(
    document: &mut Document,
    table_headers: &mut HeaderRepeats,
    row: usize,
    flow_shift: f32,
    page_origins: &PageOrigins,
) {
    let Some(&header_id) = table_headers
        .rows
        .get(&row)
        .or_else(|| table_headers.anonymous_body_tables.get(&row))
    else {
        return;
    };
    let header = table_headers
        .headers
        .get_mut(&header_id)
        .expect("prepared header");
    if header.placements.is_empty() {
        let y = header.source_y + flow_shift;
        materialize_y(document, header_id, y);
        header.source_y = y;
        let page = page_origins.page_index_for_y(y);
        header
            .placements
            .insert(page, (page_origins.origin(page), y));
    }
}

/// Makes room at the top of `current_page` for a copy of the repeated header
/// of the table that `row`, which sits at `row_y` with `height`, belongs to.
/// The first body row also brings the first header to its page when the
/// header was left alone on an earlier one. Returns how far `row` moved down
/// below the header copy, which is zero when nothing moves.
///
/// When the table repeats its footer, a row that starts a later page than
/// the one the previous body row ended on closes that page with a copy of
/// the footer below the previous row.
fn make_room_for_table_header(
    document: &mut Document,
    table_headers: &mut HeaderRepeats,
    row: usize,
    row_y: f32,
    height: f32,
    current_page: u32,
    page_origins: &PageOrigins,
) -> f32 {
    let delta = make_room_for_header_copy(
        document,
        table_headers,
        row,
        row_y,
        height,
        current_page,
        page_origins,
    );
    if let Some(&footer_id) = table_headers.footer_rows.get(&row) {
        let footer = table_headers
            .headers
            .get_mut(&footer_id)
            .expect("prepared footer");
        if let Some((page, bottom)) = footer.last_row
            && page < current_page
        {
            footer
                .placements
                .entry(page)
                .or_insert((page_origins.origin(page), bottom + footer.gap));
        }
        let bottom = row_y + delta + height;
        let end_page = page_origins
            .page_index_for_y(bottom - 0.001)
            .max(current_page);
        footer.last_row = Some((end_page, bottom));
    }
    delta
}

/// Makes room at the top of `current_page` for a copy of the repeated header
/// of the table whose bottom caption `caption`, which sits at `caption_y` with
/// `height`, moved there alone. The caption goes below the copy and the closing
/// padding and border of the table. Returns how far the caption moved down,
/// which is zero when the page already holds the copy or cannot hold it with
/// the caption.
fn make_room_for_caption_header(
    document: &mut Document,
    table_headers: &mut HeaderRepeats,
    caption: usize,
    caption_y: f32,
    height: f32,
    current_page: u32,
    page_origins: &PageOrigins,
) -> f32 {
    let Some(&(header_id, closing)) = table_headers.tail_captions.get(&caption) else {
        return 0.0;
    };
    let header = table_headers
        .headers
        .get_mut(&header_id)
        .expect("prepared header");
    let copy = header.height + header.gap + closing;
    if header.placements.contains_key(&current_page)
        || copy + height > page_origins.step_at(current_page) + 0.001
    {
        return 0.0;
    }
    let origin = page_origins.origin(current_page);
    let delta = (origin + copy - caption_y).max(0.0);
    materialize_y(document, caption, caption_y + delta);
    header.placements.insert(current_page, (origin, origin));
    delta
}

/// The header part of [`make_room_for_table_header`].
fn make_room_for_header_copy(
    document: &mut Document,
    table_headers: &mut HeaderRepeats,
    row: usize,
    row_y: f32,
    height: f32,
    current_page: u32,
    page_origins: &PageOrigins,
) -> f32 {
    let Some(&header_id) = table_headers.rows.get(&row) else {
        return 0.0;
    };
    let header = table_headers
        .headers
        .get_mut(&header_id)
        .expect("prepared header");
    if !header.body_started {
        if header
            .placements
            .keys()
            .next()
            .is_some_and(|&page| page < current_page)
        {
            // The first header belongs with the first body row, even
            // when the remaining space cannot fit both intact.
            let origin = page_origins.origin(current_page);
            let table_y = current_abs_y(document, header.table);
            materialize_y(document, header.table, table_y + origin - header.source_y);
            materialize_y(document, header_id, origin);
            header.source_y = origin;
            header.placements.clear();
        }
        header.body_started = true;
    }
    if !header.placements.contains_key(&current_page)
        && header.height + header.gap + height <= page_origins.step_at(current_page) + 0.001
    {
        let origin = page_origins.origin(current_page);
        let delta = (origin + header.height + header.gap - row_y).max(0.0);
        materialize_y(document, row, row_y + delta);
        header.placements.insert(current_page, (origin, origin));
        return delta;
    }
    0.0
}

/// Places the source of the repeated footer `footer_id` below the body rows
/// placed before it, at its position moved by `flow_shift`, or at the top of
/// the next page when it would cross the page end. Returns how much further
/// down than `flow_shift` alone the footer went.
fn place_table_footer(
    document: &mut Document,
    table_headers: &mut HeaderRepeats,
    footer_id: usize,
    flow_shift: f32,
    page_origins: &PageOrigins,
) -> f32 {
    let footer = table_headers.headers.get_mut(&footer_id).expect("footer");
    footer.visit_shift = Some(flow_shift);
    let natural = footer.source_y + flow_shift;
    let after_rows = footer
        .last_row
        .map_or(f32::NEG_INFINITY, |(_, bottom)| bottom + footer.gap);
    let mut y = natural.max(after_rows);
    let mut page = page_origins.page_index_for_y(y + 0.001);
    let page_end = page_origins.origin(page) + page_origins.step_at(page);
    let next = page.saturating_add(1);
    let edges_move = footer.closing_edges > 0.0
        && y + footer.height <= page_end + 0.001
        && avoided_box_moves(
            y,
            0.0,
            y + footer.height + footer.closing_edges,
            page_origins.origin(page),
            page_end,
        );
    // A footer that starts a run of boxes tied by avoided breaks moves with
    // them when the run crosses the page end, as a block does.
    let run_moves = footer.tied_run.is_some_and(|(tail, firm)| {
        y + footer.height <= page_end + 0.001
            && (firm || footer.height + tail <= page_origins.step_at(next) + 0.001)
            && avoided_box_moves(
                y,
                0.0,
                y + footer.height + tail,
                page_origins.origin(page),
                page_end,
            )
    });
    if (y + footer.height > page_end + 0.001 || edges_move || run_moves)
        && footer.height <= page_origins.step_at(next) + 0.001
    {
        page = next;
        y = page_origins.origin(page);
        // A footer that moves with the boxes after it opens its page below
        // a copy of the table's header, as a row there would.
        if run_moves {
            let table = footer.table;
            let fits = footer.height;
            if let Some(header) = table_headers
                .headers
                .values_mut()
                .find(|header| !header.footer && header.table == table)
                .filter(|header| {
                    !header.placements.contains_key(&page)
                        && header.height + header.gap + fits <= page_origins.step_at(page) + 0.001
                })
            {
                header.placements.insert(page, (y, y));
                y += header.height + header.gap;
            }
        }
    }
    let footer = table_headers.headers.get_mut(&footer_id).expect("footer");
    materialize_y(document, footer_id, y);
    footer.source_y = y;
    footer
        .placements
        .insert(page, (page_origins.origin(page), y));
    (y - natural).max(0.0)
}

/// Places the repeated footers of the tables once pagination has placed
/// their rows, and the copies that a body box which does not break needs on
/// the pages it runs over.
///
/// A footer source that pagination did not reach keeps its offset below the
/// table's top (see [`place_table_footer`]).
/// A body box that runs past a page edge without breaking carries the
/// table onto the next page, so that page opens with a header copy over
/// the box, and the page it runs out of closes with a footer copy over the
/// box at the page's end.
fn place_repeated_table_footers(
    document: &mut Document,
    table_headers: &mut HeaderRepeats,
    page_origins: &PageOrigins,
) {
    let footers: Vec<usize> = table_headers
        .headers
        .iter()
        .filter(|(_, group)| group.footer && group.visit_shift.is_none())
        .map(|(&id, _)| id)
        .collect();
    // cov:ignore: pagination reaches every footer unless the page limit stops it first.
    for footer_id in footers {
        // Without a visit, the footer keeps its offset below the table's top.
        let footer = &table_headers.headers[&footer_id];
        let shift = current_abs_y(document, footer.table) - footer.table_source_y;
        place_table_footer(document, table_headers, footer_id, shift, page_origins);
    }
    for index in 0..table_headers.monolithic.len() {
        let (id, groups) = table_headers.monolithic[index];
        let top = current_abs_y(document, id);
        let bottom = top + document.nodes[id].layout.size.height;
        let first = page_origins.page_index_for_y(top + 0.001);
        let last = page_origins.page_index_for_y(bottom - 0.001);
        for group_id in groups.into_iter().flatten() {
            let group = table_headers.headers.get_mut(&group_id).expect("group");
            let pages = if group.footer {
                first..last
            } else {
                first.saturating_add(1)..last.saturating_add(1)
            };
            for page in pages {
                let origin = page_origins.origin(page);
                let y = if group.footer {
                    origin + page_origins.step_at(page) - group.height
                } else {
                    origin
                };
                group.placements.entry(page).or_insert((origin, y));
            }
        }
    }
}

/// The lines of a paragraph that move down inside it, away from a break
/// that would leave too few of them on one page.
struct LineMove {
    /// The paragraph root that holds the lines.
    block_id: usize,
    /// The paragraph's flex item and its container, when the paragraph is a
    /// flex item alone on its line.
    flex_line: Option<(usize, usize)>,
    /// The first line that moves.
    line: usize,
    /// How far the lines move down.
    shift: f32,
    /// The page the moved lines start on.
    page: u32,
    /// Whether the block child laid out right before the first moved line
    /// moves with the lines too (see [`blocks_after_line`]).
    with_block_before: bool,
}

/// How far each out-of-flow box of the paragraph `moved.block_id` moves down
/// when its lines from `moved.line` on move by `moved.shift` and its box grows
/// by `growth`. A box that a fixed `top` places stays; one at the static
/// position of a line that moved goes with the line; one placed by a `bottom`
/// or a percentage `top` follows the end of the paragraph's padding box, which
/// is its containing block.
fn out_of_flow_moves(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    moved: &LineMove,
    growth: f32,
) -> Vec<(usize, f32)> {
    let root = moved.block_id;
    document
        .ifc_layout_node(root)
        .and_then(|node| Some((node, node.ifc.as_ref()?)))
        .into_iter()
        .flat_map(|(node, ifc)| {
            let height =
                node.layout.size.height - node.layout.border.top - node.layout.border.bottom;
            // The anchors are looked up once for all the boxes at a static
            // position, and a box listed twice moves once.
            let anchors = std::cell::OnceCell::new();
            let mut seen = HashSet::new();
            ifc.boxes
                .iter()
                .filter(|b| b.kind == IfcBoxKind::OutOfFlow)
                .filter_map(move |b| {
                    if !seen.insert(b.node) {
                        return None;
                    }
                    let delta = match out_of_flow_follow(document, cascade, root, b.node)? {
                        OutOfFlowFollow::TopAnchored | OutOfFlowFollow::DeferredAncestorHeight => {
                            return None;
                        }
                        OutOfFlowFollow::OnLine => {
                            let anchors = anchors.get_or_init(|| {
                                ifc.lines
                                    .as_ref()
                                    .map(|lines| first_anchors(document, lines))
                                    .unwrap_or_default()
                            });
                            let (line, ..) = anchors.get(&b.node)?;
                            (*line >= moved.line).then_some(moved.shift)?
                        }
                        OutOfFlowFollow::WithHeight => {
                            let inset = document.physical_style(b.node).inset();
                            with_height_delta(inset.top, inset.bottom, height, growth)
                        }
                    };
                    Some((b.node, delta))
                })
        })
        .collect()
}

/// How far an out-of-flow box placed by `top` or `bottom` (the other `auto`)
/// against a padding box `height` tall moves down when that box grows by
/// `growth`: a `top` moves by what its percentage of the taller box adds, and
/// a `bottom` keeps its distance to the new bottom edge, so the box moves by
/// the growth less what its percentage of the taller box adds.
fn with_height_delta(
    top: LengthPercentageAuto,
    bottom: LengthPercentageAuto,
    height: f32,
    growth: f32,
) -> f32 {
    let at = |length: LengthPercentageAuto, height: f32| {
        length.maybe_resolve(Some(height)).unwrap_or(0.0)
    };
    if top.is_auto() {
        growth - (at(bottom, height + growth) - at(bottom, height))
    } else {
        at(top, height + growth) - at(top, height)
    }
}

/// Moves the lines of the paragraph `moved.block_id` from `moved.line` on down
/// by `moved.shift`, together with the out-of-flow boxes of the paragraph that
/// follow them (see [`out_of_flow_moves`]), the floats that sit on them (see
/// [`floats_on_moved_lines`]) and the block children that come after them
/// (see [`blocks_after_line`]), and grows the paragraph's box to hold them.
/// Returns how far the box grew and the floats that moved.
fn shift_paragraph_lines(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    moved: &LineMove,
    went_ahead: Option<&[usize]>,
) -> (f32, Vec<usize>) {
    let growth = lines_shift_growth(document, moved.block_id, moved.shift);
    let follows = out_of_flow_moves(document, cascade, moved, growth);
    // Atomic boxes keep their positions on the shifted lines, whether a
    // text candidate or a delegated atomic-only line owns the move.
    let atomics: HashSet<usize> = document
        .ifc_layout_node(moved.block_id)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .map(|lines| atomic_nodes(lines.lines.get(moved.line..).unwrap_or_default()))
        .unwrap_or_default()
        .into_iter()
        .filter(|&node| node < document.nodes.len())
        .collect();
    let blocks = blocks_after_line(
        document,
        moved.block_id,
        moved.line,
        moved.with_block_before,
    );
    let floats = floats_on_moved_lines(document, moved, went_ahead);
    shift_ifc_lines_from(document, moved.block_id, moved.line, moved.shift, growth);
    for node in atomics
        .into_iter()
        .chain(blocks)
        .chain(floats.iter().copied())
    {
        document.translate_pagination_y(node, moved.shift);
    }
    for (node, delta) in follows {
        document.translate_pagination_y(node, delta);
    }
    (growth, floats)
}

/// The floats of the paragraph `moved.block_id` that move down with its lines
/// from `moved.line` on: those whose margin box starts on or below the first
/// of them. A float is placed on the line it comes in, or lower, so it starts
/// the next page with that line (Chromium); one placed beside an earlier line
/// stays. A float that went to the next page by itself stays too: one of
/// `went_ahead`, or, when the caller does not know those, any float that
/// already starts at or below where the line goes. Positions are
/// compared without the offsets of relatively positioned inlines. Lines that move up
/// leave the floats to their caller.
fn floats_on_moved_lines(
    document: &Document,
    moved: &LineMove,
    went_ahead: Option<&[usize]>,
) -> Vec<usize> {
    let node = document
        .ifc_layout_node(moved.block_id)
        .filter(|_| moved.shift > 0.0);
    let Some((node, ifc, lines)) = node.and_then(|node| {
        let ifc = node.ifc.as_ref()?;
        let lines = ifc
            .lines
            .as_ref()
            .filter(|lines| moved.line < lines.lines.len())?;
        Some((node, ifc, lines))
    }) else {
        return Vec::new();
    };
    let top = node.layout.border.top + node.layout.padding.top + lines.line_top(moved.line);
    let moved_top = top + moved.shift;
    ifc.boxes
        .iter()
        .filter(|b| b.kind == IfcBoxKind::Float)
        .map(|b| b.node)
        .filter(|&float| {
            // A relatively positioned inline around the float offsets it from
            // the line it was placed on.
            let relative: f32 = document
                .ancestors(float)
                .take_while(|&id| id != moved.block_id)
                .filter_map(|id| ifc.relative_offset(id))
                .map(|(_, y)| y)
                .sum();
            let margin_top = document.pagination_local_y(float)
                - relative
                - document.nodes[float].layout.margin.top;
            margin_top >= top - 0.01
                && went_ahead.map_or(margin_top < moved_top - 0.01, |ahead| {
                    !ahead.contains(&float)
                })
        })
        .collect()
}

/// Moves the lines of a paragraph from `moved.line` on down by `moved.shift`
/// and brings along what has to follow them: the flex lines after a flex
/// item paragraph, and the ancestors that hold the paragraph, which grow to
/// keep it. Returns how far the paragraph's box grew.
fn move_paragraph_lines(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    moved: &LineMove,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
) -> f32 {
    // A float an earlier walk moved whole to a later page stands where it
    // went, as one this walk moves does.
    let went_ahead: Vec<usize> = shifts
        .floats_ahead
        .iter()
        .map(|&(float, _)| float)
        .chain(document.context.float_resumptions.pushed_floats())
        .collect();
    let cell_lines = cell_flow_lines(document, cascade, body_id, moved);
    let height = document.nodes[moved.block_id].layout.size.height;
    let (growth, floats) = shift_paragraph_lines(document, cascade, moved, Some(&went_ahead));
    if let Some((item, container)) = moved.flex_line {
        shift_later_flex_lines(document, container, item, growth);
    }
    // A box that keeps its height holds the moved lines as overflow: nothing
    // around it moves.
    let beside = edges_break_container(document, moved.block_id, body_id).flatten();
    if let Some((inline, end)) = cell_lines {
        // Lines that move down in a table cell grow its row, as content that
        // a forced break moves there does; the row measures the paragraph
        // from its height before the lines moved. The flow after the
        // paragraph takes its growth alone, as it does after one in block
        // flow.
        document
            .table_layout_node_mut(moved.block_id)
            .layout
            .size
            .height = height;
        shift_flow_after(
            document,
            cascade,
            inline,
            Some(end),
            body_id,
            growth,
            shifts,
            flow_page,
        );
    } else if growth > 0.0
        && let Some(container) = beside
    {
        let item = item_of(document, container, moved.block_id).unwrap_or(moved.block_id);
        // The paragraph already grew with its lines; an item around it grows
        // below.
        let grown_already = if item == moved.block_id { growth } else { 0.0 };
        let before = item_bottom_in(document, container, item) - grown_already;
        let (_, passed) = grow_ancestors_below(
            document,
            cascade,
            moved.block_id,
            growth,
            body_id,
            shifts,
            flow_page,
            &IfcBoxShifts::default(),
            Some(container),
            true,
        );
        if passed > 0.001 {
            shifts.bounded.push((item, passed, flow_page));
        }
        let after = item_bottom_in(document, container, item);
        grow_container_beside_item(
            document,
            cascade,
            body_id,
            container,
            item,
            (before, after),
            shifts,
            (flow_page, moved.page),
        );
    } else if growth > 0.0 {
        grow_ancestors_for_line_move(
            document,
            cascade,
            moved.block_id,
            growth,
            body_id,
            shifts,
            flow_page,
            &IfcBoxShifts::default(),
        );
    } else if growth < 0.0 {
        // Lines that move up, to the top of the page whose edge truncated
        // the margin above them, shorten the boxes around the paragraph as a
        // truncated margin above a box does.
        let held = shrink_ancestors_after_margin_truncation(
            document,
            cascade,
            moved.block_id,
            growth,
            body_id,
            &IfcBoxShifts::default(),
        );
        shifts.bounded.extend(
            held.into_iter()
                .map(|(ancestor, held)| (ancestor, held, flow_page)),
        );
    }
    // A float that moved past the end of a box that keeps its height still
    // has its formatting context hold it.
    for float in floats {
        let height = document.nodes[float].layout.size.height;
        grow_float_context_to_moved_float(
            document, cascade, float, body_id, height, shifts, flow_page,
        );
        shifts.line_moved_floats.insert(float);
    }
    growth
}

/// For lines of a paragraph in a table cell that move down (see
/// [`cell_flow_inline`]), the inline child of the paragraph from which the
/// walk to the cell's row starts, and how far below the top of the cell the
/// first moved line starts once it moves: the top of the next page, where
/// the row's fragment on the page the lines leave ends.
fn cell_flow_lines(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    moved: &LineMove,
) -> Option<(usize, f32)> {
    let root = moved.block_id;
    if moved.shift <= 0.0 || moved.flex_line.is_some() {
        return None;
    }
    let inline = cell_flow_inline(document, cascade, root, body_id)?;
    let cell = document
        .inclusive_ancestors(root)
        .find(|&id| document.nodes[id].display() == DisplayValue::TableCell)?;
    let node = document.ifc_layout_node(root)?;
    let line_top = node.ifc.as_ref()?.lines.as_ref()?.line_top(moved.line);
    Some((
        inline,
        offset_in_cell(document, cascade, body_id, root, cell)
            + node.layout.border.top
            + node.layout.padding.top
            + line_top
            + moved.shift,
    ))
}

/// How far below the top of `moved` the block `block` starts in the normal
/// flow, when `moved` is the box that the walk from the block up chose to move
/// with it (see [`first_box_ancestor_at_start`]): the edges of the ancestors
/// above it, without the relative offsets that moved either box. It is
/// negative only by the negative margins that boxes on the way keep inside
/// the box above them, which puts the block above the top of `moved`.
fn lead_below_ancestor(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    block: usize,
    moved: usize,
) -> f32 {
    flow_distance(document, cascade, body_id, block, moved)
        .max(kept_negative_margins(document, cascade, block, moved))
}

/// How far below `moved` the flow puts `block`, without the relative offsets
/// that moved either box.
fn flow_distance(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    block: usize,
    moved: usize,
) -> f32 {
    let flow_y = |id: usize| {
        current_abs_y(document, id) - accumulated_relative_offset_y(document, cascade, body_id, id)
    };
    flow_y(block) - flow_y(moved)
}

/// The sum of the negative top margins that the boxes from `block` up to
/// `moved` keep inside a parent with a top border or padding, or one whose
/// top their margins do not collapse through (CSS 2 §8.3.1). Each is the most
/// negative of the margins that collapse together above the child.
fn kept_negative_margins(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    moved: usize,
) -> f32 {
    let mut kept = 0.0;
    let mut child = block;
    for parent in document.layout_ancestors(block) {
        if child == moved {
            break;
        }
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        let layout = &document.nodes[parent].layout;
        if layout.border.top != 0.0
            || layout.padding.top != 0.0
            || !margins_may_collapse_through(document, cascade, parent)
        {
            kept += adjoining_margin_extremes(document, cascade, child, true, None).1;
        }
        child = parent;
    }
    kept
}

/// Moves `block_id`, which the flow puts at `from_y`, so that the box it
/// moves with starts at `target_y` in the flow, and returns how far it moved.
/// Both are measured without the relative offsets of the block and its
/// ancestors, which move along with it. A `nested` block that begins its ancestors takes the
/// ancestors along rather than leaving them empty on this page, or leaving
/// their padding and border alone there when they start below `page_start`
/// (see [`first_box_ancestor_at_start`]); the ancestors above them grow to hold
/// the move, recorded in `shifts` with `flow_page` as for
/// [`grow_ancestors_for_shift`].
#[allow(clippy::too_many_arguments)]
fn move_line_block(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    block_id: usize,
    nested: bool,
    from_y: f32,
    target_y: f32,
    page_start: f32,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
    gaps: &mut external_float_gap::Index,
) -> (f32, Vec<external_float_gap::Removal>) {
    let moved = if nested {
        first_box_ancestor_at_start(document, cascade, block_id, body_id, page_start)
    } else {
        block_id
    };
    let lead = lead_below_ancestor(document, cascade, body_id, block_id, moved);
    // A negative or undefined move leaves the block where it is.
    let delta = Some(target_y - (from_y - lead))
        .filter(|delta| delta.is_finite() && *delta > 0.0)
        .unwrap_or(0.0);
    let consumed = gaps.discard_in_holder(document, cascade, moved, delta, body_id);
    if delta > 0.0 {
        let moved_y = current_abs_y(document, moved) + delta;
        let followed_ifc_boxes = materialize_y(document, moved, moved_y);
        shifts.take_back_floats_ahead(document, cascade, body_id, moved, delta);
        grow_ancestors_for_line_move(
            document,
            cascade,
            moved,
            delta,
            body_id,
            shifts,
            flow_page,
            &followed_ifc_boxes,
        );
    }
    (delta, consumed)
}

/// A validated first-child forced boundary that carries its plain holder.
#[derive(Clone, Copy)]
struct ForcedBreakHolder {
    node: usize,
    lead: f32,
    margin_top: f32,
    direct_body: bool,
}

/// How far a page transition moved a box and why.
struct TransitionMove {
    /// How far the box itself moves down.
    node_delta: f32,
    /// Whether the box moves because it would cross a page edge it avoids.
    page_overflow: bool,
    /// Whether a forced or page-type break moves the box.
    forced_transition: bool,
    /// The box that moves down, which is the candidate or the outermost
    /// ancestor that starts with it (see [`leading_holder`]).
    holder: usize,
    /// The explicitly forced first-child boundary that chose this holder.
    forced_holder: Option<ForcedBreakHolder>,
}

/// Moves the box `candidate` down by `moved.node_delta` for a page
/// transition and brings along what has to follow it: the lines of its
/// paragraph, and the ancestors or later siblings that hold or trail it.
/// Where the candidate begins an ancestor that moves to the next page with it
/// (`moved.holder`), the ancestor moves instead of the candidate, and takes
/// the candidate along. Returns whether an ancestor's height kept it from
/// holding the moved content.
fn move_box_for_transition(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    candidate: &PageCandidate,
    moved: &TransitionMove,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
) -> bool {
    let node_id = candidate.node_id;
    let TransitionMove {
        node_delta,
        page_overflow,
        forced_transition,
        holder,
        forced_holder,
    } = *moved;
    debug_assert!(
        is_descendant_or_self(document, node_id, holder),
        "the box that moves holds the candidate"
    );
    debug_assert!(
        holder == node_id || forced_holder.is_some() || (page_overflow && !forced_transition),
        "an ancestor moves for a page edge or its first child forced boundary"
    );
    document.translate_pagination_y(holder, node_delta);
    if holder == node_id
        && node_delta > 0.0
        && is_floating_box_for_pagination(document, cascade, node_id)
    {
        shifts.floats_ahead.push((node_id, node_delta));
    } else {
        shifts.take_back_floats_ahead(document, cascade, body_id, holder, node_delta);
    }
    let followed_ifc_boxes = follow_moved_ifc_block(document, holder, node_delta);
    // A nested avoiding block that moves to the next page lengthens the
    // ancestors whose content box holds it, so their continuation (and any
    // `overflow` clip) reaches the moved content. A float does not lengthen
    // the block around it; its own shift ends with its subtree.
    let mut moved_in_bounded_ancestor = false;
    let moves_in_flow_down =
        node_delta > 0.0 && !is_floating_box_for_pagination(document, cascade, node_id);
    let direct_body = forced_holder.map_or(candidate.context.is_direct_body_element(), |owner| {
        owner.direct_body
    });
    let nested_block = !direct_body && moves_in_flow_down;
    if page_overflow && nested_block && moves_reach_cell_row(document, holder) {
        // A box moved out of a page in a table cell takes the content after
        // it along, and its row grows to hold it, as for a forced break
        // there; the cells beside it stay.
        moved_in_bounded_ancestor = shift_block_flow_after(
            document, cascade, holder, body_id, node_delta, shifts, flow_page,
        );
    } else if (page_overflow || (forced_transition && direct_body)) && moves_in_flow_down {
        moved_in_bounded_ancestor = grow_ancestors_for_shift(
            document,
            cascade,
            holder,
            node_delta,
            body_id,
            shifts,
            flow_page,
            &followed_ifc_boxes,
        );
    } else if forced_transition
        && nested_block
        && !candidate.context.is_flex_item()
        && !candidate.context.is_grid_item()
        && !candidate.context.is_table_row()
    {
        // A forced break ends its ancestors' fragments on this page, which
        // their continuations account for; only the boxes after it in block
        // flow follow it, up to the first ancestor that does not stack its
        // children in block order. They follow a box that a negative margin
        // pulls back over the break too.
        moved_in_bounded_ancestor = shift_block_flow_after(
            document, cascade, holder, body_id, node_delta, shifts, flow_page,
        );
    }
    moved_in_bounded_ancestor
}

/// Grows the formatting context that holds a moved float. A formatting
/// context of its own holds its floats (CSS 2.1 §10.6.7), so an auto-height
/// one grows to the moved float's margin box, and the flow after it moves
/// down by the growth.
fn grow_float_context_to_moved_float(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    body_id: usize,
    height: f32,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
) {
    let context = float_context_root(document, cascade, node_id, body_id);
    // A table cell's `height` is a minimum: it grows with its content
    // whatever its value (CSS 2.1 §17.5.3).
    let cell = document.nodes[context].display() == DisplayValue::TableCell;
    if context != body_id
        && (cell
            || document.block_axis(context).height().is_auto()
            || definite_content_height(document, context).is_none())
    {
        let context_top = current_abs_y(document, context);
        let float_bottom = current_abs_y(document, node_id)
            + height
            + document.nodes[node_id].layout.margin.bottom;
        let layout = &document.nodes[context].layout;
        let grown = (float_bottom - context_top + edges_below(document, context))
            .min(max_border_box_height(document, cascade, context).unwrap_or(f32::INFINITY));
        let growth = grown - layout.size.height;
        if growth > 0.001 {
            document.nodes[context].layout.size.height += growth;
            document.mark_pagination_extent_dirty(context);
            // A row grows as far as its cell that grows most: only the part
            // of a cell's growth past what the row grew already moves the
            // boxes after the row. The cells beside it sit beside it, not
            // below it, so the flow follows past the row.
            let holder = cell.then(|| cell_growth_holder(document, context));
            let growth = match holder {
                Some(holder) => shifts.grow_row_for_float(holder, context, growth),
                None => growth,
            };
            if growth <= 0.001 {
                return;
            }
            grow_ancestors_for_shift(
                document,
                cascade,
                context,
                growth,
                body_id,
                shifts,
                flow_page,
                &IfcBoxShifts::default(),
            );
            match holder {
                Some((row, end)) if end > 0 => shifts.from_box.push((row, end, growth)),
                _ => shifts.growth.push((context, growth)),
            }
        }
    }
}

/// Grows the auto-height ancestors of `node_id`, which moved down by
/// `delta`, to hold it. The part of the shift an ancestor does not pass on
/// to the flow after it, because its height cannot grow or a float in it
/// already reaches below the moved content, is recorded in the bounded
/// shifts of `shifts` with `flow_page`, the page the flow returns to past
/// that ancestor. An ancestor that grows more than the flow inside it moved
/// is recorded in the growth of `shifts`, for the flow after it to follow.
/// Returns whether an ancestor's height kept it from holding the moved
/// content.
#[allow(clippy::too_many_arguments)]
fn grow_ancestors_for_shift(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    delta: f32,
    body_id: usize,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
    ifc_boxes: &IfcBoxShifts,
) -> bool {
    grow_ancestors_below(
        document, cascade, node_id, delta, body_id, shifts, flow_page, ifc_boxes, None, false,
    )
    .0
}

/// [`grow_ancestors_for_shift`] for lines that the paragraph line pass moved,
/// or the block it moved with them: a float around them grows to hold them
/// and leaves the flow after it where it is.
#[allow(clippy::too_many_arguments)]
fn grow_ancestors_for_line_move(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    delta: f32,
    body_id: usize,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
    ifc_boxes: &IfcBoxShifts,
) -> bool {
    grow_ancestors_below(
        document, cascade, node_id, delta, body_id, shifts, flow_page, ifc_boxes, None, true,
    )
    .0
}

/// Like [`grow_ancestors_for_shift`], but stops below `stop` when it is an
/// ancestor of `node_id`, leaving it and the boxes above it as they are.
/// Also returns the shift that reaches `stop`: how far the flow after its
/// child moved, or nothing when the walk ended before it.
#[allow(clippy::too_many_arguments)]
fn grow_ancestors_below(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    delta: f32,
    body_id: usize,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
    ifc_boxes: &IfcBoxShifts,
    stop: Option<usize>,
    floats_hold_growth: bool,
) -> (bool, f32) {
    grow_ancestors_and_followers(
        document,
        cascade,
        node_id,
        delta,
        body_id,
        shifts,
        flow_page,
        ifc_boxes,
        stop,
        floats_hold_growth,
        None,
    )
}

/// [`grow_ancestors_below`], with `scanned` the float whose later siblings
/// were already searched for the boxes below it.
#[allow(clippy::too_many_arguments)]
fn grow_ancestors_and_followers(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    delta: f32,
    body_id: usize,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
    ifc_boxes: &IfcBoxShifts,
    stop: Option<usize>,
    floats_hold_growth: bool,
    scanned: Option<usize>,
) -> (bool, f32) {
    let mut followers = Vec::new();
    let (mut bounded, passed) = grow_ancestors_walk(
        document,
        cascade,
        node_id,
        delta,
        body_id,
        shifts,
        flow_page,
        ifc_boxes,
        (stop, scanned),
        floats_hold_growth,
        &mut followers,
    );
    // A float below a float that grew moves down with it, and its own
    // ancestors grow to hold it. The flow takes the move from the moved
    // float on, and gives it back past it.
    for (parent, follower, shift, is_float) in followers {
        document.translate_pagination_y(follower, shift);
        let index = child_index(document, parent, follower);
        shifts.from_box.push((parent, index, shift));
        if !is_float {
            // A box that clears the float from after a block around it
            // moves in the block's parent, and the boxes after it follow.
            bounded |= grow_ancestors_below(
                document,
                cascade,
                follower,
                shift,
                body_id,
                shifts,
                flow_page,
                &IfcBoxShifts::default(),
                stop,
                true,
            )
            .0;
            continue;
        }
        shifts.from_box.push((parent, index + 1, -shift));
        let before = shifts.bounded.len();
        // The boxes after the moved float were searched with the float
        // that grew.
        bounded |= grow_ancestors_and_followers(
            document,
            cascade,
            follower,
            shift,
            body_id,
            shifts,
            flow_page,
            &IfcBoxShifts::default(),
            stop,
            true,
            Some(follower),
        )
        .0;
        // The flow past the moved float gives the move back from the box
        // after it on, not from where the walk is now.
        let mut index = 0;
        shifts.bounded.retain(|&(id, _, _)| {
            index += 1;
            index <= before || id != follower
        });
    }
    (bounded, passed)
}

/// The walk of [`grow_ancestors_below`]. `limits` holds `stop` and the
/// float whose later siblings were already searched. `followers` collects
/// the boxes below a float that grew inside another float that move once
/// the walk is done: each with its parent, how far it moves and whether it
/// is a float (or a box after the float's block that clears it).
#[allow(clippy::too_many_arguments)]
fn grow_ancestors_walk(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    delta: f32,
    body_id: usize,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
    ifc_boxes: &IfcBoxShifts,
    (stop, scanned): (Option<usize>, Option<usize>),
    floats_hold_growth: bool,
    followers: &mut Vec<(usize, usize, f32, bool)>,
) -> (bool, f32) {
    let mut bounded = false;
    // The box of the parent after which its boxes move, when it is not the
    // child the walk came from.
    let mut shifted_after = None;
    let mut child = node_id;
    // How far the flow after `child` moved, and how much `child` moved or
    // grew: a float hanging below its content can take up part of the move.
    let mut flow_delta = delta;
    let mut size_delta = delta;
    let mut float_delta = delta;
    // The box that a negative margin starting to collapse through its bottom
    // pulls the flow after up past, and by how far (see
    // `GrownHeight::margin_shift`).
    let mut pulled = None;
    let mut walk = AncestorCursor::new(document.parent_of(child), ParentRelation::Source);
    while let Some(parent) = walk.current() {
        if Some(parent) == stop {
            return (bounded, flow_delta);
        }
        if document.nodes[parent].kind() != NodeKind::Element {
            break;
        }
        walk.advance(document);
        // A `display: contents` element has no box to grow; its parent lays
        // the moved box out.
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        // An inline element of a paragraph has no box to grow either: the
        // paragraph lays the block inside it out among its lines.
        if document.nodes[parent].in_ifc_subtree() && !document.nodes[child].in_ifc_subtree() {
            continue;
        }
        // A float that grew with the lines inside it does not move the flow
        // after it: only the boxes that hold their floats grow around it (CSS
        // 2.1 §10.6.7). The flow past the float gives back the shift that
        // reached it.
        if floats_hold_growth
            && matches!(
                cascade.computed[child].float,
                FloatValue::Left | FloatValue::Right
            )
            && is_floating_box_for_pagination(document, cascade, child)
        {
            if flow_delta.abs() > 0.001 {
                shifts.bounded.push((child, flow_delta, flow_page));
                bounded = true;
            }
            let grown = size_delta;
            float_delta = size_delta;
            flow_delta = 0.0;
            size_delta = 0.0;
            let below = if scanned == Some(child) {
                BelowGrownFloat::default()
            } else {
                below_grown_float(document, cascade, body_id, parent, child, grown)
                    .unwrap_or_default()
            };
            // The floats placed below it move down once the walk is done,
            // so that the content ends here are measured before they move.
            for (follower, shift, is_float) in below.later {
                let box_parent = document.parent_of(follower).unwrap_or(parent);
                followers.push((box_parent, follower, shift, is_float));
            }
            // A box after the float that clears it starts below it, and
            // takes the boxes after it along.
            if let Some(clearing) = below.clearing {
                let ClearingBox {
                    before,
                    node,
                    outer,
                    shift,
                } = clearing;
                let box_parent = document.parent_of(node).unwrap_or(parent);
                match outer {
                    // A box after the block that holds the float moves once
                    // the walk is done, as a float below it does.
                    None => followers.push((box_parent, node, shift, false)),
                    Some(outer) => {
                        shifts.from_box.push((
                            box_parent,
                            child_index(document, box_parent, node),
                            shift,
                        ));
                        shifted_after = Some(before);
                        flow_delta = shift;
                        size_delta = shift;
                        if outer != node {
                            // A box that clears the float from inside a block of the
                            // same formatting context moves in that block, which
                            // grows with it: the boxes after the block follow by as
                            // much as the block grew.
                            document.translate_pagination_y(node, shift);
                            let (inner_bounded, passed) = grow_ancestors_below(
                                document,
                                cascade,
                                node,
                                shift,
                                body_id,
                                shifts,
                                flow_page,
                                &IfcBoxShifts::default(),
                                Some(parent),
                                true,
                            );
                            bounded |= inner_bounded;
                            shifted_after = Some(outer);
                            flow_delta = passed;
                            size_delta = passed;
                        }
                    }
                }
            }
        }
        if stop.is_none()
            && flow_delta > 0.001
            && wraps_items_into_columns(document, cascade, parent)
            && document.taffy_parent_of(child) == Some(parent)
        {
            // An item of a column flex container that wraps moves down in its
            // own column: the items after it there follow it, and the other
            // columns keep their places, as when the item grows.
            let after = item_bottom_in(document, parent, child);
            shifts.bounded.push((child, flow_delta, flow_page));
            grow_container_beside_item(
                document,
                cascade,
                body_id,
                parent,
                child,
                (after - flow_delta, after),
                shifts,
                // The page the item ends on is not known here: the flow
                // reaches it again from the next candidate's own place.
                (flow_page, flow_page),
            );
            return (bounded, 0.0);
        }
        shift_later_block_siblings(
            document,
            parent,
            shifted_after.take().unwrap_or(child),
            flow_delta,
        );
        let height = document.nodes[parent].layout.size.height;
        normal_block_shifts::refresh(document, cascade, body_id, parent);
        let grown = grown_auto_height(
            document,
            cascade,
            parent,
            child,
            flow_delta,
            size_delta,
            float_delta,
            child == node_id,
            body_id,
            ifc_boxes,
        );
        let Some(grown) = grown else {
            if flow_delta.abs() > 0.001 {
                shifts.bounded.push((parent, flow_delta, flow_page));
                bounded = true;
            }
            if float_delta.abs() > 0.001 && !holds_floats(document, cascade, parent) {
                flow_delta = 0.0;
                size_delta = 0.0;
                child = parent;
                continue;
            }
            break;
        };
        let (old_flow, new_flow) = grown.flow_heights;
        let limit = flow_delta.max(size_delta).max(float_delta);
        let size_absorbed = (grown.height - height).clamp(0.0, limit);
        let flow_absorbed = (new_flow - old_flow).clamp(0.0, limit);
        // The bottom margin that starts or stops collapsing through the box
        // moves the flow after it too, but not the box. A walk that stops
        // below a container hands the container only the growth.
        let flow_passed = flow_absorbed
            + if stop.is_none() {
                grown.margin_shift
            } else {
                0.0
            };
        if flow_passed < -0.001 {
            pulled = Some((parent, flow_passed));
        }
        let owns_floats = holds_floats(document, cascade, parent);
        bounded |= grown.capped || flow_passed < flow_delta - 0.001;
        if flow_passed < flow_delta - 0.001 {
            shifts
                .bounded
                .push((parent, flow_delta - flow_passed, flow_page));
        } else if flow_passed > flow_delta + 0.001 {
            // A box holding its floats grew around a float that moved with
            // the content it overflowed: the flow after the box follows.
            shifts.growth.push((parent, flow_passed - flow_delta));
        }
        float_delta = if owns_floats {
            0.0
        } else {
            grown.float_extent_delta
        };
        if flow_absorbed <= 0.001 && size_absorbed <= 0.001 && float_delta.abs() <= 0.001 {
            break;
        }
        let height = height + size_absorbed;
        document.nodes[parent].layout.size.height = height;
        document.mark_pagination_extent_dirty(parent);
        flow_delta = flow_passed.clamp(0.0, flow_absorbed);
        size_delta = size_absorbed;
        child = parent;
        if parent == body_id {
            break;
        }
    }
    // The blocks around that box end higher with the flow in them, as far as
    // their heights follow it.
    if let Some((node, pull)) = pulled
        && node != body_id
    {
        let held = shrink_ancestors_after_content_change(
            document,
            cascade,
            node,
            pull,
            0.0,
            body_id,
            &IfcBoxShifts::default(),
            None,
        );
        shifts.bounded.extend(
            held.into_iter()
                .map(|(ancestor, held)| (ancestor, held, flow_page)),
        );
    }
    (bounded, 0.0)
}

/// Whether the walk at `node_id` has reached the child of `parent` at index
/// `from`: `node_id` is in that child or in one after it, or comes after
/// `parent` in tree order. A node before `parent`, such as a box before the
/// `display: contents` element `parent` is or an element that holds
/// `parent`, has not reached it.
fn reaches_child(document: &Document, node_id: usize, parent: usize, from: usize) -> bool {
    let index = |up: usize, child: usize| {
        document.nodes[up]
            .children
            .iter()
            .position(|&id| id == child)
    };
    // The ancestors of `parent`, each with its child toward `parent`.
    let mut toward = Vec::new();
    let mut child = parent;
    while let Some(up) = document.parent_of(child) {
        toward.push((up, child));
        child = up;
    }
    // Each ancestor of `node_id` with its child toward `node_id`.
    let ancestors = std::iter::successors(Some(node_id), |&id| document.parent_of(id)).zip(
        std::iter::successors(document.parent_of(node_id), |&id| document.parent_of(id)),
    );
    ancestors
        .map(|(child, up)| {
            if up == parent {
                return Some(index(parent, child).is_none_or(|index| index >= from));
            }
            toward
                .iter()
                .find(|&&(ancestor, _)| ancestor == up)
                .map(|&(_, branch)| {
                    let rank = |id: usize| (table_section_rank(document, up, id), index(up, id));
                    rank(child) > rank(branch)
                })
        })
        .find_map(|reached| reached)
        .unwrap_or(true)
}

/// Where `child` comes among the children of `parent` in the order the
/// flow reaches them, before its index: a table puts its first header group
/// first and its first footer group last (CSS 2.1 §17.2).
fn table_section_rank(document: &Document, parent: usize, child: usize) -> u8 {
    if !matches!(
        document.nodes[parent].display(),
        DisplayValue::Table | DisplayValue::InlineTable
    ) {
        return 1;
    }
    let display = document.nodes[child].display();
    let first_of = |group: DisplayValue| {
        display == group
            && document.nodes[parent]
                .children
                .iter()
                .find(|&&id| document.nodes[id].display() == group)
                == Some(&child)
    };
    1 + u8::from(first_of(DisplayValue::TableFooterGroup))
        - u8::from(first_of(DisplayValue::TableHeaderGroup))
}

/// The index of `child` among the children of `parent`.
fn child_index(document: &Document, parent: usize, child: usize) -> usize {
    document.nodes[parent]
        .children
        .iter()
        .position(|&id| id == child)
        .unwrap_or_default()
}

/// The boxes after a float that grew with the lines moved inside it that
/// have to move down to stay below it (see [`below_grown_float`]).
#[derive(Default)]
struct BelowGrownFloat {
    /// The boxes after the first one in `clearing` that move by themselves:
    /// the floats placed below a float that moves and the boxes that clear
    /// one further than the boxes before them moved. Each comes with how far
    /// it moves down and whether it is a float.
    later: Vec<(usize, f32, bool)>,
    /// The first in-flow box that clears a float that moves.
    clearing: Option<ClearingBox>,
}

/// A step of the search in [`below_grown_float`]: a box to visit, with the
/// box of the float's parent that is it or holds it, or the end of a block
/// it searched inside, with how far the boxes had moved when it began.
enum SearchStep {
    Visit(usize, Option<usize>),
    Leave(usize, f32),
}

/// How much of a move of `moved` by the boxes at the end of `block` the
/// boxes after it take: as much as the block grows, which the room below
/// its content that `min-height` keeps takes first, up to its `max-height`
/// (CSS 2.1 §10.7).
fn passed_growth(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    block: usize,
    moved: f32,
) -> f32 {
    if !height_can_grow(document, block) {
        return 0.0;
    }
    let layout = &document.nodes[block].layout;
    let edges_below = layout.padding.bottom + layout.border.bottom;
    let end = flow_content_end(document, cascade, body_id, block);
    let (height, _) = grown_height(
        layout.size.height,
        border_box_height_bounds(document, block),
        edges_below,
        (end, end + moved),
    );
    (height - layout.size.height).max(0.0)
}

/// Where the in-flow content of `block`, a block that does not hold its
/// floats, ends in its border box: its in-flow children and lines, with the
/// last child's bottom margin when bottom padding or border keeps it inside
/// (CSS 2.1 §8.3.1, §10.6.3).
fn flow_content_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    block: usize,
) -> f32 {
    let layout = &document.nodes[block].layout;
    let mut end = layout.padding.top + layout.border.top;
    let mut last = None;
    for child in laid_out_children(document, block) {
        let node = &document.nodes[child];
        if node.kind() != NodeKind::Element
            || node.display() == DisplayValue::None
            || matches!(
                node.position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
            || is_floating_box_for_pagination(document, cascade, child)
        {
            continue;
        }
        let top =
            pagination_child_parent_y(document, block, child, document.pagination_local_y(child))
                - layout_relative_offset_y(document, cascade, body_id, child);
        end = end.max(top + node.layout.size.height);
        last = Some(child);
    }
    // A table cell's layout padding also holds the room its vertical
    // alignment leaves, and its children's margins never collapse through
    // it.
    if (layout.padding.bottom + layout.border.bottom > 0.0
        || encloses_child_margins(document, cascade, block))
        && let Some(last) = last
    {
        end += collapsed_end_margin(document, cascade, last);
    }
    content_bottom(document, block).map_or(end, |lines| end.max(lines))
}

/// Whether `block` keeps its last child's bottom margin inside however thin
/// its bottom edges are: a box that establishes a block formatting context,
/// such as a table cell, which no child margin collapses through (CSS 2.1
/// §8.3.1). An anonymous table cell has no style of its own, and is such
/// a box too.
fn encloses_child_margins(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
) -> bool {
    document.anonymous_table_cell(block).is_some() || holds_floats(document, cascade, block)
}

/// The bottom margin of `node_id` with the bottom margins of its last
/// in-flow descendants that collapse through its bottom edge (CSS 2.1
/// §8.3.1): those of a box with no bottom padding or border, an `auto`
/// height and no formatting context of its own.
fn collapsed_end_margin(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> f32 {
    let (mut positive, mut negative) = (0.0f32, 0.0f32);
    let mut id = node_id;
    loop {
        let layout = &document.nodes[id].layout;
        let margin = layout.margin.bottom;
        positive = positive.max(margin);
        negative = negative.min(margin);
        // A `min-height` or `max-height` that sets the used height keeps the
        // margin inside the box too.
        let (min, max) = border_box_height_bounds(document, id);
        let height = layout.size.height;
        let content_sized = (min <= 0.0 || height > min + 0.001) && height < max - 0.001;
        let passes_through = layout.padding.bottom + layout.border.bottom <= 0.0
            && !holds_floats(document, cascade, id)
            && definite_content_height(document, id).is_none()
            && content_sized;
        if !passes_through {
            break;
        }
        // Text or an inline box after the last block keeps that block's
        // margin inside the box.
        let last = page_edge_child(document, cascade, id, false, 0)
            .ok()
            .flatten()
            .filter(|&child| {
                document.nodes[child].kind() == NodeKind::Element
                    && is_block_level_flow_box(document.nodes[child].display())
            });
        match last {
            Some(child) => id = child,
            None => break,
        }
    }
    positive + negative
}

/// An in-flow box that clears a float that moves (see [`BelowGrownFloat`]).
struct ClearingBox {
    /// The box of the float's parent before the one that holds `node`.
    before: usize,
    /// The box that clears the float.
    node: usize,
    /// The box of the float's parent that is `node` or holds it, when
    /// `node` is inside the float's parent.
    outer: Option<usize>,
    /// How far `node` moves down.
    shift: f32,
}

/// Whether `node_id` is a block in the formatting context around it, whose
/// boxes take part in it.
fn shares_formatting_context(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    matches!(
        document.nodes[node_id].display(),
        DisplayValue::Block | DisplayValue::ListItem
    ) && !holds_floats(document, cascade, node_id)
}

/// The children of `node_id` that are laid out as its boxes: the children
/// of `display: contents` elements take their places.
fn laid_out_children(document: &Document, node_id: usize) -> Vec<usize> {
    let mut boxes = Vec::new();
    let mut stack: Vec<usize> = document.nodes[node_id]
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(id) = stack.pop() {
        if document.nodes[id].kind() == NodeKind::Element
            && document.nodes[id].display() == DisplayValue::Contents
        {
            stack.extend(document.nodes[id].children.iter().rev().copied());
        } else {
            boxes.push(id);
        }
    }
    boxes
}

/// The boxes after `float` in `parent` that have to move down to stay below
/// the float's margin box, now that the float grew by `grown` with the lines
/// moved inside it, when `parent` is inside another float or an absolutely
/// positioned box: the floats that were placed below it or below one of
/// those floats, and the first in-flow box that clears one of them. The boxes
/// before that box stay beside the floats. Pagination places no box inside a
/// float or an absolutely positioned box by itself; elsewhere, the boxes
/// below the float keep there through the edges [`FloatShifts`] records.
fn below_grown_float(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    parent: usize,
    float: usize,
    grown: f32,
) -> Option<BelowGrownFloat> {
    let inside_float = std::iter::once(parent)
        .chain(document.layout_ancestors(parent))
        .take_while(|&id| id != body_id)
        .any(|id| {
            is_floating_box_for_pagination(document, cascade, id)
                || matches!(
                    document.nodes[id].position(),
                    PositionValue::Absolute | PositionValue::Fixed
                )
        });
    if !inside_float {
        return None;
    }
    let boxes = laid_out_children(document, parent);
    let index = boxes.iter().position(|&id| id == float)?;
    let mut below = BelowGrownFloat::default();
    // Clearance is measured in normal-flow coordinates, without the
    // relative offsets of the boxes and their ancestors.
    let flow_y = |id| {
        current_abs_y(document, id) - accumulated_relative_offset_y(document, cascade, body_id, id)
    };
    let margin_bottom = |id: usize| {
        let layout = &document.nodes[id].layout;
        flow_y(id) + layout.size.height + layout.margin.bottom
    };
    // The floats that move, each with its side and where its margin box
    // ended before and after the move.
    let bottom = margin_bottom(float);
    let mut moved = vec![(cascade.computed[float].float, bottom - grown, bottom)];
    // The boxes after the float in tree order, each with the box of
    // `parent` that is it or holds it. The blocks of the same formatting
    // context are searched inside too, and the boxes after `parent` when
    // it is one of them.
    let mut stack: Vec<SearchStep> = boxes[index + 1..]
        .iter()
        .rev()
        .map(|&id| SearchStep::Visit(id, Some(id)))
        .collect();
    let mut before = float;
    let mut scope = parent;
    // How far the boxes from here on move with the boxes before them that
    // clear a float.
    let mut carried = 0.0;
    loop {
        while let Some(step) = stack.pop() {
            let (next, outer) = match step {
                SearchStep::Visit(next, outer) => (next, outer),
                // The boxes after a block move by as much as the block grew
                // with the boxes in it that moved.
                SearchStep::Leave(block, entered) => {
                    carried = entered
                        + passed_growth(document, cascade, body_id, block, carried - entered);
                    continue;
                }
            };
            let previous = std::mem::replace(&mut before, outer.unwrap_or(next));
            // Text, an element that generates no box and a box out of the flow
            // neither clear nor move.
            if document.nodes[next].kind() != NodeKind::Element
                || document.nodes[next].display() == DisplayValue::None
                || matches!(
                    document.nodes[next].position(),
                    PositionValue::Absolute | PositionValue::Fixed
                )
            {
                continue;
            }
            if is_floating_box_for_pagination(document, cascade, next) {
                // A float that did not fit beside a float that moves, or
                // cleared it, starts at or below where that float ended before
                // it moved.
                let top = flow_y(next) - document.nodes[next].layout.margin.top;
                let shift = moved
                    .iter()
                    .filter(|&&(_, old, _)| top >= old - 0.01)
                    .map(|&(_, _, new)| new - (top + carried))
                    .fold(0.0, f32::max);
                // A float that moves only with the boxes before it moves too:
                // the boxes after a block that takes that move in its own room
                // still clear it, and the floats placed below it follow it.
                if shift > 0.001 || carried > 0.001 {
                    let bottom = margin_bottom(next);
                    moved.push((
                        cascade.computed[next].float,
                        bottom,
                        bottom + carried + shift,
                    ));
                }
                if shift > 0.001 {
                    below.later.push((next, shift, true));
                }
                continue;
            }
            // `clear` applies to block-level boxes only.
            let clear = cascade.computed[next].clear;
            if is_block_level_flow_box(document.nodes[next].display())
                && moved
                    .iter()
                    .any(|&(side, _, _)| clears_float_side(clear, side))
            {
                let top = flow_y(next) + carried;
                let shift = moved
                    .iter()
                    .filter(|&&(side, _, _)| clears_float_side(clear, side))
                    .map(|&(_, _, new)| new - top)
                    .fold(0.0, f32::max);
                // A box that is already below the floats it clears leaves
                // the boxes after it to clear the others, and the boxes
                // after one that moves move with it.
                if shift > 0.001 {
                    if below.clearing.is_none() {
                        below.clearing = Some(ClearingBox {
                            before: previous,
                            node: next,
                            outer,
                            shift,
                        });
                    } else {
                        below.later.push((next, shift, false));
                    }
                    carried += shift;
                }
            }
            if shares_formatting_context(document, cascade, next) {
                stack.push(SearchStep::Leave(next, carried));
                stack.extend(
                    laid_out_children(document, next)
                        .into_iter()
                        .rev()
                        .map(|id| SearchStep::Visit(id, outer)),
                );
            }
        }
        let Some(up) = shares_formatting_context(document, cascade, scope)
            .then(|| document.layout_parent_of(scope))
            .flatten()
        else {
            break;
        };
        carried = passed_growth(document, cascade, body_id, scope, carried);
        stack = laid_out_children(document, up)
            .into_iter()
            .skip_while(|&id| id != scope)
            .skip(1)
            .map(|id| SearchStep::Visit(id, None))
            .collect();
        stack.reverse();
        scope = up;
    }
    Some(below)
}

/// Move `node_id` by `delta` after its block-start margin was truncated at a
/// page break, together with its inline content and the auto-height boxes
/// containing it. Returns the ancestors whose height kept them from
/// shrinking with it, each with the part of `delta` the flow after it does
/// not follow.
fn move_box_by_truncated_margin(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    delta: f32,
    flow_root: usize,
) -> Vec<(usize, f32)> {
    document.translate_pagination_y(node_id, delta);
    let followed_ifc_boxes = follow_moved_ifc_block(document, node_id, delta);
    shrink_ancestors_after_margin_truncation(
        document,
        cascade,
        node_id,
        delta,
        flow_root,
        &followed_ifc_boxes,
    )
}

/// A truncated margin shortens the auto-height boxes containing the flow.
/// Reuse the same used-child extents and size bounds as ordinary pagination
/// growth, retaining unrelated float extents and authored height limits.
/// Returns the ancestors that held the flow after them where it was (see
/// [`shrink_ancestors_after_content_change`]).
fn shrink_ancestors_after_margin_truncation(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node: usize,
    delta: f32,
    flow_root: usize,
    ifc_boxes: &IfcBoxShifts,
) -> Vec<(usize, f32)> {
    shrink_ancestors_after_content_change(
        document, cascade, node, delta, delta, flow_root, ifc_boxes, None,
    )
}

/// Propagates a shortened content extent separately from its owned floats.
/// Bounds can absorb either change before the containing flow is reached.
/// Returns each ancestor whose height kept the flow after it from following
/// the content inside it, with the part of the content's move, negative,
/// that the flow after that ancestor does not take. The part is positive
/// where a positive bottom margin that stops collapsing through an ancestor
/// takes the flow after it further up than the content moved (see
/// [`escaped_end_margin_shift`]).
#[allow(clippy::too_many_arguments)]
fn shrink_ancestors_after_content_change(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node: usize,
    delta: f32,
    mut float_delta: f32,
    flow_root: usize,
    ifc_boxes: &IfcBoxShifts,
    mut changes: Option<&mut Vec<(usize, f32)>>,
) -> Vec<(usize, f32)> {
    let mut held = Vec::new();
    if (delta >= 0.0 && float_delta >= 0.0) || !delta.is_finite() {
        return held;
    }
    let mut child = node;
    let mut flow_delta = delta;
    let mut size_delta = delta;
    let mut walk = AncestorCursor::new(document.layout_parent_of(child), ParentRelation::Layout);
    while let Some(parent) = walk.current() {
        walk.advance(document);
        if document.nodes[parent].kind() != NodeKind::Element {
            break;
        }
        if document.nodes[parent].size_containment.height {
            if flow_delta < -0.001 {
                held.push((parent, flow_delta));
            }
            break;
        }
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        let followed = if changes.is_some() && flow_delta.abs() > 0.001 {
            let layout = document.nodes[child].layout;
            let top = pagination_child_parent_y(
                document,
                parent,
                child,
                document.pagination_local_y(child),
            );
            let allowed_oof = document
                .ifc_layout_node(parent)
                .and_then(|node| node.ifc.as_ref())
                .into_iter()
                .flat_map(|ifc| &ifc.boxes)
                .filter(|item| item.kind == IfcBoxKind::OutOfFlow)
                .filter(|item| {
                    out_of_flow_follow(document, cascade, parent, item.node)
                        == Some(OutOfFlowFollow::OnLine)
                })
                .map(|item| item.node)
                .collect();
            follow_ifc_block_with_oof(
                document,
                parent,
                child,
                top,
                top + layout.size.height - size_delta,
                flow_delta,
                Some(&allowed_oof),
            )
        } else {
            IfcBoxShifts::default()
        };
        let ifc_boxes = if followed.boxes.is_empty() {
            ifc_boxes
        } else {
            &followed
        };
        shift_later_block_siblings(document, parent, child, flow_delta);
        let old_height = document.nodes[parent].layout.size.height;
        normal_block_shifts::refresh(document, cascade, flow_root, parent);
        let Some(adjusted) = grown_auto_height(
            document,
            cascade,
            parent,
            child,
            flow_delta,
            size_delta,
            float_delta,
            changes.is_some() || child == node,
            flow_root,
            ifc_boxes,
        ) else {
            if flow_delta < -0.001 {
                held.push((parent, flow_delta));
            }
            break;
        };
        document.nodes[parent].layout.size.height = adjusted.height;
        document.mark_pagination_extent_dirty(parent);
        let (old_flow, new_flow) = adjusted.flow_heights;
        // A bottom margin that stops collapsing through the box as its height
        // stays at its `min-height` moves the flow after it too: down when it
        // was negative, further up when it was positive.
        let passed = new_flow - old_flow + adjusted.margin_shift;
        let kept = flow_delta - (new_flow - old_flow).min(0.0) - adjusted.margin_shift;
        if flow_delta < 0.0
            && kept.abs() > 0.001
            && (passed > flow_delta + 0.001 || adjusted.margin_shift.abs() > 0.001)
        {
            held.push((parent, kept));
        }
        if let Some(records) = changes.as_mut()
            && (passed - flow_delta).abs() > 0.001
        {
            records.push((parent, passed - flow_delta));
        }
        let owns_floats = holds_floats(document, cascade, parent);
        float_delta = if owns_floats {
            0.0
        } else {
            adjusted.float_extent_delta
        };
        size_delta = adjusted.height - old_height;
        flow_delta = passed;
        child = parent;
        if parent == flow_root
            || (flow_delta.abs() <= 0.001
                && size_delta.abs() <= 0.001
                && float_delta.abs() <= 0.001)
        {
            break;
        }
    }
    held
}

/// Whether `block` is a block-level box laid out in plain block flow up to
/// the body: every box from it to the body stacks its children in block
/// order, and none is floated or absolutely positioned (see
/// [`placed_in_block_flow`]). A flex container or a grid whose only item holds
/// the paragraph stacks it the same way, and so does one that sets its items
/// beside one another with no other such container around it: the item
/// grows and moves the rows or lines after it (see [`grows_beside_items`]).
/// The heights of the paragraph and of its
/// ancestors may be constrained: moving the lines grows their boxes only as
/// far as those constraints allow, and an ancestor that clips at a set height
/// hides the lines moved past its clip. Size containment remains outside this
/// path.
fn is_block_flow_paragraph(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
) -> bool {
    is_paragraph_in_flow(document, cascade, block, body_id, false)
}

/// [`is_block_flow_paragraph`] that also takes a paragraph inside a float:
/// the boxes from it to the body may include floats, which grow with the
/// moved lines.
fn is_float_flow_paragraph(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
) -> bool {
    is_paragraph_in_flow(document, cascade, block, body_id, true)
}

/// Whether `block` is a block-level paragraph laid out in plain block flow
/// up to the body, through floats too when `through_floats` is set.
fn is_paragraph_in_flow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
    through_floats: bool,
) -> bool {
    matches!(
        document.nodes[block].display(),
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
    ) && !document.nodes[block].size_containment.height
        && placed_in_flow_or_floats_to_body(
            document,
            cascade,
            block,
            body_id,
            FlowPassage {
                sole_items: true,
                clips: true,
                floats: through_floats,
                paragraphs: true,
            },
        )
}

/// Whether every box from `block` up to the body stacks its children in
/// block order, and none is floated or absolutely positioned, so a box
/// growing below `block` pushes the boxes after it down. An ancestor's height
/// may be constrained: it grows only as far as `height`, `min-height` and
/// `max-height` allow, and the flow after it stays where that height puts it
/// (see [`grow_ancestors_for_shift`]). One that clips its block-axis overflow
/// at a set height, or whose size containment ignores its content, stays
/// outside this path (see [`holds_moved_lines`]).
fn placed_in_block_flow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
) -> bool {
    placed_in_flow_to_body(document, cascade, block, body_id, false, false)
}

/// [`placed_in_block_flow`], which with `through_sole_items` also passes a
/// flex container or a grid whose only item is the box the walk comes from:
/// nothing sits beside that item or after it, so a box growing inside it grows
/// the container and moves nothing else. It passes a grid whose item stacks
/// alone in its rows, and a container whose items sit beside one another as
/// [`grows_beside_items`] tells, too. With `through_clips` it also passes
/// a box that clips at a set height (see [`holds_moved_lines`]).
fn placed_in_flow_to_body(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
    through_sole_items: bool,
    through_clips: bool,
) -> bool {
    placed_in_flow_or_floats_to_body(
        document,
        cascade,
        block,
        body_id,
        FlowPassage {
            sole_items: through_sole_items,
            clips: through_clips,
            floats: false,
            paragraphs: false,
        },
    )
}

/// The boxes that [`placed_in_flow_or_floats_to_body`] passes on its way up
/// besides plain block containers.
#[derive(Clone, Copy)]
struct FlowPassage {
    /// A flex or grid container whose item grows alone (see
    /// [`placed_in_flow_to_body`]).
    sole_items: bool,
    /// A box that clips at a set height (see [`holds_moved_lines`]).
    clips: bool,
    /// A float, which grows with the lines inside it.
    floats: bool,
    /// A paragraph that lays the box out as a block among its lines: the
    /// paragraph's content after the block follows the block's moved lines.
    paragraphs: bool,
}

/// [`placed_in_flow_to_body`], which with `floats` also passes a float: it
/// grows with the lines inside it, and the boxes around it follow that growth
/// as they follow a float's (see [`grow_ancestors_below`]). With `paragraphs`
/// it also passes a paragraph that lays the box out among its lines.
fn placed_in_flow_or_floats_to_body(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
    through: FlowPassage,
) -> bool {
    let FlowPassage {
        sole_items: through_sole_items,
        clips: through_clips,
        floats: through_floats,
        paragraphs: through_paragraphs,
    } = through;
    let mut child = block;
    let mut walk = AncestorCursor::new(Some(block), ParentRelation::Layout);
    loop {
        let placed_apart = (!through_floats
            && is_floating_box_for_pagination(document, cascade, child))
            || matches!(
                document.nodes[child].position(),
                PositionValue::Absolute | PositionValue::Fixed
            );
        walk.advance(document);
        // A caption of a block table stacks its children in block order, and
        // the rows after it follow its growth: the walk goes on from the table.
        if let Some(table) = walk.current().filter(|_| !placed_apart).and_then(|parent| {
            caption_table(document, parent).filter(|_| {
                moves_later_siblings(document, parent)
                    && holds_moved_lines(document, cascade, parent, through_clips)
            })
        }) {
            child = table;
            walk = AncestorCursor::new(Some(table), ParentRelation::Layout);
            continue;
        }
        match walk.current() {
            Some(parent) if !placed_apart && parent == body_id => return true,
            // A `display: contents` element has no box; its parent lays the
            // child out.
            Some(parent)
                if !placed_apart
                    && (document.nodes[parent].display() == DisplayValue::Contents
                        || moves_later_siblings(document, parent)
                            && holds_moved_lines(document, cascade, parent, through_clips)) =>
            {
                child = parent;
            }
            // A block among the lines of a paragraph: the paragraph's content
            // after it follows it when its lines move down and it grows.
            Some(parent)
                if !placed_apart
                    && through_paragraphs
                    && ifc_paragraph_of_block(document, child) == Some(parent)
                    && matches!(
                        document.nodes[parent].display(),
                        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
                    )
                    && content_follows_block_children(document, cascade, parent)
                    && holds_moved_lines(document, cascade, parent, through_clips) =>
            {
                child = parent;
            }
            // A float in a paragraph grows beside its lines, which stay where
            // they are.
            Some(parent)
                if through_floats
                    && is_floating_box_for_pagination(document, cascade, child)
                    && document.nodes[parent].is_ifc_root()
                    && document.nodes[parent].multicol.is_none()
                    && matches!(
                        document.nodes[parent].display(),
                        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
                    )
                    && holds_moved_lines(document, cascade, parent, through_clips) =>
            {
                child = parent;
            }
            Some(parent)
                if !placed_apart
                    && through_sole_items
                    && holds_moved_lines(document, cascade, parent, through_clips)
                    && (has_no_other_item(document, parent, child)
                        || stacks_item_alone_in_its_rows(document, parent, child)
                        || grows_beside_items(document, parent, body_id)) =>
            {
                child = parent;
            }
            _ => return false,
        }
    }
}

/// Whether `container` is a horizontal flex or grid container that sets its
/// items beside one another, the only one around it up to the body: an item
/// that grows there moves the items on the rows or lines after it and leaves
/// those beside it be (see [`grow_container_beside_item`]).
fn grows_beside_items(
    document: &super::context::LayoutSession<'_>,
    container: usize,
    body_id: usize,
) -> bool {
    let node = &document.nodes[container];
    node.display().is_flex_or_grid_container()
        && !node.used_writing_mode.is_vertical()
        && edges_break_container(document, container, body_id) == Some(None)
}

/// Whether `grid` is a grid with rows that size to their content and a height
/// that follows them, and `item` is alone in the rows it spans, with the items
/// before it in rows above and the items after it in rows below: a row that
/// grows with the item pushes the items after it down, as a block pushes its
/// later siblings. A grid with a set or minimum height may stretch its rows,
/// which then absorb some of the growth instead.
fn stacks_item_alone_in_its_rows(
    document: &super::context::LayoutSession<'_>,
    grid: usize,
    item: usize,
) -> bool {
    let Some(rows) = stacking_grid_rows(document, grid) else {
        return false;
    };
    let Some(&(start, end)) = rows.get(&item) else {
        return false;
    };
    let mut before = true;
    document.nodes[grid].children.iter().all(|&child| {
        before &= child != item;
        child == item
            || !is_in_flow_grid_item_for_pagination(document, child)
            || rows.get(&child).is_some_and(|&(other_start, other_end)| {
                if before {
                    other_end <= start
                } else {
                    other_start >= end
                }
            })
    })
}

/// The rows each item of `grid` spans, when its rows size to their content
/// and its height follows them.
fn stacking_grid_rows(
    document: &super::context::LayoutSession<'_>,
    grid: usize,
) -> Option<HashMap<usize, (u16, u16)>> {
    (grid_rows_size_to_content(document, grid) && height_follows_content(document, grid)).then(
        || {
            document.nodes[grid]
                .grid_item_rows
                .iter()
                .map(|row| (row.item, (row.start, row.end)))
                .collect()
        },
    )
}

/// Whether `container` is a flex container or a grid with rows that size to
/// their content, and `item` is the only box on its lines or in its cells.
fn has_no_other_item(
    document: &super::context::LayoutSession<'_>,
    container: usize,
    item: usize,
) -> bool {
    let display = document.nodes[container].display();
    if display.is_flex_container() {
        flex_siblings_on_lines(document, container, item)
            .next()
            .is_none()
    } else if display.is_grid_container() {
        grid_rows_size_to_content(document, container)
            && document.nodes[container].children.iter().all(|&child| {
                child == item || !is_in_flow_grid_item_for_pagination(document, child)
            })
    } else {
        false
    }
}

/// The children of the flex container `container` other than `item` that sit
/// on a flex line. Collapsible white space between items, `display: none` and
/// absolutely positioned children are on no flex line; the test is the one the
/// pagination of the flex items uses, so a text node holding a no-break space
/// counts as an item.
fn flex_siblings_on_lines(
    document: &Document,
    container: usize,
    item: usize,
) -> impl Iterator<Item = &usize> {
    document.nodes[container]
        .children
        .iter()
        .filter(move |&&child| {
            child != item && is_in_flow_flex_child_for_pagination(document, container, child)
        })
}

/// The row flex container of `item` when `item` is a flex item alone on
/// its line and the container is itself placed in plain block flow (see
/// [`placed_in_block_flow`]): lines moved down inside the item grow it and
/// its flex line without moving another item beside it, and the lines after
/// it follow.
fn flex_container_of_item_alone_on_its_line(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    item: usize,
    body_id: usize,
) -> Option<usize> {
    if !height_follows_content(document, item)
        || matches!(
            document.nodes[item].position(),
            PositionValue::Absolute | PositionValue::Fixed
        )
    {
        return None;
    }
    let container = document.layout_parent_of(item).filter(|&container| {
        let computed = &cascade.computed[container];
        document.nodes[container].display().is_flex_container()
            && matches!(
                computed.flex_direction,
                FlexDirectionValue::Row | FlexDirectionValue::RowReverse
            )
            && height_follows_content(document, container)
            && placed_in_block_flow(document, cascade, container, body_id)
    })?;
    // An item that takes the whole main size leaves no room for another on
    // its line; vertical separation alone can come from cross-axis margins
    // or alignment of items that do share it.
    let layout = document.nodes[item].layout;
    let outer_width = layout.size.width + layout.margin.left + layout.margin.right;
    let line = document.nodes[container].layout;
    let line_width = line.size.width
        - line.padding.left
        - line.padding.right
        - line.border.left
        - line.border.right;
    if outer_width < line_width - 0.001 {
        return None;
    }
    // A single-line container puts every item on the item's line. In a
    // multi-line one, another item joins the line when its outer main size
    // fits in what the item leaves, which a zero or negative outer size
    // (negative margins) does.
    let wraps = cascade.computed[container].flex_wrap != FlexWrapValue::NoWrap;
    let room = line_width - outer_width;
    let top = layout.location.y - layout.margin.top;
    let bottom = layout.location.y + layout.size.height + layout.margin.bottom;
    flex_siblings_on_lines(document, container, item)
        .all(|&child| {
            let other = document.nodes[child].layout;
            let other_width = other.size.width + other.margin.left + other.margin.right;
            // A `display: contents` child lifts its children onto the lines
            // in its place, so its own box says nothing about them.
            wraps
                && document.nodes[child].display() != DisplayValue::Contents
                && other_width > room + 0.001
                && (other.location.y + other.size.height + other.margin.bottom <= top + 0.001
                    || other.location.y - other.margin.top >= bottom - 0.001)
        })
        .then_some(container)
}

/// Moves the flex lines of `container` below `item`, a flex item alone on
/// its line that grew by `shift` at its block-end edge, down by `shift`: in
/// a row flex container they follow the line that grew.
fn shift_later_flex_lines(document: &mut Document, container: usize, item: usize, shift: f32) {
    let layout = document.nodes[item].layout;
    // The item has already grown, so its old bottom is `shift` higher.
    let old_bottom = layout.location.y + layout.size.height - shift;
    let later: Vec<usize> = document.nodes[container]
        .children
        .iter()
        .copied()
        .filter(|&child| {
            // Absolutely positioned children are not on a flex line.
            child != item
                && !matches!(
                    document.nodes[child].position(),
                    PositionValue::Absolute | PositionValue::Fixed
                )
                && document.pagination_local_y(child) >= old_bottom - 0.001
        })
        .collect();
    for child in later {
        document.translate_pagination_y(child, shift);
    }
}

/// Whether the box `ancestor` can hold lines that move down inside it with
/// their paragraph: its height follows them, or it grows no further than its
/// constraints and keeps what passes its end visible. A box that clips at a
/// set height holds them too with `through_clips`, which the lines of a
/// paragraph take: the clip hides what the move pushes past it, as in
/// Chromium. Size containment makes the box ignore them.
fn holds_moved_lines(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    ancestor: usize,
    through_clips: bool,
) -> bool {
    !document.nodes[ancestor].size_containment.height
        && (through_clips || box_leaves_lines_to_pagination(document, cascade, ancestor))
}

/// Whether the height of `block` is that of its content: no `height`,
/// `max-height` or non-zero `min-height` sets it and no size containment
/// ignores the content.
fn height_follows_content(document: &super::context::LayoutSession<'_>, block: usize) -> bool {
    let node = &document.nodes[block];
    let block_axis = document.block_axis(block);
    let min_height = block_axis.min_height();
    block_axis.height().is_auto()
        && (min_height.is_auto() || style_dimension_length(min_height.into()) == Some(0.0))
        && block_axis.max_height().is_auto()
        && !node.size_containment.height
}

/// Whether the content of `block` cannot grow its height: a definite
/// `height` or a `max-height` that resolves sets it, or size containment
/// ignores the content. A percentage of a containing block whose height is
/// not definite caps nothing.
fn height_is_capped(document: &super::context::LayoutSession<'_>, block: usize) -> bool {
    definite_content_height(document, block).is_some()
        || border_box_height_bounds(document, block).1.is_finite()
        || document.nodes[block].size_containment.height
}

/// Whether `parent` is a block container whose later children follow a
/// moved child, or an inline box that lays only blocks out as one does (see
/// [`lays_blocks_out_inline_box`]). Flex, grid and table containers place
/// their children themselves, and a paragraph's lines follow a moved block
/// child through its line shifts.
fn moves_later_siblings(document: &Document, parent: usize) -> bool {
    lays_blocks_out_inline_box(document, parent)
        || matches!(
            document.nodes[parent].display(),
            DisplayValue::Block
                | DisplayValue::FlowRoot
                | DisplayValue::ListItem
                | DisplayValue::InlineBlock
                | DisplayValue::TableCaption
        ) && document.nodes[parent].multicol.is_none()
            && document
                .ifc_layout_node(parent)
                .is_none_or(|node| node.ifc.is_none())
}

/// Whether `node_id` is an inline box that holds no line of a paragraph,
/// only blocks (see [`crate::layout::multicol::block_in_inline_wrapper`]),
/// with no block-axis padding or border of its own: it stacks those blocks
/// in block flow, as the anonymous block around a block inside an inline box
/// does.
fn lays_blocks_out_inline_box(document: &Document, node_id: usize) -> bool {
    let layout = &document.nodes[node_id].layout;
    crate::layout::multicol::block_in_inline_wrapper(document, node_id)
        && layout.padding.top + layout.padding.bottom + layout.border.top + layout.border.bottom
            <= 0.0
}

/// Moves the boxes that follow `child` in the block container `parent`
/// down by `delta`, the distance `child`'s block-end edge moved: in block
/// flow they follow it.
fn shift_later_block_siblings(document: &mut Document, parent: usize, child: usize, delta: f32) {
    if moves_later_siblings(document, parent) {
        shift_later_flow_siblings(document, parent, child, delta);
    }
}

/// Moves the boxes that follow `child` in `parent`, which stacks its
/// children in block flow, down by `delta`.
fn shift_later_flow_siblings(document: &mut Document, parent: usize, child: usize, delta: f32) {
    if delta == 0.0 || !delta.is_finite() {
        return;
    }
    if document
        .normal_block_shifts
        .as_ref()
        .is_some_and(|state| !state.can_defer(delta))
    {
        document.abandon_normal_block_shifts();
    }
    if document
        .normal_block_shifts
        .as_mut()
        .is_some_and(|state| state.shift_after(parent, child, delta))
    {
        document.mark_pagination_extent_dirty(parent);
        return;
    }
    // The boxes after a box that is a child of `parent` itself are in
    // `parent`'s own child list, which is walked in place without collecting
    // the children of every move.
    if let Some(index) = document.nodes[parent].children.iter().position(|&id| {
        #[cfg(test)]
        tests::record_normal_sibling_work(0, 1);
        id == child
    }) && document.nodes[child].display() != DisplayValue::Contents
    {
        // The children of the `display: contents` boxes met on the way.
        let mut opened = Vec::new();
        for later in index + 1..document.nodes[parent].children.len() {
            let mut id = document.nodes[parent].children[later];
            loop {
                if document.nodes[id].display() == DisplayValue::Contents {
                    opened.extend(document.nodes[id].children.iter().copied());
                } else {
                    shift_block_sibling(document, id, delta);
                }
                match opened.pop() {
                    Some(next) => id = next,
                    None => break,
                }
            }
        }
        return;
    }
    // `display: contents` children generate no box: their children are laid
    // out as this box's children.
    let mut boxes = Vec::new();
    let mut stack: Vec<usize> = document.nodes[parent]
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(id) = stack.pop() {
        #[cfg(test)]
        tests::record_normal_sibling_work(0, 1);
        if document.nodes[id].display() == DisplayValue::Contents {
            stack.extend(document.nodes[id].children.iter().rev().copied());
        } else {
            boxes.push(id);
        }
    }
    let Some(index) = boxes.iter().position(|&id| id == child) else {
        return;
    };
    for &sibling in &boxes[index + 1..] {
        shift_block_sibling(document, sibling, delta);
    }
}

/// Grows the last record of `node` by `delta`, as its box grew: the records
/// of a multicol container and its spanners are painted at their own size.
fn grow_last_record(document: &mut super::context::LayoutSession<'_>, node: usize, delta: f32) {
    let tree = &mut document.fragment_tree;
    if let Some(&record) = tree
        .records_of(node)
        .iter()
        .rev()
        .find(|&&record| !tree.is_retired(record))
    {
        tree.fragments[record].rect.height += delta;
    }
}

/// Whether `child` is a spanner of `parent`, a multicol container that lays
/// its children out as boxes rather than among lines of its own.
fn spans_box_columns(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    child: usize,
    parent: usize,
) -> bool {
    document.nodes[parent].multicol.is_some()
        && document
            .ifc_layout_node(parent)
            .is_none_or(|node| node.ifc.is_none())
        && super::is_spanner_in(document, cascade, child)
}

/// Moves what follows the spanner `spanner` in the multicol container
/// `container` down by `delta`, the distance the spanner's end moved: the
/// columns after a spanner start below it (CSS Multi-column Layout 1, 6.1),
/// and so does every later spanner. The column boxes are painted from the
/// records the columns placed under the container's record, so those move
/// along with the boxes. `grows` tells that a break inside the spanner grew
/// it, rather than the spanner moving whole.
fn shift_columns_after_spanner(
    document: &mut super::context::LayoutSession<'_>,
    container: usize,
    spanner: usize,
    delta: f32,
    grows: bool,
) {
    let children = document.children_through_contents(container);
    let spanner_index = children.iter().position(|&id| id == spanner);
    let after = spanner_index.map_or(children.len(), |index| index + 1);
    // The column groups keep the order of the flow, each with the first box
    // of the run of column content it lays out, and the rules follow the
    // groups in turn. The groups after the spanner are told apart by that
    // order, not by where they lie: a negative margin can start them at or
    // above the spanner's top, and pull the spanner up over the group before
    // it.
    let before: HashSet<usize> = children[..spanner_index.unwrap_or(children.len())]
        .iter()
        .copied()
        .collect();
    let groups = &document.nodes[container].multicol_groups;
    let first_group = groups
        .iter()
        .take_while(|group| group.run_start.is_some_and(|run| before.contains(&run)))
        .count();
    // The rules of a group as the column rules counted them.
    let first_rule: usize = groups[..first_group]
        .iter()
        .filter(|group| group.height > 0.0)
        .map(|group| {
            group
                .occupied
                .iter()
                .filter(|&&column| group.occupied.contains(&column.saturating_add(1)))
                .count()
        })
        .sum();
    for group in document.nodes[container]
        .multicol_groups
        .iter_mut()
        .skip(first_group)
    {
        group.context.origin_y += delta;
    }
    for rule in document
        .column_rules
        .get_mut(&container)
        .into_iter()
        .flatten()
        .skip(first_rule)
    {
        rule.y += delta;
    }
    // The spanner's record holds the content that a break inside it moved;
    // a spanner that moved whole keeps its height.
    if grows {
        grow_last_record(document, spanner, delta);
    }
    let tree = &document.fragment_tree;
    let container_records: HashSet<usize> = tree
        .records_of(container)
        .iter()
        .copied()
        .filter(|&record| !tree.is_retired(record))
        .collect();
    let mut moved = Vec::new();
    for &later in &children[after..] {
        // The records of the box, or of the boxes inside it that a wrapper
        // without a record of its own left to the container's. What a box
        // with a record holds is placed in that record and moves with it.
        let mut stack = vec![later];
        while let Some(id) = stack.pop() {
            let mut recorded = false;
            let live = tree
                .records_of(id)
                .iter()
                .filter(|&&record| !tree.is_retired(record));
            for &record in live {
                recorded = true;
                if tree.fragments[record]
                    .parent
                    .is_some_and(|parent| container_records.contains(&parent))
                {
                    moved.push(record);
                }
            }
            if !recorded {
                stack.extend(document.nodes[id].children.iter().copied());
            }
        }
    }
    for record in moved {
        document.fragment_tree.fragments[record].translate(0.0, delta);
    }
    for &later in &children[after..] {
        shift_block_sibling(document, later, delta);
    }
}

/// Moves the box `sibling`, one of the boxes after a moved child in block
/// flow, down by `delta`. Text, boxes that are not rendered and positioned
/// boxes keep their places.
fn shift_block_sibling(document: &mut Document, sibling: usize, delta: f32) {
    if document.nodes[sibling].kind() != NodeKind::Element
        || document.nodes[sibling].display() == DisplayValue::None
        || matches!(
            document.nodes[sibling].position(),
            PositionValue::Absolute | PositionValue::Fixed
        )
    {
        return;
    }
    #[cfg(test)]
    tests::record_normal_sibling_work(1, 1);
    document.translate_pagination_y(sibling, delta);
}

/// The side and the normal-flow outer top, bottom and width of a moved
/// float.
#[derive(Clone, Copy, PartialEq)]
struct FloatEdge {
    node: usize,
    side: usize,
    top: f32,
    bottom: f32,
    width: f32,
}

/// How far a box whose top edge is at `top` moves down to reach `floor`,
/// the position it may not start above, or zero without a floor. The move
/// is at most `limit`, the shift the floats above it have taken.
fn clearance_shift(floor: Option<f32>, top: f32, limit: f32) -> f32 {
    floor.map_or(0.0, |floor| (floor - top).clamp(0.0, limit))
}

/// The margin-box edge of a float that moved to `flow_target_y`, its
/// border-box top in normal-flow coordinates. An unforced break truncates
/// the block-start margin (CSS Fragmentation 3 §5.2), so the margin box then
/// starts at the border box; after a forced break it keeps `margin_top`.
fn moved_float_edge(
    document: &Document,
    node_id: usize,
    side: usize,
    flow_target_y: f32,
    height: f32,
    margin_top: f32,
    forced_transition: bool,
) -> FloatEdge {
    let kept_margin_top = if forced_transition { margin_top } else { 0.0 };
    float_margin_edge(
        document,
        node_id,
        side,
        flow_target_y,
        height,
        kept_margin_top,
    )
}

/// The margin-box edge on `side` of the float `node_id` whose border box,
/// `height` tall, starts at `border_top` in normal-flow coordinates, below a
/// block-start margin of `margin_top`.
fn float_margin_edge(
    document: &Document,
    node_id: usize,
    side: usize,
    border_top: f32,
    height: f32,
    margin_top: f32,
) -> FloatEdge {
    let layout = &document.nodes[node_id].layout;
    FloatEdge {
        node: node_id,
        side,
        top: border_top - margin_top,
        bottom: border_top + height + layout.margin.bottom,
        width: layout.size.width + layout.margin.left + layout.margin.right,
    }
}

/// The flow position that `node_id` may not start above, given the moved
/// float `edges` of its formatting context `float_context`, or `None` when
/// they do not constrain it.
///
/// A later float's outer top stays not above an earlier float (CSS 2.1
/// §9.5.1), and below the floats it does not fit beside in the context. An
/// in-flow box's border box stays below a float it clears (CSS 2.1 §9.5.2).
fn float_floor(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    float_context: usize,
    edges: &[FloatEdge],
    floating: bool,
) -> Option<f32> {
    let cleared = edges
        .iter()
        .filter(|edge| {
            clears_float_side(
                cascade.computed[node_id].clear,
                [FloatValue::Left, FloatValue::Right][edge.side],
            )
        })
        .map(|edge| edge.bottom)
        .reduce(f32::max);
    if !floating {
        return cleared;
    }
    let layout = &document.nodes[node_id].layout;
    let outer_width = layout.size.width + layout.margin.left + layout.margin.right;
    let outer_height = layout.size.height + layout.margin.top + layout.margin.bottom;
    let context_width = document.nodes[float_context].layout.content_box_width();
    let top = float_top_beside(edges, outer_width, outer_height, context_width);
    Some(top.max(cleared.unwrap_or(f32::NEG_INFINITY)))
}

/// The highest outer top, at or below the highest top of `edges`, from
/// which a float of `outer_width` by `outer_height` fits beside them along
/// its whole height in a context of `context_width` (CSS 2.1 §9.5.1).
///
/// The candidate tops are the floats' highest top and each bottom below it,
/// where the width they take next changes. A float that fits beside none of
/// them has an infinite top. With no edges, the top is negative infinity
/// unless the float is wider than the context.
fn float_top_beside(
    edges: &[FloatEdge],
    outer_width: f32,
    outer_height: f32,
    context_width: f32,
) -> f32 {
    let top = edges
        .iter()
        .map(|edge| edge.top)
        .fold(f32::NEG_INFINITY, f32::max);
    // The width the floats take at `level`: those that started at or above
    // it and still reach below it sit side by side.
    let width_at = |level: f32| -> f32 {
        edges
            .iter()
            .filter(|edge| edge.top <= level + 0.001 && edge.bottom > level + 0.001)
            .map(|edge| edge.width)
            .sum()
    };
    let fits_from = std::iter::once(top)
        .chain(
            edges
                .iter()
                .map(|edge| edge.bottom)
                .filter(|&bottom| bottom > top),
        )
        .filter(|&level| {
            let end = level + outer_height.max(0.0);
            let beside = std::iter::once(level)
                .chain(
                    edges
                        .iter()
                        .map(|edge| edge.top)
                        .filter(|&edge_top| edge_top > level && edge_top < end),
                )
                .map(width_at)
                .fold(0.0, f32::max);
            outer_width + beside <= context_width + 0.001
        })
        .fold(f32::INFINITY, f32::min);
    top.max(fits_from)
}

/// The shifts of the floats moved to a later page, and the edges later
/// floats and clearing boxes keep below.
#[derive(Default)]
struct FloatShifts {
    /// The float whose shift is in effect, the shift, and the page the flow
    /// was on before the float moved.
    ///
    /// A float moved to a later page carries its own content with it, but
    /// the in-flow content after it in the source sits beside or above it,
    /// so the shift ends where the float's subtree does. The page the flow
    /// was on before the float moved is restored with it.
    pending: Option<(usize, f32, u32)>,
    /// The largest shift a later float may still need.
    ///
    /// A later float may not start above an earlier one (CSS 2.1 §9.5.1),
    /// so it keeps the shift of the last moved float until in-flow content
    /// opens a page.
    later: f32,
    /// The page-flow outer edges of the moved floats on each side (left,
    /// right), for each formatting context the floats were placed in.
    ///
    /// A later float in the same context needs to move only as far as it
    /// would otherwise start above one of them, or above its bottom when it
    /// clears that side or does not fit beside it. Floats in other contexts
    /// do not interact.
    edges: HashMap<usize, Vec<FloatEdge>>,
    /// How far the paragraph line pass moved or grew each float it changed,
    /// in all: the clearing boxes after the float may have to follow it that
    /// far.
    line_moved: HashMap<usize, f32>,
}

/// A float that a page break moved down.
struct MovedFloat {
    node_id: usize,
    /// The side the float sits on, or `None` for a float on neither side.
    side: Option<usize>,
    /// How far the float moved down.
    delta: f32,
    /// The page the flow was on before the float moved.
    flow_page: u32,
    /// Where the float's border box starts after the move, in normal-flow
    /// coordinates.
    flow_target_y: f32,
    /// The float's border-box height.
    height: f32,
    /// The float's block-start margin.
    margin_top: f32,
    /// Whether a forced break moved the float.
    forced_transition: bool,
}

/// A box that a page transition moved down, as the floats of its formatting
/// context see it.
struct FloatTransition {
    /// Whether the box moved because it would cross a page edge it avoids.
    page_overflow: bool,
    /// Whether a forced or page-type break moved the box.
    forced_transition: bool,
    /// How far the box moved down.
    delta: f32,
    /// Where the box's border box starts after the move.
    target_y: f32,
    /// The relative offset of the box and its ancestors.
    relative_offset: f32,
    /// The page the flow was on before the box moved.
    flow_page: u32,
    /// The box's border-box height.
    height: f32,
    /// The box's block-start margin.
    margin_top: f32,
    /// Whether an ancestor's height kept it from holding the moved content.
    moved_in_bounded_ancestor: bool,
}

/// A box that has to start below the moved floats of its formatting context.
struct FloatClearance {
    /// The formatting context the floats are in.
    context: usize,
    /// Whether the box is itself a float, rather than a box that clears one.
    floating: bool,
    /// The unpaginated top of the box in normal-flow coordinates, without
    /// the relative offsets of the box and its ancestors.
    flow_raw_y: f32,
    /// How far the box moves down.
    shift: f32,
}

impl FloatShifts {
    /// How `candidate`, at `flow_shift` into the flow, has to move down to
    /// stay below the moved floats of its formatting context, or `None`
    /// when it does not.
    fn clearance_for(
        &self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
        candidate: &PageCandidate,
        flow_shift: f32,
    ) -> Option<FloatClearance> {
        let node_id = candidate.node_id;
        let (context, edges) = self.edges_for(document, cascade, node_id, body_id)?;
        let floating = is_floating_box_for_pagination(document, cascade, node_id);
        let floor = if !floating && candidate.context.is_float_descendant() {
            None
        } else {
            float_floor(document, cascade, node_id, context, edges, floating)
        };
        let margin_top = if floating {
            document.nodes[node_id].layout.margin.top
        } else {
            0.0
        };
        // Floors are in normal-flow coordinates, without the relative
        // offsets of the box and its ancestors.
        let follower_offset = accumulated_relative_offset_y(document, cascade, body_id, node_id);
        let flow_raw_y = candidate.raw_y - follower_offset;
        let shift = clearance_shift(floor, flow_raw_y + flow_shift - margin_top, self.later);
        (shift > 0.0).then_some(FloatClearance {
            context,
            floating,
            flow_raw_y,
            shift,
        })
    }

    /// Makes the float `node_id`, which `clearance` moved down, the one
    /// later floats on its side keep below. `flow_shift` includes that move
    /// and `flow_page` is the page the flow was on before it.
    fn hold_cleared_float(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        clearance: &FloatClearance,
        flow_shift: f32,
        flow_page: u32,
    ) {
        self.pending = Some((node_id, clearance.shift, flow_page));
        if let Some(side) = float_side_index(cascade.computed[node_id].float) {
            let layout = &document.nodes[node_id].layout;
            let edge = float_margin_edge(
                document,
                node_id,
                side,
                clearance.flow_raw_y + flow_shift,
                layout.size.height,
                layout.margin.top,
            );
            self.record_edge(clearance.context, edge);
        }
    }

    /// Records `moved`: adds its shift to the pending one and, for a float
    /// on a side, keeps its margin-box edge for the floats and clearing
    /// boxes that follow it in the same formatting context.
    fn record_moved_float(
        &mut self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
        moved: &MovedFloat,
    ) {
        let inherited = self.add_to_pending(moved.node_id, moved.delta, moved.flow_page);
        // A clearing follower stays below the margin box, which a negative
        // block-end margin pulls up.
        if let Some(side) = moved.side {
            let context = float_context_root(document, cascade, moved.node_id, body_id);
            self.record_edge(
                context,
                moved_float_edge(
                    document,
                    moved.node_id,
                    side,
                    moved.flow_target_y,
                    moved.height,
                    moved.margin_top,
                    moved.forced_transition,
                ),
            );
            // The margin box then moved further than the border box, and a
            // later float may have to follow it that far.
            if !moved.forced_transition {
                self.extend_later(inherited + moved.delta + moved.margin_top);
            }
        }
    }

    /// Settles the floats of the formatting context after a page transition
    /// moved the box `candidate`. A float that moved across a page edge it
    /// avoids is recorded for the floats and clearing boxes after it, and
    /// grows the context that holds it. A box in the normal flow that opened
    /// a page ends the float edges above it. A box inside a float moves with
    /// it and changes nothing.
    fn settle_transition(
        &mut self,
        document: &mut super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
        candidate: &PageCandidate,
        moved: &FloatTransition,
        shifts: &mut SubtreeShifts,
    ) {
        let node_id = candidate.node_id;
        if moved.page_overflow && is_floating_box_for_pagination(document, cascade, node_id) {
            self.record_moved_float(
                document,
                cascade,
                body_id,
                &MovedFloat {
                    node_id,
                    side: float_side_index(cascade.computed[node_id].float),
                    delta: moved.delta,
                    flow_page: moved.flow_page,
                    // Floors are kept in normal-flow coordinates, without the
                    // relative offsets of the ancestors.
                    flow_target_y: moved.target_y - moved.relative_offset,
                    height: moved.height,
                    margin_top: moved.margin_top,
                    forced_transition: moved.forced_transition,
                },
            );
            grow_float_context_to_moved_float(
                document,
                cascade,
                node_id,
                body_id,
                moved.height,
                shifts,
                moved.flow_page,
            );
        } else if !candidate.context.is_float_descendant()
            && !is_floating_box_for_pagination(document, cascade, node_id)
        {
            let flow_top = moved.target_y - moved.relative_offset;
            self.drop_edges_for_in_flow_candidate(
                document,
                cascade,
                node_id,
                body_id,
                flow_top,
                moved.moved_in_bounded_ancestor,
            );
        }
    }

    /// Add `delta` to the pending shift of the float `node_id`, and return
    /// the shift the float already had.
    ///
    /// A float with no pending shift of its own starts one from
    /// `flow_page`; one already shifted keeps the page it moved from.
    fn add_to_pending(&mut self, node_id: usize, delta: f32, flow_page: u32) -> f32 {
        let (inherited, flow_page) = self
            .pending
            .filter(|(float_id, _, _)| *float_id == node_id)
            .map_or((0.0, flow_page), |(_, delta, page)| (delta, page));
        self.pending = Some((node_id, inherited + delta, flow_page));
        inherited
    }

    /// End the pending shift when `node_id` is outside the moved float's
    /// subtree, and return the shift and the page to restore.
    fn end_pending_outside(&mut self, document: &Document, node_id: usize) -> Option<(f32, u32)> {
        let (float_id, delta, flow_page) = self.pending?;
        if is_descendant_or_self(document, node_id, float_id) {
            return None;
        }
        self.pending = None;
        // A follower shifted less than the float before it keeps the
        // larger budget, which the other side's floor may still need.
        self.extend_later(delta);
        Some((delta, flow_page))
    }

    /// The formatting context of `node_id` and the edges of the moved
    /// floats in it, while a later float may still need to move and no
    /// float shift is pending.
    fn edges_for(
        &self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        node_id: usize,
        body_id: usize,
    ) -> Option<(usize, &[FloatEdge])> {
        (self.later != 0.0 && self.pending.is_none())
            .then(|| {
                let context = float_context_root(document, cascade, node_id, body_id);
                self.edges
                    .get(&context)
                    .map(|edges| (context, edges.as_slice()))
            })
            .flatten()
    }

    /// Records the floats around the paragraph `block_id`, which holds the
    /// text `candidate` and whose lines the paragraph line pass moved down,
    /// growing or moving it by `amount`, for the boxes that clear the floats
    /// and the floats after them in their formatting contexts.
    fn record_grown_floats(
        &mut self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
        candidate: &PageCandidate,
        block_id: usize,
        amount: f32,
    ) {
        if amount <= 0.0 || !candidate.context.is_float_descendant() {
            return;
        }
        let mut node = Some(block_id);
        while let Some(id) = node.filter(|&id| id != body_id) {
            if let Some(side) = float_side_index(cascade.computed[id].float)
                && is_floating_box_for_pagination(document, cascade, id)
            {
                let layout = &document.nodes[id].layout;
                // Floors are kept in normal-flow coordinates, without the
                // relative offsets of the float and its ancestors.
                let top = current_abs_y(document, id)
                    - accumulated_relative_offset_y(document, cascade, body_id, id);
                let mut edge = float_margin_edge(
                    document,
                    id,
                    side,
                    top,
                    layout.size.height,
                    layout.margin.top,
                );
                let context = float_context_root(document, cascade, id, body_id);
                // A float recorded before keeps the top it had then: a page
                // break may have truncated its block-start margin.
                if let Some(earlier) = self
                    .edges
                    .get(&context)
                    .and_then(|edges| edges.iter().find(|earlier| earlier.node == id))
                {
                    edge.top = earlier.top;
                }
                self.record_edge(context, edge);
                let moved = self.line_moved.entry(id).or_default();
                *moved += amount;
                let moved = *moved;
                self.extend_later(moved);
            }
            node = document.layout_parent_of(id);
        }
    }

    /// Raise the shift later floats may still need to at least `shift`.
    fn extend_later(&mut self, shift: f32) {
        self.later = self.later.max(shift);
    }

    /// Add a moved float to the floats of its formatting context. A float
    /// that moves again replaces its earlier place.
    fn record_edge(&mut self, context: usize, edge: FloatEdge) {
        let edges = self.edges.entry(context).or_default();
        edges.retain(|earlier| earlier.node != edge.node);
        edges.push(edge);
    }

    /// Drop the edges of `context` that do not reach below `flow_top` (within
    /// a small tolerance), and the later-float shift once no edges remain in
    /// any context.
    fn drop_edges_above(&mut self, context: usize, flow_top: f32) {
        if let Some(edges) = self.edges.get_mut(&context) {
            edges.retain(|edge| edge.bottom > flow_top + 0.001);
            if edges.is_empty() {
                self.edges.remove(&context);
            }
        }
        if self.edges.is_empty() {
            self.later = 0.0;
        }
    }

    /// Drop the ended float edges in the formatting context of an in-flow
    /// candidate that opened a page. A moved box inside a bounded ancestor
    /// leaves the outer flow where it was, so its floors stay.
    fn drop_edges_for_in_flow_candidate(
        &mut self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        node_id: usize,
        body_id: usize,
        flow_top: f32,
        moved_in_bounded_ancestor: bool,
    ) {
        if moved_in_bounded_ancestor {
            return;
        }
        let context = float_context_root(document, cascade, node_id, body_id);
        self.drop_edges_above(context, flow_top);
    }
}

/// Whether a float or a block with `clear` comes after the float `float` in
/// the float context `root`, outside the float itself. Independent
/// formatting contexts inside `root` place their own floats and are skipped.
fn floats_or_clearance_follow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    root: usize,
    float: usize,
) -> bool {
    later_in_float_context(document, cascade, root, float, |id| {
        is_floating_box_for_pagination(document, cascade, id)
            || cascade.computed.get(id).is_some_and(|computed| {
                computed.clear != ClearValue::None && !document.nodes[id].display().is_inline_flow()
            })
    })
}

/// Whether a box for which `hit` holds comes after the float `float` in the
/// float context `root`, outside the float itself. Independent formatting
/// contexts inside `root` place their own floats and are skipped, after
/// `hit` sees their root.
fn later_in_float_context(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    root: usize,
    float: usize,
    mut hit: impl FnMut(usize) -> bool,
) -> bool {
    let mut stack: Vec<usize> = document.nodes[root]
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    let mut after = false;
    while let Some(id) = stack.pop() {
        if id == float {
            after = true;
            continue;
        }
        if after && hit(id) {
            return true;
        }
        if after && holds_floats(document, cascade, id) {
            continue;
        }
        stack.extend(document.nodes[id].children.iter().rev().copied());
    }
    false
}

/// The box whose block formatting context places the floats and clearance
/// of `node_id`: its nearest ancestor that establishes an independent one
/// (CSS 2.1 §9.4.1), or `body_id`. Floats in different contexts do not
/// interact.
fn float_context_root(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    body_id: usize,
) -> usize {
    let mut current = node_id;
    while let Some(parent) = BoxTree::for_layout(document).parent(current, BoxParentScope::Document)
        && parent != body_id
    {
        if holds_floats(document, cascade, parent) {
            return parent;
        }
        current = parent;
    }
    body_id
}

/// Whether the box `node_id` clears floats in the block formatting context
/// of a table cell, as a child of the cell's flow, and the cell's table is
/// not inside another table cell: a box that clearance moves there takes the
/// boxes after it and the cell's row along (see [`shift_block_flow_after`]).
fn clears_in_cell_flow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    body_id: usize,
) -> bool {
    let context = float_context_root(document, cascade, node_id, body_id);
    document.nodes[context].display() == DisplayValue::TableCell
        && moves_reach_cell_row(document, node_id)
        && !document
            .ancestors(context)
            .find(|&id| document.nodes[id].display() == DisplayValue::Table)
            .is_some_and(|table| moves_reach_cell_row(document, table))
}

/// Whether `node_id` is the root of an inline formatting context or an
/// inline inside one, whose boxes sit in the root's coordinates.
fn in_inline_context(document: &Document, node_id: usize) -> bool {
    document
        .inclusive_ancestors(node_id)
        .find(|&id| {
            let display = document.nodes[id].display();
            document.nodes[id].is_ifc_root()
                || !(display.is_inline_flow() || display == DisplayValue::Contents)
        })
        .is_some_and(|id| document.nodes[id].is_ifc_root())
}

/// Whether the box `node_id` establishes an independent block formatting
/// context (CSS 2.1 §9.4.1), which holds the floats inside it.
fn holds_floats(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    holds_floats_by_style(document, cascade, node_id) || is_flex_or_grid_item(document, node_id)
}

/// [`holds_floats`] from the computed style alone, for the passes that run
/// before layout may query the style views: a box whose `overflow` is not
/// `visible` or `clip` counts as a scroll container.
fn holds_floats_in_preparation(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    use raikiri_style::property::OverflowValue;
    let computed = &cascade.computed[node_id];
    document.nodes[node_id].layout_display() == Display::FlowRoot
        || !matches!(
            document.nodes[node_id].display(),
            DisplayValue::Block | DisplayValue::ListItem
        )
        || [computed.overflow.x, computed.overflow.y]
            .into_iter()
            .any(|value| {
                matches!(
                    value,
                    OverflowValue::Hidden | OverflowValue::Scroll | OverflowValue::Auto
                )
            })
        || document.nodes[node_id].multicol.is_some()
        || document.nodes[node_id].roots_block_context()
        || establishes_containment_context(computed)
        || is_flex_or_grid_item(document, node_id)
}

/// Whether the style of the box `node_id` alone makes it hold the floats
/// inside it, without a look at the box around it.
fn holds_floats_by_style(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let computed = &cascade.computed[node_id];
    document.nodes[node_id].layout_display() == Display::FlowRoot
        || !(document.nodes[node_id].display().is_inline_flow()
            || matches!(
                document.nodes[node_id].display(),
                DisplayValue::Block | DisplayValue::ListItem
            ))
        || matches!(computed.float, FloatValue::Left | FloatValue::Right)
        || document.nodes[node_id].layout_position() == Position::Absolute
        || scrolls_overflow(document, computed, node_id)
        || document.nodes[node_id].multicol.is_some()
        || document.nodes[node_id].roots_block_context()
        || establishes_containment_context(computed)
}

/// Whether the box `node_id` is an item of a flex or grid container.
fn is_flex_or_grid_item(document: &super::context::LayoutSession<'_>, node_id: usize) -> bool {
    BoxTree::for_layout(document)
        .parent(node_id, BoxParentScope::Document)
        .is_some_and(|container| {
            document.nodes[container]
                .formatting_context
                .is_flex_or_grid()
        })
}

/// Whether the box `node_id` is a scroll container on either axis.
///
/// The only override of `overflow` hands a viewport-propagated value back as
/// `visible`, so a box whose computed `overflow` does not scroll never does.
/// The override-aware view is read for the other boxes only, which keeps the
/// common case out of the hot ancestor walks. Style comparison builds assert
/// the shortcut against the view for every box they lay out, so an override
/// that made a `visible` box scroll has to teach this function first.
pub(super) fn scrolls_overflow(
    document: &super::context::LayoutSession<'_>,
    computed: &raikiri_style::ComputedValues,
    node_id: usize,
) -> bool {
    use raikiri_style::property::OverflowValue;
    let scrolls = || {
        let overflow = document.pagination_style(node_id).overflow();
        overflow.x.is_scroll_container() || overflow.y.is_scroll_container()
    };
    let may_scroll = [computed.overflow.x, computed.overflow.y]
        .into_iter()
        .any(|value| {
            matches!(
                value,
                OverflowValue::Hidden | OverflowValue::Scroll | OverflowValue::Auto
            )
        });
    debug_assert!(may_scroll || !scrolls(), "a visible box scrolls");
    may_scroll && scrolls()
}

/// Whether layout or paint containment makes the box an independent
/// formatting context (CSS Containment 2 §3.1, §3.3).
fn establishes_containment_context(computed: &raikiri_style::ComputedValues) -> bool {
    computed
        .contain
        .contains(raikiri_style::property::Contain::LAYOUT)
        || computed
            .contain
            .contains(raikiri_style::property::Contain::PAINT)
}

/// The index of a float side in `FloatEdge::side`.
fn float_side_index(side: FloatValue) -> Option<usize> {
    match side {
        FloatValue::Left => Some(0),
        FloatValue::Right => Some(1),
        _ => None,
    }
}

/// Whether `clear` requires a box to start below a float on `side`. Layout
/// does not apply the logical values, so neither does pagination.
fn clears_float_side(clear: ClearValue, side: FloatValue) -> bool {
    match clear {
        ClearValue::Left => side == FloatValue::Left,
        ClearValue::Right => side == FloatValue::Right,
        ClearValue::Both => true,
        _ => false,
    }
}

/// The border-box height that the `max-height` of `node_id` allows, when
/// it resolves. A percentage needs a containing block of definite height.
fn max_border_box_height(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<f32> {
    let basis = BoxTree::for_layout(document)
        .parent(node_id, BoxParentScope::Document)
        .and_then(|parent| definite_content_height(document, parent));
    let max = match &cascade.computed[node_id].max_height {
        ComputedLengthPercentageOrAuto::Px(max) => Some(*max),
        value => basis.and_then(|basis| used_computed_length_percentage_or_auto(value, basis)),
    }?;
    let layout = &document.nodes[node_id].layout;
    let edges = if document.pagination_style(node_id).box_sizing() == BoxSizing::ContentBox {
        layout.padding.top + layout.padding.bottom + layout.border.top + layout.border.bottom
    } else {
        0.0
    };
    Some(max + edges)
}

/// Whether the `max-height` of `node_id` already limits its height.
fn max_height_is_reached(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    max_border_box_height(document, cascade, node_id)
        .is_some_and(|max| document.nodes[node_id].layout.size.height >= max - 0.01)
}

/// Whether the items of `grid` stack in block order like blocks, none
/// beside or over another: the grid has one column track, no definite
/// height and no `min-height` that could stretch its rows, its rows all
/// size to their content, and its items sit in rows that follow one
/// another without overlapping. A sized row fixes the height of the item
/// stretched into it, so that item cannot grow to hold a moved box.
fn grid_stacks_items(document: &super::context::LayoutSession<'_>, grid: usize) -> bool {
    let node = &document.nodes[grid];
    let block_axis = document.block_axis(grid);
    if node.grid_column_count != 1
        || !grid_rows_size_to_content(document, grid)
        || !block_axis.height().is_auto() && definite_content_height(document, grid).is_some()
        || !block_axis.min_height().is_auto()
    {
        return false;
    }
    let mut items: Vec<(u16, f32, f32)> = node
        .grid_item_row_starts
        .iter()
        .map(|&(item, row)| {
            let layout = &document.nodes[item].layout;
            (
                row,
                layout.location.y,
                layout.location.y + layout.size.height,
            )
        })
        .collect();
    items.sort_by_key(|&(row, ..)| row);
    items
        .windows(2)
        .all(|pair| pair[0].0 < pair[1].0 && pair[1].1 >= pair[0].2 - 0.01)
}

/// Whether the rows of `grid` all size to their content: every explicit
/// and implicit row track is intrinsic and uncapped, so its items grow
/// with what they hold.
fn grid_rows_size_to_content(document: &super::context::LayoutSession<'_>, grid: usize) -> bool {
    super::taffy_adapter::rows_size_to_content(&document.pagination_style(grid))
}

/// Find both ancestor conditions that prevent moving a nested candidate with one walk.
/// The body can make siblings side by side, but its own clipping is not bounded here.
///
/// `holder` is the ancestor that moves with the candidate (see
/// [`leading_holder`]), if there is one. It and the ancestors between it and
/// the candidate move whole, so they need not hold the candidate where it
/// was: only the ancestors above the holder count.
fn candidate_ancestor_constraints(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    body_id: usize,
    holder: Option<usize>,
) -> (bool, bool) {
    let mut side_by_side = false;
    let mut bounded_clipping = false;
    // A holder that is not among the ancestors takes none of them along.
    let holder = holder.filter(|&holder| document.ancestors(node_id).any(|id| id == holder));
    let mut moves_whole = holder.is_some();
    for id in document.ancestors(node_id) {
        let moves_with_candidate = moves_whole;
        moves_whole &= Some(id) != holder;
        if moves_with_candidate {
            continue;
        }
        if id != body_id && !bounded_clipping {
            bounded_clipping = clips_at_bounded_height(document, cascade, id);
        }
        side_by_side = side_by_side || puts_boxes_side_by_side(document, id);
        if (side_by_side && bounded_clipping) || id == body_id {
            break;
        }
    }
    (side_by_side, bounded_clipping)
}

/// Whether `id` clips its overflow in the block direction at a height of its
/// own, so that what overflows it takes no room on the pages.
fn clips_at_bounded_height(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    id: usize,
) -> bool {
    !matches!(cascade.computed[id].overflow.y, OverflowValue::Visible)
        && crate::paint_rules::clips_element_overflow(document, cascade, id)
        && (definite_content_height(document, id).is_some()
            || max_height_is_reached(document, cascade, id))
}

/// Whether `id` lays its children out beside one another rather than one
/// after another down the page: a multicol container, a flex container, or a
/// grid whose items do not stack like blocks.
fn puts_boxes_side_by_side(document: &super::context::LayoutSession<'_>, id: usize) -> bool {
    let formatting_context = document.nodes[id].formatting_context;
    document.nodes[id].multicol.is_some()
        || formatting_context == FormattingContext::Flex
        || formatting_context == FormattingContext::Grid && !grid_stacks_items(document, id)
}

/// Where the bottom padding and border of `node_id` can begin the next page
/// when they do not fit below its content: `Some(None)` when no ancestor puts
/// boxes beside one another, so the flow after the box follows it, and
/// `Some(Some(container))` when it sits in an item of one flex or grid
/// container that does, so that only the item, the container and what
/// follows them move. `None` when the edges stay where they are: in columns,
/// whose balance the move would change, or beside boxes at more than one
/// level.
fn edges_break_container(
    document: &super::context::LayoutSession<'_>,
    node_id: usize,
    body_id: usize,
) -> Option<Option<usize>> {
    let mut container = None;
    for id in document.ancestors(node_id) {
        if puts_boxes_side_by_side(document, id) {
            if document.nodes[id].multicol.is_some() || container.is_some() {
                return None;
            }
            container = Some(id);
        }
        if id == body_id {
            break;
        }
    }
    Some(container)
}

/// The paragraph that lays `block` out as a block among its lines, whose
/// content after the block follows it with its lines.
fn ifc_paragraph_of_block(document: &Document, block: usize) -> Option<usize> {
    document
        .layout_parent_of(block)
        .filter(|&root| lines_hold_block(document, root, block))
}

/// Whether the paragraph `root` lays `block` out as a block among its lines.
fn lines_hold_block(document: &Document, root: usize, block: usize) -> bool {
    document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .is_some_and(|lines| {
            lines
                .block_line_starts
                .iter()
                .any(|&(child, _)| child == block)
        })
}

/// The anonymous table cell that wraps `node_id` or the ancestor of it that
/// is a child of a table part (CSS 2.1 §17.2.1), if table fixup wrapped one.
fn wrapping_anonymous_cell(document: &Document, node_id: usize) -> Option<usize> {
    let mut id = node_id;
    loop {
        if let Some(cell) = document.anonymous_cell_of(id) {
            return Some(cell);
        }
        let parent = document.parent_of(id)?;
        if matches!(
            document.nodes[parent].display(),
            DisplayValue::Table
                | DisplayValue::InlineTable
                | DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup
                | DisplayValue::TableRow
                | DisplayValue::TableCell
        ) {
            return None;
        }
        id = parent;
    }
}

/// Whether `id` stacks its children in block order, so that the boxes
/// after a moved one follow it: a block container, or a grid whose items
/// stack like blocks.
fn stacks_children_in_block_flow(document: &super::context::LayoutSession<'_>, id: usize) -> bool {
    moves_later_siblings(document, id)
        || document.nodes[id].formatting_context == FormattingContext::Grid
            && grid_stacks_items(document, id)
}

/// Whether `candidate` lies in the normal flow, where its margins can adjoin
/// the margins of the boxes before and after it: it is not a float, inside a
/// float or a monolithic box, out of flow, or inside an out-of-flow box.
fn participates_in_margin_adjacency(
    document: &Document,
    cascade: &CascadeResult,
    candidate: &PageCandidate,
) -> bool {
    !candidate.context.is_float_descendant()
        && !candidate.context.is_out_of_flow_descendant()
        && !is_floating_box_for_pagination(document, cascade, candidate.node_id)
}

/// Whether the block-start margin of `candidate` can collapse with the
/// block-end margin of the block before it: it is a block-level box in the
/// normal flow, static, relatively positioned or sticky, and not text, a
/// table row, or a flex or grid item. A relative offset only paints the box
/// elsewhere, so the margin is measured without it.
fn collapses_with_previous_block(
    document: &Document,
    candidate: &PageCandidate,
    participates_in_margin_adjacency: bool,
) -> bool {
    let node = &document.nodes[candidate.node_id];
    is_block_level_flow_box(node.display())
        && matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
        )
        && !candidate.context.is_text()
        && !candidate.context.is_table_row()
        && !candidate.context.is_flex_item()
        && !candidate.context.is_grid_item()
        && participates_in_margin_adjacency
}

/// Moves the boxes after `node_id` in block flow down by `flow_delta`, up to
/// the first ancestor below `body_id` that does not stack its children in
/// block order, for a forced break before `node_id` or for `node_id` moved
/// out of a page in a table cell (see [`moves_reach_cell_row`]). Returns
/// whether an
/// ancestor's height kept it from holding the moved content.
///
/// A `display: contents` ancestor has no box; its parent lays the moved box
/// out among its siblings. A paragraph's content after a block on its lines
/// follows the same way, also when the moved box sits deeper inside that
/// block.
///
/// Each ancestor passes on only as much of the shift as it grows (see
/// [`forced_break_growth`]); the rest ends with its subtree (see
/// [`SubtreeShifts::bounded`]).
fn shift_block_flow_after(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    body_id: usize,
    flow_delta: f32,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
) -> bool {
    shift_flow_after(
        document, cascade, node_id, None, body_id, flow_delta, shifts, flow_page,
    )
}

/// [`shift_block_flow_after`] from `node_id`, where the fragment of the row
/// of a table cell around it on the page the moved content leaves ends
/// `break_end` below the top of the cell, once that content moved, when
/// given, rather than where the margin box of `node_id` now starts: the
/// lines of a paragraph that moved down to there take `node_id` along.
#[allow(clippy::too_many_arguments)]
fn shift_flow_after(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    break_end: Option<f32>,
    body_id: usize,
    flow_delta: f32,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
) -> bool {
    let mut flow_delta = flow_delta;
    let mut bounded = false;
    let mut child = node_id;
    // How far `child` has grown in all, when a table grew on earlier breaks
    // too: its stored height leaves all of that growth out.
    let mut child_total = None;
    let mut walk = AncestorCursor::new(document.parent_of(node_id), ParentRelation::Source);
    while let Some(parent) = walk.current().filter(|&parent| parent != body_id) {
        walk.advance(document);
        // The columns after a spanner, in a container whose children are
        // boxes, follow it down.
        let step = if spans_box_columns(document, cascade, child, parent) {
            FlowStep::Flow { moved_lines: false }
        } else {
            flow_step(document, child, parent)
        };
        let moved_lines = match step {
            FlowStep::Skip => continue,
            FlowStep::Inline => {
                child = parent;
                continue;
            }
            FlowStep::Stop => break,
            FlowStep::Flow { moved_lines } => moved_lines,
            FlowStep::Cell(cell) => {
                // A table cell's `height` is a minimum: the row grows only as
                // far as the cell's moved content ends past it (CSS 2.1
                // §17.5.3). The cells beside it keep their places, and the
                // flow after the row follows its growth.
                let parent = cell;
                if lines_hold_block(document, cell, child) {
                    // The lines after a block on the cell's lines follow it; the
                    // moved box itself already took them along.
                    if child != node_id {
                        let layout = document.nodes[child].layout;
                        let top = document.pagination_local_y(child);
                        let _ = follow_ifc_block(
                            document,
                            cell,
                            child,
                            top,
                            top + layout.size.height,
                            flow_delta,
                        );
                    }
                } else if !document.nodes[child].in_ifc_subtree() {
                    shift_later_flow_siblings(document, cell, child, flow_delta);
                }
                // The box the cell holds may have grown on earlier breaks too.
                let child_growth = if child == node_id {
                    0.0
                } else {
                    shifts.box_growth(child)
                };
                let passed = moved_content_growth(
                    document,
                    cascade,
                    body_id,
                    parent,
                    (child, child_growth),
                    f32::INFINITY,
                );
                bounded = true;
                let row_end = cell_row_end(document, parent);
                // A row grows as far as its cell that grows most, and an
                // earlier break in it may have grown it already: only the
                // rest of the growth moves what follows.
                // Cells outside a table share the anonymous row that wraps
                // their run (CSS 2.1 §17.2.1).
                let loose_run = row_end
                    .is_none()
                    .then(|| loose_cell_run(document, parent))
                    .flatten();
                let holder = cell_growth_holder(document, parent);
                let first_growth = !shifts.grown_rows.iter().any(|(id, _)| *id == holder);
                // A float that moved in the cell grew its stored height
                // already, which `passed` is measured from.
                let before = shifts.row_growth(holder);
                shifts.grow_row(holder, passed + shifts.float_growth(parent));
                let added = shifts.row_growth(holder) - before;
                // The cells of the row, or of the run of cells outside a
                // table, and how tall the row is.
                let row_cells = match (row_end, &loose_run) {
                    (Some((row, _, _)), _)
                        if document.nodes[row].display() == DisplayValue::TableRow =>
                    {
                        Some((
                            document.nodes[row].children.clone(),
                            document.nodes[row].layout.size.height,
                        ))
                    }
                    (None, Some((cells, _, _))) => {
                        Some((cells.clone(), document.nodes[parent].layout.size.height))
                    }
                    _ => None,
                };
                if first_growth
                    && added > 0.001
                    && let Some((cells, row_height)) = row_cells
                {
                    // The break ends the row's fragment on this page where
                    // the margin box of the moved box now starts, at the top
                    // of the next page. A break below the row, in a cell
                    // that spans the rows after it, leaves the row whole.
                    let end = break_end.unwrap_or_else(|| {
                        offset_in_cell(document, cascade, body_id, node_id, parent)
                            - document.nodes[node_id].layout.margin.top
                    });
                    let breaks_row = end - flow_delta < row_height;
                    let realigned = table_cell_fragments::realign_beside(
                        document,
                        cascade,
                        &cells,
                        parent,
                        breaks_row.then_some(end),
                        &shifts.split_cells,
                    );
                    for fragment in realigned {
                        let realigned = fragment.apply(document);
                        shifts.realigned_cells.push(realigned);
                    }
                }
                let Some((row, end, table)) = row_end else {
                    if let Some((_, last, holder)) = loose_run {
                        // The cells beside it in its run sit beside it, as in
                        // a row: only the boxes after the run follow its
                        // growth.
                        shifts.bounded.push((parent, flow_delta, flow_page));
                        if added <= 0.001 {
                            break;
                        }
                        if let Some((run_parent, run_end)) = loose_run_end(document, last) {
                            shifts.from_box.push((run_parent, run_end, added));
                        }
                        // The boxes after the run inside the boxes with
                        // `display: contents` around its end move here; the
                        // walk moves those after them in the holder.
                        let mut inner = last;
                        while let Some(up) = document.parent_of(inner).filter(|&up| up != holder) {
                            shift_later_flow_siblings(document, up, inner, added);
                            inner = up;
                        }
                        flow_delta = added;
                        child_total = Some(shifts.grow_box(parent, added));
                        child = inner;
                        continue;
                    }
                    // A cell outside a table sits in an anonymous table of its
                    // own (CSS 2.1 §17.2.1), which grows with it.
                    if added < flow_delta - 0.001 {
                        shifts.bounded.push((parent, flow_delta - added, flow_page));
                    }
                    flow_delta = added;
                    child_total = Some(shifts.grow_box(parent, added));
                    child = parent;
                    if flow_delta <= 0.001 {
                        break;
                    }
                    continue;
                };
                shifts.bounded.push((parent, flow_delta, flow_page));
                if added <= 0.001 {
                    break;
                }
                shifts.from_box.push((row, end, added));
                shift_rows_after(document, cascade, (row, end), table, added);
                // The table grows with the row: its ancestors take the growth
                // on as they take a moved box's.
                flow_delta = added;
                child_total = Some(shifts.grow_box(table, added));
                child = table;
                walk = AncestorCursor::new(document.parent_of(table), ParentRelation::Source);
                continue;
            }
        };
        // The moved box itself already took the lines after it along.
        if !moved_lines
            && child != node_id
            && ifc_paragraph_of_block(document, child) == Some(parent)
        {
            let layout = document.nodes[child].layout;
            let top = document.pagination_local_y(child);
            let _ = follow_ifc_block(
                document,
                parent,
                child,
                top,
                top + layout.size.height,
                flow_delta,
            );
        }
        if !moved_lines {
            shift_later_block_siblings(document, parent, child, flow_delta);
            if spans_box_columns(document, cascade, child, parent) {
                shift_columns_after_spanner(document, parent, child, flow_delta, child != node_id);
            }
        }
        // The moved box moved whole; an ancestor of it keeps its stored
        // height while its content end moves down with the shift.
        // Moved text grows its paragraph by its lines alone.
        let child_growth = child_total
            .take()
            .unwrap_or(if child == node_id || moved_lines {
                0.0
            } else {
                flow_delta
            });
        let passed = forced_break_growth(
            document,
            cascade,
            body_id,
            parent,
            (child, child_growth),
            flow_delta,
        );
        shifts.grow_box(parent, passed.min(flow_delta));
        if spans_box_columns(document, cascade, child, parent) {
            grow_last_record(document, parent, passed.min(flow_delta));
        }
        if passed < flow_delta - 0.001 {
            shifts
                .bounded
                .push((parent, flow_delta - passed, flow_page));
            bounded = true;
            flow_delta = passed;
            if flow_delta <= 0.001 {
                break;
            }
        }
        child = parent;
    }
    bounded
}

/// Moves the boxes of `table` after a row down by `delta`: those from index
/// `end` on in `row`, the box that holds the row, and those after it up to
/// the table. The flow reaches them `delta` further down.
fn shift_rows_after(
    document: &mut Document,
    cascade: &CascadeResult,
    (row, end): (usize, usize),
    table: usize,
    delta: f32,
) {
    for later in end..document.nodes[row].children.len() {
        shift_block_sibling(document, document.nodes[row].children[later], delta);
    }
    let mut up = row;
    while up != table
        && let Some(parent) = document.parent_of(up)
    {
        // The table puts its header and footer groups first and last
        // (CSS 2.1 §17.2), whatever their place among its children.
        let order = if parent == table {
            super::page::pagination_child_order(document, cascade, table)
        } else {
            document.nodes[parent].children.clone()
        };
        let start = order
            .iter()
            .position(|&id| id == up)
            .map_or(order.len(), |index| index + 1);
        for &later in &order[start..] {
            shift_block_sibling(document, later, delta);
        }
        up = parent;
    }
}

/// Whether a box or text moved down from `node_id` reaches the row of a
/// table cell along the walk of [`shift_block_flow_after`]: every box between
/// them stacks its children in block flow or lays them out among its lines,
/// and the cell (see [`growing_cell`]) can grow its row to hold the moved
/// content.
fn moves_reach_cell_row(document: &super::context::LayoutSession<'_>, node_id: usize) -> bool {
    let mut child = node_id;
    let mut walk = AncestorCursor::new(document.parent_of(node_id), ParentRelation::Source);
    while let Some(parent) = walk.current() {
        walk.advance(document);
        match flow_step(document, child, parent) {
            FlowStep::Skip => {}
            FlowStep::Cell(_) => return true,
            FlowStep::Stop => break,
            FlowStep::Inline | FlowStep::Flow { .. } => child = parent,
        }
    }
    false
}

/// One step of the walk of [`shift_block_flow_after`] from `child`, which
/// holds moved content, to its source parent `parent`.
enum FlowStep {
    /// `parent` generates no box (`display: contents`): its parent lays
    /// `child` out.
    Skip,
    /// `child` and `parent` are inline content of one paragraph, which
    /// moves with its lines.
    Inline,
    /// The table cell whose row grows to hold the moved content (see
    /// [`growing_cell`]).
    Cell(usize),
    /// `parent` takes the moved content on in its flow: it stacks its
    /// children in block flow, or lays `child` out among its lines. With
    /// `moved_lines`, `child` is inline content whose lines already moved.
    Flow { moved_lines: bool },
    /// `parent` takes no moved content on.
    Stop,
}

fn flow_step(
    document: &super::context::LayoutSession<'_>,
    child: usize,
    parent: usize,
) -> FlowStep {
    if document.nodes[parent].display() == DisplayValue::Contents {
        return FlowStep::Skip;
    }
    let moved_lines = document.nodes[child].in_ifc_subtree();
    if moved_lines && document.nodes[parent].in_ifc_subtree() {
        return FlowStep::Inline;
    }
    // An inline element of a paragraph has no box of its own: the paragraph
    // lays the block inside it out among its lines.
    if document.nodes[parent].in_ifc_subtree() && !document.nodes[parent].is_ifc_root() {
        return FlowStep::Skip;
    }
    if let Some(cell) = growing_cell(document, child, parent) {
        return FlowStep::Cell(cell);
    }
    if moved_lines
        || ifc_paragraph_of_block(document, child) == Some(parent)
        || stacks_children_in_block_flow(document, parent)
    {
        FlowStep::Flow { moved_lines }
    } else {
        FlowStep::Stop
    }
}

/// The table cell whose row grows to hold content moved down from `child`,
/// a child of `parent` in the walk of [`shift_block_flow_after`]: `parent`
/// itself, or the anonymous cell that wraps `child` among the boxes of
/// `parent` (CSS 2.1 §17.2.1). A cell that holds columns does not grow
/// this way. In a cell laid out as lines, `child` is a block among the
/// lines or moved inline content.
fn growing_cell(
    document: &super::context::LayoutSession<'_>,
    child: usize,
    parent: usize,
) -> Option<usize> {
    let cell = document.anonymous_cell_of(child).unwrap_or(parent);
    let node = document.ifc_layout_node(cell)?;
    (node.display() == DisplayValue::TableCell
        && node.multicol.is_none()
        && (node.ifc.is_none()
            || document.nodes[child].in_ifc_subtree()
            || lines_hold_block(document, cell, child)))
    .then_some(cell)
}

/// How far below the border-box top of `cell` the border box of `node_id`, a
/// box in it, now lies in the flow, without the relative offsets that only
/// move where the boxes between them are painted.
fn offset_in_cell(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    cell: usize,
) -> f32 {
    let laid_out: f32 = document
        .inclusive_ancestors(node_id)
        .take_while(|&id| id != cell)
        .map(|id| document.nodes[id].layout.location.y)
        .sum();
    laid_out - accumulated_relative_offset_y(document, cascade, body_id, node_id)
        + accumulated_relative_offset_y(document, cascade, body_id, cell)
}

/// The box that holds the row of the table cell `cell`, the index in it
/// after which the boxes follow the row, and the table around them. The row
/// is the cell's parent, or the anonymous row that wraps the run of cells in
/// a table or row group (CSS 2.1 §17.2.1). `None` when the cell is not in a
/// table.
fn cell_row_end(document: &Document, cell: usize) -> Option<(usize, usize, usize)> {
    let mut branch = cell;
    let mut up = document.parent_of(cell)?;
    while document.nodes[up].display() == DisplayValue::Contents {
        branch = up;
        up = document.parent_of(up)?;
    }
    let end = match document.nodes[up].display() {
        DisplayValue::TableRow => document.nodes[up].children.len(),
        DisplayValue::Table
        | DisplayValue::InlineTable
        | DisplayValue::TableRowGroup
        | DisplayValue::TableHeaderGroup
        | DisplayValue::TableFooterGroup => {
            let children = &document.nodes[up].children;
            let index = children.iter().position(|&id| id == branch)?;
            index
                + 1
                + children[index + 1..]
                    .iter()
                    .take_while(|&&id| {
                        !matches!(
                            document.nodes[id].display(),
                            DisplayValue::TableRow
                                | DisplayValue::TableRowGroup
                                | DisplayValue::TableHeaderGroup
                                | DisplayValue::TableFooterGroup
                                | DisplayValue::TableCaption
                        )
                    })
                    .count()
        }
        _ => return None,
    };
    // A row or row group outside a table belongs to the anonymous table that
    // wraps it, which is not among its ancestors and is left to
    // `loose_cell_run`.
    let table = match document.nodes[up].display() {
        DisplayValue::Table | DisplayValue::InlineTable => up,
        _ => document.table_of_row(up)?,
    };
    if document.anonymous_table_parts(table).is_some() {
        return None;
    }
    Some((up, end, table))
}

/// The run of consecutive table cells outside a table that `cell` belongs
/// to, which table fixup wraps in an anonymous row (CSS 2.1 §17.2.1), with
/// its last box (its last cell, or the row outside a table that holds the
/// cells) and the box that holds the anonymous table. `None` when `cell` is
/// not such a cell.
fn loose_cell_run(document: &Document, cell: usize) -> Option<(Vec<usize>, usize, usize)> {
    let (table, cells, last) = match document.anonymous_table_of(cell) {
        Some(table) => {
            let parts = document.anonymous_table_parts(table)?;
            let at = parts.iter().position(|&part| part == cell)?;
            let is_cell =
                |part: &&usize| document.nodes[**part].display() == DisplayValue::TableCell;
            let start = at - parts[..at].iter().rev().take_while(is_cell).count();
            let end = at + 1 + parts[at + 1..].iter().take_while(is_cell).count();
            (table, parts[start..end].to_vec(), parts[end - 1])
        }
        // A row outside a table holds its cells itself.
        None => {
            let row = document
                .ancestors(cell)
                .find(|&id| document.nodes[id].display() != DisplayValue::Contents)?;
            if document.nodes[row].display() != DisplayValue::TableRow {
                return None;
            }
            (
                document.anonymous_table_of(row)?,
                document.nodes[row].children.clone(),
                row,
            )
        }
    };
    Some((cells, last, document.table_parts_boundary(table)))
}

/// The parent of the last box `last` of a run of cells outside a table and
/// the index in it after which the boxes follow the run. Inside a box with
/// `display: contents`, that parent is the wrapper, so that the boxes after
/// the run inside it follow too.
fn loose_run_end(document: &Document, last: usize) -> Option<(usize, usize)> {
    let parent = document.parent_of(last)?;
    let index = document.nodes[parent]
        .children
        .iter()
        .position(|&child| child == last)?;
    Some((parent, index + 1))
}

/// The box whose growth a table cell's growth adds to, as a key of the
/// growth recorded so far, and the boxes after which the flow follows it:
/// the cell's row, or the run of cells outside a table that it belongs to,
/// or the cell alone.
fn cell_growth_holder(document: &Document, cell: usize) -> (usize, usize) {
    cell_row_end(document, cell)
        .map(|(row, end, _)| (row, end))
        .or_else(|| {
            loose_cell_run(document, cell).and_then(|(_, last, _)| loose_run_end(document, last))
        })
        .unwrap_or((cell, 0))
}

/// How far `block` grows when a forced break inside its child `child` moves
/// the content from there on down by `moved`, while `child` itself keeps its
/// stored height and grows by `child_growth`.
///
/// A box with a definite height that a break goes through fills the rest of
/// its fragmentainer and continues on the next one with the rest of that
/// height (CSS Fragmentation 3 §4.4), so it does not grow, and neither does a
/// box whose size containment ignores its content. The others grow to hold
/// the moved content within `min-height` and `max-height` (CSS 2.1 §10.7),
/// and a box that holds its floats keeps the room they take.
fn forced_break_growth(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    block: usize,
    (child, child_growth): (usize, f32),
    moved: f32,
) -> f32 {
    if !lines_move_grows_box(document, block) {
        return 0.0;
    }
    let (min, max) = border_box_height_bounds(document, block);
    let old = document.nodes[block].layout.size.height;
    let holds = holds_floats(document, cascade, block);
    // A height between its bounds follows the end of the in-flow content,
    // which moves down by the whole shift. The columns after a spanner sit
    // below it, floats and all, so a multicol container's content ends
    // that much lower too.
    let spanned =
        document.nodes[block].multicol.is_some() && super::is_spanner_in(document, cascade, child);
    if (!holds || spanned) && old > min + 0.001 && max.is_infinite() {
        return moved;
    }
    let growth = moved_content_growth(
        document,
        cascade,
        body_id,
        block,
        (child, child_growth),
        max,
    );
    if !holds {
        return growth;
    }
    // A box that holds its floats ends below them too (CSS 2.1 §10.6.7), and
    // the floats after the break moved down with the flow.
    let float_end = held_float_end(document, cascade, body_id, block);
    let float_growth = (old.max(float_end + edges_below(document, block)).min(max) - old).max(0.0);
    growth.max(float_growth)
}

/// The lowest margin-box end of the floats that `block` holds, in its own
/// coordinates as [`flow_content_end`] measures, or negative infinity when
/// it holds none: its float children and the floats of the same block
/// formatting context inside its in-flow children.
fn held_float_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    block: usize,
) -> f32 {
    let mut end = f32::NEG_INFINITY;
    for child in laid_out_children(document, block) {
        let node = &document.nodes[child];
        if node.kind() != NodeKind::Element
            || node.display() == DisplayValue::None
            || matches!(
                node.position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
        {
            continue;
        }
        let top =
            pagination_child_parent_y(document, block, child, document.pagination_local_y(child));
        if is_floating_box_for_pagination(document, cascade, child) {
            // A relative offset moves where the float paints, not where it
            // ends in the flow.
            let float_end = top - layout_relative_offset_y(document, cascade, body_id, child)
                + node.layout.size.height
                + node.layout.margin.bottom;
            // A footnote leaves the flow for the page's footnote area.
            if cascade.computed[child].float != FloatValue::Footnote && float_end.is_finite() {
                end = end.max(float_end);
            }
            continue;
        }
        visit_same_bfc_floats_in_subtree(document, cascade, body_id, child, top, |_, float_end| {
            end = end.max(float_end);
        });
    }
    end
}

/// The padding and border below the content of `block`. A table cell keeps
/// the room its vertical alignment leaves in its padding; only its own
/// padding and border lie below its content.
fn edges_below(document: &super::context::LayoutSession<'_>, block: usize) -> f32 {
    let layout = &document.nodes[block].layout;
    let padding_below = if document.nodes[block].display() == DisplayValue::TableCell {
        // Percentages resolve against the table's width.
        let basis = cell_row_end(document, block)
            .map(|(_, _, table)| table)
            .or_else(|| document.parent_of(block))
            .map(|table| document.nodes[table].layout.size.width);
        document
            .pagination_style(block)
            .padding()
            .resolve_or_zero(basis)
            .bottom
    } else {
        layout.padding.bottom
    };
    padding_below + layout.border.bottom
}

/// How far `block` grows, up to the border-box height `max`, to hold its
/// in-flow content once its child `child` has grown by `child_growth`.
fn moved_content_growth(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    block: usize,
    (child, child_growth): (usize, f32),
    max: f32,
) -> f32 {
    let old = document.nodes[block].layout.size.height;
    let edges_below = edges_below(document, block);
    let child_end = if child_growth > 0.0 {
        let node = &document.nodes[child];
        let margin = if edges_below > 0.0 || encloses_child_margins(document, cascade, block) {
            collapsed_end_margin(document, cascade, child)
        } else {
            0.0
        };
        pagination_child_parent_y(document, block, child, document.pagination_local_y(child))
            - layout_relative_offset_y(document, cascade, body_id, child)
            + node.layout.size.height
            + margin
            + child_growth
    } else {
        f32::NEG_INFINITY
    };
    let end = flow_content_end(document, cascade, body_id, block).max(child_end);
    (old.max(end + edges_below).min(max) - old).max(0.0)
}

/// The effective flex parent of `node_id` when its main axis is the inline
/// axis, so its items sit side by side on flex lines.
fn row_flex_container(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<usize> {
    let parent = candidates::candidate_parent(document, node_id)?;
    (document.nodes[parent].formatting_context == FormattingContext::Flex
        && matches!(
            cascade.computed[parent].flex_direction,
            FlexDirectionValue::Row | FlexDirectionValue::RowReverse
        ))
    .then_some(parent)
}

/// The in-flow peers currently sharing `node_id`'s flex line in the row flex
/// container `parent_id`, with their current coordinates: every other item
/// when the container does not wrap, and otherwise those on the line of
/// `node_id` (see [`flex_line_of`]). The line is found in the inline axis, so
/// the alignment and margins of the items do not matter. Capture membership
/// before any transition moves it.
fn flex_line_peers(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    parent_id: usize,
    node_id: usize,
) -> Vec<(usize, f32)> {
    let items: Vec<usize> = document
        .children_through_contents(parent_id)
        .into_iter()
        .filter(|&id| is_in_flow_flex_child_for_pagination(document, parent_id, id))
        .collect();
    let line = if cascade.computed[parent_id].flex_wrap == FlexWrapValue::NoWrap {
        items
    } else {
        flex_line_of(document, cascade, body_id, parent_id, items, node_id)
    };
    line.into_iter()
        .filter(|&id| id != node_id)
        .map(|id| (id, current_abs_y(document, id)))
        .collect()
}

/// The flex line of a row flex item that a forced break moves.
pub(super) struct RowFlexLine {
    /// The other items on the line, with their coordinates before the break.
    pub(super) peers: Vec<(usize, f32)>,
    /// How far below the top of the line the item's margin box starts.
    pub(super) offset: f32,
}

/// The flex line of `node_id` when it is an item of a row flex container,
/// with `y` its position in the same coordinates as the current positions of
/// the other items. The line starts at the highest margin box of its items,
/// in their places in the flow.
fn row_flex_line(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    y: f32,
) -> Option<RowFlexLine> {
    let parent_id = row_flex_container(document, cascade, node_id)?;
    let margin_box_top = |id: usize, y: f32| {
        y - layout_relative_offset_y(document, cascade, body_id, id)
            - document.nodes[id].layout.margin.top
    };
    let peers = flex_line_peers(document, cascade, body_id, parent_id, node_id);
    let own = margin_box_top(node_id, y);
    let line_top = peers
        .iter()
        .map(|&(id, before_y)| margin_box_top(id, before_y))
        .fold(own, f32::min);
    Some(RowFlexLine {
        peers,
        offset: own - line_top,
    })
}

/// How far below the top of its flex line the text `offset` below the top of
/// the box `root` starts, when a break after `source` falls before the text
/// inside a row flex container around both: on the line of the item that
/// holds the text, beside the line that holds `source`. Measured in the
/// layout of the line, which pagination has not moved apart: each box lies
/// where layout put it in its parent, and a relative offset only paints it
/// elsewhere.
fn text_offset_in_flex_line(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    source: usize,
    root: usize,
    offset: f32,
) -> Option<f32> {
    let local_top = |id: usize| {
        document.nodes[id].layout.location.y
            - layout_relative_offset_y(document, cascade, body_id, id)
    };
    // A break after a box inside the item falls inside it, not before a line.
    if is_descendant_or_self(document, source, root) {
        return None;
    }
    let mut item = root;
    let mut offset = offset;
    let container = loop {
        let parent = candidates::candidate_parent(document, item)?;
        if is_descendant_or_self(document, source, parent) {
            break row_flex_container(document, cascade, item)?;
        }
        offset += local_top(item);
        item = parent;
    };
    let margin_top = |id: usize| local_top(id) - document.nodes[id].layout.margin.top;
    let line_top = flex_line_peers(document, cascade, body_id, container, item)
        .into_iter()
        .map(|(id, _)| margin_top(id))
        .fold(margin_top(item), f32::min);
    Some(offset + local_top(item) - line_top)
}

/// Places captured peers from their saved positions, so an ancestor that
/// already moved them does not cause the displacement to be added twice.
fn place_flex_line_peers(document: &mut Document, peers: &[(usize, f32)], delta: f32) {
    for &(id, before_y) in peers {
        materialize_y(document, id, before_y + delta);
    }
}

/// Whether `display` makes a block-level box in the normal flow, whose
/// block-start margin can adjoin a page break between siblings.
fn is_block_level_flow_box(display: DisplayValue) -> bool {
    display.is_block_flex_or_grid_container()
        || matches!(
            display,
            DisplayValue::Block
                | DisplayValue::FlowRoot
                | DisplayValue::ListItem
                | DisplayValue::Table
        )
}
