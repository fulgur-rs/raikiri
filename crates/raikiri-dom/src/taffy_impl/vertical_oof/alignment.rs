//! Self-alignment of the absolutely positioned boxes of a vertical block
//! container.
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
//! Taffy merged from a box is not recomputed for its new position.

use taffy::util::MaybeResolve;
use taffy::{
    AlignItemsKeyword, AlignSelf, AlignmentSafety, Direction, LayoutPartialTree, Line, NodeId,
    OofPositioningArea,
};

use super::super::{block_right_to_left, hoisted_children, resolve_calc};
use super::used_writing_mode;
use crate::layout::context::LayoutSession;
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

/// Align the absolutely positioned boxes that `idx` is the containing block
/// of, laid out against `area`, when `idx` is a vertical block container or
/// the root of vertical lines.
pub(in crate::taffy_impl) fn align_hoisted_children(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    area: OofPositioningArea,
) {
    // Only a box laid out on vertical axes has the physical axes swapped.
    if block_right_to_left(tree, idx).is_none() {
        return;
    }
    let mode = used_writing_mode(tree, idx);
    let container = &tree.layout_node_with_style(idx).unwrap().style;
    let direction = container.direction;
    let overflow = container.overflow;
    let children = hoisted_children(tree, idx);
    for child in children {
        let child_mode = used_writing_mode(tree, child);
        let node = tree.layout_node_with_style(child).unwrap();
        let style = &node.style;
        let mut layout = node.unrounded_layout;
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
                        start: style.inset.left,
                        end: style.inset.right,
                    },
                    Line {
                        start: style.margin.left,
                        end: style.margin.right,
                    },
                    style.align_self,
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
                        start: style.inset.top,
                        end: style.inset.bottom,
                    },
                    Line {
                        start: style.margin.top,
                        end: style.margin.bottom,
                    },
                    style.justify_self,
                    layout.size.height,
                    overflow.y,
                ),
            };
            let cb_size = area_line.end - area_line.start;
            let (Some(inset_start), Some(inset_end)) = (
                inset.start.maybe_resolve(cb_size, resolve_calc),
                inset.end.maybe_resolve(cb_size, resolve_calc),
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
            let own_start_is_end = start_is_end(child_mode, style.direction, axis);
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
        }
    }
}

#[cfg(test)]
mod tests;
