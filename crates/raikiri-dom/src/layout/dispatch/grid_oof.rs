//! Out-of-flow children of a grid container laid out on the axes of a vertical
//! writing mode.
//!
//! Taffy's grid algorithm runs on the layout axes of the container (see
//! [`super::axis_adapter`]): the tracks it reports in a [`GridInfo`] are
//! measured from the block-start edge, with the columns along the inline axis
//! and the rows along the block axis. Taffy resolves the grid area of an
//! out-of-flow child from that information itself, but only for the physical
//! axes of `horizontal-tb`, with the columns horizontal and the rows vertical.
//!
//! The tracks therefore stay on layout axes, where `grid-row` and
//! `grid-column` of the child mean what they say (CSS Grid 1 §9), and
//! the area they resolve to is converted to physical axes here. Converting the
//! tracks instead would leave the named grid areas, which taffy keeps out of
//! reach, on the wrong axes.

use crate::layout::protocol::{NodeId, OofCandidates, OofLayoutResult, OofPositioningArea};
use crate::layout::taffy_adapter::GridInfo;
use crate::layout::taffy_adapter::OofSpan;
use crate::layout::taffy_adapter::algorithms::{claims_position, compute_oof_layout_for_area};
use raikiri_layout_geometry::{Edges, Point, Size};
use taffy::{ContainingBlockClaims, Direction};

use super::vertical_block::block_right_to_left;
use crate::layout::context::LayoutSession;
use crate::layout::style_views::ReadRole;
use crate::node::FormattingContext;
use raikiri_layout_style::LayoutStyleView;

/// Whether the block axis of grid container `idx` runs from right to left, when
/// the container is laid out on the axes of a vertical writing mode.
pub(super) fn right_to_left(tree: &LayoutSession<'_>, idx: usize) -> Option<bool> {
    if tree.nodes[idx].formatting_context != FormattingContext::Grid {
        return None;
    }
    block_right_to_left(tree, idx)
}

/// How the layout axes of a vertical grid container map to physical ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GridAxes {
    right_to_left: bool,
    /// The width of the border box of the container: the extent the block
    /// axis is mirrored within in `vertical-rl`.
    width: f32,
}

impl GridAxes {
    /// The axes of grid container `idx`, whose physical border box is `width`
    /// wide, when it is laid out on the axes of a vertical writing mode.
    pub(super) fn of(tree: &LayoutSession<'_>, idx: usize, width: f32) -> Option<Self> {
        Self::new(right_to_left(tree, idx), width)
    }

    /// [`Self::of`] for a container whose [`right_to_left`] is known.
    pub(super) fn new(right_to_left: Option<bool>, width: f32) -> Option<Self> {
        right_to_left.map(|right_to_left| Self {
            right_to_left,
            width,
        })
    }

    /// `rect`, in physical coordinates, on layout axes: the inline axis is
    /// the physical y axis, and the block axis the physical x axis, mirrored
    /// in `vertical-rl`.
    fn to_layout(self, rect: Edges<f32>) -> Edges<f32> {
        let (top, bottom) = if self.right_to_left {
            (self.width - rect.right, self.width - rect.left)
        } else {
            (rect.left, rect.right)
        };
        Edges {
            left: rect.top,
            right: rect.bottom,
            top,
            bottom,
        }
    }

    /// `rect`, in layout coordinates, on physical axes.
    fn to_physical(self, rect: Edges<f32>) -> Edges<f32> {
        let (left, right) = if self.right_to_left {
            (self.width - rect.bottom, self.width - rect.top)
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
}

/// The grid areas of a container's out-of-flow children, on physical axes.
pub(super) struct GridAreas<'a> {
    info: &'a GridInfo,
    /// `None` for a container laid out on the physical axes of `horizontal-tb`.
    axes: Option<GridAxes>,
}

impl<'a> GridAreas<'a> {
    /// The grid areas of the container whose layout reported `info`, if it
    /// reported anything.
    pub(super) fn of(info: Option<&'a GridInfo>, axes: Option<GridAxes>) -> Option<Self> {
        info.map(|info| Self { info, axes })
    }

    /// The grid area that the grid placement of `style` resolves to, relative
    /// to the border box of the container, given its physical `padding_box`.
    /// `direction` is the container's inline direction.
    pub(super) fn area(
        &self,
        style: &LayoutStyleView<'_>,
        direction: Direction,
        padding_box: Edges<f32>,
    ) -> Edges<f32> {
        let resolve = |padding_box| {
            self.info
                .resolve_absolute_grid_area(style, direction, padding_box)
        };
        match self.axes {
            None => resolve(padding_box),
            Some(axes) => axes.to_physical(resolve(axes.to_layout(padding_box))),
        }
    }

    /// [`Self::area`] for a container whose padding box is the positioning
    /// `area`, returned as a positioning area too.
    pub(super) fn positioning_area(
        &self,
        style: &LayoutStyleView<'_>,
        direction: Direction,
        area: OofPositioningArea,
    ) -> OofPositioningArea {
        let grid_area = self.area(
            style,
            direction,
            Edges {
                left: area.offset.x,
                right: area.offset.x + area.size.width,
                top: area.offset.y,
                bottom: area.offset.y + area.size.height,
            },
        );
        OofPositioningArea {
            size: Size {
                width: grid_area.right - grid_area.left,
                height: grid_area.bottom - grid_area.top,
            },
            offset: Point {
                x: grid_area.left,
                y: grid_area.top,
            },
        }
    }
}

/// Lay out the out-of-flow `candidates` of the vertical grid container `owner`
/// against its positioning `area`, each at the grid area its grid placement
/// resolves to.
///
/// Taffy takes the grid area of a candidate from the container's detailed
/// layout info, which holds the tracks on layout axes here and is withheld
/// from it. A candidate that is placed is laid out on its own against its grid
/// area instead; the others, with the consecutive ones batched, against `area`.
/// Candidates keep their order.
///
/// The scrollable overflow of the result is left empty: it is measured from the
/// origin of the area a candidate is laid out against, which differs from
/// candidate to candidate. The caller recomputes it from the final positions.
pub(super) fn compute_oof_layout(
    tree: &mut LayoutSession<'_>,
    owner: NodeId,
    candidates: OofSpan<'_>,
    area: OofPositioningArea,
    direction: Direction,
    claims: ContainingBlockClaims,
    axes: GridAxes,
) -> OofLayoutResult {
    let mut result = OofLayoutResult {
        hoisted: Vec::new(),
        unclaimed: OofCandidates::new(),
        scrollable_overflow_rect: Edges::ZERO,
    };
    let mut lay_out = |tree: &mut LayoutSession<'_>, candidates: OofSpan<'_>, area| {
        if candidates.is_empty() {
            return;
        }
        let mut part =
            compute_oof_layout_for_area(tree, owner, candidates, area, direction, claims);
        result.hoisted.append(&mut part.hoisted);
        result.unclaimed.append(&mut part.unclaimed);
    };
    let mut batch_start = 0;
    for (index, candidate) in candidates.iter().enumerate() {
        if !claims_position(claims, candidate.position) {
            continue;
        }
        let grid_area = {
            let style = tree.query_style(usize::from(candidate.node), ReadRole::Oof);
            if !style.has_grid_placement() {
                continue;
            }
            // Without the tracks, which taffy reports only from a layout it
            // performs, a placed box is laid out against the padding box, as
            // taffy does for any grid container it has no tracks for.
            let info = tree.context.grid_layout_info.get(&usize::from(owner));
            GridAreas::of(info, Some(axes)).map_or(area, |areas| {
                areas.positioning_area(&style, direction, area)
            })
        };
        lay_out(tree, candidates.slice(batch_start..index), area);
        lay_out(tree, candidates.slice(index..index + 1), grid_area);
        batch_start = index + 1;
    }
    lay_out(tree, candidates.slice(batch_start..candidates.len()), area);
    result
}
