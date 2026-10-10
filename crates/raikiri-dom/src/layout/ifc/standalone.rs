//! Shape text that is not part of a paragraph (page margin boxes, generated
//! pseudo-element text, list markers) with the inline engine and the
//! document's fonts.
//!
//! Font selection and line breaking use a small fixed style (normal weight,
//! preserved spaces), with the caller's writing mode, orientation and direction.

use crate::Document;
use shodo::font::FontCollection;
use shodo::geometry::{Direction, PhysicalSize, WritingMode};
use shodo::limits::Limits;
use shodo::style::{
    FontFamily, GenericFamily, InlineStyle, LineHeight, LineOptions, ParagraphStyle, TextAlign,
    TextOrientation, TextWrapMode, WhiteSpaceCollapse,
};
use shodo::{AtomicSizes, LayoutContext, Line, Paragraph, RichText};
use std::sync::atomic::Ordering;

/// Width handed to the line breaker when the caller wants one line.
const UNBOUNDED_WIDTH: f32 = 1.0e7;

/// Style of a standalone run.
#[derive(Clone, Debug, PartialEq)]
pub struct StandaloneStyle {
    /// CSS family names in order. A generic keyword is matched
    /// case-insensitively; anything else is a named family.
    pub families: Vec<String>,
    /// Font size in px; must be finite and positive.
    pub font_size: f32,
    /// Block and inline flow of the text.
    pub writing_mode: WritingMode,
    /// Orientation of characters in vertical flow.
    pub text_orientation: TextOrientation,
    /// Base inline direction; upright vertical text uses the engine's LTR flow.
    pub direction: Direction,
}

impl Default for StandaloneStyle {
    fn default() -> Self {
        Self {
            families: Vec::new(),
            font_size: 16.0,
            writing_mode: WritingMode::HorizontalTb,
            text_orientation: TextOrientation::Mixed,
            direction: Direction::Ltr,
        }
    }
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
}

fn family_from_css(name: &str) -> FontFamily {
    let trimmed = name.trim().trim_matches(['"', '\'']);
    let generic = match trimmed.to_ascii_lowercase().as_str() {
        "serif" => Some(GenericFamily::Serif),
        "sans-serif" => Some(GenericFamily::SansSerif),
        "monospace" => Some(GenericFamily::Monospace),
        "cursive" => Some(GenericFamily::Cursive),
        "fantasy" => Some(GenericFamily::Fantasy),
        "system-ui" => Some(GenericFamily::SystemUi),
        _ => None,
    };
    match generic {
        // A quoted keyword names a family called "serif", not the generic one.
        Some(generic) if !name.trim().starts_with(['"', '\'']) => FontFamily::Generic(generic),
        _ => FontFamily::Named(trimmed.to_owned()),
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

/// `text` with the blanks (space, tab, U+00A0) removed from the end of every
/// line, which [`StandaloneText::width`] leaves out.
fn without_trailing_blanks(text: &str) -> String {
    text.split('\n')
        .map(|line| line.trim_end_matches([' ', '\t', '\u{a0}']))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Break `text` into lines with the document's engine and fonts.
/// A built paragraph of one standalone run, ready to break at any width.
struct Shaped {
    paragraph: Paragraph,
    // The layout context is per run: the document's shared one needs `&mut`.
    cx: LayoutContext,
}

impl Shaped {
    fn build(
        fonts: &FontCollection,
        limits: &Limits,
        text: &str,
        style: &StandaloneStyle,
    ) -> Option<Self> {
        let inline = InlineStyle {
            font_families: style
                .families
                .iter()
                .map(|name| family_from_css(name))
                .collect(),
            font_size: style.font_size,
            direction: style.direction,
            text_orientation: style.text_orientation,
            line_height: LineHeight::Normal,
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
        let paragraph = RichText::with_limits(&paragraph_style, limits)
            .push(text, &inline)
            .build(&mut cx, fonts)
            .ok()?;
        Some(Self { paragraph, cx })
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
            &AtomicSizes::EMPTY,
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
        self.shape_standalone(text, style, StandaloneWidth::Given(width), align)
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
        self.shape_standalone(text, style, StandaloneWidth::Fit, align)
    }

    fn shape_standalone(
        &self,
        text: &str,
        style: &StandaloneStyle,
        width: StandaloneWidth,
        align: StandaloneAlign,
    ) -> Option<StandaloneText> {
        if !self.standalone_text_eligible(text, style.font_size) {
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
        let mut shaped = Shaped::build(fonts, limits, text, style)?;
        // shodo keeps the trailing spaces that fit on the last line and before
        // a forced break inside `inline_size`; the width leaves them out. The
        // width and the alignment shift therefore use a copy without trailing
        // blanks.
        let stripped = without_trailing_blanks(text);
        let mut ink = if stripped == text || stripped.is_empty() {
            None
        } else {
            Some(Shaped::build(fonts, limits, &stripped, style)?)
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
