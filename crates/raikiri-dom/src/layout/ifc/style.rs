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

pub(crate) fn map_writing_mode(mode: p::WritingMode) -> Result<WritingMode, &'static str> {
    match mode {
        p::WritingMode::HorizontalTb => Ok(WritingMode::HorizontalTb),
        p::WritingMode::VerticalRl => Ok(WritingMode::VerticalRl),
        p::WritingMode::VerticalLr => Ok(WritingMode::VerticalLr),
        p::WritingMode::SidewaysRl => Ok(WritingMode::SidewaysRl),
        p::WritingMode::SidewaysLr => Ok(WritingMode::SidewaysLr),
        _ => Err("writing-mode is not represented by shodo"),
    }
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

/// A `position: relative` box whose offsets are all `auto` or `0` and that
/// makes no stacking context: it moves nothing and paints in place.
pub(crate) fn is_inert_relative(cv: &ComputedValues) -> bool {
    let zero = |value: LengthOrAuto| match value {
        LengthOrAuto::Auto => true,
        LengthOrAuto::Px(px) => px == 0.0,
        _ => false,
    };
    cv.position == p::PositionValue::Relative
        && zero(cv.top)
        && zero(cv.right)
        && zero(cv.bottom)
        && zero(cv.left)
        && cv.z_index == p::ZIndexValue::Auto
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
    let absolute_spacing = |value: &ComputedLetterSpacing| match value {
        ComputedLetterSpacing::Px(value) => Ok(*value),
        _ => Err(unsupported(
            "percentage or calc spacing needs used-value resolution",
        )),
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
        None => absolute_spacing(&cv.letter_spacing_computed)?,
    };
    let word_spacing = match cv.word_spacing_ch_factor {
        Some(factor) => ch_length(factor, &cv.word_spacing_ch_font, cv.word_spacing_ch_offset),
        None => absolute_spacing(&cv.word_spacing_computed)?,
    };
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
    let text_transform = same_enum!(
        TextTransform, cv.text_transform, node;
        None, Capitalize, Uppercase, Lowercase, FullWidth, FullSizeKana,
        CapitalizeFullWidth, UppercaseFullWidth, LowercaseFullWidth,
        CapitalizeFullSizeKana, UppercaseFullSizeKana, LowercaseFullSizeKana,
        FullWidthFullSizeKana, CapitalizeFullWidthFullSizeKana,
        UppercaseFullWidthFullSizeKana, LowercaseFullWidthFullSizeKana
    )?;
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
        _ => return Err(unsupported("vertical-align length needs resolution")),
    };
    let line_break =
        same_enum!(LineBreak, cv.line_break, node; Auto, Loose, Normal, Strict, Anywhere)?;
    let (word_break, overflow_wrap) = match cv.word_break {
        // The deprecated keyword: normal breaking plus `overflow-wrap:
        // break-word`, whatever the authored `overflow-wrap` is. The parley
        // path maps it the same way, so min-content sizing agrees too (CSS
        // Text 3 §5.2 would give it the `anywhere` sizing instead).
        p::WordBreak::BreakWord => (s::WordBreak::Normal, s::OverflowWrap::BreakWord),
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
    let text_autospace = same_enum!(TextAutospace, cv.text_autospace, node; Normal, NoAutospace)?;
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
        line_height,
        letter_spacing,
        word_spacing,
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
///
/// The writing mode comes from `cssom_writing_mode`: the renderer-facing
/// `writing_mode` is normalized to horizontal and would silently drop vertical
/// text.
pub(crate) fn paragraph_style(
    cv: &ComputedValues,
    node: usize,
    root: InlineStyle,
) -> Result<ParagraphStyle, IfcError> {
    let writing_mode = map_writing_mode(cv.cssom_writing_mode)
        .map_err(|reason| IfcError::Unsupported { node, reason })?;
    Ok(ParagraphStyle {
        writing_mode,
        direction: root.direction,
        unicode_bidi_plaintext: root.unicode_bidi == s::UnicodeBidi::Plaintext,
        root,
        ..ParagraphStyle::default()
    })
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
    let text_align_last = same_enum!(
        TextAlignLast, cv.text_align_last, node;
        Auto, Start, End, Left, Right, Center, Justify
    )?;
    let text_justify =
        same_enum!(TextJustify, cv.text_justify, node; Auto, None, InterWord, InterCharacter)?;
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

/// Absolute inline-box edges as logical sides. Percentages, calc, and ch edges
/// need a containing-block basis or a font measurement and are rejected.
pub(crate) fn inline_edges(
    cv: &ComputedValues,
    node: usize,
    fonts: &FontCollection,
) -> Result<InlineEdges, IfcError> {
    let unsupported = |reason: &'static str| IfcError::Unsupported { node, reason };
    // The physical-to-logical side mapping below is horizontal-tb only.
    if cv.cssom_writing_mode != p::WritingMode::HorizontalTb {
        return Err(unsupported(
            "inline edges are mapped for horizontal writing only",
        ));
    }
    // An edge in `ch` records the font of the element that declared it.
    let ch_px = |provenance: &Option<ChLengthProvenance>| {
        provenance
            .as_ref()
            .map(|value| value.factor * ch_advance(fonts, &value.font))
    };
    let padding = |value: Length, ch: &Option<ChLengthProvenance>| match (ch_px(ch), value) {
        (Some(px), _) => Ok(px),
        (None, Length::Px(value)) => Ok(value),
        _ => Err(unsupported(
            "inline percentage or calc padding needs a basis",
        )),
    };
    let margin = |value: LengthOrAuto, ch: &Option<ChLengthProvenance>| match (ch_px(ch), value) {
        (Some(px), _) => Ok(px),
        (None, LengthOrAuto::Px(value)) => Ok(value),
        (None, LengthOrAuto::Auto) => Ok(0.0),
        _ => Err(unsupported(
            "inline percentage or calc margin needs a basis",
        )),
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
