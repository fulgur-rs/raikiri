//! Checked conversion of raikiri computed values into shodo text input.
//!
//! Every mapping is fail-closed: a value shodo cannot represent is an
//! [`IfcError::Unsupported`], never a silent default. Values that belong to
//! the box tree or to painting (size, position, color, background) are not
//! text input and are ignored here.

use super::ch::ch_advance;
use super::error::IfcError;
use raikiri_style::property::{self as p, FontFamilyKind, FontFamilyName};
use raikiri_style::{ChFontKey, ChLengthProvenance};
use raikiri_style::{
    ComputedLengthPercentage as Length, ComputedLengthPercentageOrAuto as LengthOrAuto,
    ComputedLetterSpacing, ComputedLineHeight, ComputedTabSize, ComputedTextIndent, ComputedValues,
};
use shodo::font::FontCollection;
use shodo::geometry::WritingMode;
use shodo::node::{InlineEdges, Sides};
use shodo::style::{
    self as s, FontFamily, InlineStyle, LineHeight, LineOptions, ParagraphStyle,
    TextCombineUpright, TextOrientation, TextWrapMode, WhiteSpaceCollapse,
};

macro_rules! same_enum {
    ($name:ident, $value:expr, $node:expr; $($variant:ident),+ $(,)?) => {
        match $value {
            $(p::$name::$variant => Ok(s::$name::$variant),)+
            _ => Err(IfcError::Unsupported {
                node: $node,
                reason: concat!(stringify!($name), " value is not represented by shodo"),
            }),
        }
    };
}

pub(crate) fn map_text_orientation(
    value: p::TextOrientation,
) -> Result<TextOrientation, &'static str> {
    match value {
        p::TextOrientation::Mixed => Ok(TextOrientation::Mixed),
        p::TextOrientation::Upright => Ok(TextOrientation::Upright),
        p::TextOrientation::Sideways => Ok(TextOrientation::Sideways),
        _ => Err("text-orientation is not represented by shodo"),
    }
}

pub(crate) fn map_text_combine_upright(
    value: p::TextCombineUpright,
) -> Result<TextCombineUpright, &'static str> {
    match value {
        p::TextCombineUpright::None => Ok(TextCombineUpright::None),
        p::TextCombineUpright::All => Ok(TextCombineUpright::All),
        _ => Err("text-combine-upright is not represented by shodo"),
    }
}

/// The paint offset of a `position: relative` box whose insets are all
/// lengths: `left` over `-right`, `top` over `-bottom` (CSS 2.1 9.4.3). `None`
/// when an inset is a percentage or `calc()`, or the box has a `z-index`
/// (which makes a stacking context); the projection refuses such a box. A
/// box that is not relative has no offset.
#[doc(hidden)]
pub fn relative_offset(cv: &ComputedValues) -> Option<(f32, f32)> {
    if cv.position != p::PositionValue::Relative {
        return Some((0.0, 0.0));
    }
    if cv.z_index != p::ZIndexValue::Auto {
        return None;
    }
    // `Ok(Some(px))` for a length, `Ok(None)` for `auto`, `Err` for a
    // percentage or calc().
    let inset = |value: LengthOrAuto| match value {
        LengthOrAuto::Auto => Ok(None),
        LengthOrAuto::Px(px) => Ok(Some(px)),
        _ => Err(()),
    };
    let (left, right, top, bottom) = (
        inset(cv.left).ok()?,
        inset(cv.right).ok()?,
        inset(cv.top).ok()?,
        inset(cv.bottom).ok()?,
    );
    let dx = left.or(right.map(|r| -r)).unwrap_or(0.0);
    let dy = top.or(bottom.map(|b| -b)).unwrap_or(0.0);
    Some((dx, dy))
}

/// The paint offset of a `position: relative` inline element of a paragraph
/// laid out by the inline engine, as [`relative_offset`] gives it, with what
/// the lines do not model degraded: an inset in a percentage or calc() is
/// taken as zero (the root's width is not known when the paragraph is
/// projected), and a `z-index` is drawn in the paragraph's order instead of
/// as a stacking context of its own.
pub(crate) fn relative_offset_in_lines(cv: &ComputedValues) -> (f32, f32) {
    if cv.position != p::PositionValue::Relative {
        return (0.0, 0.0);
    }
    let inset = |value: LengthOrAuto| match value {
        LengthOrAuto::Px(px) => Some(px),
        LengthOrAuto::Auto => None,
        _ => Some(0.0),
    };
    let dx = inset(cv.left)
        .or(inset(cv.right).map(|r| -r))
        .unwrap_or(0.0);
    let dy = inset(cv.top)
        .or(inset(cv.bottom).map(|b| -b))
        .unwrap_or(0.0);
    (dx, dy)
}

/// The key of this element's own font, which measures a `ch` value declared
/// on it.
fn own_ch_font(cv: &ComputedValues) -> ChFontKey {
    ChFontKey {
        family: cv.font_family.clone(),
        size: cv.font_size,
        weight: cv.font_weight,
        style: cv.font_style,
    }
}

/// The `font-family` list as shodo families. A generic family shodo does not
/// name is an error.
pub(crate) fn map_font_families(names: &[FontFamilyName]) -> Result<Vec<FontFamily>, &'static str> {
    names
        .iter()
        .map(|family| {
            if family.1 == FontFamilyKind::Named {
                Ok(FontFamily::Named(family.as_str().into()))
            } else {
                let generic = match family.as_str().to_ascii_lowercase().as_str() {
                    "serif" => s::GenericFamily::Serif,
                    "sans-serif" => s::GenericFamily::SansSerif,
                    "monospace" => s::GenericFamily::Monospace,
                    "cursive" => s::GenericFamily::Cursive,
                    "fantasy" => s::GenericFamily::Fantasy,
                    "system-ui" => s::GenericFamily::SystemUi,
                    // shodo names six generic families; the others take the
                    // nearest of those (CSS Fonts 4, 4.2: a UA may map a
                    // generic family it does not distinguish to another).
                    "ui-serif" | "math" | "fangsong" => s::GenericFamily::Serif,
                    "ui-sans-serif" | "ui-rounded" | "emoji" => s::GenericFamily::SansSerif,
                    "ui-monospace" => s::GenericFamily::Monospace,
                    _ => return Err("generic family is not represented by shodo"),
                };
                Ok(FontFamily::Generic(generic))
            }
        })
        .collect()
}

/// Text style of one element (or of the paragraph root).
///
/// `lang` is left unset; the caller fills it from the document.
pub(crate) fn inline_style(
    cv: &ComputedValues,
    node: usize,
    fonts: &FontCollection,
) -> Result<InlineStyle, IfcError> {
    let unsupported = |reason: &'static str| IfcError::Unsupported { node, reason };
    // Percentage or calc() letter spacing remains deferred, so project its
    // available absolute fallback for Shodo.
    let absolute_letter_spacing = |value: &ComputedLetterSpacing, fallback: f32| match value {
        ComputedLetterSpacing::Px(value) => Ok::<f32, IfcError>(*value),
        _ => Ok(fallback),
    };
    // A `ch` value is measured with the font of the element that declared it;
    // an inherited one carries that font's key, one declared here has none.
    let ch_length = |factor: f32, font: &Option<ChFontKey>, offset: f32| {
        let key = font.clone().unwrap_or_else(|| own_ch_font(cv));
        factor * ch_advance(fonts, &key) + offset
    };
    let letter_spacing = match cv.letter_spacing_ch_factor {
        Some(factor) => ch_length(
            factor,
            &cv.letter_spacing_ch_font,
            cv.letter_spacing_ch_offset,
        ),
        None => absolute_letter_spacing(&cv.letter_spacing_computed, cv.letter_spacing.px())?,
    };
    let (word_spacing_absolute, word_spacing_percent) = match cv.word_spacing_computed {
        ComputedLetterSpacing::Px(value) => (value, 0.0),
        ComputedLetterSpacing::Percent(percent) => (0.0, percent),
        ComputedLetterSpacing::Calc(value) => (value.px, value.percent),
    };
    let word_spacing_absolute = match cv.word_spacing_ch_factor {
        Some(factor) => ch_length(factor, &cv.word_spacing_ch_font, cv.word_spacing_ch_offset),
        None => word_spacing_absolute,
    };
    // CSS Text resolves word-spacing percentages against font-size; Shodo's
    // relative field uses the space advance, so resolve the percentage here.
    let word_spacing = word_spacing_absolute + cv.font_size.0 * word_spacing_percent / 100.0;
    let word_spacing_percent = 0.0;
    let line_height = match cv.line_height {
        ComputedLineHeight::Normal => LineHeight::Normal,
        ComputedLineHeight::Number(value) => LineHeight::Number(value),
        ComputedLineHeight::Length(value) => LineHeight::Px(value.0),
    };

    // The effective values already follow the winning declaration, whether it
    // was the legacy `white-space` keyword or a longhand.
    let white_space_collapse = match cv.effective_white_space_collapse {
        p::WhiteSpaceCollapse::Collapse => WhiteSpaceCollapse::Collapse,
        p::WhiteSpaceCollapse::Preserve => WhiteSpaceCollapse::Preserve,
        p::WhiteSpaceCollapse::PreserveBreaks => WhiteSpaceCollapse::PreserveBreaks,
        p::WhiteSpaceCollapse::PreserveSpaces => WhiteSpaceCollapse::PreserveSpaces,
        p::WhiteSpaceCollapse::BreakSpaces => WhiteSpaceCollapse::BreakSpaces,
        // `discard` (CSS Text 4) drops every white space character; shodo has
        // no such mode, so its white space is collapsed as for `collapse`.
        p::WhiteSpaceCollapse::Discard => WhiteSpaceCollapse::Collapse,
        _ => {
            return Err(unsupported(
                "white-space-collapse is not represented by shodo",
            ));
        }
    };
    let text_wrap_mode = match cv.effective_text_wrap_mode {
        p::TextWrapMode::Wrap => TextWrapMode::Wrap,
        p::TextWrapMode::Nowrap => TextWrapMode::NoWrap,
        _ => return Err(unsupported("text-wrap-mode is not represented by shodo")),
    };

    let font_style = match cv.font_style {
        p::FontStyle::Normal => s::FontStyle::Normal,
        p::FontStyle::Italic => s::FontStyle::Italic,
        p::FontStyle::Oblique => s::FontStyle::Oblique(14.0),
        _ => return Err(unsupported("font-style is not represented by shodo")),
    };
    let font_kerning = same_enum!(FontKerning, cv.font_kerning, node; Auto, Normal, None)?;
    let font_synthesis = s::FontSynthesis {
        weight: cv.font_synthesis.weight,
        style: match cv.font_synthesis.style {
            p::FontSynthesisStyle::None => false,
            p::FontSynthesisStyle::Auto => true,
            p::FontSynthesisStyle::ObliqueOnly => matches!(font_style, s::FontStyle::Oblique(_)),
            _ => return Err(unsupported("font-synthesis is not represented by shodo")),
        },
        small_caps: cv.font_synthesis.small_caps,
    };
    let font_variant_ligatures = match cv.font_variant_ligatures {
        p::FontVariantLigatures::Normal => s::FontVariantLigatures::default(),
        p::FontVariantLigatures::None => s::FontVariantLigatures {
            none: true,
            ..Default::default()
        },
        p::FontVariantLigatures::CommonLigatures => s::FontVariantLigatures {
            common: Some(true),
            ..Default::default()
        },
        p::FontVariantLigatures::NoCommonLigatures => s::FontVariantLigatures {
            common: Some(false),
            ..Default::default()
        },
        p::FontVariantLigatures::DiscretionaryLigatures => s::FontVariantLigatures {
            discretionary: Some(true),
            ..Default::default()
        },
        p::FontVariantLigatures::NoDiscretionaryLigatures => s::FontVariantLigatures {
            discretionary: Some(false),
            ..Default::default()
        },
        p::FontVariantLigatures::HistoricalLigatures => s::FontVariantLigatures {
            historical: Some(true),
            ..Default::default()
        },
        p::FontVariantLigatures::NoHistoricalLigatures => s::FontVariantLigatures {
            historical: Some(false),
            ..Default::default()
        },
        p::FontVariantLigatures::Contextual => s::FontVariantLigatures {
            contextual: Some(true),
            ..Default::default()
        },
        p::FontVariantLigatures::NoContextual => s::FontVariantLigatures {
            contextual: Some(false),
            ..Default::default()
        },
        _ => {
            return Err(unsupported(
                "font-variant-ligatures is not represented by shodo",
            ));
        }
    };
    let font_variant_caps = same_enum!(
        FontVariantCaps, cv.font_variant_caps, node;
        Normal, SmallCaps, AllSmallCaps, PetiteCaps, AllPetiteCaps, Unicase, TitlingCaps
    )?;
    let font_variant_position =
        same_enum!(FontVariantPosition, cv.font_variant_position, node; Normal, Sub, Super)?;
    let font_optical_sizing = match cv.font_optical_sizing {
        p::FontOpticalSizing::Auto => true,
        p::FontOpticalSizing::None => false,
        _ => {
            return Err(unsupported(
                "font-optical-sizing is not represented by shodo",
            ));
        }
    };
    let font_variant_numeric = s::FontVariantNumeric {
        lining_nums: cv.font_variant_numeric.lining_nums,
        oldstyle_nums: cv.font_variant_numeric.oldstyle_nums,
        proportional_nums: cv.font_variant_numeric.proportional_nums,
        tabular_nums: cv.font_variant_numeric.tabular_nums,
        diagonal_fractions: cv.font_variant_numeric.diagonal_fractions,
        stacked_fractions: cv.font_variant_numeric.stacked_fractions,
        ordinal: cv.font_variant_numeric.ordinal,
        slashed_zero: cv.font_variant_numeric.slashed_zero,
    };
    let font_variant_east_asian = s::FontVariantEastAsian {
        variant: cv
            .font_variant_east_asian
            .variant
            .map(|value| {
                same_enum!(
                    FontVariantEastAsianVariant, value, node;
                    Jis78, Jis83, Jis90, Jis04, Simplified, Traditional
                )
            })
            .transpose()?,
        width: cv
            .font_variant_east_asian
            .width
            .map(|value| {
                same_enum!(
                    FontVariantEastAsianWidth, value, node;
                    FullWidth, ProportionalWidth
                )
            })
            .transpose()?,
        ruby: cv.font_variant_east_asian.ruby,
    };
    let font_variations = match &cv.font_variation_settings {
        p::FontVariationSettings::Normal => Vec::new(),
        p::FontVariationSettings::Settings(values) => values
            .iter()
            .map(|value| {
                let tag = value
                    .tag
                    .as_bytes()
                    .try_into()
                    .map_err(|_| unsupported("font axis tag must have four bytes"))?;
                Ok(s::FontVariation {
                    tag,
                    value: value.value,
                })
            })
            .collect::<Result<Vec<_>, IfcError>>()?,
        _ => {
            return Err(unsupported(
                "font-variation-settings is not represented by shodo",
            ));
        }
    };
    let font_features = match &cv.font_feature_settings {
        p::FontFeatureSettings::Normal => Vec::new(),
        p::FontFeatureSettings::Features(values) => values
            .iter()
            .map(|value| s::FontFeature {
                tag: value.tag,
                value: value.value,
            })
            .collect(),
        _ => {
            return Err(unsupported(
                "font-feature-settings is not represented by shodo",
            ));
        }
    };
    // `math-auto` only changes single-letter MathML identifiers, which are
    // not laid out as paragraph text here: no transform.
    let text_transform = if cv.text_transform == p::TextTransform::MathAuto {
        Ok(s::TextTransform::None)
    } else {
        same_enum!(
            TextTransform, cv.text_transform, node;
            None, Capitalize, Uppercase, Lowercase, FullWidth, FullSizeKana,
            CapitalizeFullWidth, UppercaseFullWidth, LowercaseFullWidth,
            CapitalizeFullSizeKana, UppercaseFullSizeKana, LowercaseFullSizeKana,
            FullWidthFullSizeKana, CapitalizeFullWidthFullSizeKana,
            UppercaseFullWidthFullSizeKana, LowercaseFullWidthFullSizeKana
        )
    }?;
    let direction = match cv.direction {
        p::Direction::Ltr => shodo::geometry::Direction::Ltr,
        p::Direction::Rtl => shodo::geometry::Direction::Rtl,
        _ => return Err(unsupported("direction is not represented by shodo")),
    };
    let unicode_bidi = same_enum!(
        UnicodeBidi, cv.unicode_bidi, node;
        Normal, Embed, Isolate, BidiOverride, IsolateOverride, Plaintext
    )?;
    let vertical_align = match cv.vertical_align {
        p::VerticalAlign::Baseline => s::VerticalAlign::Baseline,
        p::VerticalAlign::Sub => s::VerticalAlign::Sub,
        p::VerticalAlign::Super => s::VerticalAlign::Super,
        p::VerticalAlign::Middle => s::VerticalAlign::Middle,
        p::VerticalAlign::TextTop => s::VerticalAlign::TextTop,
        p::VerticalAlign::TextBottom => s::VerticalAlign::TextBottom,
        p::VerticalAlign::Top => s::VerticalAlign::Top,
        p::VerticalAlign::Bottom => s::VerticalAlign::Bottom,
        p::VerticalAlign::Length(p::Length::Px(value)) => s::VerticalAlign::Length(value),
        // The cascade absolutizes lengths; an `em` left over is taken
        // against the element's own font size.
        p::VerticalAlign::Length(p::Length::Em(value)) => {
            s::VerticalAlign::Length(value * cv.font_size.0)
        }
        // A percentage refers to the element's own `line-height` (CSS 2.1
        // 10.8.1); `normal` is taken as 1.2 times the font size.
        p::VerticalAlign::Calc(calc) => {
            let line_height = match cv.line_height {
                ComputedLineHeight::Normal => 1.2 * cv.font_size.0,
                ComputedLineHeight::Number(value) => value * cv.font_size.0,
                ComputedLineHeight::Length(value) => value.0,
            };
            s::VerticalAlign::Length(calc.px + calc.percent / 100.0 * line_height)
        }
        // Other units are absolutized by the cascade; one that is not
        // shifts by nothing.
        p::VerticalAlign::Length(_) => s::VerticalAlign::Baseline,
        _ => return Err(unsupported("vertical-align is not represented by shodo")),
    };
    let line_break =
        same_enum!(LineBreak, cv.line_break, node; Auto, Loose, Normal, Strict, Anywhere)?;
    let (word_break, overflow_wrap) = match cv.word_break {
        // shodo models the legacy keyword directly, including its anywhere
        // min-content sizing behavior, while retaining the authored longhand.
        p::WordBreak::BreakWord => (
            s::WordBreak::BreakWord,
            same_enum!(OverflowWrap, cv.overflow_wrap, node; Normal, BreakWord, Anywhere)?,
        ),
        word_break => (
            same_enum!(WordBreak, word_break, node; Normal, BreakAll, KeepAll, Manual, AutoPhrase)?,
            same_enum!(OverflowWrap, cv.overflow_wrap, node; Normal, BreakWord, Anywhere)?,
        ),
    };
    let hyphens = same_enum!(Hyphens, cv.hyphens, node; None, Manual, Auto)?;
    let hyphenate_character = match &cv.hyphenate_character {
        p::HyphenateCharacter::Auto => None,
        p::HyphenateCharacter::String(value) => Some(value.to_string()),
        _ => {
            return Err(unsupported(
                "hyphenate-character is not represented by shodo",
            ));
        }
    };
    let tab_size = match cv.tab_size {
        ComputedTabSize::Number(value) => s::TabSize::Spaces(value),
        ComputedTabSize::Length(value) => s::TabSize::Px(value.0),
    };
    let text_autospace = match cv.text_autospace {
        // `auto` behaves like `normal`.
        p::TextAutospace::Normal | p::TextAutospace::Auto => s::TextAutospace::Normal,
        p::TextAutospace::NoAutospace => s::TextAutospace::NoAutospace,
        // shodo has no per-class switches: a custom set that spaces
        // ideographs next to letters or numbers is taken as `normal`, one
        // that does not as `no-autospace`.
        p::TextAutospace::Custom {
            ideograph_alpha,
            ideograph_numeric,
            ..
        } => {
            if ideograph_alpha || ideograph_numeric {
                s::TextAutospace::Normal
            } else {
                s::TextAutospace::NoAutospace
            }
        }
        _ => return Err(unsupported("text-autospace is not represented by shodo")),
    };
    let text_spacing_trim = same_enum!(
        TextSpacingTrim, cv.text_spacing_trim, node;
        Normal, SpaceAll, TrimStart, SpaceFirst, TrimBoth, TrimAll, Auto
    )?;
    let text_orientation = map_text_orientation(cv.text_orientation).map_err(unsupported)?;
    let text_combine_upright =
        map_text_combine_upright(cv.text_combine_upright).map_err(unsupported)?;

    let font_families = map_font_families(&cv.font_family).map_err(unsupported)?;

    Ok(InlineStyle {
        font_families,
        font_size: cv.font_size.0,
        font_weight: cv.font_weight,
        font_style,
        font_kerning,
        font_synthesis,
        font_variant_ligatures,
        font_variant_caps,
        font_variant_position,
        font_variant_numeric,
        font_variant_east_asian,
        font_optical_sizing,
        font_variations,
        font_features,
        line_height,
        letter_spacing,
        word_spacing,
        word_spacing_percent,
        white_space_collapse,
        text_wrap_mode,
        line_break,
        word_break,
        overflow_wrap,
        hyphens,
        hyphenate_character,
        text_transform,
        tab_size,
        text_autospace,
        text_spacing_trim,
        vertical_align,
        direction,
        unicode_bidi,
        text_orientation,
        text_combine_upright,
        ..Default::default()
    })
}

/// Paragraph-level style for a block root.
pub(crate) fn paragraph_style(
    cv: &ComputedValues,
    node: usize,
    root: InlineStyle,
) -> Result<ParagraphStyle, IfcError> {
    Ok(ParagraphStyle {
        writing_mode: shodo_writing_mode(cv.cssom_writing_mode, node)?,
        direction: root.direction,
        unicode_bidi_plaintext: root.unicode_bidi == s::UnicodeBidi::Plaintext,
        root,
        ..ParagraphStyle::default()
    })
}

pub(crate) fn shodo_writing_mode(
    mode: p::WritingMode,
    node: usize,
) -> Result<WritingMode, IfcError> {
    match mode {
        p::WritingMode::HorizontalTb => Ok(WritingMode::HorizontalTb),
        p::WritingMode::VerticalRl => Ok(WritingMode::VerticalRl),
        p::WritingMode::VerticalLr => Ok(WritingMode::VerticalLr),
        _ => Err(IfcError::Unsupported {
            node,
            reason: "writing-mode is not represented by the inline path",
        }),
    }
}

/// Line-level options of a block root. The raw `text-indent` is returned next
/// to them because its length needs the containing block's width; the options
/// carry a zero length until the caller resolves it.
pub(crate) fn line_options(
    cv: &ComputedValues,
    node: usize,
    fonts: &FontCollection,
) -> Result<(LineOptions, ComputedTextIndent), IfcError> {
    let unsupported = |reason: &'static str| IfcError::Unsupported { node, reason };
    let indent = match cv.text_indent_ch_factor {
        None => cv.text_indent,
        Some(factor) => {
            let key = cv
                .text_indent_ch_font
                .clone()
                .unwrap_or_else(|| own_ch_font(cv));
            let measured = factor * ch_advance(fonts, &key) + cv.text_indent_ch_offset;
            match cv.text_indent {
                // A `ch` value with a percentage keeps the percentage term and
                // takes the measured length as its absolute term.
                ComputedTextIndent::Calc(calc) => {
                    ComputedTextIndent::Calc(p::CalcLengthPercentage {
                        percent: calc.percent,
                        px: measured,
                    })
                }
                _ => ComputedTextIndent::Px(measured),
            }
        }
    };
    let text_align = match cv.text_align {
        p::TextAlign::Start => s::TextAlign::Start,
        p::TextAlign::End => s::TextAlign::End,
        p::TextAlign::Left => s::TextAlign::Left,
        p::TextAlign::Right => s::TextAlign::Right,
        p::TextAlign::Center | p::TextAlign::InternalCenter => s::TextAlign::Center,
        p::TextAlign::Justify => s::TextAlign::Justify,
        p::TextAlign::JustifyAll => s::TextAlign::JustifyAll,
        _ => return Err(unsupported("text-align needs the parent's resolved value")),
    };
    // `match-parent` is left to the paragraph's `text-align`, as `auto` does.
    let text_align_last = if cv.text_align_last == p::TextAlignLast::MatchParent {
        Ok(s::TextAlignLast::Auto)
    } else {
        same_enum!(
            TextAlignLast, cv.text_align_last, node;
            Auto, Start, End, Left, Right, Center, Justify
        )
    }?;
    // `distribute` is the legacy name of `inter-character` (CSS Text 3, 7.4).
    let text_justify = if cv.text_justify == p::TextJustify::Distribute {
        Ok(s::TextJustify::InterCharacter)
    } else {
        same_enum!(TextJustify, cv.text_justify, node; Auto, None, InterWord, InterCharacter)
    }?;
    let text_wrap_style =
        same_enum!(TextWrapStyle, cv.text_wrap_style, node; Auto, Balance, Pretty, Stable)?;
    Ok((
        LineOptions {
            text_align,
            text_align_last,
            text_justify,
            text_wrap_style,
            hanging_punctuation: match cv.hanging_punctuation {
                p::HangingPunctuation::None => s::HangingPunctuation::default(),
                p::HangingPunctuation::First => s::HangingPunctuation {
                    first: true,
                    ..s::HangingPunctuation::default()
                },
                _ => {
                    return Err(unsupported(
                        "hanging-punctuation is not represented by shodo",
                    ));
                }
            },
            text_indent: s::TextIndent {
                length: 0.0,
                hanging: cv.text_indent_hanging,
                each_line: cv.text_indent_each_line,
            },
            ..Default::default()
        },
        indent,
    ))
}

/// Absolute inline-box edges as logical sides, for horizontal lines (vertical
/// writing modes are laid out horizontally). A `ch` edge is measured with the
/// font that declared it; a percentage or calc() edge would need the
/// containing block's width and is taken as zero.
pub(crate) fn inline_edges(
    cv: &ComputedValues,
    node: usize,
    fonts: &FontCollection,
) -> Result<InlineEdges, IfcError> {
    let unsupported = |reason: &'static str| IfcError::Unsupported { node, reason };
    // An edge in `ch` records the font of the element that declared it.
    let ch_px = |provenance: &Option<ChLengthProvenance>| {
        provenance
            .as_ref()
            .map(|value| value.factor * ch_advance(fonts, &value.font))
    };
    let padding = |value: Length, ch: &Option<ChLengthProvenance>| match (ch_px(ch), value) {
        (Some(px), _) => Ok::<f32, IfcError>(px),
        (None, Length::Px(value)) => Ok(value),
        _ => Ok(0.0),
    };
    let margin = |value: LengthOrAuto, ch: &Option<ChLengthProvenance>| match (ch_px(ch), value) {
        (Some(px), _) => Ok::<f32, IfcError>(px),
        (None, LengthOrAuto::Px(value)) => Ok(value),
        _ => Ok(0.0),
    };
    let logical = |top, right, bottom, left| match cv.direction {
        p::Direction::Ltr => Ok(Sides {
            inline_start: left,
            inline_end: right,
            block_start: top,
            block_end: bottom,
        }),
        p::Direction::Rtl => Ok(Sides {
            inline_start: right,
            inline_end: left,
            block_start: top,
            block_end: bottom,
        }),
        _ => Err(unsupported("edge direction is not represented by shodo")),
    };
    Ok(InlineEdges {
        margin: logical(
            margin(cv.margin.top, &cv.margin_ch.top)?,
            margin(cv.margin.right, &cv.margin_ch.right)?,
            margin(cv.margin.bottom, &cv.margin_ch.bottom)?,
            margin(cv.margin.left, &cv.margin_ch.left)?,
        )?,
        padding: logical(
            padding(cv.padding.top, &cv.padding_ch.top)?,
            padding(cv.padding.right, &cv.padding_ch.right)?,
            padding(cv.padding.bottom, &cv.padding_ch.bottom)?,
            padding(cv.padding.left, &cv.padding_ch.left)?,
        )?,
        border: logical(
            cv.border.top.width().0,
            cv.border.right.width().0,
            cv.border.bottom.width().0,
            cv.border.left.width().0,
        )?,
    })
}

#[cfg(test)]
mod tests;
