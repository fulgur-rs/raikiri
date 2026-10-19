//! Containing-block predicates and CSS-specific OOF corrections.
//!
//! The shared OOF pass places boxes against the containing block that claims
//! them. This module supplies the CSS predicates used to make those claims
//! and the writing-mode and flex static-position corrections that Taffy does
//! not model.

use super::box_tree::BoxTree;
use super::oof_containing_block::select_fixed_containing_block_from_ancestors;
use super::*;
use crate::layout::context::LayoutSession;
use crate::node::{BoxPropertiesRef, FormattingContext};
use raikiri_style::property::WillChange;
use taffy::util::MaybeResolve;
use taffy::{AlignContentKeyword, AlignItemsKeyword, AlignmentSafety};

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
    let node = &document.layout_node_with_style(id).unwrap();
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
    let direction = document
        .layout_node_with_style(root)
        .unwrap()
        .style
        .direction;
    let candidates: Vec<_> = document
        .oof_coordinates
        .iter()
        .filter_map(|(&node, coordinate)| (coordinate.owner == root).then_some(node))
        .collect();
    for node in candidates {
        let (left, right, top, bottom, auto_left, auto_right, auto_top, auto_bottom) = {
            let style = &document.layout_node_with_style(node).unwrap().style;
            (
                style
                    .inset
                    .left
                    .maybe_resolve(Some(area_size.width), crate::taffy_impl::resolve_calc),
                style
                    .inset
                    .right
                    .maybe_resolve(Some(area_size.width), crate::taffy_impl::resolve_calc),
                style
                    .inset
                    .top
                    .maybe_resolve(Some(area_size.height), crate::taffy_impl::resolve_calc),
                style
                    .inset
                    .bottom
                    .maybe_resolve(Some(area_size.height), crate::taffy_impl::resolve_calc),
                style.margin.left.is_auto(),
                style.margin.right.is_auto(),
                style.margin.top.is_auto(),
                style.margin.bottom.is_auto(),
            )
        };
        let layout = &mut document.nodes[node].unrounded_layout;
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

/// Keep flex static alignment anchored to its source flex container when the
/// OOF box is sized against a different containing block.
pub(crate) fn correct_root_flex_static_positions(document: &mut LayoutSession<'_>) {
    let candidates: Vec<_> = document
        .oof_coordinates
        .iter()
        .map(|(&node, coordinate)| (node, coordinate.owner, coordinate.static_owner))
        .filter(|&(_, _, static_owner)| {
            document.nodes[static_owner].formatting_context == FormattingContext::Flex
        })
        .collect();
    for (node, owner, static_owner) in candidates {
        let style = &document.layout_node_with_style(node).unwrap().style;
        let mut layout = document.nodes[node].unrounded_layout;
        let margin_box = Size {
            width: layout.size.width + layout.margin.left + layout.margin.right,
            height: layout.size.height + layout.margin.top + layout.margin.bottom,
        };
        let (alignment, space) = static_position_alignment(document, node, margin_box);
        let static_origin = document
            .layout_origin_of(static_owner)
            .unwrap_or(Point::ZERO);
        let owner_origin = document.layout_origin_of(owner).unwrap_or(Point::ZERO);
        let static_layout = document.nodes[static_owner].unrounded_layout;
        let insets = static_layout.padding + static_layout.border;
        if style.inset.left.is_auto() && style.inset.right.is_auto() {
            let free = space.width - margin_box.width;
            layout.location.x =
                static_origin.x + insets.left + free * alignment.width + layout.margin.left
                    - owner_origin.x;
        }
        if style.inset.top.is_auto() && style.inset.bottom.is_auto() {
            let free = space.height - margin_box.height;
            layout.location.y =
                static_origin.y + insets.top + free * alignment.height + layout.margin.top
                    - owner_origin.y;
        }
        document.set_unrounded_layout(taffy::NodeId::from(node), &layout);
    }
}

/// How the parent of the absolutely positioned box `id` aligns a margin box
/// of size `margin_box` at its static position, in each axis: the share of
/// the free space before the box (0 at the start edge, 1 at the end edge and
/// 0.5 centered) and the space the box is aligned in.
fn static_position_alignment(
    tree: &Document,
    id: usize,
    margin_box: Size<f32>,
) -> (Size<f32>, Size<f32>) {
    let Some(parent) = tree.taffy_parent_of(id) else {
        return (Size::ZERO, Size::ZERO); // cov:ignore: a placed box has a parent
    };
    let parent_style = &tree.layout_node_with_style(parent).unwrap().style;
    let rtl = parent_style.direction == TaffyDirection::Rtl;
    // CSS 2.1 §10.3.7: in a right-to-left static containing block the
    // static position is the box's right edge.
    let block = Size {
        width: if rtl { 1.0 } else { 0.0 },
        height: 0.0,
    };
    if tree.nodes[parent].formatting_context == FormattingContext::Grid {
        let child_style = &tree.layout_node_with_style(id).unwrap().style;
        let layout = tree.nodes[parent].unrounded_layout;
        let insets = (layout.padding + layout.border).sum_axes();
        let space = Size {
            width: layout.size.width - insets.width,
            height: layout.size.height - insets.height,
        };
        let overflows = Size {
            width: margin_box.width > space.width,
            height: margin_box.height > space.height,
        };
        if let Some(alignment) = crate::taffy_impl::auto_grid_abspos_static_alignment(
            child_style,
            parent_style,
            overflows,
        ) {
            return (alignment, space);
        }
    }
    if tree.nodes[parent].formatting_context != FormattingContext::Flex {
        // The alignment does not depend on the size, so neither does the
        // shift.
        return (block, Size::ZERO);
    }
    // CSS Flexbox 1 §4.1: the static position of an absolutely positioned
    // child of a flex container is that of the sole flex item, aligned by
    // the container's justify-content and the child's align-self.
    // spec: https://www.w3.org/TR/css-flexbox-1/#abspos-items
    let (is_row, reverse) = match parent_style.flex_direction {
        TaffyFlexDirection::Row => (true, false),
        TaffyFlexDirection::RowReverse => (true, true),
        TaffyFlexDirection::Column => (false, false),
        TaffyFlexDirection::ColumnReverse => (false, true),
    };
    let main_is_rtl = is_row && rtl;
    let cross_is_rtl = !is_row && rtl;
    let main_reversed = reverse ^ main_is_rtl;
    let cross_reversed = (parent_style.flex_wrap == TaffyFlexWrap::WrapReverse) ^ cross_is_rtl;
    let flex = |end: bool, reversed: bool| if end ^ reversed { 1.0 } else { 0.0 };
    let layout = tree.nodes[parent].unrounded_layout;
    let insets = (layout.padding + layout.border).sum_axes();
    let space = Size {
        width: layout.size.width - insets.width,
        height: layout.size.height - insets.height,
    };
    let (cross_space, cross_size) = if is_row {
        (space.height, margin_box.height)
    } else {
        (space.width, margin_box.width)
    };
    // The box is the sole flex item, so `space-around` and `space-evenly`
    // center it (CSS Flexbox 1 §8.2). On the main axis it keeps that
    // alignment when it overflows the container, as Taffy's own static
    // position does; only the cross axis falls back to start for an
    // overflowing `safe` alignment.
    // spec: https://www.w3.org/TR/css-flexbox-1/#justify-content-property
    let justify = parent_style.justify_content;
    // `start` and `end` follow the writing mode; `flex-start`, `flex-end`
    // and the fallbacks of the distributed values follow the flex direction.
    let main = match justify.keyword() {
        AlignContentKeyword::Start => flex(false, main_is_rtl),
        AlignContentKeyword::End => flex(true, main_is_rtl),
        AlignContentKeyword::Center
        | AlignContentKeyword::SpaceAround
        | AlignContentKeyword::SpaceEvenly => 0.5,
        AlignContentKeyword::FlexEnd => flex(true, main_reversed),
        AlignContentKeyword::Normal
        | AlignContentKeyword::FlexStart
        | AlignContentKeyword::Stretch
        | AlignContentKeyword::SpaceBetween => flex(false, main_reversed),
    };
    let style = &tree.layout_node_with_style(id).unwrap().style;
    let align = style
        .align_self
        .or(Some(parent_style.align_items))
        .unwrap_or(TaffyAlignItems::STRETCH);
    // `self-start` and `self-end` follow the box's own direction where the
    // cross axis is its inline axis.
    let flip = !is_row && style.direction != parent_style.direction;
    let keyword = if align.safety == AlignmentSafety::Safe && cross_size > cross_space {
        AlignItemsKeyword::Start
    } else {
        align.keyword
    };
    let cross = match keyword {
        AlignItemsKeyword::Start | AlignItemsKeyword::Baseline => flex(false, cross_is_rtl),
        AlignItemsKeyword::End => flex(true, cross_is_rtl),
        AlignItemsKeyword::SelfStart => flex(flip, cross_is_rtl),
        AlignItemsKeyword::SelfEnd => flex(!flip, cross_is_rtl),
        AlignItemsKeyword::Center => 0.5,
        AlignItemsKeyword::FlexEnd => flex(true, cross_reversed),
        AlignItemsKeyword::Normal | AlignItemsKeyword::FlexStart | AlignItemsKeyword::Stretch => {
            flex(false, cross_reversed)
        }
    };
    let alignment = if is_row {
        Size {
            width: main,
            height: cross,
        }
    } else {
        Size {
            width: cross,
            height: main,
        }
    };
    (alignment, space)
}

#[cfg(test)]
mod tests;
