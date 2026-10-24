//! Baseline alignment that Taffy leaves out: along the inline axis of a grid or
//! a column flex container, and along the block axis of a vertical grid or
//! flex row for the items that run against its block direction.
//!
//! Taffy shares baselines along the rows of a row only: it places a grid item
//! with `justify-self: baseline`, and an item of a column flex container with
//! `align-self: baseline`, as if it were aligned to `start`. It keeps one
//! group of baselines for a row or a flex line, too, while items whose block
//! flow runs in opposite directions do not share a baseline (CSS Box
//! Alignment 3 §9.3). The container places its items first, and the items that
//! ask for a baseline are moved here, on physical axes, to where their
//! baseline-sharing groups put them.
//!
//! The items of a grid share a baseline when they align by the same set of
//! baselines in the same column, or in the same row along the block axis, and
//! those of a flex container when they sit on its one line. A group packs its
//! items against the edge of the room they are aligned in that the first
//! baselines of its items face, or the opposite one for last baselines, and
//! puts the baselines of all of them the same distance from that edge: the
//! farthest one, counting the margins.
//!
//! A box has no baseline along its own inline axis, so an item in the writing
//! mode of the container is given a synthesized one. An item in the orthogonal
//! writing mode has its block axis along the alignment axis, and shares the
//! baseline of its first line, as does an item that runs against the block
//! direction of its container.
//!
//! The container positions the out-of-flow boxes inside its items before the
//! items move, so a box whose static position lies inside a moved item is
//! carried along with it afterwards.
// spec: https://www.w3.org/TR/css-align-3/#baseline-sharing-group

use super::grid_oof::GridAxes;
use super::{TaffyChildIter, block_right_to_left, hoisted_children, used_writing_mode};
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{NodeId, OofCandidates};
use crate::layout::taffy_adapter::GridInfo;
use raikiri_layout_geometry::{Edges, Point, Size};
use raikiri_layout_style::LayoutStyleView;
use raikiri_layout_style::values::{BaselineSet, Direction, Display, Position, ResolveOrZero};
use raikiri_style::property::WritingMode;

/// The physical axis the items are aligned along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    /// The coordinate of `point` along the axis.
    fn coordinate(self, point: Point<f32>) -> f32 {
        match self {
            Self::Horizontal => point.x,
            Self::Vertical => point.y,
        }
    }

    /// The extent of `size` along the axis.
    fn extent(self, size: Size<f32>) -> f32 {
        match self {
            Self::Horizontal => size.width,
            Self::Vertical => size.height,
        }
    }

    /// The sides of `edges` at the start and at the end of the axis.
    fn sides(self, edges: Edges<f32>) -> (f32, f32) {
        match self {
            Self::Horizontal => (edges.left, edges.right),
            Self::Vertical => (edges.top, edges.bottom),
        }
    }

    /// `point` moved to `coordinate` along the axis.
    fn moved(self, mut point: Point<f32>, coordinate: f32) -> Point<f32> {
        match self {
            Self::Horizontal => point.x = coordinate,
            Self::Vertical => point.y = coordinate,
        }
        point
    }
}

/// The baseline-sharing group an item belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Group {
    /// The grid column the group is in; zero in a flex container.
    track: u16,
    /// Whether the group packs towards the end of the axis.
    towards_end: bool,
}

/// An item that aligns by a baseline, as the container placed it.
#[derive(Clone, Copy, Debug)]
struct Member {
    child: usize,
    group: Group,
    /// The room the item is aligned in, from its start to its end.
    room: (f32, f32),
    /// The extent of the border box.
    size: f32,
    /// The margins at the start and at the end of the border box.
    margin: (f32, f32),
    /// The distance from the start of the border box to the baseline.
    baseline: f32,
    /// How far a relatively positioned box sits from where it is in flow.
    offset: f32,
}

/// What an alignment pass needs to know about its container.
struct Container {
    axis: Axis,
    /// Whether the container starts the axis at its end: its `direction` is
    /// `rtl` along the inline axis, or its block flow runs from right to left
    /// along the block axis. It decides the side a first baseline is packed
    /// against, and where the container left an item that is aligned to the
    /// start.
    right_to_left: bool,
    /// Whether the dominant baseline of the container is the central one.
    central: bool,
    /// Whether the property aligning the items along the axis is
    /// `justify-self`, rather than `align-self`.
    justify: bool,
    /// Whether the pass aligns the items that run against the block direction
    /// of a vertical container along its block axis, which the container left
    /// at the start of the axis.
    opposed: bool,
}

impl Container {
    /// The container `idx` of style `style` aligning along `axis`, its inline
    /// axis, by the baselines its items share with it.
    fn new(
        tree: &LayoutSession<'_>,
        idx: usize,
        style: &LayoutStyleView<'_>,
        axis: Axis,
        justify: bool,
    ) -> Self {
        Self {
            axis,
            right_to_left: style.direction() == Direction::Rtl,
            central: tree.nodes[idx].central_baseline,
            justify,
            opposed: false,
        }
    }

    /// The vertical container `idx` aligning, along its block axis, the items
    /// that run against its block direction.
    fn opposed(tree: &LayoutSession<'_>, idx: usize) -> Self {
        Self {
            axis: Axis::Horizontal,
            right_to_left: block_right_to_left(tree, idx) == Some(true),
            central: tree.nodes[idx].central_baseline,
            justify: false,
            opposed: true,
        }
    }

    /// The baseline of the box `child` that has none of its own, whose border
    /// box is `size` wide or high along the axis its block axis runs along: its
    /// offset from the block-start edge of the box, on the baseline that is
    /// dominant for the container (CSS Align 3 §9.1).
    ///
    /// The central baseline is the middle of the box. The alphabetic one of a
    /// horizontal box is its block start, the line-over edge; that of a
    /// vertical box is its line-under edge, the left one except in
    /// `sideways-lr`: the block start of `vertical-lr`, and the block end of
    /// the other modes.
    /// spec: <https://www.w3.org/TR/css-align-3/#baseline-rules>
    fn synthesized_offset(&self, tree: &LayoutSession<'_>, child: usize, size: f32) -> f32 {
        if self.central {
            size / 2.0
        } else if self.axis == Axis::Horizontal
            && used_writing_mode(tree, child) != WritingMode::VerticalLr
        {
            size
        } else {
            0.0
        }
    }

    /// The member `child` makes of itself, with `tracks` the first and last
    /// grid column it spans and `room` the room it is aligned in; `None` when
    /// it does not align by a baseline. `parent` is the style of the container.
    fn member(
        &self,
        tree: &LayoutSession<'_>,
        parent: &LayoutStyleView<'_>,
        child: usize,
        tracks: (u16, u16),
        room: (f32, f32),
    ) -> Option<Member> {
        // Only the items that run against the block direction are placed at
        // the start for this pass.
        if self.opposed && !tree.nodes[child].opposed_baseline_group {
            return None;
        }
        let style = tree.physical_style(child);
        let set = style.self_baseline_set(parent, self.justify)?;
        // An auto margin takes the free space in the axis instead.
        let margin = style.margin();
        if match self.axis {
            Axis::Horizontal => margin.left.is_auto() || margin.right.is_auto(),
            Axis::Vertical => margin.top.is_auto() || margin.bottom.is_auto(),
        } {
            return None;
        }
        let layout = &tree.nodes[child].layout;
        let size = self.axis.extent(layout.size);
        let margin = self.axis.sides(layout.margin);
        let (baseline, towards_end) = if self.opposed || tree.is_orthogonal_to_parent(child) {
            // The block axis of the item runs along the axis, from its
            // block-start edge: the baseline of its first line sits at an
            // offset from there, and the first baselines face that edge.
            let block_start_at_end =
                self.axis == Axis::Horizontal && block_right_to_left(tree, child) == Some(true);
            let offset = tree.nodes[child]
                .own_first_baseline
                .unwrap_or_else(|| self.synthesized_offset(tree, child, size));
            (
                if block_start_at_end {
                    size - offset
                } else {
                    offset
                },
                (set == BaselineSet::First) == block_start_at_end,
            )
        } else {
            match self.axis {
                // The baseline of a horizontal box along its inline axis is
                // synthesized from its start edge.
                Axis::Horizontal => (0.0, (set == BaselineSet::Last) != self.right_to_left),
                Axis::Vertical => (
                    if self.central { size / 2.0 } else { size },
                    set == BaselineSet::Last,
                ),
            }
        };
        let track = if self.opposed {
            // The group sits in the row the items pack against the edge of,
            // which is the last one the item spans when the container runs
            // from that edge to the other one.
            if towards_end == self.right_to_left {
                tracks.0
            } else {
                tracks.1
            }
        } else {
            match set {
                BaselineSet::First => tracks.0,
                BaselineSet::Last => tracks.1,
            }
        };
        // Taffy places the box at the start of the room and then adds the
        // offset of a relative box, which moving the box must keep.
        let offset = if style.position() == Position::Relative {
            self.axis.coordinate(layout.location)
                - if self.right_to_left {
                    room.1 - size - margin.1
                } else {
                    room.0 + margin.0
                }
        } else {
            0.0
        };
        Some(Member {
            child,
            group: Group { track, towards_end },
            room,
            size,
            margin,
            baseline,
            offset,
        })
    }

    /// Move the `members` of the container `idx` to their groups' baselines,
    /// and carry the out-of-flow boxes that sit inside a moved item along with
    /// it, `candidates` being the ones still bubbling up out of the container.
    /// Returns the scrollable overflow that the moved boxes reach, relative to
    /// the container, or `None` when nothing moved.
    fn align(
        &self,
        tree: &mut LayoutSession<'_>,
        idx: usize,
        members: &[Member],
        candidates: &mut OofCandidates,
    ) -> Option<Edges<f32>> {
        let mut overflow = None;
        let mut shifts = Vec::new();
        for (index, member) in members.iter().enumerate() {
            if members[..index].iter().any(|m| m.group == member.group) {
                continue;
            }
            let group = || members.iter().filter(|m| m.group == member.group);
            // How far the baselines of the group reach from the edge it packs
            // against, counting the margins: the farthest one, which every
            // item of the group shares, whatever room it is aligned in.
            let depth = if member.group.towards_end {
                group()
                    .map(|m| m.size - m.baseline + m.margin.1)
                    .fold(f32::NEG_INFINITY, f32::max)
            } else {
                group()
                    .map(|m| m.margin.0 + m.baseline)
                    .fold(f32::NEG_INFINITY, f32::max)
            };
            for placed in group() {
                // The position along the axis that the baseline of the item
                // takes in its own room.
                let position = if member.group.towards_end {
                    placed.room.1 - depth
                } else {
                    placed.room.0 + depth
                };
                let start = position - placed.baseline + placed.offset;
                let mut layout = tree.nodes[placed.child].layout;
                let shift = start - self.axis.coordinate(layout.location);
                if shift != 0.0 {
                    shifts.push((placed.child, shift));
                }
                layout.location = self.axis.moved(layout.location, start);
                tree.set_unrounded_layout(NodeId::from(placed.child), &layout);
                let reach = reach_of(layout.location, layout.scrollable_overflow_rect);
                overflow = Some(overflow.map_or(reach, |seen: Edges<f32>| seen.union(reach)));
            }
        }
        match self.follow(tree, idx, &shifts, candidates) {
            Some(reach) => overflow.map(|seen| seen.union(reach)),
            None => overflow,
        }
    }

    /// Carry the out-of-flow boxes whose static position lies inside a moved
    /// item along with it, as the box that is their static parent: those that
    /// the container `idx` has laid out, and the `candidates` that continue
    /// to bubble up. `shifts` lists each moved item with the distance it
    /// moved. Returns the scrollable overflow that the laid out boxes that
    /// moved reach, relative to the container.
    fn follow(
        &self,
        tree: &mut LayoutSession<'_>,
        idx: usize,
        shifts: &[(usize, f32)],
        candidates: &mut OofCandidates,
    ) -> Option<Edges<f32>> {
        if shifts.is_empty() {
            return None;
        }
        let axis = self.axis;
        candidates.for_each_mut(|candidate| {
            if let Some(shift) = shift_of(tree, idx, shifts, usize::from(candidate.node)) {
                let position = match axis {
                    Axis::Horizontal => &mut candidate.static_position.x,
                    Axis::Vertical => &mut candidate.static_position.y,
                };
                position.area.start += shift;
                position.area.end += shift;
            }
        });
        let mut overflow = None;
        for child in hoisted_children(tree, idx) {
            // A box that an inset anchors in the axis does not depend on its
            // static position.
            let follows = tree
                .oof_coordinates
                .get(&child)
                .is_some_and(|c| match axis {
                    Axis::Horizontal => c.follow_static_x,
                    Axis::Vertical => c.follow_static_y,
                });
            let Some(shift) = shift_of(tree, idx, shifts, child).filter(|_| follows) else {
                continue;
            };
            let mut layout = tree.nodes[child].layout;
            layout.location = axis.moved(layout.location, axis.coordinate(layout.location) + shift);
            tree.set_unrounded_layout(NodeId::from(child), &layout);
            let reach = reach_of(layout.location, layout.scrollable_overflow_rect);
            overflow = Some(overflow.map_or(reach, |seen: Edges<f32>| seen.union(reach)));
        }
        overflow
    }
}

/// How far a box at `location` whose content reaches `overflow` from its
/// own border box reaches from the origin of its parent.
fn reach_of(location: Point<f32>, overflow: Edges<f32>) -> Edges<f32> {
    Edges {
        left: location.x + overflow.left,
        right: location.x + overflow.right,
        top: location.y + overflow.top,
        bottom: location.y + overflow.bottom,
    }
}

/// The distance the item of the container `idx` that holds `node` moved, when
/// it did.
fn shift_of(
    tree: &LayoutSession<'_>,
    idx: usize,
    shifts: &[(usize, f32)],
    node: usize,
) -> Option<f32> {
    // The item is the last box on the way up from `node` before the container.
    let item = std::iter::successors(Some(node), |&current| tree.taffy_parent_of(current))
        .take_while(|&current| current != idx)
        .last()?;
    shifts
        .iter()
        .find(|&&(moved, _)| moved == item)
        .map(|&(_, shift)| shift)
}

/// Align the items of the grid container `idx` that align by a baseline, once
/// the grid algorithm that reported `grid` has placed them: along the columns,
/// which are the inline axis of the container, by the baselines they share
/// with it, and, in a vertical container, along the rows by those of the
/// groups the items that run against its block direction make of their own.
/// `axes` maps the layout axes of a vertical container to physical ones, and is
/// `None` for a horizontal one.
///
/// Returns the scrollable overflow that the moved items reach, relative to the
/// border box of the container.
pub(super) fn align_grid(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    grid: &GridInfo,
    axes: Option<GridAxes>,
    candidates: &mut OofCandidates,
) -> Option<Edges<f32>> {
    let parent = tree.physical_style(idx);
    let inline = Container::new(
        tree,
        idx,
        &parent,
        if axes.is_some() {
            Axis::Vertical
        } else {
            Axis::Horizontal
        },
        true,
    );
    let columns = grid_members(tree, idx, grid, &parent, &inline, None);
    let opposed = Container::opposed(tree, idx);
    let rows = axes.map(|axes| grid_members(tree, idx, grid, &parent, &opposed, Some(axes)));
    let columns = inline.align(tree, idx, &columns, candidates);
    let rows = rows.and_then(|rows| opposed.align(tree, idx, &rows, candidates));
    union_of(columns, rows)
}

/// The members of the grid container `idx` for the one `container` pass: the
/// items that align along the columns, or, with the `block` axes of the
/// container given, along the rows.
fn grid_members(
    tree: &LayoutSession<'_>,
    idx: usize,
    grid: &GridInfo,
    parent: &LayoutStyleView<'_>,
    container: &Container,
    block: Option<GridAxes>,
) -> Vec<Member> {
    tree.nodes[idx]
        .grid_item_column_starts
        .iter()
        .zip(grid.items())
        .enumerate()
        .filter_map(|(index, (&(child, _), lines))| {
            let (room, tracks) = match block {
                Some(axes) => (
                    axes.block_range_to_physical(grid.item_block_range(index)?),
                    (lines.row_start, lines.row_end - 1),
                ),
                None => (
                    grid.item_inline_range(index)?,
                    (lines.column_start, lines.column_end - 1),
                ),
            };
            container.member(tree, parent, child, tracks, room)
        })
        .collect()
}

/// Align the items of the flex container `idx`, whose border box is `size`,
/// that align by a baseline along its cross axis, once the flex algorithm has
/// placed them. `vertical` tells whether the container is in a vertical
/// writing mode, and `percent_basis` is what the percentages of its padding
/// refer to.
///
/// The cross axis of a column container is its inline axis, along which
/// Taffy shares no baselines. That of a row container is its block axis, along
/// which Taffy shares the baselines of the items that run in the block
/// direction of the container; the items of a vertical row that run against it
/// are aligned here, as groups of their own.
///
/// A container that wraps its items is left to the flex algorithm, which
/// aligns by baselines in a line of its own.
///
/// Returns the scrollable overflow that the moved items reach, relative to the
/// border box of the container.
pub(super) fn align_flex(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    size: Size<f32>,
    vertical: bool,
    percent_basis: Option<f32>,
    candidates: &mut OofCandidates,
) -> Option<Edges<f32>> {
    let style = tree.physical_style(idx);
    if style.flex_wraps() {
        return None;
    }
    let (axis, container) = match (style.flex_direction_is_row(), vertical) {
        (true, false) => return None,
        (true, true) => (Axis::Horizontal, Container::opposed(tree, idx)),
        (false, vertical) => {
            let axis = if vertical {
                Axis::Vertical
            } else {
                Axis::Horizontal
            };
            (axis, Container::new(tree, idx, &style, axis, false))
        }
    };
    let (border, padding) = (
        style.border().resolve_or_zero(percent_basis),
        style.padding().resolve_or_zero(percent_basis),
    );
    // The line is as large as the content box of the container.
    let (start, end) = axis.sides(border + padding);
    let room = (start, axis.extent(size) - end);
    let members: Vec<Member> = TaffyChildIter::new_for_session(tree, idx)
        .filter(|&child| {
            tree.nodes[child].layout_display() != Display::None
                && tree.nodes[child].layout_position() != Position::Absolute
        })
        .filter_map(|child| container.member(tree, &style, child, (0, 0), room))
        .collect();
    if members.is_empty() {
        return None;
    }
    container.align(tree, idx, &members, candidates)
}

/// The scrollable overflow that reaches as far as both `first` and `second`.
fn union_of(first: Option<Edges<f32>>, second: Option<Edges<f32>>) -> Option<Edges<f32>> {
    match (first, second) {
        (Some(first), Some(second)) => Some(first.union(second)),
        (first, second) => first.or(second),
    }
}
