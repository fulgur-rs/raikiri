//! Shape text that is not part of a paragraph (page margin boxes, generated
//! pseudo-element text, list markers) with the inline engine and the
//! document's fonts.
//!
//! Font selection and line breaking use a small fixed style (normal weight,
//! preserved spaces), with the caller's writing mode, orientation and direction.

use crate::Document;
use crate::layout::ifc::style::map_font_family;
use raikiri_style::FontFamilyName;
use raikiri_traits::PaintRect;
use shodo::font::FontCollection;
use shodo::geometry::{Direction, PhysicalConverter, PhysicalSize, WritingMode};
use shodo::limits::Limits;
use shodo::node::{InlineEdges, NodeId, TextSource};
use shodo::style::{
    InlineStyle, LineHeight, LineOptions, ParagraphStyle, TextAlign, TextOrientation, TextWrapMode,
    WhiteSpaceCollapse,
};
use shodo::{
    AtomicSize, AtomicSizes, Fragment, LayoutContext, Line, Paragraph, ParagraphBuilder, RichText,
};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

/// Width handed to the line breaker when the caller wants one line.
const UNBOUNDED_WIDTH: f32 = 1.0e7;

/// Style of a standalone run.
#[derive(Clone, Debug, PartialEq)]
pub struct StandaloneStyle {
    /// The `font-family` list in order, as parsed: each entry keeps whether
    /// it is a generic keyword or a named family.
    pub families: Vec<FontFamilyName>,
    /// Font size in px; must be finite and positive.
    pub font_size: f32,
    /// Block and inline flow of the text.
    pub writing_mode: WritingMode,
    /// Orientation of characters in vertical flow.
    pub text_orientation: TextOrientation,
    /// Base inline direction; upright vertical text uses the engine's LTR flow.
    pub direction: Direction,
    /// Height of each line: `normal`, a length or a multiple of the font size.
    pub line_height: LineHeight,
}

impl Default for StandaloneStyle {
    fn default() -> Self {
        Self {
            families: Vec::new(),
            font_size: 16.0,
            writing_mode: WritingMode::HorizontalTb,
            text_orientation: TextOrientation::Mixed,
            direction: Direction::Ltr,
            line_height: LineHeight::Normal,
        }
    }
}

/// An image inside a standalone run, laid out as an atomic inline that sits on
/// the baseline of its line (CSS 2.1 §10.8.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StandaloneImage {
    /// Byte offset in the text of the place the image is inserted at. It must
    /// follow the offset of the image before it; one inside a character is
    /// moved back to the start of the character.
    pub offset: usize,
    /// Width of the image box in CSS px.
    pub width: f32,
    /// Height of the image box in CSS px.
    pub height: f32,
}

/// Alignment along the inline axis inside the given inline size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandaloneAlign {
    /// The inline-start edge (left in left-to-right text).
    Start,
    /// The inline-end edge.
    End,
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// Centred in the inline extent.
    Center,
    /// Justified; the last line is aligned to the start.
    Justify,
}

/// Lines of one shaped run.
#[derive(Debug)]
pub struct StandaloneText {
    lines: Vec<Line>,
    /// Per line, how far content moves along the logical inline axis when blanks
    /// hang instead of being aligned.
    hang_shifts: Vec<f32>,
    inline_size: f32,
    advance: f32,
    block_size: f32,
    writing_mode: WritingMode,
    container: PhysicalSize,
}

impl StandaloneText {
    /// The lines in order; glyph origins are line-local logical coordinates.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// How far the content of line `index` must move in the inline direction
    /// to be aligned without its trailing blanks.
    ///
    /// shodo keeps the trailing blanks that fit on the last line and before a
    /// forced break inside the aligned content; these blanks hang outside the
    /// aligned content instead, or a right-aligned or centred `"ab "` would
    /// sit (half) a space too far to the left.
    pub fn hang_shift(&self, index: usize) -> f32 {
        self.hang_shifts.get(index).copied().unwrap_or(0.0)
    }

    /// Longest line without trailing blanks (space, tab, U+00A0).
    pub fn inline_size(&self) -> f32 {
        self.inline_size
    }

    /// Sum of line block advances.
    pub fn block_size(&self) -> f32 {
        self.block_size
    }

    /// Physical content width, excluding inline trailing blanks.
    pub fn width(&self) -> f32 {
        if self.writing_mode.is_vertical() {
            self.block_size
        } else {
            self.inline_size
        }
    }

    /// Physical layout container, including the supplied inline constraint.
    pub fn container(&self) -> PhysicalSize {
        self.container
    }

    /// First line including hanging trailing spaces.
    pub fn advance(&self) -> f32 {
        self.advance
    }

    /// Physical content height, excluding inline trailing blanks.
    pub fn height(&self) -> f32 {
        if self.writing_mode.is_vertical() {
            self.inline_size
        } else {
            self.block_size
        }
    }

    /// The border box of each image of the run, as its index in the list the
    /// run was shaped with and its place relative to the origin of the run.
    /// An image that was not placed, because the run broke off before it, is
    /// left out.
    pub fn images(&self) -> Vec<(usize, PaintRect)> {
        let mut out = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            let converter =
                PhysicalConverter::new(line.writing_mode(), line.used_direction(), self.container);
            let shift = self.hang_shift(index);
            for fragment in line.fragments() {
                let Fragment::Atomic(atomic) = fragment else {
                    continue;
                };
                let mut logical = atomic.border_rect;
                logical.inline_start += shift;
                logical.block_start += line.block_offset();
                let rect = converter.rect(logical);
                out.push((
                    atomic.node.0 as usize,
                    PaintRect::new(rect.x, rect.y, rect.width, rect.height),
                ));
            }
        }
        out
    }
}

/// How many shaped runs a [`StandaloneMemo`] keeps.
const MEMO_CAPACITY: usize = 64;

/// The longest text, in bytes, a [`StandaloneMemo`] keeps a run of. The runs
/// it is for are headers and footers; a longer text that is new on every page
/// would pin as many copies of it and of its shaped lines as the memo holds
/// runs, which is the document's own memory many times over.
const MEMO_MAX_TEXT_BYTES: usize = 4096;

/// The longest `font-family` list a [`StandaloneMemo`] keeps a run of, since
/// each remembered run holds a copy of the list.
const MEMO_MAX_FAMILIES: usize = 32;

/// The most lines a [`StandaloneMemo`] keeps a run of: a text of up to
/// [`MEMO_MAX_TEXT_BYTES`] that is broken at every opportunity (a width probe
/// of a narrow box) has a line for almost every character.
const MEMO_MAX_LINES: usize = 32;

/// Whether a run of `text` in `style` is small enough to be remembered.
fn is_remembered(text: &str, style: &StandaloneStyle) -> bool {
    text.len() <= MEMO_MAX_TEXT_BYTES && style.families.len() <= MEMO_MAX_FAMILIES
}

/// Runs shaped by [`Document::shape_standalone_text_shared`], the least
/// recently used first.
///
/// The text that repeats on every page (a running header) is looked up again
/// on each page, so it stays at the recent end, while text that is new on
/// each page (a page number) is the first to be evicted.
#[derive(Default)]
pub(crate) struct StandaloneMemo {
    state: Mutex<MemoState>,
}

#[derive(Default)]
struct MemoState {
    /// The font generations every entry was shaped under.
    generations: Option<(u64, Option<u64>)>,
    entries: Vec<MemoEntry>,
}

struct MemoEntry {
    text: String,
    style: StandaloneStyle,
    // The bits stand in for the `f32`, which has no equality that holds for
    // every value.
    width: Option<u32>,
    align: StandaloneAlign,
    shaped: Arc<StandaloneText>,
}

impl MemoEntry {
    fn matches(
        &self,
        text: &str,
        style: &StandaloneStyle,
        width: Option<f32>,
        align: StandaloneAlign,
    ) -> bool {
        self.align == align
            && self.width == width.map(f32::to_bits)
            && self.text == text
            && self.style == *style
    }
}

impl StandaloneMemo {
    /// The state for the fonts at `generations`: runs shaped under other
    /// fonts are dropped first.
    fn state(&self, generations: (u64, Option<u64>)) -> std::sync::MutexGuard<'_, MemoState> {
        // An entry is complete before it is pushed, so a panic elsewhere
        // leaves the list usable.
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.generations != Some(generations) {
            state.entries.clear();
            state.generations = Some(generations);
        }
        state
    }

    fn get(
        &self,
        generations: (u64, Option<u64>),
        text: &str,
        style: &StandaloneStyle,
        width: Option<f32>,
        align: StandaloneAlign,
    ) -> Option<Arc<StandaloneText>> {
        let mut state = self.state(generations);
        let index = state
            .entries
            .iter()
            .rposition(|entry| entry.matches(text, style, width, align))?;
        let entry = state.entries.remove(index);
        let shaped = Arc::clone(&entry.shaped);
        state.entries.push(entry);
        Some(shaped)
    }

    fn insert(
        &self,
        generations: (u64, Option<u64>),
        text: &str,
        style: &StandaloneStyle,
        width: Option<f32>,
        align: StandaloneAlign,
        shaped: &Arc<StandaloneText>,
    ) {
        let mut state = self.state(generations);
        // Another thread may have shaped the same run in the meantime.
        state
            .entries
            .retain(|entry| !entry.matches(text, style, width, align));
        if state.entries.len() >= MEMO_CAPACITY {
            state.entries.remove(0);
        }
        state.entries.push(MemoEntry {
            text: text.to_owned(),
            style: style.clone(),
            width: width.map(f32::to_bits),
            align,
            shaped: Arc::clone(shaped),
        });
    }
}

fn align_of(align: StandaloneAlign) -> TextAlign {
    match align {
        StandaloneAlign::Start => TextAlign::Start,
        StandaloneAlign::End => TextAlign::End,
        StandaloneAlign::Left => TextAlign::Left,
        StandaloneAlign::Right => TextAlign::Right,
        StandaloneAlign::Center => TextAlign::Center,
        StandaloneAlign::Justify => TextAlign::Justify,
    }
}

/// Where each of `images` is inserted into `text`: the offset kept at or after
/// the image before it, inside the text, and at the start of a character.
fn image_offsets(text: &str, images: &[StandaloneImage]) -> Vec<usize> {
    let mut start = 0;
    images
        .iter()
        .map(|image| {
            let mut at = image.offset.clamp(start, text.len());
            while !text.is_char_boundary(at) {
                at -= 1;
            }
            start = at.max(start);
            start
        })
        .collect()
}

/// `text` with the blanks (space, tab, U+00A0) removed from the end of every
/// line, which [`StandaloneText::width`] leaves out, and `images` moved to the
/// same places in the shorter text. Blanks in front of an image are not at the
/// end of its line, so they stay.
fn without_trailing_blanks(
    text: &str,
    images: &[StandaloneImage],
) -> (String, Vec<StandaloneImage>) {
    let at = image_offsets(text, images);
    let mut stripped = String::with_capacity(text.len());
    let mut moved = images.to_vec();
    let mut next = 0;
    let mut line_start = 0;
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            stripped.push('\n');
        }
        let line_end = line_start + line.len();
        let out_start = stripped.len();
        // The images sit in order, so those on this line come next in the list.
        let on_line = at[next..]
            .iter()
            .take_while(|&&offset| offset <= line_end)
            .count();
        let kept_end = match on_line {
            0 => line_start,
            n => at[next + n - 1],
        };
        stripped.push_str(&text[line_start..kept_end]);
        stripped.push_str(text[kept_end..line_end].trim_end_matches([' ', '\t', '\u{a0}']));
        for slot in next..next + on_line {
            moved[slot].offset = out_start + (at[slot] - line_start);
        }
        next += on_line;
        line_start = line_end + 1;
    }
    (stripped, moved)
}

/// Break `text` into lines with the document's engine and fonts.
/// A built paragraph of one standalone run, ready to break at any width.
struct Shaped {
    paragraph: Paragraph,
    // The layout context is per run: the document's shared one needs `&mut`.
    cx: LayoutContext,
    /// The sizes of the images of the run, by their index in the image list.
    atomics: AtomicSizes,
}

impl Shaped {
    fn build(
        fonts: &FontCollection,
        limits: &Limits,
        text: &str,
        images: &[StandaloneImage],
        style: &StandaloneStyle,
    ) -> Option<Self> {
        let inline = InlineStyle {
            font_families: style.families.iter().filter_map(map_font_family).collect(),
            font_size: style.font_size,
            direction: style.direction,
            text_orientation: style.text_orientation,
            line_height: style.line_height,
            white_space_collapse: WhiteSpaceCollapse::Preserve,
            text_wrap_mode: TextWrapMode::Wrap,
            ..InlineStyle::default()
        };
        let paragraph_style = ParagraphStyle {
            root: inline.clone(),
            writing_mode: style.writing_mode,
            direction: style.direction,
            ..ParagraphStyle::default()
        };
        let mut cx = LayoutContext::new();
        if images.is_empty() {
            let paragraph = RichText::with_limits(&paragraph_style, limits)
                .push(text, &inline)
                .build(&mut cx, fonts)
                .ok()?;
            return Some(Self {
                paragraph,
                cx,
                atomics: AtomicSizes::new(),
            });
        }
        // Text between images goes in as spans of their own, numbered after
        // the images, whose ids are their positions in the list.
        let mut builder = ParagraphBuilder::new(&paragraph_style, limits);
        let mut atomics = AtomicSizes::new();
        let horizontal = style.writing_mode == WritingMode::HorizontalTb;
        let mut spans = images.len() as u64;
        let mut push_span = |builder: &mut ParagraphBuilder, start: usize, end: usize| {
            if end > start {
                let node = NodeId(spans);
                spans += 1;
                let offset = u32::try_from(start).unwrap_or(u32::MAX);
                builder
                    .open_inline(node, &inline, InlineEdges::default())
                    .push_text(TextSource::Dom { node, offset }, &text[start..end])
                    .close_inline();
            }
        };
        // A size that is not a length is no size at all.
        let usable = |length: f32| {
            if length.is_finite() {
                length.max(0.0)
            } else {
                0.0
            }
        };
        let mut start = 0;
        for (index, (image, at)) in images.iter().zip(image_offsets(text, images)).enumerate() {
            push_span(&mut builder, start, at);
            start = at;
            let node = NodeId(index as u64);
            builder.push_atomic(node, &inline, InlineEdges::default());
            // The baseline of an image is its bottom margin edge.
            let (width, height) = (usable(image.width), usable(image.height));
            atomics.insert(
                node,
                AtomicSize {
                    inline_size: if horizontal { width } else { height },
                    block_size: if horizontal { height } else { width },
                    ..AtomicSize::default()
                },
            );
        }
        push_span(&mut builder, start, text.len());
        let paragraph = builder.build(&mut cx, fonts).ok()?;
        Some(Self {
            paragraph,
            cx,
            atomics,
        })
    }

    fn lines(&mut self, width: Option<f32>, align: StandaloneAlign) -> Vec<Line> {
        let options = LineOptions {
            text_align: align_of(align),
            ..LineOptions::default()
        };
        self.paragraph.break_all(
            &mut self.cx,
            &options,
            width.unwrap_or(UNBOUNDED_WIDTH),
            &self.atomics,
        )
    }
}

/// The inline constraint of a standalone run.
#[derive(Clone, Copy)]
enum StandaloneWidth {
    /// A caller-supplied constraint, or none.
    Given(Option<f32>),
    /// The run's own unconstrained width, as if it were measured with no
    /// constraint first and then shaped again inside that width.
    Fit,
}

fn widest(lines: &[Line]) -> f32 {
    lines.iter().map(Line::inline_size).fold(0.0, f32::max)
}

impl Document {
    /// Whether [`Document::shape_standalone_text`] would take this text and
    /// size: the text is not empty and the size is usable. A shodo limit
    /// error is the one case this cannot foresee.
    #[doc(hidden)]
    pub fn standalone_text_eligible(&self, text: &str, font_size: f32) -> bool {
        !text.is_empty() && font_size.is_finite() && font_size > 0.0
    }

    /// Shape `text` with the inline engine and the document's fonts, or the
    /// installed fonts for a document that was never laid out.
    ///
    /// `width` constrains the logical inline size (physical height in vertical flow).
    /// `None` when the text is empty, the size is unusable (see
    /// [`Document::standalone_text_eligible`]), or a limit is exceeded: there
    /// is then nothing to draw.
    #[doc(hidden)]
    pub fn shape_standalone_text(
        &self,
        text: &str,
        style: &StandaloneStyle,
        width: Option<f32>,
        align: StandaloneAlign,
    ) -> Option<StandaloneText> {
        self.shape_standalone(text, &[], style, StandaloneWidth::Given(width), align)
    }

    /// [`Document::shape_standalone_text`] with `images` inserted into the
    /// text as atomic inlines that sit on the baseline of their line, which
    /// [`StandaloneText::images`] places. A run of images alone has no text.
    #[doc(hidden)]
    pub fn shape_standalone_text_with_images(
        &self,
        text: &str,
        images: &[StandaloneImage],
        style: &StandaloneStyle,
        width: Option<f32>,
        align: StandaloneAlign,
    ) -> Option<StandaloneText> {
        self.shape_standalone(text, images, style, StandaloneWidth::Given(width), align)
    }

    /// [`Document::shape_standalone_text`] for callers that shape the same
    /// run again and again, such as the margin boxes of every page: a run
    /// that was shaped recently with the same text, style, width and
    /// alignment is handed out again instead of being shaped a second time.
    ///
    /// The runs are remembered together with the document's fonts, so a
    /// document that gets another font collection shapes anew, and so does
    /// one whose collection changes under it (a font is registered, or a
    /// generic or fallback family list is set). A document that was never
    /// laid out has none to tie them to and shapes every time, and so does a
    /// text of more than 4 KiB, a list of more than 32 font families or a run
    /// of more than 32 lines.
    #[doc(hidden)]
    pub fn shape_standalone_text_shared(
        &self,
        text: &str,
        style: &StandaloneStyle,
        width: Option<f32>,
        align: StandaloneAlign,
    ) -> Option<Arc<StandaloneText>> {
        self.shape_standalone_text_shared_with_images(text, &[], style, width, align)
    }

    /// [`Document::shape_standalone_text_shared`] with `images` inserted as
    /// [`Document::shape_standalone_text_with_images`] does. A run with images
    /// is not remembered.
    #[doc(hidden)]
    pub fn shape_standalone_text_shared_with_images(
        &self,
        text: &str,
        images: &[StandaloneImage],
        style: &StandaloneStyle,
        width: Option<f32>,
        align: StandaloneAlign,
    ) -> Option<Arc<StandaloneText>> {
        let remembered = self
            .ifc
            .as_ref()
            .filter(|_| images.is_empty() && is_remembered(text, style));
        let Some(state) = remembered else {
            return self
                .shape_standalone_text_with_images(text, images, style, width, align)
                .map(Arc::new);
        };
        // Read before shaping: a change of the fonts while this run is shaped
        // then drops the run again at the next look-up.
        let generations = state.fonts.generations();
        if let Some(shaped) = state
            .standalone_memo
            .get(generations, text, style, width, align)
        {
            return Some(shaped);
        }
        let shaped = Arc::new(self.shape_standalone_text(text, style, width, align)?);
        if shaped.lines().len() <= MEMO_MAX_LINES {
            state
                .standalone_memo
                .insert(generations, text, style, width, align, &shaped);
        }
        Some(shaped)
    }

    /// Shape `text` inside its own unconstrained width.
    ///
    /// The result is the one [`Document::shape_standalone_text`] gives with
    /// `width` set to the [`StandaloneText::width`] of an unconstrained run,
    /// but the text is shaped once instead of twice.
    #[doc(hidden)]
    pub fn shape_standalone_text_fitted(
        &self,
        text: &str,
        style: &StandaloneStyle,
        align: StandaloneAlign,
    ) -> Option<StandaloneText> {
        self.shape_standalone(text, &[], style, StandaloneWidth::Fit, align)
    }

    fn shape_standalone(
        &self,
        text: &str,
        images: &[StandaloneImage],
        style: &StandaloneStyle,
        width: StandaloneWidth,
        align: StandaloneAlign,
    ) -> Option<StandaloneText> {
        // Images give a run something to lay out when there is no text.
        let images_alone = text.is_empty()
            && !images.is_empty()
            && style.font_size.is_finite()
            && style.font_size > 0.0;
        if !self.standalone_text_eligible(text, style.font_size) && !images_alone {
            return None;
        }
        let installed;
        let (fonts, limits) = match &self.ifc {
            Some(state) => (&state.fonts, &state.limits),
            None => {
                installed = (crate::fonts::system_font_collection(), Limits::default());
                (&installed.0, &installed.1)
            }
        };
        let mut shaped = Shaped::build(fonts, limits, text, images, style)?;
        // shodo keeps the trailing spaces that fit on the last line and before
        // a forced break inside `inline_size`; the width leaves them out. The
        // width and the alignment shift therefore use a copy without trailing
        // blanks, whose images stay at the same places in the text.
        let (stripped, ink_images) = without_trailing_blanks(text, images);
        let mut ink = if stripped == text || (stripped.is_empty() && ink_images.is_empty()) {
            None
        } else {
            Some(Shaped::build(fonts, limits, &stripped, &ink_images, style)?)
        };
        let width = match width {
            StandaloneWidth::Given(width) => width,
            StandaloneWidth::Fit => {
                let measured = match &mut ink {
                    Some(ink) => widest(&ink.lines(None, align)),
                    None if stripped == text => widest(&shaped.lines(None, align)),
                    None => 0.0,
                };
                let measured = measured.max(0.0);
                Some(if style.writing_mode.is_vertical() {
                    shaped.lines(None, align).iter().map(Line::block_size).sum()
                } else {
                    measured
                })
            }
        };
        let lines = shaped.lines(width, align);
        let ink_lines = if stripped == text {
            None
        } else {
            Some(
                ink.as_mut()
                    .map_or_else(Vec::new, |ink| ink.lines(width, align)),
            )
        };
        let line_width = widest(ink_lines.as_deref().unwrap_or(&lines));
        // Start-like alignments put the content at the same place with or
        // without its trailing blanks; the copy's lines pair with the text's
        // lines by index (stripping only shortens line ends).
        let rtl = lines
            .first()
            .is_some_and(|line| line.used_direction() == Direction::Rtl);
        let factor = match align {
            StandaloneAlign::End => 1.0,
            StandaloneAlign::Right if !rtl => 1.0,
            StandaloneAlign::Left if rtl => 1.0,
            StandaloneAlign::Center => 0.5,
            _ => 0.0,
        };
        let hang_shifts = match &ink_lines {
            Some(ink) if factor > 0.0 => lines
                .iter()
                .zip(ink.iter().map(Some).chain(std::iter::repeat(None)))
                .map(|(line, ink)| {
                    let ink_size = ink.map_or(0.0, Line::inline_size);
                    (factor * (line.inline_size() - ink_size)).max(0.0)
                })
                .collect(),
            _ => Vec::new(),
        };
        // `inline_size + hang_end` does not change when shodo retains a space.
        let advance = lines
            .first()
            .map_or(line_width, |line| line.inline_size() + line.hang_end());
        let block_size = lines.iter().map(Line::block_size).sum();
        let inline_extent = width.unwrap_or(line_width).max(0.0);
        let container = if style.writing_mode.is_vertical() {
            PhysicalSize {
                width: block_size,
                height: inline_extent,
            }
        } else {
            PhysicalSize {
                width: inline_extent,
                height: block_size,
            }
        };
        if let Some(state) = &self.ifc {
            state.standalone_calls.fetch_add(1, Ordering::Relaxed);
        }
        Some(StandaloneText {
            lines,
            hang_shifts,
            inline_size: line_width.max(0.0),
            advance: advance.max(0.0),
            block_size,
            writing_mode: style.writing_mode,
            container,
        })
    }

    /// How many times [`Document::shape_standalone_text`] produced a result.
    #[doc(hidden)]
    pub fn standalone_text_calls(&self) -> usize {
        self.ifc
            .as_ref()
            .map_or(0, |state| state.standalone_calls.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests;
