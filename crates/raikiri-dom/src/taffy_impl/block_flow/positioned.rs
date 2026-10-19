//! Collect out-of-flow children for their containing-block dispatch.

use super::sizing::Metrics;
use taffy::{
    AxisStaticPosition, OofCandidate, OofCandidates, OofPositioningArea, Point, Position, Size,
};

pub(super) fn push_candidate(
    candidates: &mut OofCandidates,
    node: taffy::NodeId,
    order: usize,
    position: Position,
    static_position: Point<AxisStaticPosition>,
) {
    candidates.push(OofCandidate {
        node,
        order: order as u32,
        position,
        static_position,
    });
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
