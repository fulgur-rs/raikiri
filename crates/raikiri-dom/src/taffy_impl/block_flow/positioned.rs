//! Collect out-of-flow children for their containing-block dispatch.

use super::sizing::Metrics;
use crate::LayoutStyle as Style;
use crate::layout::protocol::{
    AxisStaticPosition, NodeId, OofCandidate, OofCandidates, OofPosition, OofPositioningArea,
};
use crate::layout::taffy_adapter::algorithms::static_position_from_alignment;
use raikiri_layout_geometry::{Line, Point, Size};
use taffy::{AlignItems, Direction};

/// Record the out-of-flow child `node` of a block container for the
/// containing block that claims it.
pub(super) fn push_candidate(
    candidates: &mut OofCandidates,
    node: NodeId,
    order: usize,
    position: OofPosition,
    static_position: Point<AxisStaticPosition>,
) {
    candidates.push(OofCandidate {
        node,
        order: order as u32,
        position,
        static_position,
    });
}

/// The static position of the out-of-flow `child` of the block container
/// `container`, whose content box spans `inline_area` in the inline axis and
/// where the box would have been at `static_y` in the block axis.
///
/// The static-position rectangle of such a child spans the container's
/// content box in the inline axis and is a zero-height line at its
/// hypothetical position in the block axis; the child's `justify-self` and
/// `align-self` align it in that rectangle, and `normal` and `stretch` act as
/// `start`.
// spec: https://www.w3.org/TR/css-position-3/#staticpos-rect
pub(crate) fn aligned_static_position(
    container: &Style,
    child: &Style,
    direction: Direction,
    inline_area: Line<f32>,
    static_y: f32,
) -> Point<AxisStaticPosition> {
    let block_area = Line {
        start: static_y,
        end: static_y,
    };
    // Block containers have no `align-items`: an `auto` `align-self` is
    // `normal`.
    let justify_self = child
        .justify_self
        .unwrap_or(container.justify_items)
        .resolve_self_relative(child.direction, direction, true);
    let align_self = child
        .align_self
        .unwrap_or(AlignItems::NORMAL)
        .resolve_self_relative(child.direction, direction, false);
    Point {
        x: static_position_from_alignment(justify_self, inline_area, direction == Direction::Rtl),
        y: static_position_from_alignment(align_self, block_area, false),
    }
}

pub(super) fn positioning_area(metrics: Metrics, size: Size<f32>) -> OofPositioningArea {
    OofPositioningArea {
        size: Size {
            width: (size.width - metrics.border.horizontal_axis_sum() - metrics.scrollbar.width)
                .max(0.0),
            height: (size.height - metrics.border.vertical_axis_sum() - metrics.scrollbar.height)
                .max(0.0),
        },
        offset: Point {
            x: metrics.border.left,
            y: metrics.border.top,
        },
    }
}
