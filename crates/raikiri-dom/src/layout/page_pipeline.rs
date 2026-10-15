pub(crate) mod column_projection;
mod sibling_shifts;
use column_projection::{ParagraphCache, ParagraphProjection, ProjectionWork};

use super::ifc::boxes::IfcBoxKind;
use super::ifc::text_lines::{IfcTextLine, TextLineIndex};
use super::*;
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
    if std::iter::once(body_id)
        .chain(document.parent_of(body_id))
        .any(|id| !matches!(cascade.computed[id].overflow.y, OverflowValue::Visible))
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
        if matches!(
            computed.position,
            PositionValue::Fixed | PositionValue::Running(_)
        ) {
            continue;
        }
        // Inside columns only an absolutely positioned box placed against a
        // containing block outside them is measured, with its subtree.
        // A `display: contents` element generates no positioned box, so its
        // children stay in the column flow.
        let positioned = computed.position == PositionValue::Absolute
            && computed.display != DisplayValue::Contents;
        let in_columns = match in_columns {
            Some(true) if positioned => None,
            Some(false) if positioned => continue,
            other => other,
        };
        // Layout places some boxes without their relative offset, which
        // the painter adds to the box and its subtree.
        let paint_offset = painted_relative_offset_y(document, cascade, body_id, node_id);
        let y = if node_id == body_id {
            paint_offset
        } else {
            parent_y + node.unrounded_layout.location.y + paint_offset
        };
        // A `display: contents` body has no box of its own to measure.
        let bottom = if node_id == body_id && computed.display == DisplayValue::Contents {
            f32::NEG_INFINITY
        } else if node_id == body_id {
            y + body_border_box_end(
                document,
                cascade,
                body_id,
                root_flow_offset,
                page_area_height,
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
    let computed = &cascade.computed[node_id];
    // A `display: contents` element generates no box to offset.
    if !matches!(
        computed.position,
        PositionValue::Relative | PositionValue::Sticky
    ) || matches!(
        computed.display,
        DisplayValue::Contents | DisplayValue::None
    ) || relative_offset_is_painted(document, cascade, body_id, node_id)
        // Layout places a sticky float without its offset.
        || (computed.position == PositionValue::Sticky
            && is_floating_box_for_pagination(document, cascade, node_id))
    {
        return 0.0;
    }
    // Percentages resolve against the containing block's height when it
    // is definite, and otherwise act as `auto` (CSS 2.1 §9.3.2). Layout
    // takes `top` over `-bottom` in every writing mode.
    super::relative::used_relative_insets(
        document,
        cascade,
        computed,
        node_id,
        super::relative::Precedence::LeftTop,
    )
    .1
    .unwrap_or(0.0)
}

/// The relative offsets that layout applied to `node_id` and to its
/// ancestors inside the body. They all moved the box away from its
/// normal-flow position.
fn accumulated_relative_offset_y(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> f32 {
    std::iter::successors(Some(node_id), |&id| document.parent_of(id))
        .take_while(|&id| id != body_id)
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
    super::relative::used_relative_offset(document, cascade, &cascade.computed[node_id], node_id).1
}

/// Whether layout leaves the relative offset of `node_id` to the painter.
fn relative_offset_is_painted(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> bool {
    let computed = &cascade.computed[node_id];
    if !matches!(computed.position, PositionValue::Relative) {
        return false;
    }
    let float = matches!(computed.float, FloatValue::Left | FloatValue::Right)
        && document
            .layout_parent_of(node_id)
            .is_none_or(|parent| !cascade.computed[parent].display.is_flex_or_grid_container());
    let inline_level = computed.display.is_inline_level();
    let table_part = matches!(
        computed.display,
        DisplayValue::TableCaption
            | DisplayValue::TableCell
            | DisplayValue::TableRow
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
    );
    ((node_id == body_id || float || table_part) && !inline_level)
        || document.nodes[node_id].needs_relative_block_paint_offset()
}

/// The block-end edge of the body's border box in page coordinates.
///
/// Layout sizes the body to the page content box, so its laid-out height
/// says nothing about the height the author gave it. A definite `height`,
/// clamped by `max-height` and `min-height`, gives the content height. An
/// auto-height body ends with its last in-flow child, followed by
/// its own block-end padding and border. The body's border box starts after
/// the root's margin (`root_flow_offset`) and its own block-start margin.
fn body_border_box_end(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    root_flow_offset: f32,
    page_area_height: f32,
) -> f32 {
    let computed = &cascade.computed[body_id];
    // Percentages resolve against the root's used height only when it is
    // definite; otherwise they behave as `auto`. The root's own percentages
    // resolve against the initial containing block, the page area.
    let root_height = document.parent_of(body_id).and_then(|root| {
        let definite = |value| used_computed_length_percentage_or_auto(value, page_area_height);
        let root_computed = &cascade.computed[root];
        let height = definite(root_computed.height)?;
        let height = definite(root_computed.max_height).map_or(height, |max| height.min(max));
        let height = definite(root_computed.min_height).map_or(height, |min| height.max(min));
        // The basis is the root's content box.
        let edges = if matches!(
            root_computed.box_sizing,
            raikiri_style::property::BoxSizing::BorderBox
        ) {
            let layout = &document.nodes[root].unrounded_layout;
            layout.padding.top + layout.padding.bottom + layout.border.top + layout.border.bottom
        } else {
            0.0
        };
        Some((height - edges).max(0.0))
    });
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
    let body = &document.nodes[body_id];
    let layout = &body.unrounded_layout;
    let edges_top = layout.padding.top + layout.border.top;
    let edges_bottom = layout.padding.bottom + layout.border.bottom;
    let top = root_flow_offset + document.body_block_start_margin;
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
    // A definite height, even zero, fixes the border box; content
    // overflowing it is measured on its own.
    if let Some(height) = used(computed.height) {
        let height = used(computed.max_height).map_or(height, |max| height.min(max));
        return border_box_end(height.max(min_height));
    }
    let min_end = if min_height > 0.0 {
        border_box_end(min_height)
    } else {
        f32::NEG_INFINITY
    };
    // Without block-end edges or a `min-height` an auto-height body ends
    // with its content, which the flow already paginated. Text of the body's
    // own paragraph is located by its lines, which are not measured here.
    if (edges_bottom <= 0.0 && min_height <= 0.0) || body.is_ifc_root() {
        return min_end;
    }
    // In-flow children are in page coordinates already, with the margins
    // between them collapsed, so the content ends at the block-end margin
    // edge of the last one; block-end edges or a `min-height` keep that
    // margin, negative or not, inside the body. A body that establishes a
    // formatting context of its own also encloses its floats.
    // The children of a `display: contents` element take part as the body's
    // own, and a relative offset moves a box without moving the flow.
    let encloses_floats = computed.display == DisplayValue::FlowRoot;
    let body_content_height = (layout.size.height - edges_top - edges_bottom).max(0.0);
    let mut content_end = top + edges_top;
    let mut last_flow_end = None;
    // Popped in document order.
    let mut children: Vec<(usize, f32)> = body
        .children
        .iter()
        .rev()
        .map(|&child| (child, 0.0))
        .collect();
    while let Some((child, offset)) = children.pop() {
        let node = &document.nodes[child];
        let child_computed = &cascade.computed[child];
        if !node.is_in_document() || node.is_display_none() || node.kind() != NodeKind::Element {
            continue;
        }
        let layout = &node.unrounded_layout;
        if child_computed.display == DisplayValue::Contents {
            let offset = offset + layout.location.y;
            children.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|&grandchild| (grandchild, offset)),
            );
            continue;
        }
        if !matches!(
            child_computed.position,
            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
        ) {
            continue;
        }
        if super::page::is_floating_box_for_pagination(document, cascade, child) {
            // The painter adds a float's relative offset; its layout box is
            // where the float sits in the flow.
            let end = offset + layout.location.y + layout.size.height + layout.margin.bottom;
            if encloses_floats && end.is_finite() {
                content_end = content_end.max(end);
            }
            continue;
        }
        // Layout resolves the insets of relative and sticky boxes alike,
        // against the body's content box.
        let relative_shift = if matches!(
            child_computed.position,
            PositionValue::Relative | PositionValue::Sticky
        ) {
            let inset = |value| used_computed_length_percentage_or_auto(value, body_content_height);
            inset(child_computed.top)
                .or_else(|| inset(child_computed.bottom).map(|bottom| -bottom))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let end =
            offset + layout.location.y - relative_shift + layout.size.height + layout.margin.bottom;
        if end.is_finite() {
            last_flow_end = Some(end);
        }
    }
    if let Some(end) = last_flow_end {
        content_end = content_end.max(end);
    }
    // `max-height` caps the auto height, measured as `box-sizing` says;
    // `min-height` still wins over it.
    let content_height = content_end - (top + edges_top);
    let max_end = used(computed.max_height).map_or(f32::INFINITY, border_box_end);
    let auto_end = top + edges_top + content_height + edges_bottom;
    min_end.max(auto_end.min(max_end))
}

// Cache the nearest column context for each visited source node. Row flex
// suppresses local named-flow ownership, but its content still occupies the
// enclosing physical page. Each source edge is resolved once per layout pass.
fn enclosing_column_context(
    node_id: usize,
    document: &Document,
    cascade: &CascadeResult,
    cache: &mut [Option<Option<usize>>],
) -> Option<usize> {
    let mut current = node_id;
    let mut visited = Vec::new();
    let context = loop {
        if let Some(cached) = cache[current] {
            break cached;
        }
        visited.push(current);
        let Some(parent) = document.parent_of(current) else {
            break None;
        };
        let computed = &cascade.computed[parent];
        if computed.display.is_flex_container()
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
        cache[node] = Some(context);
    }
    context
}

fn column_context_occupied(
    context: usize,
    page: u32,
    occupied: &HashMap<usize, u32>,
    document: &Document,
    cascade: &CascadeResult,
    cache: &mut [Option<Option<usize>>],
) -> bool {
    let mut current = Some(context);
    while let Some(id) = current {
        if occupied.get(&id) == Some(&page) {
            return true;
        }
        current = enclosing_column_context(id, document, cascade, cache);
    }
    false
}

fn occupy_column_contexts(
    mut context: Option<usize>,
    page: u32,
    occupied: &mut HashMap<usize, u32>,
    document: &Document,
    cascade: &CascadeResult,
    cache: &mut [Option<Option<usize>>],
) {
    while let Some(id) = context {
        if occupied.insert(id, page) == Some(page) {
            break;
        }
        context = enclosing_column_context(id, document, cascade, cache);
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
            child.position,
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
        let parent = &cascade.computed[parent_id];
        if !parent.display.is_grid_container()
            || !matches!(child.grid_row_start, GridLineValue::Auto)
            || !matches!(child.grid_row_end, GridLineValue::Auto)
            || !matches!(child.grid_column_start, GridLineValue::Auto)
            || !matches!(child.grid_column_end, GridLineValue::Auto)
        {
            continue;
        }
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
/// - Set body.style.size to the page content box with `apply_page_content_box_to_body`
/// - Run taffy with the body as the root (`compute_body_root_layout`), store
///   results in Node.unrounded_layout and move the body's content below its
///   used block-start margin
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
        if let Some(values) = document.clearance_page_values.clone() {
            let mut named = cascade.clone();
            named.page_values = values;
            clear_floats_at_forced_breaks(document, &named);
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
    crate::layout::ifc::assign::prepare_legacy_inside_markers_before_taffy(document, cascade)?;
    // Step 3: <body> lookup
    let body_id = find_body(document).ok_or_else(|| LayoutError::Internal {
        message: "no <body> element found (fragment parse not supported yet)".to_string(),
    })?;
    let body_insets = BodyInlineInsets::resolve(document, cascade, body_id, content_width);

    // Establish the foundational multicolumn fragmentainer projection after
    // text shaping, so direct text can be split by its actual line count.
    // Boxes with an `auto` width chain up to the body take the body's content
    // width, which its horizontal margins, padding and borders narrow.
    // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
    prepare_multicol_layout(document, cascade, body_insets.content_width(content_width));

    // The body is the taffy root, which ignores its margins. Its block-start
    // margin is resolved here and placed after layout (or carried as top
    // padding, see below), where it can collapse with its first child's;
    // the bottom margin has no effect on a root sized to the page content
    // box. The horizontal margins, UA or authored, become inline padding of
    // the root after Step 4 (`move_body_inline_margins_into_padding`).
    let body_margin_top = body_margin_top(document, cascade, body_id, content_width);
    {
        let style_margin = &mut document.nodes[body_id].style.margin;
        style_margin.top = LengthPercentageAuto::length(0.0);
        style_margin.bottom = LengthPercentageAuto::length(0.0);
    }
    propagate_body_overflow_to_viewport(document, cascade, body_id);
    let body_top_may_collapse = body_top_margin_may_collapse(document, cascade, body_id);

    // Step 4: force body.style.size to the page content box, not the full paper size.
    // Page margins are painted/represented outside this taffy root.
    apply_page_content_box_to_body(document, body_id, page_box, margins, insets);
    move_body_inline_margins_into_padding(document, body_id, body_insets, content_width);
    // A body whose content is one inline-engine paragraph has line boxes that
    // keep its top margin from collapsing; carry it as top padding so the
    // lines and the boxes among them start below it.
    let body_is_paragraph = document.nodes[body_id].is_ifc_root();
    let body_top_padding = BodyTopPadding::capture(document, body_id, content_width);
    let mut body_padded_top = if body_is_paragraph {
        body_top_padding.apply(document, body_id, body_margin_top)
    } else {
        0.0
    };
    // CSS 2.1 §10.3.7 absolute width:auto shrink-to-fit needs no pre-pass:
    // taffy already shrink-wraps direct-body and nested absolute boxes alike.
    // See the §10.3.7 note on the layout helpers for the removed fill override.

    // Fixed boxes laid out by the inline engine shrink against the page area.
    if let Some(state) = document.ifc.as_mut() {
        state.page_width = Some(content_width);
    }
    // Step 5: taffy compute
    let available_space = taffy::Size {
        width: AvailableSpace::Definite(content_width),
        height: AvailableSpace::Definite(content_height),
    };
    let escaped_top_margin = compute_body_root_layout(
        document,
        TaffyNodeId::from(body_id),
        available_space,
        body_top_may_collapse,
    );
    // A block box that starts the body's paragraph was placed by its own
    // top margins inside the paragraph; those collapse with the body's. Lay
    // the body out again with only the part of the collapsed margin that
    // they leave over as top padding. Only the body's own box changes, so
    // the boxes inside keep their cached layouts.
    let mut paragraph_collapsed_top = None;
    if body_is_paragraph && let Some(leading) = body_paragraph_leading_margin(document, body_id) {
        let collapsed = leading.collapse_with_margin(body_margin_top).resolve();
        let padding = collapsed - leading.resolve();
        if (padding.max(0.0) - body_padded_top).abs() > 1e-4 {
            body_padded_top = body_top_padding.apply(document, body_id, padding);
            document.nodes[body_id].cache.clear();
            compute_body_root_layout(document, TaffyNodeId::from(body_id), available_space, false);
        }
        // A negative remainder is clamped, as padding cannot be negative.
        paragraph_collapsed_top = Some(body_padded_top + leading.resolve());
    }
    // CSS 2.1 §8.3.1: the body's top margin and the top margins that escaped
    // its first in-flow children collapse into one, the largest positive
    // margin plus the most negative one. When nothing escaped (top padding
    // or border, a new formatting context, no in-flow child) this is the
    // body's own margin. Move the content below it.
    document.body_block_start_margin = if body_is_paragraph {
        paragraph_collapsed_top.unwrap_or(body_padded_top)
    } else {
        let collapsed = escaped_top_margin
            .collapse_with_margin(body_margin_top)
            .resolve();
        shift_body_content(document, body_id, collapsed);
        collapsed
    };
    take_layout_failure(document)?;
    // Step 5a: post-layout corrections taffy does not make: the static
    // position of auto-placed grid abspos items, the containing block of
    // nested fixed boxes, and auto-height ancestors of floats.
    // Taffy places fixed boxes against their parents, not the page area,
    // their containing block.
    // The body is sized to the page area, but a content-box body with
    // borders or padding outgrows it.
    let page_area = taffy::Size {
        width: content_width,
        height: content_height,
    };
    document.fixed_page_area = (content_width, content_height);
    place_fixed_boxes_in_page_area(document, cascade, body_id, page_area);
    // Laying those boxes out again can fail a table or exhaust the
    // fragment budget like the root layout above.
    take_layout_failure(document)?;
    // After the fixed boxes, whose relayout lays out the grids inside them
    // again with taffy's static positions.
    realign_grid_abspos_static_positions(document, cascade);
    propagate_float_bottoms_to_auto_height_ancestors(document, cascade);
    refresh_projected_multicol_text_fragments(document);
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
        let (width_ch, height_ch) =
            match crate::layout::bridge::vertical_logical_size(doc, cascade, idx) {
                Some(size) => (&size.width_ch, &size.height_ch),
                None => (&cv.width_ch, &cv.height_ch),
            };
        let width = measure(width_ch);
        let height = measure(height_ch);
        let min_width = measure(&cv.min_width_ch);
        // Use the same normalized logical block axis as the min/max bridge.
        let min_height_ch = if cv.writing_mode == WritingMode::HorizontalTb
            && cv.cssom_writing_mode != WritingMode::HorizontalTb
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
        while let Some(parent) = doc.parent_of(child) {
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

    // Each page's slice of the shared flow space: from its origin to the next
    // page's origin, or its content height for the last page.
    for (page_slot, page) in pages.iter_mut().enumerate() {
        let page_start = page.content_origin_y;
        let page_end = ordered_slices
            .get(page_slot + 1)
            .map(|next| next.content_origin_y)
            .filter(|next| next.is_finite() && *next > page_start)
            .unwrap_or(page_start + page.content_box.height);
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
    let root_generates_box = cascade.root_element_computed().display != DisplayValue::None;
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
        line_start_offset: u32,
        is_repeat: bool,
        fragmentainer: u32,
        fragment_clip: Option<PaintRect>,
        overflow_chain: Option<usize>,
        own_overflow_source: Option<usize>,
    }

    let mut nodes = Vec::new();
    let mut ifc_origin: HashMap<usize, (f32, f32)> = HashMap::new();
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
        node: body_id,
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
        let is_repeat = inherited_repeat
            || matches!(
                cascade
                    .computed
                    .get(node_id)
                    .map(|computed| &computed.position),
                Some(PositionValue::Fixed)
            );
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
        if cascade
            .computed
            .get(node_id)
            .is_some_and(|cv| cv.display == DisplayValue::ListItem)
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
            if origin.0.is_finite() && origin.1.is_finite() {
                text_roots.push(ProjectedTextRoot {
                    node: root_id,
                    x: origin.0,
                    y: origin.1,
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
                    nodes.push(PageFragmentSource {
                        abs_x: root_x + offset_x,
                        abs_y: root_y + first_top,
                        width: finite_nonnegative(owned.width),
                        height: finite_nonnegative(last_bottom - first_top),
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
            let mut item = PageFragmentItem::new(
                source.node_id,
                PageFragmentRect::new(source.abs_x, y, source.width, fragment_height),
                kind,
                fragment_index as u32,
                fragment_count,
                source.is_repeat || is_table_header,
            )
            .with_page_index(page.page_index)
            .with_box_extent(box_y, source.height)
            .with_content_rect(source.content_insets.map(|insets| {
                PageFragmentRect::new(
                    source.abs_x + insets.left,
                    box_y + insets.top,
                    (source.width - insets.left - insets.right).max(0.0),
                    (source.height - insets.top - insets.bottom).max(0.0),
                )
            }));
            item.is_table_header_repeat = is_table_header;
            item.fragmentainer = source.fragmentainer;
            item.overflow_chain = source.overflow_chain;
            item.own_overflow_source = source.own_overflow_source;
            item.fragment_clip = source.fragment_clip.map(|clip| {
                PageFragmentRect::new(
                    clip.x,
                    clip.y + box_y - source.abs_y,
                    clip.width,
                    clip.height,
                )
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
    // crossing that start had before it. The body spans one page area from
    // the top of the relayout's first page, which the relayout drops; when
    // it did not cross the start in the earlier layout, it has no fragment
    // after it.
    if let Some(earlier) = &document.earlier_fragments
        && let Some(first) = pages.first_mut()
    {
        let body = NodeId::new(body_id as u64);
        first.items.retain(|item| {
            item.node_id != body || item.box_y >= -0.01 || earlier.contains_key(&body)
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
        *fragment_counts
            .entry(item.node_id)
            .or_insert_with(|| before(&item.node_id).unwrap_or(0)) += 1;
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

/// Collect deterministic page-local link events from page snapshots.
///
/// The event geometry is copied from the correlated `PageFragmentItem`, so a
/// consumer can join on `(placement_node_id, page_index)` without seeing the
/// layout engine's internal tree. `anchor_node_id` preserves the owning `<a>`
/// identity when a link wraps text or replaced descendants. Raw trimmed
/// `href` values, including empty and relative values, are not URL-parsed.
/// A box ancestor is omitted when a more specific text or replaced placement
/// already represents the same link, retaining one useful hit rectangle per
/// visible leaf and a fallback for empty/non-text links.
pub(crate) fn page_fragment_events_from_pages(
    document: &Document,
    pages: &[PageFragment],
) -> Vec<PageFragmentEvent> {
    let Some(body_id) = find_body(document) else {
        return Vec::new();
    };

    let mut owners = HashMap::<usize, usize>::new();
    let mut hrefs = HashMap::<usize, String>::new();
    let mut stack = vec![(body_id, None::<usize>)];
    while let Some((node_id, inherited_owner)) = stack.pop() {
        let node = &document.nodes[node_id];
        if !node.is_in_document() || node.is_non_rendered_html_element() || node.is_display_none() {
            continue;
        }
        let owner = if node.kind() == NodeKind::Element && node.tag_name() == Some("a") {
            node.attribute("href")
                .map(str::trim)
                .map(|href| {
                    hrefs.entry(node_id).or_insert_with(|| href.to_owned());
                    node_id
                })
                .or(inherited_owner)
        } else {
            inherited_owner
        };
        if let Some(owner) = owner {
            owners.insert(node_id, owner);
        }
        for &child_id in node.children.iter().rev() {
            stack.push((child_id, owner));
        }
    }

    let mut events = Vec::new();
    for page in pages {
        let linked_items: Vec<&PageFragmentItem> = page
            .items
            .iter()
            .filter(|item| {
                usize::try_from(item.node_id.0)
                    .ok()
                    .and_then(|node_id| owners.get(&node_id))
                    .and_then(|owner| hrefs.get(owner))
                    .is_some()
                    && item.rect.width > 0.0
                    && item.rect.height > 0.0
            })
            .collect();
        let mut suppressed_boxes = HashSet::new();
        if linked_items
            .iter()
            .any(|item| item.kind == PageFragmentKind::Box)
        {
            for item in &linked_items {
                let node_id = usize::try_from(item.node_id.0)
                    .expect("linked item NodeId must fit the local arena index");
                let owner = owners.get(&node_id);
                // Start at the parent: another fragment of this node is not
                // a descendant. A marked ancestor's remaining chain has
                // already been visited for this page and anchor identity.
                let mut parent = document.parent_of(node_id);
                while let Some(id) = parent {
                    #[cfg(all(test, debug_assertions))]
                    tests::link_ancestor_tests::note_probe();
                    if owners.get(&id) != owner || !suppressed_boxes.insert(id) {
                        break;
                    }
                    parent = document.parent_of(id);
                }
            }
        }
        for item in linked_items.iter().copied() {
            // `linked_items` has already validated all three lookups above.
            // Keeping the invariant explicit here avoids a second set of
            // impossible branches in the hot event projection loop.
            let placement_node_id = usize::try_from(item.node_id.0)
                .expect("linked item NodeId must fit the local arena index");
            let anchor_id = *owners
                .get(&placement_node_id)
                .expect("linked item must have an anchor owner");
            let href = hrefs
                .get(&anchor_id)
                .expect("anchor owner must retain its href");
            if item.kind == PageFragmentKind::Box {
                #[cfg(all(test, debug_assertions))]
                tests::link_ancestor_tests::note_probe();
                if suppressed_boxes.contains(&placement_node_id) {
                    continue;
                }
            }
            events.push(PageFragmentEvent::Link(PageFragmentLinkEvent::new(
                NodeId::new(anchor_id as u64),
                item.node_id,
                page.page_index,
                item.rect,
                item.fragment_index,
                item.fragment_count,
                item.is_repeat,
                item.line_range,
                PageFragmentLink::new(href.clone()),
            )));
        }
    }
    events.sort_by(|left, right| match (left, right) {
        (PageFragmentEvent::Link(left), PageFragmentEvent::Link(right)) => left
            .page_index
            .cmp(&right.page_index)
            .then_with(|| left.anchor_node_id.cmp(&right.anchor_node_id))
            .then_with(|| left.placement_node_id.cmp(&right.placement_node_id))
            .then_with(|| left.rect.y.total_cmp(&right.rect.y))
            .then_with(|| left.rect.x.total_cmp(&right.rect.x))
            .then_with(|| left.fragment_index.cmp(&right.fragment_index)),
    });
    events
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
        let computed = &cascade.computed[id];
        node.kind() == NodeKind::Element
            && node.is_in_document()
            && !node.is_non_rendered_html_element()
            && !matches!(
                computed.display,
                DisplayValue::None | DisplayValue::Contents
            )
            && !matches!(
                computed.position,
                PositionValue::Absolute | PositionValue::Fixed
            )
    };
    let is_float =
        |document: &Document, id: usize| is_floating_box_for_pagination(document, cascade, id);
    // The in-flow and floating child boxes of `parent`, and its nonempty text
    // as `None`, with the children of a `display: contents` element standing
    // in its place.
    let child_boxes = |document: &Document, parent: usize| {
        let mut boxes = Vec::new();
        // `None` stands for the `::before` content of a flattened element.
        let mut pending: Vec<Option<usize>> = document.nodes[parent]
            .children
            .iter()
            .rev()
            .map(|&child| Some(child))
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
                && (cascade.computed[child].display == DisplayValue::Contents
                    || cascade.computed[child].display == DisplayValue::Inline
                        && in_flow(document, child)
                        && !is_float(document, child)
                        && !is_monolithic_for_pagination(document, cascade, child))
            {
                // An inline box is content of its parent's line, so its own
                // break properties do not apply, but pagination still breaks
                // at the blocks inside it. One without children still makes
                // a line of its own.
                if node.children.is_empty()
                    && cascade.computed[child].display == DisplayValue::Inline
                {
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
            } else if in_flow(document, child) {
                // An atomic inline box, a replaced element among them, is
                // content of its parent's line, which pagination does not
                // break around or inside.
                let inline_level =
                    cascade.computed[child].display.is_inline_level() && !is_float(document, child);
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
            let last = child_boxes(document, id)
                .into_iter()
                .rev()
                .find(|child| child.is_none_or(|child| !is_float(document, child)));
            match last {
                Some(Some(child)) => id = child,
                _ => break None,
            }
        };
        for &id in chain.iter().rev() {
            if page_applies_to_box(document, cascade, id) {
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
        for item in child_boxes(document, parent) {
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
            let own_page_name = if page_applies_to_box(document, cascade, child) {
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
        let computed = &cascade.computed[child];
        if !node.is_in_document()
            || node.is_non_rendered_html_element()
            || computed.display == DisplayValue::None
            || matches!(
                computed.position,
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
        if computed.display == DisplayValue::Contents && !node.has_before_or_after_content {
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
        if computed.display != DisplayValue::Contents {
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
                computed.display,
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
    if cascade.root_element_computed().display == DisplayValue::None {
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
    if cascade.root_element_computed().display == DisplayValue::None {
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

    let body_id = find_body(document).ok_or_else(|| LayoutError::Internal {
        message: "no <body> element found (fragment parse not supported yet)".to_string(),
    })?;
    let margins = page_margins(cascade, page_box);
    let insets = page_content_insets(cascade, page_box);
    // The body root sits at the top of the page content box, so the html
    // element's block-start margin is not represented in descendant
    // coordinates; carry it into the initial fragmentainer cursor. Margins of
    // the root element's box do not collapse (CSS 2.1 §8.3.1), so it simply
    // adds to the body's used block-start margin, which layout already
    // placed inside the body root.
    let html_margin_top = document.nodes[document.root]
        .children
        .iter()
        .copied()
        .find(|&node_id| document.nodes[node_id].tag_name() == Some("html"))
        .and_then(|html_id| {
            used_style_length_percentage_auto(
                document.nodes[html_id].style.margin.top,
                insets.page_area_width(margins, page_box),
            )
            .or_else(|| {
                used_computed_length_percentage_or_auto(
                    cascade.computed[html_id].margin.top,
                    insets.page_area_width(margins, page_box),
                )
            })
            .map(|value| value.max(0.0))
        })
        .unwrap_or(0.0);
    let body_margin_top = document.body_block_start_margin;
    // Keep the scheduled inline size identical to the first layout pass.
    let content_width = insets.page_area_width(margins, page_box);
    let content_height = insets.page_area_height(margins, page_box);
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
    let page_index_for_end = |end: f32| page_origins.page_index_for_end(end);
    // A root margin that reaches into a later fragmentainer consumes whole
    // leading pages.  Do not leave the first content run stranded halfway
    // down the first nonblank page: the margin that adjoins the unforced
    // break is truncated (CSS Fragmentation 3 §5.2).
    // The body's part of that margin is already in the layout coordinates,
    // so only the rest is added to the content.
    let root_flow_start = if page_index_for_y(html_margin_top) > 0 {
        // The root's own margin crosses the break. The root's margins do not
        // collapse with the body's, so the body's margin starts after the
        // truncated one.
        let page_index = page_index_for_y(html_margin_top);
        control.check_discovery_page_index(page_index)?;
        page_origin(page_index)
    } else {
        html_margin_top
    };
    // The body's margin may cross a later break of its own; its remainder is
    // truncated as well, so the content starts at the top of that page.
    let body_content_start = root_flow_start + body_margin_top;
    let body_content_page = page_index_for_y(body_content_start);
    let root_flow_offset = if body_content_page > page_index_for_y(root_flow_start) {
        control.check_discovery_page_index(body_content_page)?;
        page_origin(body_content_page) - body_margin_top
    } else {
        root_flow_start
    };

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
            if id >= document.nodes.len() {
                break;
            }
        }
        y
    }

    /// The outermost box whose top edge is that of `block`, which starts its
    /// ancestors: each of them up to it has no top border or padding, begins
    /// where its child does and renders nothing before it. Moving that box moves `block` without leaving the
    /// ancestors' empty starts behind on the earlier page.
    fn first_box_ancestor_at_top(
        document: &Document,
        cascade: &CascadeResult,
        block: usize,
        body_id: usize,
    ) -> usize {
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
                            && cascade.computed[sibling].display != DisplayValue::None
                    }
                    _ => node
                        .text_content()
                        .is_some_and(|text| !text.trim().is_empty()),
                };
                renders
                    && (child_index.is_none_or(|child_index| index < child_index)
                        || node.kind() == NodeKind::Element
                            && matches!(
                                cascade.computed[sibling].position,
                                PositionValue::Absolute | PositionValue::Fixed
                            ))
            });
            if renders_before {
                break;
            }
            child = parent;
            if cascade.computed[parent].display == DisplayValue::Contents {
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

    fn has_explicit_positioned_block_ancestor(
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
    ) -> bool {
        let mut current = Some(document.ifc_source_owner(node_id));
        while let Some(id) = current {
            let computed = &cascade.computed[id];
            let inset = &document.nodes[id].style.inset;
            if matches!(
                computed.position,
                PositionValue::Absolute | PositionValue::Fixed
            ) && (!inset.top.is_auto() || !inset.bottom.is_auto())
            {
                return true;
            }
            current = document.parent_of(id);
        }
        false
    }

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
            if document.nodes[child].style.position == TaffyPosition::Absolute
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

    fn materialize_y(document: &mut Document, node_id: usize, desired_y: f32) {
        if !desired_y.is_finite() {
            return;
        }
        let actual_y = current_abs_y(document, node_id);
        let delta = desired_y - actual_y;
        if delta.is_finite() {
            document
                .table_layout_node_mut(node_id)
                .unrounded_layout
                .location
                .y += delta;
            if node_id < document.nodes.len() {
                follow_moved_ifc_block(document, node_id, delta);
            }
        }
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
        let Some(first) = text_lines
            .get(text)
            .and_then(|owned| owned.lines.first().copied())
        else {
            return;
        };
        let first_top = positioned_owned_text_line_bounds(
            document,
            text_lines
                .get(text)
                .expect("indexed first text line has an owner"),
        )
        .and_then(|lines| lines.first().map(|line| line.0))
        .unwrap_or(first.top);
        let layout = document
            .ifc_layout_node(root)
            .expect("valid paragraph root")
            .unrounded_layout;
        let old_top = layout.border.top + layout.padding.top + first_top;
        let Some(ifc) = document.table_layout_node_mut(root).ifc.as_mut() else {
            return;
        };
        let Some(lines) = ifc.lines.as_mut() else {
            return;
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
            return;
        };
        let layout = document.nodes[node_id].unrounded_layout;
        let old_top = layout.location.y - delta;
        let old_bottom = old_top + layout.size.height;
        let Some(ifc) = document.table_layout_node_mut(root).ifc.as_mut() else {
            return;
        };
        let Some(lines) = ifc.lines.as_mut() else {
            return;
        };
        let Some(&(_, first_after)) = lines
            .block_line_starts
            .iter()
            .find(|(block, _)| *block == node_id)
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
                *after |= b.node == node_id;
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
            if id >= document.nodes.len() || guard > document.nodes.len() {
                break;
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
                break;
            }
        }
        false
    }

    #[derive(Clone)]
    struct PageCandidate {
        node_id: usize,
        raw_y: f32,
        height: f32,
        is_text: bool,
        is_rendered_leaf: bool,
        is_direct_body_element: bool,
        is_table_row: bool,
        is_flex_item: bool,
        is_grid_item: bool,
        is_named: bool,
        is_float_descendant: bool,
        is_out_of_flow_descendant: bool,
        /// Page type inherited from the nearest containing class-A box.
        /// `None` is the anonymous page type, not an unknown value.
        page_name: Option<String>,
        /// Column-flex descendants compare names within their containing flow.
        named_flex_context: Option<usize>,
        /// Whether the candidate takes part in page-name transitions. Floats
        /// and out-of-flow subtrees are parallel flows whose names are
        /// ignored, except for a named box inside an absolutely positioned
        /// box left at its static position, which keeps its name in the page
        /// flow.
        page_name_in_flow: bool,
        deferred_named_break_after: bool,
        /// For a text node of an ifc paragraph: the paragraph root and the
        /// text's offset below the root's border-box top. The root's lines
        /// are painted from the root, so the text moves by moving the root.
        ifc_root: Option<(usize, f32)>,
    }

    /// Whether `node_id` or an ancestor is a fixed box, or an absolutely
    /// positioned box with a non-auto block-start or block-end inset, which
    /// moves it away from its static position in the page flow. A fixed box
    /// repeats on every page, so it never joins the page flow.
    fn in_inset_out_of_flow_subtree(
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
    ) -> bool {
        let mut current = Some(node_id);
        while let Some(id) = current {
            if document.nodes[id].kind() == NodeKind::Element {
                match cascade.computed[id].position {
                    PositionValue::Fixed => return true,
                    PositionValue::Absolute => {
                        let inset = &document.nodes[id].style.inset;
                        if !inset.top.is_auto() || !inset.bottom.is_auto() {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            current = document.parent_of(id);
        }
        false
    }

    fn has_nested_named_page_descendant(
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        depth: u32,
    ) -> bool {
        let mut pending: Vec<_> = document
            .get_node(node_id)
            .into_iter()
            .flat_map(|node| node.children.iter().rev().copied())
            .map(|child_id| (child_id, depth.saturating_add(1)))
            .collect();
        while let Some((current_id, current_depth)) = pending.pop() {
            let node = &document.nodes[current_id];
            let computed = &cascade.computed[current_id];
            if !node.is_in_document()
                || node.is_non_rendered_html_element()
                || computed.display == DisplayValue::None
                || matches!(
                    computed.position,
                    PositionValue::Absolute | PositionValue::Fixed
                )
                || is_floating_box_for_pagination(document, cascade, current_id)
            {
                continue;
            }
            if current_depth >= 2
                && page_applies_to_box(document, cascade, current_id)
                && selected_page_name(cascade, current_id).is_some()
            {
                return true;
            }
            // Names inside a monolithic box have no effect on pagination.
            if is_monolithic_for_pagination(document, cascade, current_id) {
                continue;
            }
            if let Some(node) = document.get_node(current_id) {
                pending.extend(
                    node.children
                        .iter()
                        .rev()
                        .copied()
                        .map(|child_id| (child_id, current_depth.saturating_add(1))),
                );
            }
        }
        false
    }

    /// A named box containing a hidden subtree still establishes an explicit
    /// page boundary. Keep that boundary distinct from the zero-height named
    /// runs that are otherwise coalesced at one source coordinate.
    fn has_display_none_descendant(document: &Document, node_id: usize) -> bool {
        let mut pending: Vec<_> = document
            .get_node(node_id)
            .into_iter()
            .flat_map(|node| node.children.iter().rev().copied())
            .collect();
        while let Some(current_id) = pending.pop() {
            let Some(node) = document.get_node(current_id) else {
                continue;
            };
            if node.is_display_none() {
                return true;
            }
            pending.extend(node.children.iter().rev().copied());
        }
        false
    }

    // Candidate collection carries the recursive layout state explicitly so page
    // membership is decided from source coordinates before local page offsets.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn collect_candidates(
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        parent_abs_y: f32,
        direct_body_child: bool,
        body_id: usize,
        parent_height: f32,
        page_step: f32,
        inherited_page_name: Option<String>,
        inside_table: bool,
        flex_column_parent: bool,
        grid_single_column_parent: bool,
        inside_flex: bool,
        named_flex_context: Option<usize>,
        inside_float: bool,
        inside_out_of_flow: bool,
        out: &mut Vec<PageCandidate>,
        text_lines: &mut TextLineIndex,
        control: &PageLayoutControl<'_>,
    ) -> Result<(), LayoutError> {
        stacker::maybe_grow(128 * 1024, 1024 * 1024, || {
            collect_candidates_inner(
                document,
                cascade,
                node_id,
                parent_abs_y,
                direct_body_child,
                body_id,
                parent_height,
                page_step,
                inherited_page_name,
                inside_table,
                flex_column_parent,
                grid_single_column_parent,
                inside_flex,
                named_flex_context,
                inside_float,
                inside_out_of_flow,
                out,
                text_lines,
                control,
            )
        })
    }

    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn collect_candidates_inner(
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        parent_abs_y: f32,
        direct_body_child: bool,
        body_id: usize,
        parent_height: f32,
        page_step: f32,
        inherited_page_name: Option<String>,
        inside_table: bool,
        flex_column_parent: bool,
        grid_single_column_parent: bool,
        inside_flex: bool,
        named_flex_context: Option<usize>,
        inside_float: bool,
        inside_out_of_flow: bool,
        out: &mut Vec<PageCandidate>,
        text_lines: &mut TextLineIndex,
        control: &PageLayoutControl<'_>,
    ) -> Result<(), LayoutError> {
        control.check_aborted()?;
        let Some(node) = document.get_node(node_id) else {
            return Ok(()); // cov:ignore: traversal IDs come from this document's valid child graph.
        };
        if !node.is_in_document() || node.is_non_rendered_html_element() {
            return Ok(());
        }
        match node.kind() {
            NodeKind::Text => {
                if (direct_body_child || parent_height > 0.0)
                    && matches!(
                        &node.data,
                        crate::node::NodeData::Text(text) if !text.text_content.trim().is_empty()
                    )
                {
                    // A text node of an ifc paragraph has no layout of its own:
                    // it stands for the lines it owns, measured from the root's
                    // content box. The inline elements between the root and
                    // the text pass the root's border-box y down unchanged, so
                    // `parent_abs_y` is the root's. A text node that is a root
                    // itself (an anonymous flex or grid item) has a box of its
                    // own, located like any other.
                    text_lines.prepare_for(document, node_id, || control.check_aborted())?;
                    let (raw_y, height, ifc_root) = match text_lines.get(node_id) {
                        Some(owned) if owned.root != node_id => {
                            let root_layout = document
                                .ifc_layout_node(owned.root)
                                .expect("owned text lines require an IFC layout view")
                                .unrounded_layout;
                            let root_y = if owned.root >= document.nodes.len() {
                                root_layout.location.y
                            } else {
                                0.0
                            };
                            let positioned = positioned_owned_text_line_bounds(document, owned)
                                // cov:ignore: ifc_text_lines returns Some only when these root line records exist.
                                .unwrap_or_else(|| {
                                    owned
                                        .lines
                                        .iter()
                                        .map(|line| (line.top, line.bottom))
                                        .collect()
                                });
                            let first_top = positioned.first().map_or(0.0, |line| line.0);
                            let last_bottom = positioned
                                .iter()
                                .map(|line| line.1)
                                .fold(f32::NEG_INFINITY, f32::max);
                            let offset =
                                root_layout.border.top + root_layout.padding.top + first_top;
                            (
                                parent_abs_y + root_y + offset,
                                (last_bottom - first_top).max(0.0),
                                Some((owned.root, offset)),
                            )
                        }
                        _ => (
                            parent_abs_y + node.unrounded_layout.location.y,
                            node.unrounded_layout.size.height.max(0.0),
                            None,
                        ),
                    };
                    out.push(PageCandidate {
                        node_id,
                        raw_y,
                        height,
                        is_text: true,
                        is_rendered_leaf: false,
                        is_direct_body_element: false,
                        is_table_row: false,
                        is_flex_item: false,
                        is_grid_item: false,
                        is_named: false,
                        is_float_descendant: inside_float,
                        is_out_of_flow_descendant: inside_out_of_flow,
                        page_name: inherited_page_name,
                        named_flex_context,
                        page_name_in_flow: !inside_float && !inside_out_of_flow,
                        deferred_named_break_after: false,
                        ifc_root,
                    });
                }
            }
            NodeKind::Element => {
                if node.is_display_none() {
                    return Ok(());
                }
                // An element inside a paragraph laid out by the inline engine
                // moves with the paragraph's lines; breaking at it on its own
                // would leave the lines where they are. Its text stands for
                // lines of the root and the boxes inside it are located from
                // the root, like the root's own children, so both are
                // collected as if they were. The flag is set only when the
                // inline engine is switched on.
                if node.flags.contains(NodeFlags::IN_IFC_SUBTREE) {
                    for &child_id in &node.children {
                        if matches!(
                            document.nodes[child_id].kind(),
                            NodeKind::Element | NodeKind::Text
                        ) {
                            collect_candidates(
                                document,
                                cascade,
                                child_id,
                                parent_abs_y,
                                direct_body_child,
                                body_id,
                                parent_height,
                                page_step,
                                inherited_page_name.clone(),
                                inside_table,
                                flex_column_parent,
                                grid_single_column_parent,
                                inside_flex,
                                named_flex_context,
                                inside_float,
                                inside_out_of_flow,
                                out,
                                text_lines,
                                control,
                            )?; // cov:ignore: LLVM coverage maps this multiline call to its first expression line.
                        }
                    }
                    return Ok(());
                }
                let raw_y = parent_abs_y + node.unrounded_layout.location.y;
                let computed = &cascade.computed[node_id];
                // A floated box does not establish a page transition merely
                // because it carries an inherited/explicit `page` value.  In
                // particular, CSS Page 3 keeps a float in the preceding
                // fragmentainer in the page-name-float cases.
                let is_float = is_floating_box_for_pagination(document, cascade, node_id);
                let float_subtree = inside_float || is_float;
                let out_of_flow_subtree = inside_out_of_flow
                    || matches!(
                        computed.position,
                        PositionValue::Absolute | PositionValue::Fixed
                    );
                // An absolutely positioned box left at its static position
                // still lays its in-flow descendants out where the page flow
                // puts them, so their page names take part in pagination.
                let inset_out_of_flow_subtree =
                    out_of_flow_subtree && in_inset_out_of_flow_subtree(document, cascade, node_id);
                let explicit_page_name = selected_page_name(cascade, node_id);
                // CSS Page 3 §4.1: `page` applies to boxes that create class A
                // break points, which are block-level. An inline-level box,
                // such as a replaced element in a line or an inline-block,
                // takes the page type of its containing block; it is a
                // candidate carrying that inherited name instead.
                let inline_level = !page_applies_to_box(document, cascade, node_id);
                // The `page` property on an out-of-flow box does not open a
                // normal-flow page boundary. Keep its inherited page context,
                // but do not use its explicit name to split pagination.
                let parent_box = box_parent(document, cascade, node_id, BoxParentScope::Document);
                let is_direct_flex_item = parent_box
                    .is_some_and(|parent| cascade.computed[parent].display.is_flex_container());
                let own_page_name = if !inline_level
                    && !float_subtree
                    && (!inside_flex || named_flex_context.is_some())
                    && !is_direct_flex_item
                    && !inset_out_of_flow_subtree
                    && !matches!(
                        computed.position,
                        PositionValue::Absolute | PositionValue::Fixed
                    ) {
                    explicit_page_name.clone()
                } else {
                    None
                };
                let (has_propagated_page_name, propagated_page_name) = propagated_start_page_name(
                    document,
                    cascade,
                    node_id,
                    inherited_page_name.as_deref(),
                );
                let column_flex = computed.display.is_flex_container()
                    && matches!(
                        computed.flex_direction,
                        FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
                    );
                let deferred_named_break_after = matches!(computed.display, DisplayValue::Flex)
                    && !column_flex
                    && has_nested_named_page_descendant(document, cascade, node_id, 0);
                // The items of a row flex container sit side by side rather
                // than as class A siblings, so their names do not become the
                // container's start page value.
                let row_flex = computed.display.is_flex_container() && !column_flex;
                let page_name = if column_flex
                    || (row_flex && !deferred_named_break_after)
                    || (named_flex_context.is_some() && !deferred_named_break_after)
                {
                    own_page_name.clone().or(inherited_page_name.clone())
                } else if has_propagated_page_name {
                    propagated_page_name
                } else {
                    own_page_name.clone().or(inherited_page_name.clone())
                };
                let child_page_name = own_page_name.clone().or(inherited_page_name);
                let is_body = node_id == body_id;
                let participates_in_flow = matches!(
                    computed.position,
                    PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                );
                let is_tall_direct_absolute = direct_body_child
                    && matches!(computed.position, PositionValue::Absolute)
                    && node.unrounded_layout.size.height > page_step;
                let table_row_candidate = inside_table
                    && matches!(computed.display, DisplayValue::TableRow)
                    && !inside_float
                    && !inside_flex;
                let flex_item_candidate =
                    flex_column_parent && !inside_float && !out_of_flow_subtree;
                let grid_item_candidate =
                    grid_single_column_parent && !inside_float && !out_of_flow_subtree;
                // The table engine stores row geometry on its cells rather
                // than on the anonymous row box. Derive the row border box
                // from those direct cells so pagination can keep the row
                // together without changing table sizing.
                let (candidate_raw_y, candidate_height) = if table_row_candidate {
                    let mut row_top = f32::INFINITY;
                    let mut row_bottom = f32::NEG_INFINITY;
                    for &child_id in &node.children {
                        let Some(child) = document.get_node(child_id) else {
                            continue;
                        };
                        let child_top = child.unrounded_layout.location.y;
                        row_top = row_top.min(child_top);
                        row_bottom = row_bottom.max(child_top + child.unrounded_layout.size.height);
                    }
                    if row_top.is_finite() && row_bottom.is_finite() && row_bottom > row_top {
                        (raw_y + row_top, row_bottom - row_top)
                    } else {
                        (raw_y, node.unrounded_layout.size.height.max(0.0))
                    }
                } else {
                    (raw_y, node.unrounded_layout.size.height.max(0.0))
                };
                // Non-text content can establish the anonymous page before
                // a later named descendant, even within the first flex item.
                // Container geometry alone does not establish that context.
                let is_rendered_leaf = named_flex_context.is_some()
                    && computed.display != DisplayValue::Contents
                    && !float_subtree
                    && !out_of_flow_subtree
                    && candidate_height > 0.0
                    && !node.children.iter().any(|&child| {
                        let child_node = &document.nodes[child];
                        if child_node.is_display_none()
                            || child_node.is_non_rendered_html_element()
                            || matches!(
                                cascade.computed[child].position,
                                PositionValue::Absolute | PositionValue::Fixed
                            )
                            || is_floating_box_for_pagination(document, cascade, child)
                        {
                            return false;
                        }
                        match &child_node.data {
                            crate::node::NodeData::Text(text) => {
                                !text.text_content.trim().is_empty()
                            }
                            crate::node::NodeData::Element(_) => {
                                child_node.unrounded_layout.size.height > 0.0
                            }
                            _ => false,
                        }
                    });
                // `break-inside: avoid` applies to block-level boxes at any
                // depth (CSS Fragmentation 3 §3.2), not only to the body's
                // children. A nested avoiding block is a candidate so the
                // pass can move it to the next page intact; its descendants
                // follow it. Tables and flex containers keep their own
                // item-level candidates.
                let nested_avoid_candidate = participates_in_flow
                    && !direct_body_child
                    && !inline_level
                    && !inside_table
                    && !inside_flex
                    && !out_of_flow_subtree
                    && is_block_level_flow_box(computed.display)
                    && matches!(
                        computed.break_inside,
                        raikiri_style::property::BreakInside::Avoid
                            | raikiri_style::property::BreakInside::AvoidPage
                    )
                    // Grid items in other tracks do not follow a moved item
                    // in block flow, so grid content keeps its own handling.
                    && !has_side_by_side_ancestor(document, cascade, node_id, body_id)
                    // A clipping ancestor of bounded height keeps its clip
                    // where it is, so moving the box would only hide it.
                    && !has_bounded_clipping_ancestor(document, cascade, node_id, body_id);
                // A nested float follows an earlier float that moved in its
                // formatting context (CSS 2.1 §9.5.1), and a nested block
                // that clears one stays below it (§9.5.2), so both are
                // candidates like the body's children.
                let nested_float_candidate = (is_float
                    || (!inline_level
                        && is_block_level_flow_box(computed.display)
                        && [FloatValue::Left, FloatValue::Right]
                            .into_iter()
                            .any(|side| clears_float_side(computed.clear, side))))
                    && participates_in_flow
                    && !direct_body_child
                    && !inside_table
                    && !inside_flex
                    && !out_of_flow_subtree
                    && !has_side_by_side_ancestor(document, cascade, node_id, body_id)
                    // A clipping ancestor of bounded height keeps its clip
                    // where it is, so moving the box would only hide it.
                    && !has_bounded_clipping_ancestor(document, cascade, node_id, body_id);
                let is_break_candidate = !is_body
                    && computed.display != DisplayValue::Contents
                    && !inside_float
                    && ((participates_in_flow
                        && direct_body_child
                        && (explicit_page_name.is_none() || inline_level))
                        || nested_avoid_candidate
                        || nested_float_candidate
                        || is_tall_direct_absolute
                        || table_row_candidate
                        || flex_item_candidate
                        || is_rendered_leaf
                        || grid_item_candidate
                        || deferred_named_break_after
                        || own_page_name.is_some()
                        || page_break_is_forced(document, node_id, computed.break_before)
                        || page_break_is_forced(document, node_id, computed.break_after));
                if is_break_candidate {
                    out.push(PageCandidate {
                        node_id,
                        raw_y: candidate_raw_y,
                        height: candidate_height,
                        is_text: false,
                        is_rendered_leaf,
                        is_direct_body_element: direct_body_child,
                        is_table_row: table_row_candidate,
                        is_flex_item: flex_item_candidate,
                        is_grid_item: grid_item_candidate,
                        is_named: own_page_name.is_some(),
                        is_float_descendant: inside_float,
                        is_out_of_flow_descendant: out_of_flow_subtree,
                        page_name: page_name.clone(),
                        named_flex_context,
                        page_name_in_flow: !inside_float
                            && (!out_of_flow_subtree || own_page_name.is_some()),
                        deferred_named_break_after,
                        ifc_root: None,
                    });
                }
                // An atomic inline-level box or an orthogonal flow is
                // monolithic: like a float, its content offers no class A
                // break points and its page names do not open pages. A
                // blockified flex or grid item is no longer atomic.
                let monolithic =
                    !is_body && is_monolithic_for_pagination(document, cascade, node_id);
                let child_order = pagination_child_order(document, cascade, node_id);
                for child_id in child_order {
                    let child_parent_y =
                        pagination_child_parent_y(document, node_id, child_id, raw_y);
                    collect_candidates(
                        document,
                        cascade,
                        child_id,
                        child_parent_y,
                        box_parent(document, cascade, child_id, BoxParentScope::Document)
                            == Some(body_id),
                        body_id,
                        node.unrounded_layout.size.height.max(0.0),
                        page_step,
                        child_page_name.clone(),
                        inside_table
                            || matches!(
                                computed.display,
                                DisplayValue::Table
                                    | DisplayValue::InlineTable
                                    | DisplayValue::TableRowGroup
                                    | DisplayValue::TableHeaderGroup
                                    | DisplayValue::TableFooterGroup
                                    | DisplayValue::TableRow
                            ),
                        computed.display.is_flex_container()
                            && matches!(
                                computed.flex_direction,
                                FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
                            )
                            || (computed.display == DisplayValue::Contents && flex_column_parent),
                        computed.display.is_grid_container()
                            && document.nodes[node_id].grid_column_count == 1,
                        inside_flex || matches!(computed.display, DisplayValue::Flex),
                        if column_flex {
                            Some(node_id)
                        } else if computed.display.is_flex_container() {
                            None
                        } else {
                            named_flex_context
                        },
                        float_subtree || monolithic,
                        out_of_flow_subtree,
                        out,
                        text_lines,
                        control,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut candidates = Vec::new();
    let mut text_lines = TextLineIndex::default();
    collect_candidates(
        document,
        cascade,
        body_id,
        root_flow_offset,
        false,
        body_id,
        0.0,
        page_step,
        None,
        false,
        false,
        false,
        false,
        None,
        false,
        false,
        &mut candidates,
        &mut text_lines,
        control,
    )?;
    let mut table_headers = super::table::headers::HeaderRepeats::prepare(
        document,
        cascade,
        body_id,
        page_step,
        |id| root_flow_offset + current_abs_y(document, id),
    );
    // A table row group with `break-inside: avoid` starts on the next page
    // when it would cross the current one. Its first row carries the block
    // end of the group's rows up to the first forced break between them (a
    // forced break already splits the group), and, when it is the table's
    // first row of a table without a caption, its offset below the table's
    // top: the table's spacing and borders above it move with it.
    let mut avoided_row_groups = HashMap::<usize, (f32, f32)>::new();
    // A header group repeated on each page is placed by the repetition
    // path; any other header group avoids breaks like a body group.
    let avoiding_row_group = |group: usize| {
        (matches!(
            cascade.computed[group].display,
            DisplayValue::TableRowGroup | DisplayValue::TableFooterGroup
        ) || (cascade.computed[group].display == DisplayValue::TableHeaderGroup
            && !table_headers.headers.contains_key(&group)))
            && matches!(
                cascade.computed[group].break_inside,
                raikiri_style::property::BreakInside::Avoid
                    | raikiri_style::property::BreakInside::AvoidPage
            )
    };
    if candidates.iter().any(|candidate| {
        candidate.is_table_row
            && document
                .parent_of(candidate.node_id)
                .is_some_and(avoiding_row_group)
    }) {
        let candidate_y: HashMap<usize, f32> = candidates
            .iter()
            .map(|candidate| (candidate.node_id, candidate.raw_y))
            .collect();
        let table_of = |row: usize| {
            let mut table = document.parent_of(row);
            while let Some(id) = table
                && !matches!(
                    cascade.computed[id].display,
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
                    *first = (candidate.node_id, candidate.raw_y);
                }
            }
        }
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
            let table = table_of(row);
            let first_in_table = table.is_some_and(|table| {
                table_first_rows
                    .get(&table)
                    .is_some_and(|&(first, _)| first == row)
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
                    // A caption above the grid stays on the current page.
                    let lead = table
                        .filter(|&table| {
                            first_in_table
                                && !document.nodes[table].children.iter().any(|&child| {
                                    cascade.computed[child].display == DisplayValue::TableCaption
                                        && cascade.computed[child].caption_side
                                            == raikiri_style::property::CaptionSideValue::Top
                                })
                        })
                        .map(|table| {
                            candidate_y.get(&table).copied().unwrap_or_else(|| {
                                root_flow_offset + current_abs_y(document, table)
                            })
                        })
                        .map_or(0.0, |table_y| (candidate.raw_y - table_y).max(0.0));
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
    // Connected class-A sibling runs have a legal break before their first
    // box. Move a run only when it fits in one page, otherwise relax its
    // avoidance constraints so an oversized run still makes progress.
    let mut avoided_run_ends = HashMap::<usize, f32>::new();
    let mut previous: Option<&PageCandidate> = None;
    let mut run_start: Option<usize> = None;
    for candidate in &candidates {
        if !(candidate.is_direct_body_element
            || document.nodes[candidate.node_id].kind() == NodeKind::Text
                && box_parent(
                    document,
                    cascade,
                    candidate.node_id,
                    BoxParentScope::Document,
                ) == Some(body_id))
        {
            continue;
        }
        let computed = &cascade.computed[candidate.node_id];
        // Floats and positioned boxes form parallel flows. Their placement
        // does not interrupt a connected run of normal-flow siblings.
        if is_floating_box_for_pagination(document, cascade, candidate.node_id)
            || matches!(
                computed.position,
                PositionValue::Absolute | PositionValue::Fixed
            )
        {
            continue;
        }
        let eligible = candidate.is_direct_body_element
            && (computed.display.is_block_flex_or_grid_container()
                || matches!(
                    computed.display,
                    DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
                ))
            && !matches!(computed.position, PositionValue::Running(_));
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
    let mut flow_shift = 0.0_f32;
    let mut current_page = 0_u32;
    let mut max_page = 0_u32;
    // The last page the normal flow (floats included) has reached, apart
    // from absolutely positioned boxes and footnotes, which paint in
    // parallel flows.
    let mut flow_max_page = 0_u32;
    // Keep a break-after attached to its source box until its descendants
    // have been visited.  Applying it to the first flex child would turn a
    // break-after on the flex container into an extra blank page.
    let mut pending_break_source: Option<usize> = None;
    // When a forced-break box underflows into the preceding page, descendants
    // follow the box while later siblings still begin after the break boundary.
    let mut pending_underflow: Option<(usize, f32)> = None;
    // A paragraph whose later lines were moved to the next page to keep
    // enough widows there: `(root, first moved line, shift)`. The flow after
    // the paragraph, and its texts that start on a moved line, take the shift
    // once they are reached; its texts before that line stay where they are.
    let mut pending_line_split: Option<(usize, usize, f32)> = None;
    // A float moved to a later page carries its own content with it, but
    // the in-flow content after it in the source sits beside or above it,
    // so the shift ends where the float's subtree does. A later float may
    // not start above an earlier one (CSS 2.1 §9.5.1), so it keeps the
    // shift of the last moved float until in-flow content opens a page.
    // The page the flow was on before the float moved is restored with it.
    let mut pending_float_shift: Option<(usize, f32, u32)> = None;
    let mut later_float_shift = 0.0_f32;
    // The page-flow outer edges of the moved floats on each side (left,
    // right), for each formatting context the floats were placed in: a
    // later float in the same context needs to move only as far as it would
    // otherwise start above one of them, or above its bottom when it clears
    // that side or does not fit beside it. Floats in other contexts do not
    // interact.
    let mut later_float_edges: HashMap<usize, Vec<FloatEdge>> = HashMap::new();
    // A nested box that moved inside an ancestor whose height cannot grow
    // to hold it overflows that ancestor: the part of the shift the
    // ancestor did not absorb ends where its subtree does, and the page the
    // flow was on is restored with it.
    let mut pending_bounded_shifts: Vec<(usize, f32, u32)> = Vec::new();
    // Formatting-context roots that grew around a moved float, with the
    // growth the flow after them keeps.
    let mut pending_context_growth: Vec<(usize, f32)> = Vec::new();
    // Unpaginated border-box bottom of the last in-flow block-level child
    // seen for each parent, while no other content followed it. The gap to
    // the next such sibling is their collapsed margin.
    let mut previous_block_bottoms: HashMap<usize, (usize, f32)> = HashMap::new();
    let mut margins_moved_boxes = false;
    // Named class-A boxes at one source coordinate form one page transition.
    // This coalesces zero-height named runs such as a/b/c/d/e without creating
    // one blank page per zero-height box.
    let mut last_named_raw_y: Option<f32> = None;
    let mut last_named_context: Option<usize> = None;
    let mut last_named_was_zero_height = false;
    let mut last_named_had_display_none_descendant = false;
    let mut saw_child = false;
    // Paragraph blocks whose leading line or whose orphans and widows move
    // them as a unit to the next page. Track the first text child so a block
    // is not reflowed repeatedly for later lines.
    let mut checked_block_text = HashSet::new();
    let mut checked_orphans_widows = HashSet::new();
    // Footnotes are collected in source order and stacked upward from the
    // bottom edge of their anchor page. The normal-flow height is removed
    // from `flow_shift`, leaving the box only in this page-local area.
    let mut footnote_heights: HashMap<u32, f32> = HashMap::new();
    let mut current_page_name =
        selected_page_name(cascade, body_id).or_else(|| first_page_name(document, cascade));
    let mut page_names = vec![current_page_name.clone()];
    let mut outer_page_name = current_page_name.clone();
    let mut seen_named_flex_contexts = HashMap::new();
    let mut column_context_cache = vec![None; document.nodes.len()];
    let mut preceding_flex_items = HashMap::<usize, (usize, f32, u32)>::new();

    let mut trailing_flex_child_by_parent = HashMap::<usize, Option<usize>>::new();
    // Paragraphs laid out by the inline engine that already had a candidate
    // inside them. Only the first candidate of a paragraph may move its root:
    // a later one would move again what the earlier ones placed.
    let mut entered_ifc_roots = HashSet::new();
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
            && let Some((_, height, page)) =
                preceding_flex_items.insert(context, (node_id, candidate.height, current_page))
            && height > 0.0
            && page == current_page
        {
            occupy_column_contexts(
                Some(context),
                current_page,
                &mut seen_named_flex_contexts,
                document,
                cascade,
                &mut column_context_cache,
            );
        }
        let (name_context_seen, comparison_page_name) = if !name_participates_in_flow {
            (false, current_page_name.clone())
        } else {
            match candidate.named_flex_context {
                Some(context) if candidate.deferred_named_break_after => {
                    // Empty enclosing column boxes do not open an anonymous
                    // page before the row's established deferred boundary.
                    let seen = column_context_occupied(
                        context,
                        current_page,
                        &seen_named_flex_contexts,
                        document,
                        cascade,
                        &mut column_context_cache,
                    ) || current_page_name.is_some();
                    outer_page_name = candidate.page_name.clone();
                    (saw_child && seen, current_page_name.clone())
                }
                Some(context)
                    if candidate.is_named || candidate.is_text || candidate.is_rendered_leaf =>
                {
                    // Nested column flows share the physical page opened by
                    // their preceding sibling, including an inner flow's end.
                    // A previously seen anonymous type is still a page type.
                    let seen = column_context_occupied(
                        context,
                        current_page,
                        &seen_named_flex_contexts,
                        document,
                        cascade,
                        &mut column_context_cache,
                    ) || current_page_name.is_some();
                    (saw_child && seen, current_page_name.clone())
                }
                Some(_) => (false, candidate.page_name.clone()),
                None => {
                    let previous = if candidate.is_named {
                        current_page_name.clone()
                    } else {
                        outer_page_name.clone()
                    };
                    if !candidate.is_float_descendant {
                        outer_page_name = candidate.page_name.clone();
                    }
                    (saw_child, previous)
                }
            }
        };
        // Leaving a flex item's internal named flow keeps the last opened
        // physical page until an outer boundary or natural overflow opens one.
        let keep_local_page_type =
            candidate.named_flex_context.is_none() && current_page_name != comparison_page_name;
        let page_before_candidate = current_page;
        let moves_ifc_root = {
            let mut first = candidate
                .ifc_root
                .filter(|(root, _)| *root >= document.nodes.len())
                .is_none_or(|(root, _)| entered_ifc_roots.insert(root));
            let mut current = document.parent_of(node_id);
            while let Some(id) = current {
                if document.nodes[id].is_ifc_root() && !entered_ifc_roots.insert(id) {
                    first = false;
                    // Higher IFC roots were registered when this root was first entered.
                    break;
                }
                current = document.parent_of(id);
            }
            first
        };
        let ifc_root = candidate.ifc_root.filter(|_| moves_ifc_root);
        if let Some((root, first_moved, shift)) = pending_line_split
            && (!is_descendant_or_self(document, node_id, root)
                || candidate
                    .ifc_root
                    .is_some_and(|(text_root, _)| text_root == root)
                    && text_lines
                        .get(node_id)
                        .and_then(|owned| owned.lines.first())
                        .is_some_and(|line| line.line >= first_moved))
        {
            flow_shift += shift;
            pending_line_split = None;
        }
        if let Some((ancestor_id, correction)) = pending_underflow
            && !is_descendant_or_self(document, node_id, ancestor_id)
        {
            flow_shift += correction;
            pending_underflow = None;
        }
        // Leaving several bounded subtrees at once returns to the page the
        // earliest of them started from.
        let mut restored_page: Option<u32> = None;
        pending_context_growth.retain(|&(context, growth)| {
            if is_descendant_or_self(document, node_id, context) {
                return true;
            }
            flow_shift += growth;
            false
        });
        pending_bounded_shifts.retain(|&(ancestor_id, delta, flow_page)| {
            if is_descendant_or_self(document, node_id, ancestor_id) {
                return true;
            }
            flow_shift -= delta;
            restored_page = Some(restored_page.map_or(flow_page, |page| page.min(flow_page)));
            false
        });
        if let Some(page) = restored_page {
            current_page = page;
        }
        if let Some((float_id, delta, flow_page)) = pending_float_shift
            && !is_descendant_or_self(document, node_id, float_id)
        {
            flow_shift -= delta;
            current_page = flow_page;
            pending_float_shift = None;
            // A follower shifted less than the float before it keeps the
            // larger budget, which the other side's floor may still need.
            later_float_shift = later_float_shift.max(delta);
        }
        let float_edges = (later_float_shift != 0.0 && pending_float_shift.is_none())
            .then(|| {
                let context = float_context_root(document, cascade, node_id, body_id);
                later_float_edges
                    .get(&context)
                    .map(|edges| (context, edges.clone()))
            })
            .flatten();
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
                let top = float_edges
                    .iter()
                    .map(|edge| edge.top)
                    .fold(f32::NEG_INFINITY, f32::max);
                let outer_height = layout.size.height + layout.margin.top + layout.margin.bottom;
                // The width the floats take at `level`: those that started
                // at or above it and still reach below it sit side by side.
                let width_at = |level: f32| -> f32 {
                    float_edges
                        .iter()
                        .filter(|edge| edge.top <= level + 0.001 && edge.bottom > level + 0.001)
                        .map(|edge| edge.width)
                        .sum()
                };
                // The highest level, from the floats' top down through their
                // bottoms, from which the float fits beside the floats along
                // its whole height.
                let fits_from = std::iter::once(top)
                    .chain(
                        float_edges
                            .iter()
                            .map(|edge| edge.bottom)
                            .filter(|&bottom| bottom > top),
                    )
                    .filter(|&level| {
                        let end = level + outer_height.max(0.0);
                        let beside = std::iter::once(level)
                            .chain(
                                float_edges
                                    .iter()
                                    .map(|edge| edge.top)
                                    .filter(|&edge_top| edge_top > level && edge_top < end),
                            )
                            .map(width_at)
                            .fold(0.0, f32::max);
                        outer_width + beside <= context_width + 0.001
                    })
                    .fold(f32::INFINITY, f32::min);
                Some(top.max(fits_from).max(cleared.unwrap_or(f32::NEG_INFINITY)))
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
                (floor - (flow_raw_y + flow_shift - margin_top)).clamp(0.0, later_float_shift)
            });
            if shift > 0.0 {
                flow_shift += shift;
                if floating {
                    pending_float_shift = Some((node_id, shift, current_page));
                    // The shifted float is now the one later floats on its
                    // side keep below.
                    if let Some(index) = float_side_index(cascade.computed[node_id].float) {
                        let layout = &document.nodes[node_id].unrounded_layout;
                        let top = flow_raw_y + flow_shift;
                        record_float_edge(
                            &mut later_float_edges,
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
                        &mut pending_bounded_shifts,
                        &mut pending_context_growth,
                        current_page,
                    );
                }
                // The box may now start on a later page than the flow.
                current_page = current_page.max(page_index_for_y(candidate.raw_y + flow_shift));
            }
        }
        if document.get_node(node_id).is_none() {
            continue;
        }
        let participates_in_margin_adjacency = !candidate.is_float_descendant
            && !candidate.is_out_of_flow_descendant
            && !is_floating_box_for_pagination(document, cascade, node_id);
        // Explicit positioned insets follow their containing box, not margin
        // changes in a parallel normal flow. Both-auto boxes keep static placement.
        let uses_positioned_coordinates = candidate.is_out_of_flow_descendant
            && has_explicit_positioned_block_ancestor(document, cascade, node_id);
        if candidate.is_text {
            // In-flow text between two blocks separates them: the next block's
            // gap is no longer a margin adjoining the previous block.
            if participates_in_margin_adjacency
                && let Some(parent_id) = candidate
                    .ifc_root
                    .map(|(root, _)| root)
                    .or_else(|| box_parent(document, cascade, node_id, BoxParentScope::Document))
            {
                previous_block_bottoms.remove(&parent_id);
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
                if let Some((root, _)) = candidate.ifc_root {
                    let layout = document
                        .ifc_layout_node(root)
                        .expect("valid paragraph root")
                        .unrounded_layout;
                    let first_top = positioned_owned_text_line_bounds(
                        document,
                        text_lines
                            .get(node_id)
                            .expect("text candidate has indexed ownership"),
                    )
                    .and_then(|lines| lines.first().map(|line| line.0))
                    .unwrap_or(0.0);
                    current_abs_y(document, root)
                        + layout.border.top
                        + layout.padding.top
                        + first_top
                } else {
                    current_abs_y(document, node_id)
                }
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
                .filter(|&root| root < document.nodes.len() && root != node_id);
            let line_block_parent = paragraph_root
                .or(document.parent_of(node_id))
                .filter(|parent_id| {
                    !uses_positioned_coordinates
                        && (!checked_block_text.contains(parent_id)
                            || !checked_orphans_widows.contains(parent_id))
                })
                .and_then(|parent_id| {
                    if document.layout_parent_of(parent_id)? == body_id {
                        Some((parent_id, false))
                    } else {
                        (!candidate.is_float_descendant
                            && !candidate.is_out_of_flow_descendant
                            && is_block_flow_paragraph(document, cascade, parent_id, body_id))
                        .then_some((parent_id, true))
                    }
                });
            // Moves the block, which sits at `from_y`, so that its flow
            // position starts at `target_y`, and returns how far it moved.
            // Its relative offsets move along with it. A nested block that
            // begins its ancestors, with no border or padding between them,
            // takes the ancestors along rather than leaving them empty on
            // this page; the ancestors above them grow to hold the move.
            let mut move_line_block = |document: &mut Document,
                                       block_id: usize,
                                       nested: bool,
                                       from_y: f32,
                                       target_y: f32| {
                let relative = accumulated_relative_offset_y(document, cascade, body_id, block_id);
                // A negative or undefined move, which a relative offset can
                // produce, leaves the block where it is.
                let delta = Some(target_y + relative - from_y)
                    .filter(|delta| delta.is_finite() && *delta > 0.0)
                    .unwrap_or(0.0);
                if delta > 0.0 {
                    let moved = if nested {
                        first_box_ancestor_at_top(document, cascade, block_id, body_id)
                    } else {
                        block_id
                    };
                    let moved_y = current_abs_y(document, moved) + delta;
                    materialize_y(document, moved, moved_y);
                    if nested {
                        grow_ancestors_for_shift(
                            document,
                            cascade,
                            moved,
                            delta,
                            body_id,
                            &mut pending_bounded_shifts,
                            &mut pending_context_growth,
                            page_before_candidate,
                        );
                    }
                }
                delta
            };
            if let Some((block_id, nested)) = line_block_parent
                && checked_block_text.insert(block_id)
                && style_dimension_length(document.nodes[block_id].style.size.height)
                    .is_some_and(|value| value >= 0.0)
            {
                let block_height = document.nodes[block_id].unrounded_layout.size.height;
                let block_raw_y = root_flow_offset + current_abs_y(document, block_id) - flow_shift;
                let relative_text_y = raw_y - block_raw_y;
                let block_effective_y = effective_y - relative_text_y;
                let page_start = page_origin(current_page);
                let page_end = page_start + page_step_at(current_page);
                let first_line_overflows = effective_y.is_finite()
                    && height.is_finite()
                    && effective_y >= page_start
                    && block_effective_y >= page_start
                    && block_effective_y < page_end
                    && block_height.is_finite()
                    && block_height <= page_step_at(current_page) + 0.001
                    && effective_y + height > page_end;
                if first_line_overflows {
                    let natural_page = page_index_for_y(effective_y + height);
                    let target_page = current_page.saturating_add(1).max(natural_page);
                    check_candidate_page!('candidate_loop, target_page);
                    let target_y = page_origin(target_page);
                    let delta =
                        move_line_block(document, block_id, nested, block_effective_y, target_y);
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
            // as a unit, but its later lines can move down inside it.
            let flex_line_paragraph = paragraph_root
                .filter(|_| {
                    line_block_parent.is_none()
                        && !uses_positioned_coordinates
                        && !candidate.is_float_descendant
                        && !candidate.is_out_of_flow_descendant
                })
                .and_then(|root| {
                    flex_container_of_item_alone_on_its_line(document, cascade, root, body_id)
                        .map(|container| (root, container))
                });
            if let Some((block_id, nested)) =
                line_block_parent.or(flex_line_paragraph.map(|(root, _)| (root, true)))
                && checked_orphans_widows.insert(block_id)
                && let Some(line_metrics) = paragraph_line_bounds(document, node_id, &text_lines)
            {
                let page_start = page_origin(current_page);
                let page_end = page_start + page_step_at(current_page);
                let total_lines = line_metrics.len();
                let starts_on_page = line_metrics.first().is_some_and(|(line_top, _)| {
                    let top = effective_y + line_top;
                    top >= page_start - 0.001 && top < page_end
                });
                let block_height = document.nodes[block_id].unrounded_layout.size.height;
                let fits_on_page =
                    block_height.is_finite() && block_height <= page_step_at(current_page) + 0.001;
                let lines_before_break = line_metrics
                    .iter()
                    .take_while(|(_, line_bottom)| effective_y + line_bottom <= page_end + 0.001)
                    .count();
                let needs_break = lines_before_break < total_lines;
                let orphans = cascade.computed[block_id].orphans.max(1) as usize;
                let widows = cascade.computed[block_id].widows.max(1) as usize;
                let violates_line_constraints = needs_break
                    && (lines_before_break < orphans
                        || total_lines.saturating_sub(lines_before_break) < widows);
                // Only the widows are short, and the paragraph has lines
                // enough for both constraints: the break moves up between
                // earlier lines (CSS Fragmentation 3 §3.3), so the last
                // `widows` lines start the next page and the lines before
                // them stay.
                let earlier_break = (starts_on_page
                    && violates_line_constraints
                    && lines_before_break >= orphans
                    && orphans + widows <= total_lines
                    && paragraph_root == Some(block_id))
                .then(|| total_lines - widows)
                .filter(|&line| {
                    // Widows that cannot fit on the next page would not be
                    // kept together there either; the natural break stays.
                    let moved_height = line_metrics[total_lines - 1].1 - line_metrics[line].0;
                    // A box with a definite height keeps it; its lines
                    // cannot grow it.
                    pending_line_split.is_none()
                        && height_follows_content(document, block_id)
                        && moved_height <= page_step_at(current_page.saturating_add(1)) + 0.001
                        && document
                            .ifc_layout_node(block_id)
                            .and_then(|node| node.ifc.as_ref())
                            .is_some_and(|ifc| {
                                ifc.boxes.is_empty() && ifc.multicol_fragments.is_none()
                            })
                        && line > 0
                })
                .map(|line| {
                    let target_y = page_origin(current_page.saturating_add(1));
                    (line, target_y - (effective_y + line_metrics[line].0))
                })
                .filter(|(_, shift)| shift.is_finite() && *shift > 0.0);
                if let Some((line, shift)) = earlier_break {
                    check_candidate_page!('candidate_loop, current_page.saturating_add(1));
                    shift_ifc_lines_from(document, block_id, line, shift);
                    if let Some((item, container)) = flex_line_paragraph {
                        shift_later_flex_lines(document, cascade, container, item, shift);
                    }
                    if nested {
                        grow_ancestors_for_shift(
                            document,
                            cascade,
                            block_id,
                            shift,
                            body_id,
                            &mut pending_bounded_shifts,
                            &mut pending_context_growth,
                            page_before_candidate,
                        );
                    }
                    if text_lines
                        .get(node_id)
                        .and_then(|owned| owned.lines.last())
                        .is_some_and(|last| last.line >= line)
                    {
                        height += shift;
                    }
                    pending_line_split = Some((block_id, line, shift));
                } else if line_block_parent.is_some()
                    && starts_on_page
                    && fits_on_page
                    && violates_line_constraints
                {
                    let block_raw_y =
                        root_flow_offset + current_abs_y(document, block_id) - flow_shift;
                    // The block already sits `flow_shift` below its raw
                    // position, so the move is measured from where it is now.
                    let block_effective_y = effective_y - (raw_y - block_raw_y);
                    let target_page = current_page.saturating_add(1);
                    check_candidate_page!('candidate_loop, target_page);
                    let delta = move_line_block(
                        document,
                        block_id,
                        nested,
                        block_effective_y,
                        page_origin(target_page),
                    );
                    flow_shift += delta;
                    effective_y += delta;
                }
            }
            if let Some((ifc_root, offset)) = ifc_root {
                materialize_y(document, ifc_root, effective_y - offset);
            } else if let Some((root, _)) = candidate.ifc_root
                && let Some(first) = text_lines
                    .get(node_id)
                    .and_then(|owned| owned.lines.first().copied())
            {
                // A later text of a paragraph whose root already moved: its
                // lines go where the flow puts them, and the ones after
                // follow (an earlier box of the paragraph may have grown).
                let layout = document
                    .ifc_layout_node(root)
                    .expect("valid paragraph root")
                    .unrounded_layout;
                let first_top = positioned_owned_text_line_bounds(
                    document,
                    text_lines
                        .get(node_id)
                        .expect("text candidate has indexed ownership"),
                )
                .and_then(|lines| lines.first().map(|line| line.0))
                .unwrap_or(first.top);
                let current = current_abs_y(document, root)
                    + layout.border.top
                    + layout.padding.top
                    + first_top;
                follow_moved_ifc_text(document, root, node_id, effective_y - current, &text_lines);
            }
            materialize_y(document, node_id, effective_y);
            let named_page_change = name_context_seen
                && height > 0.0
                && name_participates_in_flow
                && candidate_page_name != comparison_page_name;
            let pending_break_applies = pending_break_source
                .is_some_and(|source| !is_descendant_or_self(document, node_id, source));
            let consumes_pending_break = pending_break_applies && name_participates_in_flow;
            if saw_child {
                if consumes_pending_break || named_page_change {
                    let natural_page = if effective_y.is_finite() && effective_y >= 0.0 {
                        page_index_for_y(effective_y)
                    } else {
                        current_page
                    };
                    let target_page = current_page.saturating_add(1).max(natural_page);
                    check_candidate_page!('candidate_loop, target_page);
                    let target_y = page_origin(target_page);
                    // The candidate was materialized to `effective_y` above,
                    // so this is the additional movement for this boundary.
                    // Keep it separate from `flow_shift`: the latter also
                    // affects candidates whose ancestor is not moved here.
                    let node_delta = target_y - effective_y;
                    let shift_delta = target_y - effective_y;
                    if node_delta.is_finite() && shift_delta.is_finite() {
                        // The text of an ifc paragraph is painted from its
                        // root, so the root carries the movement.
                        let moved = ifc_root.map_or(node_id, |(root, _)| root);
                        document
                            .table_layout_node_mut(moved)
                            .unrounded_layout
                            .location
                            .y += node_delta;
                        // A later text of a paragraph whose root already
                        // moved: its lines, and the ones after, move alone.
                        if ifc_root.is_none()
                            && let Some((root, _)) = candidate.ifc_root
                        {
                            follow_moved_ifc_text(document, root, node_id, node_delta, &text_lines);
                        }
                        flow_shift += shift_delta;
                        effective_y += shift_delta;
                    }
                    current_page = target_page;
                    current_page_name = candidate_page_name.clone();
                } else if effective_y.is_finite() && effective_y >= 0.0 {
                    let natural_page = page_index_for_y(effective_y);
                    check_candidate_page!('candidate_loop, natural_page);
                    current_page = current_page.max(natural_page);
                }
            } else {
                saw_child = true;
                current_page = 0;
                if name_participates_in_flow {
                    current_page_name = candidate_page_name.clone();
                }
            }
            if height > 0.0
                && name_participates_in_flow
                && !named_page_change
                && !consumes_pending_break
                && (!keep_local_page_type || current_page > page_before_candidate)
            {
                current_page_name = candidate_page_name;
            }
            check_candidate_page!('candidate_loop, current_page);
            if page_names.len() <= current_page as usize {
                page_names.resize(current_page as usize + 1, None);
            }
            page_names[current_page as usize] = current_page_name.clone();
            max_page = max_page.max(current_page);
            if !candidate.is_out_of_flow_descendant {
                flow_max_page = flow_max_page.max(current_page);
            }
            if name_participates_in_flow && height > 0.0 {
                let context =
                    enclosing_column_context(node_id, document, cascade, &mut column_context_cache);
                occupy_column_contexts(
                    context,
                    current_page,
                    &mut seen_named_flex_contexts,
                    document,
                    cascade,
                    &mut column_context_cache,
                );
            }
            if height > 0.0 && effective_y.is_finite() && effective_y >= 0.0 {
                let end = (effective_y + height).max(effective_y);
                if end.is_finite() && end > 0.0 {
                    let end_page = page_index_for_end(end);
                    control.check_discovery_page_index(end_page)?;
                    max_page = max_page.max(end_page);
                    if !candidate.is_out_of_flow_descendant {
                        flow_max_page = flow_max_page.max(end_page);
                    }
                    // The pages a box continues onto keep the page type it
                    // started on until a later box names another, even when
                    // an enclosing box running onto them named them first.
                    let names_end = control.capped_page_index(end_page);
                    if page_names.len() <= names_end as usize {
                        page_names.resize(names_end as usize + 1, current_page_name.clone());
                    }
                    if !candidate.is_out_of_flow_descendant && names_end > current_page {
                        for name in &mut page_names[current_page as usize + 1..=names_end as usize]
                        {
                            name.clone_from(&current_page_name);
                        }
                    }
                }
            }
            if consumes_pending_break {
                pending_break_source = None;
            }
            let candidate_break_after =
                page_break_is_forced(document, node_id, cascade.computed[node_id].break_after)
                    || candidate.deferred_named_break_after;
            let pending_source_is_ancestor = pending_break_source
                .is_some_and(|source| is_descendant_or_self(document, node_id, source));
            if candidate_break_after && !pending_source_is_ancestor {
                pending_break_source = Some(node_id);
            }
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
            let anchor_page = if effective_y.is_finite() && effective_y >= 0.0 {
                page_index_for_y(effective_y)
            } else {
                current_page
            };
            let note_page = current_page.max(anchor_page);
            check_candidate_page!('candidate_loop, note_page);
            let used = footnote_heights.entry(note_page).or_insert(0.0);
            let target_y = (page_origin(note_page) + page_step_at(note_page) - *used - height)
                .max(page_origin(note_page));
            materialize_y(document, node_id, target_y);
            *used += height;
            flow_shift -= height;
            max_page = max_page.max(note_page);
            saw_child = true;
            continue;
        }
        // Copy layout data before any break adjustment so the immutable node
        // borrow does not overlap the in-place location update below.
        let raw_y = candidate.raw_y;
        let height = candidate.height;
        if let Some(&header_id) = table_headers.rows.get(&node_id) {
            let header = table_headers
                .headers
                .get_mut(&header_id)
                .expect("prepared header");
            if header.placements.is_empty() {
                let y = header.source_y + flow_shift;
                materialize_y(document, header_id, y);
                header.source_y = y;
                let page = page_index_for_y(y);
                header.placements.insert(page, (page_origin(page), y));
            }
        }
        let mut effective_y = if uses_positioned_coordinates {
            current_abs_y(document, node_id)
        } else {
            raw_y + flow_shift
        };
        materialize_y(document, node_id, effective_y);
        let forced_before = page_break_is_forced(document, node_id, computed.break_before);
        let style = &document.nodes[node_id].style;
        let margin_top = used_style_length_percentage_auto(style.margin.top, content_width)
            .or_else(|| used_computed_length_percentage_or_auto(computed.margin.top, content_width))
            .unwrap_or(0.0);
        // The block-end margin of an in-flow box is truncated at the break
        // after it (CSS Fragmentation 3 §5.2), so it does not break the box.
        // A float's margin box stays whole.
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
        // A block with `break-inside: avoid-page` stays intact when its used
        // block size plus its trailing margin would cross the current page.
        // This is the direct block-flow case; nested formatting contexts still
        // require fragment-level break opportunities.
        let avoided_run_overflow = avoided_run_ends.get(&node_id).is_some_and(|end| {
            let run_height = *end - raw_y;
            let page_height = page_step_at(current_page);
            let destination_height = page_step_at(current_page.saturating_add(1));
            run_height.is_finite()
                && run_height > 0.0
                && run_height + margin_top.max(0.0) <= destination_height
                && effective_y > page_origin(current_page)
                && effective_y < page_origin(current_page) + page_height
                && effective_y + run_height > page_origin(current_page) + page_height
        });
        // Whether the box fits is decided at its normal-flow position,
        // before any relative offset.
        let relative_offset = accumulated_relative_offset_y(document, cascade, body_id, node_id);
        let flow_y = effective_y - relative_offset;
        // `break-inside` does not apply to inline boxes (CSS Fragmentation 3
        // §3.2): the block boxes inside an avoiding inline box break as they
        // would without it. An atomic inline, such as an inline table, is a
        // single box in its line and still moves as a whole.
        let page_overflow = matches!(
            computed.break_inside,
            raikiri_style::property::BreakInside::Avoid
                | raikiri_style::property::BreakInside::AvoidPage
        ) && (computed.display != DisplayValue::Inline
            || page_applies_to_box(document, cascade, node_id))
            // A box already at the top of its page would break inside on
            // any page, so moving it would only leave a blank page.
            && flow_y - margin_top.max(0.0) > page_origin(current_page) + 0.001
            && flow_y.is_finite()
            && flow_y >= page_origin(current_page)
            && flow_y < page_origin(current_page) + page_step_at(current_page)
            && flow_y + height + trailing_margin
                > page_origin(current_page) + page_step_at(current_page);
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
            && candidate.named_flex_context == last_named_context
            && last_named_was_zero_height
            && !last_named_had_display_none_descendant
            && last_named_raw_y.is_some_and(|previous| (raw_y - previous).abs() <= 0.001);
        let named_page_change = name_context_seen
            && candidate_page_name != comparison_page_name
            && !same_named_coordinate;
        // A forced break at the start of a page already opened by natural
        // overflow describes the same boundary and must not create a blank
        // page.  This matters when the page height changes after page zero:
        // the fixed-height pass may have considered the box to be on the
        // current page, while the scheduled pass places it exactly at the
        // next page origin.
        // The forced break sits before the box's margin box, so the test uses
        // its block-start margin edge. A border box that lands exactly on a
        // page origin only because the preceding collapsed margin filled the
        // previous page has not opened that page: the break still moves it,
        // and the margin after the forced break is kept.
        let forced_break_at_page_start = forced_before && {
            let margin_box_y = effective_y - margin_top.max(0.0);
            let margin_box_page = if margin_box_y.is_finite() && margin_box_y >= 0.0 {
                page_index_for_y(margin_box_y)
            } else {
                current_page
            };
            margin_box_page > current_page
                || (margin_box_y.is_finite()
                    && (margin_box_y - page_origin(current_page)).abs() <= 0.001)
        };
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
        let table_row_overflow = candidate.is_table_row
            && height + repeated_header_height
                <= page_step_at(current_page.saturating_add(1)) + 0.001
            && flow_y.is_finite()
            && flow_y >= page_origin(current_page)
            && flow_y < page_origin(current_page) + page_step_at(current_page)
            && flow_y + height > page_origin(current_page) + page_step_at(current_page);
        // A row group that is not already at the top of its page moves as a
        // whole, even when it is taller than a page and breaks there.
        let row_group = avoided_row_groups.get(&node_id).copied();
        let row_group_overflow = row_group.is_some_and(|(end, _)| {
            let page_end = page_origin(current_page) + page_step_at(current_page);
            flow_y.is_finite()
                && flow_y > page_origin(current_page) + 0.001
                && flow_y < page_end
                && flow_y + (end - raw_y) > page_end
        });
        // A trailing flex item may extend into the containing flex box's
        // continuation at the page edge. Earlier items still move intact when
        // they would cross a fragmentainer, preserving the existing column
        // flex pagination behavior.
        let flex_item_is_last = candidate.is_flex_item
            && box_parent(document, cascade, node_id, BoxParentScope::Document).is_some_and(
                |parent_id| {
                    let trailing_child = trailing_flex_child_by_parent
                        .entry(parent_id)
                        .or_insert_with(|| {
                            trailing_flex_child_for_pagination(document, cascade, parent_id)
                        });
                    *trailing_child == Some(node_id)
                },
            );
        // An item already at the top of its page, or one taller than the
        // next page, would cross a page edge wherever it went: pushing it
        // only leaves a blank page behind (CSS Fragmentation 3 §4.4 lets
        // monolithic content overflow a fresh fragmentainer).
        // The page top is judged by the margin box, since a block-start margin
        // already at the page top would stay with the item.
        let item_can_move = effective_y - margin_top.max(0.0) > page_origin(current_page) + 0.001
            && height <= page_step_at(current_page.saturating_add(1)) + 0.001;
        let flex_item_overflow = candidate.is_flex_item
            && !flex_item_is_last
            && item_can_move
            && effective_y.is_finite()
            && effective_y < page_origin(current_page) + page_step_at(current_page)
            && effective_y + height > page_origin(current_page) + page_step_at(current_page);
        let grid_item_overflow = candidate.is_grid_item
            && item_can_move
            && effective_y.is_finite()
            && effective_y < page_origin(current_page) + page_step_at(current_page)
            && effective_y + height > page_origin(current_page) + page_step_at(current_page);

        // A relative offset is not part of the flow gap between siblings, so
        // a positioned box cannot tell its margin apart from its offset.
        let collapses_with_previous_block = is_block_level_flow_box(computed.display)
            && matches!(computed.position, PositionValue::Static)
            && !candidate.is_text
            && !candidate.is_table_row
            && !candidate.is_flex_item
            && !candidate.is_grid_item
            && participates_in_margin_adjacency;
        let pending_break_applies = pending_break_source
            .is_some_and(|source| !is_descendant_or_self(document, node_id, source));
        let page_transition = saw_child
            && (pending_break_applies
                || (forced_before && !forced_break_at_page_start)
                || named_page_change
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
                // CSS Fragmentation 3 §5.2: margins adjoining an unforced
                // break are truncated, while a forced break keeps the
                // block-start margin of the box after it.
                let forced_transition = pending_break_applies
                    || (forced_before && !forced_break_at_page_start)
                    || named_page_change;
                let natural_page = if effective_y.is_finite() && effective_y >= 0.0 {
                    let page = page_index_for_y(effective_y);
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
                        && flow_max_page >= page
                        && margin_box_y.is_finite()
                    {
                        let margin_box_page = page_index_for_y(margin_box_y.max(0.0));
                        if (margin_box_y - page_origin(margin_box_page)).abs() <= 0.001 {
                            margin_box_page
                        } else {
                            margin_box_page.saturating_add(1)
                        }
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
                        target_y + margin_top.max(0.0)
                    } else if row_group_overflow {
                        target_y + row_group.map_or(0.0, |(_, lead)| lead) + relative_offset
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
                let row_flex_break_before = forced_before
                    && document.parent_of(node_id).is_some_and(|parent_id| {
                        cascade.computed[parent_id].display.is_flex_container()
                            && matches!(
                                cascade.computed[parent_id].flex_direction,
                                FlexDirectionValue::Row | FlexDirectionValue::RowReverse
                            )
                    });
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
                    if page_overflow && nested_block {
                        moved_in_bounded_ancestor = grow_ancestors_for_shift(
                            document,
                            cascade,
                            node_id,
                            node_delta,
                            body_id,
                            &mut pending_bounded_shifts,
                            &mut pending_context_growth,
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
                        // A `display: contents` ancestor has no box; its
                        // parent lays the moved box out among its siblings.
                        // A box pulled back over the break by a negative
                        // margin still leaves the flow after it at the break.
                        let flow_delta = node_delta.max(shift_delta);
                        let paragraph = ifc_paragraph_of_block(document, node_id);
                        let mut child = node_id;
                        let mut next = document.parent_of(node_id);
                        while let Some(parent) = next
                            && parent != body_id
                        {
                            next = document.parent_of(parent);
                            if cascade.computed[parent].display == DisplayValue::Contents {
                                continue;
                            }
                            if Some(parent) != paragraph
                                && !stacks_children_in_block_flow(document, cascade, parent)
                            {
                                break;
                            }
                            shift_later_block_siblings(
                                document, cascade, parent, child, flow_delta,
                            );
                            child = parent;
                        }
                    }
                    // A forced break on one wrapped row-flex item belongs to
                    // its whole flex line. Move same-line siblings together;
                    // other lines keep their existing flow coordinates.
                    if row_flex_break_before && let Some(parent_id) = document.parent_of(node_id) {
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
                    if node_target_y < target_y {
                        flow_shift += node_delta;
                        pending_underflow = Some((node_id, shift_delta - node_delta));
                    } else {
                        // A kept margin moves the box's descendants and its
                        // later siblings along with the box.
                        flow_shift += node_delta;
                    }
                    if page_overflow && is_floating_box_for_pagination(document, cascade, node_id) {
                        let (inherited, flow_page) = pending_float_shift
                            .filter(|(float_id, _, _)| *float_id == node_id)
                            .map_or((0.0, page_before_candidate), |(_, delta, page)| {
                                (delta, page)
                            });
                        pending_float_shift = Some((node_id, inherited + node_delta, flow_page));
                        // A clearing follower stays below the margin box,
                        // which a negative block-end margin pulls up.
                        if let Some(index) = float_side_index(computed.float) {
                            let context = float_context_root(document, cascade, node_id, body_id);
                            // Floors are kept in normal-flow coordinates,
                            // without the relative offsets of the ancestors.
                            let flow_target_y = node_target_y - relative_offset;
                            let outer_bottom = flow_target_y
                                + height
                                + document.nodes[node_id].unrounded_layout.margin.bottom;
                            // An unforced break truncates the block-start
                            // margin (CSS Fragmentation 3 §5.2), so the margin
                            // box then starts at the border box.
                            let outer_top = if forced_transition {
                                flow_target_y - margin_top
                            } else {
                                flow_target_y
                            };
                            let layout = &document.nodes[node_id].unrounded_layout;
                            record_float_edge(
                                &mut later_float_edges,
                                context,
                                FloatEdge {
                                    node: node_id,
                                    side: index,
                                    top: outer_top,
                                    bottom: outer_bottom,
                                    width: layout.size.width
                                        + layout.margin.left
                                        + layout.margin.right,
                                },
                            );
                            // The margin box then moved further than the
                            // border box, and a later float may have to
                            // follow it that far.
                            if !forced_transition {
                                later_float_shift =
                                    later_float_shift.max(inherited + node_delta + margin_top);
                            }
                        }
                        // A formatting context of its own holds its floats
                        // (CSS 2.1 §10.6.7), so an auto-height one grows to
                        // the moved float's margin box, and the flow after
                        // it moves down by the growth.
                        let context = float_context_root(document, cascade, node_id, body_id);
                        let context_style = &document.nodes[context].style;
                        if context != body_id
                            && (context_style.size.height.is_auto()
                                || definite_content_height(document, cascade, context).is_none())
                        {
                            let context_top = current_abs_y(document, context);
                            let float_bottom = current_abs_y(document, node_id)
                                + height
                                + document.nodes[node_id].unrounded_layout.margin.bottom;
                            let layout = &document.nodes[context].unrounded_layout;
                            let grown = (float_bottom - context_top
                                + layout.padding.bottom
                                + layout.border.bottom)
                                .min(
                                    max_border_box_height(document, cascade, context)
                                        .unwrap_or(f32::INFINITY),
                                );
                            let growth = grown - layout.size.height;
                            if growth > 0.001 {
                                document.nodes[context].unrounded_layout.size.height += growth;
                                grow_ancestors_for_shift(
                                    document,
                                    cascade,
                                    context,
                                    growth,
                                    body_id,
                                    &mut pending_bounded_shifts,
                                    &mut pending_context_growth,
                                    page_before_candidate,
                                );
                                pending_context_growth.push((context, growth));
                            }
                        }
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
                            if let Some(edges) = later_float_edges.get_mut(&context) {
                                edges.retain(|edge| edge.bottom > flow_top + 0.001);
                                if edges.is_empty() {
                                    later_float_edges.remove(&context);
                                }
                            }
                            if later_float_edges.is_empty() {
                                later_float_shift = 0.0;
                            }
                        }
                    }
                    effective_y += node_delta;
                }
                current_page = target_page;
                if pending_break_applies {
                    pending_break_source = None;
                }
            } else if effective_y.is_finite() && effective_y >= 0.0 {
                let mut natural_page = page_index_for_y(effective_y);
                // An unforced break that falls inside the collapsed margin
                // between this box and its preceding sibling truncates that
                // margin (CSS Fragmentation 3 §5.2): the box starts at the
                // top of the page instead of keeping the part of the margin
                // that crossed the page edge.
                let previous = box_parent(document, cascade, node_id, BoxParentScope::Document)
                    .and_then(|parent_id| previous_block_bottoms.get(&parent_id).copied())
                    .map(|(previous, bottom)| (previous, raw_y - bottom))
                    .filter(|(_, gap)| gap.is_finite() && *gap > 0.0);
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
                    if natural_page > 0
                        && effective_y > page_top + 0.001
                        && previous_bottom <= page_top + 0.001
                        // Only a margin between adjoining siblings is
                        // truncated; a box that is not a candidate may lie
                        // between two nested candidates.
                        && adjoins_previous_sibling(document, cascade, node_id, previous)
                    {
                        // A forced break that already coincides with this
                        // page edge sits before the margin, which therefore
                        // stays on the new page.
                        let target_y = if forced_before {
                            page_top + margin_top
                        } else {
                            page_top
                        };
                        let delta = target_y - effective_y;
                        margins_moved_boxes |= delta != 0.0;
                        document.nodes[node_id].unrounded_layout.location.y += delta;
                        follow_moved_ifc_block(document, node_id, delta);
                        flow_shift += delta;
                        effective_y += delta;
                        if target_y < page_top {
                            // Descendants follow the negative margin; later
                            // siblings still follow the forced break boundary.
                            pending_underflow = Some((node_id, page_top - target_y));
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
                && matches!(computed.display, DisplayValue::Inline)
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
                box_parent(document, cascade, node_id, BoxParentScope::Document)
        {
            if collapses_with_previous_block {
                previous_block_bottoms.insert(parent_id, (node_id, raw_y + height));
            } else {
                previous_block_bottoms.remove(&parent_id);
            }
        }

        if let Some(&header_id) = table_headers.rows.get(&node_id) {
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
                    let origin = page_origin(current_page);
                    let table_y = current_abs_y(document, header.table);
                    materialize_y(document, header.table, table_y + origin - header.source_y);
                    materialize_y(document, header_id, origin);
                    header.source_y = origin;
                    header.placements.clear();
                }
                header.body_started = true;
            }
            if !header.placements.contains_key(&current_page)
                && header.height + header.gap + height <= page_step_at(current_page) + 0.001
            {
                let origin = page_origin(current_page);
                let delta = (origin + header.height + header.gap - effective_y).max(0.0);
                effective_y += delta;
                materialize_y(document, node_id, effective_y);
                flow_shift += delta;
                header.placements.insert(current_page, (origin, origin));
            }
        }

        if name_participates_in_flow
            && (candidate.is_direct_body_element
                || candidate.is_named
                || candidate.is_rendered_leaf
                || candidate.deferred_named_break_after)
            && (!keep_local_page_type || page_transition || current_page > page_before_candidate)
        {
            current_page_name = candidate_page_name;
            if candidate.is_direct_body_element || candidate.named_flex_context.is_some() {
                last_named_context = candidate.named_flex_context;
                last_named_raw_y = candidate.is_named.then_some(raw_y);
                last_named_was_zero_height = candidate.is_named && height <= 0.001;
                last_named_had_display_none_descendant =
                    candidate.is_named && has_display_none_descendant(document, node_id);
            }
        }
        check_candidate_page!('candidate_loop, current_page);
        if page_names.len() <= current_page as usize {
            page_names.resize(current_page as usize + 1, None);
        }
        page_names[current_page as usize] = current_page_name.clone();

        max_page = max_page.max(current_page);
        if !candidate.is_out_of_flow_descendant {
            flow_max_page = flow_max_page.max(current_page);
        }
        if height > 0.0 && effective_y.is_finite() && effective_y >= 0.0 {
            // A box ending exactly at a page edge belongs to the preceding
            // page.  `f32::EPSILON` is too small at ordinary CSS coordinates
            // and rounds away, so use the half-open interval directly:
            // ceil(end / step) - 1.  The box remains visible on every slice it
            // intersects; line-level splitting is a later pass.
            let end = (effective_y + height).max(effective_y);
            if end.is_finite() && end > 0.0 {
                // A fixed-height direct block that fits on the physical paper
                // is monolithic in this minimal paginator.  Page border/padding
                // reduce the fragmentainer's content height, but must not make
                // the block manufacture a second page merely to expose its
                // overflow into the page decoration area.
                let monolithic_paper_fit = candidate.is_direct_body_element
                    && (insets.top > 0.0
                        || insets.right > 0.0
                        || insets.bottom > 0.0
                        || insets.left > 0.0)
                    && effective_y >= page_origin(current_page)
                    && effective_y + height <= page_origin(current_page) + page_box.height + 0.001
                    && style_dimension_length(document.nodes[node_id].style.size.height)
                        .is_some_and(|value| value >= 0.0);
                if !monolithic_paper_fit {
                    let end_page = page_index_for_end(end);
                    control.check_discovery_page_index(end_page)?;
                    max_page = max_page.max(end_page);
                    if !candidate.is_out_of_flow_descendant {
                        flow_max_page = flow_max_page.max(end_page);
                    }
                    // The pages a box continues onto keep the page type it
                    // started on until a later box names another, even when
                    // an enclosing box running onto them named them first.
                    let names_end = control.capped_page_index(end_page);
                    if page_names.len() <= names_end as usize {
                        page_names.resize(names_end as usize + 1, current_page_name.clone());
                    }
                    if !candidate.is_out_of_flow_descendant && names_end > current_page {
                        for name in &mut page_names[current_page as usize + 1..=names_end as usize]
                        {
                            name.clone_from(&current_page_name);
                        }
                    }
                }
            }
        }

        if name_participates_in_flow
            && (candidate.is_named || (candidate.is_rendered_leaf && height > 0.0))
        {
            let context =
                enclosing_column_context(node_id, document, cascade, &mut column_context_cache);
            occupy_column_contexts(
                context,
                current_page,
                &mut seen_named_flex_contexts,
                document,
                cascade,
                &mut column_context_cache,
            );
        }
        if candidate.is_flex_item
            && let Some(context) = candidate.named_flex_context
        {
            preceding_flex_items.insert(context, (node_id, height, current_page));
        }
        let candidate_break_after = page_break_is_forced(document, node_id, computed.break_after)
            || candidate.deferred_named_break_after;

        let pending_source_is_ancestor = pending_break_source
            .is_some_and(|source| is_descendant_or_self(document, node_id, source));
        if candidate_break_after && !pending_source_is_ancestor {
            pending_break_source = Some(node_id);
        }
    }

    super::table::headers::finish_boxes(document, &table_headers);
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
            let end_page = page_index_for_end(end);
            control.check_discovery_page_index(end_page)?;
            max_page = max_page.max(end_page);
        }
    }
    // Pages past the last one the normal flow reached hold only boxes outside
    // it, and keep the page type the flow ended on, whatever page type was
    // active when one of those boxes first reached them.
    let names_end = control.capped_page_index(max_page);
    if page_names.len() <= names_end as usize {
        page_names.resize(names_end as usize + 1, current_page_name.clone());
    }
    if names_end > flow_max_page {
        for name in &mut page_names[flow_max_page as usize + 1..=names_end as usize] {
            name.clone_from(&current_page_name);
        }
    }

    // Truncating margins can move an ancestor of a fixed box. Restore only
    // its block position, preserving the already-corrected subtree layout.
    if margins_moved_boxes {
        super::fixed::restore_fixed_box_block_positions(document, cascade, body_id, content_height);
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

    control.check_discovery_page_index(max_page)?;
    let last_page = if control.page_limit_reached.get() {
        control.max_pages.unwrap_or(0).saturating_sub(1)
    } else {
        max_page
    };
    Ok((0..=last_page)
        .map(|page_index| PageSlice {
            page_index,
            content_origin_y: page_origin(page_index),
            page_name: page_names.get(page_index as usize).cloned().flatten(),
        })
        .collect())
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
    if !style.size.height.is_auto() && definite_content_height(document, cascade, parent).is_some()
    {
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
    let containing_height = box_parent(document, cascade, parent, BoxParentScope::Document)
        .and_then(|block| definite_content_height(document, cascade, block));
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
    let siblings_moved = moves_later_siblings(document, cascade, parent);
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
        if cascade.computed[child].display == DisplayValue::Contents {
            children.extend(child_node.children.iter().rev().copied());
            continue;
        }
        if child_node.kind() == NodeKind::Text
            || cascade.computed[child].display == DisplayValue::None
            || child_node.style.position == taffy::Position::Absolute
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
        let grown = unconstrained + (after - before).max(0.0);
        (grown.clamp(min, max).max(old), grown > max + 0.001)
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
fn definite_content_height(
    document: &Document,
    cascade: &CascadeResult,
    block: usize,
) -> Option<f32> {
    let node = &document.nodes[block];
    let raw = node.style.size.height.into_raw();
    let definite = if raw.is_calc() || raw.tag() == taffy::CompactLength::PERCENT_TAG {
        box_parent(document, cascade, block, BoxParentScope::Document).is_none_or(|containing| {
            document.nodes[containing].kind() == NodeKind::Document
                || definite_content_height(document, cascade, containing).is_some()
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
            if !takes_no_block_space(document, cascade, found) {
                return false;
            }
        }
        if cascade.computed[parent].display != DisplayValue::Contents {
            return false;
        }
        current = parent;
    }
}

/// Whether `node_id` is a block box that takes no block space, so that only
/// margins lie between the boxes on either side of it.
fn takes_no_block_space(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    let layout = &node.unrounded_layout;
    node.kind() == NodeKind::Element
        && matches!(
            cascade.computed[node_id].display,
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
        || cascade.computed[node_id].display == DisplayValue::None
        || node.style.position == taffy::Position::Absolute
        || is_floating_box_for_pagination(document, cascade, node_id)
    {
        return;
    }
    if node.kind() != NodeKind::Text && cascade.computed[node_id].display == DisplayValue::Contents
    {
        for &id in node.children.iter().rev() {
            push_in_flow_boxes_rev(document, cascade, id, boxes);
        }
        return;
    }
    boxes.push(node_id);
}

/// Grows the auto-height ancestors of `node_id`, which moved down by
/// `delta`, to hold it. The part of the shift an ancestor does not pass on
/// to the flow after it, because its height cannot grow or a float in it
/// already reaches below the moved content, is recorded in
/// `pending_bounded_shifts` with `flow_page`, the page the flow returns to
/// past that ancestor. An ancestor that grows more than the flow inside
/// it moved is recorded in `pending_growth`, for the flow after it to
/// follow. Returns whether an ancestor's height kept it from holding the
/// moved content.
#[allow(clippy::too_many_arguments)]
fn grow_ancestors_for_shift(
    document: &mut Document,
    cascade: &CascadeResult,
    node_id: usize,
    delta: f32,
    body_id: usize,
    pending_bounded_shifts: &mut Vec<(usize, f32, u32)>,
    pending_growth: &mut Vec<(usize, f32)>,
    flow_page: u32,
) -> bool {
    let mut bounded = false;
    let mut child = node_id;
    // How far the flow after `child` moved, and how much `child` moved or
    // grew: a float hanging below its content can take up part of the move.
    let mut flow_delta = delta;
    let mut size_delta = delta;
    let mut next = document.parent_of(child);
    while let Some(parent) = next
        && parent != body_id
    {
        next = document.parent_of(parent);
        // A `display: contents` element has no box to grow; its parent lays
        // the moved box out.
        if cascade.computed[parent].display == DisplayValue::Contents {
            continue;
        }
        shift_later_block_siblings(document, cascade, parent, child, flow_delta);
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
            pending_bounded_shifts.push((parent, flow_delta, flow_page));
            bounded = true;
            break;
        };
        let (old_in_flow, in_flow) = grown.in_flow_heights;
        let limit = flow_delta.max(size_delta);
        let size_absorbed = (grown.height - height).clamp(0.0, limit);
        let flow_absorbed = (in_flow - old_in_flow).clamp(0.0, limit);
        if flow_absorbed < flow_delta - 0.001 {
            pending_bounded_shifts.push((parent, flow_delta - flow_absorbed, flow_page));
            bounded |= grown.capped;
        } else if flow_absorbed > flow_delta + 0.001 {
            // A box holding its floats grew around a float that moved with
            // the content it overflowed: the flow after the box follows.
            pending_growth.push((parent, flow_absorbed - flow_delta));
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
    }
    bounded
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
        cascade.computed[block].display,
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
                cascade.computed[child].position,
                PositionValue::Absolute | PositionValue::Fixed
            );
        match document.layout_parent_of(child) {
            Some(parent) if !placed_apart && parent == body_id => return true,
            // A `display: contents` element has no box; its parent lays the
            // child out.
            Some(parent)
                if !placed_apart
                    && (cascade.computed[parent].display == DisplayValue::Contents
                        || moves_later_siblings(document, cascade, parent)
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
            cascade.computed[item].position,
            PositionValue::Absolute | PositionValue::Fixed
        )
    {
        return None;
    }
    let container = document.layout_parent_of(item).filter(|&container| {
        let computed = &cascade.computed[container];
        computed.display == DisplayValue::Flex
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
                        let computed = &cascade.computed[child];
                        computed.display != DisplayValue::None
                            && !matches!(
                                computed.position,
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
                && cascade.computed[child].display != DisplayValue::Contents
                && other_width > room + 0.001
                && (other.location.y + other.size.height + other.margin.bottom <= top + 0.001
                    || other.location.y - other.margin.top >= bottom - 0.001)
        })
        .then_some(container)
}

/// Moves the flex lines of `container` below `item`, a flex item alone on
/// its line that grew by `shift` at its block-end edge, down by `shift`: in
/// a row flex container they follow the line that grew.
fn shift_later_flex_lines(
    document: &mut Document,
    cascade: &CascadeResult,
    container: usize,
    item: usize,
    shift: f32,
) {
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
                    cascade.computed[child].position,
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
fn moves_later_siblings(document: &Document, cascade: &CascadeResult, parent: usize) -> bool {
    matches!(
        cascade.computed[parent].display,
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
fn shift_later_block_siblings(
    document: &mut Document,
    cascade: &CascadeResult,
    parent: usize,
    child: usize,
    delta: f32,
) {
    if !moves_later_siblings(document, cascade, parent) || delta <= 0.0 {
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
        if cascade.computed[id].display == DisplayValue::Contents {
            stack.extend(document.nodes[id].children.iter().rev().copied());
        } else {
            boxes.push(id);
        }
    }
    let Some(index) = boxes.iter().position(|&id| id == child) else {
        return;
    };
    for &sibling in &boxes[index + 1..] {
        let computed = &cascade.computed[sibling];
        if document.nodes[sibling].kind() != NodeKind::Element
            || computed.display == DisplayValue::None
            || matches!(
                computed.position,
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

/// Adds a moved float to the floats of its formatting context. A float
/// that moves again replaces its earlier place.
fn record_float_edge(edges: &mut HashMap<usize, Vec<FloatEdge>>, context: usize, edge: FloatEdge) {
    let edges = edges.entry(context).or_default();
    edges.retain(|earlier| earlier.node != edge.node);
    edges.push(edge);
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
    while let Some(parent) = box_parent(document, cascade, current, BoxParentScope::Document)
        && parent != body_id
    {
        if holds_floats(document, cascade, parent) {
            return parent;
        }
        current = parent;
    }
    body_id
}

/// Whether the box `node_id` establishes an independent block formatting
/// context (CSS 2.1 §9.4.1), which holds the floats inside it.
fn holds_floats(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    let computed = &cascade.computed[node_id];
    let style = &document.nodes[node_id].style;
    let item =
        box_parent(document, cascade, node_id, BoxParentScope::Document).is_some_and(|container| {
            cascade.computed[container]
                .display
                .is_flex_or_grid_container()
        });
    item || !matches!(
        computed.display,
        DisplayValue::Block | DisplayValue::ListItem | DisplayValue::Inline
    ) || matches!(computed.float, FloatValue::Left | FloatValue::Right)
        || matches!(style.position, taffy::Position::Absolute)
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
    let basis = box_parent(document, cascade, node_id, BoxParentScope::Document)
        .and_then(|parent| definite_content_height(document, cascade, parent));
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
            && (definite_content_height(document, cascade, id).is_some()
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
fn grid_stacks_items(document: &Document, cascade: &CascadeResult, grid: usize) -> bool {
    let node = &document.nodes[grid];
    if node.grid_column_count != 1
        || !grid_rows_size_to_content(document, grid)
        || !node.style.size.height.is_auto()
            && definite_content_height(document, cascade, grid).is_some()
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
fn has_side_by_side_ancestor(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    body_id: usize,
) -> bool {
    let mut current = document.parent_of(node_id);
    while let Some(id) = current {
        let display = cascade.computed[id].display;
        if document.nodes[id].multicol.is_some()
            || display.is_flex_container()
            || display.is_grid_container() && !grid_stacks_items(document, cascade, id)
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
fn stacks_children_in_block_flow(document: &Document, cascade: &CascadeResult, id: usize) -> bool {
    moves_later_siblings(document, cascade, id)
        || cascade.computed[id].display.is_grid_container()
            && grid_stacks_items(document, cascade, id)
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
