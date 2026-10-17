use super::box_tree::BoxTree;
use crate::node::BoxPropertiesRef;
mod candidates;
pub(crate) mod column_projection;
mod leading_margins;
mod link_events;
mod sibling_shifts;
mod transposed_flow;
use candidates::{CandidateCollector, PageCandidate, has_display_none_descendant};
use column_projection::{ParagraphCache, ParagraphProjection, ProjectionWork};
pub(crate) use link_events::page_fragment_events_from_pages;

use super::ifc::boxes::IfcBoxKind;
use super::ifc::text_lines::{IfcTextLine, TextLineIndex};
use super::table::headers::HeaderRepeats;
use super::*;
use crate::node::FormattingContext;
use crate::page_projection::records::OverflowClipSource;
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
    let mut skipped_parent = document.parent_of(child);
    while let Some(id) = skipped_parent.filter(|&id| id != parent) {
        child_parent_y += document.nodes[id].unrounded_layout.location.y;
        skipped_parent = document.parent_of(id);
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
    document: &Document,
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
        let y = parent_y + node.unrounded_layout.location.y + paint_offset;
        let bottom = if node_id == body_id {
            block_border_box_end(
                document,
                cascade,
                body_id,
                root_flow_offset,
                page_area_height,
                false,
            )
        } else if in_columns.is_none() && node.unrounded_layout.size.height > 0.0 {
            y + node.unrounded_layout.size.height
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
            end = end.max(
                y + node.unrounded_layout.border.top
                    + node.unrounded_layout.padding.top
                    + lines.shifted_height(),
            );
        }
        if in_columns.is_none() {
            let roots = fragment_roots.get(&node_id).map_or(&[][..], Vec::as_slice);
            end = end.max(fragment_tree_end(
                fragments,
                &fragment_children,
                roots,
                y,
                |node_id| element_clips_overflow(document, cascade, node_id),
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

/// The lowest edge painted by the fragments under `roots`, placed at the
/// block offset `y`.
///
/// A fragment paints at its parent's origin plus its own offset, and a
/// fragmentainer clip, intersected with the clips of the enclosing
/// fragments, hides what extends past its column. A fragment whose element
/// `clips_overflow` also hides what its descendants paint past its own box.
fn fragment_tree_end(
    fragments: &[crate::fragment::LayoutFragment],
    fragment_children: &HashMap<usize, Vec<usize>>,
    roots: &[usize],
    y: f32,
    clips_overflow: impl Fn(usize) -> bool,
) -> f32 {
    let mut end = f32::NEG_INFINITY;
    let mut pending: Vec<(usize, f32, f32)> = roots
        .iter()
        .map(|&index| (index, y, f32::INFINITY))
        .collect();
    while let Some((index, parent_origin, inherited_clip_end)) = pending.pop() {
        let rect = fragments[index].rect;
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
    std::iter::successors(Some(node_id), |&id| {
        if id == body_id {
            None
        } else {
            document.parent_of(id)
        }
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
    document: &Document,
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
                if document.pagination_on_transposed_axes {
                    // Physical width percentages use the containing block's
                    // content width, now the transposed layout's height.
                    Some(parent.unrounded_layout.content_box_height())
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
    let layout = &root.unrounded_layout;
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
    let mut last_flow_margin = 0.0;
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
        let layout = &node.unrounded_layout;
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
        let end =
            offset + layout.location.y - relative_shift + layout.size.height + layout.margin.bottom;
        if end.is_finite() {
            // A reverse column flex container's source order need not end
            // with its lowest item. Its item margins never collapse.
            if root.formatting_context == FormattingContext::Flex {
                content_end = content_end.max(end);
            }
            last_flow_end = Some(end);
            last_flow_margin = layout.margin.bottom;
        }
    }
    if let Some(end) = last_flow_end {
        // Without block-end edges the last child's block-end margin
        // collapses through the end of the root, unless the root is a
        // formatting context of its own.
        let end = if edges_end || encloses_floats {
            end
        } else {
            end - last_flow_margin
        };
        content_end = content_end.max(end);
    }
    // `max-height` caps the auto height, measured as `box-sizing` says;
    // `min-height` still wins over it.
    let content_height = content_end - (top + edges_top);
    let auto_end = top + edges_top + content_height + edges_bottom;
    min_end.max(auto_end.min(max_end))
}

/// Whether pagination may extend the auto height of `node_id` around its
/// committed content.
fn grows_with_paginated_content(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    let ordinary_block = node.formatting_context == FormattingContext::Block
        && matches!(
            node.display(),
            DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
        );
    let flex = node.formatting_context == FormattingContext::Flex;
    // A row-flex item can have a used stretch height even when its
    // auto-sized flex line must still grow around paginated content.
    let assigned_height = node.percent_basis_height.is_some()
        && (node.style.aspect_ratio.is_some()
            || node.taffy_position() == TaffyPosition::Absolute
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
        && (node.style.size.height.is_auto()
            || definite_content_height(document, node_id).is_none())
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
    document: &Document,
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
            let overflow = end
                - current_abs_y(document, node_id)
                - document.nodes[node_id].unrounded_layout.size.height;
            (overflow.is_finite() && overflow > 0.0).then_some((node_id, overflow))
        })
        .collect()
}

/// Extend auto-height block and flex boxes around their committed paginated
/// content, from descendants to ancestors. Page shifts may have been inherited
/// from a parallel flow rather than introduced at the box's own break.
/// Content that `overflow` says already overflowed a box before pagination
/// keeps overflowing it by the same amount.
fn update_paginated_block_heights(
    document: &mut Document,
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
        if height.is_finite() && height > document.nodes[node_id].unrounded_layout.size.height {
            document.nodes[node_id].unrounded_layout.size.height = height;
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
        let mut current = node_id;
        let mut visited = Vec::new();
        let context = loop {
            if let Some(cached) = self.cache[current] {
                break cached;
            }
            visited.push(current);
            let Some(parent) = document.parent_of(current) else {
                break None;
            };
            let computed = &cascade.computed[parent];
            if document.nodes[parent].formatting_context == FormattingContext::Flex
                && matches!(
                    computed.flex_direction,
                    FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
                )
            {
                break Some(parent);
            }
            current = parent;
        };
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
            raw_y: candidate.is_named.then_some(raw_y),
            context: candidate.named_flex_context,
            was_zero_height: candidate.is_named && height <= 0.001,
            had_display_none_descendant: candidate.is_named
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
pub fn relayout_text_for_width(document: &mut Document, cascade: &CascadeResult, max_advance: f32) {
    document.page_projection.clear();
    crate::layout::ifc::flow::rebreak_roots(document, cascade, max_advance);
}

/// Correct the static position of grid abspos items whose placement is `auto`.
/// Taffy handles explicit grid-area placement, but its static-position fallback
/// uses the border edge instead of the grid content box.
///
/// The margin box is aligned in the content box by the item's `justify-self`
/// and `align-self`, whose `auto` takes the container's `justify-items` and
/// `align-items` (CSS Grid 2 §10.2).
// spec: https://www.w3.org/TR/css-grid-2/#static-position
fn realign_grid_abspos_static_positions(document: &mut Document, cascade: &CascadeResult) {
    fn align_offset(value: AlignSelfValue, parent: SelfAlignmentValue, free: f32) -> f32 {
        let value = match value {
            AlignSelfValue::Auto => parent,
            AlignSelfValue::Value(value) => value,
            _ => SelfAlignmentValue::Start,
        };
        match value {
            SelfAlignmentValue::Center => free / 2.0,
            SelfAlignmentValue::End | SelfAlignmentValue::FlexEnd => free,
            _ => 0.0,
        }
    }
    for child_id in 0..document.nodes.len() {
        let child = &cascade.computed[child_id];
        if !matches!(
            document.nodes[child_id].position(),
            PositionValue::Absolute | PositionValue::Fixed
        ) || !matches!(child.top, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(child.right, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(child.bottom, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(child.left, ComputedLengthPercentageOrAuto::Auto)
        {
            continue;
        }
        #[cfg(all(test, debug_assertions))]
        crate::layout::test_support::GRID_PARENT_LOOKUP_STEPS
            .with(|steps| steps.set(steps.get() + 1));
        let Some(parent_id) = document.parent_of(child_id) else {
            continue;
        };
        if document.nodes[parent_id].formatting_context != FormattingContext::Grid
            || !matches!(child.grid_row_start, GridLineValue::Auto)
            || !matches!(child.grid_row_end, GridLineValue::Auto)
            || !matches!(child.grid_column_start, GridLineValue::Auto)
            || !matches!(child.grid_column_end, GridLineValue::Auto)
        {
            continue;
        }
        let parent = &cascade.computed[parent_id];
        let parent_size = document.nodes[parent_id].unrounded_layout.size;
        let border_left = parent.border.left.width().px();
        let border_right = parent.border.right.width().px();
        let border_top = parent.border.top.width().px();
        let border_bottom = parent.border.bottom.width().px();
        let parent_style = &document.nodes[parent_id].style;
        let padding_left =
            used_style_length_percentage(parent_style.padding.left, parent_size.width)
                .unwrap_or(0.0);
        let padding_right =
            used_style_length_percentage(parent_style.padding.right, parent_size.width)
                .unwrap_or(0.0);
        let padding_top = used_style_length_percentage(parent_style.padding.top, parent_size.width)
            .unwrap_or(0.0);
        let padding_bottom =
            used_style_length_percentage(parent_style.padding.bottom, parent_size.width)
                .unwrap_or(0.0);
        let content_width =
            (parent_size.width - border_left - border_right - padding_left - padding_right)
                .max(0.0);
        let content_height =
            (parent_size.height - border_top - border_bottom - padding_top - padding_bottom)
                .max(0.0);
        let child_layout = document.nodes[child_id].unrounded_layout;
        // With every inset `auto`, auto margins are zero (CSS 2.1 §10.3.7,
        // §10.6.4).
        let auto = |side: &ComputedLengthPercentageOrAuto, used: f32| {
            if matches!(side, ComputedLengthPercentageOrAuto::Auto) {
                0.0
            } else {
                used
            }
        };
        let margin = taffy::Rect {
            left: auto(&child.margin.left, child_layout.margin.left),
            right: auto(&child.margin.right, child_layout.margin.right),
            top: auto(&child.margin.top, child_layout.margin.top),
            bottom: auto(&child.margin.bottom, child_layout.margin.bottom),
        };
        let free_width = content_width - child_layout.size.width - margin.left - margin.right;
        let justify = align_offset(child.justify_self, parent.justify_items, free_width);
        // The inline start of a right-to-left container is its right edge.
        let justify = if parent.direction == Direction::Rtl {
            free_width - justify
        } else {
            justify
        };
        let x = border_left + padding_left + margin.left + justify;
        let y = border_top
            + padding_top
            + margin.top
            + align_offset(
                child.align_self,
                parent.align_items,
                content_height - child_layout.size.height - margin.top - margin.bottom,
            );
        let child_layout = &mut document.nodes[child_id].unrounded_layout;
        child_layout.location.x = x;
        child_layout.location.y = y;
    }
}

/// Lay out a Document on one A4 page (or the specified PageBox).
///
/// Text is laid out by the inline engine with the fonts the Document holds
/// ([`Document::set_font_collection`]). A Document that was given no fonts
/// takes the process-wide layer of the installed fonts
/// ([`crate::system_font_collection`]) on its first layout.
///
/// # In-place changes
/// - Bridge computed values to taffy::Style with `apply_computed_to_style`
///   and assign the paragraphs of the inline engine
/// - Create a definite layout-only page-area ICB and lay out the document
///   element and its ordinary descendants through Taffy
/// - Store each box's used geometry in Node.unrounded_layout
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
pub fn layout_single_page(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<(), LayoutError> {
    layout_single_page_with_table_projection(document, cascade, page_box, false, false)
}

/// The error a layout pass recorded on `document` while taffy ran: a
/// rejected table or an exhausted fragment budget.
fn take_layout_failure(document: &mut Document) -> Result<(), LayoutError> {
    if let Some(message) = document.table_layout_error.take() {
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
    document: &mut Document,
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
    // Fonts are only read when a document is laid out: one that was given
    // none takes the installed fonts here, not when it is created.
    if document.ifc.is_none() {
        document.set_font_collection(crate::fonts::system_font_collection());
    }

    // Step 0: layout_warnings re-entrance clear: this Vec is populated
    // over the course of a pass (bridges below, then the taffy compute step
    // via `set_unrounded_layout`) and drained near the end of this function,
    // but an early `?` return (Step 3) would otherwise leave a previous call's
    // leftover entries for the next call to inherit.
    document.layout_warnings.clear();
    document.table_layout_error = None;
    document.fragment_tree.clear();
    document.column_rules.clear();
    for node in &mut document.nodes {
        node.multicol_groups.clear();
        node.multicol_rows.clear();
    }
    document.fragmentation_stack.clear();

    // Step 1: ComputedValues → taffy::Style bridge (currently a no-op site).
    apply_computed_to_style(document, cascade)?;
    if paged {
        if let Some(breaks) = &document.clearance_breaks {
            let breaks = breaks.applied_to(cascade);
            clear_floats_at_forced_breaks(document, &breaks);
        } else {
            clear_floats_at_forced_breaks(document, cascade);
        }
    } else if !document.forced_break_clearance.is_empty()
        || !document.forced_break_line_clearance.is_empty()
    {
        document.forced_break_clearance.clear();
        document.forced_break_line_clearance.clear();
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

    // Step 2b: resolve `ch` lengths of box properties with the inline
    // engine's fonts before taffy sizes the boxes.
    prepare_ch_box_values_before_taffy(document, cascade);
    // Step 3: <body> lookup
    let body_id = find_body(document).ok_or_else(|| LayoutError::Internal {
        message: "no <body> element found (fragment parse not supported yet)".to_string(),
    })?;
    propagate_body_overflow_to_viewport(document, cascade, body_id);
    prepare_authored_block_widths(document, cascade, content_width);
    if let Some(state) = document.ifc.as_mut() {
        state.page_width = Some(content_width);
    }
    document.fixed_page_area = Some((content_width, content_height));
    let icb =
        super::initial_containing_block::prepare(document, cascade, content_width, content_height);
    let available_space = taffy::Size {
        width: AvailableSpace::Definite(content_width),
        height: AvailableSpace::Definite(content_height),
    };
    taffy::compute_root_layout(document, TaffyNodeId::from(icb), available_space);
    take_layout_failure(document)?;
    // Step 5a: post-layout corrections taffy does not make: the static
    // position of auto-placed grid abspos items, the containing block of
    // nested fixed boxes, and auto-height ancestors of floats.
    // Taffy places fixed boxes against their parents, not the page area,
    // their containing block.
    let page_area = taffy::Size {
        width: content_width,
        height: content_height,
    };
    document.fixed_page_area = Some((content_width, content_height));
    super::static_position::realign_inline_level_abspos_static_positions(document, cascade);
    let root = super::initial_containing_block::root_element(document).unwrap_or(body_id);
    place_fixed_boxes_in_page_area(document, cascade, root, page_area);
    // Laying those boxes out again can fail a table or exhaust the
    // fragment budget like the root layout above.
    take_layout_failure(document)?;
    // After the fixed boxes, whose relayout lays out the grids inside them
    // again with taffy's static positions.
    realign_grid_abspos_static_positions(document, cascade);
    // Again for the boxes inside fixed boxes that relayout put back; the
    // pass is idempotent, so the boxes placed above stay where they are.
    super::static_position::realign_inline_level_abspos_static_positions(document, cascade);
    propagate_float_bottoms_to_auto_height_ancestors(document, cascade);
    // Step 5b: check semantic parent-child geometry invariants and replace
    // any invalid subtree with the deterministic fallback (zero). Step 5
    // (`sanitize_taffy_layout` via `set_unrounded_layout`) guarantees only
    // finite values. This check therefore sits one layer above it; see the
    // `enforce_layout_invariants` documentation. Events enter the same
    // `document.layout_warnings` buffer as Steps 1 / 2; Step 6 drains it.
    enforce_layout_invariants(document, body_id);
    document.fragment_tree.finalize();
    document.column_rules = crate::column_rules::prepare(document, cascade)?;

    // Step 6: replay buffered LayoutWarn events.
    //
    // `document.layout_warnings` accumulated events from this function's own
    // bridge calls (Step 1 / Step 2, via `&mut document.layout_warnings`
    // passed directly), from `<Document as
    // taffy::LayoutPartialTree>::set_unrounded_layout` (invoked internally
    // by `compute_root_layout` just above, via `self` — see
    // `Document::layout_warnings`'s doc for why that trait-fixed signature
    // can only reach an owned buffer, not a live observer), and from Step 5b
    // (`enforce_layout_invariants`) just above, which
    // pushes into the same buffer directly since it already holds `&mut
    // Document`.
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
    for event in document.layout_warnings.drain(..) {
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
pub struct PageSlice {
    /// Zero-based page number.
    pub page_index: u32,
    /// Body-content y coordinate at which this page begins.
    pub content_origin_y: f32,
    /// Named page selected by the first class-A box on this page.
    pub page_name: Option<String>,
}

/// Controls page production and cancellation in the paginator.
///
/// The default matches [`raikiri_traits::RenderLimits::default`]. Pass `None`
/// to [`Self::new`] only when the caller intentionally wants unbounded pages.
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

    fn check_discovery_page_index(&self, page_index: u32) -> Result<(), LayoutError> {
        match self.check_page_index(page_index) {
            Err(LayoutError::PageLimitExceeded { .. }) if self.truncate_at_page_limit => {
                self.page_limit_reached.set(true);
                Ok(())
            }
            result => result,
        }
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
/// Resolve `ch` lengths on box properties to px in the taffy style, with
/// the fonts the inline engine lays the text out with.
pub(crate) fn prepare_ch_box_values_before_taffy(doc: &mut Document, cascade: &CascadeResult) {
    // `ch` is measured with the inline engine's fonts, the ones the text is
    // laid out with.
    let Some(fonts) = doc.ifc.as_ref().map(|state| state.fonts.clone()) else {
        return;
    };
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let cv = &cascade.computed[idx];
        let measure = |provenance: &Option<ChLengthProvenance>| {
            provenance.as_ref().map(|provenance| {
                let mut used = provenance
                    .with_advance(crate::layout::ifc::ch::ch_advance(&fonts, &provenance.font));
                used.px = if used.px.is_nan() {
                    0.0
                } else {
                    used.px.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
                };
                used
            })
        };
        // A box sized by its vertical logical sizes takes their `ch` lengths
        // on the physical axes the layout bridge gave them.
        let (width_ch, height_ch) = match crate::layout::bridge::vertical_logical_size(cascade, idx)
        {
            Some(size) => (&size.width_ch, &size.height_ch),
            None => (&cv.width_ch, &cv.height_ch),
        };
        let width = measure(width_ch);
        let height = measure(height_ch);
        let min_width = measure(&cv.min_width_ch);
        // Use the same logical block axis as the min/max bridge: a box on
        // vertical axes has its logical block minimum as its minimum width.
        let min_height_ch = if !crate::layout::bridge::uses_vertical_axes(cascade, idx)
            && cv.cssom_writing_mode.is_vertical()
            && matches!(cv.min_height, ComputedLengthPercentageOrAuto::Auto)
        {
            &cv.min_block_size_ch
        } else {
            &cv.min_height_ch
        };
        let min_height = measure(min_height_ch);
        let max_width = measure(&cv.max_width_ch);
        let max_height = measure(&cv.max_height_ch);
        let sides = |sides: &raikiri_style::property::Sides<Option<ChLengthProvenance>>| {
            [
                measure(&sides.top),
                measure(&sides.right),
                measure(&sides.bottom),
                measure(&sides.left),
            ]
        };
        let [padding_top, padding_right, padding_bottom, padding_left] = sides(&cv.padding_ch);
        let [margin_top, margin_right, margin_bottom, margin_left] = sides(&cv.margin_ch);
        let calc_values = &mut doc.calc_values;
        let style = &mut doc.nodes[idx].style;
        // A sum with a percentage resolves against the containing block in
        // taffy, through the same arena as other calc() values. Sizes and
        // padding cannot be negative; margins can.
        let mut used = |value: CalcLengthPercentage, non_negative: bool| {
            if value.percent == 0.0 {
                LengthPercentage::length(if non_negative {
                    value.px.max(0.0)
                } else {
                    value.px
                })
            } else {
                LengthPercentage::calc(crate::taffy_impl::push_calc(
                    calc_values,
                    value,
                    non_negative,
                ))
            }
        };
        if let Some(width) = width {
            style.size.width = used(width, true).into();
        }
        if let Some(height) = height {
            style.size.height = used(height, true).into();
        }
        if let Some(width) = min_width {
            style.min_size.width = used(width, true).into();
        }
        if let Some(height) = min_height {
            style.min_size.height = used(height, true).into();
        }
        if let Some(width) = max_width {
            style.max_size.width = used(width, true).into();
        }
        if let Some(height) = max_height {
            style.max_size.height = used(height, true).into();
        }
        if let Some(top) = padding_top {
            style.padding.top = used(top, true);
        }
        if let Some(right) = padding_right {
            style.padding.right = used(right, true);
        }
        if let Some(bottom) = padding_bottom {
            style.padding.bottom = used(bottom, true);
        }
        if let Some(left) = padding_left {
            style.padding.left = used(left, true);
        }
        if let Some(top) = margin_top {
            style.margin.top = used(top, false).into();
        }
        if let Some(right) = margin_right {
            style.margin.right = used(right, false).into();
        }
        if let Some(bottom) = margin_bottom {
            style.margin.bottom = used(bottom, false).into();
        }
        if let Some(left) = margin_left {
            style.margin.left = used(left, false).into();
        }
    }
}

/// Grow every auto-height ancestor of a left or right float to the float's
/// bottom edge, including the ancestor's bottom border.
///
/// Taffy does not include floated descendants in an auto-height containing
/// block's used height. This keeps following flow from moving upward after a
/// fragmented flex item with a float descendant.
pub(crate) fn propagate_float_bottoms_to_auto_height_ancestors(
    doc: &mut Document,
    cascade: &CascadeResult,
) {
    doc.in_flow_heights.clear();
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element
            || !doc.nodes[idx].is_in_document()
            || !matches!(
                cascade.computed[idx].float,
                FloatValue::Left | FloatValue::Right
            )
        {
            continue;
        }
        let mut child = idx;
        // A float in a paragraph sits in the paragraph root's coordinates and
        // adds no height to the inline elements that hold it (CSS 2.1 §10.6.1).
        while let Some(parent) = doc.layout_parent_of(child) {
            if matches!(
                cascade.computed[parent].height,
                ComputedLengthPercentageOrAuto::Auto
            ) {
                let child_bottom = doc.nodes[child].unrounded_layout.location.y
                    + doc.nodes[child].unrounded_layout.size.height
                    + cascade.computed[parent].border.bottom.width().px();
                let height = doc.nodes[parent].unrounded_layout.size.height;
                if child_bottom > height {
                    doc.in_flow_heights.entry(parent).or_insert(height);
                    doc.nodes[parent].unrounded_layout.size.height = child_bottom;
                }
            }
            child = parent;
        }
    }
}

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

/// A break moved up between a paragraph's lines: the first line that moves,
/// how far it and the lines after it move, and the page they move to.
type LineBreakMove = (usize, f32, u32);

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

/// Moves line `line` of the paragraph `root` and every line after it down by
/// `shift`, and grows the root's box to hold them.
fn shift_ifc_lines_from(document: &mut Document, root: usize, line: usize, shift: f32) {
    let node = document.table_layout_node_mut(root);
    if let Some(lines) = node.ifc.as_mut().and_then(|ifc| ifc.lines.as_mut()) {
        lines.shifts.push(line, shift);
        node.unrounded_layout.size.height += shift;
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
    current_abs_y(document, root) + text_first_line_offset(document, root, owned, 0.0)
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
        && let Some(first) = owned.lines.first()
    {
        let current = current_abs_y(document, root)
            + text_first_line_offset(document, root, owned, first.top);
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
    document
        .table_layout_node_mut(moved)
        .unrounded_layout
        .location
        .y += delta;
    if moved_root.is_none()
        && let Some(root) = text_ifc_root
    {
        follow_moved_ifc_text(document, root, node_id, delta, text_lines);
    }
}

/// Block offset of the first line of `owned` from the border-box top of its
/// paragraph `root`, after the paragraph's column ranges have moved it.
/// `fallback` is the line top used when no positioned first line is found:
/// the paragraph has no line records, or `owned` has no lines.
fn text_first_line_offset(
    document: &Document,
    root: usize,
    owned: &IfcTextLines,
    fallback: f32,
) -> f32 {
    let layout = document
        .ifc_layout_node(root)
        .expect("valid paragraph root")
        .unrounded_layout;
    let first_top = positioned_owned_text_line_bounds(document, owned)
        .and_then(|lines| lines.first().map(|line| line.0))
        .unwrap_or(fallback);
    layout.border.top + layout.padding.top + first_top
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

pub(crate) fn project_slices_with_control(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
    page_geometries: &[PageFragmentPageGeometry],
    control: &PageLayoutControl<'_>,
) -> Result<ProjectedSlices, LayoutError> {
    let fallback_geometry = resolve_page_fragment_geometry(cascade, page_box, 0);
    let mut ordered_slices: Vec<&PageSlice> = slices.iter().collect();
    ordered_slices.sort_by(|left, right| {
        left.page_index
            .cmp(&right.page_index)
            .then_with(|| left.content_origin_y.total_cmp(&right.content_origin_y))
    });
    let mut pages: Vec<PageFragment> = ordered_slices
        .iter()
        .map(|slice| {
            let geometry = page_geometries
                .iter()
                .find(|geometry| geometry.page_index == slice.page_index)
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

    let mut work = ProjectionWork::new(
        control,
        document.fragment_tree.work_limit(document.nodes.len()),
    );
    work.check()?;
    let mut text_roots = Vec::new();
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

    // Collect absolute post-pagination coordinates.  The arena index is the
    // stable NodeId projection used by `raikiri_traits::Dom`; sorting by it
    // reproduces fulgur's deterministic BTreeMap iteration order regardless of
    // traversal implementation details.
    #[derive(Clone)]
    struct PageFragmentSource {
        node_id: NodeId,
        node_kind: NodeKind,
        is_img: bool,
        abs_x: f32,
        abs_y: f32,
        width: f32,
        height: f32,
        content_insets: Option<PageFragmentInsets>,
        line_metrics: Option<Vec<(f32, f32)>>,
        /// The lines of `line_metrics` stack along x, from the block-start
        /// edge of their paragraph.
        lines_along_x: bool,
        line_start_offset: u32,
        is_repeat: bool,
        fragmentainer: u32,
        fragment_clip: Option<PaintRect>,
        overflow_chain: Option<usize>,
        own_overflow_source: Option<usize>,
    }

    impl PageFragmentSource {
        /// This source with its geometry in the flow space of `axis`.
        fn in_flow_space(&self, axis: FlowAxis) -> Self {
            let (abs_x, abs_y, width, height) =
                axis.to_flow((self.abs_x, self.abs_y, self.width, self.height));
            let along_flow = self.lines_along_x == matches!(axis, FlowAxis::X { .. });
            Self {
                abs_x,
                abs_y,
                width,
                height,
                content_insets: self.content_insets.map(|insets| axis.sides_to_flow(insets)),
                // Lines that stack across the flow all share the source's
                // extent along it.
                line_metrics: self.line_metrics.as_ref().map(|metrics| {
                    if along_flow {
                        metrics.clone()
                    } else {
                        vec![(0.0, height); metrics.len()]
                    }
                }),
                fragment_clip: self.fragment_clip.map(|clip| {
                    let (x, y, width, height) =
                        axis.to_flow((clip.x, clip.y, clip.width, clip.height));
                    PaintRect::new(x, y, width, height)
                }),
                ..self.clone()
            }
        }
    }

    let mut nodes = Vec::new();
    let mut ifc_origin: HashMap<usize, (f32, f32)> = HashMap::new();
    // The content width of each paragraph whose lines stack along x, and
    // whether they stack from right to left.
    let mut ifc_vertical: HashMap<usize, (f32, bool)> = HashMap::new();
    // Each inline element's rectangle is the union of its pieces after
    // pagination moves their lines.
    let mut ifc_piece_bounds: HashMap<usize, HashMap<usize, Vec<(BoxRect, u32)>>> = HashMap::new();
    let mut fragmented_inline_nodes = HashSet::new();
    let mut ifc_text_lines = HashMap::new();
    let mut paragraph_cache = HashMap::<usize, ParagraphProjection>::new();
    let fragments = document.layout_fragments();
    let mut fragments_by_parent_and_node = HashMap::<(Option<usize>, usize), Vec<usize>>::new();
    let mut fragmented_nodes = HashSet::new();
    let mut fragment_counts_by_node = HashMap::<usize, usize>::new();
    for (index, fragment) in fragments.iter().enumerate() {
        fragments_by_parent_and_node
            .entry((fragment.parent, fragment.node_id))
            .or_default()
            .push(index);
        fragmented_nodes.insert(fragment.node_id);
        *fragment_counts_by_node.entry(fragment.node_id).or_default() += 1;
    }
    let mut repeated_placement_nodes = HashSet::new();
    let mut pending: Vec<usize> = fragment_counts_by_node
        .into_iter()
        .filter_map(|(node, count)| (count > 1).then_some(node))
        .collect();
    while let Some(node) = pending.pop() {
        if repeated_placement_nodes.insert(node) {
            pending.extend(document.nodes[node].children.iter().copied());
        }
    }
    let mut placement_overflow_clips = Vec::new();
    struct ProjectionVisit {
        node: usize,
        parent_origin: (f32, f32),
        repeat: bool,
        fragment: Option<usize>,
        fragment_origin: (f32, f32),
        is_fragment_visit: bool,
        fragment_clip: Option<PaintRect>,
        overflow_chain: Option<usize>,
        paragraph_column: Option<(usize, usize)>,
    }
    let mut stack = vec![ProjectionVisit {
        node: super::initial_containing_block::root_element(document).unwrap_or(body_id),
        parent_origin: (0.0, 0.0),
        repeat: false,
        fragment: None,
        fragment_origin: (0.0, 0.0),
        is_fragment_visit: false,
        fragment_clip: None,
        overflow_chain: None,
        paragraph_column: None,
    }];
    while let Some(visit) = stack.pop() {
        work.check()?;
        let node_id = visit.node;
        if repeated_placement_nodes.contains(&node_id) {
            work.charge(1)?;
        }
        let (parent_abs_x, parent_abs_y) = visit.parent_origin;
        let inherited_repeat = visit.repeat;
        let Some(node) = document.get_node(node_id) else {
            continue; // cov:ignore: document-owned child links are valid by construction.
        };
        if !node.is_in_document() || node.is_non_rendered_html_element() || node.is_display_none() {
            continue;
        }
        let layout = node.unrounded_layout;
        // A nested column container has a placement record and a retained
        // container root for the same source node. Enter that root before
        // visiting the node's DOM children, whose records belong to it.
        if !visit.is_fragment_visit
            || fragments_by_parent_and_node.contains_key(&(visit.fragment, node_id))
        {
            if let Some(indices) = fragments_by_parent_and_node.get(&(visit.fragment, node_id)) {
                for &index in indices.iter().rev() {
                    let origin = fragments[index].paint_origin(
                        visit.parent_origin,
                        (layout.location.x, layout.location.y),
                        visit.fragment_origin,
                    );
                    let fragment_clip = fragments[index]
                        .fragmentainer_clip
                        .map(|clip| {
                            let clip = PaintRect::new(
                                origin.0 + clip.x - fragments[index].rect.x,
                                origin.1 + clip.y - fragments[index].rect.y,
                                clip.width,
                                clip.height,
                            );
                            if let Some(parent) = visit.fragment_clip {
                                let x = clip.x.max(parent.x);
                                let y = clip.y.max(parent.y);
                                PaintRect::new(
                                    x,
                                    y,
                                    ((clip.x + clip.width).min(parent.x + parent.width) - x)
                                        .max(0.0),
                                    ((clip.y + clip.height).min(parent.y + parent.height) - y)
                                        .max(0.0),
                                )
                            } else {
                                clip
                            }
                        })
                        .or(visit.fragment_clip);
                    stack.push(ProjectionVisit {
                        node: node_id,
                        parent_origin: (origin.0 - layout.location.x, origin.1 - layout.location.y),
                        repeat: inherited_repeat,
                        fragment: Some(index),
                        fragment_origin: origin,
                        is_fragment_visit: true,
                        fragment_clip,
                        overflow_chain: visit.overflow_chain,
                        paragraph_column: visit.paragraph_column,
                    });
                }
                continue;
            }
            if fragmented_nodes.contains(&node_id) {
                continue;
            }
        }
        let abs_x = parent_abs_x + layout.location.x;
        let abs_y = parent_abs_y + layout.location.y;
        let own_fragment = visit
            .is_fragment_visit
            .then(|| fragments[visit.fragment.unwrap()]);
        let width = finite_nonnegative(
            own_fragment.map_or(layout.size.width, |fragment| fragment.rect.width),
        );
        let height = finite_nonnegative(
            own_fragment.map_or(layout.size.height, |fragment| fragment.rect.height),
        );
        // A fixed-position subtree is painted in every committed page. The
        // existing layout pass already computes one viewport-relative box;
        // preserve that geometry and mark the records as complete repeats
        // instead of clipping them as in-flow content.
        let is_repeat = inherited_repeat || super::fixed::is_page_fixed(document, cascade, node_id);
        let include = match node.kind() {
            NodeKind::Text => node
                .text_content()
                .is_some_and(|text| !text.trim().is_empty()),
            NodeKind::Element => true,
            _ => false, // cov:ignore: non-rendered node kinds are filtered by the document invariant.
        };
        let own_overflow_source = if repeated_placement_nodes.contains(&node_id)
            && crate::paint_rules::clips_element_overflow(document, cascade, node_id)
        {
            let border_box = PaintRect::new(
                abs_x + node.table_grid_box.map_or(0.0, |rect| rect.x),
                abs_y + node.table_grid_box.map_or(0.0, |rect| rect.y),
                node.table_grid_box.map_or(width, |rect| rect.width),
                node.table_grid_box.map_or(height, |rect| rect.height),
            );
            let border = layout.border;
            let index = placement_overflow_clips.len();
            placement_overflow_clips.push(PlacementOverflowClipSource {
                node: NodeId::new(node_id as u64),
                parent: visit.overflow_chain,
                source: OverflowClipSource {
                    border_box,
                    geometry: crate::paint_rules::OverflowClipGeometry::new(
                        &cascade.computed[node_id],
                        border_box,
                        PaintInsets::new(border.top, border.right, border.bottom, border.left),
                    ),
                    is_repeat,
                },
            });
            Some(index)
        } else {
            None
        };
        let content_overflow_chain = own_overflow_source.or(visit.overflow_chain);
        if node.display() == DisplayValue::ListItem
            && !(crate::generated_content::inside_marker_in_flow(cascade, node_id)
                && node.is_ifc_root())
            && own_fragment.is_none_or(|fragment| fragment.fragment_index == 0)
        {
            text_roots.push(ProjectedTextRoot {
                node: crate::generated_content::generated_node_id(
                    node_id,
                    raikiri_style::PseudoElem::Marker,
                ),
                x: abs_x,
                y: abs_y,
                width: 0.0,
                lines_along_x: false,
                is_repeat,
                fragmentainer: None,
                fragment_clip: visit.fragment_clip,
                overflow_chain: visit.overflow_chain,
            });
        }
        for (root_id, root_node) in std::iter::once((node_id, node))
            .filter(|(_, node)| node.is_ifc_root())
            .chain(document.anonymous_table_cells(node_id))
        {
            let root_layout = root_node.unrounded_layout;
            let (root_x, root_y) = if root_id == node_id {
                (abs_x, abs_y)
            } else {
                (
                    abs_x + root_layout.location.x,
                    abs_y + root_layout.location.y,
                )
            };
            // The content-box origin of a paragraph laid out by the inline
            // engine: its text nodes have no layout of their own, and their
            // lines are measured from here. A root is visited before its text.
            let origin = (
                root_x + root_layout.border.left + root_layout.padding.left,
                root_y + root_layout.border.top + root_layout.padding.top,
            );
            ifc_origin.insert(root_id, origin);
            if let Some(mode) = root_node.ifc_writing_mode()
                && mode.is_vertical()
            {
                ifc_vertical.insert(
                    root_id,
                    (
                        root_layout.content_box_width(),
                        matches!(
                            mode,
                            shodo::geometry::WritingMode::VerticalRl
                                | shodo::geometry::WritingMode::SidewaysRl
                        ),
                    ),
                );
            }
            if origin.0.is_finite() && origin.1.is_finite() {
                text_roots.push(ProjectedTextRoot {
                    node: root_id,
                    x: origin.0,
                    y: origin.1,
                    width: root_layout.content_box_width(),
                    lines_along_x: ifc_vertical.contains_key(&root_id),
                    is_repeat,
                    fragmentainer: visit.fragment.map(|index| fragments[index].fragmentainer),
                    fragment_clip: visit.fragment_clip,
                    overflow_chain: content_overflow_chain,
                });
            }
            let column = visit.fragment.map(|index| fragments[index].fragmentainer);
            if let std::collections::hash_map::Entry::Vacant(entry) = paragraph_cache.entry(root_id)
            {
                entry.insert(ParagraphProjection::prepare(
                    document,
                    cascade,
                    root_id,
                    column.unwrap_or(0),
                    &mut work,
                )?);
            }
            let prepared = &paragraph_cache[&root_id];
            let selected = if prepared.multicol
                && let Some(column) = column
            {
                if let Some(data) = prepared.columns.get(&column) {
                    work.charge(data.record_count)?;
                    data.clone()
                } else {
                    Default::default()
                }
            } else {
                if repeated_placement_nodes.contains(&root_id) {
                    work.charge(prepared.combined.record_count)?;
                }
                let mut data = prepared.combined.clone();
                if !prepared.multicol {
                    for pieces in data.bounds.values_mut() {
                        for (_, number) in pieces {
                            *number = column.unwrap_or(0) as u32;
                        }
                    }
                    for groups in data.owners.values_mut() {
                        for (_, _, number, _) in groups {
                            *number = column.unwrap_or(0) as u32;
                        }
                    }
                }
                data
            };
            fragmented_inline_nodes.extend(selected.fragmented);
            ifc_piece_bounds.insert(root_id, selected.bounds);
            ifc_text_lines.extend(selected.owners);
        }
        // An inline element of an inline engine paragraph is where its pieces
        // are on the lines; its recorded location is relative to its nearest
        // inline ancestor and does not follow lines that pagination moved.
        let (abs_x, abs_y, width, height) = if node.kind() == NodeKind::Element
            && node.in_ifc_subtree()
            && let Some(root) = document.ifc_root_of(node_id)
            && let (Some(&(root_x, root_y)), Some(bounds_by_node)) =
                (ifc_origin.get(&root), ifc_piece_bounds.get(&root))
            && let Some((rect, _)) = bounds_by_node
                .get(&node_id)
                .and_then(|pieces| pieces.first())
        {
            (
                root_x + rect.x,
                root_y + rect.y,
                finite_nonnegative(rect.width),
                finite_nonnegative(rect.height),
            )
        } else {
            (abs_x, abs_y, width, height)
        };
        if include && abs_x.is_finite() && abs_y.is_finite() {
            let ifc_lines = (node.kind() == NodeKind::Text)
                .then(|| {
                    ifc_text_lines.remove(&node_id).or_else(|| {
                        document
                            .ifc_root_of(node_id)
                            .and_then(|root| paragraph_cache.get(&root))
                            .filter(|prepared| prepared.combined.owners.contains_key(&node_id))
                            .map(|_| Vec::new())
                    })
                })
                .flatten();
            let source = PageFragmentSource {
                node_id: NodeId::new(node_id as u64),
                node_kind: node.kind(),
                is_img: node.tag_name() == Some("img"),
                abs_x: abs_x + node.table_grid_box.map_or(0.0, |rect| rect.x),
                abs_y: abs_y + node.table_grid_box.map_or(0.0, |rect| rect.y),
                width: node.table_grid_box.map_or(width, |rect| rect.width),
                height: node.table_grid_box.map_or(height, |rect| rect.height),
                content_insets: (node.kind() == NodeKind::Element
                    && !fragmented_inline_nodes.contains(&node_id))
                .then(|| {
                    PageFragmentInsets::new(
                        layout.border.top + layout.padding.top,
                        layout.border.right + layout.padding.right,
                        layout.border.bottom + layout.padding.bottom,
                        layout.border.left + layout.padding.left,
                    )
                }),
                line_metrics: (node.kind() == NodeKind::Text).then(Vec::new),
                lines_along_x: false,
                line_start_offset: 0,
                is_repeat,
                fragmentainer: visit
                    .fragment
                    .map_or(0, |index| fragments[index].fragmentainer as u32),
                fragment_clip: visit.fragment_clip,
                overflow_chain: visit.overflow_chain,
                own_overflow_source,
            };
            let inline_pieces = (node.kind() == NodeKind::Element && node.in_ifc_subtree())
                .then(|| document.ifc_root_of(node_id))
                .flatten()
                .and_then(|root| {
                    Some((
                        ifc_origin.get(&root)?,
                        ifc_piece_bounds.get(&root)?.get(&node_id),
                    ))
                });
            if let Some(groups) = ifc_lines {
                for (offset_x, start, column, owned) in groups {
                    let (root_x, root_y) = ifc_origin
                        .get(&owned.root)
                        .copied()
                        .unwrap_or((abs_x, abs_y));
                    let first_top = owned.lines.first().map_or(0.0, |line| line.top);
                    let last_bottom = owned.lines.last().map_or(0.0, |line| line.bottom);
                    let block_extent = finite_nonnegative(last_bottom - first_top);
                    let inline_extent = finite_nonnegative(owned.width);
                    // Lines are measured along the paragraph's block axis,
                    // from its block-start content edge.
                    let (abs_x, abs_y, width, height) = match ifc_vertical.get(&owned.root) {
                        Some(&(content_width, true)) => (
                            root_x + content_width - last_bottom,
                            root_y + offset_x,
                            block_extent,
                            inline_extent,
                        ),
                        Some(_) => (
                            root_x + first_top,
                            root_y + offset_x,
                            block_extent,
                            inline_extent,
                        ),
                        None => (
                            root_x + offset_x,
                            root_y + first_top,
                            inline_extent,
                            block_extent,
                        ),
                    };
                    nodes.push(PageFragmentSource {
                        abs_x,
                        abs_y,
                        width,
                        height,
                        lines_along_x: ifc_vertical.contains_key(&owned.root),
                        line_metrics: Some(
                            owned
                                .lines
                                .iter()
                                .map(|line| (line.top - first_top, line.bottom - first_top))
                                .collect(),
                        ),
                        line_start_offset: start as u32,
                        fragmentainer: column,
                        ..source.clone()
                    });
                }
            } else if let Some((&(root_x, root_y), pieces)) = inline_pieces {
                for (rect, column) in pieces.into_iter().flatten() {
                    nodes.push(PageFragmentSource {
                        abs_x: root_x + rect.x,
                        abs_y: root_y + rect.y,
                        width: finite_nonnegative(rect.width),
                        height: finite_nonnegative(rect.height),
                        fragmentainer: *column,
                        ..source.clone()
                    });
                }
            } else {
                nodes.push(source);
            }
        } // cov:ignore: layout sanitization normally keeps source coordinates finite.

        if node.kind() == NodeKind::Element {
            let paragraph_column = if node.is_ifc_root() {
                visit.fragment.and_then(|index| {
                    paragraph_cache
                        .get(&node_id)
                        .filter(|prepared| prepared.children.is_some())
                        .map(|_| (node_id, fragments[index].fragmentainer))
                })
            } else {
                visit.paragraph_column
            };
            let children = paragraph_column.map_or(node.children.as_slice(), |(root, column)| {
                paragraph_cache[&root]
                    .children
                    .as_ref()
                    .unwrap()
                    .get(&(node_id, column))
                    .map_or(&[], Vec::as_slice)
            });
            // The children of an inline element of an inline engine paragraph
            // are located from the paragraph's root, not from the element.
            let (base_x, base_y) = if document.contributes_layout_offset(node_id) {
                (abs_x, abs_y)
            } else {
                (parent_abs_x, parent_abs_y)
            };
            for &child_id in children.iter().rev() {
                stack.push(ProjectionVisit {
                    node: child_id,
                    parent_origin: (base_x, base_y),
                    repeat: is_repeat,
                    fragment: visit.fragment,
                    fragment_origin: visit.fragment_origin,
                    is_fragment_visit: false,
                    fragment_clip: visit.fragment_clip,
                    overflow_chain: content_overflow_chain,
                    paragraph_column,
                });
            }
        }
    }
    nodes.sort_by_key(|node| node.node_id);
    // A relayout resumed at a page start lays the content before it out
    // without the elements wholly before it, so a box crossing the start
    // begins lower than in the earlier layout. It keeps the top the earlier
    // layout gave it; its end, after the start, is the same in both. A box
    // with no border or padding above the start begins at the start itself.
    if let Some(earlier) = &document.earlier_fragments
        && let Some((page_start, _)) = pages.first().and_then(|page| page.flow_range)
    {
        for source in &mut nodes {
            if let Some(crossing) = earlier.get(&source.node_id)
                && !source.is_repeat
                && source.abs_y <= page_start + 0.01
            {
                let bottom = source.abs_y + source.height;
                source.abs_y = page_start + crossing.top;
                source.height = (bottom - source.abs_y).max(0.0);
            }
        }
    }

    // The pages with a flow range, in page order, and whether both ends of
    // their ranges never decrease, so that a source's pages can be found by
    // binary search instead of testing every page.
    let flow_slots: Vec<(usize, f32, f32)> = pages
        .iter()
        .enumerate()
        .filter_map(|(page_slot, page)| page.flow_range.map(|(start, end)| (page_slot, start, end)))
        .collect();
    let flow_slots_ordered = flow_slots
        .windows(2)
        .all(|pair| pair[0].1 <= pair[1].1 && pair[0].2 <= pair[1].2);
    for source in &nodes {
        work.check()?;
        let source = &source.in_flow_space(flow_axis);
        let header_placements = document
            .table_objects
            .headers
            .placements_of(source.node_id.0 as usize);
        let is_table_header = header_placements.is_some();
        let kind = match source.node_kind {
            NodeKind::Text => PageFragmentKind::Text,
            NodeKind::Element if source.is_img => PageFragmentKind::Replaced,
            _ => PageFragmentKind::Box,
        };
        let mut placements = Vec::new();
        let repeat_line_range = source.line_metrics.as_deref().and_then(|metrics| {
            (!metrics.is_empty()).then(|| {
                PageFragmentLineRange::new(0, u32::try_from(metrics.len()).unwrap_or(u32::MAX))
            })
        });
        if let Some(header_placements) = header_placements {
            // Repeated header boxes visit only their own pages, and every copy
            // is charged so header size times page count stays bounded.
            for (page_index, origin, shift) in header_placements {
                let start = pages.partition_point(|page| page.page_index < page_index);
                for (page_slot, page) in pages.iter().enumerate().skip(start) {
                    if page.page_index != page_index {
                        break;
                    }
                    let Some((page_start, _)) = page
                        .flow_range
                        .filter(|&(page_start, _)| (page_start - origin).abs() <= 0.001)
                    else {
                        continue; // cov:ignore: layout schedules placements at the slices' own finite page origins.
                    };
                    work.charge(1)?;
                    let y = source.abs_y + shift - page_start;
                    placements.push((page_slot, y, source.height, repeat_line_range, y));
                }
            }
        }
        // A repeated source lands on every page. Any other source only
        // intersects the pages whose flow range overlaps it, which form one
        // run of `flow_slots` when the ranges are ordered.
        let candidates = if is_table_header {
            &flow_slots[..0]
        } else if source.is_repeat || !flow_slots_ordered {
            &flow_slots[..]
        } else if source.height > 0.0 {
            let bottom = source.abs_y + source.height;
            let first = flow_slots.partition_point(|&(_, _, end)| end <= source.abs_y);
            let last = first + flow_slots[first..].partition_point(|&(_, start, _)| start < bottom);
            &flow_slots[first..last]
        } else {
            let first = flow_slots.partition_point(|&(_, _, end)| end < source.abs_y);
            let last =
                first + flow_slots[first..].partition_point(|&(_, start, _)| start <= source.abs_y);
            &flow_slots[first..last]
        };
        for &(page_slot, page_start, page_end) in candidates {
            if source.is_repeat {
                placements.push((
                    page_slot,
                    source.abs_y,
                    source.height,
                    repeat_line_range,
                    source.abs_y,
                ));
                continue;
            }
            let (intersects, fragment_y, fragment_height) = if source.height > 0.0 {
                match flow_page_overlap(source.abs_y, source.height, page_start, page_end) {
                    Some((y, height)) => (true, y, height),
                    None => (false, 0.0, 0.0),
                }
            } else {
                (
                    source.abs_y >= page_start && source.abs_y <= page_end,
                    (source.abs_y - page_start).max(0.0),
                    0.0,
                )
            };
            if intersects {
                let line_range = source.line_metrics.as_deref().and_then(|metrics| {
                    line_range_for_page(metrics, source.abs_y, page_start, page_end)
                });
                placements.push((
                    page_slot,
                    fragment_y,
                    fragment_height,
                    line_range,
                    source.abs_y - page_start,
                ));
            }
        }
        let fragment_count = placements.len() as u32;
        for (fragment_index, (page_slot, y, fragment_height, line_range, box_y)) in
            placements.into_iter().enumerate()
        {
            let Some(page) = pages.get_mut(page_slot) else {
                continue; // cov:ignore: placements are indexed from the same slices used to build pages.
            };
            let page_axis = page.flow_axis;
            let to_page = |rect: (f32, f32, f32, f32)| {
                let (x, y, width, height) = page_axis.to_layout(rect);
                PageFragmentRect::new(x, y, width, height)
            };
            let rect = to_page((source.abs_x, y, source.width, fragment_height));
            // A page cuts boxes only along the flow; across it, the whole box
            // is the placed one.
            let (item_box_y, item_box_height) = match flow_axis {
                FlowAxis::Y => (box_y, source.height),
                FlowAxis::X { .. } => (rect.y, rect.height),
            };
            let mut item = PageFragmentItem::new(
                source.node_id,
                rect,
                kind,
                fragment_index as u32,
                fragment_count,
                source.is_repeat || is_table_header,
            )
            .with_page_index(page.page_index)
            .with_box_extent(item_box_y, item_box_height)
            .with_flow_box_y(box_y)
            .with_content_rect(source.content_insets.map(|insets| {
                to_page((
                    source.abs_x + insets.left,
                    box_y + insets.top,
                    (source.width - insets.left - insets.right).max(0.0),
                    (source.height - insets.top - insets.bottom).max(0.0),
                ))
            }));
            item.is_table_header_repeat = is_table_header;
            item.fragmentainer = source.fragmentainer;
            item.overflow_chain = source.overflow_chain;
            item.own_overflow_source = source.own_overflow_source;
            item.fragment_clip = source.fragment_clip.map(|clip| {
                to_page((
                    clip.x,
                    clip.y + box_y - source.abs_y,
                    clip.width,
                    clip.height,
                ))
            });
            page.items.push(match line_range {
                Some(range) => item.with_line_range(PageFragmentLineRange::new(
                    range.start.saturating_add(source.line_start_offset),
                    range.end.saturating_add(source.line_start_offset),
                )),
                None => item,
            });
        }
    }

    // A relayout resumed at a page start continues the fragments the boxes
    // crossing that start had before it. The body begins on the relayout's
    // first page, which the relayout drops; when it did not cross the start
    // in the earlier layout, it has no fragment after it.
    if let Some(earlier) = &document.earlier_fragments
        && let Some(first) = pages.first_mut()
    {
        let body = NodeId::new(body_id as u64);
        first.items.retain(|item| {
            item.node_id != body || item.flow_box_y >= -0.01 || earlier.contains_key(&body)
        });
    }
    let earlier = document.earlier_fragments.as_ref();
    let before = |node: &NodeId| {
        earlier
            .and_then(|earlier| earlier.get(node))
            .map(|crossing| crossing.earlier_fragments)
    };

    // A source can have several column pieces on the same physical page.
    // Count all committed pieces together so edge ownership remains stable.
    let mut fragment_counts = HashMap::<NodeId, u32>::new();
    for item in pages.iter().flat_map(|page| &page.items) {
        *fragment_counts.entry(item.node_id).or_insert_with(|| {
            before(&item.node_id).unwrap_or(0)
                + document
                    .later_fragments
                    .get(&item.node_id)
                    .copied()
                    .unwrap_or(0)
        }) += 1;
    }
    let mut fragment_indices = HashMap::<NodeId, u32>::new();
    for item in pages.iter_mut().flat_map(|page| &mut page.items) {
        let index = fragment_indices
            .entry(item.node_id)
            .or_insert_with(|| before(&item.node_id).unwrap_or(0));
        item.fragment_index = *index;
        item.fragment_count = fragment_counts[&item.node_id];
        *index += 1;
    }

    // Retain each clipping source once. Page-local clips are resolved on access,
    // so a deep open-axis chain cannot multiply storage by the page count.
    let clip_sources = nodes
        .iter()
        .filter_map(|source| {
            let id = source.node_id.0 as usize;
            if repeated_placement_nodes.contains(&id) {
                return None;
            }
            if !crate::paint_rules::clips_element_overflow(document, cascade, id) {
                return None;
            }
            let border = document.nodes[id].unrounded_layout.border;
            let border_box =
                PaintRect::new(source.abs_x, source.abs_y, source.width, source.height);
            Some((
                source.node_id,
                OverflowClipSource {
                    border_box,
                    geometry: crate::paint_rules::OverflowClipGeometry::new(
                        &cascade.computed[id],
                        border_box,
                        PaintInsets::new(border.top, border.right, border.bottom, border.left),
                    ),
                    is_repeat: source.is_repeat,
                },
            ))
        })
        .collect();

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
fn clear_floats_at_forced_breaks(document: &mut Document, cascade: &CascadeResult) {
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
                        && !is_monolithic_for_pagination(document, cascade, child))
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
                && is_static_position_absolute(document, child)
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
                         trailing: &mut HashMap<usize, Option<String>>,
                         id: usize,
                         inherited: Option<String>| {
        let mut chain = Vec::new();
        let mut id = id;
        let mut name = loop {
            if let Some(name) = trailing.get(&id) {
                break name.clone();
            }
            chain.push(id);
            if is_monolithic_for_pagination(document, cascade, id) {
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
            trailing.insert(id, name.clone());
        }
        name.or(inherited)
    };
    let mut trailing = HashMap::new();
    // Whether a box with the page value `name` after content that ended with
    // `previous` starts a new page.
    let changes_page = |previous: &Option<Option<String>>, name: &Option<String>| {
        previous.as_ref().is_some_and(|previous| previous != name)
    };
    // Walk the boxes with the page value each inherits. Floats take their
    // parent's page, and their insides are not walked.
    let mut cleared = Vec::new();
    let mut cleared_lines = Vec::new();
    let mut pending = vec![(0_usize, None::<String>)];
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
        .then(|| inherited.clone());
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
                previous_page_name = Some(inherited.clone());
                continue;
            };
            if is_static_position_absolute(document, child) {
                // An absolutely positioned box left at its static position
                // keeps the named pages of its in-flow content in the page
                // flow, as pagination does, while its own `page` value and
                // its unnamed content take no part. Its content is in a
                // formatting context of its own, so only the boxes after it
                // start below the earlier floats. A monolithic box passes no
                // page name on.
                let last = (!is_monolithic_for_pagination(document, cascade, child))
                    .then(|| {
                        child_boxes(document, child, false)
                            .into_iter()
                            .rev()
                            .find(|item| item.is_none_or(|item| !is_float(document, item)))
                    })
                    .flatten()
                    .flatten();
                if let Some(last) = last {
                    let name = end_page_name(document, &mut trailing, last, inherited.clone());
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
                previous_page_name = Some(inherited.clone());
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
                    document.nodes[child].style.clear = taffy::Clear::Both;
                    cleared.push(child);
                    float_after_break = true;
                }
                if forced_after {
                    after_forced_break = true;
                    float_after_break = false;
                }
                continue;
            }
            float_after_break = false;
            last_block = Some(child);
            let start_page_name =
                match propagated_start_page_name(document, cascade, child, inherited.as_deref()) {
                    (true, name) => name,
                    (false, _) => inherited.clone(),
                };
            let named_break = changes_page(&previous_page_name, &start_page_name);
            let forced = after_forced_break
                || named_break
                || page_break_is_forced(document, child, cascade.computed[child].break_before);
            // A break after the last box inside this one is carried out to
            // the next box, as the paginator does.
            after_forced_break = page_edge_constraint(document, cascade, child, false).0;
            previous_page_name = Some(end_page_name(
                document,
                &mut trailing,
                child,
                inherited.clone(),
            ));
            let own_page_name = if page_applies_to_box(document, child) {
                selected_page_name(cascade, child).or_else(|| inherited.clone())
            } else {
                inherited.clone()
            };
            // Pagination does not break inside a monolithic box.
            if !is_monolithic_for_pagination(document, cascade, child) {
                pending.push((child, own_page_name));
            }
            // Floats on either side belong to the earlier page, so a
            // one-sided clear is widened too.
            if forced {
                document.nodes[child].style.clear = taffy::Clear::Both;
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
    if cleared != document.forced_break_clearance
        || cleared_lines != document.forced_break_line_clearance
    {
        document.forced_break_clearance = cleared;
        document.forced_break_line_clearance = cleared_lines;
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
    let mut ancestor = document.layout_parent_of(node_id);
    while let Some(id) = ancestor {
        if document.nodes[id].multicol.is_some() {
            return false;
        }
        ancestor = document.layout_parent_of(id);
    }
    true
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

pub(crate) fn selected_page_name(cascade: &CascadeResult, node_id: usize) -> Option<String> {
    match cascade.page_values.get(node_id) {
        Some(raikiri_style::property::PageValue::Named(name)) => Some(name.to_string()),
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
pub fn layout_pages(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_steps(document, cascade, page_box, &[])
}

/// Like [`layout_pages`], but first use `resolver` to resolve the intrinsic
/// size of `<img>` elements. The result is stored in the same `Document`,
/// so normal layout after pagination and the paint pixel source can refer
/// to the same image.
pub fn layout_pages_with_resolver(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_resolver_and_base_url(document, cascade, page_box, resolver, None)
}

/// [`layout_pages_with_resolver`] with relative image URLs resolved against a
/// document base URL.
pub fn layout_pages_with_resolver_and_base_url(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_resolver_and_base_url_and_control(
        document,
        cascade,
        page_box,
        resolver,
        base_url,
        &PageLayoutControl::default(),
    )
}

/// [`layout_pages_with_resolver_and_base_url`] with an explicit page limit
/// and cancellation check.
pub fn layout_pages_with_resolver_and_base_url_and_control(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
    control: &PageLayoutControl<'_>,
) -> Result<Vec<PageSlice>, LayoutError> {
    control.check_page_index(0)?;
    super::validate_layout_depth(document)?;
    document.page_projection.clear();
    document.mark_in_document_flags();
    resolve_document_images(document, cascade, resolver, base_url)?;
    layout_pages_with_page_geometry_and_control(document, cascade, page_box, &[], &[], control)
}

/// Like [`layout_pages_with_page_geometry`], but first use `resolver` to
/// resolve the intrinsic sizes of `<img>` elements.
pub fn layout_pages_with_page_geometry_and_resolver(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    resolver: &dyn ReplacedResolver,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_geometry_and_resolver_and_base_url(
        document,
        cascade,
        page_box,
        page_steps,
        page_widths,
        resolver,
        None,
    )
}

/// [`layout_pages_with_page_geometry_and_resolver`] with document-relative image URLs.
#[allow(clippy::too_many_arguments)]
pub fn layout_pages_with_page_geometry_and_resolver_and_base_url(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_geometry_and_resolver_and_base_url_and_control(
        document,
        cascade,
        page_box,
        page_steps,
        page_widths,
        resolver,
        base_url,
        &PageLayoutControl::default(),
    )
}

/// [`layout_pages_with_page_geometry_and_resolver_and_base_url`] with an
/// explicit page limit and cancellation check.
#[allow(clippy::too_many_arguments)]
pub fn layout_pages_with_page_geometry_and_resolver_and_base_url_and_control(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
    control: &PageLayoutControl<'_>,
) -> Result<Vec<PageSlice>, LayoutError> {
    control.check_page_index(0)?;
    super::validate_layout_depth(document)?;
    document.page_projection.clear();
    document.mark_in_document_flags();
    resolve_document_images(document, cascade, resolver, base_url)?;
    layout_pages_with_page_geometry_and_control(
        document,
        cascade,
        page_box,
        page_steps,
        page_widths,
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

/// Layout ordinary block flow with an optional per-page content-height schedule.
///
/// An empty schedule preserves the historical fixed fragmentainer height.  The
/// paged WPT adapter supplies a schedule when `@page` changes the page size or
/// margins after the first page; the normal API remains fixed-size by default.
pub fn layout_pages_with_page_steps(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_geometry(document, cascade, page_box, page_steps, &[])
}

/// Layout ordinary block flow with per-page content heights and widths.
///
/// `page_widths` contains content-box widths corresponding to `page_steps`.
/// When present, percentage-sized boxes are rescaled after pagination so a
/// later page with a different containing-block width does not retain the
/// first page's used percentage width.  An empty width schedule keeps the
/// existing fixed-page behavior.
pub fn layout_pages_with_page_geometry(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_geometry_and_control(
        document,
        cascade,
        page_box,
        page_steps,
        page_widths,
        &PageLayoutControl::default(),
    )
}

/// [`layout_pages_with_page_geometry`] with an explicit page limit and
/// cancellation check.
pub fn layout_pages_with_page_geometry_and_control(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    control: &PageLayoutControl<'_>,
) -> Result<Vec<PageSlice>, LayoutError> {
    control.page_limit_reached.set(false);
    control.check_page_index(0)?;
    // A root with `display: none` generates no boxes, so there is nothing to
    // lay out or break into later pages: the document prints as a single
    // blank page.
    if BoxPropertiesRef::from_computed(cascade.root_element_computed()).display()
        == DisplayValue::None
    {
        // Fragments and a projection of an earlier layout no longer apply.
        document.fragment_tree.clear();
        document.column_rules.clear();
        document.page_projection.clear();
        return Ok(vec![PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }]);
    }
    layout_single_page_with_table_projection(document, cascade, page_box, true, true)?;
    control.check_page_index(0)?;

    let body_id = super::initial_containing_block::root_element(document).ok_or_else(|| {
        LayoutError::Internal {
            message: "no document element found".to_string(),
        }
    })?;
    let margins = page_margins(cascade, page_box);
    let insets = page_content_insets(cascade, page_box);
    let Some(axes) = transposed_flow::FlowAxes::of_root(document, cascade) else {
        return paginate_flow(
            document,
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
        );
    };
    // The pages of a document whose root lays out its blocks on vertical
    // axes follow the physical x axis: each page takes the width of its page
    // area from the flow. The paginator breaks along y, so it runs on the
    // transposed document. The pages keep the inline size of the first page
    // area.
    let transposed = transposed_flow::TransposedDocument::transpose(
        document,
        body_id,
        axes,
        insets.page_area_width(margins, page_box),
    );
    let transposed_cascade = transposed.cascade(cascade);
    let pages = paginate_flow(
        document,
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
    );
    transposed.restore(document);
    pages
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
    /// The page type of the flow at the candidate being placed.
    current: Option<String>,
    /// The page type the enclosing flow set last; an unnamed candidate
    /// outside a named flex context compares its own against it.
    outer: Option<String>,
    by_page: Vec<Option<String>>,
}

impl PageNames {
    fn new(first: Option<String>) -> Self {
        Self {
            by_page: vec![first.clone()],
            outer: first.clone(),
            current: first,
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
        if !candidate.page_name_in_flow {
            return (false, self.current.clone());
        }
        match candidate.named_flex_context {
            Some(context) if candidate.deferred_named_break_after => {
                // Empty enclosing column boxes do not open an anonymous
                // page before the row's established deferred boundary.
                let seen = column_contexts.is_occupied(context, page, document, cascade)
                    || self.current.is_some();
                self.outer = candidate.page_name.clone();
                (saw_child && seen, self.current.clone())
            }
            Some(context)
                if candidate.is_named || candidate.is_text || candidate.is_rendered_leaf =>
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
                let previous = if candidate.is_named {
                    self.current.clone()
                } else {
                    self.outer.clone()
                };
                if !candidate.is_float_descendant {
                    self.outer = candidate.page_name.clone();
                }
                (saw_child, previous)
            }
        }
    }

    /// Give `page` the current page type.
    fn name_page(&mut self, page: u32) {
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

/// Break the laid-out flow of `document` into pages of `geometry` along y.
/// `page_steps` and `page_widths` give the content height and width of each
/// page area, as for [`layout_pages_with_page_geometry_and_control`].
#[allow(clippy::too_many_arguments)]
fn paginate_flow(
    document: &mut Document,
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
    // A page with margins consuming the entire paper still needs a finite
    // cursor for forced breaks.  No valid page box reaches this path in normal
    // CSS, but the fallback keeps the API panic-free for direct callers.
    let fixed_page_step = if content_height.is_finite() && content_height > 0.0 {
        content_height
    } else {
        page_box.height.max(1.0)
    };
    // A page-size/margin change can make the fragmentainer height vary by
    // page.  The default API supplies no schedule and therefore follows the
    // fixed first-page height used historically.
    let page_step = fixed_page_step;
    let page_origins = PageOrigins::new(page_steps, fixed_page_step);
    let page_step_at = |page_index: u32| page_origins.step_at(page_index);
    let page_origin = |page_index: u32| page_origins.origin(page_index);
    let page_index_for_y = |y: f32| page_origins.page_index_for_y(y);
    let page_index_in_flow = |y: f32| page_origins.page_index_in_flow(y);
    let page_index_for_end = |end: f32| page_origins.page_index_for_end(end);
    // The document element has its own used margin, outside its BFC.
    // Truncate the part adjoining an unforced break without changing any
    // relative positioning offset carried by its border-box location.
    let root_margin = document.nodes[body_id].unrounded_layout.margin.top;
    let root_margin_page = page_index_for_y(root_margin);
    if root_margin_page > 0 {
        control.check_discovery_page_index(root_margin_page)?;
        document.nodes[body_id].unrounded_layout.location.y -=
            root_margin - page_origin(root_margin_page);
    }
    let leading_margins_moved =
        leading_margins::truncate(document, cascade, body_id, &page_origins, control)?
            || root_margin_page > 0;
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
        let mut child = owner;
        while let Some(parent) = document.layout_parent_of(child) {
            if document.nodes[child].taffy_position() == TaffyPosition::Absolute
                || document.nodes[child].style.float.is_floated()
            {
                break;
            }
            // A paragraph's line shifts follow its boxes' current positions.
            if document.nodes[parent].ifc.is_some() {
                sibling_shifts.apply(document, &mut fragment_edits, parent);
            }
            follow_moved_ifc_block(document, child, delta);
            sibling_shifts.record(document, parent, child, delta, &mut column_work)?;
            if !document.nodes[parent].style.size.height.is_auto() {
                break;
            }
            document.nodes[parent].unrounded_layout.size.height += delta;
            fragment_edits.grow(parent, delta);
            if parent == body_id {
                break; // cov:ignore: the body has a pinned viewport height and exits at the preceding fixed-height boundary.
            }
            child = parent;
        }
    }
    sibling_shifts.apply_all(document, &mut fragment_edits);
    fragment_edits.finish(document, &mut column_work)?;
    document.fragment_tree.break_flow_work_used = work_limit - column_work.remaining();
    document.fragment_tree.finalize();
    document.column_rules = crate::column_rules::prepare(document, cascade)?;

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
    let mut table_headers =
        HeaderRepeats::prepare(document, cascade, roots.contents, page_step, |id| {
            root_flow_offset + current_abs_y(document, id)
        });
    let table_row_leads = table_row_leads(document, cascade, &candidates, root_flow_offset);
    let avoided_row_groups = avoided_row_groups(
        document,
        cascade,
        &candidates,
        &table_headers,
        &table_row_leads,
    );
    let avoided_run_ends = avoided_run_ends(
        document,
        cascade,
        &candidates,
        roots.contents_box,
        content_width,
    );
    let mut flow_shift = 0.0_f32;
    let mut current_page = 0_u32;
    let mut reach = PageReach::default();
    let mut pending_break = PendingBreak::default();
    let mut deferred_shifts = DeferredShifts::default();
    // Where each table row sits in its parent before pagination; the table
    // boxes around the rows that moved follow them once every row is placed.
    let table_row_origins: Vec<(usize, f32)> = candidates
        .iter()
        .filter(|candidate| candidate.is_table_row)
        .map(|candidate| {
            let row = candidate.node_id;
            (row, document.nodes[row].unrounded_layout.location.y)
        })
        .collect();
    let mut float_shifts = FloatShifts::default();
    let mut subtree_shifts = SubtreeShifts::default();
    let mut sibling_margins = SiblingMargins::default();
    let mut last_named = LastNamedBox::default();
    let mut saw_child = false;
    let mut checked_paragraphs = CheckedParagraphs::default();
    // Footnotes are collected in source order and stacked upward from the
    // bottom edge of their anchor page. The normal-flow height is removed
    // from `flow_shift`, leaving the box only in this page-local area.
    let mut footnote_areas = FootnoteAreas::default();
    let mut page_names = PageNames::new(
        selected_page_name(cascade, body_id).or_else(|| first_page_name(document, cascade)),
    );
    let mut column_contexts = ColumnContexts::new(document.nodes.len());
    let mut preceding_flex_items = PrecedingFlexItems::default();
    let mut trailing_flex_children = TrailingFlexChildren::default();
    let mut entered_paragraphs = EnteredParagraphs::default();
    macro_rules! check_candidate_page {
        ($label:lifetime, $page_index:expr) => {
            match control.check_page_index($page_index) {
                Ok(()) => {}
                Err(LayoutError::PageLimitExceeded { .. }) if control.truncate_at_page_limit => {
                    control.page_limit_reached.set(true);
                    break $label;
                }
                Err(error) => return Err(error),
            }
        };
    }

    'candidate_loop: for candidate in candidates {
        check_candidate_page!('candidate_loop, current_page);
        let node_id = candidate.node_id;
        // Header lines stay together and are placed from the table's rows,
        // rather than acquiring independent text-page shifts.
        if table_headers.owner(node_id).is_some() {
            continue;
        }
        let name_participates_in_flow = candidate.page_name_in_flow;
        if name_participates_in_flow
            && candidate.is_flex_item
            && let Some(context) = candidate.named_flex_context
            && preceding_flex_items.record(context, candidate.height, current_page)
        {
            column_contexts.occupy(Some(context), current_page, document, cascade);
        }
        let (name_context_seen, comparison_page_name) = page_names.comparison(
            &candidate,
            saw_child,
            &mut column_contexts,
            document,
            cascade,
            current_page,
        );
        // Leaving a flex item's internal named flow keeps the last opened
        // physical page until an outer boundary or natural overflow opens one.
        let keep_local_page_type =
            candidate.named_flex_context.is_none() && page_names.current != comparison_page_name;
        let page_before_candidate = current_page;
        let moves_ifc_root = entered_paragraphs.enter(document, candidate.ifc_root, node_id);
        let ifc_root = candidate.ifc_root.filter(|_| moves_ifc_root);
        deferred_shifts.apply_reached(
            document,
            &text_lines,
            node_id,
            candidate.ifc_root,
            &mut flow_shift,
        );
        if let Some(page) = subtree_shifts.leave(document, node_id, &mut flow_shift) {
            current_page = page;
        }
        if let Some((delta, flow_page)) = float_shifts.end_pending_outside(document, node_id) {
            flow_shift -= delta;
            current_page = flow_page;
        }
        let float_edges = float_shifts.edges_for(document, cascade, node_id, body_id);
        if let Some((float_context, float_edges)) = float_edges {
            let floating = is_floating_box_for_pagination(document, cascade, node_id);
            let computed = &cascade.computed[node_id];
            // A later float's outer top stays not above an earlier float
            // (CSS 2.1 §9.5.1), and below the floats it does not fit beside
            // in the context. An in-flow box's border box stays below a float
            // it clears (CSS 2.1 §9.5.2).
            let layout = &document.nodes[node_id].unrounded_layout;
            let outer_width = layout.size.width + layout.margin.left + layout.margin.right;
            let context_width = document.nodes[float_context]
                .unrounded_layout
                .content_box_width();
            let cleared = float_edges
                .iter()
                .filter(|edge| {
                    clears_float_side(
                        computed.clear,
                        [FloatValue::Left, FloatValue::Right][edge.side],
                    )
                })
                .map(|edge| edge.bottom)
                .reduce(f32::max);
            let floor = if floating {
                let outer_height = layout.size.height + layout.margin.top + layout.margin.bottom;
                let top = float_top_beside(&float_edges, outer_width, outer_height, context_width);
                Some(top.max(cleared.unwrap_or(f32::NEG_INFINITY)))
            } else if candidate.is_float_descendant {
                None
            } else {
                cleared
            };
            let margin_top = if floating {
                document.nodes[node_id].unrounded_layout.margin.top
            } else {
                0.0
            };
            // Floors are in normal-flow coordinates, without the relative
            // offsets of the box and its ancestors.
            let follower_offset =
                accumulated_relative_offset_y(document, cascade, body_id, node_id);
            let flow_raw_y = candidate.raw_y - follower_offset;
            let shift = floor.map_or(0.0, |floor| {
                (floor - (flow_raw_y + flow_shift - margin_top)).clamp(0.0, float_shifts.later)
            });
            if shift > 0.0 {
                flow_shift += shift;
                if floating {
                    float_shifts.pending = Some((node_id, shift, current_page));
                    // The shifted float is now the one later floats on its
                    // side keep below.
                    if let Some(index) = float_side_index(cascade.computed[node_id].float) {
                        let layout = &document.nodes[node_id].unrounded_layout;
                        let top = flow_raw_y + flow_shift;
                        float_shifts.record_edge(
                            float_context,
                            FloatEdge {
                                node: node_id,
                                side: index,
                                top: top - layout.margin.top,
                                bottom: top + layout.size.height + layout.margin.bottom,
                                width: outer_width,
                            },
                        );
                    }
                }
                if !floating && !candidate.is_direct_body_element {
                    // The ancestors of a nested clearing box grow to hold it.
                    grow_ancestors_for_shift(
                        document,
                        cascade,
                        node_id,
                        shift,
                        body_id,
                        &mut subtree_shifts,
                        current_page,
                    );
                }
                // The box may now start on a later page than the flow.
                current_page = current_page.max(page_index_for_y(candidate.raw_y + flow_shift));
            }
        }
        if !document.is_source_node(node_id) {
            continue;
        }
        let participates_in_margin_adjacency =
            participates_in_margin_adjacency(document, cascade, &candidate);
        // Explicit positioned insets follow their containing box, not margin
        // changes in a parallel normal flow. Both-auto boxes keep static placement.
        let uses_positioned_coordinates = candidate.is_out_of_flow_descendant
            && has_explicit_positioned_block_ancestor(document, node_id);
        if candidate.is_text {
            // The inline layout root whose lines hold this text, if any.
            let text_ifc_root = candidate.ifc_root.map(|(root, _)| root);
            // In-flow text between two blocks separates them: the next block's
            // gap is no longer a margin adjoining the previous block.
            if participates_in_margin_adjacency
                && let Some(parent_id) = text_ifc_root.or_else(|| {
                    BoxTree::for_layout(document).parent(node_id, BoxParentScope::Document)
                })
            {
                sibling_margins.previous_bottoms.remove(&parent_id);
            }
            // A footnote's descendants follow the moved footnote box. They are
            // deliberately not paginated independently, otherwise the stale
            // pre-pagination coordinates would move them back into normal flow.
            if candidate.is_float_descendant && has_footnote_ancestor(cascade, node_id, document) {
                continue;
            }
            // Text is not itself a class-A box, but non-empty text can be the
            // content that follows a nested named box.  Carry the containing
            // box's page type so that content after `page:b` inside a
            // `page:a` box resumes on an `a` page rather than becoming an
            // anonymous page.  Only direct body text consumes a pending
            // break-after; descendants must wait until their containing box
            // has finished.
            let raw_y = candidate.raw_y;
            let mut height = candidate.height;
            let candidate_page_name = candidate.page_name.clone();
            let mut effective_y = if uses_positioned_coordinates {
                positioned_text_start_y(document, &text_lines, node_id, text_ifc_root)
            } else {
                raw_y + flow_shift
            };
            // A fixed-height block whose first line would straddle the
            // current fragmentainer is pushed as a unit. This matches normal
            // block fragmentation for monolithic content while leaving tall
            // blocks and already-forced page transitions to the existing
            // fragment logic.
            // A block inside an inline element of an inline engine paragraph
            // is laid out from the paragraph's root, as a direct child is.
            // A block nested in plain block flow is moved the same way; its
            // ancestors then grow to hold it.
            // The block of text in an IFC paragraph is its root, including nested inline elements.
            let paragraph_root = candidate
                .ifc_root
                .map(|(root, _)| root)
                .filter(|&root| document.is_source_node(root) && root != node_id);
            let line_block_parent = line_block_parent(
                document,
                cascade,
                &roots,
                &candidate,
                paragraph_root,
                &checked_paragraphs,
                uses_positioned_coordinates,
            );
            if let Some((block_id, nested)) = line_block_parent
                && checked_paragraphs.block_text.insert(block_id)
                && style_dimension_length(document.nodes[block_id].style.size.height)
                    .is_some_and(|value| value >= 0.0)
            {
                let block_height = document.nodes[block_id].unrounded_layout.size.height;
                let block_effective_y = block_effective_y(
                    document,
                    block_id,
                    raw_y,
                    effective_y,
                    root_flow_offset,
                    flow_shift,
                );
                if first_line_overflows(
                    effective_y,
                    height,
                    block_effective_y,
                    block_height,
                    page_origin(current_page),
                    page_step_at(current_page),
                ) {
                    let natural_page = page_index_for_y(effective_y + height);
                    let target_page = current_page.saturating_add(1).max(natural_page);
                    check_candidate_page!('candidate_loop, target_page);
                    let target_y = page_origin(target_page);
                    let delta = move_line_block(
                        document,
                        cascade,
                        body_id,
                        block_id,
                        nested,
                        block_effective_y,
                        target_y,
                        &mut subtree_shifts,
                        page_before_candidate,
                    );
                    flow_shift += delta;
                    effective_y += delta;
                }
            }
            // `orphans` and `widows` constrain breaks between line boxes, not
            // breaks between block-level children.  When the paragraph has
            // lines enough for both, a break that leaves too few widows moves
            // up between earlier lines; otherwise a fitting block is moved as
            // a unit when its natural split would violate either constraint.
            // Oversized blocks that cannot break earlier keep their natural
            // break.
            // A paragraph that is a flex item alone on its line cannot move
            // as a unit, but its later lines can move down inside it. It is
            // looked for only when no line block parent was found.
            let flex_line_paragraph = flex_line_paragraph(
                document,
                cascade,
                body_id,
                &candidate,
                paragraph_root.filter(|_| line_block_parent.is_none()),
                uses_positioned_coordinates,
            );
            if let Some((block_id, nested)) =
                line_block_parent.or(flex_line_paragraph.map(|(root, _)| (root, true)))
                && checked_paragraphs.orphans_widows.insert(block_id)
                && let Some(line_metrics) = paragraph_line_bounds(document, node_id, &text_lines)
            {
                let page_start = page_origin(current_page);
                let page_end = page_start + page_step_at(current_page);
                let line_fit = ParagraphLineFit::measure(
                    &line_metrics,
                    effective_y,
                    page_start,
                    page_end,
                    cascade.computed[block_id].orphans,
                    cascade.computed[block_id].widows,
                );
                let block_height = document.nodes[block_id].unrounded_layout.size.height;
                let fits_on_page =
                    block_height.is_finite() && block_height <= page_step_at(current_page) + 0.001;
                // Lines move down inside the paragraph only when it has
                // lines enough for both constraints, is its own inline layout
                // root, has no earlier split pending, and can shift its lines.
                let splittable = line_fit.has_room_for_both_constraints()
                    && paragraph_root == Some(block_id)
                    && deferred_shifts.line_splits.is_empty()
                    && lines_can_shift_inside(document, block_id);
                let [earlier_break, final_break] = if splittable {
                    line_fit.line_moves(&line_metrics, effective_y, current_page, &page_origins)
                } else {
                    [None, None]
                };
                if earlier_break.is_some() || final_break.is_some() {
                    for (line, shift, page) in earlier_break.into_iter().chain(final_break) {
                        check_candidate_page!('candidate_loop, page);
                        shift_ifc_lines_from(document, block_id, line, shift);
                        if let Some((item, container)) = flex_line_paragraph {
                            shift_later_flex_lines(document, container, item, shift);
                        }
                        grow_ancestors_for_shift(
                            document,
                            cascade,
                            block_id,
                            shift,
                            body_id,
                            &mut subtree_shifts,
                            page_before_candidate,
                        );
                        if text_lines
                            .get(node_id)
                            .and_then(|owned| owned.lines.last())
                            .is_some_and(|last| last.line >= line)
                        {
                            height += shift;
                        }
                        deferred_shifts.line_splits.push((block_id, line, shift));
                    }
                } else if line_block_parent.is_some()
                    && line_fit.starts_on_page
                    && fits_on_page
                    && line_fit.violates_line_constraints
                {
                    // The move is measured from where the block is now.
                    let block_effective_y = block_effective_y(
                        document,
                        block_id,
                        raw_y,
                        effective_y,
                        root_flow_offset,
                        flow_shift,
                    );
                    let target_page = current_page.saturating_add(1);
                    check_candidate_page!('candidate_loop, target_page);
                    let delta = move_line_block(
                        document,
                        cascade,
                        body_id,
                        block_id,
                        nested,
                        block_effective_y,
                        page_origin(target_page),
                        &mut subtree_shifts,
                        page_before_candidate,
                    );
                    flow_shift += delta;
                    effective_y += delta;
                }
            }
            place_text_candidate(
                document,
                &text_lines,
                node_id,
                ifc_root,
                text_ifc_root,
                effective_y,
            );
            let named_page_change = name_context_seen
                && height > 0.0
                && name_participates_in_flow
                && candidate_page_name != comparison_page_name;
            let pending_break_applies = pending_break.applies_to(document, node_id);
            let consumes_pending_break = pending_break_applies && name_participates_in_flow;
            if saw_child {
                if consumes_pending_break || named_page_change {
                    let natural_page = page_index_in_flow(effective_y).unwrap_or(current_page);
                    let target_page = current_page.saturating_add(1).max(natural_page);
                    check_candidate_page!('candidate_loop, target_page);
                    let target_y = page_origin(target_page);
                    // The candidate was materialized to `effective_y` above,
                    // so this is the additional movement for this boundary.
                    // The same amount advances `flow_shift`, which also moves
                    // later candidates whose ancestor is not moved here.
                    let delta = target_y - effective_y;
                    if delta.is_finite() {
                        shift_text_candidate(
                            document,
                            &text_lines,
                            node_id,
                            ifc_root.map(|(root, _)| root),
                            text_ifc_root,
                            delta,
                        );
                        flow_shift += delta;
                        effective_y += delta;
                    }
                    current_page = target_page;
                    page_names.current = candidate_page_name.clone();
                } else if let Some(natural_page) = page_index_in_flow(effective_y) {
                    check_candidate_page!('candidate_loop, natural_page);
                    current_page = current_page.max(natural_page);
                }
            } else {
                saw_child = true;
                current_page = 0;
                if name_participates_in_flow {
                    page_names.current = candidate_page_name.clone();
                }
            }
            if height > 0.0
                && name_participates_in_flow
                && !named_page_change
                && !consumes_pending_break
                && (!keep_local_page_type || current_page > page_before_candidate)
            {
                page_names.current = candidate_page_name;
            }
            check_candidate_page!('candidate_loop, current_page);
            if name_participates_in_flow && height > 0.0 {
                column_contexts.occupy_enclosing(node_id, current_page, document, cascade);
            }
            let end_page = candidate_flow_end(effective_y, height).map(page_index_for_end);
            let in_flow = !candidate.is_out_of_flow_descendant;
            record_candidate_pages(
                &mut reach,
                &mut page_names,
                control,
                current_page,
                end_page,
                in_flow,
            )?; // cov:ignore: LLVM coverage maps this multiline call's error exit to its own line.
            if consumes_pending_break {
                pending_break.clear();
            }
            pending_break.note_after(
                document,
                cascade,
                node_id,
                candidate.deferred_named_break_after,
            );
            continue;
        }

        let computed = &cascade.computed[node_id];
        let candidate_page_name = candidate.page_name.clone();
        // `float: footnote` is a page-local out-of-flow placement. Keep the
        // anchor's page, stack multiple notes from the bottom, and remove the
        // note's ordinary-flow footprint so following content can continue.
        if matches!(computed.float, FloatValue::Footnote) {
            let raw_y = candidate.raw_y;
            let height = candidate.height;
            let effective_y = raw_y + flow_shift;
            let anchor_page = page_index_in_flow(effective_y).unwrap_or(current_page);
            let note_page = current_page.max(anchor_page);
            check_candidate_page!('candidate_loop, note_page);
            let target_y = footnote_areas.stack(
                note_page,
                page_origin(note_page),
                page_origin(note_page) + page_step_at(note_page),
                height,
            );
            materialize_y(document, node_id, target_y);
            flow_shift -= height;
            reach.include(note_page, false);
            saw_child = true;
            continue;
        }
        // Copy layout data before any break adjustment so the immutable node
        // borrow does not overlap the in-place location update below.
        let raw_y = candidate.raw_y;
        let height = candidate.height;
        place_first_table_header(
            document,
            &mut table_headers,
            node_id,
            flow_shift,
            &page_origins,
        );
        let mut effective_y = if uses_positioned_coordinates {
            current_abs_y(document, node_id)
        } else {
            raw_y + flow_shift
        };
        materialize_y(document, node_id, effective_y);
        let forced_before = page_break_is_forced(document, node_id, computed.break_before);
        // Set when a forced break starts this box's page, which makes the box
        // the owner of the margins after the break.
        let mut starts_after_forced_break = false;
        // A block box with no block size and no in-flow content lets its
        // block-start and block-end margins collapse through it.
        let margins_collapse_through = collapses_through(document, cascade, node_id, height);
        let (margin_top, trailing_margin) =
            break_margins(document, cascade, node_id, content_width);
        let page_start = page_origin(current_page);
        let page_end = page_start + page_step_at(current_page);
        let avoided_run_overflow = avoided_run_ends.get(&node_id).is_some_and(|end| {
            avoided_run_moves(
                *end - raw_y,
                margin_top,
                effective_y,
                page_start,
                page_end,
                page_step_at(current_page.saturating_add(1)),
            )
        });
        // Whether the box fits is decided at its normal-flow position,
        // before any relative offset.
        let relative_offset = accumulated_relative_offset_y(document, cascade, body_id, node_id);
        let flow_y = effective_y - relative_offset;
        // `break-inside` does not apply to inline boxes (CSS Fragmentation 3
        // §3.2): the block boxes inside an avoiding inline box break as they
        // would without it. An atomic inline, such as an inline table, is a
        // single box in its line and still moves as a whole.
        let page_overflow = avoids_page_break_inside(computed.break_inside)
            && (!document.nodes[node_id].display().is_inline_flow()
                || page_applies_to_box(document, node_id))
            && avoided_box_moves(
                flow_y,
                margin_top,
                flow_y + height + trailing_margin,
                page_start,
                page_end,
            );
        // A named page requested by a class-A box starts a new page when it
        // differs from the current named page. `auto` leaves the current page
        // type in place; it does not manufacture a second break boundary.
        // Any non-empty class-A box with a different page name starts a new
        // page.  `None` is the anonymous/unnamed page type, so an anonymous
        // box after a named one is a transition too.  Named boxes that start
        // at the coordinate of a preceding zero-height named box join its
        // transition: no block-size has been consumed on that page yet, so a
        // break there would only leave the zero-height boxes alone on a page.
        // This matters for chains of empty named boxes with overflowing text,
        // including the first non-empty box that follows the chain.
        let same_named_coordinate = candidate.is_named
            && (candidate.is_direct_body_element || candidate.named_flex_context.is_some())
            && last_named.shares_transition(candidate.named_flex_context, raw_y);
        let named_page_change = name_context_seen
            && candidate_page_name != comparison_page_name
            && !same_named_coordinate;
        // A forced break at the start of a page already opened by natural
        // overflow describes the same boundary and must not create a blank
        // page. The forced break sits before the box's margin box, so the
        // test uses its block-start margin edge.
        let forced_break_at_page_start = forced_before
            && margin_box_starts_page(
                &page_origins,
                effective_y - margin_top.max(0.0),
                current_page,
                page_start,
            );
        // Keep an ordinary table row intact when it would cross the current
        // page. The row is a class-A boundary owned by the table engine; its
        // cells and descendants follow the same flow shift below. Rowspans,
        // cell-internal breaks remain out of this narrow row-boundary pass.
        // A row that follows a repeated header must fit beside the header's
        // copy on the next page; otherwise moving it gains nothing.
        let repeated_header_height = table_headers
            .rows
            .get(&node_id)
            .and_then(|header_id| table_headers.headers.get(header_id))
            .map_or(0.0, |header| header.height + header.gap);
        let table_row_lead = table_row_leads.get(&node_id).copied().unwrap_or(0.0);
        let table_row_overflow = candidate.is_table_row
            && table_row_moves(
                flow_y,
                height,
                table_row_lead + height + repeated_header_height,
                page_start,
                page_end,
                page_step_at(current_page.saturating_add(1)),
            );
        let row_group = avoided_row_groups.get(&node_id).copied();
        let row_group_overflow = row_group
            .is_some_and(|(end, _)| row_group_moves(flow_y, end - raw_y, page_start, page_end));
        // A trailing flex item may extend into the containing flex box's
        // continuation at the page edge. Earlier items still move intact when
        // they would cross a fragmentainer, preserving the existing column
        // flex pagination behavior.
        let flex_item_is_last = candidate.is_flex_item
            && trailing_flex_children.is_trailing(document, cascade, node_id);
        let item_overflow = || {
            item_moves_past_page_end(
                effective_y,
                height,
                margin_top,
                page_start,
                page_end,
                page_step_at(current_page.saturating_add(1)),
            )
        };
        let flex_item_overflow = candidate.is_flex_item && !flex_item_is_last && item_overflow();
        let grid_item_overflow = candidate.is_grid_item && item_overflow();

        let collapses_with_previous_block =
            collapses_with_previous_block(document, &candidate, participates_in_margin_adjacency);
        let pending_break_applies = pending_break.applies_to(document, node_id);
        // CSS Fragmentation 3 §5.2: margins adjoining an unforced break are
        // truncated, while a forced break keeps the block-start margin of the
        // box after it.
        let forced_transition = pending_break_applies
            || (forced_before && !forced_break_at_page_start)
            || named_page_change;
        let page_transition = saw_child
            && (forced_transition
                || page_overflow
                || avoided_run_overflow
                || table_row_overflow
                || row_group_overflow
                || flex_item_overflow
                || grid_item_overflow);
        if saw_child {
            // A break-after on the preceding box and a break-before (or named
            // page transition) on this box describe the same boundary, not two
            // blank pages.
            if page_transition {
                let natural_page = if let Some(page) = page_index_in_flow(effective_y) {
                    // Preceding content that overflowed past the current
                    // page (a tall box, or floats it contains) has put this
                    // box partway down a later page. A page-type change sits
                    // after all of that content, so it opens the page after
                    // the one the box's margin box starts on, unless that
                    // margin box is already at the top of a page. Content
                    // kept on its page as monolithic (see
                    // `monolithic_paper_fit`) has not opened that later page.
                    let margin_box_y = effective_y - margin_top.max(0.0);
                    if named_page_change
                        && page > current_page
                        && reach.flow_max >= page
                        && margin_box_y.is_finite()
                    {
                        page_origins.page_starting_at_or_after(margin_box_y)
                    } else {
                        page
                    }
                } else {
                    current_page
                };
                let target_page = current_page.saturating_add(1).max(natural_page);
                check_candidate_page!('candidate_loop, target_page);
                let target_y = page_origin(target_page);
                // A negative block-start margin can pull a forced-break box
                // back into the preceding page.  Keep the break boundary for
                // following siblings (and for page count), but materialize
                // this box at the underflowed coordinate.  This is the
                // `underflow-from-next-page` case from CSS Break.
                let underflow_y = target_y + margin_top;
                let node_target_y =
                    if forced_transition && margin_top < 0.0 && underflow_y < target_y {
                        underflow_y
                    } else if forced_transition {
                        // The forced break is before the box's margin box, so a
                        // positive block-start margin remains on the new page.
                        // A table's first row keeps the table's top edge above it.
                        target_y + margin_top.max(0.0) + table_row_lead
                    } else if row_group_overflow {
                        target_y + row_group.map_or(0.0, |(_, lead)| lead) + relative_offset
                    } else if table_row_overflow {
                        target_y + table_row_lead + relative_offset
                    } else {
                        // A relative offset moves with the box to the page
                        // it starts on.
                        target_y + relative_offset
                    };
                // The candidate was materialized to `effective_y` above,
                // so this is the additional movement for this boundary.
                // Keep `flow_shift` at the page boundary even when the
                // candidate itself underflows into the preceding page.
                let node_delta = node_target_y - effective_y;
                let shift_delta = target_y - effective_y;
                let row_flex_break_before =
                    forced_before && in_row_flex_container(document, cascade, node_id);
                if node_delta.is_finite() && shift_delta.is_finite() {
                    document.nodes[node_id].unrounded_layout.location.y += node_delta;
                    follow_moved_ifc_block(document, node_id, node_delta);
                    // A nested avoiding block that moves to the next page
                    // lengthens the ancestors whose content box holds it, so
                    // their continuation (and any `overflow` clip) reaches the
                    // moved content.
                    // A float does not lengthen the block around it; its
                    // own shift ends with its subtree.
                    let mut moved_in_bounded_ancestor = false;
                    let nested_block = !candidate.is_direct_body_element
                        && node_delta > 0.0
                        && !is_floating_box_for_pagination(document, cascade, node_id);
                    if (page_overflow || (forced_transition && candidate.is_direct_body_element))
                        && node_delta > 0.0
                        && !is_floating_box_for_pagination(document, cascade, node_id)
                    {
                        moved_in_bounded_ancestor = grow_ancestors_for_shift(
                            document,
                            cascade,
                            node_id,
                            node_delta,
                            body_id,
                            &mut subtree_shifts,
                            page_before_candidate,
                        );
                    } else if forced_transition
                        && nested_block
                        && !candidate.is_flex_item
                        && !candidate.is_grid_item
                        && !candidate.is_table_row
                    {
                        // A forced break ends its ancestors' fragments on
                        // this page, which their continuations account for;
                        // only the boxes after it in block flow follow it,
                        // up to the first ancestor that does not stack its
                        // children in block order.
                        // A box pulled back over the break by a negative
                        // margin still leaves the flow after it at the break.
                        shift_block_flow_after(
                            document,
                            node_id,
                            body_id,
                            node_delta.max(shift_delta),
                        );
                    }
                    // A forced break on one wrapped row-flex item belongs to
                    // its whole flex line. Move same-line siblings together;
                    // other lines keep their existing flow coordinates.
                    if row_flex_break_before && let Some(parent_id) = document.parent_of(node_id) {
                        move_flex_line_siblings(document, parent_id, node_id, raw_y, shift_delta);
                    }
                    if node_target_y < target_y {
                        flow_shift += node_delta;
                        deferred_shifts.underflow = Some((node_id, shift_delta - node_delta));
                    } else {
                        // A kept margin moves the box's descendants and its
                        // later siblings along with the box.
                        flow_shift += node_delta;
                    }
                    if page_overflow && is_floating_box_for_pagination(document, cascade, node_id) {
                        let (inherited, flow_page) = float_shifts
                            .pending
                            .filter(|(float_id, _, _)| *float_id == node_id)
                            .map_or((0.0, page_before_candidate), |(_, delta, page)| {
                                (delta, page)
                            });
                        float_shifts.pending = Some((node_id, inherited + node_delta, flow_page));
                        // A clearing follower stays below the margin box,
                        // which a negative block-end margin pulls up.
                        if let Some(index) = float_side_index(computed.float) {
                            let context = float_context_root(document, cascade, node_id, body_id);
                            // Floors are kept in normal-flow coordinates,
                            // without the relative offsets of the ancestors.
                            let flow_target_y = node_target_y - relative_offset;
                            float_shifts.record_edge(
                                context,
                                moved_float_edge(
                                    document,
                                    node_id,
                                    index,
                                    flow_target_y,
                                    height,
                                    margin_top,
                                    forced_transition,
                                ),
                            );
                            // The margin box then moved further than the
                            // border box, and a later float may have to
                            // follow it that far.
                            if !forced_transition {
                                float_shifts.extend_later(inherited + node_delta + margin_top);
                            }
                        }
                        grow_float_context_to_moved_float(
                            document,
                            cascade,
                            node_id,
                            body_id,
                            height,
                            &mut subtree_shifts,
                            page_before_candidate,
                        );
                    } else if !candidate.is_float_descendant
                        && !is_floating_box_for_pagination(document, cascade, node_id)
                    {
                        // In-flow content that opened a page ends the floors
                        // of its formatting context for later floats: they
                        // start below it anyway. A float that still reaches
                        // below the content's new place keeps its floor for
                        // later clearing boxes. A box that moved inside an
                        // ancestor that cannot grow leaves the outer flow
                        // where it was, so the floors stay.
                        if !moved_in_bounded_ancestor {
                            let context = float_context_root(document, cascade, node_id, body_id);
                            let flow_top = node_target_y - relative_offset;
                            float_shifts.drop_edges_above(context, flow_top);
                        }
                    }
                    effective_y += node_delta;
                }
                current_page = target_page;
                if forced_transition {
                    sibling_margins.forced_break_box = margins_collapse_through.then_some(node_id);
                    starts_after_forced_break = true;
                }
                if pending_break_applies {
                    pending_break.clear();
                }
            } else if effective_y.is_finite() && effective_y >= 0.0 {
                // A box with no block size at a page's end still fits on
                // that page, so it does not open the next one: a forced
                // break after it would otherwise leave it alone on a page.
                let mut natural_page = if height <= 0.0 {
                    page_index_for_end(effective_y)
                } else {
                    page_index_for_y(effective_y)
                };
                // A forced break that coincides with a page edge also starts
                // that page.
                if forced_before {
                    sibling_margins.forced_break_box = margins_collapse_through.then_some(node_id);
                    starts_after_forced_break = true;
                }
                // An unforced break that falls inside the collapsed margin
                // between this box and its preceding sibling truncates that
                // margin (CSS Fragmentation 3 §5.2): the box starts at the
                // top of the page instead of keeping the part of the margin
                // that crossed the page edge.
                let previous = sibling_margins.gap_after_previous(document, node_id, raw_y);
                if let Some((previous, gap)) = previous
                    && collapses_with_previous_block
                    // A clearing box's gap may contain clearance. Its used
                    // clearance is not recorded separately, so retain the gap.
                    && matches!(computed.clear, raikiri_style::property::ClearValue::None)
                {
                    let page_top = page_origin(natural_page);
                    let previous_bottom = effective_y - gap;
                    // A border box ending exactly at the break still has
                    // an adjoining margin on the following page.
                    // An empty box that a forced break placed at the page
                    // top leaves its collapsed margin after the break.
                    // A later, unforced page edge inside that margin still
                    // truncates it.
                    let follows_forced_break =
                        sibling_margins.follows_forced_break(document, cascade, previous, node_id);
                    let after_forced_break =
                        follows_forced_break && (previous_bottom - page_top).abs() <= 0.001;
                    if natural_page > 0
                        && effective_y > page_top + 0.001
                        && previous_bottom <= page_top + 0.001
                        && !after_forced_break
                        // Only a margin between adjoining siblings is
                        // truncated; a box that is not a candidate may lie
                        // between two nested candidates.
                        && adjoins_previous_sibling(document, cascade, node_id, previous)
                    {
                        // A forced break that already coincides with this
                        // page edge sits before the margin, which therefore
                        // stays on the new page. Otherwise the margin breaks
                        // at the first page edge inside it, and the rest of
                        // it is truncated together with the blank pages it
                        // would fill.
                        let (target_top, target_y) = if forced_before {
                            (page_top, page_top + margin_top)
                        } else {
                            // Only the collapsed margin is truncated, not
                            // clearance or other space in the gap.
                            let collapsed = adjoining_margin(document, cascade, previous, false)
                                .max(adjoining_margin(document, cascade, node_id, true));
                            let margin_start = previous_bottom.max(effective_y - collapsed);
                            let first_edge = page_origins.first_edge_in_margin(
                                margin_start,
                                follows_forced_break,
                                natural_page,
                            );
                            (page_origin(first_edge), page_origin(first_edge))
                        };
                        let delta = target_y - effective_y;
                        sibling_margins.moved_boxes |= delta != 0.0;
                        document.nodes[node_id].unrounded_layout.location.y += delta;
                        follow_moved_ifc_block(document, node_id, delta);
                        shrink_ancestors_after_margin_truncation(
                            document, cascade, node_id, delta, body_id,
                        );
                        flow_shift += delta;
                        effective_y += delta;
                        if target_y < target_top {
                            // Descendants follow the negative margin; later
                            // siblings still follow the forced break boundary.
                            deferred_shifts.underflow = Some((node_id, target_top - target_y));
                        } else {
                            natural_page = page_index_for_y(effective_y);
                        }
                    }
                }
                check_candidate_page!('candidate_loop, natural_page);
                current_page = current_page.max(natural_page);
            }
            // An inline direct child keeps its source-order inline x position
            // in taffy's single pre-pagination layout. A forced page boundary
            // starts a fresh page formatting context, so reset that box's
            // inline origin before painting the next slice.
            if page_transition
                && candidate.is_direct_body_element
                && document.nodes[node_id].display().is_inline_flow()
            {
                document.nodes[node_id].unrounded_layout.location.x = 0.0;
            }
        } else {
            // A forced break before the first class-A box does not manufacture
            // a leading blank page.
            saw_child = true;
            current_page = 0;
        }
        // Parallel flows do not interrupt adjoining normal-flow siblings.
        if participates_in_margin_adjacency
            && let Some(parent_id) =
                BoxTree::for_layout(document).parent(node_id, BoxParentScope::Document)
        {
            if !starts_after_forced_break {
                sibling_margins.pass_on_forced_break(
                    document,
                    cascade,
                    parent_id,
                    node_id,
                    collapses_with_previous_block && margins_collapse_through,
                );
            }
            sibling_margins.record_bottom(
                parent_id,
                node_id,
                collapses_with_previous_block,
                raw_y + height,
            );
        }

        let header_delta = make_room_for_table_header(
            document,
            &mut table_headers,
            node_id,
            effective_y,
            height,
            current_page,
            &page_origins,
        );
        effective_y += header_delta;
        flow_shift += header_delta;

        if name_participates_in_flow
            && (candidate.is_direct_body_element
                || candidate.is_named
                || candidate.is_rendered_leaf
                || candidate.deferred_named_break_after)
            && (!keep_local_page_type || page_transition || current_page > page_before_candidate)
        {
            page_names.current = candidate_page_name;
            if candidate.is_direct_body_element || candidate.named_flex_context.is_some() {
                last_named = LastNamedBox::new(document, &candidate, node_id, raw_y, height);
            }
        }
        check_candidate_page!('candidate_loop, current_page);
        // A fixed-height direct block that fits on the physical paper is
        // monolithic in this minimal paginator.  Page border/padding reduce
        // the fragmentainer's content height, but must not make the block
        // manufacture a second page merely to expose its overflow into the
        // page decoration area.
        let monolithic_paper_fit = || {
            candidate.is_direct_body_element
                && (insets.top > 0.0
                    || insets.right > 0.0
                    || insets.bottom > 0.0
                    || insets.left > 0.0)
                && effective_y >= page_origin(current_page)
                && effective_y + height <= page_origin(current_page) + page_box.height + 0.001
                && style_dimension_length(document.nodes[node_id].style.size.height)
                    .is_some_and(|value| value >= 0.0)
        };
        let end_page = candidate_flow_end(effective_y, height)
            .filter(|_| !monolithic_paper_fit())
            .map(page_index_for_end);
        let in_flow = !candidate.is_out_of_flow_descendant;
        record_candidate_pages(
            &mut reach,
            &mut page_names,
            control,
            current_page,
            end_page,
            in_flow,
        )?;

        if name_participates_in_flow
            && (candidate.is_named || (candidate.is_rendered_leaf && height > 0.0))
        {
            column_contexts.occupy_enclosing(node_id, current_page, document, cascade);
        }
        if candidate.is_flex_item
            && let Some(context) = candidate.named_flex_context
        {
            preceding_flex_items.record(context, height, current_page);
        }
        pending_break.note_after(
            document,
            cascade,
            node_id,
            candidate.deferred_named_break_after,
        );
    }

    // A row moves within its parent with the page it opens, and with the
    // rows before it in the same table that opened one.
    let moved_table_rows: HashMap<usize, f32> = table_row_origins
        .into_iter()
        .filter_map(|(row, origin)| {
            let delta = document.nodes[row].unrounded_layout.location.y - origin;
            (delta > 0.001).then_some((row, delta))
        })
        .collect();
    super::table::row_moves::follow_moved_rows(document, &moved_table_rows, |table| {
        table_headers
            .headers
            .values()
            .any(|header| header.table == table)
    });
    super::table::headers::finish_boxes(document, &table_headers);
    update_paginated_block_heights(
        document,
        cascade,
        body_id,
        content_height,
        &unpaginated_overflow,
    );
    table_headers.index();
    document.table_objects.headers = table_headers;

    // The candidates above stand for the paginated flow only. Boxes outside
    // it, such as a nested absolutely positioned box or the overflow of a
    // monolithic box, still need pages to paint on, and so do table edges
    // that `finish_boxes` grew over repeated headers.
    if !control.page_limit_reached.get() {
        // Pagination materialized the moved boxes in page coordinates, which
        // already include `root_flow_offset`; boxes it left alone paint where
        // their own coordinates put them.
        let end =
            content_overflow_end(document, cascade, body_id, root_flow_offset, content_height);
        if end.is_finite() && end > 0.0 {
            reach.include_checked(control, page_index_for_end(end), false)?;
        }
    }
    // Pages past the last one the normal flow reached hold only boxes outside
    // it, and keep the page type the flow ended on, whatever page type was
    // active when one of those boxes first reached them.
    let names_end = control.capped_page_index(reach.max);
    page_names.continue_onto(reach.flow_max, names_end, true);

    // Truncating margins can move an ancestor of a fixed box. Restore only
    // its block position, preserving the already-corrected subtree layout.
    if sibling_margins.moved_boxes || leading_margins_moved {
        let root = super::initial_containing_block::root_element(document).unwrap_or(body_id);
        super::fixed::restore_fixed_box_block_positions(document, cascade, root, content_height);
    }

    if !page_widths.is_empty() {
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
                let page_index = page_index_for_y(current_abs_y(document, node_id));
                let Some(target_width) = page_widths
                    .get(page_index as usize)
                    .copied()
                    .filter(|width| width.is_finite() && *width > 0.0)
                else {
                    continue;
                };
                let scale = target_width / base_width;
                if scale.is_finite() && (scale - 1.0).abs() > 0.0001 {
                    document.nodes[node_id].unrounded_layout.size.width *= scale;
                }
            }
        }
    }

    control.check_discovery_page_index(reach.max)?;
    let last_page = if control.page_limit_reached.get() {
        control.max_pages.unwrap_or(0).saturating_sub(1)
    } else {
        reach.max
    };
    Ok((0..=last_page)
        .map(|page_index| PageSlice {
            page_index,
            content_origin_y: page_origin(page_index),
            page_name: page_names
                .by_page
                .get(page_index as usize)
                .cloned()
                .flatten(),
        })
        .collect())
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
    if candidates.iter().any(|candidate| candidate.is_table_row) {
        let candidate_y: HashMap<usize, f32> = candidates
            .iter()
            .map(|candidate| (candidate.node_id, candidate.raw_y))
            .collect();
        let table_of = |row: usize| {
            let mut table = document.parent_of(row);
            while let Some(id) = table
                && !matches!(
                    document.nodes[id].display(),
                    DisplayValue::Table | DisplayValue::InlineTable
                )
            {
                table = document.parent_of(id);
            }
            table
        };
        // Each table's first row in visual order: the table layout places a
        // footer group after the body groups wherever it is in the source.
        let mut table_first_rows = HashMap::<usize, (usize, f32)>::new();
        for candidate in candidates.iter().filter(|candidate| candidate.is_table_row) {
            if let Some(table) = table_of(candidate.node_id) {
                let first = table_first_rows
                    .entry(table)
                    .or_insert((candidate.node_id, candidate.raw_y));
                if candidate.raw_y < first.1 {
                    *first = (candidate.node_id, candidate.raw_y); // cov:ignore: rows are collected in visual order, so a later row never starts higher.
                } // cov:ignore: closes the visual-order guard above.
            }
        }
        for (table, (row, row_y)) in table_first_rows {
            // A caption above the grid stays on the current page; an
            // absolutely positioned one is out of the table's flow. Content
            // outside the rows, wrapped in an anonymous row, may come before
            // the first row.
            if document.nodes[table].children.iter().any(|&child| {
                document.nodes[child].display() == DisplayValue::TableCaption
                    && cascade.computed[child].caption_side
                        == raikiri_style::property::CaptionSideValue::Top
                    && document.nodes[child].taffy_position() != taffy::Position::Absolute
                    || document.nodes[child].display() != DisplayValue::TableRow
                        && document.anonymous_table_cells(child).next().is_some()
            }) || document.anonymous_table_cells(table).next().is_some()
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
        candidate.is_table_row
            && document
                .parent_of(candidate.node_id)
                .is_some_and(avoiding_row_group)
    }) {
        let mut group_first_rows = HashMap::<usize, Option<usize>>::new();
        // The last row seen in each row group: rows of a nested table are
        // visited between two rows of the outer group.
        let mut previous_rows = HashMap::<Option<usize>, usize>::new();
        for candidate in candidates.iter().filter(|candidate| candidate.is_table_row) {
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
/// margin, keyed by its first box.
fn avoided_run_ends(
    document: &Document,
    cascade: &CascadeResult,
    candidates: &[PageCandidate],
    body_id: usize,
    content_width: f32,
) -> HashMap<usize, f32> {
    let mut avoided_run_ends = HashMap::<usize, f32>::new();
    let mut previous: Option<&PageCandidate> = None;
    let mut run_start: Option<usize> = None;
    for candidate in candidates {
        if !(candidate.is_direct_body_element
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
        let eligible = candidate.is_direct_body_element
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
        if !eligible {
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
            avoided_run_ends.insert(start, candidate.raw_y + candidate.height + margin_bottom);
        } else {
            run_start = None;
        }
        previous = Some(candidate);
    }
    avoided_run_ends
}

// Pagination adjusts selected boxes after taffy has produced one normal
// flow layout.  Keep the arena's parent relation so a descendant can be
// materialized relative to an already-shifted ancestor instead of being
// shifted twice.
fn current_abs_y(document: &Document, node_id: usize) -> f32 {
    let mut id = document.ifc_source_owner(node_id);
    let mut y = if id != node_id {
        document
            .ifc_layout_node(node_id)
            .expect("valid anonymous root")
            .unrounded_layout
            .location
            .y
    } else {
        0.0
    };
    let mut guard = 0_usize;
    while guard <= document.nodes.len() {
        y += document.nodes[id].unrounded_layout.location.y;
        // A box of an inline engine paragraph is located from the root,
        // not from the inline elements around it.
        let Some(parent_id) = document
            .parent_of(id)
            .and_then(|_| document.layout_parent_of(id))
        else {
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
    let top = |id: usize| current_abs_y(document, id);
    let block_top = top(block);
    let mut moved = block;
    let mut child = block;
    while let Some(parent) = document.layout_parent_of(child)
        && parent != body_id
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
        child = parent;
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        let layout = &document.nodes[parent].unrounded_layout;
        if layout.border.top != 0.0
            || layout.padding.top != 0.0
            || (top(parent) - block_top).abs() > 0.01
        {
            break;
        }
        moved = parent;
    }
    moved
}

fn has_explicit_positioned_block_ancestor(document: &Document, node_id: usize) -> bool {
    let mut current = Some(document.ifc_source_owner(node_id));
    while let Some(id) = current {
        let inset = &document.nodes[id].style.inset;
        if matches!(
            document.nodes[id].position(),
            PositionValue::Absolute | PositionValue::Fixed
        ) && (!inset.top.is_auto() || !inset.bottom.is_auto())
        {
            return true;
        }
        current = document.parent_of(id);
    }
    false
}

fn materialize_y(document: &mut Document, node_id: usize, desired_y: f32) {
    if !desired_y.is_finite() {
        return; // cov:ignore: callers pass finite page positions.
    }
    let actual_y = current_abs_y(document, node_id);
    let delta = desired_y - actual_y;
    if delta.is_finite() {
        document
            .table_layout_node_mut(node_id)
            .unrounded_layout
            .location
            .y += delta;
        if document.is_source_node(node_id) {
            follow_moved_ifc_block(document, node_id, delta);
        }
    } // cov:ignore: only source boxes are materialized during pagination.
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
    let old_top = text_first_line_offset(document, root, owned, first.top);
    let Some(ifc) = document.table_layout_node_mut(root).ifc.as_mut() else {
        return; // cov:ignore: the paragraph root of an indexed text carries its inline layout.
    };
    let Some(lines) = ifc.lines.as_mut() else {
        return; // cov:ignore: the paragraph root of an indexed text carries its line records.
    };
    lines.shifts.push(first.line, delta);
    let boxes: Vec<usize> = ifc.boxes.iter().map(|b| b.node).collect();
    for b in boxes {
        let location = &mut document.nodes[b].unrounded_layout.location;
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
/// margin pulls up beside the block still follows it.
fn follow_moved_ifc_block(document: &mut Document, node_id: usize, delta: f32) {
    if delta == 0.0 || !delta.is_finite() || document.nodes[node_id].in_ifc_subtree() {
        return;
    }
    let Some(root) = document.layout_parent_of(node_id) else {
        return; // cov:ignore: a block child of an inline-engine paragraph has a layout parent.
    };
    let layout = document.nodes[node_id].unrounded_layout;
    let old_top = layout.location.y - delta;
    let old_bottom = old_top + layout.size.height;
    follow_ifc_block(document, root, node_id, old_top, old_bottom, delta);
}

/// The content after `block`, a block child of the paragraph `root` that
/// spanned `old_top..old_bottom`, moves down by a finite, nonzero
/// `delta`: the lines after
/// it and the paragraph's boxes after it in source order. A box inside
/// `block` that moves to a later page takes the rest of the paragraph
/// along this way even though `block` itself stays.
fn follow_ifc_block(
    document: &mut Document,
    root: usize,
    block: usize,
    old_top: f32,
    old_bottom: f32,
    delta: f32,
) {
    let Some(ifc) = document.table_layout_node_mut(root).ifc.as_mut() else {
        return;
    };
    let Some(lines) = ifc.lines.as_mut() else {
        return; // cov:ignore: a paragraph whose block child moves carries its line records.
    };
    let Some(&(_, first_after)) = lines
        .block_line_starts
        .iter()
        .find(|(child, _)| *child == block)
    else {
        return;
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
    for (b, kind, later) in boxes {
        let location = &mut document.nodes[b].unrounded_layout.location;
        let follows = if kind == IfcBoxKind::OutOfFlow {
            location.y >= old_bottom - 0.01
        } else {
            later && location.y >= old_top - 0.01
        };
        if follows {
            location.y += delta;
        }
    }
}

fn is_descendant_or_self(document: &Document, node_id: usize, ancestor_id: usize) -> bool {
    let mut current = Some(node_id);
    let mut guard = 0_usize;
    while let Some(id) = current {
        if id == ancestor_id {
            return true;
        }
        if !document.is_source_node(id) || guard > document.nodes.len() {
            break; // cov:ignore: defensive stop at a generated node or a parent cycle.
        }
        current = document.parent_of(id);
        guard += 1;
    }
    false
}

fn has_footnote_ancestor(cascade: &CascadeResult, node_id: usize, document: &Document) -> bool {
    let mut current = document.parent_of(node_id);
    let mut guard = 0_usize;
    while let Some(id) = current {
        if cascade
            .computed
            .get(id)
            .is_some_and(|computed| matches!(computed.float, FloatValue::Footnote))
        {
            return true;
        }
        current = document.parent_of(id);
        guard += 1;
        if guard > document.nodes.len() {
            break; // cov:ignore: parent links form a tree, so the guard never trips.
        }
    }
    false
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
/// `Ok(ResolvedIntrinsic { disposition: Fallback { .. } })` instead.
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
pub fn layout_single_page_with_resolver(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
) -> Result<(), LayoutError> {
    layout_single_page_with_resolver_and_base_url(document, cascade, page_box, resolver, None)
}

/// [`layout_single_page_with_resolver`] with document-relative image URLs.
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
fn has_in_flow_content(document: &Document, cascade: &CascadeResult, id: usize) -> bool {
    // On transposed axes the paragraph's width is its inline size, while the
    // generated boxes keep their physical styles.
    let vertical = document.pagination_on_transposed_axes;
    let has_paragraph_content = document.ifc_layout_node(id).is_some_and(|node| {
        let width = node.unrounded_layout.size.width;
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
                || node.unrounded_layout.size.height > 0.0
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
    document: &Document,
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
                    document.nodes[parent].unrounded_layout.size.height,
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
fn generated_box_separates(document: &Document, node_id: usize, previous: usize) -> bool {
    use raikiri_style::PseudoElem;

    let separates = |element: usize, pseudo: PseudoElem| {
        let id = crate::generated_content::generated_node_id(element, pseudo) as u64;
        let vertical = document.pagination_on_transposed_axes;
        BoxTree::for_layout(document)
            .parent(element, BoxParentScope::Document)
            .and_then(|parent| document.ifc_layout_node(parent))
            .is_some_and(|node| {
                let width = node.unrounded_layout.size.width;
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
    while let Some(parent) = document.parent_of(current) {
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

/// The largest positive margin among the block-start (`start`) or block-end
/// margin of `id` and the margins of its first or last in-flow child that
/// adjoin it, which all collapse into one (CSS 2 §8.3.1).
fn adjoining_margin(document: &Document, cascade: &CascadeResult, id: usize, start: bool) -> f32 {
    let layout = &document.nodes[id].unrounded_layout;
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
    let child_layout = &document.nodes[child].unrounded_layout;
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
    document: &Document,
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
    /// The border-box height the flow after the box follows, before and
    /// after the move. It is less than the used height when floats the box
    /// does not contain hang below its in-flow content.
    in_flow_heights: (f32, f32),
    /// Whether `max-height` keeps the box from holding its moved content.
    capped: bool,
}

/// The used border-box height of the `height: auto` box `parent` after its
/// child `moved` (already moved by `flow_delta`, or grown by `size_delta`
/// while the in-flow content inside it moved by `flow_delta`) and the
/// in-flow children after it shift down by `flow_delta`, clamped by
/// `min-height` and `max-height` (CSS 2.1 §10.7). `None` when the height is
/// not `auto`, so the ancestor keeps its size. `lines_moved` tells whether
/// the lines of `parent`'s paragraph after `moved` already carry the shift.
///
/// Layout also grows a box that does not hold its floats to the floats'
/// bottom (see [`propagate_float_bottoms_to_auto_height_ancestors`]), while
/// the flow after it follows only its in-flow content (CSS 2.1 §10.6.3).
/// Both edges are tracked, so the content moving inside a float that hangs
/// below it does not also push the flow after the box.
#[allow(clippy::too_many_arguments)]
fn grown_auto_height(
    document: &Document,
    cascade: &CascadeResult,
    parent: usize,
    moved: usize,
    flow_delta: f32,
    size_delta: f32,
    lines_moved: bool,
    body_id: usize,
) -> Option<GrownHeight> {
    let node = &document.nodes[parent];
    let style = &node.style;
    // A percentage height against an indefinite containing block behaves
    // as `auto` (CSS 2.1 §10.5).
    if !style.size.height.is_auto() && definite_content_height(document, parent).is_some() {
        return None;
    }
    let layout = &node.unrounded_layout;
    let vertical_edges =
        layout.padding.top + layout.padding.bottom + layout.border.top + layout.border.bottom;
    let box_sizing_edges = if style.box_sizing == taffy::BoxSizing::ContentBox {
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
    let bound = |value: taffy::LengthPercentageAuto, auto: f32| {
        resolve_length_percentage_auto(value, containing_height)
            .map_or(auto, |value| value + box_sizing_edges)
    };
    let min = bound(style.min_size.height, 0.0);
    let max = bound(style.max_size.height, f32::INFINITY).max(min);
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
    let mut shifted = false;
    let mut last = None;
    // The ends of `moved` and of the children after it, before and after
    // the shift, from their current `end`. `moved_by` is how far `moved`'s
    // end moved.
    let shift = |child: usize, end: f32, moved_by: f32, shifted: &mut bool| {
        if child == moved {
            *shifted = true;
            (end - moved_by, end)
        } else if *shifted && siblings_moved {
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
            || child_node.taffy_position() == taffy::Position::Absolute
        {
            continue;
        }
        let child_layout = &child_node.unrounded_layout;
        let top = pagination_child_parent_y(document, parent, child, child_layout.location.y);
        if is_floating_box_for_pagination(document, cascade, child) {
            if contains_floats {
                // A float's margin box ends the content.
                let ends = shift(
                    child,
                    top + child_layout.size.height + child_layout.margin.bottom,
                    flow_delta,
                    &mut shifted,
                );
                extend(&mut flow, ends);
                extend(&mut size, ends);
            } else if matches!(
                cascade.computed[parent].height,
                ComputedLengthPercentageOrAuto::Auto
            ) {
                // Layout grew the box to the float's border box and the
                // box's bottom border; the edges below are added back last.
                let ends = shift(
                    child,
                    top + child_layout.size.height + layout.border.bottom - edges_below,
                    flow_delta,
                    &mut shifted,
                );
                extend(&mut size, ends);
            }
            continue;
        }
        let relative_offset = layout_relative_offset_y(document, cascade, body_id, child);
        let size_end = top + child_layout.size.height - relative_offset;
        let size_ends = shift(child, size_end, size_delta, &mut shifted);
        // A box holding its floats holds those of its children too, which
        // their grown heights include.
        let flow_ends = if contains_floats {
            size_ends
        } else {
            let in_flow_height = document
                .in_flow_heights
                .get(&child)
                .map_or(child_layout.size.height, |&height| {
                    height.min(child_layout.size.height)
                });
            let end = top + in_flow_height - relative_offset;
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
    let old_in_flow = document
        .in_flow_heights
        .get(&parent)
        .map_or(old, |&height| height.min(old));
    let (in_flow_height, capped) = grow(old_in_flow, flow);
    shifted.then(|| GrownHeight {
        height: grow(old, size).0.max(in_flow_height),
        in_flow_heights: (old_in_flow, in_flow_height),
        capped,
    })
}

/// The content-box height of `block` when it is definite: a length, or a
/// percentage of a definite containing block, the initial containing block
/// being definite (CSS 2.1 §10.5).
fn definite_content_height(document: &Document, block: usize) -> Option<f32> {
    let node = &document.nodes[block];
    let raw = node.style.size.height.into_raw();
    let definite = if raw.is_plain_calc() || raw.tag() == taffy::CompactLength::PERCENT_TAG {
        BoxTree::for_layout(document)
            .parent(block, BoxParentScope::Document)
            .is_none_or(|containing| {
                document.nodes[containing].kind() == NodeKind::Document
                    || definite_content_height(document, containing).is_some()
            })
    } else {
        raw.tag() == taffy::CompactLength::LENGTH_TAG
    };
    definite.then(|| node.unrounded_layout.content_box_height())
}

/// Whether only margins separate `previous` from `node_id`: `previous` is
/// the in-flow box laid out just before `node_id` among its parent's boxes,
/// or every box between them takes no block space. Collapsed white space and
/// out-of-flow boxes are skipped. `display: contents` elements generate no
/// box, so their children stand in their place.
fn adjoins_previous_sibling(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    previous: usize,
) -> bool {
    let mut current = node_id;
    loop {
        let Some(parent) = document.parent_of(current) else {
            return false;
        };
        let siblings = &document.nodes[parent].children;
        let Some(index) = siblings.iter().position(|&id| id == current) else {
            return false;
        };
        let mut boxes = Vec::new();
        for &id in siblings[..index].iter().rev() {
            push_in_flow_boxes_rev(document, cascade, id, &mut boxes);
        }
        for found in boxes {
            if found == previous {
                return true;
            }
            if !takes_no_block_space(document, found) {
                return false;
            }
        }
        if document.nodes[parent].display() != DisplayValue::Contents {
            return false;
        }
        current = parent;
    }
}

/// Whether `node_id` is a block box that takes no block space, so that only
/// margins lie between the boxes on either side of it.
fn takes_no_block_space(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    let layout = &node.unrounded_layout;
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
    let node = &document.nodes[node_id];
    // White space that `white-space` preserves still generates lines.
    let collapse = cascade.computed[node_id].effective_white_space_collapse;
    let collapsed_text = node.kind() == NodeKind::Text
        && node.text_content().is_some_and(|text| {
            text.chars()
                .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
                && match collapse {
                    raikiri_style::property::WhiteSpaceCollapse::Collapse
                    | raikiri_style::property::WhiteSpaceCollapse::Discard => true,
                    raikiri_style::property::WhiteSpaceCollapse::PreserveBreaks => {
                        !text.contains(['\n', '\r'])
                    }
                    _ => text.is_empty(),
                }
        });
    // Comments and other non-element, non-text nodes generate no box.
    if !matches!(node.kind(), NodeKind::Element | NodeKind::Text)
        || collapsed_text
        || document.nodes[node_id].display() == DisplayValue::None
        || node.taffy_position() == taffy::Position::Absolute
        || is_floating_box_for_pagination(document, cascade, node_id)
    {
        return;
    }
    if node.kind() != NodeKind::Text && document.nodes[node_id].display() == DisplayValue::Contents
    {
        for &id in node.children.iter().rev() {
            push_in_flow_boxes_rev(document, cascade, id, boxes);
        }
        return;
    }
    boxes.push(node_id);
}

/// The footnote area at the bottom of each page, filled upward in source order.
#[derive(Default)]
struct FootnoteAreas {
    /// The height of the notes already stacked on each page.
    used: HashMap<u32, f32>,
}

impl FootnoteAreas {
    /// Stacks a note of `height` on `page`, whose box spans `page_start` to
    /// `page_end`, and returns the note's top: just above the notes already
    /// there, but not above the page start.
    fn stack(&mut self, page: u32, page_start: f32, page_end: f32, height: f32) -> f32 {
        let used = self.used.entry(page).or_insert(0.0);
        let top = (page_end - *used - height).max(page_start);
        *used += height;
        top
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

/// Whether a box whose margin box starts at `margin_box_y` falls on a page
/// after `current_page`, or at `page_start`, the start of `current_page`.
///
/// This matters when the page height changes after page zero: the
/// fixed-height pass may have considered the box to be on the current page,
/// while the scheduled pass places it exactly at the next page origin. A
/// border box that lands exactly on a page origin only because the preceding
/// collapsed margin filled the previous page has not opened that page: the
/// break still moves it, and the margin after the forced break is kept.
fn margin_box_starts_page(
    page_origins: &PageOrigins,
    margin_box_y: f32,
    current_page: u32,
    page_start: f32,
) -> bool {
    let margin_box_page = page_origins
        .page_index_in_flow(margin_box_y)
        .unwrap_or(current_page);
    margin_box_page > current_page
        || (margin_box_y.is_finite() && (margin_box_y - page_start).abs() <= 0.001)
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
                        .is_some_and(|line| line.line >= first_moved);
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
        let mut current = document.parent_of(node_id);
        while let Some(id) = current {
            if document.nodes[id].is_ifc_root() && !self.roots.insert(id) {
                first = false;
                // Higher IFC roots were registered when this root was first entered.
                break;
            }
            current = document.parent_of(id);
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
    document: &Document,
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
                (!candidate.is_float_descendant
                    && !candidate.is_out_of_flow_descendant
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
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    candidate: &PageCandidate,
    paragraph_root: Option<usize>,
    uses_positioned_coordinates: bool,
) -> Option<(usize, usize)> {
    paragraph_root
        .filter(|_| {
            !uses_positioned_coordinates
                && !candidate.is_float_descendant
                && !candidate.is_out_of_flow_descendant
        })
        .and_then(|root| {
            flex_container_of_item_alone_on_its_line(document, cascade, root, body_id)
                .map(|container| (root, container))
        })
}

/// Whether the text from `text_y` with `text_height` starts at or below the
/// start of the page that starts at `page_start` and is `page_step` tall and
/// ends past that page's end, while its block, from `block_y` with
/// `block_height`, starts on that page and is no taller than it. Such a fixed-height block is pushed to the next page as a
/// unit, as monolithic content is.
fn first_line_overflows(
    text_y: f32,
    text_height: f32,
    block_y: f32,
    block_height: f32,
    page_start: f32,
    page_step: f32,
) -> bool {
    let page_end = page_start + page_step;
    text_y.is_finite()
        && text_height.is_finite()
        && text_y >= page_start
        && block_y >= page_start
        && block_y < page_end
        && block_height.is_finite()
        && block_height <= page_step + 0.001
        && text_y + text_height > page_end
}

/// Whether later lines of the paragraph `block_id` can move down inside it:
/// its height grows with them (a box with a definite height keeps it), and it
/// holds nothing but text in one column.
fn lines_can_shift_inside(document: &Document, block_id: usize) -> bool {
    height_follows_content(document, block_id)
        && document
            .ifc_layout_node(block_id)
            .and_then(|node| node.ifc.as_ref())
            .is_some_and(|ifc| ifc.boxes.is_empty() && ifc.multicol_fragments.is_none())
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
        let widows = widows.max(1) as usize;
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
}

impl SiblingMargins {
    /// Whether the margin between `previous` and `node_id` still adjoins
    /// the latest forced break.
    fn follows_forced_break(
        &self,
        document: &Document,
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

    /// Moves the forced break on to `node_id`, a child of `parent_id`, or
    /// ends it. An empty box right after the forced-break box passes the
    /// break on to the margin after it when its margins collapse through
    /// with that margin (`collapses_through`); any other box ends it.
    fn pass_on_forced_break(
        &mut self,
        document: &Document,
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
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    content_width: f32,
) -> (f32, f32) {
    let style = &document.nodes[node_id].style;
    let computed = &cascade.computed[node_id];
    let margin_top = used_style_length_percentage_auto(style.margin.top, content_width)
        .or_else(|| used_computed_length_percentage_or_auto(computed.margin.top, content_width))
        .unwrap_or(0.0);
    let trailing_margin = if is_floating_box_for_pagination(document, cascade, node_id) {
        used_style_length_percentage_auto(style.margin.bottom, content_width)
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

/// Moves `block_id`, which sits at `from_y`, so that its flow position starts
/// at `target_y`, and returns how far it moved. Its relative offsets move
/// along with it. A `nested` block that begins its ancestors, with no border
/// or padding between them, takes the ancestors along rather than leaving
/// them empty on this page; the ancestors above them grow to hold the move,
/// recorded in `shifts` with `flow_page` as for [`grow_ancestors_for_shift`].
#[allow(clippy::too_many_arguments)]
fn move_line_block(
    document: &mut Document,
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
        materialize_y(document, moved, moved_y);
        grow_ancestors_for_shift(document, cascade, moved, delta, body_id, shifts, flow_page);
    }
    delta
}

/// Grows the formatting context that holds a moved float. A formatting
/// context of its own holds its floats (CSS 2.1 §10.6.7), so an auto-height
/// one grows to the moved float's margin box, and the flow after it moves
/// down by the growth.
fn grow_float_context_to_moved_float(
    document: &mut Document,
    cascade: &CascadeResult,
    node_id: usize,
    body_id: usize,
    height: f32,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
) {
    let context = float_context_root(document, cascade, node_id, body_id);
    let context_style = &document.nodes[context].style;
    if context != body_id
        && (context_style.size.height.is_auto()
            || definite_content_height(document, context).is_none())
    {
        let context_top = current_abs_y(document, context);
        let float_bottom = current_abs_y(document, node_id)
            + height
            + document.nodes[node_id].unrounded_layout.margin.bottom;
        let layout = &document.nodes[context].unrounded_layout;
        let grown = (float_bottom - context_top + layout.padding.bottom + layout.border.bottom)
            .min(max_border_box_height(document, cascade, context).unwrap_or(f32::INFINITY));
        let growth = grown - layout.size.height;
        if growth > 0.001 {
            document.nodes[context].unrounded_layout.size.height += growth;
            grow_ancestors_for_shift(
                document, cascade, context, growth, body_id, shifts, flow_page,
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
fn grow_ancestors_for_shift(
    document: &mut Document,
    cascade: &CascadeResult,
    node_id: usize,
    delta: f32,
    body_id: usize,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
) -> bool {
    let mut bounded = false;
    let mut child = node_id;
    // How far the flow after `child` moved, and how much `child` moved or
    // grew: a float hanging below its content can take up part of the move.
    let mut flow_delta = delta;
    let mut size_delta = delta;
    let mut next = document.parent_of(child);
    while let Some(parent) = next {
        if document.nodes[parent].kind() != NodeKind::Element {
            break;
        }
        next = document.parent_of(parent);
        // A `display: contents` element has no box to grow; its parent lays
        // the moved box out.
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        shift_later_block_siblings(document, parent, child, flow_delta);
        let height = document.nodes[parent].unrounded_layout.size.height;
        let grown = grown_auto_height(
            document,
            cascade,
            parent,
            child,
            flow_delta,
            size_delta,
            child == node_id,
            body_id,
        );
        let Some(grown) = grown else {
            shifts.bounded.push((parent, flow_delta, flow_page));
            bounded = true;
            break;
        };
        let (old_in_flow, in_flow) = grown.in_flow_heights;
        let limit = flow_delta.max(size_delta);
        let size_absorbed = (grown.height - height).clamp(0.0, limit);
        let flow_absorbed = (in_flow - old_in_flow).clamp(0.0, limit);
        if flow_absorbed < flow_delta - 0.001 {
            shifts
                .bounded
                .push((parent, flow_delta - flow_absorbed, flow_page));
            bounded |= grown.capped;
        } else if flow_absorbed > flow_delta + 0.001 {
            // A box holding its floats grew around a float that moved with
            // the content it overflowed: the flow after the box follows.
            shifts.growth.push((parent, flow_absorbed - flow_delta));
        }
        if flow_absorbed <= 0.001 && size_absorbed <= 0.001 {
            break;
        }
        let height = height + size_absorbed;
        let in_flow = old_in_flow + flow_absorbed;
        if in_flow < height - 0.001 {
            document.in_flow_heights.insert(parent, in_flow);
        } else {
            document.in_flow_heights.remove(&parent);
        }
        document.nodes[parent].unrounded_layout.size.height = height;
        flow_delta = flow_absorbed;
        size_delta = size_absorbed;
        child = parent;
        if parent == body_id {
            break;
        }
    }
    bounded
}

/// A truncated margin shortens the auto-height boxes containing the flow.
/// Reuse the same used-child extents and size bounds as ordinary pagination
/// growth, retaining unrelated float extents and authored height limits.
fn shrink_ancestors_after_margin_truncation(
    document: &mut Document,
    cascade: &CascadeResult,
    node: usize,
    delta: f32,
    flow_root: usize,
) {
    if delta >= 0.0 || !delta.is_finite() {
        return;
    }
    let mut child = node;
    let mut flow_delta = delta;
    let mut size_delta = delta;
    let mut next = document.layout_parent_of(child);
    while let Some(parent) = next {
        next = document.layout_parent_of(parent);
        if document.nodes[parent].kind() != NodeKind::Element
            || document.nodes[parent].size_containment.height
        {
            break;
        }
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        shift_later_block_siblings(document, parent, child, flow_delta);
        let old_height = document.nodes[parent].unrounded_layout.size.height;
        let Some(adjusted) = grown_auto_height(
            document,
            cascade,
            parent,
            child,
            flow_delta,
            size_delta,
            child == node,
            flow_root,
        ) else {
            break;
        };
        document.nodes[parent].unrounded_layout.size.height = adjusted.height;
        let (old_flow, new_flow) = adjusted.in_flow_heights;
        if document.in_flow_heights.contains_key(&parent)
            || (adjusted.height - new_flow).abs() > 0.001
        {
            document.in_flow_heights.insert(parent, new_flow);
        }
        size_delta = adjusted.height - old_height;
        flow_delta = new_flow - old_flow;
        child = parent;
        if parent == flow_root {
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
    document: &Document,
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
    document: &Document,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
) -> bool {
    let mut child = block;
    loop {
        let placed_apart = is_floating_box_for_pagination(document, cascade, child)
            || matches!(
                document.nodes[child].position(),
                PositionValue::Absolute | PositionValue::Fixed
            );
        match document.layout_parent_of(child) {
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
    document: &Document,
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
    let layout = document.nodes[item].unrounded_layout;
    let outer_width = layout.size.width + layout.margin.left + layout.margin.right;
    let line = document.nodes[container].unrounded_layout;
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
            let other = document.nodes[child].unrounded_layout;
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
    let layout = document.nodes[item].unrounded_layout;
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
                && document.nodes[child].unrounded_layout.location.y >= old_bottom - 0.001
        })
        .collect();
    for child in later {
        document.nodes[child].unrounded_layout.location.y += shift;
    }
}

/// Whether the height of `block` is that of its content: no `height`,
/// `max-height` or non-zero `min-height` sets it and no size containment
/// ignores the content.
fn height_follows_content(document: &Document, block: usize) -> bool {
    let node = &document.nodes[block];
    let min_height = node.style.min_size.height;
    node.style.size.height.is_auto()
        && (min_height.is_auto() || style_dimension_length(min_height.into()) == Some(0.0))
        && node.style.max_size.height.is_auto()
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
        document
            .table_layout_node_mut(sibling)
            .unrounded_layout
            .location
            .y += delta;
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
    let layout = &document.nodes[node_id].unrounded_layout;
    let outer_bottom = flow_target_y + height + layout.margin.bottom;
    let outer_top = if forced_transition {
        flow_target_y - margin_top
    } else {
        flow_target_y
    };
    FloatEdge {
        node: node_id,
        side,
        top: outer_top,
        bottom: outer_bottom,
        width: layout.size.width + layout.margin.left + layout.margin.right,
    }
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

impl FloatShifts {
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
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        body_id: usize,
    ) -> Option<(usize, Vec<FloatEdge>)> {
        (self.later != 0.0 && self.pending.is_none())
            .then(|| {
                let context = float_context_root(document, cascade, node_id, body_id);
                self.edges
                    .get(&context)
                    .map(|edges| (context, edges.clone()))
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
}

/// The box whose block formatting context places the floats and clearance
/// of `node_id`: its nearest ancestor that establishes an independent one
/// (CSS 2.1 §9.4.1), or `body_id`. Floats in different contexts do not
/// interact.
fn float_context_root(
    document: &Document,
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
    std::iter::successors(Some(node_id), |&id| document.parent_of(id))
        .find(|&id| {
            let display = document.nodes[id].display();
            document.nodes[id].is_ifc_root()
                || !(display.is_inline_flow() || display == DisplayValue::Contents)
        })
        .is_some_and(|id| document.nodes[id].is_ifc_root())
}

/// Whether the box `node_id` establishes an independent block formatting
/// context (CSS 2.1 §9.4.1), which holds the floats inside it.
fn holds_floats(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    let computed = &cascade.computed[node_id];
    let style = &document.nodes[node_id].style;
    let item = BoxTree::for_layout(document)
        .parent(node_id, BoxParentScope::Document)
        .is_some_and(|container| {
            document.nodes[container]
                .formatting_context
                .is_flex_or_grid()
        });
    document.nodes[node_id].taffy_display() == Display::FlowRoot
        || item
        || !(document.nodes[node_id].display().is_inline_flow()
            || matches!(
                document.nodes[node_id].display(),
                DisplayValue::Block | DisplayValue::ListItem
            ))
        || matches!(computed.float, FloatValue::Left | FloatValue::Right)
        || document.nodes[node_id].taffy_position() == taffy::Position::Absolute
        || style.overflow.x.is_scroll_container()
        || style.overflow.y.is_scroll_container()
        || document.nodes[node_id].multicol.is_some()
        || establishes_containment_context(computed)
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
    document: &Document,
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
    let node = &document.nodes[node_id];
    let layout = &node.unrounded_layout;
    let edges = if node.style.box_sizing == taffy::BoxSizing::ContentBox {
        layout.padding.top + layout.padding.bottom + layout.border.top + layout.border.bottom
    } else {
        0.0
    };
    Some(max + edges)
}

/// Whether the `max-height` of `node_id` already limits its height.
fn max_height_is_reached(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    max_border_box_height(document, cascade, node_id)
        .is_some_and(|max| document.nodes[node_id].unrounded_layout.size.height >= max - 0.01)
}

/// Whether an ancestor of `node_id` inside the body clips its overflow in
/// the block axis and has a definite height, or one its `max-height`
/// already limits, so it does not grow with a moved descendant.
fn has_bounded_clipping_ancestor(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    body_id: usize,
) -> bool {
    let mut current = document.parent_of(node_id);
    while let Some(id) = current {
        if id == body_id {
            break;
        }
        let computed = &cascade.computed[id];
        if !matches!(computed.overflow.y, OverflowValue::Visible)
            && crate::paint_rules::clips_element_overflow(document, cascade, id)
            && (definite_content_height(document, id).is_some()
                || max_height_is_reached(document, cascade, id))
        {
            return true;
        }
        current = document.parent_of(id);
    }
    false
}

/// Whether the items of `grid` stack in block order like blocks, none
/// beside or over another: the grid has one column track, no definite
/// height and no `min-height` that could stretch its rows, its rows all
/// size to their content, and its items sit in rows that follow one
/// another without overlapping. A sized row fixes the height of the item
/// stretched into it, so that item cannot grow to hold a moved box.
fn grid_stacks_items(document: &Document, grid: usize) -> bool {
    let node = &document.nodes[grid];
    if node.grid_column_count != 1
        || !grid_rows_size_to_content(document, grid)
        || !node.style.size.height.is_auto() && definite_content_height(document, grid).is_some()
        || !node.style.min_size.height.is_auto()
    {
        return false;
    }
    let mut items: Vec<(u16, f32, f32)> = node
        .grid_item_row_starts
        .iter()
        .map(|&(item, row)| {
            let layout = &document.nodes[item].unrounded_layout;
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
fn grid_rows_size_to_content(document: &Document, grid: usize) -> bool {
    let style = &document.nodes[grid].style;
    // `auto`, `min-content` and `max-content` grow with the content;
    // `fit-content()` caps it.
    let auto = |track: &taffy::TrackSizingFunction| {
        track.min.is_intrinsic()
            && (track.max.is_auto() || track.max.is_min_content() || track.max.is_max_content())
    };
    style
        .grid_template_rows
        .iter()
        .all(|component| match component {
            taffy::GridTemplateComponent::Single(track) => auto(track),
            taffy::GridTemplateComponent::Repeat(repetition) => repetition.tracks.iter().all(auto),
        })
        && style.grid_auto_rows.iter().all(auto)
}

/// Whether a flex or multi-column container, or a grid whose items do not
/// stack in block order, contains `node_id`, up to and including `body_id`: moving such a
/// box would also displace the boxes beside it.
fn has_side_by_side_ancestor(document: &Document, node_id: usize, body_id: usize) -> bool {
    let mut current = document.parent_of(node_id);
    while let Some(id) = current {
        let formatting_context = document.nodes[id].formatting_context;
        if document.nodes[id].multicol.is_some()
            || formatting_context == FormattingContext::Flex
            || formatting_context == FormattingContext::Grid && !grid_stacks_items(document, id)
        {
            return true;
        }
        if id == body_id {
            break;
        }
        current = document.parent_of(id);
    }
    false
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
fn stacks_children_in_block_flow(document: &Document, id: usize) -> bool {
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
    !candidate.is_float_descendant
        && !candidate.is_out_of_flow_descendant
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
        && !candidate.is_text
        && !candidate.is_table_row
        && !candidate.is_flex_item
        && !candidate.is_grid_item
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
    document: &mut Document,
    node_id: usize,
    body_id: usize,
    flow_delta: f32,
) {
    let mut child = node_id;
    let mut next = document.parent_of(node_id);
    while let Some(parent) = next
        && parent != body_id
    {
        next = document.parent_of(parent);
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        let paragraph = ifc_paragraph_of_block(document, child) == Some(parent);
        if !paragraph && !stacks_children_in_block_flow(document, parent) {
            break;
        }
        // The moved box itself already took the lines after it along.
        if paragraph && child != node_id {
            let layout = document.nodes[child].unrounded_layout;
            let top = layout.location.y;
            follow_ifc_block(
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
