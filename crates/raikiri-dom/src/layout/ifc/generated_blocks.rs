//! Block-level `::before` and `::after` boxes inside a paragraph.
//!
//! A generated box with a block-level `display` is a block child of its
//! element (CSS 2.1 12.1, CSS Display 3 §2). Its content stays in the
//! element's paragraph, so the box is marked there by two block-in-inline
//! tokens, one at its start and one at its end: the line loop places the box's
//! top margin, border and padding at the start token, and its height, bottom
//! padding, border and margin at the end token. The lines between the tokens
//! are the box's lines, set in by its inline-side edges.

use super::inline_boxes::{BoxRect, GeneratedBlockPiece};
use crate::layout::dispatch::block_flow::context::{BlockContext, BoxProbe, BoxSlot};
use crate::layout::protocol::CollapsibleMarginSet;
use raikiri_style::property::{
    BoxSizing, BreakBetween, ClearValue, ColumnCountValue, Contain, DisplayValue, OverflowValue,
    Sides,
};
use raikiri_style::{
    ComputedColumnWidth, ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrCalc,
    ComputedTextIndent, ComputedValues,
};
use shodo::style::LineOptions;
use taffy::Direction;

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

/// Resolve the `start` and `end` alignments of `options` against a box whose
/// direction is right-to-left when `rtl`, for lines that keep the opposite
/// direction of the paragraph around it. The paragraph's direction still
/// orders the lines' content and sets the side their indent is on.
pub(crate) fn align_physically(options: &mut LineOptions, rtl: bool) {
    use shodo::style::{TextAlign, TextAlignLast};
    let (start, end) = if rtl {
        (TextAlign::Right, TextAlign::Left)
    } else {
        (TextAlign::Left, TextAlign::Right)
    };
    let (last_start, last_end) = if rtl {
        (TextAlignLast::Right, TextAlignLast::Left)
    } else {
        (TextAlignLast::Left, TextAlignLast::Right)
    };
    // An `auto` last line of justified text is aligned to the start
    // (CSS Text 3 §6.2).
    if options.text_align_last == TextAlignLast::Auto && options.text_align == TextAlign::Justify {
        options.text_align_last = last_start;
    }
    match options.text_align {
        TextAlign::Start => options.text_align = start,
        TextAlign::End => options.text_align = end,
        _ => {}
    }
    match options.text_align_last {
        TextAlignLast::Start => options.text_align_last = last_start,
        TextAlignLast::End => options.text_align_last = last_end,
        _ => {}
    }
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
    /// Whether the box establishes an independent formatting context, which
    /// keeps margins from collapsing through it.
    formatting_root: bool,
    /// The box's own line options and raw `text-indent`, from its computed
    /// style.
    own_lines: (LineOptions, ComputedTextIndent),
    break_before: BreakBetween,
    break_after: BreakBetween,
}

impl GeneratedBlock {
    /// The geometry of the generated box styled by `cv`, marked by the tokens
    /// `start` and `end`, whose lines are set by the line options and raw
    /// `text-indent` in `lines`; `orthogonal` says its writing mode is
    /// orthogonal to its box parent's.
    pub(crate) fn new(
        start: u64,
        end: u64,
        cv: &ComputedValues,
        lines: (LineOptions, ComputedTextIndent),
        orthogonal: bool,
    ) -> Self {
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
            formatting_root: orthogonal || establishes_formatting_context(cv),
            own_lines: lines,
            break_before: cv.break_before,
            break_after: cv.break_after,
        }
    }

    /// The forced page breaks the box asks for among the lines of its
    /// paragraph, once its end token was met: one before the box when lines
    /// come before it, one after it. `open` is the state its start token
    /// left, `placed` the box if it has one, and `lines` the number of lines
    /// laid out so far.
    pub(crate) fn forced_breaks(
        &self,
        open: &OpenBlock,
        placed: Option<&PlacedBlock>,
        lines: usize,
    ) -> impl Iterator<Item = ForcedBreak> {
        let forced = |value| matches!(value, BreakBetween::Page | BreakBetween::Always);
        let before = (forced(self.break_before) && open.lines_before > 0).then(|| ForcedBreak {
            line: open.lines_before,
            value: self.break_before,
            box_top: placed.map(|placed| placed.border_box.y),
            gap: open.top_set.resolve().max(0.0),
        });
        let after = forced(self.break_after).then_some(ForcedBreak {
            line: lines,
            value: self.break_after,
            box_top: None,
            gap: 0.0,
        });
        before.into_iter().chain(after)
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
        let sizes = [self.height, self.min_height, self.max_height];
        self.used_size(sizes, content, edges, definite_height)
    }

    /// The content-box size along one axis, from that axis's `[size, min,
    /// max]` properties: see [`Self::used_height`].
    fn used_size(
        &self,
        [size, min_size, max_size]: [ComputedLengthPercentageOrAuto; 3],
        content: f32,
        edges: f32,
        definite: Option<f32>,
    ) -> f32 {
        let content_box = |value| {
            length_or_auto(value, definite).map(|size: f32| {
                if self.border_box {
                    (size - edges).max(0.0)
                } else {
                    size
                }
            })
        };
        let mut used = content_box(size).unwrap_or(content);
        if let Some(max) = content_box(max_size) {
            used = used.min(max);
        }
        if let Some(min) = content_box(min_size) {
            used = used.max(min);
        }
        used.max(0.0)
    }

    /// Whether the box keeps margins from collapsing through it even with no
    /// content (CSS 2.1 8.3.1): it establishes an independent formatting
    /// context, has block-axis padding or border, or has a used block size
    /// above zero. `inline_size` is the containing block's inline size, and
    /// `vertical` says the block axis is horizontal, so that the box's left
    /// and right sides and its width are on it.
    pub(crate) fn separates_margins(&self, inline_size: f32, vertical: bool) -> bool {
        let edges = self.edges(inline_size);
        let (block_edges, sizes) = if vertical {
            (
                edges.left + edges.right,
                [self.width, self.min_width, self.max_width],
            )
        } else {
            (
                edges.top + edges.bottom,
                [self.height, self.min_height, self.max_height],
            )
        };
        self.formatting_root
            || block_edges > 0.0
            || self.used_size(sizes, 0.0, block_edges, None) > 0.0
    }

    /// The line options of the box's own lines, with the `text-indent`
    /// resolved against its content width `content` (CSS Text 3 §8.1).
    fn own_line_options(&self, content: f32) -> LineOptions {
        let (mut options, indent) = self.own_lines;
        options.text_indent.length = super::flow::resolve_indent(indent, content);
        options
    }

    /// The box as `open` leaves it when its border box is `width` wide and it
    /// avoids the floats, before any of its lines is laid out: the state its
    /// lines are measured in.
    fn measured(&self, flow: &Flow<'_, '_>, edges: &Sides<f32>, width: f32) -> OpenBlock {
        let content = (width - edges.left - edges.right).max(0.0);
        OpenBlock {
            lines_before: 0,
            content_top: None,
            flow_top: 0.0,
            top_set: CollapsibleMarginSet::ZERO,
            starts_paragraph: false,
            has_clearance: false,
            consumed: 0.0,
            inline: (edges.left, flow.width - width + edges.right),
            rtl: flow.rtl,
            own_options: self.own_line_options(content),
            continues: false,
            avoids_floats: true,
        }
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

    /// Where the box goes among the floats already placed in `flow`, as a block
    /// child that establishes a formatting context goes (CSS 2.1 9.5): its
    /// border box does not overlap them, so it is set beside them where it
    /// fits and below them where it does not. `min_top` is the highest
    /// border-box top it may take, from the paragraph's content-box top;
    /// `margins` and `edges` are the box's margins and padding plus border.
    /// `None` when no float reaches that far down. `lines_height` measures
    /// the lines of a box with an `auto` height, as they are not laid out yet.
    fn beside_floats(
        &self,
        flow: &Flow<'_, '_>,
        margins: &Sides<f32>,
        edges: &Sides<f32>,
        min_top: f32,
        lines_height: LinesHeight<'_>,
    ) -> Option<Beside> {
        let from = flow.top_edge + min_top;
        if !flow.ctx.has_active_floats(from) {
            return None;
        }
        let direction = if flow.rtl {
            Direction::Rtl
        } else {
            Direction::Ltr
        };
        let sides = edges.left + edges.right;
        let definite = self.content_width(Some(flow.width), edges);
        // The border-box width in a slot that leaves `stretch` for an `auto`
        // width.
        let border_width = |stretch: f32| {
            definite.unwrap_or_else(|| {
                self.clamp_width((stretch - sides).max(0.0), Some(flow.width), edges)
            }) + sides
        };
        let stretched = |stretch: f32| stretch.max(-(margins.left + margins.right));
        // The box must keep clear of every float its border box reaches. A
        // height of its own is known before the lines are laid out; an `auto`
        // height follows the lines the box has at that width; lines that
        // cannot be measured on their own leave only its minimum height.
        let own_height = length_or_auto(self.height, flow.definite_height).is_some();
        let mut height = |width: f32| {
            let content = if own_height {
                0.0
            } else {
                lines_height(&self.measured(flow, edges, width)).unwrap_or(0.0)
            };
            self.used_height(content, edges.top + edges.bottom, flow.definite_height)
                + edges.top
                + edges.bottom
        };
        let BoxSlot { slot, width } = flow.ctx.find_box_slot(
            from,
            [margins.left, margins.right],
            direction,
            self.clear,
            |probe| match probe {
                BoxProbe::Width { stretch } => border_width(stretched(stretch)),
                BoxProbe::Height { width } => height(width),
            },
        );
        let stretch = stretched(slot.stretch_width);
        // `auto` side margins share the room the box leaves; the slot already
        // holds the margins that are not `auto`.
        let auto = |value| length_or_auto(value, Some(flow.width)).is_none();
        let autos = u8::from(auto(self.margin.left)) + u8::from(auto(self.margin.right));
        let free = (stretch - width).max(0.0);
        let mut left = slot.x - flow.content_left;
        if auto(self.margin.left) {
            left += free / f32::from(autos);
        } else if flow.rtl && autos == 0 {
            // A narrow box in a right-to-left paragraph sits at the end side.
            left += (slot.border_width - width).max(0.0);
        }
        Some(Beside {
            left,
            width,
            top: slot.y - flow.top_edge,
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
/// that is not finite. A `calc()` with no percentage needs no basis.
pub(super) fn length_or_auto(
    value: ComputedLengthPercentageOrAuto,
    basis: Option<f32>,
) -> Option<f32> {
    match (value, basis) {
        (ComputedLengthPercentageOrAuto::Px(_), None) => {
            crate::layout::page::used_computed_length_percentage_or_auto(value, 0.0)
        }
        (ComputedLengthPercentageOrAuto::Calc(calc), None) if calc.percent == 0.0 => {
            crate::layout::page::used_computed_length_percentage_or_auto(value, 0.0)
        }
        (_, Some(basis)) => {
            crate::layout::page::used_computed_length_percentage_or_auto(value, basis)
        }
        _ => None,
    }
}

/// The border box of a block-level `::before` or `::after` that establishes a
/// formatting context, set among the floats before it: see
/// [`GeneratedBlock::beside_floats`].
struct Beside {
    /// Left edge from the paragraph's content box.
    left: f32,
    width: f32,
    /// Top from the paragraph's content-box top.
    top: f32,
}

/// A forced page break that a block-level `::before` or `::after` asks for
/// with `break-before` or `break-after`, between the lines of its paragraph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ForcedBreak {
    /// The first line after the break. A break after the box is recorded
    /// whether or not a line follows it.
    pub(crate) line: usize,
    /// The `break-before` or `break-after` value that asks for the break.
    pub(crate) value: BreakBetween,
    /// The block offset of the border-box top of the box that a break before
    /// it moves, when the box was placed; the break is then before the box,
    /// not before its first line. From the paragraph's content-box top.
    pub(crate) box_top: Option<f32>,
    /// The margin that stays above the content after the break, at the top of
    /// the next page: the collapsed top margin of a box that the break is
    /// before. Lines after a box start at the top of the page.
    pub(crate) gap: f32,
}

/// The box of a block-level `::before` or `::after` as the line loop placed
/// it, in the paragraph's content box, before pagination moves any line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlacedBlock {
    /// The inline engine's id of the pseudo-element.
    pub(crate) node: u64,
    /// Border box.
    pub(crate) border_box: BoxRect,
    /// Content box: the border box less its border and padding.
    pub(crate) content_box: BoxRect,
    /// The first of the box's lines.
    pub(crate) first_line: usize,
    /// The line after the box's last one.
    pub(crate) end_line: usize,
}

impl PlacedBlock {
    /// The box cut where pagination moves its lines by a different amount
    /// than the line before them. `lines` are the paragraph's lines;
    /// `shift_of` gives how far pagination moved a line. The top of the box
    /// moves with its first line, and a box without lines with the line
    /// before it, so it stays with the content above it.
    pub(crate) fn pieces(
        &self,
        lines: &[shodo::Line],
        shift_of: impl Fn(usize) -> f32,
    ) -> Vec<GeneratedBlockPiece> {
        let end_line = self.end_line.min(lines.len());
        let anchor = if self.first_line < end_line {
            self.first_line
        } else {
            self.first_line.saturating_sub(1)
        };
        let bottom = self.border_box.y + self.border_box.height;
        let mut cuts = Vec::new();
        let mut start = (self.border_box.y, shift_of(anchor), anchor);
        // Lines that overflow below the box cut nothing of it.
        for (index, line) in lines
            .iter()
            .enumerate()
            .take(end_line)
            .skip(anchor + 1)
            .take_while(|(_, line)| line.block_offset() < bottom)
        {
            let shift = shift_of(index);
            if shift != start.1 {
                cuts.push((start, line.block_offset()));
                start = (line.block_offset(), shift, index);
            }
        }
        cuts.push((start, bottom));
        let count = cuts.len();
        cuts.into_iter()
            .enumerate()
            .map(|(index, ((top, shift, line), end))| {
                let moved = |rect: BoxRect| BoxRect {
                    y: rect.y + shift,
                    ..rect
                };
                let slice = |rect: BoxRect| {
                    let y0 = rect.y.max(top);
                    let y1 = (rect.y + rect.height).min(end);
                    moved(BoxRect {
                        y: y0,
                        height: (y1 - y0).max(0.0),
                        ..rect
                    })
                };
                GeneratedBlockPiece {
                    node: self.node as usize,
                    line,
                    border_box: slice(self.border_box),
                    content_box: slice(self.content_box),
                    joined_border_box: moved(self.border_box),
                    joined_content_box: moved(self.content_box),
                    shift,
                    has_block_start_edge: index == 0,
                    has_block_end_edge: index + 1 == count,
                }
            })
            .collect()
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
    /// Where the content box starts without padding, border or clearance:
    /// below the margins collapsed at its top. A float may push the box's
    /// first line further down, but not the box (CSS 2.1 9.5).
    flow_top: f32,
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
    /// The box's own line options, with its `text-indent` resolved against
    /// its content width.
    own_options: LineOptions,
    /// The box goes on from an earlier fragmentainer, where its first line
    /// was laid out.
    continues: bool,
    /// The box was set among the floats before it, which it does not overlap:
    /// its lines take its whole content width.
    avoids_floats: bool,
}

impl OpenBlock {
    /// The box after `lines` lines of the paragraph: once one of them is the
    /// box's, its first line is laid out.
    pub(crate) fn after(self, lines: usize) -> Self {
        Self {
            continues: self.continues || lines > self.lines_before,
            ..self
        }
    }

    /// The box as it goes on at the top of the next fragmentainer, after a
    /// fragmentainer that ends at `end`: the box fills it to there.
    pub(crate) fn continued(self, end: f32) -> Self {
        let top = self.content_top.unwrap_or(self.flow_top);
        Self {
            lines_before: 0,
            content_top: Some(0.0),
            starts_paragraph: false,
            has_clearance: false,
            consumed: self.consumed + (end - top).max(0.0),
            ..self
        }
    }

    /// The line options of the box's next line, after `lines` lines of the
    /// paragraph, and whether it is the box's first line.
    pub(crate) fn line_options(&self, lines: usize) -> (LineOptions, bool) {
        let mut options = self.own_options;
        let first = lines == self.lines_before && !self.continues;
        // The engine starts the line after the box's start token as one after
        // a forced break, not as a first line: its `text-indent` is the
        // box's first-line indent only when it applies to such lines too.
        if first {
            options.text_indent.each_line = true;
        }
        (options, first)
    }

    /// Whether the box was placed clear of the floats before it, so that no
    /// float narrows its lines or moves them down.
    pub(crate) fn avoids_floats(&self) -> bool {
        self.avoids_floats
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
        let float_start = if space.inline_start > 0.0 && !self.avoids_floats {
            space.inline_start
        } else {
            f32::NEG_INFINITY
        };
        let float_end = if space_end < width && !self.avoids_floats {
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
    /// Border plus padding on the left of the paragraph's content box.
    pub(crate) content_left: f32,
    /// Border plus padding above the paragraph's content box.
    pub(crate) top_edge: f32,
    pub(crate) definite_height: Option<f32>,
    /// Nothing is placed in the paragraph yet.
    pub(crate) at_start: bool,
    /// Adjoining margins leave the paragraph through its top edge.
    pub(crate) top_margin_escapes: bool,
    /// The paragraph's lines start at the right.
    pub(crate) rtl: bool,
}

/// Measures the block size of the lines of a box with an `auto` height, in the
/// state `open` would leave the box in at the width in question: `None` when
/// they cannot be laid out on their own.
pub(crate) type LinesHeight<'a> = &'a mut dyn FnMut(&OpenBlock) -> Option<f32>;

/// Place the top of `block` at its start token: its top margin with the
/// margins above it, its clearance, and its top border and padding.
/// `lines_height` measures the lines of a box that goes among floats.
pub(crate) fn open(
    block: &GeneratedBlock,
    flow: Flow<'_, '_>,
    lines_height: LinesHeight<'_>,
) -> OpenBlock {
    let margins = block.margins(flow.width);
    let edges = block.edges(flow.width);
    let top_set = flow.pending.collapse_with_margin(margins.top);
    // Clearance (CSS 2.1 9.5.2) puts the top border edge below the floats
    // it clears; the margins above no longer collapse with it.
    let clear_y = (block.clear != taffy::Clear::None)
        .then(|| flow.ctx.cleared_threshold(block.clear))
        .flatten()
        .map(|threshold| threshold - flow.top_edge);
    let hypothetical = if flow.at_start && flow.top_margin_escapes && flow.ctx.origin_resolved() {
        flow.ctx
            .hypothetical_top_position(top_set)
            .unwrap_or(*flow.y)
    } else {
        *flow.y + top_set.resolve()
    };
    let escapes = flow.at_start && flow.top_margin_escapes;
    let flow_top = if escapes { *flow.y } else { hypothetical };
    let clearance = clear_y
        .filter(|&clear_y| flow.ctx.has_adjoining_float(block.clear) || hypothetical < clear_y);
    // A box that establishes a formatting context keeps clear of the floats
    // before it: it goes beside them where it fits, else it is pushed below
    // them, which like clearance takes its margins out of the collapse. A
    // float adjoining the margins that escape above the box moves with them,
    // so the box is first tried beside that float without its margin.
    let adjoining = escapes && flow.ctx.has_adjoining_float(taffy::Clear::Both);
    let top_bound = clearance.unwrap_or(if adjoining { *flow.y } else { hypothetical });
    let beside = block
        .formatting_root
        .then(|| block.beside_floats(&flow, &margins, &edges, top_bound, lines_height))
        .flatten();
    let cleared = clearance.is_some();
    let clearance = beside
        .as_ref()
        .map(|beside| beside.top)
        .filter(|&top| top > top_bound + 0.001)
        .or(clearance);
    let content_top = if let Some(clear_y) = clearance {
        Some(clear_y + edges.top)
    } else if edges.top > 0.0 {
        Some(flow_top + edges.top)
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
    let inline = match &beside {
        Some(beside) => (
            beside.left + edges.left,
            flow.width - beside.left - beside.width + edges.right,
        ),
        None => block.inline_insets(flow.width, flow.rtl, &edges),
    };
    OpenBlock {
        lines_before: flow.lines.len(),
        content_top,
        flow_top,
        top_set,
        starts_paragraph: flow.at_start,
        // Margins that escape above the paragraph move the floats that adjoin
        // them, so whether they escape cannot depend on where those floats
        // are: the layout would not settle.
        has_clearance: if escapes {
            cleared
        } else {
            clearance.is_some()
        },
        consumed: 0.0,
        inline,
        rtl: flow.rtl,
        // A percentage `text-indent` refers to the box's own width.
        own_options: block.own_line_options((flow.width - inline.0 - inline.1).max(0.0)),
        continues: false,
        avoids_floats: beside.is_some(),
    }
}

/// Place the bottom of `block` at its end token: the box's height, from its
/// lines unless it has one of its own, and its bottom padding, border and
/// margin. `open` is the state its start token left. Returns the box as
/// placed, unless its margins collapse through it.
pub(crate) fn close(
    block: &GeneratedBlock,
    open: OpenBlock,
    flow: Flow<'_, '_>,
) -> Option<PlacedBlock> {
    let margins = block.margins(flow.width);
    let edges = block.edges(flow.width);
    let first_line = flow.lines.get(open.lines_before);
    let content_top = open
        .content_top
        .or_else(|| first_line.map(|_| open.flow_top));
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
        (height > 0.0 || edges.bottom > 0.0 || block.formatting_root).then(|| {
            if open.starts_paragraph && flow.top_margin_escapes {
                *flow.y
            } else {
                *flow.y + flow.pending.resolve()
            }
        })
    }) else {
        *flow.pending = flow.pending.collapse_with_margin(margins.bottom);
        if open.starts_paragraph {
            *flow.leading_block_margin = Some(*flow.pending);
        }
        return None;
    };
    if open.starts_paragraph && !open.has_clearance {
        *flow.leading_block_margin = Some(open.top_set);
    }
    *flow.y = top + height + edges.bottom;
    *flow.pending = CollapsibleMarginSet::from_margin(margins.bottom);
    // The inline insets hold the margins with the edges inside them.
    let left = open.inline.0 - edges.left;
    let right = flow.width - open.inline.1 + edges.right;
    let border_box = BoxRect {
        x: left,
        y: top - edges.top,
        width: (right - left).max(0.0),
        height: edges.top + height + edges.bottom,
    };
    Some(PlacedBlock {
        node: block.start,
        border_box,
        content_box: BoxRect {
            x: left + edges.left,
            y: top,
            width: (right - left - edges.left - edges.right).max(0.0),
            height,
        },
        first_line: open.lines_before,
        end_line: flow.lines.len(),
    })
}

/// Whether a block-level box styled by `cv` establishes an independent
/// formatting context: any display other than a plain block or list item, a
/// scroll container, a multi-column container, or layout or paint
/// containment.
fn establishes_formatting_context(cv: &ComputedValues) -> bool {
    let visible = |overflow| matches!(overflow, OverflowValue::Visible | OverflowValue::Clip);
    !matches!(cv.display, DisplayValue::Block | DisplayValue::ListItem)
        || !visible(cv.overflow.x)
        || !visible(cv.overflow.y)
        || !matches!(cv.column_count, ColumnCountValue::Auto)
        || !matches!(cv.column_width, ComputedColumnWidth::Auto)
        || cv.contain.contains(Contain::LAYOUT)
        || cv.contain.contains(Contain::PAINT)
}

#[cfg(test)]
mod tests;
