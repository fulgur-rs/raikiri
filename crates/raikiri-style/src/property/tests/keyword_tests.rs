use super::*;

/// Every keyword a table serializes must parse back to the same variant,
/// case-insensitively, and no two variants may share a keyword.
macro_rules! assert_keyword_round_trip {
    ($($ty:ident),+ $(,)?) => {$(
        let mut seen = std::collections::HashSet::new();
        for &value in $ty::ALL {
            let css = value.as_css_str();
            assert!(seen.insert(css), "{} repeats {css:?}", stringify!($ty));
            assert_eq!($ty::from_css_ident(css), Some(value), "{css}");
            assert_eq!($ty::from_css_ident(&css.to_ascii_uppercase()), Some(value), "{css}");
        }
        assert_eq!($ty::from_css_ident("not-a-keyword"), None);
    )+};
}

#[test]
fn keyword_tables_round_trip_through_their_parsers() {
    assert_keyword_round_trip!(
        BackgroundAttachment,
        BackgroundRepeatKeyword,
        BorderCollapseValue,
        BorderStyle,
        BoxSizing,
        BreakBetween,
        BreakInside,
        CaptionSideValue,
        ClearValue,
        ColumnFillValue,
        ContentAlignmentValue,
        ContentPart,
        ContentTextKeyword,
        Direction,
        DisplayValue,
        EmptyCellsValue,
        FlexDirectionValue,
        FlexWrapValue,
        FillRule,
        FloatValue,
        FontKerning,
        FontOpticalSizing,
        FontStyle,
        FontVariantCaps,
        FontVariantEmoji,
        FontVariantLigatures,
        FontVariantPosition,
        GeometryBox,
        HangingPunctuation,
        HorizontalSide,
        Hyphens,
        Isolation,
        LineBreak,
        ListStylePosition,
        MixBlendMode,
        ObjectFit,
        OutlineStyle,
        OverflowValue,
        OverflowWrap,
        QuoteKeyword,
        RadialExtent,
        RadialShape,
        RubyPosition,
        TableLayoutValue,
        TextAlign,
        TextAlignAll,
        TextAlignLast,
        TextCombineUpright,
        TextOverflowValue,
        TextDecorationSkipInk,
        TextDecorationStyle,
        TextJustify,
        TextOrientation,
        TextSpacingTrim,
        TextWrapMode,
        TextWrapStyle,
        UnicodeBidi,
        StringFetchMode,
        VerticalSide,
        Visibility,
        VisualBox,
        WhiteSpace,
        WhiteSpaceCollapse,
        WordBreak,
        WritingMode,
    );
}

#[test]
fn multi_token_keyword_tables_round_trip_through_the_property_parser() {
    for &value in TextTransform::ALL {
        assert_eq!(
            parse_entire(value.as_css_str(), "text-transform"),
            Some(PropertyValue::TextTransform(value)),
            "{}",
            value.as_css_str()
        );
    }
    for &value in WordSpaceTransform::ALL {
        assert_eq!(
            parse_entire(value.as_css_str(), "word-space-transform"),
            Some(PropertyValue::WordSpaceTransform(value)),
            "{}",
            value.as_css_str()
        );
    }
}
