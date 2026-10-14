//! Used offsets of relatively positioned boxes whose offset the painter
//! applies instead of layout.

use super::ifc::projection::REPLACED_BOX_TAGS;
use super::ifc::style::inset_precedence;
use super::multicol::spanning::is_spanner;
use super::page::used_computed_length_percentage_or_auto;
use super::{BoxParentScope, box_parent};
use crate::document::Document;
use crate::fragment::FragmentationContext;
use crate::node::Node;
use raikiri_style::property::{DisplayValue, PositionValue, WritingMode};
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
/// that does not span the columns. The body is laid out in the page area,
/// so that is its containing block. The width is always known; a
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
    let block = containing_block(document, cascade, node_id);
    let is_body = || super::ifc::assign::is_layout_root(document, node_id);
    // An anonymous cell takes its direction and writing mode from its row,
    // and the initial containing block of the body from the root element
    // (CSS Writing Modes 4 §8).
    let source = if is_body() {
        document
            .parent_of(node_id)
            .filter(|&root| document.nodes[root].kind() == NodeKind::Element)
    } else {
        block.map(|block| block.source)
    };
    let rtl =
        source.is_some_and(|block| document.nodes[block].style.direction == taffy::Direction::Rtl);
    let mode = match crate::taffy_impl::used_writing_mode(document, source.unwrap_or(node_id)) {
        WritingMode::VerticalRl => shodo::geometry::WritingMode::VerticalRl,
        WritingMode::VerticalLr => shodo::geometry::WritingMode::VerticalLr,
        WritingMode::SidewaysRl => shodo::geometry::WritingMode::SidewaysRl,
        WritingMode::SidewaysLr => shodo::geometry::WritingMode::SidewaysLr,
        _ => shodo::geometry::WritingMode::HorizontalTb,
    };
    let (right_wins, bottom_wins) = inset_precedence(mode, rtl);
    let length = |value| match value {
        ComputedLengthPercentageOrAuto::Px(px) => Some(Some(px)),
        ComputedLengthPercentageOrAuto::Auto | ComputedLengthPercentageOrAuto::MinContent => {
            Some(None)
        }
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
        if is_body() {
            return Some(document.fixed_page_area.0);
        }
        let block = document.ifc_layout_node(block?.key)?;
        Some(if is_spanner(document, node_id) {
            content_width(block)
        } else {
            column_content_width(block)
        })
    };
    let height = || {
        if is_body() {
            return Some(document.fixed_page_area.1);
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
/// Tables 3 §3.5). A box that table fixup wrapped in an anonymous cell (CSS
/// Tables 3 §3.9) has that cell instead.
fn containing_block(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<ContainingBlock> {
    let mut child = node_id;
    loop {
        let block = box_parent(document, cascade, child, BoxParentScope::Document)?;
        if let Some(key) = document.anonymous_cell_of(child) {
            return Some(ContainingBlock { key, source: block });
        }
        if !is_non_atomic_inline(document, cascade, block)
            && !is_table_part(document, cascade, block)
        {
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
fn is_table_part(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    document.nodes[node_id].kind() == NodeKind::Element
        && cascade.computed.get(node_id).is_some_and(|cv| {
            matches!(
                cv.display,
                DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup
                    | DisplayValue::TableRow
                    | DisplayValue::TableColumnGroup
                    | DisplayValue::TableColumn
            )
        })
}

/// Whether `node_id` lays out as a non-atomic inline box. The cascade keeps
/// `inline` on elements that layout blockifies (absolutely positioned boxes,
/// CSS 2.1 §9.7, and flex and grid items, CSS Display 3 §2.7) and on
/// replaced elements, which are atomic inlines.
fn is_non_atomic_inline(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    node.kind() == NodeKind::Element
        && cascade
            .computed
            .get(node_id)
            .is_some_and(|cv| cv.display == DisplayValue::Inline)
        && node.style.position != taffy::Position::Absolute
        && !node
            .tag_name()
            .is_some_and(|tag| REPLACED_BOX_TAGS.contains(&tag))
        && !box_parent(document, cascade, node_id, BoxParentScope::Document).is_some_and(|parent| {
            cascade
                .computed
                .get(parent)
                .is_some_and(|cv| cv.display.is_flex_or_grid_container())
        })
}

/// The content-box width of `block`.
fn content_width(block: &Node) -> f32 {
    let layout = &block.unrounded_layout;
    layout.size.width
        - layout.padding.left
        - layout.padding.right
        - layout.border.left
        - layout.border.right
}

/// The content-box width of `block`, or of one of its columns when it is a
/// multi-column container.
fn column_content_width(block: &Node) -> f32 {
    let width = content_width(block);
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
    if cv.position != PositionValue::Relative {
        return (0.0, 0.0);
    }
    let (dx, dy) = used_relative_insets(document, cascade, cv, node_id, Precedence::WritingMode);
    (dx.unwrap_or(0.0), dy.unwrap_or(0.0))
}

#[cfg(test)]
mod tests;
