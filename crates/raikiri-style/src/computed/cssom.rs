//! CSSOM serialization of computed values, keyed by property name.

use cssparser::ToCss as _;

use crate::ChFontKey;
use crate::computed::ComputedValues;
use crate::property::{
    CalcLengthPercentage, CssColor, PropertyValue, serialize_calc_length_percentage,
    serialize_css_color, serialize_dimension, serialize_number, serialize_percentage,
    serialize_value,
};
use crate::resolve::{
    ComputedLength, ComputedLetterSpacing, ComputedLineHeight, ComputedTabSize,
    ComputedTextDecorationThickness, ComputedTextIndent, ComputedTextUnderlineOffset,
};

/// A property whose computed value [`ComputedProperty::serialize`] can read
/// back as CSSOM text.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputedProperty {
    WhiteSpace,
    WhiteSpaceCollapse,
    LineBreak,
    HyphenateCharacter,
    HyphenateLimitChars,
    Hyphens,
    Font,
    FontKerning,
    FontVariantCaps,
    FontOpticalSizing,
    FontVariantEmoji,
    FontLanguageOverride,
    FontVariantLigatures,
    FontSynthesis,
    FontVariantPosition,
    FontPalette,
    FontVariantNumeric,
    FontVariantEastAsian,
    FontVariationSettings,
    OverflowWrap,
    WordBreak,
    TextTransform,
    TextCombineUpright,
    TextOrientation,
    Direction,
    UnicodeBidi,
    WritingMode,
    TextAutospace,
    WordSpaceTransform,
    TextDecorationSkipInk,
    TextDecorationSkipSpaces,
    TextDecoration,
    TextDecorationStyle,
    TextDecorationLine,
    TextShadow,
    TextDecorationInset,
    TextDecorationColor,
    TextUnderlinePosition,
    TextUnderlineOffset,
    TextEmphasisPosition,
    TextEmphasisStyle,
    TextEmphasis,
    TextSpacingTrim,
    TextSpacing,
    TextWrap,
    TextWrapMode,
    TextWrapStyle,
    TextAlign,
    TextAlignLast,
    TextJustify,
    TextIndent,
    TabSize,
    LetterSpacing,
    WordSpacing,
}

impl ComputedProperty {
    /// Every property, in declaration order.
    #[cfg(test)]
    pub(crate) const ALL: &'static [Self] = &[
        Self::WhiteSpace,
        Self::WhiteSpaceCollapse,
        Self::LineBreak,
        Self::HyphenateCharacter,
        Self::HyphenateLimitChars,
        Self::Hyphens,
        Self::Font,
        Self::FontKerning,
        Self::FontVariantCaps,
        Self::FontOpticalSizing,
        Self::FontVariantEmoji,
        Self::FontLanguageOverride,
        Self::FontVariantLigatures,
        Self::FontSynthesis,
        Self::FontVariantPosition,
        Self::FontPalette,
        Self::FontVariantNumeric,
        Self::FontVariantEastAsian,
        Self::FontVariationSettings,
        Self::OverflowWrap,
        Self::WordBreak,
        Self::TextTransform,
        Self::TextCombineUpright,
        Self::TextOrientation,
        Self::Direction,
        Self::UnicodeBidi,
        Self::WritingMode,
        Self::TextAutospace,
        Self::WordSpaceTransform,
        Self::TextDecorationSkipInk,
        Self::TextDecorationSkipSpaces,
        Self::TextDecoration,
        Self::TextDecorationStyle,
        Self::TextDecorationLine,
        Self::TextShadow,
        Self::TextDecorationInset,
        Self::TextDecorationColor,
        Self::TextUnderlinePosition,
        Self::TextUnderlineOffset,
        Self::TextEmphasisPosition,
        Self::TextEmphasisStyle,
        Self::TextEmphasis,
        Self::TextSpacingTrim,
        Self::TextSpacing,
        Self::TextWrap,
        Self::TextWrapMode,
        Self::TextWrapStyle,
        Self::TextAlign,
        Self::TextAlignLast,
        Self::TextJustify,
        Self::TextIndent,
        Self::TabSize,
        Self::LetterSpacing,
        Self::WordSpacing,
    ];

    /// Looks up a property by name, ASCII case-insensitively. Legacy aliases
    /// such as `word-wrap` map to the property they alias.
    ///
    /// This needs no style data, so callers can reject unsupported names
    /// before doing any style or layout work.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(cssparser::match_ignore_ascii_case! { name,
            "white-space" => Self::WhiteSpace,
            "white-space-collapse" => Self::WhiteSpaceCollapse,
            "line-break" => Self::LineBreak,
            "hyphenate-character" => Self::HyphenateCharacter,
            "hyphenate-limit-chars" => Self::HyphenateLimitChars,
            "hyphens" => Self::Hyphens,
            "font" => Self::Font,
            "font-kerning" => Self::FontKerning,
            "font-variant-caps" => Self::FontVariantCaps,
            "font-optical-sizing" => Self::FontOpticalSizing,
            "font-variant-emoji" => Self::FontVariantEmoji,
            "font-language-override" => Self::FontLanguageOverride,
            "font-variant-ligatures" => Self::FontVariantLigatures,
            "font-synthesis" => Self::FontSynthesis,
            "font-variant-position" => Self::FontVariantPosition,
            "font-palette" => Self::FontPalette,
            "font-variant-numeric" => Self::FontVariantNumeric,
            "font-variant-east-asian" => Self::FontVariantEastAsian,
            "font-variation-settings" => Self::FontVariationSettings,
            "overflow-wrap" => Self::OverflowWrap,
            "word-wrap" => Self::OverflowWrap,
            "word-break" => Self::WordBreak,
            "text-transform" => Self::TextTransform,
            "text-combine-upright" => Self::TextCombineUpright,
            "text-orientation" => Self::TextOrientation,
            "direction" => Self::Direction,
            "unicode-bidi" => Self::UnicodeBidi,
            "writing-mode" => Self::WritingMode,
            "text-autospace" => Self::TextAutospace,
            "word-space-transform" => Self::WordSpaceTransform,
            "text-decoration-skip-ink" => Self::TextDecorationSkipInk,
            "text-decoration-skip-spaces" => Self::TextDecorationSkipSpaces,
            "text-decoration" => Self::TextDecoration,
            "text-decoration-style" => Self::TextDecorationStyle,
            "text-decoration-line" => Self::TextDecorationLine,
            "text-shadow" => Self::TextShadow,
            "text-decoration-inset" => Self::TextDecorationInset,
            "text-decoration-color" => Self::TextDecorationColor,
            "text-underline-position" => Self::TextUnderlinePosition,
            "text-underline-offset" => Self::TextUnderlineOffset,
            "text-emphasis-position" => Self::TextEmphasisPosition,
            "text-emphasis-style" => Self::TextEmphasisStyle,
            "text-emphasis" => Self::TextEmphasis,
            "text-spacing-trim" => Self::TextSpacingTrim,
            "text-spacing" => Self::TextSpacing,
            "text-wrap" => Self::TextWrap,
            "text-wrap-mode" => Self::TextWrapMode,
            "text-wrap-style" => Self::TextWrapStyle,
            "text-align" => Self::TextAlign,
            "text-align-last" => Self::TextAlignLast,
            "text-justify" => Self::TextJustify,
            "text-indent" => Self::TextIndent,
            "tab-size" => Self::TabSize,
            "letter-spacing" => Self::LetterSpacing,
            "word-spacing" => Self::WordSpacing,
            _ => return None,
        })
    }

    /// Serializes this property's computed value from `computed`, or `None`
    /// when the value has no computed CSSOM form (such as an intermediate
    /// `text-align` value).
    ///
    /// `ch_advance` returns the advance of `0` in the given font. It is only
    /// called for lengths authored in `ch`, which need font data the style
    /// layer does not have.
    pub fn serialize(
        self,
        computed: &ComputedValues,
        ch_advance: &mut dyn FnMut(&ChFontKey) -> f32,
    ) -> Option<String> {
        let value = match self {
            ComputedProperty::Direction => computed.direction.as_css_str(),
            ComputedProperty::Font => return serialize_font_shorthand(computed),
            ComputedProperty::FontKerning => computed.font_kerning.as_css_str(),
            ComputedProperty::FontVariantCaps => computed.font_variant_caps.as_css_str(),
            ComputedProperty::FontOpticalSizing => computed.font_optical_sizing.as_css_str(),
            ComputedProperty::FontVariantEmoji => computed.font_variant_emoji.as_css_str(),
            ComputedProperty::FontLanguageOverride => {
                return serialize_value(&PropertyValue::FontLanguageOverride(
                    computed.font_language_override.clone(),
                ));
            }
            ComputedProperty::FontVariantLigatures => computed.font_variant_ligatures.as_css_str(),
            ComputedProperty::FontSynthesis => {
                return serialize_value(&PropertyValue::FontSynthesis(computed.font_synthesis));
            }
            ComputedProperty::FontVariantPosition => computed.font_variant_position.as_css_str(),
            ComputedProperty::FontPalette => {
                return serialize_value(&PropertyValue::FontPalette(computed.font_palette.clone()));
            }
            ComputedProperty::FontVariantNumeric => {
                return serialize_value(&PropertyValue::FontVariantNumeric(
                    computed.font_variant_numeric,
                ));
            }
            ComputedProperty::FontVariantEastAsian => {
                return serialize_value(&PropertyValue::FontVariantEastAsian(
                    computed.font_variant_east_asian,
                ));
            }
            ComputedProperty::FontVariationSettings => {
                return serialize_value(&PropertyValue::FontVariationSettings(
                    computed.font_variation_settings.clone(),
                ));
            }
            ComputedProperty::TextCombineUpright => computed.text_combine_upright.as_css_str(),
            ComputedProperty::TextOrientation => computed.text_orientation.as_css_str(),
            ComputedProperty::WritingMode => computed.cssom_writing_mode.as_css_str(),
            ComputedProperty::UnicodeBidi => computed.unicode_bidi.as_css_str(),
            ComputedProperty::TextWrap => {
                let mode = computed.text_wrap.as_css_str();
                let style = computed.text_wrap_style.as_css_str();
                let value = if mode == "wrap" {
                    if style == "auto" {
                        "wrap".to_owned()
                    } else {
                        style.to_owned()
                    }
                } else if style == "auto" {
                    mode.to_owned()
                } else {
                    format!("{mode} {style}")
                };
                return Some(value);
            }
            ComputedProperty::HyphenateLimitChars => {
                let value = &computed.hyphenate_limit_chars;
                let values = [&value.total, &value.before, &value.after];
                // Omit the third value only when it defaults to the second; two `auto` values serialize as `auto`.
                let value_count = if values[1] == values[2] {
                    if matches!(values[1], &crate::property::HyphenateLimitCharsValue::Auto) {
                        1
                    } else {
                        2
                    }
                } else {
                    3
                };
                let mut serialized = Vec::with_capacity(value_count);
                for component in values.iter().take(value_count).copied() {
                    serialized.push(match *component {
                        crate::property::HyphenateLimitCharsValue::Auto => "auto".to_owned(),
                        crate::property::HyphenateLimitCharsValue::Integer(value) => {
                            value.to_string()
                        }
                    });
                }
                return Some(serialized.join(" "));
            }
            ComputedProperty::HyphenateCharacter => {
                let value = match &computed.hyphenate_character {
                    crate::property::HyphenateCharacter::Auto => "auto".to_owned(),
                    crate::property::HyphenateCharacter::String(value) => {
                        let mut serialized = String::new();
                        cssparser::serialize_string(value, &mut serialized)
                            .expect("serializing a CSS string into String cannot fail");
                        serialized
                    }
                };
                return Some(value);
            }
            ComputedProperty::TextSpacing => {
                let trim =
                    serialize_value(&PropertyValue::TextSpacingTrim(computed.text_spacing_trim))?;
                let autospace =
                    serialize_value(&PropertyValue::TextAutospace(computed.text_autospace))?;
                let value = if trim == "normal" && autospace == "normal" {
                    "normal".to_owned()
                } else if trim == "space-all" && autospace == "no-autospace" {
                    "none".to_owned()
                } else if trim == "auto" && autospace == "auto" {
                    "auto".to_owned()
                } else {
                    // Keep `normal` explicit when omitting it would turn the other
                    // `auto` longhand into the shorthand's two-longhand `auto` alias.
                    let keep_normal = (trim == "auto") != (autospace == "auto");
                    let mut components = Vec::with_capacity(2);
                    if trim != "normal" || keep_normal {
                        components.push(trim);
                    }
                    if autospace != "normal" || keep_normal {
                        components.push(autospace);
                    }
                    components.join(" ")
                };
                return Some(value);
            }
            ComputedProperty::TextSpacingTrim => computed.text_spacing_trim.as_css_str(),
            ComputedProperty::TextAutospace => {
                return serialize_value(&PropertyValue::TextAutospace(computed.text_autospace));
            }
            ComputedProperty::WordSpaceTransform => computed.word_space_transform.as_css_str(),
            ComputedProperty::TextDecorationSkipInk => {
                computed.text_decoration_skip_ink.as_css_str()
            }
            ComputedProperty::TextDecorationSkipSpaces => {
                return serialize_value(&PropertyValue::TextDecorationSkipSpaces(
                    computed.text_decoration_skip_spaces,
                ));
            }
            ComputedProperty::TextDecoration => {
                let mut components = Vec::with_capacity(4);
                let line = serialize_value(&PropertyValue::TextDecorationLine(
                    computed.text_decoration_line,
                ))?;
                if line != "none" {
                    components.push(line);
                }
                if computed.text_decoration_style != crate::property::TextDecorationStyle::Solid {
                    components.push(computed.text_decoration_style.as_css_str().to_owned());
                }
                if computed.text_decoration_thickness
                    != crate::ComputedTextDecorationThickness::Auto
                {
                    components.push(computed.text_decoration_thickness.to_css_string());
                }
                let color = match computed.text_decoration_color {
                    crate::property::TextDecorationColor::CurrentColor => None,
                    crate::property::TextDecorationColor::Resolved(color) => Some(color),
                };
                if let Some(color) = color {
                    components.push(color.to_css_string());
                }
                if components.is_empty() {
                    components.push("none".to_owned());
                }
                return Some(components.join(" "));
            }
            ComputedProperty::TextDecorationStyle => computed.text_decoration_style.as_css_str(),
            ComputedProperty::TextDecorationLine => {
                return serialize_value(&PropertyValue::TextDecorationLine(
                    computed.text_decoration_line,
                ));
            }
            ComputedProperty::TextDecorationInset => {
                let value = match computed.text_decoration_inset {
                    crate::ComputedTextDecorationInset::Auto => "auto".to_owned(),
                    crate::ComputedTextDecorationInset::Lengths { start, end } => {
                        let start_px = computed
                            .text_decoration_inset_start_ch
                            .as_ref()
                            .map_or(start.px(), |provenance| {
                                provenance.factor * ch_advance(&provenance.font)
                            });
                        let end_px = computed
                            .text_decoration_inset_end_ch
                            .as_ref()
                            .map_or(end.px(), |provenance| {
                                provenance.factor * ch_advance(&provenance.font)
                            });
                        let start = ComputedLength(start_px).to_css_string();
                        if start_px == end_px {
                            start
                        } else {
                            let end = ComputedLength(end_px).to_css_string();
                            format!("{start} {end}")
                        }
                    }
                };
                return Some(value);
            }
            ComputedProperty::TextEmphasisPosition => {
                return serialize_value(&PropertyValue::TextEmphasisPosition(
                    computed.text_emphasis_position,
                ));
            }
            ComputedProperty::TextShadow => {
                if computed.text_shadow.is_empty() {
                    return Some("none".to_owned());
                }
                let mut shadows = Vec::with_capacity(computed.text_shadow.len());
                for shadow in computed.text_shadow.iter() {
                    let color = match shadow.color {
                        crate::property::TextShadowColor::CurrentColor => computed.color,
                        crate::property::TextShadowColor::Resolved(color) => color,
                    };
                    shadows.push(format!(
                        "{} {} {} {}",
                        color.to_css_string(),
                        shadow.offset_x.to_css_string(),
                        shadow.offset_y.to_css_string(),
                        shadow.blur_radius.to_css_string()
                    ));
                }
                return Some(shadows.join(", "));
            }
            ComputedProperty::TextEmphasisStyle => {
                return serialize_value(&PropertyValue::TextEmphasisStyle(
                    computed.text_emphasis_style.clone(),
                ));
            }
            ComputedProperty::TextEmphasis => {
                let style = serialize_value(&PropertyValue::TextEmphasisStyle(
                    computed.text_emphasis_style.clone(),
                ))?;
                let color = match computed.text_emphasis_color {
                    crate::property::TextDecorationColor::CurrentColor => computed.color,
                    crate::property::TextDecorationColor::Resolved(color) => color,
                };
                return Some(format!("{style} {}", color.to_css_string()));
            }
            ComputedProperty::TextUnderlinePosition => {
                return serialize_value(&PropertyValue::TextUnderlinePosition(
                    computed.text_underline_position,
                ));
            }
            ComputedProperty::TextUnderlineOffset => {
                return Some(computed.text_underline_offset.to_css_string());
            }
            ComputedProperty::TextDecorationColor => {
                let color = match computed.text_decoration_color {
                    crate::property::TextDecorationColor::CurrentColor => computed.color,
                    crate::property::TextDecorationColor::Resolved(color) => color,
                };
                return Some(color.to_css_string());
            }
            ComputedProperty::LetterSpacing => match measured_spacing(
                computed.letter_spacing_computed,
                computed.letter_spacing_ch_factor,
                computed.letter_spacing_ch_font.as_ref(),
                computed,
                ch_advance,
            ) {
                // `normal` computes to zero, so a zero length reads back as `normal`.
                crate::ComputedLetterSpacing::Px(0.0) => "normal",
                letter_spacing => return Some(letter_spacing.to_css_string()),
            },
            ComputedProperty::WordSpacing => {
                let word_spacing = measured_spacing(
                    computed.word_spacing_computed,
                    computed.word_spacing_ch_factor,
                    computed.word_spacing_ch_font.as_ref(),
                    computed,
                    ch_advance,
                );
                return Some(word_spacing.to_css_string());
            }
            ComputedProperty::TextIndent => {
                let text_indent = match (computed.text_indent_ch_factor, computed.text_indent) {
                    (Some(factor), ComputedTextIndent::Px(_)) => {
                        // An inherited `ch` indent keeps measuring against the
                        // font that declared it.
                        let font = computed
                            .text_indent_ch_font
                            .clone()
                            .unwrap_or_else(|| own_ch_font(computed));
                        ComputedTextIndent::Px(factor * ch_advance(&font))
                    }
                    (_, text_indent) => text_indent,
                };
                let mut value = text_indent.to_css_string();
                if computed.text_indent_hanging {
                    value.push_str(" hanging");
                }
                if computed.text_indent_each_line {
                    value.push_str(" each-line");
                }
                return Some(value);
            }
            ComputedProperty::TabSize => return Some(computed.tab_size.to_css_string()),
            ComputedProperty::WhiteSpace => computed.white_space.as_css_str(),
            ComputedProperty::WhiteSpaceCollapse => computed.white_space_collapse.as_css_str(),
            ComputedProperty::LineBreak => computed.line_break.as_css_str(),
            ComputedProperty::Hyphens => computed.hyphens.as_css_str(),
            ComputedProperty::OverflowWrap => computed.overflow_wrap.as_css_str(),
            ComputedProperty::WordBreak => computed.word_break.as_css_str(),
            ComputedProperty::TextAlign => match computed.text_align {
                // Intermediate-only values must not leak as computed CSSOM values.
                crate::property::TextAlign::MatchParent
                | crate::property::TextAlign::Inherit
                | crate::property::TextAlign::InternalCenter => return None,
                text_align => text_align.as_css_str(),
            },
            ComputedProperty::TextWrapMode => computed.text_wrap.as_css_str(),
            ComputedProperty::TextWrapStyle => computed.text_wrap_style.as_css_str(),
            ComputedProperty::TextAlignLast => computed.text_align_last.as_css_str(),
            ComputedProperty::TextJustify => match computed.text_justify {
                // CSS Text 3 defines legacy `distribute` as computing to `inter-character`.
                crate::property::TextJustify::Distribute => "inter-character",
                text_justify => text_justify.as_css_str(),
            },
            ComputedProperty::TextTransform => computed.text_transform.as_css_str(),
        };
        Some(value.to_owned())
    }
}

fn serialize_font_shorthand(computed: &ComputedValues) -> Option<String> {
    // Reset-only subproperties cannot be expressed by the serialized `font` grammar.
    if computed.font_kerning != crate::property::FontKerning::Auto
        || computed.font_optical_sizing != crate::property::FontOpticalSizing::Auto
        || computed.font_variant_emoji != crate::property::FontVariantEmoji::Normal
        || computed.font_language_override != crate::property::FontLanguageOverride::Normal
        || computed.font_variant_ligatures != crate::property::FontVariantLigatures::Normal
        || computed.font_variant_position != crate::property::FontVariantPosition::Normal
        || computed.font_variant_numeric != crate::property::FontVariantNumeric::initial()
        || computed.font_variant_east_asian != crate::property::FontVariantEastAsian::initial()
        || computed.font_variation_settings != crate::property::FontVariationSettings::Normal
    {
        return None;
    }

    let mut preface = Vec::with_capacity(3);
    let style = match computed.font_style {
        crate::property::FontStyle::Normal => None,
        crate::property::FontStyle::Italic => Some("italic"),
        crate::property::FontStyle::Oblique => Some("oblique"),
    };
    if let Some(style) = style {
        preface.push(style.to_owned());
    }
    match computed.font_variant_caps {
        crate::property::FontVariantCaps::Normal => {}
        crate::property::FontVariantCaps::SmallCaps => preface.push("small-caps".to_owned()),
        _ => return None,
    }
    if computed.font_weight != 400.0 {
        preface.push(serialize_number(computed.font_weight));
    }

    let mut value = preface.join(" ");
    if !value.is_empty() {
        value.push(' ');
    }
    value.push_str(&computed.font_size.to_css_string());
    match computed.line_height {
        ComputedLineHeight::Normal => {}
        ComputedLineHeight::Number(number) => {
            value.push('/');
            value.push_str(&serialize_number(number));
        }
        ComputedLineHeight::Length(length) => {
            value.push('/');
            value.push_str(&length.to_css_string());
        }
    }
    value.push(' ');
    value.push_str(&serialize_font_family(computed.font_family.as_ref())?);
    Some(value)
}

fn serialize_font_family(families: &[crate::property::FontFamilyName]) -> Option<String> {
    if families.is_empty() {
        return None;
    }

    let mut serialized = Vec::with_capacity(families.len());
    for family in families {
        let name = family.as_str();
        if family.1 == crate::property::FontFamilyKind::Generic {
            serialized.push(name.to_owned());
        } else {
            let mut quoted = String::new();
            cssparser::serialize_string(name, &mut quoted)
                .expect("serializing a CSS string into String cannot fail");
            serialized.push(quoted);
        }
    }
    Some(serialized.join(", "))
}

// Computed-value serialization for types that exist only after cascade. Rules
// that belong to one property rather than to the value type (such as
// `letter-spacing: 0px` serializing as `normal`) stay with the caller.

/// The element's own font as a `ch` measuring key.
fn own_ch_font(computed: &ComputedValues) -> ChFontKey {
    ChFontKey {
        family: computed.font_family.clone(),
        size: computed.font_size,
        weight: computed.font_weight,
        style: computed.font_style,
    }
}

/// Replaces the style layer's `ch` fallback in a `letter-spacing` or
/// `word-spacing` length with the measured `0` advance of the font that
/// declared it, falling back to the element's own font. Percentages and
/// `calc()` carry no `ch` factor.
fn measured_spacing(
    value: ComputedLetterSpacing,
    ch_factor: Option<f32>,
    ch_font: Option<&ChFontKey>,
    computed: &ComputedValues,
    ch_advance: &mut dyn FnMut(&ChFontKey) -> f32,
) -> ComputedLetterSpacing {
    match (ch_factor, value) {
        (Some(factor), ComputedLetterSpacing::Px(_)) => {
            let advance = match ch_font {
                Some(font) => ch_advance(font),
                None => ch_advance(&own_ch_font(computed)),
            };
            ComputedLetterSpacing::Px(factor * advance)
        }
        _ => value,
    }
}

/// Serializes a computed mixed `calc()`. Cascade folds a zero term into the
/// plain length or percentage form before a value reaches here; values built
/// directly get the same folding so they serialize identically.
fn serialize_computed_calc(calc: &CalcLengthPercentage) -> String {
    if calc.percent == 0.0 {
        serialize_dimension(calc.px, "px")
    } else if calc.px == 0.0 {
        serialize_percentage(calc.percent)
    } else {
        serialize_calc_length_percentage(calc)
    }
}

impl cssparser::ToCss for ComputedLetterSpacing {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        match *self {
            Self::Px(px) => dest.write_str(&serialize_dimension(px, "px")),
            Self::Percent(percent) => dest.write_str(&serialize_percentage(percent)),
            Self::Calc(calc) => dest.write_str(&serialize_computed_calc(&calc)),
        }
    }
}

impl cssparser::ToCss for ComputedTextIndent {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        match *self {
            Self::Px(px) => dest.write_str(&serialize_dimension(px, "px")),
            Self::Percent(percent) => dest.write_str(&serialize_percentage(percent)),
            Self::Calc(calc) => dest.write_str(&serialize_computed_calc(&calc)),
        }
    }
}

impl cssparser::ToCss for ComputedTextUnderlineOffset {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        match *self {
            Self::Auto => dest.write_str("auto"),
            Self::Length(length) => length.to_css(dest),
            Self::Percent(percent) => dest.write_str(&serialize_percentage(percent)),
            Self::Calc(calc) => dest.write_str(&serialize_computed_calc(&calc)),
        }
    }
}

impl cssparser::ToCss for ComputedTextDecorationThickness {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        match *self {
            Self::Auto => dest.write_str("auto"),
            Self::FromFont => dest.write_str("from-font"),
            Self::Length(length) => length.to_css(dest),
        }
    }
}

impl cssparser::ToCss for ComputedTabSize {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        match *self {
            Self::Number(number) => dest.write_str(&serialize_number(number)),
            Self::Length(length) => length.to_css(dest),
        }
    }
}

impl cssparser::ToCss for ComputedLength {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        dest.write_str(&serialize_dimension(self.px(), "px"))
    }
}

impl cssparser::ToCss for CssColor {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        dest.write_str(&serialize_css_color(self))
    }
}

#[cfg(test)]
mod tests;
