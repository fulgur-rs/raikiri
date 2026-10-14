//! Block-level `::before` and `::after` boxes inside a paragraph.
//!
//! A generated box with a block-level `display` is a block child of its
//! element (CSS 2.1 12.1, CSS Display 3 §2). Its content stays in the
//! element's paragraph, so the box is marked there by two block-in-inline
//! tokens, one at its start and one at its end: the line loop places the box's
//! top margin, border and padding at the start token, and its height, bottom
//! padding, border and margin at the end token. The lines between the tokens
//! are the box's lines, set in by its inline-side edges.

use raikiri_style::property::{BoxSizing, ClearValue, Sides};
use raikiri_style::{
    ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrCalc, ComputedValues,
};
use taffy::{BlockContext, CollapsibleMarginSet};

/// Marks the end token of a block-level `::before` or `::after`: the id of
/// its start token with this bit set. The ids of other content of the
/// paragraph never carry it (see [`end_token_id`]).
const END_TOKEN_BIT: u64 = 1 << 60;

/// The id of the end token of the generated block whose start token is
/// `start`; `None` when `start` already uses the bit, for an element too far
/// into the arena to encode.
pub(crate) fn end_token_id(start: u64) -> Option<u64> {
    (start & END_TOKEN_BIT == 0).then_some(start | END_TOKEN_BIT)
}

/// The box geometry of one block-level `::before` or `::after`, as computed:
/// lengths are resolved when the paragraph is laid out, against its width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeneratedBlock {
    /// Id of the start token, the inline engine's id of the pseudo-element.
    pub(crate) start: u64,
    /// Id of the end token.
    pub(crate) end: u64,
    margin: Sides<ComputedLengthPercentageOrAuto>,
    padding: Sides<ComputedLengthPercentageOrCalc>,
    /// Border widths in px, by side.
    border: Sides<f32>,
    width: ComputedLengthPercentageOrAuto,
    min_width: ComputedLengthPercentageOrAuto,
    max_width: ComputedLengthPercentageOrAuto,
    height: ComputedLengthPercentageOrAuto,
    min_height: ComputedLengthPercentageOrAuto,
    max_height: ComputedLengthPercentageOrAuto,
    border_box: bool,
    clear: taffy::Clear,
}

impl GeneratedBlock {
    /// The geometry of the generated box styled by `cv`, marked by the tokens
    /// `start` and `end`.
    pub(crate) fn new(start: u64, end: u64, cv: &ComputedValues) -> Self {
        Self {
            start,
            end,
            margin: cv.margin,
            padding: cv.padding,
            border: Sides {
                top: cv.border.top.width().px(),
                right: cv.border.right.width().px(),
                bottom: cv.border.bottom.width().px(),
                left: cv.border.left.width().px(),
            },
            width: cv.width,
            min_width: cv.min_width,
            max_width: cv.max_width,
            height: cv.height,
            min_height: cv.min_height,
            max_height: cv.max_height,
            border_box: cv.box_sizing == BoxSizing::BorderBox,
            // Logical sides are not mapped, as for an element's box (see the
            // taffy bridge).
            clear: match cv.clear {
                ClearValue::Left => taffy::Clear::Left,
                ClearValue::Right => taffy::Clear::Right,
                ClearValue::Both => taffy::Clear::Both,
                _ => taffy::Clear::None,
            },
        }
    }

    /// Margins in px against the containing block width `basis`; `auto` is
    /// zero.
    fn margins(&self, basis: f32) -> Sides<f32> {
        let side = |value| length_or_auto(value, Some(basis)).unwrap_or(0.0);
        Sides {
            top: side(self.margin.top),
            right: side(self.margin.right),
            bottom: side(self.margin.bottom),
            left: side(self.margin.left),
        }
    }

    /// Padding plus border in px on each side, against `basis`.
    fn edges(&self, basis: f32) -> Sides<f32> {
        Sides {
            top: self.padding.top.resolve(basis) + self.border.top,
            right: self.padding.right.resolve(basis) + self.border.right,
            bottom: self.padding.bottom.resolve(basis) + self.border.bottom,
            left: self.padding.left.resolve(basis) + self.border.left,
        }
    }

    /// The content-box height of the box whose content is `content` tall,
    /// with `edges` of padding and border above and below. A percentage
    /// height counts only against the definite `definite_height` of the
    /// containing block (CSS 2.1 10.5).
    fn used_height(&self, content: f32, edges: f32, definite_height: Option<f32>) -> f32 {
        let content_box = |value| {
            length_or_auto(value, definite_height).map(|size: f32| {
                if self.border_box {
                    (size - edges).max(0.0)
                } else {
                    size
                }
            })
        };
        let mut height = content_box(self.height).unwrap_or(content);
        if let Some(max) = content_box(self.max_height) {
            height = height.min(max);
        }
        if let Some(min) = content_box(self.min_height) {
            height = height.max(min);
        }
        height.max(0.0)
    }

    /// The physical left and right insets of the box's lines from the
    /// paragraph's content box `width` wide: margin, border and padding.
    /// With a definite width, the margins take the room the box leaves
    /// (CSS 2.1 10.3.3): `auto` margins share it, else the end-side margin
    /// takes it, the left one in a right-to-left paragraph.
    fn inline_insets(&self, width: f32, rtl: bool, edges: &Sides<f32>) -> (f32, f32) {
        let mut left = length_or_auto(self.margin.left, Some(width));
        let mut right = length_or_auto(self.margin.right, Some(width));
        let insets = |left: f32, right: f32| (left + edges.left, right + edges.right);
        let content = match self.content_width(Some(width), edges) {
            Some(content) => content,
            None => {
                // An `auto` width stretches over the room its margins leave.
                // When `min-width` or `max-width` clamps it, the box is placed
                // as one with that definite width (CSS 2.1 10.4).
                let stretched =
                    (width - left.unwrap_or(0.0) - right.unwrap_or(0.0) - edges.left - edges.right)
                        .max(0.0);
                let clamped = self.clamp_width(stretched, Some(width), edges);
                if clamped == stretched {
                    return insets(left.unwrap_or(0.0), right.unwrap_or(0.0));
                }
                clamped
            }
        };
        let free = width - content - edges.left - edges.right;
        // An over-constrained box takes its `auto` margins as zero.
        if left.unwrap_or(0.0) + right.unwrap_or(0.0) > free {
            left = left.or(Some(0.0));
            right = right.or(Some(0.0));
        }
        let (left, right) = match (left, right) {
            (None, None) => (free / 2.0, free / 2.0),
            (None, Some(right)) => (free - right, right),
            (Some(left), None) => (left, free - left),
            (Some(_), Some(right)) if rtl => (free - right, right),
            (Some(left), Some(_)) => (left, free - left),
        };
        insets(left, right)
    }

    /// The definite content-box width of the box, clamped by its minimum and
    /// maximum; `None` for an `auto` width or a percentage without `basis`.
    fn content_width(&self, basis: Option<f32>, edges: &Sides<f32>) -> Option<f32> {
        let content = self.content_box_width(self.width, basis, edges)?;
        Some(self.clamp_width(content, basis, edges))
    }

    /// The content-box width `content` clamped by the box's minimum and
    /// maximum widths, never below zero.
    fn clamp_width(&self, mut content: f32, basis: Option<f32>, edges: &Sides<f32>) -> f32 {
        if let Some(max) = self.content_box_width(self.max_width, basis, edges) {
            content = content.min(max);
        }
        if let Some(min) = self.content_box_width(self.min_width, basis, edges) {
            content = content.max(min);
        }
        content.max(0.0)
    }

    /// The content-box width a width property `value` gives, if definite.
    fn content_box_width(
        &self,
        value: ComputedLengthPercentageOrAuto,
        basis: Option<f32>,
        edges: &Sides<f32>,
    ) -> Option<f32> {
        length_or_auto(value, basis).map(|size| {
            if self.border_box {
                (size - edges.left - edges.right).max(0.0)
            } else {
                size
            }
        })
    }

    /// The margin-box width the box needs at least, for the intrinsic widths
    /// of its paragraph: its definite width, or for an `auto` width its
    /// minimum width, with its edges and margins. Percentages count as zero,
    /// as the width they resolve against is not known yet. The lines of an
    /// `auto`-width box are measured with the paragraph's own lines.
    pub(crate) fn intrinsic_width(&self) -> f32 {
        let edges = self.edges(0.0);
        let content = self
            .content_width(None, &edges)
            .unwrap_or_else(|| self.clamp_width(0.0, None, &edges));
        let margins = self.margins(0.0);
        (content + edges.left + edges.right + margins.left + margins.right).max(0.0)
    }
}

/// A length, or a percentage of `basis` when there is one; `None` for `auto`
/// and the other keywords, for a percentage without a basis, and for a value
/// that is not finite.
fn length_or_auto(value: ComputedLengthPercentageOrAuto, basis: Option<f32>) -> Option<f32> {
    match (value, basis) {
        (ComputedLengthPercentageOrAuto::Px(_), None) => {
            crate::layout::page::used_computed_length_percentage_or_auto(value, 0.0)
        }
        (_, Some(basis)) => {
            crate::layout::page::used_computed_length_percentage_or_auto(value, basis)
        }
        _ => None,
    }
}

/// Which token of a generated block the line loop met.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Token<'a> {
    Start(&'a GeneratedBlock),
    End(&'a GeneratedBlock),
}

/// The token of `blocks` with the id `node`, if any, with the index of its
/// block. The tokens come in document order, so the block at `hint` is
/// looked at first.
pub(crate) fn token(
    blocks: &[GeneratedBlock],
    node: u64,
    hint: usize,
) -> Option<(usize, Token<'_>)> {
    fn matching(index: usize, block: &GeneratedBlock, node: u64) -> Option<(usize, Token<'_>)> {
        if block.start == node {
            Some((index, Token::Start(block)))
        } else if block.end == node {
            Some((index, Token::End(block)))
        } else {
            None
        }
    }
    blocks
        .get(hint)
        .and_then(|block| matching(hint, block, node))
        .or_else(|| {
            blocks
                .iter()
                .enumerate()
                .find_map(|(index, block)| matching(index, block, node))
        })
}

/// A generated block whose start token was placed and whose end token was
/// not, in the line loop.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenBlock {
    /// The number of lines laid out before the box.
    lines_before: usize,
    /// The block offset of the box's content box from the paragraph's
    /// content-box top, once its top margin has been placed: at the start
    /// token when padding, border or clearance separate the margin from the
    /// content, else at the box's first line.
    content_top: Option<f32>,
    /// The margins collapsed at the box's top: those above it with its own.
    top_set: CollapsibleMarginSet,
    /// The box starts the paragraph: nothing is above it.
    starts_paragraph: bool,
    has_clearance: bool,
    /// The content height the box took in earlier fragmentainers, which its
    /// own height, min-height and max-height still count (CSS Fragmentation
    /// 3, 4.4).
    consumed: f32,
    /// The physical left and right insets of the box's lines.
    pub(crate) inline: (f32, f32),
    /// The paragraph is right-to-left: the left side is the end side.
    rtl: bool,
}

impl OpenBlock {
    /// The box as it goes on at the top of the next fragmentainer, after a
    /// fragmentainer whose `lines` end at `end`: the box fills it to there.
    pub(crate) fn continued(self, lines: &[shodo::Line], end: f32) -> Self {
        let top = self
            .content_top
            .or_else(|| lines.get(self.lines_before).map(|line| line.block_offset()));
        Self {
            lines_before: 0,
            content_top: Some(0.0),
            starts_paragraph: false,
            has_clearance: false,
            consumed: self.consumed + top.map_or(0.0, |top| (end - top).max(0.0)),
            ..self
        }
    }

    /// The line space `space` of a line of the box, set in by the box's
    /// inline-side edges from the paragraph's content box `width` wide.
    pub(crate) fn narrow(
        &self,
        space: super::flow::LineSpace,
        width: f32,
    ) -> super::flow::LineSpace {
        // A negative end-side inset, from a negative margin or a box wider
        // than the paragraph, moves the end of the lines out past the
        // paragraph's content box, but not into the room a float takes. The
        // start of a line cannot move before the content box.
        let (left, right) = self.inline;
        let space_end = space.inline_start + space.inline_size;
        let float_start = if space.inline_start > 0.0 {
            space.inline_start
        } else {
            f32::NEG_INFINITY
        };
        let float_end = if space_end < width {
            space_end
        } else {
            f32::INFINITY
        };
        let mut start = left.max(float_start);
        let mut end = (width - right).min(float_end);
        if self.rtl {
            end = end.min(width);
        } else {
            start = start.max(0.0);
        }
        super::flow::LineSpace {
            inline_start: start,
            inline_size: (end - start).max(0.0),
        }
    }
}

/// The state of the line loop a generated block's tokens move.
pub(crate) struct Flow<'a, 'b> {
    /// Block offset of the next box from the paragraph's content-box top.
    pub(crate) y: &'a mut f32,
    /// Margins not placed yet.
    pub(crate) pending: &'a mut CollapsibleMarginSet,
    pub(crate) leading_block_margin: &'a mut Option<CollapsibleMarginSet>,
    pub(crate) lines: &'a [shodo::Line],
    pub(crate) ctx: &'a BlockContext<'b>,
    /// The paragraph's content-box width.
    pub(crate) width: f32,
    /// Border plus padding above the paragraph's content box.
    pub(crate) top_edge: f32,
    pub(crate) definite_height: Option<f32>,
    /// Nothing is placed in the paragraph yet.
    pub(crate) at_start: bool,
    /// The paragraph's lines start at the right.
    pub(crate) rtl: bool,
}

/// Place the top of `block` at its start token: its top margin with the
/// margins above it, its clearance, and its top border and padding.
pub(crate) fn open(block: &GeneratedBlock, flow: Flow<'_, '_>) -> OpenBlock {
    let margins = block.margins(flow.width);
    let edges = block.edges(flow.width);
    let top_set = flow.pending.collapse_with_margin(margins.top);
    // Clearance (CSS 2.1 9.5.2) puts the top border edge below the floats
    // it clears; the margins above no longer collapse with it.
    let clear_y = (block.clear != taffy::Clear::None)
        .then(|| flow.ctx.cleared_threshold(block.clear))
        .flatten()
        .map(|threshold| threshold - flow.top_edge);
    let hypothetical = *flow.y + top_set.resolve();
    let clearance = clear_y
        .filter(|&clear_y| flow.ctx.has_adjoining_float(block.clear) || hypothetical < clear_y);
    let content_top = if let Some(clear_y) = clearance {
        Some(clear_y + edges.top)
    } else if edges.top > 0.0 {
        Some(hypothetical + edges.top)
    } else {
        None
    };
    match content_top {
        Some(top) => {
            *flow.y = top;
            *flow.pending = CollapsibleMarginSet::ZERO;
        }
        None => *flow.pending = top_set,
    }
    OpenBlock {
        lines_before: flow.lines.len(),
        content_top,
        top_set,
        starts_paragraph: flow.at_start,
        has_clearance: clearance.is_some(),
        consumed: 0.0,
        inline: block.inline_insets(flow.width, flow.rtl, &edges),
        rtl: flow.rtl,
    }
}

/// Place the bottom of `block` at its end token: the box's height, from its
/// lines unless it has one of its own, and its bottom padding, border and
/// margin. `open` is the state its start token left.
pub(crate) fn close(block: &GeneratedBlock, open: OpenBlock, flow: Flow<'_, '_>) {
    let margins = block.margins(flow.width);
    let edges = block.edges(flow.width);
    let first_line = flow.lines.get(open.lines_before);
    let content_top = open
        .content_top
        .or_else(|| first_line.map(|line| line.block_offset()));
    let content = content_top.map_or(0.0, |top| (*flow.y - top).max(0.0));
    let height = (block.used_height(
        open.consumed + content,
        edges.top + edges.bottom,
        flow.definite_height,
    ) - open.consumed)
        .max(0.0);
    let Some(top) = content_top.or_else(|| {
        // An empty box whose margins collapse through it (CSS 2.1 8.3.1)
        // adds them to the margins above it.
        (height > 0.0 || edges.bottom > 0.0).then(|| *flow.y + flow.pending.resolve())
    }) else {
        *flow.pending = flow.pending.collapse_with_margin(margins.bottom);
        return;
    };
    if open.starts_paragraph && !open.has_clearance {
        *flow.leading_block_margin = Some(open.top_set);
    }
    *flow.y = top + height + edges.bottom;
    *flow.pending = CollapsibleMarginSet::from_margin(margins.bottom);
}

#[cfg(test)]
mod tests;
