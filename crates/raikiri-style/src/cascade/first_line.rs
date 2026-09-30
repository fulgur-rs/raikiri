//! CSS first-line applicability and alternate inline inheritance.

use crate::property::PropertyKey;

/// Properties supported here that apply to an inline first-line box.
/// CSS Pseudo-Elements 4 §2.1.2 permits font, background, text decoration,
/// inline typesetting/layout, color and opacity. Box geometry and the three
/// excluded writing properties do not apply through the pseudo rule.
pub(crate) fn first_line_property_applies(key: PropertyKey) -> bool {
    use PropertyKey::*;
    matches!(
        key,
        Color
            | Opacity
            | Font
            | FontFamily
            | FontSize
            | FontWeight
            | FontStyle
            | FontVariantCaps
            | FontKerning
            | FontOpticalSizing
            | FontVariantEmoji
            | FontLanguageOverride
            | FontVariantLigatures
            | FontSynthesis
            | FontVariantPosition
            | FontPalette
            | FontVariantNumeric
            | FontVariantEastAsian
            | FontVariationSettings
            | Background
            | BackgroundColor
            | BackgroundImage
            | BackgroundRepeat
            | BackgroundAttachment
            | BackgroundClip
            | BackgroundOrigin
            | BackgroundSize
            | BackgroundPosition
            | LineHeight
            | VerticalAlign
            | RubyPosition
            | TextTransform
            | WordBreak
            | LineBreak
            | OverflowWrap
            | LetterSpacing
            | WordSpacing
            | WhiteSpace
            | WhiteSpaceCollapse
            | Hyphens
            | HyphenateCharacter
            | HyphenateLimitChars
            | TabSize
            | TextAutospace
            | TextSpacing
            | TextSpacingTrim
            | WordSpaceTransform
            | TextCombineUpright
            | TextDecoration
            | TextDecorationLine
            | TextDecorationColor
            | TextDecorationStyle
            | TextDecorationThickness
            | TextDecorationInset
            | TextDecorationSkipInk
            | TextDecorationSkipSpaces
            | TextShadow
            | TextUnderlineOffset
            | TextUnderlinePosition
            | TextEmphasis
            | TextEmphasisPosition
            | TextEmphasisStyle
            | TextEmphasisColor
    )
}

#[cfg(test)]
mod tests;
