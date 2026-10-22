//! Axis conversion for a box laid out on the transposed axes of a vertical
//! writing mode.
//!
//! Taffy's algorithms know only the physical axes of `horizontal-tb`. To run
//! one on a vertical box, the inputs it is given are swapped (physical height
//! becomes the transposed width), the algorithm runs on those transposed
//! axes, and what it produces is mapped back to physical coordinates. In
//! `vertical-rl` the physical block axis runs from right to left, so mapping
//! a block-axis coordinate back also reverses it within the container's
//! physical width (the `extent` the functions take).
//!
//! The functions here hold the conversion only, with nothing specific to a
//! block container: the stretching of orthogonal children, fit-content
//! sizing and the frame bookkeeping stay with [`super::vertical_block`].
//! Margin collapsing is kept across a conversion only when the caller says
//! both sides share a block formatting context.

use taffy::tree::RequestedAxis;
use taffy::{
    AbsoluteAxis, AxisStaticEdge, AxisStaticPosition, Baselines, CollapsibleMarginSet, Layout,
    LayoutInput, LayoutOutput, Line, Point, Rect, Size,
};

/// `rect` with its physical sides moved to the transposed sides they map
/// to: top and bottom become left and right, and the block-start side
/// becomes the top.
#[cfg(test)]
pub(super) fn transpose_rect<T: Copy>(rect: Rect<T>, right_to_left: bool) -> Rect<T> {
    let (block_start, block_end) = if right_to_left {
        (rect.right, rect.left)
    } else {
        (rect.left, rect.right)
    };
    Rect {
        left: rect.top,
        right: rect.bottom,
        top: block_start,
        bottom: block_end,
    }
}

/// Map sides from the transposed geometry back to physical coordinates.
pub(super) fn physical_rect<T: Copy>(rect: Rect<T>, right_to_left: bool) -> Rect<T> {
    let (left, right) = if right_to_left {
        (rect.bottom, rect.top)
    } else {
        (rect.top, rect.bottom)
    };
    Rect {
        left,
        right,
        top: rect.left,
        bottom: rect.right,
    }
}

pub(super) fn swap<T>(size: Size<T>) -> Size<T> {
    Size {
        width: size.height,
        height: size.width,
    }
}

/// `inputs` with the axes swapped. Margin collapsing is preserved only when
/// the input and output stay within the same transposed vertical BFC.
pub(super) fn transpose_inputs(inputs: LayoutInput, preserve_margin_collapse: bool) -> LayoutInput {
    LayoutInput {
        axis: AbsoluteAxis::try_from(inputs.axis)
            .map_or(RequestedAxis::Both, |axis| axis.other_axis().into()),
        known_dimensions: swap(inputs.known_dimensions),
        known_dimensions_are_definite: swap(inputs.known_dimensions_are_definite),
        parent_size: swap(inputs.parent_size),
        available_space: swap(inputs.available_space),
        vertical_margins_are_collapsible: if preserve_margin_collapse {
            inputs.vertical_margins_are_collapsible
        } else {
            Line::FALSE
        },
        ..inputs
    }
}

/// Convert an output produced on the vertical block's transposed axes back
/// to physical axes. Its margins are left to the parent, which reads them
/// from the box's style: they do not collapse with the box's children.
pub(super) fn transpose_output_to_physical(
    mut output: LayoutOutput,
    right_to_left: bool,
    reverse_extent: f32,
    preserve_margin_collapse: bool,
) -> LayoutOutput {
    let oof_positioning_area = output
        .oof_positioning_area
        .map(|area| taffy::OofPositioningArea {
            size: swap(area.size),
            offset: Point {
                x: if right_to_left {
                    reverse_extent - area.offset.y - area.size.height
                } else {
                    area.offset.y
                },
                y: area.offset.x,
            },
        });
    for candidate in output.oof_candidates.as_mut_slice() {
        let inline_axis = candidate.static_position.x;
        let block_axis = candidate.static_position.y;
        candidate.static_position = Point {
            x: if right_to_left {
                reverse_static_position(block_axis, reverse_extent)
            } else {
                block_axis
            },
            y: inline_axis,
        };
    }
    let top_margin = if preserve_margin_collapse {
        output.top_margin
    } else {
        CollapsibleMarginSet::ZERO
    };
    let bottom_margin = if preserve_margin_collapse {
        output.bottom_margin
    } else {
        CollapsibleMarginSet::ZERO
    };
    LayoutOutput {
        size: swap(output.size),
        scrollable_overflow_rect: swap_rect_axes(output.scrollable_overflow_rect),
        baselines: Baselines::NONE,
        top_margin,
        bottom_margin,
        margins_can_collapse_through: preserve_margin_collapse
            && output.margins_can_collapse_through,
        oof_positioning_area,
        ..output
    }
}

/// Convert a child's physical-axis output to the transposed axes used by its
/// vertical block parent. In `vertical-rl`, the physical block axis is
/// reversed, while the physical inline axis is not.
pub(super) fn transpose_child_output_to_vertical(
    mut output: LayoutOutput,
    right_to_left: bool,
    reverse_extent: f32,
    preserve_margin_collapse: bool,
) -> LayoutOutput {
    let oof_positioning_area = output
        .oof_positioning_area
        .map(|area| taffy::OofPositioningArea {
            size: swap(area.size),
            offset: Point {
                x: area.offset.y,
                y: if right_to_left {
                    reverse_extent - area.offset.x - area.size.width
                } else {
                    area.offset.x
                },
            },
        });
    for candidate in output.oof_candidates.as_mut_slice() {
        let physical_inline_axis = candidate.static_position.y;
        let physical_block_axis = candidate.static_position.x;
        candidate.static_position = Point {
            x: physical_inline_axis,
            y: if right_to_left {
                reverse_static_position(physical_block_axis, reverse_extent)
            } else {
                physical_block_axis
            },
        };
    }
    let top_margin = if preserve_margin_collapse {
        output.top_margin
    } else {
        CollapsibleMarginSet::ZERO
    };
    let bottom_margin = if preserve_margin_collapse {
        output.bottom_margin
    } else {
        CollapsibleMarginSet::ZERO
    };
    LayoutOutput {
        size: swap(output.size),
        scrollable_overflow_rect: swap_rect_axes(output.scrollable_overflow_rect),
        baselines: Baselines::NONE,
        top_margin,
        bottom_margin,
        margins_can_collapse_through: preserve_margin_collapse
            && output.margins_can_collapse_through,
        oof_positioning_area,
        ..output
    }
}

pub(super) fn reverse_static_position(
    position: AxisStaticPosition,
    extent: f32,
) -> AxisStaticPosition {
    let reverse_edge = |edge| match edge {
        AxisStaticEdge::Start => AxisStaticEdge::End,
        AxisStaticEdge::Center => AxisStaticEdge::Center,
        AxisStaticEdge::End => AxisStaticEdge::Start,
    };
    AxisStaticPosition {
        area: Line {
            start: extent - position.area.end,
            end: extent - position.area.start,
        },
        align: taffy::AxisStaticAlign {
            keyword: reverse_edge(position.align.keyword),
            safety: position.align.safety,
            fallback: reverse_edge(position.align.fallback),
        },
    }
}

/// A scrollable overflow rectangle, measured along the inline and block
/// axes from the scroll origin, with its axes swapped.
pub(super) fn swap_rect_axes(rect: Rect<f32>) -> Rect<f32> {
    Rect {
        left: rect.top,
        right: rect.bottom,
        top: rect.left,
        bottom: rect.right,
    }
}

/// The physical layout of a child placed by taffy at `layout` in a
/// transposed container whose physical width is `container_width`.
pub(super) fn physical_layout(
    layout: &Layout,
    container_width: f32,
    right_to_left: bool,
) -> Layout {
    let block_start = layout.location.y;
    Layout {
        order: layout.order,
        location: Point {
            x: if right_to_left {
                container_width - block_start - layout.size.height
            } else {
                block_start
            },
            y: layout.location.x,
        },
        size: swap(layout.size),
        scrollable_overflow_rect: swap_rect_axes(layout.scrollable_overflow_rect),
        scrollbar_size: swap(layout.scrollbar_size),
        border: physical_rect(layout.border, right_to_left),
        padding: physical_rect(layout.padding, right_to_left),
        margin: physical_rect(layout.margin, right_to_left),
    }
}

#[cfg(test)]
mod tests;
