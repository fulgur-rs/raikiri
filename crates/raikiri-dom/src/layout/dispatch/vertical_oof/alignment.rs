//! Self-alignment of the absolutely positioned boxes of a container on the
//! axes of a vertical writing mode: a block, flex or grid container.
//!
//! Taffy knows only `horizontal-tb`. It aligns an absolutely positioned box
//! in the physical horizontal axis by `justify-self`, from the left edge
//! unless `direction` is `rtl`, and in the physical vertical axis by
//! `align-self`, from the top edge. CSS Position 3 applies the properties to
//! the axes of the containing block's writing mode instead
//! (<https://drafts.csswg.org/css-position-3/#abspos-alignment>): in a
//! vertical mode `align-self` places the box along the physical horizontal
//! block axis, which runs from right to left in `vertical-rl` and
//! `sideways-rl`, and `justify-self` along the physical vertical inline axis,
//! which runs from bottom to top in `sideways-lr` and under `direction: rtl`.
//!
//! The boxes are laid out by Taffy first. This module then places each one
//! again, in every axis in which both insets are definite and no margin is
//! `auto`, with the alignment property and the start edge of that axis.
//! Sizes are left as Taffy found them, and so are the axes in which an inset
//! is `auto`, which the static position handles. The scrollable overflow
//! Taffy merged from the boxes is recomputed for their new positions.

use crate::layout::protocol::{NodeId, OofPositioningArea};
use raikiri_layout_geometry::{Edges, Line, Point, Size};
use raikiri_layout_style::values::{
    AlignItemsKeyword, AlignSelf, AlignmentSafety, Contain, Direction, MaybeResolve, Overflow,
};

use super::super::grid_oof::GridAreas;
use super::super::{block_right_to_left, hoisted_children};
use super::used_writing_mode;
use crate::layout::context::LayoutSession;
use crate::layout::style_views::ReadRole;
use raikiri_style::property::WritingMode;

/// A physical axis of the positioning area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// The self-alignment of a box in one axis, from the point of view of the
/// containing block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placement {
    /// `normal`: stretch-fit sizing for a box that can be stretched,
    /// otherwise the start edge.
    Normal,
    /// `stretch`: stretch-fit sizing, otherwise the start edge.
    Stretch,
    Start,
    Center,
    End,
}

/// A physical edge of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Start,
    Center,
    End,
}

/// Whether the logical start edge of a box with writing mode `mode` and
/// bridged `direction` is the physical end edge of `axis`: the right edge of
/// the horizontal axis, the bottom edge of the vertical one.
///
/// The bridge has already run the inline axis of a `sideways-lr` box from
/// bottom to top in its `direction`, and ignores `direction` under
/// `text-orientation: upright`.
fn start_is_end(mode: WritingMode, direction: Direction, axis: Axis) -> bool {
    let rtl = direction == Direction::Rtl;
    match (mode, axis) {
        (WritingMode::HorizontalTb, Axis::Horizontal) => rtl,
        (WritingMode::HorizontalTb, Axis::Vertical) => false,
        // The block axis runs from right to left.
        (WritingMode::VerticalRl | WritingMode::SidewaysRl, Axis::Horizontal) => true,
        (_, Axis::Horizontal) => false,
        (_, Axis::Vertical) => rtl,
    }
}

/// `alignment` as a [`Placement`], with `self-start` and `self-end` resolved:
/// they name the edges of the box's own writing mode, which are the
/// containing block's when `own_start_is_end` equals `start_is_end`.
fn placement(
    alignment: Option<AlignSelf>,
    own_start_is_end: bool,
    start_is_end: bool,
) -> Placement {
    let flip = own_start_is_end != start_is_end;
    match alignment.unwrap_or(AlignSelf::NORMAL).keyword {
        AlignItemsKeyword::Normal => Placement::Normal,
        AlignItemsKeyword::Stretch => Placement::Stretch,
        AlignItemsKeyword::Center => Placement::Center,
        AlignItemsKeyword::SelfStart if flip => Placement::End,
        AlignItemsKeyword::SelfEnd if flip => Placement::Start,
        AlignItemsKeyword::End | AlignItemsKeyword::FlexEnd | AlignItemsKeyword::SelfEnd => {
            Placement::End
        }
        // The fallback alignment of `baseline` is `start`.
        AlignItemsKeyword::Start
        | AlignItemsKeyword::FlexStart
        | AlignItemsKeyword::SelfStart
        | AlignItemsKeyword::Baseline => Placement::Start,
    }
}

/// Where a box of `size` is placed within `line` when aligned to `edge`.
fn align_in_line(line: Line<f32>, size: f32, edge: Edge) -> f32 {
    match edge {
        Edge::Start => line.start,
        Edge::Center => (line.start + line.end - size) / 2.0,
        Edge::End => line.end - size,
    }
}

/// One axis of an absolutely positioned box whose insets are both definite.
struct AxisBox {
    /// The positioning area in border-box coordinates.
    area: Line<f32>,
    /// The resolved physical insets.
    inset: Line<f32>,
    /// The resolved physical margins.
    margin: Line<f32>,
    /// The border-box size.
    size: f32,
    placement: Placement,
    safety: AlignmentSafety,
    /// Whether the logical start edge of the containing block is the
    /// physical end edge.
    reversed: bool,
    /// Whether the containing block scrolls in this axis.
    scrolls: bool,
}

impl AxisBox {
    /// The border-box start of the box within the inset-modified containing
    /// block, in border-box coordinates of the positioning area's container
    /// (<https://drafts.csswg.org/css-position-3/#resolving-insets>).
    fn location(&self) -> f32 {
        let cb_size = self.area.end - self.area.start;
        let mut imcb = Line {
            start: self.inset.start,
            end: cb_size - self.inset.end,
        };
        // Over-constrained insets: the one on the logical end side is the
        // weaker, and shrinks until the box has no room left.
        if imcb.end < imcb.start {
            if self.reversed {
                imcb.start = imcb.end;
            } else {
                imcb.end = imcb.start;
            }
        }
        let start_edge = if self.reversed {
            Edge::End
        } else {
            Edge::Start
        };
        let edge = match self.placement {
            Placement::Center => Edge::Center,
            Placement::End if self.reversed => Edge::Start,
            Placement::End => Edge::End,
            Placement::Normal | Placement::Stretch | Placement::Start => start_edge,
        };
        let margin_box = self.size + self.margin.start + self.margin.end;
        let imcb_size = imcb.end - imcb.start;
        let start = match self.safety {
            AlignmentSafety::Unsafe => align_in_line(imcb, margin_box, edge),
            AlignmentSafety::Safe => {
                let edge = if margin_box > imcb_size {
                    start_edge
                } else {
                    edge
                };
                align_in_line(imcb, margin_box, edge)
            }
            // A box that overflows is moved to minimize the overflow of its
            // containing block, as Taffy does for horizontal writing
            // (<https://www.w3.org/TR/css-align-3/#auto-safety-position>).
            AlignmentSafety::Default => {
                if margin_box <= imcb_size || self.placement == Placement::Normal {
                    align_in_line(imcb, margin_box, edge)
                } else {
                    let mut limit = Line {
                        start: imcb.start.min(0.0),
                        end: imcb.end.max(cb_size),
                    };
                    if self.scrolls {
                        if self.reversed {
                            limit.start = f32::NEG_INFINITY;
                        } else {
                            limit.end = f32::INFINITY;
                        }
                    }
                    if margin_box <= limit.end - limit.start {
                        align_in_line(imcb, margin_box, edge)
                            .min(limit.end - margin_box)
                            .max(limit.start)
                    } else {
                        align_in_line(limit, margin_box, start_edge)
                    }
                }
            }
        };
        self.area.start + start + self.margin.start
    }
}

/// The scrollable overflow that the boxes in `children` contribute to their
/// containing block when they are at the positions of their layouts, laid out
/// against `area`: what Taffy merges for them once it has laid them out.
///
/// The rectangle is measured in physical coordinates from the top left corner
/// of the padding box, as the layout of a container on vertical axes reports
/// its own. A reversed inline direction moves where the container starts to
/// scroll along the vertical axis, never the horizontal one, so `direction`
/// does not mirror it.
fn hoisted_overflow(
    tree: &LayoutSession<'_>,
    children: &[usize],
    area: OofPositioningArea,
    container_overflow: Point<Overflow>,
) -> Edges<f32> {
    let container_scrolls =
        container_overflow.x.is_scroll_container() || container_overflow.y.is_scroll_container();
    let mut rect = Edges::ZERO;
    for &child in children {
        let layout = tree.nodes[child].layout;
        let style = tree.physical_style(child);
        let location = Point {
            x: layout.location.x - area.offset.x,
            y: layout.location.y - area.offset.y,
        };
        rect = rect.union(overflow_contribution(
            location,
            layout.size,
            layout.scrollable_overflow_rect,
            style.overflow(),
            style.contain(),
            container_scrolls,
        ));
    }
    rect
}

/// The scrollable overflow rect that a box at `location` with `size` and its
/// own `overflow_rect` contributes to a container that scrolls when
/// `container_scrolls` (<https://drafts.csswg.org/css-overflow-3/#scrollable>).
/// A box that clips or contains its overflow contributes its border box only.
fn overflow_contribution(
    location: Point<f32>,
    size: Size<f32>,
    overflow_rect: Edges<f32>,
    overflow: Point<Overflow>,
    contain: Contain,
    container_scrolls: bool,
) -> Edges<f32> {
    let clips = overflow.x.is_scroll_container()
        || overflow.y.is_scroll_container()
        || contain.contains_scrollable_overflow();
    let propagates = |axis: Overflow| !clips && axis == Overflow::Visible;
    let (propagates_x, propagates_y) = (propagates(overflow.x), propagates(overflow.y));
    let end = Size {
        width: if propagates_x {
            size.width.max(overflow_rect.right)
        } else {
            size.width
        },
        height: if propagates_y {
            size.height.max(overflow_rect.bottom)
        } else {
            size.height
        },
    };
    if end.width <= 0.0 || end.height <= 0.0 {
        return Edges::ZERO;
    }
    let start = Point {
        x: if propagates_x {
            overflow_rect.left.min(0.0)
        } else {
            0.0
        },
        y: if propagates_y {
            overflow_rect.top.min(0.0)
        } else {
            0.0
        },
    };
    let contribution = Edges {
        left: location.x + start.x,
        right: location.x + end.width,
        top: location.y + start.y,
        bottom: location.y + end.height,
    };
    // What lies before the origin of a scroll container cannot be reached.
    if container_scrolls && (contribution.right <= 0.0 || contribution.bottom <= 0.0) {
        Edges::ZERO
    } else {
        contribution
    }
}

/// Align the absolutely positioned boxes that `idx` is the containing block
/// of, laid out against `area`, when `idx` is a vertical block, flex or grid
/// container or the root of vertical lines.
///
/// Returns the scrollable overflow of all the boxes at their positions, to
/// replace what Taffy merged for them, when a box moved.
pub(in crate::layout::dispatch) fn align_hoisted_children(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    area: OofPositioningArea,
) -> Option<Edges<f32>> {
    align_children(tree, idx, area, None)
}

/// [`align_hoisted_children`] for a vertical grid container, aligning each box
/// in the grid area of its grid placement within `area`, the padding box of
/// the container. Without the `grid` areas, which taffy reports only from a
/// layout it performs, every box is aligned in `area`.
pub(in crate::layout::dispatch) fn align_hoisted_children_in_grid(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    area: OofPositioningArea,
    grid: Option<&GridAreas<'_>>,
) -> Option<Edges<f32>> {
    align_children(tree, idx, area, grid)
}

/// The scrollable overflow that the boxes `idx` is the containing block of
/// contribute at their positions, laid out against `area`.
pub(in crate::layout::dispatch) fn hoisted_overflow_rect(
    tree: &LayoutSession<'_>,
    idx: usize,
    area: OofPositioningArea,
) -> Edges<f32> {
    let overflow = tree.physical_style(idx).overflow();
    hoisted_overflow(tree, &hoisted_children(tree, idx), area, overflow)
}

fn align_children(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    area: OofPositioningArea,
    grid: Option<&GridAreas<'_>>,
) -> Option<Edges<f32>> {
    // Only a box laid out on vertical axes has the physical axes swapped.
    block_right_to_left(tree, idx)?;
    let mode = used_writing_mode(tree, idx);
    let (direction, overflow) = {
        let container = tree.physical_style(idx);
        (container.direction(), container.overflow())
    };
    let children = hoisted_children(tree, idx);
    let overall_area = area;
    let mut moved_any = false;
    for &child in &children {
        let area = match grid {
            Some(grid) => {
                let style = tree.query_style(child, ReadRole::Oof);
                grid.positioning_area(&style, direction, overall_area)
            }
            None => overall_area,
        };
        let child_mode = used_writing_mode(tree, child);
        let mut layout = tree.nodes[child].layout;
        let (child_inset, child_margin, child_direction, align_self, justify_self) = {
            let style = tree.physical_style(child);
            (
                style.inset(),
                style.margin(),
                style.direction(),
                style.bridge_align_self(),
                style.bridge_justify_self(),
            )
        };
        let mut moved = false;
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let (area_line, inset, margin, alignment, size, scroll) = match axis {
                // The block axis, aligned by `align-self`.
                Axis::Horizontal => (
                    Line {
                        start: area.offset.x,
                        end: area.offset.x + area.size.width,
                    },
                    Line {
                        start: child_inset.left,
                        end: child_inset.right,
                    },
                    Line {
                        start: child_margin.left,
                        end: child_margin.right,
                    },
                    align_self,
                    layout.size.width,
                    overflow.x,
                ),
                // The inline axis, aligned by `justify-self`.
                Axis::Vertical => (
                    Line {
                        start: area.offset.y,
                        end: area.offset.y + area.size.height,
                    },
                    Line {
                        start: child_inset.top,
                        end: child_inset.bottom,
                    },
                    Line {
                        start: child_margin.top,
                        end: child_margin.bottom,
                    },
                    justify_self,
                    layout.size.height,
                    overflow.y,
                ),
            };
            let cb_size = area_line.end - area_line.start;
            let (Some(inset_start), Some(inset_end)) = (
                inset.start.maybe_resolve(cb_size),
                inset.end.maybe_resolve(cb_size),
            ) else {
                continue;
            };
            if margin.start.is_auto() || margin.end.is_auto() {
                continue;
            }
            let (margin_start, margin_end) = match axis {
                Axis::Horizontal => (layout.margin.left, layout.margin.right),
                Axis::Vertical => (layout.margin.top, layout.margin.bottom),
            };
            let reversed = start_is_end(mode, direction, axis);
            let own_start_is_end = start_is_end(child_mode, child_direction, axis);
            let aligned = AxisBox {
                area: area_line,
                inset: Line {
                    start: inset_start,
                    end: inset_end,
                },
                margin: Line {
                    start: margin_start,
                    end: margin_end,
                },
                size,
                placement: placement(alignment, own_start_is_end, reversed),
                safety: alignment.map_or(AlignmentSafety::Default, |value| value.safety),
                reversed,
                scrolls: scroll.is_scroll_container(),
            }
            .location();
            match axis {
                Axis::Horizontal => layout.location.x = aligned,
                Axis::Vertical => layout.location.y = aligned,
            }
            moved = true;
        }
        if moved {
            tree.set_unrounded_layout(NodeId::from(child), &layout);
            moved_any = true;
        }
    }
    moved_any.then(|| hoisted_overflow(tree, &children, overall_area, overflow))
}

#[cfg(test)]
mod tests;
