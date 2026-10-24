//! Containing-block predicates and CSS-specific OOF corrections.
//!
//! The shared OOF pass places boxes against the containing block that claims
//! them. This module supplies the CSS predicates used to make those claims
//! and the correction of the root pass's auto margins for the writing mode,
//! which Taffy does not model. It overwrites no other coordinate of a placed
//! box: the final coordinates of an out-of-flow box come from Taffy's own
//! positioning, and from the alignment of the boxes of a container on the
//! axes of a vertical writing mode (see `vertical_oof`).

use super::box_tree::BoxTree;
use super::oof_containing_block::select_fixed_containing_block_from_ancestors;
use super::*;
use crate::layout::context::LayoutSession;
use crate::layout::taffy_adapter::MaybeResolve;
use crate::node::BoxPropertiesRef;
use raikiri_style::property::WillChange;
use taffy::CoreStyle;

/// Whether the element generates a non-atomic inline box: an inline box that
/// was not blockified and is not a replaced element or form control.
pub fn generates_inline_box(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    generates_inline_box_with_properties(document, id, cv, query_box_properties(document, id, cv))
}

fn query_box_properties<'a>(
    document: &Document,
    id: usize,
    cv: &'a ComputedValues,
) -> BoxPropertiesRef<'a> {
    let properties = BoxPropertiesRef::from_computed(cv);
    if super::ifc::assign::is_layout_root(document, id) {
        properties.blockify_root()
    } else {
        properties
    }
}

pub(super) fn generates_inline_box_with_properties(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
    properties: BoxPropertiesRef<'_>,
) -> bool {
    use crate::layout::ifc::projection::{ATOMIC_TAGS, REPLACED_BOX_TAGS};
    // spec: https://www.w3.org/TR/css-display-3/#transformations
    // Out-of-flow boxes and flex and grid items are blockified.
    let node = &document.nodes[id];
    let tag = node.tag_name().unwrap_or("");
    properties.display().is_inline_flow()
        && !super::ifc::assign::is_layout_root(document, id)
        && !matches!(
            properties.position(),
            PositionValue::Absolute | PositionValue::Fixed
        )
        && cv.float == FloatValue::None
        && !node.is_inline_svg_root()
        && !ATOMIC_TAGS.contains(&tag)
        && !REPLACED_BOX_TAGS.contains(&tag)
        // The bridge asks before rebuilding anonymous cells. Inspect the
        // current source ancestry rather than the preceding pass's wrappers.
        && !BoxTree::for_layout(document)
            .parent(id, BoxParentScope::LayoutRoot)
            .is_some_and(|parent| document.nodes[parent].display().is_flex_or_grid_container())
}

/// Whether the element's `transform` applies to it: layout makes it the
/// containing block of its fixed descendants, and the painter moves it.
pub fn is_transformable(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    is_transformable_with_properties(document, id, cv, query_box_properties(document, id, cv))
}

/// Whether the element has a transform that applies to it: a non-empty
/// `transform` on a transformable box.
pub fn has_applied_transform(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    !cv.transform.is_empty() && is_transformable(document, id, cv)
}

/// Whether the captured box accepts transforms; shared with page projection.
pub(crate) fn is_transformable_for_layout(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
) -> bool {
    is_transformable_with_properties(document, id, cv, document.nodes[id].box_properties())
}

fn is_transformable_with_properties(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
    properties: BoxPropertiesRef<'_>,
) -> bool {
    // spec: https://www.w3.org/TR/css-transforms-1/#terminology
    // Non-replaced inline and table-column boxes are not transformable.
    !generates_table_column_box(document, id, cv, properties)
        && !generates_inline_box_with_properties(document, id, cv, properties)
}

/// Whether the element's `display: table-column` or `table-column-group`
/// survives blockification (CSS Display 3 §2.7): the root, out-of-flow boxes
/// and flex and grid items become block boxes instead.
fn generates_table_column_box(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
    properties: BoxPropertiesRef<'_>,
) -> bool {
    matches!(
        properties.display(),
        DisplayValue::TableColumn | DisplayValue::TableColumnGroup
    ) && !super::ifc::assign::is_layout_root(document, id)
        && !matches!(
            properties.position(),
            PositionValue::Absolute | PositionValue::Fixed
        )
        && cv.float == FloatValue::None
        && !BoxTree::for_layout(document)
            .parent(id, BoxParentScope::LayoutRoot)
            .is_some_and(|parent| document.nodes[parent].display().is_flex_or_grid_container())
}

/// Whether layout and paint containment apply to the element.
fn containable(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
    properties: BoxPropertiesRef<'_>,
) -> bool {
    // spec: https://www.w3.org/TR/css-contain-2/#containment-layout
    // They have no effect on internal table boxes other than cells, and on
    // non-atomic inline boxes.
    !matches!(
        properties.display(),
        DisplayValue::Contents
            | DisplayValue::TableRow
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
            | DisplayValue::TableColumn
            | DisplayValue::TableColumnGroup
    ) && !generates_inline_box_with_properties(document, id, cv, properties)
}

/// Whether this fixed box uses the page area instead of an ancestor box.
pub(super) fn is_page_fixed(document: &Document, cascade: &CascadeResult, id: usize) -> bool {
    document.nodes[id].position() == &PositionValue::Fixed
        && matches!(
            select_fixed_containing_block_from_ancestors(document, cascade, id),
            super::oof_containing_block::ContainingBlockSelection::Page
        )
}

/// Whether `id` is the containing block of its fixed-position descendants.
pub fn establishes_fixed_containing_block(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
) -> bool {
    fixed_containing_block_with_properties(document, id, cv, query_box_properties(document, id, cv))
}

/// Whether the captured layout box contains its fixed-position descendants.
pub(crate) fn establishes_fixed_containing_block_for_layout(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
) -> bool {
    fixed_containing_block_with_properties(document, id, cv, document.nodes[id].box_properties())
}

fn fixed_containing_block_with_properties(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
    properties: BoxPropertiesRef<'_>,
) -> bool {
    // spec: https://www.w3.org/TR/css-transforms-1/#containing-block-for-all-descendants
    // spec: https://drafts.fxtf.org/filter-effects-1/#FilterProperty
    // spec: https://www.w3.org/TR/css-contain-2/#containment-layout
    // spec: https://www.w3.org/TR/css-will-change-1/#will-change
    // A box that is not generated contains nothing (CSS Display 3 §2.5).
    // The style is looked at first: almost no element sets any of these.
    let transform = !cv.transform.is_empty() || cv.will_change.contains(WillChange::TRANSFORM);
    let filter = !cv.filter.is_empty() || cv.will_change.contains(WillChange::FILTER);
    let contain = cv.contain.contains_positioned_descendants()
        || cv.will_change.contains(WillChange::CONTAIN);
    if properties.display() == DisplayValue::Contents || !(transform || filter || contain) {
        return false;
    }
    // A filter on the document root does not make it a containing block.
    let root = || {
        document
            .parent_of(id)
            .is_some_and(|parent| document.parent_of(parent).is_none())
    };
    (transform && is_transformable_with_properties(document, id, cv, properties))
        || (filter && !root())
        || (contain && containable(document, id, cv, properties))
}

/// Whether `id` is the containing block of its absolutely positioned
/// descendants.
pub(super) fn establishes_absolute_containing_block(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
) -> bool {
    // spec: https://www.w3.org/TR/CSS21/visudet.html#containing-block-details
    // spec: https://www.w3.org/TR/css-display-3/#box-generation
    let properties = document.nodes[id].box_properties();
    (properties.display() != DisplayValue::Contents
        && (*properties.position() != PositionValue::Static
            || cv.will_change.contains(WillChange::POSITION)))
        || fixed_containing_block_with_properties(document, id, cv, properties)
}

/// Correct the root OOF pass's auto margins for the principal writing mode.
/// Taffy's root fallback models the inline axis as horizontal; the CSS page
/// area uses the principal writing mode's inline and block axes.
pub(crate) fn correct_root_oof_auto_margins(
    document: &mut LayoutSession<'_>,
    root: usize,
    area_size: Size<f32>,
) {
    let vertical = document.nodes[root].used_writing_mode.is_vertical();
    let direction = document.physical_style(root).direction();
    let candidates: Vec<_> = document
        .oof_coordinates
        .iter()
        .filter_map(|(&node, coordinate)| (coordinate.owner == root).then_some(node))
        .collect();
    for node in candidates {
        let (left, right, top, bottom, auto_left, auto_right, auto_top, auto_bottom) = {
            let style = document.physical_style(node);
            let (inset, margin) = (style.inset(), style.margin());
            (
                inset
                    .left
                    .maybe_resolve(Some(area_size.width), crate::taffy_impl::resolve_calc),
                inset
                    .right
                    .maybe_resolve(Some(area_size.width), crate::taffy_impl::resolve_calc),
                inset
                    .top
                    .maybe_resolve(Some(area_size.height), crate::taffy_impl::resolve_calc),
                inset
                    .bottom
                    .maybe_resolve(Some(area_size.height), crate::taffy_impl::resolve_calc),
                margin.left.is_auto(),
                margin.right.is_auto(),
                margin.top.is_auto(),
                margin.bottom.is_auto(),
            )
        };
        let layout = &mut document.nodes[node].layout;
        if auto_left
            && auto_right
            && let (Some(left), Some(right)) = (left, right)
        {
            let free = area_size.width - left - right - layout.size.width;
            let margin = if !vertical && free < 0.0 {
                0.0
            } else {
                free / 2.0
            };
            layout.margin.left = margin;
            layout.margin.right = margin;
            layout.location.x = if direction == TaffyDirection::Rtl {
                area_size.width - right - layout.size.width - margin
            } else {
                left + margin
            };
        }
        if auto_top
            && auto_bottom
            && let (Some(top), Some(bottom)) = (top, bottom)
        {
            let free = area_size.height - top - bottom - layout.size.height;
            let margin = if vertical && free < 0.0 {
                0.0
            } else {
                free / 2.0
            };
            layout.margin.top = margin;
            layout.margin.bottom = margin;
            layout.location.y = top + margin;
        }
    }
}

#[cfg(test)]
mod tests;
