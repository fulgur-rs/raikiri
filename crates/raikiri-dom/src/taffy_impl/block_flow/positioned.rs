//! Collect out-of-flow children for their containing-block dispatch.

use super::sizing::Metrics;
use taffy::{
    AlignItems, AxisStaticPosition, Direction, Line, OofCandidate, OofCandidates,
    OofPositioningArea, Point, Position, Size, Style,
};

pub(super) struct CandidateContext<'a> {
    pub(super) direction: Direction,
    pub(super) container_width: f32,
    pub(super) metrics: Metrics,
    /// The block container: its `justify-items` stands for a child's `auto`
    /// `justify-self`.
    pub(super) container: &'a Style,
    /// The out-of-flow child.
    pub(super) child: &'a Style,
}

/// Record the out-of-flow child `node` of a block container for the
/// containing block that claims it.
pub(super) fn push_candidate(
    candidates: &mut OofCandidates,
    node: taffy::NodeId,
    order: usize,
    position: Position,
    static_y: f32,
    context: CandidateContext<'_>,
) {
    let CandidateContext {
        direction,
        container_width,
        metrics,
        container,
        child,
    } = context;
    let inline_area = Line {
        start: metrics.inset.left,
        end: container_width - metrics.inset.right,
    };
    candidates.push(OofCandidate {
        node,
        order: order as u32,
        position,
        static_position: aligned_static_position(
            container,
            child,
            direction,
            inline_area,
            static_y,
        ),
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
        x: AxisStaticPosition::from_alignment(
            justify_self,
            inline_area,
            direction == Direction::Rtl,
        ),
        y: AxisStaticPosition::from_alignment(align_self, block_area, false),
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
