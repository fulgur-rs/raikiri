use super::box_tree::BoxTree;
use crate::document::ancestors::{AncestorCursor, ParentRelation};
use crate::node::BoxPropertiesRef;
mod candidates;
pub(crate) mod column_projection;
mod footnotes;
mod fragment_sources;
mod leading_margins;
mod line_ranges;
mod link_events;
mod pagination_state;
mod sibling_adjacency;
mod sibling_shifts;
mod source_walk;
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

use super::ifc::boxes::IfcBoxKind;
use super::ifc::text_lines::{IfcTextLine, TextLineIndex, single_unplaced_line};
use super::table::headers::HeaderRepeats;
use super::taffy_adapter::CollapsibleMarginSet;
use super::*;
use crate::node::FormattingContext;
use crate::page_projection::records::OverflowClipSource;
use raikiri_layout_style::values::{BoxSizing, Position};
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
        child_parent_y += document.nodes[id].layout.location.y;
    }
    child_parent_y
}

/// The lowest block-end edge of the boxes that paint below the paginated
/// flow: the body's own border box, nested absolutely positioned boxes, and
/// content overflowing a box that offers no break points (for example a
/// `contain: size` box). Measured from the body's coordinate space after
/// pagination moved the flow.
///
/// Fixed boxes repeat on every page and running elements leave the flow, so
/// their subtrees do not extend the document. A box that clips its
/// block-axis overflow contributes its own border box but not its content.
/// Inline elements are located by their paragraph's lines, which take part
/// in pagination directly, so only the boxes inside them are measured.
///
/// The in-flow content of a multicol container is projected into columns,
/// so its unfragmented layout position says nothing about the page it paints
/// on. Where a column layout cut that content into fragments, the fragments
/// are measured instead: a box that cannot break and is taller than its
/// column overflows it (CSS Fragmentation 3, 4.4). An absolutely positioned
/// box in the columns whose containing block is the container or one of its
/// ancestors is not projected, though: it paints at its laid-out position
/// and is measured there.
fn content_overflow_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    root_flow_offset: f32,
    page_area_height: f32,
) -> f32 {
    use raikiri_style::property::OverflowValue;
    // The body's overflow, or the root's, clips at the viewport, which
    // stays within the paginated flow.
    if super::viewport_overflow_source(document, cascade)
        .is_some_and(|id| cascade.computed[id].overflow.y != OverflowValue::Visible)
    {
        return f32::NEG_INFINITY;
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
    // `Some(true)` marks the in-flow content of a multicol container where an
    // absolutely positioned box still takes its containing block from
    // outside the columns, `Some(false)` content where it does not.
    let mut stack = vec![(body_id, 0.0_f32, None::<bool>)];
    while let Some((node_id, parent_y, in_columns)) = stack.pop() {
        let node = &document.nodes[node_id];
        if !node.is_in_document()
            || node.is_display_none()
            || node.is_non_rendered_html_element()
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
            let in_columns = in_columns.map(|outside| {
                outside
                    && !super::fixed::establishes_absolute_containing_block(
                        document, node_id, computed,
                    )
            });
            stack.extend(
                node.children
                    .iter()
                    .map(|&child| (child, parent_y, in_columns)),
            );
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
        // Inside columns only an absolutely positioned box placed against a
        // containing block outside them is measured, with its subtree.
        // A `display: contents` element generates no positioned box, so its
        // children stay in the column flow.
        let positioned = matches!(document.nodes[node_id].position(), PositionValue::Absolute)
            && document.nodes[node_id].display() != DisplayValue::Contents;
        let in_columns = match in_columns {
            Some(true) if positioned => None,
            Some(false) if positioned => continue,
            other => other,
        };
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
        let bottom = if node_id == body_id {
            block_border_box_end(
                document,
                cascade,
                body_id,
                root_flow_offset,
                page_area_height,
                false,
            )
        } else if in_columns.is_none() && node.layout.size.height > 0.0 {
            y + node.layout.size.height
        } else {
            f32::NEG_INFINITY
        };
        if bottom.is_finite() {
            end = end.max(bottom);
        }
        // A `display: contents` element generates no box and clips nothing,
        // as in painting.
        let clips = node_id != body_id && element_clips_overflow(document, cascade, node_id);
        if clips {
            continue;
        }
        // Generated block content and overflowing lines have no source box
        // to visit. Measure retained paragraph flow even when an authored
        // height or max-height limits its ordinary containing box.
        if in_columns.is_none()
            && node.multicol.is_none()
            && let Some(lines) = node.ifc.as_ref().and_then(|root| root.lines.as_ref())
        {
            end = end
                .max(y + node.layout.border.top + node.layout.padding.top + lines.shifted_height());
        }
        if in_columns.is_none() {
            let roots = fragment_roots.get(&node_id).map_or(&[][..], Vec::as_slice);
            end = end.max(fragment_tree_end(
                fragments,
                &fragment_children,
                roots,
                y,
                |node_id| element_clips_overflow(document, cascade, node_id),
                |node_id| painted_relative_offset_y(document, cascade, body_id, node_id),
            ));
        }
        let in_columns = if let Some(outside) = in_columns {
            Some(
                outside
                    && !super::fixed::establishes_absolute_containing_block(
                        document, node_id, computed,
                    ),
            )
        } else if node.multicol.is_some() {
            Some(true)
        } else {
            None
        };
        stack.extend(node.children.iter().map(|&child| (child, y, in_columns)));
    }
    end
}

/// Whether the element hides block-axis overflow when painted.
fn element_clips_overflow(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    !matches!(cascade.computed[node_id].overflow.y, OverflowValue::Visible)
        && crate::paint_rules::clips_element_overflow(document, cascade, node_id)
}

/// The block offset at which the overflow clip of `node_id` hides what its
/// content paints, for a border box that starts at `y`: the border box's own
/// edge, or the farther one that `overflow-clip-margin` names on an
/// `overflow: clip` axis. A clip edge inside the border box (a padding box
/// clip) still counts as the border box's edge.
fn clip_block_end(document: &Document, cascade: &CascadeResult, node_id: usize, y: f32) -> f32 {
    let layout = &document.nodes[node_id].layout;
    let (border, padding) = (layout.border, layout.padding);
    let geometry = crate::paint_rules::OverflowClipGeometry::new(
        &cascade.computed[node_id],
        PaintRect::new(0.0, y, layout.size.width, layout.size.height),
        PaintInsets::new(border.top, border.right, border.bottom, border.left),
        PaintInsets::new(padding.top, padding.right, padding.bottom, padding.left),
        crate::paint_rules::is_replaced_element(document, node_id),
    );
    y + layout.size.height + geometry.block_end_outset()
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
                inherited_clip_end.min(parent_origin + clip.y + clip.height)
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

/// The block offset that layout applied to a relatively positioned or
/// sticky box. It moves the box but not the flow around it (CSS 2.1
/// §9.4.3), and fragmentation happens before it (CSS Fragmentation 3 §5.5).
fn layout_relative_offset_y(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> f32 {
    // A `display: contents` element generates no box to offset.
    if !matches!(
        document.nodes[node_id].position(),
        PositionValue::Relative | PositionValue::Sticky
    ) || matches!(
        document.nodes[node_id].display(),
        DisplayValue::Contents | DisplayValue::None
    ) || relative_offset_is_painted(document, cascade, body_id, node_id)
        // Layout places a sticky float without its offset.
        || (matches!(document.nodes[node_id].position(), PositionValue::Sticky)
            && is_floating_box_for_pagination(document, cascade, node_id))
    {
        return 0.0;
    }
    // Percentages resolve against the containing block's height when it
    // is definite, and otherwise act as `auto` (CSS 2.1 §9.3.2). Layout
    // takes `top` over `-bottom` in every writing mode.
    super::relative::layout_relative_insets(
        document,
        cascade,
        node_id,
        super::relative::Precedence::LeftTop,
    )
    .1
    .unwrap_or(0.0)
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
    if !relative_offset_is_painted(document, cascade, body_id, node_id) {
        return 0.0;
    }
    super::relative::used_relative_offset_for_layout(document, cascade, node_id).1
}

/// Whether layout leaves the relative offset of `node_id` to the painter.
fn relative_offset_is_painted(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> bool {
    if !matches!(document.nodes[node_id].position(), PositionValue::Relative) {
        return false;
    }
    super::relative::placed_without_relative_offset_for_layout(
        document,
        cascade,
        Some(body_id),
        node_id,
    ) || document.nodes[node_id].needs_relative_block_paint_offset()
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
fn block_border_box_end(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    flow_root: usize,
    root_flow_offset: f32,
    page_area_height: f32,
    whole_box: bool,
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
    let used = |value: ComputedLengthPercentageOrAuto| {
        match value {
            ComputedLengthPercentageOrAuto::Px(value) => Some(value),
            ComputedLengthPercentageOrAuto::Percent(_)
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
    let min_height = used(computed.min_height).unwrap_or(0.0);
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
    // A definite height, even zero, fixes the border box; content
    // overflowing it is measured on its own.
    if let Some(height) = used(computed.height) {
        let height = used(computed.max_height).map_or(height, |max| height.min(max));
        return border_box_end(height.max(min_height)).max(generated_end);
    }
    let min_end = if min_height > 0.0 {
        border_box_end(min_height)
    } else {
        f32::NEG_INFINITY
    };
    let max_end = used(computed.max_height).map_or(f32::INFINITY, border_box_end);
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
    let mut last_flow_end = None;
    // Where the margin box of the last child ends in the committed flow.
    // Pagination may have cut a margin that crossed a page edge, which the
    // margins themselves do not show.
    let mut last_margin_end = f32::INFINITY;
    // The block-end margins that adjoin the end of the flow: the one of the
    // last child with a border box of its own and those of the empty children
    // after it, whose margins collapse through them (CSS 2 §8.3.1).
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
                offset + layout.location.y - relative_shift
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
        let border_end = offset + layout.location.y - relative_shift + layout.size.height;
        let end = border_end + layout.margin.bottom;
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
                trailing = trailing
                    .collapse_with_margin(layout.margin.top)
                    .collapse_with_margin(layout.margin.bottom);
            } else {
                last_flow_end = Some(border_end);
                trailing = CollapsibleMarginSet::from_margin(layout.margin.bottom);
            }
        }
    }
    // Without block-end edges the last child's block-end margin collapses
    // through the end of the root, unless the root is a formatting context of
    // its own; a `min-height` leaves it out as well. Margins that collapsed
    // through every child joined the root's block-start margin when nothing
    // separates them, so they lie outside the root too.
    let encloses_margins = edges_bottom > 0.0 || encloses_floats;
    if let Some(end) = last_flow_end {
        let end = if encloses_margins {
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
                        parent_node.percent_basis_height.is_some()
                            || !grid_rows_size_to_content(document, parent)
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

/// How far the content of each flex item that
/// [`update_paginated_block_heights`] may extend already reaches past the
/// item's block-end edge before pagination moves anything, by node id.
///
/// A flex item's used height comes from the flex algorithm, which may clamp
/// or shrink it below its content; that content overflows the item rather
/// than extending it. Only the growth pagination adds is enclosed. Other
/// boxes are left out: their auto height is the one the extension measures.
fn unpaginated_content_overflow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    root: usize,
    page_area_height: f32,
) -> HashMap<usize, f32> {
    document
        .preorder(root)
        .filter(|&node_id| {
            document.taffy_parent_of(node_id).is_some_and(|parent| {
                document.nodes[parent].formatting_context == FormattingContext::Flex
            }) && grows_with_paginated_content(document, node_id)
        })
        .filter_map(|node_id| {
            let end = block_border_box_end(document, cascade, node_id, 0.0, page_area_height, true);
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
    let moved_table_rows: HashMap<usize, f32> = row_origins
        .into_iter()
        .filter_map(|(row, origin)| {
            let delta = document.nodes[row].layout.location.y - origin;
            (delta > 0.001).then_some((row, delta))
        })
        .collect();
    super::table::row_moves::follow_moved_rows(document, &moved_table_rows, |table| {
        table_headers
            .headers
            .values()
            .any(|header| header.table == table)
    });
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
) {
    let order: Vec<_> = document.preorder(root).collect();
    for node_id in order.into_iter().rev() {
        if !grows_with_paginated_content(document, node_id) {
            continue;
        }
        let end = block_border_box_end(document, cascade, node_id, 0.0, page_area_height, true);
        let height =
            end - current_abs_y(document, node_id) - overflow.get(&node_id).copied().unwrap_or(0.0);
        if height.is_finite() && height > document.nodes[node_id].layout.size.height {
            document.nodes[node_id].layout.size.height = height;
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
    for node in &mut document.nodes {
        node.multicol_groups.clear();
        node.multicol_rows.clear();
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
            {
                document.forced_break_clearance.clear();
                document.forced_break_line_clearance.clear();
                document.forced_break_float_floor.clear();
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
            let icb = super::initial_containing_block::prepare(
                document,
                cascade,
                content_width,
                content_height,
            );
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
    /// Empty keeps the fixed page height. Vertical pagination instead uses
    /// [`Self::page_widths`] for its block-axis page advances.
    pub page_steps: &'a [f32],
    /// Per-page content widths; empty keeps the fixed page width.
    /// Horizontal pagination scales percentage widths relative to the first
    /// scheduled width, leaving missing entries unchanged. For a vertical
    /// root, these widths provide the successive block-axis page advances.
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
    /// margin above it at the edge that opens `natural_page`: the top of the
    /// page it lands on and its border-box top.
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
    ) -> (f32, f32) {
        if forced_before {
            let page_top = self.origin(natural_page);
            (page_top, page_top + margin_top)
        } else {
            let first_edge =
                self.first_edge_in_margin(margin_start(), follows_forced_break, natural_page);
            let page_top = self.origin(first_edge);
            (page_top, page_top)
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
    /// of a page.
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
        let margin_box_y = effective_y - margin_top.max(0.0);
        if forced_transition && page > current_page && flow_max >= page && margin_box_y.is_finite()
        {
            self.page_starting_at_or_after(margin_box_y)
        } else {
            page
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
    let line_count = document
        .ifc_layout_node(owned.root)?
        .ifc
        .as_ref()?
        .lines
        .as_ref()?
        .lines
        .len();
    let paragraph = IfcTextLines {
        root: owned.root,
        width: owned.width,
        lines: (0..line_count)
            .map(|line| IfcTextLine {
                line,
                top: 0.0,
                bottom: 0.0,
            })
            .collect(),
    };
    Some(
        positioned_owned_text_line_bounds(document, &paragraph)?
            .into_iter()
            .map(|(top, bottom)| (top - first_top, bottom - first_top))
            .collect(),
    )
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

/// The break that keeps `widows` lines on the last page of a paragraph that
/// runs over more pages, whether or not `earlier_break` repaired its first
/// break. The pages between are full, so only the break before the last page
/// can leave too few; it moves up the same way while the page before keeps
/// `orphans`. `line_metrics` is as for [`widows_break_on_next_page`] and has
/// at least `orphans + widows` lines.
fn widows_break_on_last_page(
    line_metrics: &[(f32, f32)],
    effective_y: f32,
    current_page: u32,
    orphans: usize,
    widows: usize,
    earlier_break: Option<LineBreakMove>,
    page_origins: &PageOrigins,
) -> Option<LineBreakMove> {
    let total_lines = line_metrics.len();
    let moved = |line: usize| {
        earlier_break
            .filter(|&(first, _, _)| line >= first)
            .map_or(0.0, |(_, shift, _)| shift)
    };
    let top = |line: usize| effective_y + line_metrics[line].0 + moved(line);
    let bottom = |line: usize| effective_y + line_metrics[line].1 + moved(line);
    // Each line belongs to the page that holds its center (see
    // `line_center_on_page`).
    let page_of =
        |line: usize| page_origins.page_index_for_y((top(line) + bottom(line)) * 0.5 + 0.001);
    let first_on = |page: u32| {
        (0..total_lines)
            .find(|&line| page_of(line) >= page)
            .unwrap_or(total_lines)
    };
    let last_page = page_of(total_lines - 1);
    let line = total_lines - widows;
    (last_page > current_page.saturating_add(1)
        && total_lines - first_on(last_page) < widows
        && line >= first_on(last_page - 1) + orphans
        && bottom(total_lines - 1) - top(line) <= page_origins.step_at(last_page) + 0.001)
        .then(|| (line, page_origins.origin(last_page) - top(line), last_page))
        .filter(|(_, shift, _)| shift.is_finite() && *shift > 0.0)
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
fn forced_line_breaks(document: &Document, root: usize) -> Vec<ForcedLineBreak> {
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
                .filter(|forced| page_break_is_forced(document, root, forced.value))
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
/// `shift`, and grows the root's box to hold them.
fn shift_ifc_lines_from(document: &mut Document, root: usize, line: usize, shift: f32) {
    let node = document.table_layout_node_mut(root);
    if let Some(lines) = node.ifc.as_mut().and_then(|ifc| ifc.lines.as_mut()) {
        lines.shifts.push(line, shift);
        node.layout.size.height += shift;
    }
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
    document.table_layout_node_mut(moved).layout.location.y += delta;
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
                let top = root_lines.line_top(line.line);
                let height = root_lines.lines[line.line].block_size();
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
        text_roots,
        placement_overflow_clips,
        paragraph_cache,
        repeated_placement_nodes,
    } = walk_sources(document, cascade, body_id, &mut work)?;
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
            && !node.is_non_rendered_html_element()
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
            if let crate::node::NodeData::Text(text) = &node.data {
                if node.is_in_document() && !text.text_content.trim().is_empty() {
                    boxes.push(None);
                }
            } else if node.kind() == NodeKind::Element
                && node.is_in_document()
                && !node.is_non_rendered_html_element()
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
                && !node.is_non_rendered_html_element()
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
    // Walk the boxes with the page value each inherits. Floats take their
    // parent's page, and their insides are not walked.
    let mut cleared = Vec::new();
    let mut cleared_lines = Vec::new();
    let mut cleared_to_float_floor = Vec::new();
    let mut pending = vec![(0_usize, None::<&str>)];
    while let Some((parent, inherited)) = pending.pop() {
        let mut after_forced_break = false;
        let mut float_after_break = false;
        // The last in-flow block among the parent's boxes: when the parent
        // is a paragraph of the inline engine, the lines after it are laid
        // out from its bottom.
        let mut last_block = None;
        // Generated `::before` content is not an arena child; like text, it
        // takes the parent's page.
        let mut previous_page_name = crate::generated_content::is_in_flow_generated_text(
            cascade,
            parent,
            raikiri_style::PseudoElem::Before,
        )
        .then_some(inherited);
        for item in child_boxes(document, parent, true) {
            let Some(child) = item else {
                // Text takes the parent's page and starts the new page itself.
                // Its lines have no element to give clearance to, so the
                // paragraph starts them below the earlier floats after the
                // block that ends right before the break.
                if let Some(block) = last_block.take()
                    && (after_forced_break || changes_page(&previous_page_name, &inherited))
                {
                    cleared_lines.push(block);
                }
                after_forced_break = false;
                float_after_break = false;
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
                // The first float right after a forced break goes to the new
                // page, so it clears the earlier page's floats too; the floats
                // after it line up beside it. The break still carries on to
                // the next in-flow box, which pagination would otherwise leave
                // on the earlier page. A float's own forced breaks move it
                // and the boxes after it to the next page the same way, and so
                // does a change of named page: pagination puts a float on its
                // parent's page.
                let named_break = changes_page(&previous_page_name, &inherited);
                previous_page_name = Some(inherited);
                let forced_after =
                    page_break_is_forced(document, child, cascade.computed[child].break_after);
                if named_break
                    || page_break_is_forced(document, child, cascade.computed[child].break_before)
                {
                    after_forced_break = true;
                    float_after_break = false;
                }
                // A break of the float's own is not right after the block
                // before it, so the lines after that block stay where they
                // are.
                if named_break
                    || forced_after
                    || page_break_is_forced(document, child, cascade.computed[child].break_before)
                {
                    last_block = None;
                }
                if after_forced_break && !float_after_break {
                    document.record_pass_override(
                        child,
                        raikiri_layout_style::OwnedPropertyOverride::clear(ClearValue::Both),
                    );
                    cleared.push(child);
                    float_after_break = true;
                }
                if forced_after {
                    after_forced_break = true;
                    float_after_break = false;
                }
                continue;
            }
            // A float right after the break started the new page, so the box
            // after it lines up beside that float instead of below it.
            let beside_page_start_float = std::mem::take(&mut float_after_break);
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
            let own_break = named_break
                || page_break_is_forced(document, child, cascade.computed[child].break_before);
            let forced = after_forced_break || own_break;
            // A break after the last box inside this one is carried out to
            // the next box, as the paginator does.
            after_forced_break = page_edge_constraint(document, cascade, child, false).0;
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
                pending.push((child, own_page_name));
            }
            // Floats on either side belong to the earlier page, so a
            // one-sided clear is widened too.
            if forced
                && beside_page_start_float
                && !own_break
                && !document.nodes[parent].used_writing_mode.is_vertical()
            {
                // Layout clears the floats before the one that starts the
                // page, not that float.
                cleared_to_float_floor.push(child);
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
            cleared_lines.push(block);
        }
    }
    cleared.sort_unstable();
    cleared_lines.sort_unstable();
    cleared_to_float_floor.sort_unstable();
    if cleared != document.forced_break_clearance
        || cleared_lines != document.forced_break_line_clearance
        || cleared_to_float_floor != document.forced_break_float_floor
    {
        document.forced_break_clearance = cleared;
        document.forced_break_line_clearance = cleared_lines;
        document.forced_break_float_floor = cleared_to_float_floor;
        document.layout_dirty = true;
    }
}

fn page_break_is_forced(document: &Document, node_id: usize, value: BreakBetween) -> bool {
    if matches!(value, BreakBetween::Page) {
        return true;
    }
    if !matches!(value, BreakBetween::Always) {
        return false;
    }
    !document
        .layout_ancestors(node_id)
        .any(|id| document.nodes[id].multicol.is_some())
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
            || node.is_non_rendered_html_element()
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
            if text.text_content.trim().is_empty() {
                continue;
            }
            return Ok(Some(child));
        }
        if document.nodes[child].display() == DisplayValue::Contents
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
        let node = &document.nodes[id];
        let computed = &cascade.computed[id];
        if document.nodes[id].display() != DisplayValue::Contents {
            let edge = if before {
                computed.break_before
            } else {
                computed.break_after
            };
            forced |= page_break_is_forced(document, id, edge);
            avoided |= matches!(edge, BreakBetween::Avoid | BreakBetween::AvoidPage);
        }
        if node.has_before_or_after_content
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
    let (slices, moved) = layout_pages_attempt(
        document,
        cascade,
        page_box,
        page_steps,
        page_widths,
        control,
        false,
    )?;
    if !moved {
        return Ok(slices);
    }
    // The columns of a multicol container were filled page by page where the
    // container began, and the flow then moved the container, so they stand
    // in the wrong pages. Laying the document out again with the container's
    // columns left whole keeps them in the pages the flow cuts them into.
    layout_pages_attempt(
        document,
        cascade,
        page_box,
        page_steps,
        page_widths,
        control,
        true,
    )
    .map(|(slices, _)| slices)
}

/// Lay out and break `document` into pages once, and whether pagination
/// moved a multicol container whose columns it had filled page by page, which
/// leaves `slices` empty. With `unpaged_plain_columns`, no container has its
/// columns filled that way.
fn layout_pages_attempt(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    control: &PageLayoutControl<'_>,
    unpaged_plain_columns: bool,
) -> Result<(Vec<PageSlice>, bool), LayoutError> {
    control.page_limit_reached.set(false);
    control.check_page_index(0)?;
    let mut session = super::context::LayoutSession::with_cascade(document, cascade);
    session.context.unpaged_plain_columns = unpaged_plain_columns;
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
        return Ok((
            vec![PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None,
            }],
            false,
        ));
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
    Ok((slices, session.context.page_filled_columns_moved))
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
        fragment_edits.grow(parent, delta);
        if parent == body_id {
            break; // cov:ignore: the body has a pinned viewport height and exits at the preceding fixed-height boundary.
        }
        child = parent;
        walk.advance(document);
    }
    Ok(())
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
    let unpaginated_overflow =
        unpaginated_content_overflow(document, cascade, body_id, content_height);
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
        .filter(|(_, node)| !node.multicol_groups.is_empty())
        .map(|(id, _)| (current_abs_y(document, id), id))
        .collect();
    spanning_owners.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Fragment records change for every owner; they are installed together
    // once all owners are paginated.
    let mut fragment_edits = FragmentEdits::new(&document.fragment_tree);
    // So are the moves of the boxes after each owner, which are applied to a
    // parent's children before they are read.
    let mut sibling_shifts = sibling_shifts::SiblingShifts::default();
    for (_, owner) in spanning_owners {
        let owner_y = root_flow_offset
            + current_abs_y(document, owner)
            + sibling_shifts.offset(document, owner);
        sibling_shifts.apply(document, &mut fragment_edits, owner);
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

    // Where the columns of each container were filled page by page, from the
    // position the container has now.
    let refilled: Vec<_> = document
        .context
        .page_filled_columns
        .iter()
        .map(|&owner| (owner, current_abs_y(document, owner)))
        .collect();
    let roots = candidates::FlowRoots::new(document, body_id);
    let mut candidates = Vec::new();
    let mut text_lines = TextLineIndex::default();
    CandidateCollector::new(
        document,
        cascade,
        roots,
        page_step,
        &mut candidates,
        &mut text_lines,
        control,
    )
    .collect_flow(root_flow_offset)?;
    let table_headers =
        HeaderRepeats::prepare(document, cascade, roots.contents, page_step, |id| {
            root_flow_offset + current_abs_y(document, id)
        });
    let avoided_breaks = AvoidedBreaks::collect(
        document,
        cascade,
        &candidates,
        &table_headers,
        roots.contents_box,
        content_width,
        root_flow_offset,
    );
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
    };
    let mut candidates = candidates.into_iter();
    // Each candidate is placed while the page limit lets pagination reach the
    // page the flow is on.
    while let Some(candidate) = candidates.next()
        && control.within_page_limit(state.current_page)?
    {
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
            break;
        }
    }

    // A container the walk moved has its columns filled for another place,
    // and its caller lays the document out again without that.
    if refilled
        .iter()
        .any(|&(owner, y)| (current_abs_y(document, owner) - y).abs() > 1e-3)
    {
        document.context.page_filled_columns_moved = true;
        return Ok(Vec::new());
    }
    follow_moved_table_rows(document, table_row_origins, &state.table_headers);
    super::table::headers::finish_boxes(document, &state.table_headers);
    update_paginated_block_heights(
        document,
        cascade,
        body_id,
        content_height,
        &unpaginated_overflow,
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
    Ok(page_slices(
        control.last_page(state.reach.max),
        &page_origins,
        &state.page_names,
    ))
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

/// Where each table row among `candidates` sits in its parent before
/// pagination; the table boxes around the rows that moved follow them once
/// every row is placed.
fn table_row_origins(document: &Document, candidates: &[PageCandidate]) -> Vec<(usize, f32)> {
    candidates
        .iter()
        .filter(|candidate| candidate.context.is_table_row())
        .map(|candidate| {
            let row = candidate.node_id;
            (row, document.nodes[row].layout.location.y)
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
            if let Some(table) = table_of_row(document, candidate.node_id) {
                let first = table_first_rows
                    .entry(table)
                    .or_insert((candidate.node_id, candidate.raw_y));
                if candidate.raw_y < first.1 {
                    *first = (candidate.node_id, candidate.raw_y); // cov:ignore: rows are collected in visual order, so a later row never starts higher.
                } // cov:ignore: closes the visual-order guard above.
            }
        }
        for (table, (row, row_y)) in table_first_rows {
            if !rows_start_the_grid(document, cascade, table) {
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

/// The table, block or inline, that the row `row` belongs to.
fn table_of_row(document: &Document, row: usize) -> Option<usize> {
    document.ancestors(row).find(|&id| {
        matches!(
            document.nodes[id].display(),
            DisplayValue::Table | DisplayValue::InlineTable
        )
    })
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
/// table among `candidates` whose rows start its grid and whose header does
/// not repeat: the part of the table that a box tied to the one before it
/// cannot be broken away from, since a row moves whole. A first row in a row
/// group that avoids breaks inside it takes the whole group along
/// (`row_groups`).
fn first_row_ends(
    document: &Document,
    cascade: &CascadeResult,
    candidates: &[PageCandidate],
    table_headers: &HeaderRepeats,
    row_groups: &HashMap<usize, (f32, f32)>,
) -> HashMap<usize, f32> {
    // Each table's first row in visual order, with the block end of the row:
    // the table layout places a footer group after the body groups wherever
    // it is in the source.
    let mut first_rows = HashMap::<usize, (usize, f32)>::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.context.is_table_row())
    {
        // The rows of an inline table, nested in a cell or not, are not rows
        // of a block table: the plain-table filter below drops that table.
        if let Some(table) = table_of_row(document, candidate.node_id) {
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
    first_rows
        .into_iter()
        .filter(|&(table, (row, end))| {
            end.is_finite()
                && is_plain_table_box(document, table)
                && rows_start_the_grid(document, cascade, table)
                && !forced_break_before_first_row(document, cascade, table, row)
                && !document.nodes[table]
                    .children
                    .iter()
                    .any(|child| table_headers.headers.contains_key(child))
        })
        .map(|(table, (row, end))| {
            let group_end = row_groups
                .get(&row)
                .map_or(end, |&(group_end, _)| group_end);
            (table, end.max(group_end))
        })
        .collect()
}

/// Whether a forced page break comes before `row`, the first row of `table`:
/// its own `break-before`, or that of the row group around it. The page edge
/// constraint of the table does not look inside it, so the box before the
/// table would be tied to a row that starts a page of its own.
fn forced_break_before_first_row(
    document: &Document,
    cascade: &CascadeResult,
    table: usize,
    row: usize,
) -> bool {
    [
        Some(row),
        document.parent_of(row).filter(|&group| group != table),
    ]
    .into_iter()
    .flatten()
    .any(|id| page_break_is_forced(document, id, cascade.computed[id].break_before))
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
            let forced_before =
                page_break_is_forced(document, row, cascade.computed[row].break_before)
                    || previous_rows
                        .insert(document.parent_of(row), row)
                        .is_some_and(|previous| {
                            page_break_is_forced(
                                document,
                                previous,
                                cascade.computed[previous].break_after,
                            )
                        });
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

/// Connected class-A sibling runs have a legal break before their first
/// box. Pagination moves a run only when it fits in one page, otherwise it
/// relaxes the run's avoidance constraints so an oversized run still makes
/// progress.
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
) -> HashMap<usize, f32> {
    let first_row_ends = first_row_ends(document, cascade, candidates, table_headers, row_groups);
    // The last box of each run, keyed by the run's first box, and the margin
    // below it.
    let mut run_lasts = HashMap::<usize, (&PageCandidate, f32)>::new();
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
        // A table ends the run it joins and starts none: only its first row
        // stays with the box before it.
        let ends_run = !eligible
            && candidate.context.is_direct_body_element()
            && first_row_ends.contains_key(&candidate.node_id);
        if !eligible && !ends_run {
            previous = None;
            run_start = None;
            continue;
        }
        let connected = previous.is_some_and(|prior| {
            let (after_forced, after_avoided) =
                page_edge_constraint(document, cascade, prior.node_id, false);
            let (before_forced, before_avoided) =
                page_edge_constraint(document, cascade, candidate.node_id, true);
            !after_forced
                && !before_forced
                && prior.page_name == candidate.page_name
                && (after_avoided || before_avoided)
        });
        if let Some(prior) = previous.filter(|_| connected) {
            let start = *run_start.get_or_insert(prior.node_id);
            let margin_bottom =
                used_computed_length_percentage_or_auto(computed.margin.bottom, content_width)
                    .unwrap_or(0.0)
                    .max(0.0);
            run_lasts.insert(start, (candidate, margin_bottom));
        } else {
            run_start = None;
        }
        if ends_run {
            previous = None;
            run_start = None;
        } else {
            previous = Some(candidate);
        }
    }
    let join_ties = !super::page::has_named_pages(cascade);
    let mut ends: HashMap<usize, f32> = run_lasts
        .into_iter()
        .map(|(start, (last, margin_bottom))| {
            // The last box of the run stays with the box before it only as far
            // as the page has to hold of it.
            let head = match first_row_ends.get(&last.node_id) {
                // A table that avoids breaks inside it stays whole.
                Some(&row_end) => {
                    (!avoids_page_break_inside(cascade.computed[last.node_id].break_inside))
                        .then_some(row_end - last.raw_y)
                }
                None => avoided_run_head(document, cascade, body_id, last.node_id, join_ties).end,
            };
            let end = match head {
                Some(head) if head < last.height - 0.001 => last.raw_y + head,
                _ => last.raw_y + last.height + margin_bottom,
            };
            (start, end)
        })
        .collect();
    // A box that holds a run of its own at its start, or starts a run among
    // the boxes of its parent, carries where that run ends (see
    // `CandidateCollector`): it moves with the run when that crosses a page.
    for candidate in candidates {
        if let Some(unit) = candidate.context.avoided_unit() {
            let end = candidate.raw_y + unit;
            ends.entry(candidate.node_id)
                .and_modify(|known| *known = known.max(end))
                .or_insert(end);
        }
    }
    ends
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
/// row (see [`held_table_end`]). A box that avoids breaks inside it, a box
/// that is not a plain block or table, a paragraph that cannot move its lines
/// down inside it, a block that moves whole with its closing edges (see
/// [`lone_box_tail`]), and anything with a break, a float or generated content
/// at its start is kept whole.
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
            HeldStep::Whole if tied => {
                return HeldPart {
                    end: Some((offset + document.nodes[current].layout.size.height).min(clip_end)),
                    tied,
                };
            }
            HeldStep::Whole => return HeldPart { end: None, tied },
            HeldStep::End(end) => {
                return HeldPart {
                    end: Some((offset + end).min(clip_end)),
                    tied,
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
    /// The block in full.
    Whole,
    /// Up to this far below the block's border-box top.
    End(f32),
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
    if node.multicol.is_some()
        || node.has_before_or_after_content
        || avoids_page_break_inside(cascade.computed[current].break_inside)
        || !(is_table || is_plain_block_box(document, current))
    {
        return HeldStep::Whole;
    }
    // The breaks at the start of the box that ties it to the one before it
    // belong to the run, not to the boxes inside it.
    if current != node_id && !joined {
        let (forced, avoided) = page_edge_constraint(document, cascade, current, true);
        if forced || avoided {
            return HeldStep::Whole;
        }
    }
    // A table holds its first row, which moves whole.
    if is_table {
        return held_table_end(document, cascade, current).map_or(HeldStep::Whole, HeldStep::End);
    }
    // A block whose box is followed by closing edges alone moves whole
    // rather than leave them on the next page (see `lone_box_tail`), and
    // takes the box before it along.
    if lone_box_tail(document, cascade, body_id, current).is_some_and(|tail| tail > 0.001) {
        return HeldStep::End(node.layout.size.height);
    }
    if node.is_ifc_root() {
        return held_lines_end(document, cascade, current).map_or(HeldStep::Whole, HeldStep::End);
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
                return HeldStep::Whole;
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
            if is_plain_block_box(document, before)
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
        return HeldStep::Whole;
    };
    if takes_no_block_space(document, first) {
        return HeldStep::Whole;
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
    if breaks_between && !lines_can_shift_inside(document, current) {
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
/// first row, which moves whole, or the row group around that row when the
/// group avoids breaks inside it. `None` when the table is held whole, which
/// is also the case when it may repeat a header group or something comes
/// before its first row.
fn held_table_end(document: &Document, cascade: &CascadeResult, table: usize) -> Option<f32> {
    let children = document.children_through_contents(table);
    if !rows_start_the_grid(document, cascade, table)
        || children
            .iter()
            .any(|&child| document.nodes[child].display() == DisplayValue::TableHeaderGroup)
    {
        return None;
    }
    let (row, _) = children
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
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
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
                if page_break_is_forced(document, held, cascade.computed[held].break_after)
                    || page_break_is_forced(document, next, cascade.computed[next].break_before)
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

/// Whether `node_id` is a table that ends a run of boxes tied by avoided
/// breaks: one whose first row is the part the page holds with the box before
/// it (see [`held_table_end`]). Any other table is left out of the run.
fn ends_a_run_as_table(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    is_plain_table_box(document, node_id) && held_table_end(document, cascade, node_id).is_some()
}

/// Whether an avoided break (`break-after` of the box `before` or
/// `break-before` of the box `after`) ties the two boxes, with no forced break
/// at either.
fn boxes_are_tied(
    document: &Document,
    cascade: &CascadeResult,
    before: usize,
    after: usize,
) -> bool {
    let (forced_after, avoided_after) = page_edge_constraint(document, cascade, before, false);
    let (forced_before, avoided_before) = page_edge_constraint(document, cascade, after, true);
    !forced_after && !forced_before && (avoided_after || avoided_before)
}

/// The boxes of a flow that avoid a page break inside them or between them,
/// found once from its candidates.
#[derive(Default)]
struct AvoidedBreaks {
    /// The block end of each run of boxes joined by avoided breaks, keyed by
    /// the box that starts it among the boxes of its parent or holds it at the
    /// start of its own (see [`avoided_run_ends`]).
    run_ends: HashMap<usize, f32>,
    /// How far below its table's top edge the first row of each table
    /// starts (see [`table_row_leads`]).
    row_leads: HashMap<usize, f32>,
    /// The block end and table lead of each avoided table row group, keyed
    /// by its first row (see [`avoided_row_groups`]).
    row_groups: HashMap<usize, (f32, f32)>,
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
    /// The box starts a run of boxes joined by avoided breaks that would
    /// cross the page edge.
    avoided_run: bool,
    /// The box is a table row that would cross the page edge.
    table_row: bool,
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
            || self.row_group_lead.is_some()
            || self.flex_item
            || self.grid_item
    }
}

impl AvoidedBreaks {
    /// Finds the avoided breaks of `candidates`, whose flow starts at
    /// `root_flow_offset` and whose class-A boxes sit in `contents_box`.
    fn collect(
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        candidates: &[PageCandidate],
        table_headers: &HeaderRepeats,
        contents_box: usize,
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
            contents_box,
            content_width,
        );
        Self {
            run_ends,
            row_leads,
            row_groups,
        }
    }

    /// How the box of `candidate`, placed as `fit` says, moves to the next
    /// page. `trailing_flex_children` remembers the trailing item of each
    /// flex container it looks up.
    fn measure(
        &self,
        document: &Document,
        cascade: &CascadeResult,
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
            page_start,
            page_end,
            next_page_step,
        } = *fit;
        let avoided_run = self.run_ends.get(&node_id).is_some_and(|end| {
            avoided_run_moves(
                *end - raw_y,
                margin_top,
                effective_y,
                page_start,
                page_end,
                next_page_step,
            )
        });
        let page = if box_avoids_page_break_inside(
            document,
            node_id,
            cascade.computed[node_id].break_inside,
        ) {
            avoided_box_moves(
                flow_y,
                margin_top,
                flow_y + height + trailing_margin,
                page_start,
                page_end,
            )
        } else {
            // A paragraph of one line of generated text is kept whole by its
            // line alone: the margin and padding after the line break across
            // the edge like those of a paragraph of text. A box shorter than
            // the line ends where the box does.
            candidate
                .context
                .unplaced_line_end()
                .is_some_and(|line_end| {
                    avoided_box_moves(
                        flow_y,
                        margin_top,
                        flow_y + line_end.min(height),
                        page_start,
                        page_end,
                    )
                })
        };
        // A box inside another that moves with its run lengthens the boxes
        // around it, as one that moves for a break it avoids does.
        let page = page || (avoided_run && !candidate.context.is_direct_body_element());
        // Keep an ordinary table row intact when it would cross the current
        // page. The row is a class-A boundary owned by the table engine; its
        // cells and descendants follow the same flow shift below. Rowspans,
        // cell-internal breaks remain out of this narrow row-boundary pass.
        // A row that follows a repeated header must fit beside the header's
        // copy on the next page; otherwise moving it gains nothing.
        let repeated_header_height = table_headers.repeated_height_before(node_id);
        let table_row_lead = self.row_leads.get(&node_id).copied().unwrap_or(0.0);
        let table_row = candidate.context.is_table_row()
            && table_row_moves(
                flow_y,
                height,
                table_row_lead + height + repeated_header_height,
                page_start,
                page_end,
                next_page_step,
            );
        let row_group_lead = self
            .row_groups
            .get(&node_id)
            .copied()
            .filter(|&(end, _)| row_group_moves(flow_y, end - raw_y, page_start, page_end))
            .map(|(_, lead)| lead);
        // A trailing flex item may extend into the containing flex box's
        // continuation at the page edge. Earlier items still move intact when
        // they would cross a fragmentainer, preserving the existing column
        // flex pagination behavior.
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
        let flex_item = candidate.context.is_flex_item() && !flex_item_is_last && item_overflow();
        let grid_item = candidate.context.is_grid_item() && item_overflow();
        BoxOverflow {
            page,
            avoided_run,
            table_row,
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
        y += document.nodes[id].layout.location.y;
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

/// The outermost box whose top edge is that of `block`, which starts its
/// ancestors: each of them up to it has no top border or padding, begins
/// where its child does and renders nothing before it. Moving that box moves `block` without leaving the
/// ancestors' empty starts behind on the earlier page.
fn first_box_ancestor_at_top(document: &Document, block: usize, body_id: usize) -> usize {
    let block_top = current_abs_y(document, block);
    let mut child_top = block_top;
    let mut unusual_coordinates = document.oof_coordinates.contains_key(&block)
        || document.ifc_source_owner(block) != block
        || document.nodes[block]
            .flags
            .contains(NodeFlags::IN_IFC_SUBTREE);
    let mut moved = block;
    let mut child = block;
    for parent in document
        .layout_ancestors(block)
        .take_while(|&id| id != body_id)
    {
        // Content before `child`, such as a float beside the paragraph,
        // already sits on this page, and so may a later absolutely
        // positioned box placed against the ancestor; moving the ancestor
        // would take either along.
        let siblings = document.nodes[parent].layout_children();
        let child_index = siblings.iter().position(|&sibling| sibling == child);
        let renders_before = siblings.iter().enumerate().any(|(index, &sibling)| {
            let node = &document.nodes[sibling];
            let renders = match node.kind() {
                NodeKind::Element => {
                    node.is_in_document()
                        && !node.is_non_rendered_html_element()
                        && document.nodes[sibling].display() != DisplayValue::None
                }
                _ => node
                    .text_content()
                    .is_some_and(|text| !text.trim().is_empty()),
            };
            renders
                && (child_index.is_none_or(|child_index| index < child_index)
                    || node.kind() == NodeKind::Element
                        && matches!(
                            document.nodes[sibling].position(),
                            PositionValue::Absolute | PositionValue::Fixed
                        ))
        });
        if renders_before {
            break;
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
            child_top - document.nodes[child].layout.location.y
        };
        child = parent;
        child_top = parent_top;
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        let layout = &document.nodes[parent].layout;
        if layout.border.top != 0.0
            || layout.padding.top != 0.0
            || (parent_top - block_top).abs() > 0.01
        {
            break;
        }
        moved = parent;
    }
    moved
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
    ) && (!inset_is_auto(computed.top) || !inset_is_auto(computed.bottom))
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
        document.table_layout_node_mut(node_id).layout.location.y += delta;
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
    let boxes: Vec<usize> = ifc.boxes.iter().map(|b| b.node).collect();
    for b in boxes {
        let location = &mut document.nodes[b].layout.location;
        if location.y >= old_top - 0.01 {
            location.y += delta;
        }
    }
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
    let Some(root) = document.layout_parent_of(node_id) else {
        return IfcBoxShifts::default(); // cov:ignore: a block child of an inline-engine paragraph has a layout parent.
    };
    let layout = document.nodes[node_id].layout;
    let old_top = layout.location.y - delta;
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
        let location = &mut document.nodes[b].layout.location;
        let follows = if kind == IfcBoxKind::OutOfFlow {
            location.y >= old_bottom - 0.01
        } else {
            later && location.y >= old_top - 0.01
        };
        if follows {
            location.y += delta;
            shifts.shifted.push(b);
        }
    }
    shifts
}

fn is_descendant_or_self(document: &Document, node_id: usize, ancestor_id: usize) -> bool {
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
/// the gap.
fn collapsed_margin_start(
    document: &Document,
    cascade: &CascadeResult,
    previous: usize,
    node_id: usize,
    previous_bottom: f32,
    border_box_top: f32,
) -> f32 {
    let collapsed = adjoining_margin(document, cascade, previous, false)
        .max(adjoining_margin(document, cascade, node_id, true));
    previous_bottom.max(border_box_top - collapsed)
}

/// The largest positive margin among the block-start (`start`) or block-end
/// margin of `id` and the margins of its first or last in-flow child that
/// adjoin it, which all collapse into one (CSS 2 §8.3.1).
fn adjoining_margin(document: &Document, cascade: &CascadeResult, id: usize, start: bool) -> f32 {
    let layout = &document.nodes[id].layout;
    let own = if start {
        layout.margin.top
    } else {
        layout.margin.bottom
    }
    .max(0.0);
    let edges = if start {
        layout.border.top + layout.padding.top
    } else {
        layout.border.bottom + layout.padding.bottom
    };
    if edges > 0.0 || !margins_may_collapse_through(document, cascade, id) {
        return own;
    }
    let mut boxes = Vec::new();
    for &child in document.nodes[id].children.iter().rev() {
        push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
    }
    let child = if start { boxes.last() } else { boxes.first() };
    let Some(&child) = child else {
        return own;
    };
    let child_layout = &document.nodes[child].layout;
    // The child's margin adjoins only when its border box touches this box's
    // content edge, which a used height or a line before it would prevent.
    let touches = if start {
        child_layout.location.y <= 0.001
    } else {
        child_layout.location.y + child_layout.size.height >= layout.size.height - 0.001
    };
    if document.nodes[child].kind() == NodeKind::Element
        && !document.nodes[child].display().is_inline_level()
        && touches
    {
        own.max(adjoining_margin(document, cascade, child, start))
    } else {
        own
    }
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
    let style = document.pagination_style(parent);
    let block_axis = document.block_axis(parent);
    // A percentage height against an indefinite containing block behaves
    // as `auto` (CSS 2.1 §10.5).
    let can_grow =
        block_axis.height().is_auto() || definite_content_height(document, parent).is_none();
    let layout = &node.layout;
    let vertical_edges =
        layout.padding.top + layout.padding.bottom + layout.border.top + layout.border.bottom;
    let box_sizing_edges = if style.box_sizing() == BoxSizing::ContentBox {
        vertical_edges
    } else {
        0.0
    };
    // A percentage bound resolves against a containing block of definite
    // height (CSS 2.1 §10.7).
    let containing_height = BoxTree::for_layout(document)
        .parent(parent, BoxParentScope::Document)
        .and_then(|block| definite_content_height(document, block));
    // One that does not resolve behaves as `0` for `min-height` and as
    // `none` for `max-height`.
    let bound = |value: LengthPercentageAuto, auto: f32| {
        resolve_length_percentage_auto(value, containing_height)
            .map_or(auto, |value| value + box_sizing_edges)
    };
    let min = bound(block_axis.min_height(), 0.0);
    let max = bound(block_axis.max_height(), f32::INFINITY).max(min);
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
    let mut last = None;
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
    // `display: contents` children generate no box: their children are laid
    // out as this box's children. A grid's items follow one another in the
    // order of their rows.
    let mut children = pagination_child_order(document, cascade, parent);
    children.reverse();
    while let Some(child) = children.pop() {
        let child_node = &document.nodes[child];
        if document.nodes[child].display() == DisplayValue::Contents {
            children.extend(child_node.children.iter().rev().copied());
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
        let top = pagination_child_parent_y(document, parent, child, child_layout.location.y);
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
        visit_same_bfc_floats_in_subtree(document, cascade, body_id, child, top, |float, end| {
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
        });
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
        if let Some(((flow_ends, size_ends), _)) =
            last.replace(((flow_ends, size_ends), child_layout.margin.bottom))
        {
            extend(&mut flow, flow_ends);
            extend(&mut size, size_ends);
        }
    }
    // Only the last child's bottom margin, which may be negative, ends the
    // content; earlier margins collapse with the boxes after them.
    if let Some(((flow_ends, size_ends), margin)) = last {
        let margin = if contains_margins { margin } else { 0.0 };
        extend(&mut flow, (flow_ends.0 + margin, flow_ends.1 + margin));
        extend(&mut size, (size_ends.0 + margin, size_ends.1 + margin));
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
    let grow = |old: f32, (before, after): (f32, f32)| {
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
    };
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
    shifted.then(|| GrownHeight {
        height: if can_grow {
            grow(old, size).0.max(in_flow_height)
        } else {
            old
        },
        flow_heights: (old, in_flow_height),
        float_extent_delta,
        capped,
    })
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

/// Visits each left or right float end in `branch`, in `parent`'s flow
/// coordinates. Traversal stops at an independent BFC so its floats stay
/// represented by its border box.
fn visit_same_bfc_floats_in_subtree(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    branch: usize,
    branch_top: f32,
    mut visit: impl FnMut(usize, f32),
) {
    if document.nodes[branch].kind() != NodeKind::Element
        || holds_floats(document, cascade, branch)
        || (matches!(cascade.computed[branch].float, FloatValue::Footnote)
            && is_floating_box_for_pagination(document, cascade, branch))
        // cov:ignore: abs/fixed are caught above; running() is normalized by cascade.
        || !matches!(
            document.nodes[branch].position(),
            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
        )
    {
        return;
    }
    let branch_offset = branch_top
        - layout_relative_offset_y(document, cascade, body_id, branch)
        - if document.contributes_layout_offset(branch) {
            0.0
        } else {
            document.nodes[branch].layout.location.y
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
                offset + layout.location.y
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
            let bottom = offset + layout.location.y + layout.size.height + layout.margin.bottom;
            if bottom.is_finite() {
                visit(child, bottom);
            }
            continue;
        }
        if holds_floats(document, cascade, child) {
            continue;
        }
        let relative_offset = layout_relative_offset_y(document, cascade, body_id, child);
        let child_offset = if document.contributes_layout_offset(child) {
            offset + layout.location.y - relative_offset
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
        sibling_adjacency::FlowBox::Skip => {}
        sibling_adjacency::FlowBox::Contents => {
            for &id in document.nodes[node_id].children.iter().rev() {
                push_in_flow_boxes_rev(document, cascade, id, boxes);
            }
        }
        sibling_adjacency::FlowBox::Box => boxes.push(node_id),
    }
}

/// A forced break after a box, kept attached to that box until its
/// descendants have been visited. Applying it to the first flex child would
/// turn a break-after on the flex container into an extra blank page.
#[derive(Default)]
struct PendingBreak {
    source: Option<usize>,
}

impl PendingBreak {
    /// Whether the pending break falls before `node_id`, which is outside the
    /// box that forced it.
    fn applies_to(&self, document: &Document, node_id: usize) -> bool {
        self.source
            .is_some_and(|source| !is_descendant_or_self(document, node_id, source))
    }

    /// Drop the pending break once a later box has consumed it.
    fn clear(&mut self) {
        self.source = None;
    }

    /// Make `node_id` the source of the pending break when it forces a break
    /// after itself, unless that source is already the node itself or one of
    /// its ancestors.
    fn note_after(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        deferred_named_break_after: bool,
    ) {
        let breaks_after =
            page_break_is_forced(document, node_id, cascade.computed[node_id].break_after)
                || deferred_named_break_after;
        let source_is_ancestor = self
            .source
            .is_some_and(|source| is_descendant_or_self(document, node_id, source));
        if breaks_after && !source_is_ancestor {
            self.source = Some(node_id);
        }
    }
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
/// its lead from the table's top edge. Except after a forced break, the box's
/// `relative_offset` moves with it to the page it starts on.
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
        underflow_y
    } else if forced_transition {
        target_y + margin_top.max(0.0) + table_row_lead
    } else if let Some(lead) = row_group_lead {
        target_y + lead + relative_offset
    } else if table_row_overflow {
        target_y + table_row_lead + relative_offset
    } else {
        target_y + relative_offset
    }
}

/// Where a page transition places a box, and how far that moves the box and
/// the flow after it.
struct TransitionLanding {
    /// The top of the page the box moves to.
    page_top: f32,
    /// Where the box's border box starts after the move (see
    /// [`transition_target_y`]).
    target_y: f32,
    /// How far the box itself moves down.
    node_delta: f32,
    /// How far the flow after the box moves down.
    shift_delta: f32,
}

impl TransitionLanding {
    /// The landing of a box that was materialized at `effective_y` and moves
    /// to the page that starts at `page_top`.
    ///
    /// Both distances are the additional movement for this boundary, since the
    /// box already sits at `effective_y`. The flow shift stays at the page
    /// boundary even when the box itself underflows into the preceding page.
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
            page_top,
            target_y,
            node_delta: target_y - effective_y,
            shift_delta: page_top - effective_y,
        }
    }

    /// Whether both distances are usable numbers, so that the box can move.
    fn is_finite(&self) -> bool {
        self.node_delta.is_finite() && self.shift_delta.is_finite()
    }

    /// The correction that later siblings still need when the box underflows
    /// into the page above the one it moves to: its descendants follow the
    /// box, while the flow after it begins at the page boundary. It is only
    /// meaningful when [`is_finite`](Self::is_finite) holds.
    fn underflow_correction(&self) -> Option<f32> {
        (self.target_y < self.page_top).then_some(self.shift_delta - self.node_delta)
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

/// Whether a run of `run_height` that avoids breaks inside it, starting at
/// `effective_y` below a block-start `margin_top`, moves whole to the next
/// page, whose block size is `next_page_step`.
///
/// A run of sibling blocks joined by `break-before` or `break-after: avoid`
/// stays intact when its block size, through the last box's trailing margin,
/// would cross the current page. This is the direct block-flow case; nested
/// formatting contexts still require fragment-level break opportunities.
fn avoided_run_moves(
    run_height: f32,
    margin_top: f32,
    effective_y: f32,
    page_start: f32,
    page_end: f32,
    next_page_step: f32,
) -> bool {
    run_height.is_finite()
        && run_height > 0.0
        && run_height + margin_top.max(0.0) <= next_page_step
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

/// How far below the border-box top of the in-flow block `node_id` the line
/// of its paragraph ends, when the paragraph is a single line of generated
/// text that no text node places (see [`single_unplaced_line`]).
///
/// No text candidate moves such a line, so its block moves whole when the
/// line would cross a page edge, as a line of text does. Only a horizontal
/// paragraph is measured: the lines of a vertical one are columns.
fn unplaced_line_end(document: &Document, cascade: &CascadeResult, node_id: usize) -> Option<f32> {
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
    let line = single_unplaced_line(document, node_id)?;
    let layout = document.ifc_layout_node(node_id)?.layout;
    Some(layout.border.top + layout.padding.top + line.bottom)
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

/// Whether a box ending the blocks around it with `edges` of bottom padding
/// and border below it moves to the next page, so that those edges do not
/// break away from it.
///
/// When the last box of a block fits its page but the block's closing edges
/// do not, a break between the two would leave the edges alone at the top of
/// the next page. A break between boxes (a class A break point) is preferred
/// to that one (class C, CSS Break 3 §4.1), as browsers do, so the last box
/// moves along with the edges, even when the next page cannot hold the box
/// with all of them: the edges run on past its end as they would anywhere.
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
fn closing_edges_move(candidate: &PageCandidate, fit: &BoxFit) -> bool {
    let edges = candidate.context.closing_edges();
    let inside = candidate.context.closing_edges_inside();
    let span = candidate.context.closing_edges_span();
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
        && box_end - inside <= page_end + 0.001
        && candidate.height + span - inside <= next_page_step + 0.001
        && avoided_box_moves(flow_y, margin_top, box_end + edges, page_start, page_end)
}

/// The block size of the bottom padding and border of the block `node_id`
/// that close it below its last box, when they follow that box: the block is
/// not the body, stacks its boxes in block flow, and has a height that is that
/// of its content.
fn closing_edges_of(
    document: &super::context::LayoutSession<'_>,
    node_id: usize,
    body_id: usize,
) -> Option<f32> {
    let node = &document.nodes[node_id];
    (node_id != body_id
        && node.kind() == NodeKind::Element
        && !node.used_writing_mode.is_vertical()
        && moves_later_siblings(document, node_id)
        && height_follows_content(document, node_id))
    .then_some(node.layout.padding.bottom + node.layout.border.bottom)
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
    document.nodes[node_id].layout.location.y
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
/// flow, a paragraph of one line, or a flex container, grid or table that moves
/// whole (see [`container_moves_whole`]). A float within the block moves with
/// it.
///
/// With no box before it in the block, a break that would leave this tail
/// alone on the next page falls before the block instead (CSS Break 3 §4.1
/// prefers a break between boxes to one between a box and the edges of its
/// parent), as browsers do. A paragraph of more lines breaks between them and
/// leaves the tail to its last lines, which is not this block's to move.
fn lone_box_tail(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    let mut current = node_id;
    // Where the box ends, from the top of `node_id`.
    let mut content_end = 0.0;
    loop {
        let node = &document.nodes[current];
        // Generated boxes are not among the children, and the columns of a
        // multicol container break on their own.
        if node.multicol.is_some() || node.has_before_or_after_content {
            return None;
        }
        if current != node_id && node.is_ifc_root() {
            let lines = document
                .ifc_layout_node(current)
                .and_then(|node| node.ifc.as_ref())
                .and_then(|ifc| ifc.lines.as_ref())?;
            if !lines.block_line_starts.is_empty() || lines.lines.len() > 1 {
                return None;
            }
            break;
        }
        let mut boxes = Vec::new();
        for &child in &node.children {
            if is_floating_box_for_pagination(document, cascade, child) {
                // A float beside the box moves along with the block.
                if !float_within_box(document, node_id, child) {
                    return None;
                }
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
                if closing_edges_of(document, current, body_id).is_some()
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
                if closing_edges_of(document, current, body_id).is_some()
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
    Some(document.nodes[node_id].layout.size.height - content_end)
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
        || lone_box_tail(document, cascade, body_id, node_id).is_some()
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
    // A block box in the flow that takes block space. Anything else between
    // the boxes, such as a float or text, is left to the rules for it.
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
        // A child that generates no box, such as collapsed white space, is
        // not between the boxes.
        let Some(before) = (0..first)
            .rev()
            .find(|&index| flow_flags[index] != (None, false))
        else {
            break true;
        };
        if !is_box(before) {
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
    // A break inside one of the boxes of the run at a better place than
    // before the run is taken instead.
    (first != last
        && (first..=last)
            .filter(|&index| flow_flags[index] != (None, false))
            .all(|index| {
                moves_whole_at_best(document, cascade, body_id, children[index], has_named_pages)
            }))
    .then_some(if starts_block {
        AvoidChain::Start
    } else {
        AvoidChain::Run { first }
    })
}

/// For each child of a block, how far below its border-box top the run of
/// boxes tied by avoided breaks ends that the child starts, when something in
/// the flow of the block lies before the child: the page may break before it,
/// and the run moves to the next page together when it would cross the edge.
/// `children` and `flow_flags` are as for [`avoid_chain_before_last_box`].
///
/// Only block boxes next to each other make a run: a float or text between
/// them ends it, and a table ends it by joining it (see
/// [`ends_a_run_as_table`]). The run ends where its last box holds the page
/// (see [`avoided_run_head`]).
fn nested_run_units(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    children: &[usize],
    flow_flags: &[(Option<usize>, bool)],
) -> Vec<Option<f32>> {
    // A table ends a run it joins and starts none: only its first row stays
    // with the box before it.
    let is_flow_box = |index: usize| {
        let id = children[index];
        let (in_flow, floated) = flow_flags[index];
        in_flow == Some(id) && !floated && !takes_no_block_space(document, id)
    };
    let is_box = |index: usize| is_flow_box(index) && is_plain_block_box(document, children[index]);
    let is_run_end = |index: usize| {
        is_box(index)
            || (is_flow_box(index) && ends_a_run_as_table(document, cascade, children[index]))
    };
    let first_in_flow = flow_flags
        .iter()
        .position(|&(in_flow, _)| in_flow.is_some());
    let mut units = vec![None; children.len()];
    let mut finish = |run: Option<(usize, usize)>| {
        let Some((start, last)) = run.filter(|&(start, _)| Some(start) != first_in_flow) else {
            return;
        };
        let (start_box, last_box) = (children[start], children[last]);
        let last_height = document.nodes[last_box].layout.size.height;
        let head = match avoided_run_head(document, cascade, body_id, last_box, true).end {
            Some(head) if head < last_height - 0.001 => head,
            _ => last_height + document.nodes[last_box].layout.margin.bottom.max(0.0),
        };
        units[start] = Some(
            normal_flow_y(document, cascade, body_id, last_box)
                - normal_flow_y(document, cascade, body_id, start_box)
                + head,
        );
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
            is_box(before)
                && boxes_are_tied(document, cascade, children[before], children[index])
                && is_run_end(index)
        });
        match (tied_before, run) {
            (Some(_), Some((start, _))) => run = Some((start, index)),
            (Some(before), None) => run = Some((before, index)),
            (None, _) => {
                finish(run.take());
            }
        }
        previous = Some(index);
    }
    finish(run);
    units
}

/// The break that keeps a paragraph's last `widows` lines with the `edges` of
/// bottom padding and border that close the blocks around it, when the lines
/// fit the page but the edges do not. A break between the last line and the
/// edges (class C) is not taken while one between lines (class B) honors
/// `orphans` and `widows`, as in [`closing_edges_move`]. A paragraph with no
/// more lines than `orphans` keeps the orphans it can, down to one line, and
/// its last line moves. The edges run on past the end of the next page as
/// they would anywhere; that page only has to hold the lines. `line_metrics`
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
    let widows = usable_widows(line_metrics.len(), orphans, widows);
    widows_break_on_next_page(
        line_metrics,
        effective_y,
        current_page,
        widows,
        page_origins,
    )
}

/// Whether the float `float` lies within the block-axis extent of the content
/// of the box `unit` that holds it, margins included. It then moves along with
/// `unit`, ends where the content does, and reaches no page edge that the
/// content does not: it is no place to break at. A float that reaches into the
/// padding or border below the content, or past the box, is not.
fn float_within_box(document: &Document, unit: usize, float: usize) -> bool {
    let layout = &document.nodes[float].layout;
    let top = current_abs_y(document, float) - layout.margin.top;
    let bottom = current_abs_y(document, float) + layout.size.height + layout.margin.bottom;
    let unit_layout = &document.nodes[unit].layout;
    let unit_top = current_abs_y(document, unit);
    let content_bottom =
        unit_top + unit_layout.size.height - unit_layout.padding.bottom - unit_layout.border.bottom;
    top >= unit_top - 0.001 && bottom <= content_bottom + 0.001
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
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let mut pending = document.nodes[node_id].children.clone();
    while let Some(id) = pending.pop() {
        let node = &document.nodes[id];
        // Text and elements that generate no box break nothing.
        if node.kind() != NodeKind::Element
            || !node.is_in_document()
            || node.is_non_rendered_html_element()
            || node.display() == DisplayValue::None
        {
            continue;
        }
        let computed = &cascade.computed[id];
        if page_break_is_forced(document, id, computed.break_before)
            || page_break_is_forced(document, id, computed.break_after)
            || selected_page_name(cascade, id).is_some()
        {
            return true;
        }
        pending.extend_from_slice(&node.children);
    }
    false
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
/// whose rows end exactly at the page end breaks before its last row instead,
/// which is not done.
fn container_moves_whole(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    has_named_pages: bool,
) -> bool {
    let node = &document.nodes[node_id];
    // Generated boxes are not among the children, so the container cannot be
    // judged.
    if !is_plain_container_box(document, node_id)
        || node.multicol.is_some()
        || node.has_before_or_after_content
        || is_floating_box_for_pagination(document, cascade, node_id)
        || has_mandatory_break_inside(document, cascade, node_id)
    {
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
            // before the block. Named pages may start a page between boxes
            // whatever ties them.
            [_, _, ..] if !has_named_pages => {
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
    /// When a forced-break box underflows into the preceding page, descendants
    /// follow the box while later siblings still begin after the break
    /// boundary: `(box, correction)`.
    underflow: Option<(usize, f32)>,
    /// Lines of a paragraph that were moved to a later page to keep enough
    /// widows there: `(root, first moved line, shift)`. The flow after the
    /// paragraph, and its texts that start on a moved line, take each shift
    /// once they are reached; its texts before that line stay where they are.
    line_splits: Vec<(usize, usize, f32)>,
}

impl DeferredShifts {
    /// Adds to `flow_shift` every pending shift that `node_id` has reached,
    /// line splits first, and drops them.
    fn apply_reached(
        &mut self,
        document: &Document,
        text_lines: &TextLineIndex,
        node_id: usize,
        ifc_root: Option<(usize, f32)>,
        flow_shift: &mut f32,
    ) {
        self.line_splits.retain(|&(root, first_moved, shift)| {
            let reached = !is_descendant_or_self(document, node_id, root)
                || ifc_root.is_some_and(|(text_root, _)| text_root == root)
                    && text_lines
                        .get(node_id)
                        .and_then(|owned| owned.lines.first())
                        .is_some_and(|line| line.line >= first_moved)
                || follows_line_of_paragraph(document, node_id, root, first_moved);
            if reached {
                *flow_shift += shift;
            }
            !reached
        });
        if let Some((ancestor_id, correction)) = self.underflow
            && !is_descendant_or_self(document, node_id, ancestor_id)
        {
            *flow_shift += correction;
            self.underflow = None;
        }
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
    /// Blocks whose first line was checked against the page end.
    block_text: HashSet<usize>,
    /// Blocks whose orphans and widows were checked.
    orphans_widows: HashSet<usize>,
    /// Runs of lines between the block children of a paragraph that were
    /// checked against the page end: `(paragraph root, first line)`.
    lines_after_blocks: HashSet<(usize, usize)>,
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
/// is moved the same way as a direct child; its ancestors then grow to hold
/// it. There is no such block for text placed by positioned insets, for a
/// block whose checks have all run, or for a nested block that is not in
/// plain block flow or that lies inside a float or an out-of-flow box.
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
                (!candidate.context.is_float_descendant()
                    && !candidate.context.is_out_of_flow_descendant()
                    && is_block_flow_paragraph(document, cascade, parent_id, roots.element))
                .then_some((parent_id, true))
            }
        })
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

/// Whether in-flow content comes before the box `node_id` in the flow below
/// `body_id`: a sibling of the box, or of an ancestor of it that is not the
/// body, which generates an in-flow box (see [`push_in_flow_boxes_rev`]). The
/// flow can break before such a box, between it and what comes first. A box
/// that starts the flow, below nothing but margins, cannot be broken from.
fn follows_flow_content(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> bool {
    let mut node = node_id;
    let mut boxes = Vec::new();
    while node != body_id {
        let Some(parent) = document.parent_of(node) else {
            return false;
        };
        for &child in document.nodes[parent]
            .children
            .iter()
            .take_while(|&&child| child != node)
        {
            push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
        }
        if !boxes.is_empty() {
            return true;
        }
        node = parent;
    }
    false
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

/// Whether later lines of the paragraph `block_id` can move down inside it:
/// its height grows with them (a box with a definite height keeps it), and it
/// holds nothing but text in one column.
fn lines_can_shift_inside(document: &super::context::LayoutSession<'_>, block_id: usize) -> bool {
    height_follows_content(document, block_id)
        && document
            .ifc_layout_node(block_id)
            .and_then(|node| node.ifc.as_ref())
            .is_some_and(|ifc| ifc.boxes.is_empty() && ifc.multicol_fragments.is_none())
}

/// Whether the lines of the paragraph `block_id` that follow its block
/// children can move down inside it: its height grows with them, it is in one
/// column, and the only boxes it lays out are those block children, which
/// follow the moved lines when pagination reaches them.
fn lines_can_shift_between_blocks(
    document: &super::context::LayoutSession<'_>,
    block_id: usize,
) -> bool {
    height_follows_content(document, block_id)
        && document
            .ifc_layout_node(block_id)
            .and_then(|node| node.ifc.as_ref())
            .is_some_and(|ifc| {
                ifc.multicol_fragments.is_none()
                    && ifc.boxes.iter().all(|b| b.kind == IfcBoxKind::Block)
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
    let first = text_lines.get(node_id)?.lines.first()?.line;
    let lines = document
        .ifc_layout_node(root)?
        .ifc
        .as_ref()?
        .lines
        .as_ref()?;
    let mut start = None;
    let mut end = lines.lines.len();
    for &(_, after) in &lines.block_line_starts {
        if after <= first {
            start = start.max(Some(after));
        } else {
            end = end.min(after);
        }
    }
    Some(start?..end)
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

/// Whether `node_id` lies in a block child of the paragraph `root` that comes
/// after line `line`, so that moving the lines from `line` on moves it too.
fn follows_line_of_paragraph(
    document: &Document,
    node_id: usize,
    root: usize,
    line: usize,
) -> bool {
    let block = std::iter::successors(Some(node_id), |&node| document.layout_parent_of(node))
        .find(|&node| document.layout_parent_of(node) == Some(root));
    document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .and_then(|ifc| ifc.lines.as_ref())
        .zip(block)
        .is_some_and(|(lines, block)| {
            lines
                .block_line_starts
                .iter()
                .any(|&(child, after)| child == block && after > line)
        })
}

/// How a paragraph that crosses the page end is repaired so that its break
/// honors `orphans` and `widows`.
#[derive(Debug, PartialEq)]
enum LineBreakFix {
    /// The natural break stays.
    Keep,
    /// Lines move down inside the paragraph, the earlier break first.
    MoveLines([Option<LineBreakMove>; 2]),
    /// The whole block moves to the next page.
    MoveBlock,
}

/// How a paragraph's lines fall against the end of the current page, and
/// whether the natural break between them honors `orphans` and `widows`.
struct ParagraphLineFit {
    /// The number of lines the paragraph's text owns.
    total_lines: usize,
    /// Whether the first line starts on the current page.
    starts_on_page: bool,
    /// The leading lines whose block-end edge is at or before the page end,
    /// up to the first line that is not.
    lines_before_break: usize,
    /// The used `orphans`, at least 1.
    orphans: usize,
    /// The used `widows`, at least 1.
    widows: usize,
    /// Whether the lines need a break that leaves fewer than `orphans`
    /// lines before it or fewer than `widows` lines after it.
    violates_line_constraints: bool,
}

impl ParagraphLineFit {
    /// Whether the first line starts on the current page and the paragraph
    /// has lines enough for both `orphans` and `widows`.
    fn has_room_for_both_constraints(&self) -> bool {
        self.starts_on_page && self.orphans + self.widows <= self.total_lines
    }

    /// Whether a block taller than a page gains from moving whole to the next
    /// page: the break leaves fewer than `orphans` lines on this page, and
    /// the first line, `first_line_bottom` below the top of the block, fits a
    /// page of `page_step`. A line taller than the page gains nothing by
    /// moving.
    fn gains_from_moving_tall_block(&self, first_line_bottom: f32, page_step: f32) -> bool {
        self.leaves_too_few_orphans() && block_fits_page_step(first_line_bottom, page_step)
    }

    /// Whether the break leaves fewer than `orphans` lines on the page, or
    /// all the lines of a paragraph shorter than that do not fit it.
    fn leaves_too_few_orphans(&self) -> bool {
        self.lines_before_break < self.orphans.min(self.total_lines)
    }

    /// The line moves that split a paragraph able to move its lines down
    /// inside it: a break moved up between earlier lines, then the break on
    /// the paragraph's last page.
    ///
    /// When only the widows are short, the break moves up between earlier
    /// lines (CSS Fragmentation 3 §3.3), so the last `widows` lines start the
    /// next page and the lines before them stay.
    fn line_moves(
        &self,
        line_metrics: &[(f32, f32)],
        effective_y: f32,
        current_page: u32,
        page_origins: &PageOrigins,
    ) -> [Option<LineBreakMove>; 2] {
        let earlier_break = (self.violates_line_constraints
            && self.lines_before_break >= self.orphans)
            .then(|| {
                widows_break_on_next_page(
                    line_metrics,
                    effective_y,
                    current_page,
                    self.widows,
                    page_origins,
                )
            })
            .flatten();
        let final_break = widows_break_on_last_page(
            line_metrics,
            effective_y,
            current_page,
            self.orphans,
            self.widows,
            earlier_break,
            page_origins,
        );
        [earlier_break, final_break]
    }

    /// How to repair the break between the paragraph's lines on the page
    /// `current_page`. A block that can move whole moves whenever the break
    /// leaves too few orphans, as lines moved down inside it would leave
    /// its top on this page. Else lines move down inside the paragraph when
    /// it has lines enough for both constraints and `can_move_lines` allows
    /// it; else the whole block moves when `can_move_block` and the break
    /// violates a constraint; otherwise the natural break stays.
    fn fix(
        &self,
        line_metrics: &[(f32, f32)],
        effective_y: f32,
        current_page: u32,
        page_origins: &PageOrigins,
        can_move_block: bool,
        can_move_lines: impl FnOnce() -> bool,
    ) -> LineBreakFix {
        let move_block = can_move_block && self.starts_on_page && self.violates_line_constraints;
        let moves = if !(move_block && self.leaves_too_few_orphans())
            && self.has_room_for_both_constraints()
            && can_move_lines()
        {
            self.line_moves(line_metrics, effective_y, current_page, page_origins)
        } else {
            [None, None]
        };
        if moves.iter().any(Option::is_some) {
            LineBreakFix::MoveLines(moves)
        } else if move_block {
            LineBreakFix::MoveBlock
        } else {
            LineBreakFix::Keep
        }
    }

    /// Measures `line_metrics`, the block-start and block-end edges of each
    /// line relative to `effective_y`, against the page from `page_start`
    /// to `page_end`, with the computed `orphans` and `widows`.
    fn measure(
        line_metrics: &[(f32, f32)],
        effective_y: f32,
        page_start: f32,
        page_end: f32,
        orphans: i32,
        widows: i32,
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
        let widows = usable_widows(total_lines, orphans, widows.max(1) as usize);
        let violates_line_constraints = needs_break
            && (lines_before_break < orphans
                || total_lines.saturating_sub(lines_before_break) < widows);
        Self {
            total_lines,
            starts_on_page,
            lines_before_break,
            orphans,
            widows,
            violates_line_constraints,
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
    /// The empty box whose block-end margin still adjoins the latest forced
    /// break: the forced-break box itself, or an empty box right after it.
    /// A margin collapsed through it follows the break and is kept.
    forced_break_box: Option<usize>,
    /// Whether truncating a margin moved a box.
    moved_boxes: bool,
    /// The box that forces a break after it, which a float took to the new
    /// page, until the in-flow box after the float is placed. That box starts
    /// the new page as well, so the break sits before its margin.
    break_taken_by_float: Option<usize>,
}

/// A box that starts on `natural_page`, below a page edge that may fall in the
/// margin above it.
struct MarginBreak {
    node_id: usize,
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
}

/// How a page edge truncates the margin above a box.
struct MarginTruncation {
    /// How far the box moves; negative when it moves up.
    delta: f32,
    /// How far a negative margin pulls the box above the top of its page,
    /// if it does.
    underflow: Option<f32>,
}

impl SiblingMargins {
    /// How the top of `at.natural_page` truncates the margin between the box
    /// `at` and the sibling block before it, or `None` when the page edge
    /// falls outside that margin or no margin separates the two. A box that
    /// clears floats is left alone, since its gap may hold clearance.
    fn truncation_at_break(
        &mut self,
        document: &super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        page_origins: &PageOrigins,
        at: &MarginBreak,
    ) -> Option<MarginTruncation> {
        let node_id = at.node_id;
        // A clearing box's gap may contain clearance. Its used clearance is
        // not recorded separately, so retain the gap.
        if !at.collapses_with_previous_block
            || !matches!(
                cascade.computed[node_id].clear,
                raikiri_style::property::ClearValue::None
            )
        {
            return None;
        }
        let (previous, gap) = self.gap_after_previous(document, node_id, at.raw_y)?;
        let forced_before = at.forced_before
            || self
                .break_taken_by_float
                .is_some_and(|source| is_descendant_or_self(document, source, previous));
        let page_top = page_origins.origin(at.natural_page);
        let previous_bottom = at.effective_y - gap;
        let follows_forced_break = self.follows_forced_break(document, cascade, previous, node_id);
        if !page_edge_falls_in_margin(
            at.natural_page,
            page_top,
            previous_bottom,
            at.effective_y,
            follows_forced_break,
        )
            // Only a margin between adjoining siblings is truncated; a box
            // that is not a candidate may lie between two nested candidates.
            || !self.adjacency.adjoins(document, cascade, node_id, previous)
        {
            return None;
        }
        let (target_top, target_y) = page_origins.truncated_margin_target(
            at.natural_page,
            forced_before,
            at.margin_top,
            || {
                collapsed_margin_start(
                    document,
                    cascade,
                    previous,
                    node_id,
                    previous_bottom,
                    at.effective_y,
                )
            },
            follows_forced_break,
        );
        Some(MarginTruncation {
            delta: target_y - at.effective_y,
            underflow: (target_y < target_top).then_some(target_top - target_y),
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
    /// `node_id`, if both are known and the gap is finite.
    fn gap_after_previous(
        &self,
        document: &Document,
        node_id: usize,
        raw_y: f32,
    ) -> Option<(usize, f32)> {
        BoxTree::for_layout(document)
            .parent(node_id, BoxParentScope::Document)
            .and_then(|parent_id| self.previous_bottoms.get(&parent_id).copied())
            .map(|(previous, bottom)| (previous, raw_y - bottom))
            .filter(|(_, gap)| gap.is_finite() && *gap > 0.0)
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
    ) {
        let follows_forced_box =
            self.previous_bottoms
                .get(&parent_id)
                .is_some_and(|&(previous, _)| {
                    self.follows_forced_break(document, cascade, previous, node_id)
                });
        self.forced_break_box = (follows_forced_box && collapses_through).then_some(node_id);
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
}

impl SubtreeShifts {
    /// End the shifts of the subtrees `node_id` is outside of, applying
    /// them to `flow_shift`, and return the page the flow returns to when a
    /// bounded subtree ends.
    ///
    /// Leaving several bounded subtrees at once returns to the page the
    /// earliest of them started from.
    fn leave(&mut self, document: &Document, node_id: usize, flow_shift: &mut f32) -> Option<u32> {
        let mut restored_page: Option<u32> = None;
        self.growth.retain(|&(context, growth)| {
            if is_descendant_or_self(document, node_id, context) {
                return true;
            }
            *flow_shift += growth;
            false
        });
        self.bounded.retain(|&(ancestor_id, delta, flow_page)| {
            if is_descendant_or_self(document, node_id, ancestor_id) {
                return true;
            }
            *flow_shift -= delta;
            restored_page = Some(restored_page.map_or(flow_page, |page| page.min(flow_page)));
            false
        });
        restored_page
    }
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
        .or_else(|| used_computed_length_percentage_or_auto(computed.margin.top, content_width))
        .unwrap_or(0.0);
    let trailing_margin = if is_floating_box_for_pagination(document, cascade, node_id) {
        used_style_length_percentage_auto(block_axis.margin_bottom(), content_width)
            .or_else(|| {
                used_computed_length_percentage_or_auto(computed.margin.bottom, content_width)
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
/// it lands on records the placement.
fn place_first_table_header(
    document: &mut Document,
    table_headers: &mut HeaderRepeats,
    row: usize,
    flow_shift: f32,
    page_origins: &PageOrigins,
) {
    let Some(&header_id) = table_headers.rows.get(&row) else {
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
fn make_room_for_table_header(
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
}

/// Moves the lines of a paragraph from `moved.line` on down by `moved.shift`
/// and brings along what has to follow them: the flex lines after a flex
/// item paragraph, and the ancestors that hold the paragraph, which grow to
/// keep it.
fn move_paragraph_lines(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    moved: &LineMove,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
) {
    shift_ifc_lines_from(document, moved.block_id, moved.line, moved.shift);
    if let Some((item, container)) = moved.flex_line {
        shift_later_flex_lines(document, container, item, moved.shift);
    }
    grow_ancestors_for_shift(
        document,
        cascade,
        moved.block_id,
        moved.shift,
        body_id,
        shifts,
        flow_page,
        &IfcBoxShifts::default(),
    );
}

/// Moves `block_id`, which sits at `from_y`, so that its flow position starts
/// at `target_y`, and returns how far it moved. Its relative offsets move
/// along with it. A `nested` block that begins its ancestors, with no border
/// or padding between them, takes the ancestors along rather than leaving
/// them empty on this page; the ancestors above them grow to hold the move,
/// recorded in `shifts` with `flow_page` as for [`grow_ancestors_for_shift`].
#[allow(clippy::too_many_arguments)]
fn move_line_block(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    block_id: usize,
    nested: bool,
    from_y: f32,
    target_y: f32,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
) -> f32 {
    let relative = accumulated_relative_offset_y(document, cascade, body_id, block_id);
    // A negative or undefined move, which a relative offset can produce,
    // leaves the block where it is.
    let delta = Some(target_y + relative - from_y)
        .filter(|delta| delta.is_finite() && *delta > 0.0)
        .unwrap_or(0.0);
    if delta > 0.0 {
        let moved = if nested {
            first_box_ancestor_at_top(document, block_id, body_id)
        } else {
            block_id
        };
        let moved_y = current_abs_y(document, moved) + delta;
        let followed_ifc_boxes = materialize_y(document, moved, moved_y);
        grow_ancestors_for_shift(
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
    delta
}

/// How far a page transition moved a box and why.
struct TransitionMove {
    /// How far the box itself moves down.
    node_delta: f32,
    /// How far the flow after the box moves down.
    shift_delta: f32,
    /// Whether the box moves because it would cross a page edge it avoids.
    page_overflow: bool,
    /// Whether a forced or page-type break moves the box.
    forced_transition: bool,
}

/// Moves the box `candidate` down by `moved.node_delta` for a page
/// transition and brings along what has to follow it: the lines of its
/// paragraph, and the ancestors or later siblings that hold or trail it.
/// Returns whether an ancestor's height kept it from holding the moved
/// content.
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
        shift_delta,
        page_overflow,
        forced_transition,
    } = *moved;
    document.nodes[node_id].layout.location.y += node_delta;
    let followed_ifc_boxes = follow_moved_ifc_block(document, node_id, node_delta);
    // A nested avoiding block that moves to the next page lengthens the
    // ancestors whose content box holds it, so their continuation (and any
    // `overflow` clip) reaches the moved content. A float does not lengthen
    // the block around it; its own shift ends with its subtree.
    let mut moved_in_bounded_ancestor = false;
    let moves_in_flow_down =
        node_delta > 0.0 && !is_floating_box_for_pagination(document, cascade, node_id);
    let nested_block = !candidate.context.is_direct_body_element() && moves_in_flow_down;
    if (page_overflow || (forced_transition && candidate.context.is_direct_body_element()))
        && moves_in_flow_down
    {
        moved_in_bounded_ancestor = grow_ancestors_for_shift(
            document,
            cascade,
            node_id,
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
        // children in block order. A box pulled back over the break by a
        // negative margin still leaves the flow after it at the break.
        shift_block_flow_after(document, node_id, body_id, node_delta.max(shift_delta));
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
    if context != body_id
        && (document.block_axis(context).height().is_auto()
            || definite_content_height(document, context).is_none())
    {
        let context_top = current_abs_y(document, context);
        let float_bottom = current_abs_y(document, node_id)
            + height
            + document.nodes[node_id].layout.margin.bottom;
        let layout = &document.nodes[context].layout;
        let grown = (float_bottom - context_top + layout.padding.bottom + layout.border.bottom)
            .min(max_border_box_height(document, cascade, context).unwrap_or(f32::INFINITY));
        let growth = grown - layout.size.height;
        if growth > 0.001 {
            document.nodes[context].layout.size.height += growth;
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
            shifts.growth.push((context, growth));
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
    let mut bounded = false;
    let mut child = node_id;
    // How far the flow after `child` moved, and how much `child` moved or
    // grew: a float hanging below its content can take up part of the move.
    let mut flow_delta = delta;
    let mut size_delta = delta;
    let mut float_delta = delta;
    let mut walk = AncestorCursor::new(document.parent_of(child), ParentRelation::Source);
    while let Some(parent) = walk.current() {
        if document.nodes[parent].kind() != NodeKind::Element {
            break;
        }
        walk.advance(document);
        // A `display: contents` element has no box to grow; its parent lays
        // the moved box out.
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        shift_later_block_siblings(document, parent, child, flow_delta);
        let height = document.nodes[parent].layout.size.height;
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
        let owns_floats = holds_floats(document, cascade, parent);
        bounded |= grown.capped || flow_absorbed < flow_delta - 0.001;
        if flow_absorbed < flow_delta - 0.001 {
            shifts
                .bounded
                .push((parent, flow_delta - flow_absorbed, flow_page));
        } else if flow_absorbed > flow_delta + 0.001 {
            // A box holding its floats grew around a float that moved with
            // the content it overflowed: the flow after the box follows.
            shifts.growth.push((parent, flow_absorbed - flow_delta));
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
        flow_delta = flow_absorbed;
        size_delta = size_absorbed;
        child = parent;
        if parent == body_id {
            break;
        }
    }
    bounded
}

/// Move `node_id` by `delta` after its block-start margin was truncated at a
/// page break, together with its inline content and the auto-height boxes
/// containing it.
fn move_box_by_truncated_margin(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    delta: f32,
    flow_root: usize,
) {
    document.nodes[node_id].layout.location.y += delta;
    let followed_ifc_boxes = follow_moved_ifc_block(document, node_id, delta);
    shrink_ancestors_after_margin_truncation(
        document,
        cascade,
        node_id,
        delta,
        flow_root,
        &followed_ifc_boxes,
    );
}

/// A truncated margin shortens the auto-height boxes containing the flow.
/// Reuse the same used-child extents and size bounds as ordinary pagination
/// growth, retaining unrelated float extents and authored height limits.
fn shrink_ancestors_after_margin_truncation(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node: usize,
    delta: f32,
    flow_root: usize,
    ifc_boxes: &IfcBoxShifts,
) {
    if delta >= 0.0 || !delta.is_finite() {
        return;
    }
    let mut child = node;
    let mut flow_delta = delta;
    let mut size_delta = delta;
    let mut float_delta = delta;
    let mut walk = AncestorCursor::new(document.layout_parent_of(child), ParentRelation::Layout);
    while let Some(parent) = walk.current() {
        walk.advance(document);
        if document.nodes[parent].kind() != NodeKind::Element
            || document.nodes[parent].size_containment.height
        {
            break;
        }
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        shift_later_block_siblings(document, parent, child, flow_delta);
        let old_height = document.nodes[parent].layout.size.height;
        let Some(adjusted) = grown_auto_height(
            document,
            cascade,
            parent,
            child,
            flow_delta,
            size_delta,
            float_delta,
            child == node,
            flow_root,
            ifc_boxes,
        ) else {
            break;
        };
        document.nodes[parent].layout.size.height = adjusted.height;
        let (old_flow, new_flow) = adjusted.flow_heights;
        let owns_floats = holds_floats(document, cascade, parent);
        float_delta = if owns_floats {
            0.0
        } else {
            adjusted.float_extent_delta
        };
        size_delta = adjusted.height - old_height;
        flow_delta = new_flow - old_flow;
        child = parent;
        if parent == flow_root
            || (flow_delta.abs() <= 0.001
                && size_delta.abs() <= 0.001
                && float_delta.abs() <= 0.001)
        {
            break;
        }
    }
}

/// Whether `block` is a block-level box laid out in plain block flow up to
/// the body: every box from it to the body stacks its children in block
/// order, none is floated or absolutely positioned, and its own height and
/// every ancestor's height follow their content (see
/// [`height_follows_content`]), so moving `block` down moves the boxes after
/// it and grows its ancestors to hold it.
fn is_block_flow_paragraph(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
) -> bool {
    if !matches!(
        document.nodes[block].display(),
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
    ) {
        return false;
    }
    // A box whose height does not follow its lines fragments at the break
    // itself, as the same box does as a flex or grid item, rather than moving
    // as a unit with its lines.
    height_follows_content(document, block)
        && placed_in_block_flow(document, cascade, block, body_id)
}

/// Whether every box from `block` up to the body stacks its children in
/// block order and grows with its content, and none is floated or
/// absolutely positioned, so a box growing below `block` pushes the boxes
/// after it down.
fn placed_in_block_flow(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
) -> bool {
    let mut child = block;
    let mut walk = AncestorCursor::new(Some(block), ParentRelation::Layout);
    loop {
        let placed_apart = is_floating_box_for_pagination(document, cascade, child)
            || matches!(
                document.nodes[child].position(),
                PositionValue::Absolute | PositionValue::Fixed
            );
        walk.advance(document);
        match walk.current() {
            Some(parent) if !placed_apart && parent == body_id => return true,
            // A `display: contents` element has no box; its parent lays the
            // child out.
            Some(parent)
                if !placed_apart
                    && (document.nodes[parent].display() == DisplayValue::Contents
                        || moves_later_siblings(document, parent)
                            && height_follows_content(document, parent)) =>
            {
                child = parent;
            }
            _ => return false,
        }
    }
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
    document.nodes[container]
        .children
        .iter()
        .filter(|&&child| {
            // Collapsed white space between items, `display: none` and
            // absolutely positioned children are on no flex line.
            child != item
                && match &document.nodes[child].data {
                    crate::node::NodeData::Text(text) => !text.text_content.trim().is_empty(),
                    _ => {
                        document.nodes[child].display() != DisplayValue::None
                            && !matches!(
                                document.nodes[child].position(),
                                PositionValue::Absolute | PositionValue::Fixed
                            )
                    }
                }
        })
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
                && document.nodes[child].layout.location.y >= old_bottom - 0.001
        })
        .collect();
    for child in later {
        document.nodes[child].layout.location.y += shift;
    }
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

/// Whether `parent` is a block container whose later children follow a
/// moved child. Flex, grid and table containers place their children
/// themselves, and a paragraph's lines follow a moved block child through
/// its line shifts.
fn moves_later_siblings(document: &Document, parent: usize) -> bool {
    matches!(
        document.nodes[parent].display(),
        DisplayValue::Block
            | DisplayValue::FlowRoot
            | DisplayValue::ListItem
            | DisplayValue::InlineBlock
    ) && document.nodes[parent].multicol.is_none()
        && document
            .ifc_layout_node(parent)
            .is_none_or(|node| node.ifc.is_none())
}

/// Moves the boxes that follow `child` in the block container `parent`
/// down by `delta`, the distance `child`'s block-end edge moved: in block
/// flow they follow it.
fn shift_later_block_siblings(document: &mut Document, parent: usize, child: usize, delta: f32) {
    if !moves_later_siblings(document, parent) || delta == 0.0 || !delta.is_finite() {
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
        if document.nodes[sibling].kind() != NodeKind::Element
            || document.nodes[sibling].display() == DisplayValue::None
            || matches!(
                document.nodes[sibling].position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
        {
            continue;
        }
        document.table_layout_node_mut(sibling).layout.location.y += delta;
    }
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
    let computed = &cascade.computed[node_id];
    let item = BoxTree::for_layout(document)
        .parent(node_id, BoxParentScope::Document)
        .is_some_and(|container| {
            document.nodes[container]
                .formatting_context
                .is_flex_or_grid()
        });
    document.nodes[node_id].layout_display() == Display::FlowRoot
        || item
        || !(document.nodes[node_id].display().is_inline_flow()
            || matches!(
                document.nodes[node_id].display(),
                DisplayValue::Block | DisplayValue::ListItem
            ))
        || matches!(computed.float, FloatValue::Left | FloatValue::Right)
        || document.nodes[node_id].layout_position() == Position::Absolute
        || scrolls_overflow(document, computed, node_id)
        || document.nodes[node_id].multicol.is_some()
        || establishes_containment_context(computed)
}

/// Whether the box `node_id` is a scroll container on either axis.
///
/// The only override of `overflow` hands a viewport-propagated value back as
/// `visible`, so a box whose computed `overflow` does not scroll never does.
/// The override-aware view is read for the other boxes only, which keeps the
/// common case out of the hot ancestor walks. Style comparison builds assert
/// the shortcut against the view for every box they lay out, so an override
/// that made a `visible` box scroll has to teach this function first.
fn scrolls_overflow(
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
    let max = match cascade.computed[node_id].max_height {
        ComputedLengthPercentageOrAuto::Px(max) => Some(max),
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
fn candidate_ancestor_constraints(
    document: &super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    body_id: usize,
) -> (bool, bool) {
    let mut side_by_side = false;
    let mut bounded_clipping = false;
    for id in document.ancestors(node_id) {
        if id != body_id && !bounded_clipping {
            let computed = &cascade.computed[id];
            bounded_clipping = !matches!(computed.overflow.y, OverflowValue::Visible)
                && crate::paint_rules::clips_element_overflow(document, cascade, id)
                && (definite_content_height(document, id).is_some()
                    || max_height_is_reached(document, cascade, id));
        }
        if !side_by_side {
            let formatting_context = document.nodes[id].formatting_context;
            side_by_side = document.nodes[id].multicol.is_some()
                || formatting_context == FormattingContext::Flex
                || formatting_context == FormattingContext::Grid
                    && !grid_stacks_items(document, id);
        }
        if (side_by_side && bounded_clipping) || id == body_id {
            break;
        }
    }
    (side_by_side, bounded_clipping)
}

/// The paragraph that lays `block` out as a block among its lines, whose
/// content after the block follows it with its lines.
fn ifc_paragraph_of_block(document: &Document, block: usize) -> Option<usize> {
    document.layout_parent_of(block).filter(|&root| {
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
    })
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
/// block-end margin of the block before it: it is a statically positioned
/// block-level box in the normal flow, and not text, a table row, or a flex
/// or grid item. A relative offset is not part of the flow gap between
/// siblings, so a positioned box cannot tell its margin apart from its
/// offset.
fn collapses_with_previous_block(
    document: &Document,
    candidate: &PageCandidate,
    participates_in_margin_adjacency: bool,
) -> bool {
    let node = &document.nodes[candidate.node_id];
    is_block_level_flow_box(node.display())
        && matches!(node.position(), PositionValue::Static)
        && !candidate.context.is_text()
        && !candidate.context.is_table_row()
        && !candidate.context.is_flex_item()
        && !candidate.context.is_grid_item()
        && participates_in_margin_adjacency
}

/// Moves the boxes after `node_id` in block flow down by `flow_delta`, up to
/// the first ancestor below `body_id` that does not stack its children in
/// block order.
///
/// A `display: contents` ancestor has no box; its parent lays the moved box
/// out among its siblings. A paragraph's content after a block on its lines
/// follows the same way, also when the moved box sits deeper inside that
/// block.
fn shift_block_flow_after(
    document: &mut super::context::LayoutSession<'_>,
    node_id: usize,
    body_id: usize,
    flow_delta: f32,
) {
    let mut child = node_id;
    let mut walk = AncestorCursor::new(document.parent_of(node_id), ParentRelation::Source);
    while let Some(parent) = walk.current().filter(|&parent| parent != body_id) {
        walk.advance(document);
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        let paragraph = ifc_paragraph_of_block(document, child) == Some(parent);
        if !paragraph && !stacks_children_in_block_flow(document, parent) {
            break;
        }
        // The moved box itself already took the lines after it along.
        if paragraph && child != node_id {
            let layout = document.nodes[child].layout;
            let top = layout.location.y;
            let _ = follow_ifc_block(
                document,
                parent,
                child,
                top,
                top + layout.size.height,
                flow_delta,
            );
        }
        shift_later_block_siblings(document, parent, child, flow_delta);
        child = parent;
    }
}

/// Whether the parent of `node_id` is a flex container whose main axis is
/// the inline axis, so its items sit side by side on flex lines.
fn in_row_flex_container(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    document.parent_of(node_id).is_some_and(|parent_id| {
        document.nodes[parent_id].formatting_context == FormattingContext::Flex
            && matches!(
                cascade.computed[parent_id].flex_direction,
                FlexDirectionValue::Row | FlexDirectionValue::RowReverse
            )
    })
}

/// Moves the siblings of `node_id` in `parent_id` that start at its raw
/// position `raw_y` down by `shift_delta`, taking them as the other items on
/// its flex line. Every other sibling keeps its flow coordinates.
fn move_flex_line_siblings(
    document: &mut Document,
    parent_id: usize,
    node_id: usize,
    raw_y: f32,
    shift_delta: f32,
) {
    let siblings = document.nodes[parent_id].children.clone();
    for sibling_id in siblings {
        if sibling_id == node_id {
            continue;
        }
        let sibling_raw_y = current_abs_y(document, sibling_id);
        if (sibling_raw_y - raw_y).abs() <= 0.001 {
            materialize_y(document, sibling_id, sibling_raw_y + shift_delta);
        }
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
