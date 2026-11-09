//! The room each flex line gives the items that align by a baseline.
//!
//! A column container that wraps its items shares a baseline among the items of
//! each line, and has to know where each line starts and ends across the
//! container for that. The flex algorithm keeps its lines private, and puts an
//! item that aligns by a baseline against one edge of its line, whatever the
//! baselines of the others in the line are. So the container replays what its
//! items reported, once with those items at the start of their line and once at
//! the end: the two places are the edges of the room each item is aligned in,
//! and the items with the same edges share a line. No child is laid out again.

use std::collections::{HashMap, HashSet};

use super::FlexReplayCall;
use super::wrap_reverse::{Overrides, replay_with};
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutInput, NodeId};
use crate::layout::style_views::ReadRole;
use raikiri_layout_geometry::Size;
use raikiri_layout_style::values::{AlignContent, AlignSelf, Direction};

/// How far apart two edges that are the same may be, in pixels, for rounding
/// of the sums that the flex algorithm makes of them.
pub(crate) const SAME_EDGE: f32 = 0.01;

/// The room that a flex line gives an item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LineRoom {
    /// The line the item is in, counting from zero in the order of the edges.
    pub(crate) line: u16,
    /// The near and far edge of the line across the container, relative to its
    /// border box. They are the left and right edges of a container whose
    /// lines run across horizontally, and the top and bottom edges of one that
    /// runs vertically: the replay is made on the layout axes of the container,
    /// where the inline axis is the physical vertical one in that case.
    pub(crate) edges: (f32, f32),
}

impl LineRoom {
    /// The size of the line across the container.
    pub(crate) fn size(&self) -> f32 {
        (self.edges.1 - self.edges.0).max(0.0)
    }
}

/// The room the line of each of the `members` gives it, where `members` are
/// the items of the column container `node` that align by a baseline, or `None`
/// when the replay does not reproduce the layout that placed the items.
/// `calls` are the results the children reported for `inputs`, `size` is the
/// border box the container settled on, and `content` is the alignment of the
/// lines that placed them, when it is not the container's own.
pub(crate) fn rooms(
    tree: &LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    calls: &[FlexReplayCall],
    size: Size<f32>,
    members: &HashSet<usize>,
    content: Option<AlignContent>,
) -> Option<HashMap<usize, LineRoom>> {
    cross_rooms(tree, node, inputs, calls, size, members, content, false)
}

/// [`rooms`] for a container whose lines run along its rows when `row`, so
/// that its lines lie across the vertical axis and their edges are the top
/// and bottom edges of a line, or along its columns otherwise.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cross_rooms(
    tree: &LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    calls: &[FlexReplayCall],
    size: Size<f32>,
    members: &HashSet<usize>,
    content: Option<AlignContent>,
    row: bool,
) -> Option<HashMap<usize, LineRoom>> {
    let replay = |alignment| {
        replay_with(
            tree,
            node,
            inputs,
            calls,
            Overrides {
                content,
                items: Some((members, alignment)),
            },
        )
        .filter(|(output, _)| output.size == size)
    };
    let (_, at_start) = replay(AlignSelf::START)?;
    let (_, at_end) = replay(AlignSelf::END)?;
    let rtl = tree
        .query_style(usize::from(node), ReadRole::FlexContainer)
        .direction()
        == Direction::Rtl;
    let mut edges = Vec::with_capacity(members.len());
    for &child in members {
        let (start, end) = (at_start.get(&child)?, at_end.get(&child)?);
        if row {
            edges.push((
                start.location.y - start.margin.top,
                end.location.y + end.size.height + end.margin.bottom,
                child,
            ));
            continue;
        }
        // `start` is the left edge of the line in a left-to-right container and
        // the right edge in a right-to-left one, and `end` the other.
        let (left, right) = if rtl {
            (
                end.location.x - end.margin.left,
                start.location.x + start.size.width + start.margin.right,
            )
        } else {
            (
                start.location.x - start.margin.left,
                end.location.x + end.size.width + end.margin.right,
            )
        };
        edges.push((left, right, child));
    }
    edges.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut rooms = HashMap::with_capacity(edges.len());
    let mut previous: Option<(f32, f32)> = None;
    let mut line = 0_u16;
    for (left, right, child) in edges {
        // An item that is on the edges of the one before it is in its line.
        if previous.is_some_and(|(near, far)| {
            (near - left).abs() > SAME_EDGE || (far - right).abs() > SAME_EDGE
        }) {
            line = line.saturating_add(1);
        }
        previous = Some((left, right));
        rooms.insert(
            child,
            LineRoom {
                line,
                edges: (left, right),
            },
        );
    }
    Some(rooms)
}
