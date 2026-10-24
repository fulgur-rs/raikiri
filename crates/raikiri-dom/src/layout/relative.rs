//! Used offsets of relatively positioned boxes whose offset the painter
//! applies instead of layout.

use super::BoxParentScope;
use super::box_tree::BoxTree;
use super::ifc::projection::REPLACED_BOX_TAGS;
use super::ifc::style::inset_precedence;
use super::multicol::spanning::is_spanner_in;
use super::page::used_computed_length_percentage_or_auto;
use crate::document::Document;
use crate::fragment::FragmentationContext;
use crate::node::{BoxPropertiesRef, Node};
use raikiri_style::property::{DisplayValue, FloatValue, PositionValue, WritingMode};
use raikiri_style::{CascadeResult, ComputedLengthPercentageOrAuto, ComputedValues};
use raikiri_traits::NodeKind;

/// The used horizontal and vertical insets of `node_id`, whatever its
/// `position`. When opposing insets both apply, the one on the start side
/// of the containing block's writing mode and direction wins (CSS Position
/// 3 §3.3). `None` stands for `auto`.
///
/// Percentages and `calc()` resolve against the content box of the
/// containing block: the nearest ancestor box that is not a non-atomic
/// inline, and inside a multi-column container the column box of a child
/// that does not span the columns. The root element uses the page-area
/// ICB; the body uses its ordinary ancestor box. The width is known; a
/// percentage of the height counts as `auto` unless layout gave the
/// containing block a definite height (CSS 2.1 §9.3.2, §10.5): a specified
/// one, or one its flex, grid or absolute positioning fixed.
pub(crate) fn used_relative_insets(
    document: &Document,
    cascade: &CascadeResult,
    cv: &ComputedValues,
    node_id: usize,
    precedence: Precedence,
) -> (Option<f32>, Option<f32>) {
    relative_insets_with_box_tree(
        document,
        cascade,
        cv,
        node_id,
        precedence,
        BoxTree::for_cascade(document, cascade),
    )
}

/// The insets of a box captured by the current layout pass.
pub(crate) fn layout_relative_insets(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    precedence: Precedence,
) -> (Option<f32>, Option<f32>) {
    relative_insets_with_box_tree(
        document,
        cascade,
        &cascade.computed[node_id],
        node_id,
        precedence,
        BoxTree::for_layout(document),
    )
}

fn relative_insets_with_box_tree(
    document: &Document,
    cascade: &CascadeResult,
    cv: &ComputedValues,
    node_id: usize,
    precedence: Precedence,
    boxes: BoxTree<'_>,
) -> (Option<f32>, Option<f32>) {
    let block = containing_block(document, boxes, node_id);
    let is_root = || super::ifc::assign::is_layout_root(document, node_id);
    // The ICB takes its writing mode and direction from the document element.
    // An anonymous cell takes them from its source row.
    let source = if is_root() {
        Some(node_id)
    } else {
        block.map(|block| block.source)
    };
    let used_mode = crate::layout::dispatch::used_writing_mode(document, source.unwrap_or(node_id));
    // The bridge flips the direction of a `sideways-lr` box laid out on
    // vertical axes, whose inline axis runs from bottom to top; the writing
    // mode below already says so.
    let rtl = source.is_some_and(|block| {
        let vertical_axes = crate::layout::bridge::uses_vertical_axes(cascade, block);
        let flipped = used_mode == WritingMode::SidewaysLr && vertical_axes;
        let direction = super::bridge::direction_for_axes(&cascade.computed[block], vertical_axes);
        (direction == taffy::Direction::Rtl) != flipped
    });
    let mode = match used_mode {
        WritingMode::VerticalRl => shodo::geometry::WritingMode::VerticalRl,
        WritingMode::VerticalLr => shodo::geometry::WritingMode::VerticalLr,
        WritingMode::SidewaysRl => shodo::geometry::WritingMode::SidewaysRl,
        WritingMode::SidewaysLr => shodo::geometry::WritingMode::SidewaysLr,
        _ => shodo::geometry::WritingMode::HorizontalTb,
    };
    let (right_wins, bottom_wins) = inset_precedence(mode, rtl);
    let length = |value| match value {
        ComputedLengthPercentageOrAuto::Px(px) => Some(Some(px)),
        ComputedLengthPercentageOrAuto::Auto
        | ComputedLengthPercentageOrAuto::MinContent
        | ComputedLengthPercentageOrAuto::MaxContent
        | ComputedLengthPercentageOrAuto::FitContent
        | ComputedLengthPercentageOrAuto::FitContentOf(_) => Some(None),
        ComputedLengthPercentageOrAuto::Percent(_) | ComputedLengthPercentageOrAuto::Calc(_) => {
            None
        }
    };
    // The bases are only looked up for an inset that needs them.
    let resolve = |value, basis: &dyn Fn() -> Option<f32>| {
        length(value).unwrap_or_else(|| {
            basis().and_then(|basis| used_computed_length_percentage_or_auto(value, basis))
        })
    };
    let width = || {
        if is_root() {
            return Some(document.fixed_page_area.unwrap_or_default().0);
        }
        let block = document.ifc_layout_node(block?.key)?;
        Some(if is_spanner_in(document, cascade, node_id) {
            block.layout.content_box_width()
        } else {
            column_content_width(block)
        })
    };
    let height = || {
        if is_root() {
            return Some(document.fixed_page_area.unwrap_or_default().1);
        }
        document.ifc_layout_node(block?.key)?.percent_basis_height
    };
    let pick = |start: Option<f32>, end: Option<f32>, end_wins: bool| {
        if end_wins {
            end.or(start)
        } else {
            start.or(end)
        }
    };
    let left = resolve(cv.left, &width);
    let right = resolve(cv.right, &width).map(|right| -right);
    let top = resolve(cv.top, &height);
    let bottom = resolve(cv.bottom, &height).map(|bottom| -bottom);
    match precedence {
        Precedence::WritingMode => (
            pick(left, right, right_wins),
            pick(top, bottom, bottom_wins),
        ),
        Precedence::LeftTop => (left.or(right), top.or(bottom)),
    }
}

/// Whether layout places `node_id` without its relative offset, which the
/// painter applies instead: the body (`body`), a float outside a flex or
/// grid container, and a table caption, cell, row or row group. Inline-level
/// boxes are placed on their lines with the offset.
pub fn placed_without_relative_offset(
    document: &Document,
    cascade: &CascadeResult,
    body: Option<usize>,
    node_id: usize,
    cv: &ComputedValues,
) -> bool {
    placed_without_offset_with_properties(
        document,
        BoxTree::for_cascade(document, cascade),
        body,
        node_id,
        cv,
        BoxPropertiesRef::from_computed(cv),
    )
}

/// Whether the captured box's relative offset is left for paint.
pub(crate) fn placed_without_relative_offset_for_layout(
    document: &Document,
    cascade: &CascadeResult,
    body: Option<usize>,
    node_id: usize,
) -> bool {
    placed_without_offset_with_properties(
        document,
        BoxTree::for_layout(document),
        body,
        node_id,
        &cascade.computed[node_id],
        document.nodes[node_id].box_properties(),
    )
}

fn placed_without_offset_with_properties(
    document: &Document,
    boxes: BoxTree<'_>,
    _body: Option<usize>,
    node_id: usize,
    cv: &ComputedValues,
    properties: BoxPropertiesRef<'_>,
) -> bool {
    let float = matches!(cv.float, FloatValue::Left | FloatValue::Right)
        && document.layout_parent_of(node_id).is_none_or(|parent| {
            !boxes
                .display(parent)
                .is_some_and(DisplayValue::is_flex_or_grid_container)
        });
    let table_part = matches!(
        properties.display(),
        DisplayValue::TableCaption
            | DisplayValue::TableCell
            | DisplayValue::TableRow
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
    );
    (float || table_part) && !properties.display().is_inline_level()
}

/// Which of two opposing insets wins.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Precedence {
    /// The start side of the containing block's writing mode and direction
    /// (CSS Position 3 §3.3), as the painter applies.
    WritingMode,
    /// `left` and `top`, as taffy's block layout applies whatever the
    /// writing mode.
    LeftTop,
}

/// The containing block of a box: `key` names the box laid out, which may
/// be an anonymous table cell, and `source` the element whose style it
/// takes.
#[derive(Clone, Copy)]
struct ContainingBlock {
    key: usize,
    source: usize,
}

/// The nearest ancestor of `node_id` that can be its containing block:
/// `display: contents` elements, non-atomic inlines and table rows, row
/// groups and columns are skipped, so a table cell has its table (CSS
/// Tables 3 §3.5). A box that table fixup wrapped in an anonymous cell or
/// table (CSS Tables 3 §3.9) has that box instead.
fn containing_block(
    document: &Document,
    boxes: BoxTree<'_>,
    node_id: usize,
) -> Option<ContainingBlock> {
    let mut child = node_id;
    loop {
        let block = boxes.parent(child, BoxParentScope::Document)?;
        if let Some(key) = document.anonymous_cell_of(child) {
            return Some(ContainingBlock { key, source: block });
        }
        // A cell in a row outside any table has the anonymous table that
        // wraps the row, which takes its inherited style from the parent.
        if let Some(key) = document.anonymous_table_of(child) {
            return Some(ContainingBlock { key, source: block });
        }
        if !is_non_atomic_inline(document, boxes, block) && !is_table_part(document, boxes, block) {
            return Some(ContainingBlock {
                key: block,
                source: block,
            });
        }
        child = block;
    }
}

/// Whether `node_id` is a table row, row group, column or column group,
/// which lay out inside the table's grid.
fn is_table_part(document: &Document, boxes: BoxTree<'_>, node_id: usize) -> bool {
    document.nodes[node_id].kind() == NodeKind::Element
        && boxes.display(node_id).is_some_and(|display| {
            matches!(
                display,
                DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup
                    | DisplayValue::TableRow
                    | DisplayValue::TableColumnGroup
                    | DisplayValue::TableColumn
            )
        })
}

/// Whether the box is a non-atomic inline in the selected box tree.
fn is_non_atomic_inline(document: &Document, boxes: BoxTree<'_>, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    node.kind() == NodeKind::Element
        && !super::ifc::assign::is_layout_root(document, node_id)
        && boxes.properties(node_id).is_some_and(|properties| {
            properties.display().is_inline_flow()
                && !matches!(
                    properties.position(),
                    PositionValue::Absolute | PositionValue::Fixed
                )
        })
        && !node
            .tag_name()
            .is_some_and(|tag| REPLACED_BOX_TAGS.contains(&tag))
        && !boxes
            .parent(node_id, BoxParentScope::Document)
            .is_some_and(|parent| {
                boxes
                    .display(parent)
                    .is_some_and(DisplayValue::is_flex_or_grid_container)
            })
}

/// The content-box width of `block`, or of one of its columns when it is a
/// multi-column container.
fn column_content_width(block: &Node) -> f32 {
    let width = block.layout.content_box_width();
    block
        .multicol
        .and_then(|style| FragmentationContext::resolve(width, None, style))
        .map_or(width, |context| context.column_width)
}

/// The offset by which a `position: relative` box at `node_id` moves from
/// its layout location, as the painter applies it to boxes that layout
/// places without it; `(0, 0)` for any other `position`.
/// The supplied `cascade` determines ancestor boxes; `cv` remains the
/// caller-selected style of the box being offset.
pub fn used_relative_offset(
    document: &Document,
    cascade: &CascadeResult,
    cv: &ComputedValues,
    node_id: usize,
) -> (f32, f32) {
    if *BoxPropertiesRef::from_computed(cv).position() != PositionValue::Relative {
        return (0.0, 0.0);
    }
    let (dx, dy) = used_relative_insets(document, cascade, cv, node_id, Precedence::WritingMode);
    (dx.unwrap_or(0.0), dy.unwrap_or(0.0))
}

#[cfg(test)]
mod tests;

/// The painted relative offset of a box captured by the current layout pass.
pub(crate) fn used_relative_offset_for_layout(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> (f32, f32) {
    if !matches!(document.nodes[node_id].position(), PositionValue::Relative) {
        return (0.0, 0.0);
    }
    let (dx, dy) = layout_relative_insets(document, cascade, node_id, Precedence::WritingMode);
    (dx.unwrap_or(0.0), dy.unwrap_or(0.0))
}
