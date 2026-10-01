//! Shape text that is not part of a paragraph (page margin boxes, generated
//! pseudo-element text, list markers) with the inline engine and the
//! document's fonts.
//!
//! The callers know only a font list, a size and an alignment, so the style
//! is a small fixed one: weight 400, normal, line height `normal`, spaces
//! preserved, no spacing or transform. The paragraph is left to right; a
//! right-to-left run inside it is reordered by the engine, and its glyph
//! origins stay inside the line.

use crate::Document;
use shodo::font::FontCollection;
use shodo::limits::Limits;
use shodo::style::{
    FontFamily, GenericFamily, InlineStyle, LineHeight, LineOptions, ParagraphStyle, TextAlign,
    TextWrapMode, WhiteSpaceCollapse,
};
use shodo::{AtomicSizes, LayoutContext, Line, RichText};
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
}

/// Horizontal alignment inside the given width.
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
    /// Centred in the width.
    Center,
    /// Justified; the last line is aligned to the start.
    Justify,
}

/// Lines of one shaped run.
pub struct StandaloneText {
    lines: Vec<Line>,
    /// Per line, how far its content moves right when its trailing blanks
    /// hang instead of being aligned.
    hang_shifts: Vec<f32>,
    width: f32,
    advance: f32,
    height: f32,
}

impl StandaloneText {
    /// The lines in order; glyph origins are relative to the line, and
    /// `Line::block_offset` places each line below the first.
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

    /// Widest line without its trailing blanks (space, tab, U+00A0).
    pub fn width(&self) -> f32 {
        self.width
    }

    /// First line including hanging trailing spaces.
    pub fn advance(&self) -> f32 {
        self.advance
    }

    /// Sum of the line advances.
    pub fn height(&self) -> f32 {
        self.height
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
fn shape_lines(
    fonts: &FontCollection,
    limits: &Limits,
    text: &str,
    style: &StandaloneStyle,
    width: Option<f32>,
    align: StandaloneAlign,
) -> Option<Vec<Line>> {
    let inline = InlineStyle {
        font_families: style
            .families
            .iter()
            .map(|name| family_from_css(name))
            .collect(),
        font_size: style.font_size,
        line_height: LineHeight::Normal,
        white_space_collapse: WhiteSpaceCollapse::Preserve,
        text_wrap_mode: TextWrapMode::Wrap,
        ..InlineStyle::default()
    };
    let paragraph_style = ParagraphStyle {
        root: inline.clone(),
        ..ParagraphStyle::default()
    };
    // The layout context is per call: the document's shared one needs
    // `&mut`.
    let mut cx = LayoutContext::new();
    let paragraph = RichText::with_limits(&paragraph_style, limits)
        .push(text, &inline)
        .build(&mut cx, fonts)
        .ok()?;
    let options = LineOptions {
        text_align: align_of(align),
        ..LineOptions::default()
    };
    Some(paragraph.break_all(
        &mut cx,
        &options,
        width.unwrap_or(UNBOUNDED_WIDTH),
        &AtomicSizes::EMPTY,
    ))
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
        let lines = shape_lines(fonts, limits, text, style, width, align)?;
        // shodo keeps the trailing spaces that fit on the last line and before
        // a forced break inside `inline_size`; the width leaves them out. The
        // width and the alignment shift therefore use a copy without trailing
        // blanks.
        let stripped = without_trailing_blanks(text);
        let ink_lines = if stripped == text {
            None
        } else if stripped.is_empty() {
            Some(Vec::new())
        } else {
            Some(shape_lines(fonts, limits, &stripped, style, width, align)?)
        };
        let line_width = widest(ink_lines.as_deref().unwrap_or(&lines));
        // Start-like alignments put the content at the same place with or
        // without its trailing blanks; the copy's lines pair with the text's
        // lines by index (stripping only shortens line ends).
        let factor = match align {
            StandaloneAlign::End | StandaloneAlign::Right => 1.0,
            StandaloneAlign::Center => 0.5,
            StandaloneAlign::Start | StandaloneAlign::Left | StandaloneAlign::Justify => 0.0,
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
        let height = lines.iter().map(Line::block_size).sum();
        if let Some(state) = &self.ifc {
            state.standalone_calls.fetch_add(1, Ordering::Relaxed);
        }
        Some(StandaloneText {
            lines,
            hang_shifts,
            width: line_width.max(0.0),
            advance: advance.max(0.0),
            height,
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
