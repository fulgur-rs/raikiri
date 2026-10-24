//! Baseline alignment along the inline axis of a grid or a column flex
//! container.
//!
//! Taffy shares baselines along the rows of a row only: it places a grid item
//! with `justify-self: baseline`, and an item of a column flex container with
//! `align-self: baseline`, as if it were aligned to `start`. The container
//! places its items first, and the items that ask for a baseline are moved
//! here, on physical axes, to where their baseline-sharing groups put them
//! (CSS Box Alignment 3 §9.3).
//!
//! The items of a grid share a baseline when they align by the same set of
//! baselines in the same column, and those of a column flex container when
//! they sit on its one line. A group packs its items against the edge of the
//! room they are aligned in that the first baselines of its items face, or the
//! opposite one for last baselines, and puts the baselines of all of them at
//! one position: the farthest one from that edge, counting the margins.
//!
//! A box has no baseline along its own inline axis, so an item in the writing
//! mode of the container is given a synthesized one. An item in the orthogonal
//! writing mode has its block axis along the alignment axis, and shares the
//! baseline of its first line.
// spec: https://www.w3.org/TR/css-align-3/#baseline-sharing-group

use super::{TaffyChildIter, block_right_to_left};
use crate::layout::context::LayoutSession;
use crate::layout::protocol::NodeId;
use crate::layout::taffy_adapter::GridInfo;
use raikiri_layout_geometry::{Edges, Point, Size};
use raikiri_layout_style::LayoutStyleView;
use raikiri_layout_style::values::{BaselineSet, Direction, Display, Position, ResolveOrZero};

/// The physical axis the items are aligned along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
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
    /// Whether the `direction` of the container is `rtl`, which decides the
    /// side a horizontal container packs first baselines against.
    right_to_left: bool,
    /// Whether the dominant baseline of the container is the central one.
    central: bool,
    /// Whether the property aligning the items along the axis is
    /// `justify-self`, rather than `align-self`.
    justify: bool,
}

impl Container {
    /// The container `idx` of style `style` aligning along `axis`.
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
        let (baseline, towards_end) = if tree.is_orthogonal_to_parent(child) {
            // The block axis of the item runs along the axis, from its
            // block-start edge: the baseline of its first line sits at an
            // offset from there, and the first baselines face that edge.
            let block_start_at_end =
                self.axis == Axis::Horizontal && block_right_to_left(tree, child) == Some(true);
            let offset = tree.nodes[child].orthogonal_first_baseline.unwrap_or(
                if self.axis == Axis::Vertical && self.central {
                    size / 2.0
                } else {
                    0.0
                },
            );
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
        let track = match set {
            BaselineSet::First => tracks.0,
            BaselineSet::Last => tracks.1,
        };
        // Taffy places the box at the start of the room and then adds the
        // offset of a relative box, which moving the box must keep.
        let offset = if style.position() == Position::Relative {
            let start = match self.axis {
                Axis::Horizontal => layout.location.x,
                Axis::Vertical => layout.location.y,
            };
            start
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

    /// Move the `members` to their groups' baselines; the scrollable overflow
    /// that the moved items reach, relative to the container, or `None` when
    /// nothing moved.
    fn align(&self, tree: &mut LayoutSession<'_>, members: &[Member]) -> Option<Edges<f32>> {
        let mut overflow = None;
        for (index, member) in members.iter().enumerate() {
            if members[..index].iter().any(|m| m.group == member.group) {
                continue;
            }
            let group = members.iter().filter(|m| m.group == member.group);
            // The position along the axis that every baseline of the group
            // takes: the farthest one from the edge the group packs against.
            let position = if member.group.towards_end {
                member.room.1
                    - group
                        .map(|m| m.size - m.baseline + m.margin.1)
                        .fold(f32::NEG_INFINITY, f32::max)
            } else {
                member.room.0
                    + group
                        .map(|m| m.margin.0 + m.baseline)
                        .fold(f32::NEG_INFINITY, f32::max)
            };
            for placed in members.iter().filter(|m| m.group == member.group) {
                let start = position - placed.baseline + placed.offset;
                let mut layout = tree.nodes[placed.child].layout;
                layout.location = self.axis.moved(layout.location, start);
                tree.set_unrounded_layout(NodeId::from(placed.child), &layout);
                let reach = Edges {
                    left: layout.location.x + layout.scrollable_overflow_rect.left,
                    right: layout.location.x + layout.scrollable_overflow_rect.right,
                    top: layout.location.y + layout.scrollable_overflow_rect.top,
                    bottom: layout.location.y + layout.scrollable_overflow_rect.bottom,
                };
                overflow = Some(overflow.map_or(reach, |seen: Edges<f32>| seen.union(reach)));
            }
        }
        overflow
    }
}

/// Align the items of the grid container `idx` that align by a baseline along
/// its inline axis, once the grid algorithm that reported `grid` has placed
/// them. `vertical` tells whether the inline axis is the physical y axis.
///
/// Returns the scrollable overflow that the moved items reach, relative to the
/// border box of the container.
pub(super) fn align_grid(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    grid: &GridInfo,
    vertical: bool,
) -> Option<Edges<f32>> {
    let parent = tree.physical_style(idx);
    let container = Container::new(
        tree,
        idx,
        &parent,
        if vertical {
            Axis::Vertical
        } else {
            Axis::Horizontal
        },
        true,
    );
    let members: Vec<Member> = tree.nodes[idx]
        .grid_item_column_starts
        .iter()
        .zip(grid.items())
        .enumerate()
        .filter_map(|(index, (&(child, _), lines))| {
            let room = grid.item_inline_range(index)?;
            container.member(
                tree,
                &parent,
                child,
                (lines.column_start, lines.column_end - 1),
                room,
            )
        })
        .collect();
    if members.is_empty() {
        return None;
    }
    container.align(tree, &members)
}

/// Align the items of the column flex container `idx`, whose border box is
/// `size`, that align by a baseline along its cross axis, which is its inline
/// axis, once the flex algorithm has placed them. `percent_basis` is what the
/// percentages of the padding of the container refer to.
///
/// A container that wraps its items is left to the flex algorithm, which
/// aligns by baselines in a line of its own.
///
/// Returns the scrollable overflow that the moved items reach, relative to the
/// border box of the container.
pub(super) fn align_column_flex(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    size: Size<f32>,
    vertical: bool,
    percent_basis: Option<f32>,
) -> Option<Edges<f32>> {
    let style = tree.physical_style(idx);
    if style.flex_direction_is_row() || style.flex_wraps() {
        return None;
    }
    let (border, padding) = (
        style.border().resolve_or_zero(percent_basis),
        style.padding().resolve_or_zero(percent_basis),
    );
    let axis = if vertical {
        Axis::Vertical
    } else {
        Axis::Horizontal
    };
    // The line is as large as the content box of the container.
    let (start, end) = axis.sides(border + padding);
    let room = (start, axis.extent(size) - end);
    let container = Container::new(tree, idx, &style, axis, false);
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
    container.align(tree, &members)
}
