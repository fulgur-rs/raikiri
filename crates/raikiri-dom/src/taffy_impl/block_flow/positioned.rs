//! Initial padding-box placement using the public out-of-flow routine.

use super::sizing::Metrics;
use crate::layout::context::LayoutSession;
use taffy::{
    AxisStaticEdge, AxisStaticPosition, Direction, LayoutBlockContainer, NodeId, OofCandidate,
    OofPositioningArea, Point, Position, Rect, Size,
};

pub(super) struct Candidate {
    pub node: NodeId,
    pub order: usize,
    pub static_position: Point<f32>,
}

pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
    owner: NodeId,
    children: &[Candidate],
    container: Metrics,
    size: Size<f32>,
) -> Rect<f32> {
    let direction = tree.get_block_container_style(owner).direction;
    let candidates: Vec<_> = children
        .iter()
        .map(|child| OofCandidate {
            node: child.node,
            order: child.order as u32,
            position: Position::Absolute,
            static_position: Point {
                x: AxisStaticPosition::from_edge(
                    if direction == Direction::Rtl {
                        size.width - container.inset.right
                    } else {
                        child.static_position.x
                    },
                    if direction == Direction::Rtl {
                        AxisStaticEdge::End
                    } else {
                        AxisStaticEdge::Start
                    },
                ),
                y: AxisStaticPosition::from_edge(child.static_position.y, AxisStaticEdge::Start),
            },
        })
        .collect();
    let area = OofPositioningArea {
        size: Size {
            width: (size.width
                - container.border.horizontal_axis_sum()
                - container.scrollbar.width)
                .max(0.0),
            height: (size.height
                - container.border.vertical_axis_sum()
                - container.scrollbar.height)
                .max(0.0),
        },
        offset: Point {
            x: container.border.left,
            y: container.border.top,
        },
    };
    crate::taffy_impl::oof::layout_candidates(tree, owner, &candidates, area, direction)
}
