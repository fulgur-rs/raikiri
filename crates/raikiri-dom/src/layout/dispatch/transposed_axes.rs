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
//!
//! The conversions of an output and of a layout swap the axes of the
//! scrollable overflow rectangle without mirroring it. [`OverflowFrame`] maps
//! it to physical coordinates by mirroring within the padding box, for the
//! callers that need the rectangle in physical coordinates.

use crate::layout::protocol::{
    AbsoluteAxis, AxisStaticAlign, AxisStaticEdge, AxisStaticPosition, CollapsibleMarginSet,
    LayoutInput, LayoutOutput, OofPositioningArea, RequestedAxis,
};
use raikiri_layout_geometry::{BoxLayout, Edges, Line, Point, Size};

/// `rect` with its physical sides moved to the transposed sides they map
/// to: top and bottom become left and right, and the block-start side
/// becomes the top.
#[cfg(test)]
pub(super) fn transpose_rect<T: Copy>(rect: Edges<T>, right_to_left: bool) -> Edges<T> {
    let (block_start, block_end) = if right_to_left {
        (rect.right, rect.left)
    } else {
        (rect.left, rect.right)
    };
    Edges {
        left: rect.top,
        right: rect.bottom,
        top: block_start,
        bottom: block_end,
    }
}

/// Map sides from the transposed geometry back to physical coordinates.
fn physical_rect<T: Copy>(rect: Edges<T>, right_to_left: bool) -> Edges<T> {
    let (left, right) = if right_to_left {
        (rect.bottom, rect.top)
    } else {
        (rect.top, rect.bottom)
    };
    Edges {
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
    let oof_positioning_area = output.oof_positioning_area.map(|area| OofPositioningArea {
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
    output.oof_candidates.for_each_mut(|candidate| {
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
    });
    swap_output_axes(output, oof_positioning_area, preserve_margin_collapse)
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
    let oof_positioning_area = output.oof_positioning_area.map(|area| OofPositioningArea {
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
    output.oof_candidates.for_each_mut(|candidate| {
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
    });
    swap_output_axes(output, oof_positioning_area, preserve_margin_collapse)
}

/// The part of an output conversion that does not depend on the direction:
/// swap the sizes and the overflow rectangle, and keep the margin collapsing
/// state only when the caller says both sides share a block formatting
/// context. The baselines are carried over as they are: a box in a vertical
/// writing mode reports them as offsets along its block axis from its
/// block-start border edge, which is the layout y axis on this side too. A
/// caller whose box has none of the kind the other side reads resets them.
fn swap_output_axes(
    output: LayoutOutput,
    oof_positioning_area: Option<OofPositioningArea>,
    preserve_margin_collapse: bool,
) -> LayoutOutput {
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
        top_margin,
        bottom_margin,
        margins_can_collapse_through: preserve_margin_collapse
            && output.margins_can_collapse_through,
        oof_positioning_area,
        ..output
    }
}

/// Mirror a static position within an axis of length `extent`, swapping the
/// start and end alignments with it.
fn reverse_static_position(position: AxisStaticPosition, extent: f32) -> AxisStaticPosition {
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
        align: AxisStaticAlign {
            keyword: reverse_edge(position.align.keyword),
            safety: position.align.safety,
            fallback: reverse_edge(position.align.fallback),
        },
    }
}

/// A scrollable overflow rectangle, measured along the inline and block
/// axes from the scroll origin, with its axes swapped.
fn swap_rect_axes(rect: Edges<f32>) -> Edges<f32> {
    Edges {
        left: rect.top,
        right: rect.bottom,
        top: rect.left,
        bottom: rect.right,
    }
}

/// The size of the padding box inside a border box of `border_box`, given
/// its `border` on the same axes.
pub(super) fn padding_box(border_box: Size<f32>, border: Edges<f32>) -> Size<f32> {
    Size {
        width: (border_box.width - border.left - border.right).max(0.0),
        height: (border_box.height - border.top - border.bottom).max(0.0),
    }
}

/// How the scrollable overflow rectangle of a box on transposed axes maps to
/// physical coordinates and back.
///
/// Taffy measures the rectangle on the layout axes from the corner of the
/// padding box where scrolling starts: `left` and `right` along the inline
/// axis, `top` and `bottom` down the block axis. Under `direction: rtl` it
/// reads the inline axis from its far end, so the inline offsets are
/// measured from there. Physical coordinates are measured from the top left
/// corner of the padding box, with x to the right and y down, the way the
/// layout and the positioning area of the same box are. Overflow past the
/// block-end side of a `vertical-rl` box therefore lies to the left of the
/// box, at a negative x, while a swap of the axes would put it to the right,
/// and overflow before its block start lies past the physical right edge.
///
/// Taffy reports a box whose content does not overflow, and a run that does
/// not lay the box out, as an all-zero rectangle. That rectangle holds no
/// overflow and stays zero in either direction, which also means that an
/// output without layout data, which carries no padding box either, has
/// nothing to convert.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct OverflowFrame {
    /// The block axis runs from right to left (`vertical-rl`, `sideways-rl`).
    pub(super) block_reversed: bool,
    /// The inline axis is read from the physical bottom: taffy runs it with
    /// the direction `rtl`.
    pub(super) inline_reversed: bool,
    /// The padding box on the layout axes: the inline size by the block size.
    /// These are the extents the reversed axes are mirrored within.
    pub(super) padding_box: Size<f32>,
}

impl OverflowFrame {
    /// The rectangle on physical axes of `rect` on the layout axes.
    pub(super) fn physical(self, rect: Edges<f32>) -> Edges<f32> {
        if rect == Edges::ZERO {
            return rect;
        }
        let block = self.padding_box.height;
        let inline = self.padding_box.width;
        let (left, right) = if self.block_reversed {
            (block - rect.bottom, block - rect.top)
        } else {
            (rect.top, rect.bottom)
        };
        let (top, bottom) = if self.inline_reversed {
            (inline - rect.right, inline - rect.left)
        } else {
            (rect.left, rect.right)
        };
        Edges {
            left,
            right,
            top,
            bottom,
        }
    }

    /// The rectangle on the layout axes of `rect` on physical axes: the
    /// inverse of [`Self::physical`].
    pub(super) fn on_layout_axes(self, rect: Edges<f32>) -> Edges<f32> {
        if rect == Edges::ZERO {
            return rect;
        }
        let block = self.padding_box.height;
        let inline = self.padding_box.width;
        let (top, bottom) = if self.block_reversed {
            (block - rect.right, block - rect.left)
        } else {
            (rect.left, rect.right)
        };
        let (left, right) = if self.inline_reversed {
            (inline - rect.bottom, inline - rect.top)
        } else {
            (rect.top, rect.bottom)
        };
        Edges {
            left,
            right,
            top,
            bottom,
        }
    }
}

/// The physical layout of a child placed by taffy at `layout` in a
/// transposed container whose physical width is `container_width`.
pub(super) fn physical_layout(
    layout: &BoxLayout,
    container_width: f32,
    right_to_left: bool,
) -> BoxLayout {
    let block_start = layout.location.y;
    BoxLayout {
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
