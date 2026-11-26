use super::*;
use crate::fragment::{FragmentRect, LayoutFragment};
use crate::layout::context::LayoutSession;
use crate::layout::ifc::projection::{ATOMIC_TAGS, REPLACED_BOX_TAGS};
use crate::node::FormattingContext;
use raikiri_layout_geometry::{Edges, Point, Size};
use raikiri_style::property::BreakInside;
use std::collections::HashMap;

pub(super) fn supports(
    tree: &LayoutSession<'_>,
    root: usize,
    context: FragmentationContext,
) -> bool {
    if !context
        .available_height
        .is_some_and(|height| height.is_finite() && height > 0.0)
        || !tree.nodes[root]
            .multicol
            .is_some_and(|style| style.horizontal)
    {
        return false;
    }
    let mut pending: Vec<_> = flow_children(tree, root)
        .into_iter()
        .map(|id| (id, 0usize, true, false, false))
        .collect();
    let mut visible = false;
    let mut needs_break_flow = false;
    let mut projected_floats = 0usize;
    let mut projected_text_lines = false;
    let mut edged_wrappers = false;
    // The last flag is whether the parent is a grid that [`collect`] places
    // row by row.
    while let Some((id, depth, projected, inside_formatting_context, in_row_grid)) = pending.pop() {
        if depth >= 128 {
            return false;
        }
        let node = layout_node(tree, id);
        if !node.is_in_document() || node.is_display_none() {
            continue;
        }
        // Rendered children are elements or text; flat-tree membership
        // already excludes comments, processing instructions and fragments.
        if let NodeData::Text(text) = &node.data {
            // Text inside an atomic box keeps that box's measured subtree.
            // Direct projected text would need translated line continuations.
            if projected && !text.text_content.trim().is_empty() {
                return false;
            }
            continue;
        }
        // Flex/grid item breaks need their own row/order propagation rules.
        // Only outer constraints enter this measured-context projection,
        // except the items of a grid that [`collect`] places row by row.
        if inside_formatting_context
            && (node.break_before != BreakBetween::Auto
                || node.break_after != BreakBetween::Auto
                || node.break_inside != BreakInside::Auto)
            && !in_row_grid
        {
            return false;
        }
        visible = true;
        if node.layout_position() == Position::Absolute {
            // A resumed plain wrapper would replay this unfragmented subtree
            // in every continuation. Atomic parents and root siblings paint once.
            if projected && depth > 0 {
                return false;
            }
            continue;
        }
        let floated = is_floated(tree, id);
        if projected && floated {
            projected_floats += 1;
            // Isolated float measurement cannot reconstruct sibling packing
            // against the narrower column. Preserve the previous strategy.
            if projected_floats > 1 {
                return false;
            }
        }
        projected_text_lines |= projected
            && !floated
            && matches!(node.display(), DisplayValue::Block | DisplayValue::ListItem)
            && node
                .ifc
                .as_ref()
                .and_then(|root| root.lines.as_ref())
                .is_some_and(|lines| !lines.lines.is_empty());
        // Generated content on a resumed wrapper has no line continuation
        // here. Joint normal-flow IFC measurement also owns float exclusion;
        // measuring that text in isolation would discard its narrowed lines.
        if projected
            && ((!atomic(tree, id) && node.has_before_or_after_content)
                || (!floated
                    && matches!(node.display(), DisplayValue::Block | DisplayValue::ListItem)
                    && node
                        .ifc
                        .as_ref()
                        .and_then(|root| root.lines.as_ref())
                        .is_some_and(|lines| lines.beside_floats)))
        {
            return false;
        }
        // This projection preserves already measured atomic subtrees, but
        // does not reconstruct block margins, clearance or block-axis wrapper
        // sizes. Leave those boxes to the existing geometry-preserving
        // strategies.
        if projected && has_box_constraints(tree, id) {
            return false;
        }
        // The block-axis edges of a wrapper are placed around the flow boxes
        // it holds, so a wrapper with edges needs one to hold. They are placed
        // once each, so cloned decorations stay with the other strategies. An
        // inline box takes no block-axis room (see `BlockEdges::of`).
        if projected
            && !atomic(tree, id)
            && node.display() != DisplayValue::Inline
            && has_block_edges(tree, id)
        {
            if node.clones_box_decorations || !has_in_flow_element_child(tree, id) {
                return false;
            }
            edged_wrappers = true;
        }
        // Keep existing strategies for contexts whose only constraints are
        // inside avoidance, line breaking or sizing. This seam handles the
        // between-box constraints that require rollback and propagation.
        needs_break_flow |= forced(node.break_before)
            || forced(node.break_after)
            || avoided(node.break_before)
            || avoided(node.break_after)
            || matches!(node.break_inside, BreakInside::AvoidColumn)
            // The other strategies do not place a grid row by its items.
            || (in_row_grid && node.break_inside != BreakInside::Auto);
        if node.multicol.is_some()
            || !(node.display().is_block_flex_or_grid_container()
                || matches!(
                    node.display(),
                    DisplayValue::Block
                        | DisplayValue::InlineBlock
                        | DisplayValue::FlowRoot
                        | DisplayValue::ListItem
                )
                || block_in_inline_wrapper(tree, id))
            || node.has_logical_min_block_size
        {
            return false;
        }
        let descendants_projected = projected && !atomic(tree, id);
        let descendants_inside_context =
            inside_formatting_context || node.display().is_block_flex_or_grid_container();
        let rows = projected && !is_floated(tree, id) && row_grid(tree, id);
        pending.extend(flow_children(tree, id).into_iter().map(|child| {
            (
                child,
                depth + 1,
                descendants_projected,
                descendants_inside_context,
                rows,
            )
        }));
    }
    // A full-width float can push lines below itself without narrowing any
    // final line. Preserve that shared vertical placement as well.
    // Floats take no block-flow advance here, so the edges of a wrapper
    // would not be placed around them.
    visible
        && needs_break_flow
        && !(projected_floats > 0 && (projected_text_lines || edged_wrappers))
}

/// A box that holds flow boxes and has no height of its own to place.
#[derive(Clone, Copy)]
struct Wrapper {
    node: usize,
    /// The inline start of its border box, from that of its parent fragment.
    offset: f32,
    /// The inline size of its border box.
    width: f32,
    edges: BlockEdges,
}

/// The block-axis margins of a wrapper, and its padding and border above and
/// below its content. With `box-decoration-break: slice`, the top edge is on
/// its first fragment only and the bottom edge on its last.
#[derive(Clone, Copy, Default)]
struct BlockEdges {
    margin_top: f32,
    top: f32,
    bottom: f32,
    margin_bottom: f32,
}

impl BlockEdges {
    /// The edges of the wrapper `id`, with percentages of the inline size of
    /// the column content box `basis` it is laid out in. An inline box that
    /// holds blocks takes no room for them (CSS 2, 10.8.1).
    fn of(tree: &LayoutSession<'_>, id: usize, basis: f32) -> Self {
        if tree.nodes[id].display() == DisplayValue::Inline {
            return Self::default();
        }
        let style = style_of(tree, id);
        let (margin, padding, border) = (style.margin(), style.padding(), style.border());
        let basis = basis.max(0.0);
        let inset = |padding: LengthPercentage, border: LengthPercentage| {
            super::multicol_resolve_inset(tree, padding, basis)
                + super::multicol_resolve_inset(tree, border, basis)
        };
        let resolve = |value: LengthPercentageAuto| {
            value
                .maybe_resolve(Some(basis))
                .filter(|value| value.is_finite())
                .unwrap_or(0.0)
        };
        Self {
            margin_top: resolve(margin.top),
            top: inset(padding.top, border.top),
            bottom: inset(padding.bottom, border.bottom),
            margin_bottom: resolve(margin.bottom),
        }
    }
}

/// Adjoining block-axis margins, which collapse into the largest positive one
/// plus the most negative one (CSS 2, 8.3.1).
#[derive(Clone, Copy, Default)]
struct Margins {
    positive: f32,
    negative: f32,
}

impl Margins {
    fn add(&mut self, margin: f32) {
        self.positive = self.positive.max(margin);
        self.negative = self.negative.min(margin);
    }

    fn take(&mut self) -> f32 {
        std::mem::take(self).collapsed()
    }

    fn collapsed(self) -> f32 {
        self.positive + self.negative
    }
}

/// The content box that the boxes being collected are placed in, as an inline
/// range from the border box of their parent fragment.
#[derive(Clone, Copy)]
struct ContentBox {
    start: f32,
    width: f32,
}

impl ContentBox {
    /// The column itself, which holds the children of the container.
    fn column(context: FragmentationContext) -> Self {
        Self {
            start: 0.0,
            width: context.column_width,
        }
    }
}

#[derive(Clone)]
struct FlowBox {
    node: usize,
    ancestors: Vec<Wrapper>,
    width: f32,
    height: f32,
    /// The inline start of the border box, from the border box of its parent
    /// fragment.
    inline_offset: f32,
    before: BreakBetween,
    after: BreakBetween,
    floated: bool,
    splittable: bool,
    /// How many of the innermost `ancestors` start at this box, which places
    /// their top margins and edges before it.
    opens: usize,
    /// How many of the innermost `ancestors` end at this box, which places
    /// their bottom edges and margins after it.
    closes: usize,
    /// The gap above a grid row, which joins the margins before it.
    gap: f32,
    /// The items of a grid row that the box stands for, placed in place of
    /// `node`. A row splits where the column ends only when all its items do.
    row: Vec<RowItem>,
}

/// An item of a grid row, from the border box of its grid and the top of its
/// row.
#[derive(Clone, Copy)]
struct RowItem {
    node: usize,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// Whether the box `id` breaks between columns like a box without
/// descendants, which this strategy cuts where the column ends.
fn splits_as_leaf(tree: &LayoutSession<'_>, id: usize) -> bool {
    !tree.nodes[id].display().is_block_flex_or_grid_container()
        && tree.nodes[id].display() != DisplayValue::InlineBlock
        // Replaced content has no internal break points. Painting a
        // continuation would rescale and replay the entire source.
        && !tree.nodes[id].tag_name().is_some_and(|tag| {
            ATOMIC_TAGS.contains(&tag) || REPLACED_BOX_TAGS.contains(&tag)
        })
        // Descendants keep their measured subtree; this seam has no
        // translated child continuations to replay in later columns.
        && !has_content(tree, id)
        && !matches!(
            tree.nodes[id].break_inside,
            BreakInside::Avoid | BreakInside::AvoidColumn
        )
}

/// Whether `id` holds rendered children, generated content, or text.
fn has_content(tree: &LayoutSession<'_>, id: usize) -> bool {
    has_rendered_element_child(tree, id)
        || tree.nodes[id].has_before_or_after_content
        || flow_children(tree, id).into_iter().any(|child| {
            let child = layout_node(tree, child);
            child.is_in_document()
                && !child.is_display_none()
                && matches!(&child.data, NodeData::Text(text) if !text.text_content.trim().is_empty())
        })
}

/// Whether the grid `id` is placed row by row: an item of it has a break
/// property, and each item is an element that spans one row of a grid
/// without block-axis constraints of its own.
fn row_grid(tree: &LayoutSession<'_>, id: usize) -> bool {
    let node = layout_node(tree, id);
    if node.display() != DisplayValue::Grid
        || node.multicol.is_some()
        || node.used_writing_mode.is_vertical()
        || node.has_logical_min_block_size
        || node.break_inside != BreakInside::Auto
        || node.layout_position() == Position::Absolute
        || is_floated(tree, id)
        || has_box_constraints(tree, id)
        || style_of(tree, id).size().height != Dimension::auto()
        || !style_of(tree, id).min_size().height.is_auto()
        || !style_of(tree, id).max_size().height.is_auto()
        || style_of(tree, id).overflow()
            != (Point {
                x: TaffyOverflow::Visible,
                y: TaffyOverflow::Visible,
            })
        || row_gap(tree, id).is_none()
        || style_of(tree, id).aspect_ratio().is_some()
        || node.grid_item_rows.is_empty()
        || node
            .grid_item_rows
            .iter()
            .any(|row| row.start.checked_add(1) != Some(row.end))
    {
        return false;
    }
    let items: Vec<_> = flow_children(tree, id)
        .into_iter()
        .filter(|&child| {
            let child = layout_node(tree, child);
            child.is_in_document()
                && !child.is_display_none()
                && !matches!(&child.data, NodeData::Text(text) if text.text_content.trim().is_empty())
        })
        .collect();
    // Items with content keep the old path unless they avoid breaks inside:
    // their lines and children have break points inside the row that this
    // strategy does not split at.
    items.len() == node.grid_item_rows.len()
        && items.iter().all(|&item| {
            matches!(layout_node(tree, item).data, NodeData::Element(_))
                && layout_node(tree, item).layout_position() != Position::Absolute
                && (!has_content(tree, item)
                    || matches!(
                        layout_node(tree, item).break_inside,
                        BreakInside::Avoid | BreakInside::AvoidColumn
                    ))
        })
        && items
            .iter()
            .map(|&item| layout_node(tree, item))
            .any(|item| {
                item.break_before != BreakBetween::Auto
                    || item.break_after != BreakBetween::Auto
                    || item.break_inside != BreakInside::Auto
            })
}

/// The row gap of the grid `id`, when it is a length: a percentage resolves
/// against a height the rows give, which this strategy does not settle.
fn row_gap(tree: &LayoutSession<'_>, id: usize) -> Option<f32> {
    match style_of(tree, id).gap().height.expand() {
        ExpandedLengthPercentage::Length(gap) => Some(gap.max(0.0)),
        ExpandedLengthPercentage::Percent(percent) => (percent == 0.0).then_some(0.0),
        ExpandedLengthPercentage::Calc(_) => None,
    }
}

/// The rows of the grid `id`, laid out `height` tall, as flow boxes holding
/// their items. The gap above a row goes before it like a margin, which a
/// break between columns truncates (CSS Grid 1, 12). Each row reaches down
/// to the gap below it; the first one starts at the top of the grid. `None`
/// when the grid has no rows or its row gap is not a length.
fn grid_row_boxes(tree: &LayoutSession<'_>, id: usize, height: f32) -> Option<Vec<FlowBox>> {
    let gap = row_gap(tree, id)?;
    let mut items = tree.nodes[id].grid_item_rows.to_vec();
    items.sort_by_key(|row| row.start);
    let mut boxes: Vec<FlowBox> = Vec::new();
    let mut tops: Vec<f32> = Vec::new();
    let mut index = 0;
    while index < items.len() {
        let start = items[index].start;
        let top = if boxes.is_empty() {
            0.0
        } else {
            items[index].block_start
        };
        let mut row = Vec::new();
        let (mut before, mut after) = (BreakBetween::Auto, BreakBetween::Auto);
        while index < items.len() && items[index].start == start {
            let item = items[index].item;
            let layout = tree.nodes[item].layout;
            row.push(RowItem {
                node: item,
                x: layout.location.x,
                y: layout.location.y - top,
                width: layout.size.width,
                height: layout.size.height,
            });
            before = propagated(before, descendant_edge(tree, item, true, 0));
            after = propagated(after, descendant_edge(tree, item, false, 0));
            index += 1;
        }
        boxes.push(FlowBox {
            node: row.first()?.node,
            ancestors: Vec::new(),
            width: 0.0,
            height: 0.0,
            inline_offset: 0.0,
            before,
            after,
            floated: false,
            // A row breaks where its items do, so all of them must.
            splittable: row.iter().all(|item| splits_as_leaf(tree, item.node)),
            opens: 0,
            closes: 0,
            gap: if tops.is_empty() { 0.0 } else { gap },
            row,
        });
        tops.push(top);
    }
    for (index, row) in boxes.iter_mut().enumerate() {
        let bottom = tops
            .get(index + 1)
            .map_or(height, |&next| next - gap)
            .max(tops[index]);
        row.height = bottom - tops[index];
    }
    Some(boxes)
}

impl FlowBox {
    /// The room above the box: the top margins and edges of the wrappers
    /// that start at it, collapsed with the `margins` before it. A top edge
    /// takes the margins above it; the rest stay in `margins`.
    fn lead(&self, margins: &mut Margins) -> f32 {
        let mut lead = 0.0;
        for wrapper in &self.ancestors[self.ancestors.len() - self.opens..] {
            margins.add(wrapper.edges.margin_top);
            if wrapper.edges.top > 0.0 {
                lead += margins.take() + wrapper.edges.top;
            }
        }
        margins.add(self.gap);
        lead + margins.take()
    }

    /// The room below the box taken by the bottom edges of the wrappers that
    /// end at it, from the innermost out. A bottom edge takes the margins
    /// before it; the bottom margins after the last edge stay in `margins`.
    fn trail(&self, margins: &mut Margins) -> f32 {
        let mut trail = 0.0;
        for wrapper in self.ancestors[self.ancestors.len() - self.closes..]
            .iter()
            .rev()
        {
            if wrapper.edges.bottom > 0.0 {
                trail += margins.take() + wrapper.edges.bottom;
            }
            margins.add(wrapper.edges.margin_bottom);
        }
        trail
    }
}

/// Whether the box has a margin, padding, border, inset or clearance, or,
/// unless it is atomic, a block-axis size limit or a clip. Of a box that is not
/// atomic, the margin, padding, border and inline size are no constraint here:
/// [`collect`] places such a wrapper in its column by its inline box, and
/// [`layout`] adds its block-axis edges before its first box and after its
/// last one.
fn has_box_constraints(tree: &LayoutSession<'_>, id: usize) -> bool {
    let zero = LengthPercentageAuto::length(0.0);
    let style = style_of(tree, id);
    let margin = style.margin();
    let (padding, border) = (style.padding(), style.border());
    if style.inset() != Edges::splat(LengthPercentageAuto::auto()) || style.clear() != Clear::None {
        return true;
    }
    if atomic(tree, id) {
        return margin != Edges::splat(zero)
            || padding != Edges::splat(LengthPercentage::ZERO)
            || border != Edges::splat(LengthPercentage::ZERO);
    }
    let visible = Point {
        x: TaffyOverflow::Visible,
        y: TaffyOverflow::Visible,
    };
    // The inline axis of an orthogonal flow is the block axis of the columns,
    // and an inline box that holds blocks has its padding, border and margin
    // only on its first and last fragments (CSS 2, 9.2.1.1).
    let node = layout_node(tree, id);
    ((node.used_writing_mode.is_vertical() || node.display() == DisplayValue::Inline)
        && has_inline_constraints(tree, id))
        || (node.used_writing_mode.is_vertical() && has_block_edges(tree, id))
        || !style.min_size().height.is_auto()
        || !style.max_size().height.is_auto()
        || style.overflow() != visible
}

/// Whether the box has a margin, padding or border above or below it.
fn has_block_edges(tree: &LayoutSession<'_>, id: usize) -> bool {
    let zero = LengthPercentageAuto::length(0.0);
    let style = style_of(tree, id);
    let (margin, padding, border) = (style.margin(), style.padding(), style.border());
    margin.top != zero
        || margin.bottom != zero
        || padding.top != LengthPercentage::ZERO
        || padding.bottom != LengthPercentage::ZERO
        || border.top != LengthPercentage::ZERO
        || border.bottom != LengthPercentage::ZERO
}

/// Whether the box has a margin, padding, border or size along the inline
/// axis, so that block flow does not make it as wide as the content box of its
/// parent.
fn has_inline_constraints(tree: &LayoutSession<'_>, id: usize) -> bool {
    let zero = LengthPercentageAuto::length(0.0);
    let style = style_of(tree, id);
    let (margin, padding, border) = (style.margin(), style.padding(), style.border());
    margin.left != zero
        || margin.right != zero
        || padding.left != LengthPercentage::ZERO
        || padding.right != LengthPercentage::ZERO
        || border.left != LengthPercentage::ZERO
        || border.right != LengthPercentage::ZERO
        || !style.size().width.is_auto()
        || !style.min_size().width.is_auto()
        || !style.max_size().width.is_auto()
}

fn forced(value: BreakBetween) -> bool {
    matches!(value, BreakBetween::Always | BreakBetween::Column)
}

fn avoided(value: BreakBetween) -> bool {
    matches!(value, BreakBetween::Avoid | BreakBetween::AvoidColumn)
}

fn atomic(tree: &LayoutSession<'_>, id: usize) -> bool {
    let node = layout_node(tree, id);
    let style = style_of(tree, id);
    style.float().is_floated()
        || (node.display().is_block_flex_or_grid_container()
            || matches!(
                node.display(),
                DisplayValue::InlineBlock | DisplayValue::FlowRoot
            ))
        || style.size().height != Dimension::auto()
        // The height follows from the width, which the children do not give.
        || style.aspect_ratio().is_some()
        || matches!(
            node.break_inside,
            BreakInside::Avoid | BreakInside::AvoidColumn
        )
        || !has_rendered_element_child(tree, id)
}

/// An inline box that holds only block-level boxes and no line of its own.
/// Block-in-inline splitting leaves no inline fragment with content around
/// those blocks (CSS 2.1, 9.2.1.1), so the box flows like a block wrapper.
pub(crate) fn block_in_inline_wrapper(tree: &Document, id: usize) -> bool {
    let node = layout_node(tree, id);
    node.display() == DisplayValue::Inline
        && node.formatting_context == FormattingContext::Block
        && node.ifc.is_none()
        && !in_paragraph_with_lines(tree, id)
        && has_rendered_element_child(tree, id)
        && node.children.iter().all(|&child| {
            let child = layout_node(tree, child);
            !child.is_in_document()
                || child.is_display_none()
                || if child.kind() == NodeKind::Element {
                    !child.display().is_inline_level() && child.display() != DisplayValue::Contents
                } else {
                    child.text_content().is_none_or(|text| {
                        text.chars()
                            .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
                    })
                }
        })
}

/// Whether the inline box belongs to a paragraph that lays out lines, such as
/// one holding preserved white space next to the blocks. The block-in-inline
/// split then leaves those lines around the blocks, and they are painted by
/// that paragraph rather than by a line-less wrapper.
fn in_paragraph_with_lines(tree: &Document, id: usize) -> bool {
    std::iter::successors(tree.nodes[id].parent, |&ancestor| {
        tree.nodes[ancestor].parent
    })
    .find(|&ancestor| tree.nodes[ancestor].display() != DisplayValue::Inline)
    .and_then(|ancestor| tree.nodes[ancestor].ifc.as_ref())
    .and_then(|root| root.lines.as_ref())
    .is_some_and(|lines| !lines.lines.is_empty())
}

/// Whether a child of the box is an element in normal flow.
fn has_in_flow_element_child(tree: &LayoutSession<'_>, id: usize) -> bool {
    flow_children(tree, id).into_iter().any(|child| {
        let node = &tree.nodes[child];
        node.is_in_document()
            && !node.is_display_none()
            && matches!(node.data, NodeData::Element(_))
            && node.layout_position() != Position::Absolute
            && !is_floated(tree, child)
    })
}

fn has_rendered_element_child(tree: &Document, id: usize) -> bool {
    flow_children(tree, id).into_iter().any(|child| {
        tree.nodes[child].is_in_document()
            && !tree.nodes[child].is_display_none()
            && matches!(tree.nodes[child].data, NodeData::Element(_))
    })
}

fn propagated(outer: BreakBetween, inner: BreakBetween) -> BreakBetween {
    if forced(inner) || (!forced(outer) && avoided(inner)) {
        inner
    } else if forced(outer) || avoided(outer) {
        outer
    } else {
        inner
    }
}

fn descendant_edge(
    tree: &LayoutSession<'_>,
    id: usize,
    before: bool,
    depth: usize,
) -> BreakBetween {
    let own = if before {
        tree.nodes[id].break_before
    } else {
        tree.nodes[id].break_after
    };
    if depth >= 128 || tree.nodes[id].display().is_block_flex_or_grid_container() {
        return own;
    }
    let children = flow_children(tree, id);
    let in_flow = |&&child: &&usize| {
        tree.nodes[child].is_in_document()
            && !tree.nodes[child].is_display_none()
            && !is_floated(tree, child)
            && tree.nodes[child].layout_position() != Position::Absolute
            && matches!(tree.nodes[child].data, NodeData::Element(_))
    };
    let child = if before {
        children.iter().find(in_flow)
    } else {
        children.iter().rev().find(in_flow)
    };
    child.map_or(own, |&child| {
        propagated(own, descendant_edge(tree, child, before, depth + 1))
    })
}

fn collect(
    tree: &mut LayoutSession<'_>,
    id: usize,
    context: FragmentationContext,
    content: ContentBox,
    ancestors: &mut Vec<Wrapper>,
    out: &mut Vec<FlowBox>,
) {
    if tree.fragment_tree.limit_exceeded
        || !tree.nodes[id].is_in_document()
        || tree.nodes[id].is_display_none()
        || !matches!(tree.nodes[id].data, NodeData::Element(_))
        || tree.nodes[id].layout_position() == Position::Absolute
    {
        return;
    }
    let (float, auto_width) = {
        let style = style_of(tree, id);
        (style.float(), style.size().width == Dimension::auto())
    };
    let floated = float.is_floated();
    let monolithic = atomic(tree, id);
    // A wrapper with an inline box of its own is as wide as block flow makes
    // it within its parent, which its margins, size limits and authored width
    // decide. Its padding and border then narrow what it holds.
    // A box that `justify-self` places at a position fits its content, so it
    // is sized and placed like one too.
    let justified = !floated
        && auto_width
        && crate::layout::dispatch::block_flow::justify_fits_content(tree, id);
    let inline_box = !monolithic && (justified || has_inline_constraints(tree, id));
    let input = if inline_box {
        super::column_child_layout_input(
            tree,
            id,
            content.width,
            context.available_height,
            AvailableSpace::MaxContent,
        )
    } else {
        let known_width = (tree.nodes[id].multicol_auto_width
            && !floated
            && (tree.nodes[id].display().is_block_flex_or_grid_container()
                || matches!(
                    tree.nodes[id].display(),
                    DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
                ))
            && auto_width
            && tree.nodes[id].image_intrinsic_box().is_none()
            && tree.nodes[id].tag_name() != Some("img"))
        .then(|| {
            if justified {
                super::column_fit_content_width(
                    tree,
                    id,
                    content.width,
                    context.available_height,
                    content.width,
                )
            } else {
                content.width
            }
        });
        super::column_child_input(
            known_width,
            content.width,
            context.available_height,
            AvailableSpace::MaxContent,
        )
    };
    let output = tree.compute_child_layout(NodeId::from(id), input);
    let before = descendant_edge(tree, id, true, 0);
    let after = descendant_edge(tree, id, false, 0);
    // The measured box belongs to a column, not the original full-width
    // containing block. Its parent determines block inline alignment; physical
    // float sides remain independent of inline direction.
    let parent_rtl = tree
        .taffy_parent_of(id)
        .is_some_and(|parent| style_of(tree, parent).direction() == Direction::Rtl);
    // A grid whose items have break properties is placed row by row, in a
    // wrapper as wide as the grid (CSS Grid 1, 12).
    if let Some(mut rows) = (!floated && row_grid(tree, id))
        .then(|| grid_row_boxes(tree, id, output.size.height.max(0.0)))
        .flatten()
    {
        let offset = content.start
            + crate::layout::dispatch::block_flow::justified_offset(
                tree,
                id,
                parent_rtl,
                content.width - output.size.width,
            )
            .unwrap_or(if parent_rtl {
                content.width - output.size.width
            } else {
                0.0
            });
        ancestors.push(Wrapper {
            node: id,
            offset,
            width: output.size.width,
            edges: BlockEdges::default(),
        });
        let start = out.len();
        for row in &mut rows {
            row.ancestors = ancestors.clone();
        }
        out.append(&mut rows);
        ancestors.pop();
        // A grid in rows has an item, so it has a row.
        let last = out.len() - 1;
        out[start].before = propagated(before, out[start].before);
        out[last].after = propagated(after, out[last].after);
        out[start].opens = 1;
        out[last].closes = 1;
        return;
    }
    if monolithic {
        out.push(FlowBox {
            node: id,
            ancestors: ancestors.clone(),
            width: output.size.width,
            height: output.size.height.max(0.0),
            inline_offset: content.start
                + match float {
                    Float::Right => content.width - output.size.width,
                    Float::Left => 0.0,
                    Float::None => crate::layout::dispatch::block_flow::justified_offset(
                        tree,
                        id,
                        parent_rtl,
                        content.width - output.size.width,
                    )
                    .unwrap_or(if parent_rtl {
                        content.width - output.size.width
                    } else {
                        0.0
                    }),
                },
            before,
            after,
            floated,
            splittable: !floated && splits_as_leaf(tree, id),
            opens: 0,
            closes: 0,
            gap: 0.0,
            row: Vec::new(),
        });
        return;
    }
    let (wrapper, inner) = if inline_box {
        wrapper_boxes(tree, id, content, output.size.width, parent_rtl)
    } else {
        (
            Wrapper {
                node: id,
                offset: content.start,
                width: content.width,
                edges: BlockEdges::default(),
            },
            ContentBox {
                start: 0.0,
                width: content.width,
            },
        )
    };
    let wrapper = Wrapper {
        edges: BlockEdges::of(tree, id, content.width),
        ..wrapper
    };
    let start = out.len();
    let depth = ancestors.len();
    ancestors.push(wrapper);
    for child in flow_children(tree, id) {
        collect(tree, child, context, inner, ancestors, out);
        if tree.fragment_tree.limit_exceeded {
            break;
        }
    }
    ancestors.pop();
    if out.len() > start {
        out[start].before = propagated(before, out[start].before);
        let last = out.len() - 1;
        out[last].after = propagated(after, out[last].after);
        // The wrapper starts at its first flow box and ends at its last one,
        // and so do the wrappers it holds that start or end there.
        let flow = start..out.len();
        if let Some(first) = flow.clone().find(|&index| !out[index].floated) {
            out[first].opens = out[first].opens.max(out[first].ancestors.len() - depth);
        }
        if let Some(last) = flow.rev().find(|&index| !out[index].floated) {
            out[last].closes = out[last].closes.max(out[last].ancestors.len() - depth);
        }
    }
}

/// The wrapper `id`, `width` wide, and the content box it holds its children
/// in, from its own border box.
fn wrapper_boxes(
    tree: &LayoutSession<'_>,
    id: usize,
    content: ContentBox,
    width: f32,
    parent_rtl: bool,
) -> (Wrapper, ContentBox) {
    let style = style_of(tree, id);
    let (margin_left, margin_right) =
        super::column_horizontal_margins(&style, content.width, width);
    let justified = crate::layout::dispatch::block_flow::justified_offset(
        tree,
        id,
        parent_rtl,
        content.width - margin_left - margin_right - width,
    );
    // `justify-self` places a box whose margins are not auto, and a
    // right-to-left parent otherwise aligns a narrower box to its right edge.
    let offset = if let Some(justified) = justified {
        content.start + margin_left + justified
    } else if parent_rtl {
        content.start + content.width - width - margin_right
    } else {
        content.start + margin_left
    };
    let (padding, border) = (style.padding(), style.border());
    let basis = content.width.max(0.0);
    let inset = |padding: LengthPercentage, border: LengthPercentage| {
        super::multicol_resolve_inset(tree, padding, basis)
            + super::multicol_resolve_inset(tree, border, basis)
    };
    let (start, end) = (
        inset(padding.left, border.left),
        inset(padding.right, border.right),
    );
    (
        Wrapper {
            node: id,
            offset,
            width,
            edges: BlockEdges::default(),
        },
        ContentBox {
            start,
            width: (width - start - end).max(0.0),
        },
    )
}

fn reserve_measurement_work(tree: &mut LayoutSession<'_>, root: usize) -> bool {
    // The remaining fragment capacity also bounds measurement, unless the
    // document is large enough that its work allowance outgrows the cap.
    let work_limit = tree.fragment_tree.work_limit(tree.nodes.len());
    let work = work_limit.saturating_sub(tree.fragment_tree.break_flow_work_used);
    let available = if work_limit > tree.fragment_tree.limit {
        work
    } else {
        tree.fragment_tree
            .limit
            .saturating_sub(tree.fragment_tree.fragments.len())
            .min(work)
    };
    let mut remaining = available;
    // Store one child cursor per ancestor, never an entire fanout. Each
    // nonatomic collection ancestor remeasures its subtree. Taffy's filtered
    // child indexing scans a prefix again, including hidden children and
    // whitespace predicates. Charge those scans before any extra measurement.
    let mut pending = vec![(root, 0usize, 0usize, true, 0usize)];
    while let Some((parent, next, measurements, collect_children, scanned)) = pending.last_mut() {
        let Some(&child) = tree.nodes[*parent].children.get(*next) else {
            pending.pop();
            continue;
        };
        *next += 1;
        let node = layout_node(tree, child);
        let in_document = node.is_in_document();
        let raw_cost = match &node.data {
            NodeData::Text(text) if in_document => text.text_content.len().max(1),
            _ => 1,
        };
        *scanned = scanned.saturating_add(raw_cost);
        let element = matches!(node.data, NodeData::Element(_));
        let hidden = node.is_display_none();
        let collecting = *collect_children
            && in_document
            && element
            && !hidden
            && node.layout_position() != Position::Absolute;
        let cost = measurements
            .saturating_mul(raw_cost)
            .saturating_add(if in_document {
                measurements.saturating_mul(*scanned)
            } else {
                0
            })
            .saturating_add(usize::from(collecting));
        let Some(next_remaining) = remaining.checked_sub(cost) else {
            tree.fragment_tree.limit_exceeded = true;
            return false;
        };
        remaining = next_remaining;
        // Hidden nodes are processed by a measured parent, but their own
        // compute callback returns immediately without visiting descendants.
        if !in_document || !element || hidden || (*parent == root && !collecting) {
            continue;
        }
        let measurements = measurements.saturating_add(usize::from(collecting));
        pending.push((
            child,
            0,
            measurements,
            collecting && !atomic(tree, child),
            0,
        ));
    }
    tree.fragment_tree.break_flow_work_used += available - remaining;
    true
}

fn fragment(
    node: usize,
    parent: Option<usize>,
    column: usize,
    rect: FragmentRect,
) -> LayoutFragment {
    LayoutFragment {
        node_id: node,
        parent,
        fragmentainer: column,
        rect,
        fragmentainer_clip: None,
        fragment_index: 0,
        fragment_count: 1,
        line_start: None,
        line_end: None,
    }
}

pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
    root: usize,
    context: FragmentationContext,
    fallback: Size<f32>,
    content_origin: Point<f32>,
) -> f32 {
    let Some(height) = context.available_height else {
        return fallback.height;
    };
    if tree.fragment_tree.limit_exceeded || !reserve_measurement_work(tree, root) {
        return fallback.height;
    }
    let mut boxes = Vec::new();
    for child in flow_children(tree, root) {
        collect(
            tree,
            child,
            context,
            ContentBox::column(context),
            &mut Vec::new(),
            &mut boxes,
        );
        if tree.fragment_tree.limit_exceeded {
            return fallback.height;
        }
    }
    tree.fragment_tree.retire_previous_roots(root);
    let Some(container) = tree.fragment_tree.try_push(fragment(
        root,
        None,
        context.column_index,
        FragmentRect {
            x: context.origin_x,
            y: context.origin_y,
            width: fallback.width,
            height: fallback.height,
        },
    )) else {
        return fallback.height;
    };
    // A lookahead run includes normal-flow siblings across parallel floats.
    // Floats occupy space for painting, but consume no block-flow advance.
    // Each run maps its first box to the advance of the whole run and the
    // lead of that first box, which placement replaces with the lead it finds.
    let mut runs = HashMap::<usize, (f32, f32)>::new();
    let mut previous: Option<usize> = None;
    let mut start = 0;
    let mut advance = 0.0;
    let mut first_lead = 0.0;
    let mut pending = Margins::default();
    for (index, item) in boxes.iter().enumerate() {
        if item.floated {
            continue;
        }
        let connected = previous.is_some_and(|prior| {
            !forced(boxes[prior].after)
                && !forced(item.before)
                && (avoided(boxes[prior].after) || avoided(item.before))
        });
        if !connected {
            pending = Margins::default();
        }
        let lead = item.lead(&mut pending);
        let extent = lead + item.height + item.trail(&mut pending);
        if connected {
            advance += extent;
            runs.insert(start, (advance, first_lead));
        } else {
            start = index;
            advance = extent;
            first_lead = lead;
        }
        previous = Some(index);
    }
    let column_bottom = content_origin.y + height;
    let mut column = 0usize;
    let mut cursor = 0.0f32;
    let mut previous_after = BreakBetween::Auto;
    let mut saw_flow_box = false;
    // The margins that adjoin the next flow box, not yet placed.
    let mut margins = Margins::default();
    let mut wrappers = HashMap::<(usize, usize), (usize, f32)>::new();
    // The last fragment of each wrapper, which reaches the end of its column
    // once the wrapper continues in a later one.
    let mut last_wrapper_fragment = HashMap::<usize, (usize, f32)>::new();
    for (index, item) in boxes.iter().enumerate() {
        let opened = &item.ancestors[item.ancestors.len() - item.opens..];
        // The margins and top edges of the wrappers that start at the box.
        let lead = item.lead(&mut { margins });
        // A run of boxes kept together moves to the next column whole when it
        // fits there, unless its first box breaks where the column ends: the
        // run is kept together after that break. A single box breaks there.
        let breaks_inside = item.splittable && cursor + lead + item.height > height;
        let run = runs
            .get(&index)
            .filter(|_| !breaks_inside)
            .map(|&(run, first_lead)| run + lead - first_lead);
        let extent = lead + item.height;
        // A forced edge on prior normal flow also determines an adjacent
        // float's column. Consume it once without charging float height to flow.
        let forced_break = (saw_flow_box && forced(previous_after))
            || (!item.floated && saw_flow_box && forced(item.before));
        // The margins next to an unforced break are truncated (CSS
        // Fragmentation 3, 5.2); those after a forced break are kept.
        let mut truncated = false;
        if forced_break
            || (!item.floated
                && (cursor > 0.0
                    && (run.is_some_and(|run| run <= height && cursor + run > height)
                        || (cursor + extent > height && !item.splittable)
                        || (cursor + lead >= height && item.height > 0.0))))
        {
            column = column.saturating_add(1);
            cursor = 0.0;
            previous_after = BreakBetween::Auto;
            truncated = !forced_break;
            // The margins before a break end at it, whatever broke there.
            margins = Margins::default();
        }
        // The top of the border box of each wrapper that starts at the box. A
        // wrapper without a top edge is where the margins below it end.
        let mut tops = vec![0.0f32; opened.len()];
        // The margins next to an empty box collapse through it (CSS 2, 8.3.1).
        let mut through = 0.0;
        if !item.floated {
            let mut waiting = 0;
            for (position, wrapper) in opened.iter().enumerate() {
                if !truncated {
                    margins.add(wrapper.edges.margin_top);
                }
                if wrapper.edges.top > 0.0 {
                    cursor += margins.take();
                    truncated = false;
                    for top in &mut tops[waiting..=position] {
                        *top = content_origin.y + cursor;
                    }
                    waiting = position + 1;
                    cursor += wrapper.edges.top;
                }
            }
            // The gap above a grid row is truncated like a margin.
            if !truncated {
                margins.add(item.gap);
            }
            if item.height > 0.0 || waiting > 0 {
                cursor += margins.take();
            } else {
                through = margins.collapsed();
                cursor += through;
            }
            for top in &mut tops[waiting..] {
                *top = content_origin.y + cursor;
            }
        }
        let start_column = column;
        let mut remaining = item.height;
        let mut first = true;
        // Which items of a grid row have a fragment yet.
        let mut placed = vec![false; item.row.len()];
        loop {
            if cursor >= height && !item.floated {
                column = column.saturating_add(1);
                cursor = 0.0;
            }
            let used = if item.splittable {
                remaining.min((height - cursor).max(0.0))
            } else {
                remaining
            };
            let x = content_origin.x + context.column_offset_x(column);
            let y = content_origin.y + cursor;
            let mut parent_fragment = container;
            let mut parent_y = 0.0;
            let first_opened = item.ancestors.len() - item.opens;
            for (depth, &ancestor) in item.ancestors.iter().enumerate() {
                let (id, top) = if let Some(&(id, top)) = wrappers.get(&(column, ancestor.node)) {
                    if !item.floated {
                        tree.fragment_tree.fragments[id].rect.height =
                            (y + used - top).max(tree.fragment_tree.fragments[id].rect.height);
                    }
                    (id, top)
                } else {
                    let top = if first && depth >= first_opened && !item.floated {
                        tops[depth - first_opened]
                    } else {
                        y
                    };
                    let Some(id) = tree.fragment_tree.try_push(fragment(
                        ancestor.node,
                        Some(parent_fragment),
                        column,
                        FragmentRect {
                            x: if parent_fragment == container {
                                x + ancestor.offset
                            } else {
                                ancestor.offset
                            },
                            y: top - parent_y,
                            width: ancestor.width,
                            height: if item.floated { 0.0 } else { y + used - top },
                        },
                    )) else {
                        return fallback.height;
                    };
                    wrappers.insert((column, ancestor.node), (id, top));
                    if let Some((earlier, earlier_top)) =
                        last_wrapper_fragment.insert(ancestor.node, (id, top))
                    {
                        let rect = &mut tree.fragment_tree.fragments[earlier].rect;
                        rect.height = rect.height.max(column_bottom - earlier_top);
                    }
                    (id, top)
                };
                parent_fragment = id;
                parent_y = top;
            }
            if !item.row.is_empty() {
                // The items of a grid row, in the fragment of their grid.
                let origin = x + item
                    .ancestors
                    .iter()
                    .map(|ancestor| ancestor.offset)
                    .sum::<f32>();
                // The part of the row in this column, from its top. What
                // overflows the row goes in its last part.
                let start = item.height - remaining;
                let end = if remaining - used <= 0.001 {
                    f32::INFINITY
                } else {
                    start + used
                };
                for (row_item, placed) in item.row.iter().zip(&mut placed) {
                    // An item starts in the part its top falls in, and the
                    // first part also holds what sticks out above the row.
                    let top = if *placed || start > 0.0 {
                        row_item.y.max(start)
                    } else {
                        row_item.y
                    };
                    let bottom = (row_item.y + row_item.height).min(end);
                    if (*placed && bottom <= top) || (!*placed && row_item.y >= end) {
                        continue;
                    }
                    // Past the fragment limit, the layout stops at the next box.
                    let pushed = tree.fragment_tree.try_push(fragment(
                        row_item.node,
                        Some(parent_fragment),
                        column,
                        FragmentRect {
                            x: row_item.x,
                            y: y + top - start - parent_y,
                            width: row_item.width,
                            height: (bottom - top).max(0.0),
                        },
                    ));
                    if let Some(id) = pushed.filter(|_| !*placed) {
                        *placed = true;
                        let mut layout = tree.nodes[row_item.node].layout;
                        layout.location = Point {
                            x: origin + row_item.x,
                            y: y + top - start,
                        };
                        tree.set_unrounded_layout(NodeId::from(row_item.node), &layout);
                        tree.fragment_tree.reparent_roots(row_item.node, id);
                    }
                }
                first = false;
            } else {
                let Some(id) = tree.fragment_tree.try_push(fragment(
                    item.node,
                    Some(parent_fragment),
                    column,
                    FragmentRect {
                        x: if parent_fragment == container {
                            x + item.inline_offset
                        } else {
                            item.inline_offset
                        },
                        y: y - parent_y,
                        width: item.width,
                        height: used,
                    },
                )) else {
                    return fallback.height;
                };
                if first {
                    let mut layout = tree.nodes[item.node].layout;
                    layout.size = Size {
                        width: item.width,
                        height: item.height,
                    };
                    layout.location = Point {
                        x: x + item
                            .ancestors
                            .iter()
                            .map(|ancestor| ancestor.offset)
                            .sum::<f32>()
                            + item.inline_offset,
                        y,
                    };
                    tree.set_unrounded_layout(NodeId::from(item.node), &layout);
                    tree.fragment_tree.reparent_roots(item.node, id);
                    first = false;
                }
            }
            if !item.floated {
                cursor += used;
            }
            remaining -= used;
            if !item.splittable || remaining <= 0.001 {
                break;
            }
            column = column.saturating_add(1);
            cursor = 0.0;
        }
        if column == start_column {
            cursor -= through;
        }
        if !item.floated {
            // The bottom edges and margins of the wrappers that end at the box,
            // from the innermost out. A bottom edge holds the margins before
            // it inside the wrappers around it.
            let closing = item.ancestors.len() - item.closes;
            for depth in (closing..item.ancestors.len()).rev() {
                let edges = item.ancestors[depth].edges;
                if edges.bottom > 0.0 {
                    cursor += margins.take() + edges.bottom;
                    let end = content_origin.y + cursor;
                    for ancestor in &item.ancestors[..=depth] {
                        if let Some(&(id, top)) = wrappers.get(&(column, ancestor.node)) {
                            let rect = &mut tree.fragment_tree.fragments[id].rect;
                            rect.height = rect.height.max(end - top);
                        }
                    }
                }
                margins.add(edges.margin_bottom);
            }
            previous_after = item.after;
            saw_flow_box = true;
        }
    }
    fallback.height
}

#[cfg(test)]
mod tests;
