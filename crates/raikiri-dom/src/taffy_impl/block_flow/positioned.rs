//! Collect out-of-flow children for their containing-block dispatch.

use super::sizing::Metrics;
use taffy::{
    AxisStaticEdge, AxisStaticPosition, Direction, OofCandidate, OofCandidates, OofPositioningArea,
    Point, Position, Size,
};

pub(super) struct CandidateContext {
    pub(super) direction: Direction,
    pub(super) container_width: f32,
    pub(super) metrics: Metrics,
}

pub(super) fn push_candidate(
    candidates: &mut OofCandidates,
    node: taffy::NodeId,
    order: usize,
    position: Position,
    static_position: Point<f32>,
    context: CandidateContext,
) {
    let CandidateContext {
        direction,
        container_width,
        metrics,
    } = context;
    candidates.push(OofCandidate {
        node,
        order: order as u32,
        position,
        static_position: Point {
            x: AxisStaticPosition::from_edge(
                if direction == Direction::Rtl {
                    container_width - metrics.inset.right
                } else {
                    static_position.x
                },
                if direction == Direction::Rtl {
                    AxisStaticEdge::End
                } else {
                    AxisStaticEdge::Start
                },
            ),
            y: AxisStaticPosition::from_edge(static_position.y, AxisStaticEdge::Start),
        },
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
