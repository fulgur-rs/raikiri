//! CSS-wide keywords on the properties whose specified value is a plain copy
//! of the computed one.
//!
//! `inherit` takes the parent's computed value, `initial` the initial value,
//! and `unset` one of the two by whether the property is inherited (CSS
//! Cascading 4 section 7.3). `revert` and `revert-layer` reach the cascade as
//! markers like the keywords above and are rolled back before this runs; one
//! that finds nothing to roll back behaves as `unset`.
//!
//! A property joins by adding one row to the table below: its key, whether it
//! is inherited, and the specified value its computed field converts back to.
//! A row also lets the parser keep the keyword for that property instead of
//! dropping the declaration. A row whose computed value reads back wrongly for
//! the initial style, because it depends on another property, adds an
//! `initial =>` value of its own.

use std::sync::{Arc, LazyLock};

use crate::computed::{ComputedBackgroundLayer, ComputedValues};
use crate::property::{
    BORDER_WIDTH_MEDIUM_PX, ColumnWidthValue, CssWideKeyword, FloatValue, Length, PositionValue,
    PropertyKey, PropertyValue, TextIndentValue,
};
use crate::resolve::{
    ComputedColumnWidth, lift_background_size, lift_border_spacing, lift_box_shadow,
    lift_css_position, lift_flex_basis, lift_length_percentage_or_normal, lift_letter_spacing,
    lift_line_height, lift_tab_size, lift_text_decoration_inset, lift_text_decoration_thickness,
    lift_text_indent, lift_text_underline_offset, lift_transform, lift_word_spacing,
};

static INITIAL: LazyLock<ComputedValues> = LazyLock::new(ComputedValues::initial);

/// A copy of `value`, written so a computed field converts to its specified
/// value the same way whether or not the type is `Copy`.
fn dup<T: Clone>(value: &T) -> T {
    value.clone()
}

/// The computed `float` of `source`: CSS 2.1 section 9.7 computes it to `none`
/// on an absolutely positioned box, which the stored field keeps as authored.
fn computed_float(source: &ComputedValues) -> FloatValue {
    match source.position {
        PositionValue::Absolute | PositionValue::Fixed => FloatValue::None,
        _ => source.float,
    }
}

/// The specified `column-width` a computed one converts back to.
fn lift_column_width(computed: ComputedColumnWidth) -> ColumnWidthValue {
    match computed {
        ComputedColumnWidth::Auto => ColumnWidthValue::Auto,
        ComputedColumnWidth::Px(px) => ColumnWidthValue::Length(Length::Px(px)),
    }
}

/// The per-layer list of one background longhand, read from the layers the
/// computed style holds. The computed style cuts every list to the number of
/// images, so an item the source listed beyond its last image is not kept; a
/// property that takes this list and has more images repeats it from its
/// start, as any list does (CSS Backgrounds and Borders 3 section 2.1).
fn layers<T>(source: &ComputedValues, item: impl Fn(&ComputedBackgroundLayer) -> T) -> Arc<[T]> {
    source.background_layers().iter().map(item).collect()
}

macro_rules! keyword_defaults {
    (
        $source:ident;
        $($key:ident, $inherited:literal => $value:expr $(, initial => $initial:expr)?;)*
    ) => {
        /// Whether a CSS-wide keyword on `key` is resolved by this table.
        pub(crate) const fn covers(key: PropertyKey) -> bool {
            matches!(key, $(PropertyKey::$key)|*)
        }

        /// Whether `key` is an inherited property, which decides `unset`.
        const fn is_inherited(key: PropertyKey) -> bool {
            match key {
                $(PropertyKey::$key => $inherited,)*
                _ => false,
            }
        }

        /// The specified value that `initial` gives `key` when it is not the
        /// one the initial style reads back as; `None` for any other key.
        fn initial_value(key: PropertyKey) -> Option<PropertyValue> {
            match key {
                $($(PropertyKey::$key => Some($initial),)?)*
                _ => None,
            }
        }

        /// The specified value that `$source`'s computed value of `key`
        /// converts back to; `None` for a key outside the table.
        fn specified_from(key: PropertyKey, $source: &ComputedValues) -> Option<PropertyValue> {
            Some(match key {
                $(PropertyKey::$key => $value,)*
                _ => return None,
            })
        }

        /// Every key of the table.
        #[cfg(test)]
        pub(crate) const KEYS: &[PropertyKey] = &[$(PropertyKey::$key),*];
    };
}

keyword_defaults! {
    source;
    Position, false => PropertyValue::Position(dup(&source.position));
    HangingPunctuation, true => PropertyValue::HangingPunctuation(dup(&source.hanging_punctuation));
    BoxSizing, false => PropertyValue::BoxSizing(dup(&source.box_sizing));
    TextDecorationLine, false => PropertyValue::TextDecorationLine(dup(&source.text_decoration_line));
    TextDecorationStyle, false => PropertyValue::TextDecorationStyle(dup(&source.text_decoration_style));
    TextTransform, true => PropertyValue::TextTransform(dup(&source.text_transform));
    ZIndex, false => PropertyValue::ZIndex(dup(&source.z_index));
    WordBreak, true => PropertyValue::WordBreak(dup(&source.word_break));
    OverflowWrap, true => PropertyValue::OverflowWrap(dup(&source.overflow_wrap));
    BreakBefore, false => PropertyValue::BreakBefore(dup(&source.break_before));
    BreakAfter, false => PropertyValue::BreakAfter(dup(&source.break_after));
    BreakInside, false => PropertyValue::BreakInside(dup(&source.break_inside));
    Float, false => PropertyValue::Float(computed_float(source));
    Clear, false => PropertyValue::Clear(dup(&source.clear));
    TextWrapStyle, true => PropertyValue::TextWrapStyle(dup(&source.text_wrap_style));
    FlexDirection, false => PropertyValue::FlexDirection(dup(&source.flex_direction));
    FlexWrap, false => PropertyValue::FlexWrap(dup(&source.flex_wrap));
    FlexGrow, false => PropertyValue::FlexGrow(dup(&source.flex_grow));
    FlexShrink, false => PropertyValue::FlexShrink(dup(&source.flex_shrink));
    Order, false => PropertyValue::Order(dup(&source.order));
    JustifyContent, false => PropertyValue::JustifyContent(dup(&source.justify_content));
    AlignContent, false => PropertyValue::AlignContent(dup(&source.align_content));
    AlignItems, false => PropertyValue::AlignItems(dup(&source.align_items));
    AlignSelf, false => PropertyValue::AlignSelf(dup(&source.align_self));
    Hyphens, true => PropertyValue::Hyphens(dup(&source.hyphens));
    LineBreak, true => PropertyValue::LineBreak(dup(&source.line_break));
    TextJustify, true => PropertyValue::TextJustify(dup(&source.text_justify));
    TextCombineUpright, true => PropertyValue::TextCombineUpright(dup(&source.text_combine_upright));
    UnicodeBidi, false => PropertyValue::UnicodeBidi(dup(&source.unicode_bidi));
    FontVariantCaps, true => PropertyValue::FontVariantCaps(dup(&source.font_variant_caps));
    GridTemplateAreas, false => PropertyValue::GridTemplateAreas(dup(&source.grid_template_areas));
    GridAutoFlow, false => PropertyValue::GridAutoFlow(dup(&source.grid_auto_flow));
    GridRowStart, false => PropertyValue::GridRowStart(dup(&source.grid_row_start));
    GridRowEnd, false => PropertyValue::GridRowEnd(dup(&source.grid_row_end));
    GridColumnStart, false => PropertyValue::GridColumnStart(dup(&source.grid_column_start));
    GridColumnEnd, false => PropertyValue::GridColumnEnd(dup(&source.grid_column_end));
    JustifyItems, false => PropertyValue::JustifyItems(dup(&source.justify_items));
    JustifySelf, false => PropertyValue::JustifySelf(dup(&source.justify_self));
    Orphans, true => PropertyValue::Orphans(dup(&source.orphans));
    Widows, true => PropertyValue::Widows(dup(&source.widows));
    RubyPosition, true => PropertyValue::RubyPosition(dup(&source.ruby_position));
    ObjectFit, false => PropertyValue::ObjectFit(dup(&source.object_fit));
    Isolation, false => PropertyValue::Isolation(dup(&source.isolation));
    Contain, false => PropertyValue::Contain(dup(&source.contain));
    WillChange, false => PropertyValue::WillChange(dup(&source.will_change));
    MixBlendMode, false => PropertyValue::MixBlendMode(dup(&source.mix_blend_mode));
    MaskImage, false => PropertyValue::MaskImage(dup(&source.mask_image));
    ClipPath, false => PropertyValue::ClipPath(dup(&source.clip_path));
    Filter, false => PropertyValue::Filter(dup(&source.filter));
    TableLayout, false => PropertyValue::TableLayout(dup(&source.table_layout));
    BorderCollapse, true => PropertyValue::BorderCollapse(dup(&source.border_collapse));
    CaptionSide, true => PropertyValue::CaptionSide(dup(&source.caption_side));
    EmptyCells, true => PropertyValue::EmptyCells(dup(&source.empty_cells));
    TextDecorationSkipInk, true => PropertyValue::TextDecorationSkipInk(dup(&source.text_decoration_skip_ink));
    TextDecorationSkipSpaces, true => PropertyValue::TextDecorationSkipSpaces(dup(&source.text_decoration_skip_spaces));
    TextEmphasisPosition, true => PropertyValue::TextEmphasisPosition(dup(&source.text_emphasis_position));
    TextUnderlinePosition, true => PropertyValue::TextUnderlinePosition(dup(&source.text_underline_position));
    ColumnCount, false => PropertyValue::ColumnCount(dup(&source.column_count));
    TextAutospace, true => PropertyValue::TextAutospace(dup(&source.text_autospace));
    HyphenateCharacter, true => PropertyValue::HyphenateCharacter(dup(&source.hyphenate_character));
    HyphenateLimitChars, true => PropertyValue::HyphenateLimitChars(dup(&source.hyphenate_limit_chars));
    TextSpacingTrim, true => PropertyValue::TextSpacingTrim(dup(&source.text_spacing_trim));
    WordSpaceTransform, true => PropertyValue::WordSpaceTransform(dup(&source.word_space_transform));
    TextEmphasisStyle, true => PropertyValue::TextEmphasisStyle(dup(&source.text_emphasis_style));
    FontKerning, true => PropertyValue::FontKerning(dup(&source.font_kerning));
    FontOpticalSizing, true => PropertyValue::FontOpticalSizing(dup(&source.font_optical_sizing));
    FontVariantEmoji, true => PropertyValue::FontVariantEmoji(dup(&source.font_variant_emoji));
    FontLanguageOverride, true => PropertyValue::FontLanguageOverride(dup(&source.font_language_override));
    FontVariantLigatures, true => PropertyValue::FontVariantLigatures(dup(&source.font_variant_ligatures));
    FontSynthesis, true => PropertyValue::FontSynthesis(dup(&source.font_synthesis));
    FontVariantPosition, true => PropertyValue::FontVariantPosition(dup(&source.font_variant_position));
    FontPalette, true => PropertyValue::FontPalette(dup(&source.font_palette));
    FontVariantNumeric, true => PropertyValue::FontVariantNumeric(dup(&source.font_variant_numeric));
    FontVariantEastAsian, true => PropertyValue::FontVariantEastAsian(dup(&source.font_variant_east_asian));
    FontVariationSettings, true => PropertyValue::FontVariationSettings(dup(&source.font_variation_settings));
    ColumnFill, false => PropertyValue::ColumnFill(dup(&source.column_fill));
    FontFeatureSettings, true => PropertyValue::FontFeatureSettings(dup(&source.font_feature_settings));
    TextOverflow, false => PropertyValue::TextOverflow(dup(&source.text_overflow));
    LineHeight, true => PropertyValue::LineHeight(lift_line_height(source.line_height));
    LetterSpacing, true => {
        PropertyValue::LetterSpacing(lift_letter_spacing(source.letter_spacing_computed))
    };
    WordSpacing, true => {
        PropertyValue::WordSpacing(lift_word_spacing(source.word_spacing_computed))
    };
    TabSize, true => PropertyValue::TabSize(lift_tab_size(source.tab_size));
    BorderSpacing, true => PropertyValue::BorderSpacing(lift_border_spacing(source.border_spacing));
    FlexBasis, false => PropertyValue::FlexBasis(lift_flex_basis(source.flex_basis));
    RowGap, false => PropertyValue::RowGap(lift_length_percentage_or_normal(source.row_gap));
    ColumnGap, false => PropertyValue::ColumnGap(lift_length_percentage_or_normal(source.column_gap));
    ColumnWidth, false => PropertyValue::ColumnWidth(lift_column_width(source.column_width));
    OverflowX, false => PropertyValue::OverflowX(source.overflow.x);
    OverflowY, false => PropertyValue::OverflowY(source.overflow.y);
    TextIndent, true => {
        PropertyValue::TextIndent(TextIndentValue {
            length: lift_text_indent(source.text_indent),
            hanging: source.text_indent_hanging,
            each_line: source.text_indent_each_line,
        })
    };
    TextUnderlineOffset, true => {
        PropertyValue::TextUnderlineOffset(lift_text_underline_offset(source.text_underline_offset))
    };
    TextDecorationThickness, false => {
        PropertyValue::TextDecorationThickness(lift_text_decoration_thickness(
            source.text_decoration_thickness,
        ))
    };
    TextDecorationInset, false => {
        PropertyValue::TextDecorationInset(lift_text_decoration_inset(source.text_decoration_inset))
    };
    Content, false => PropertyValue::Content(dup(&source.content));
    StringSet, false => PropertyValue::StringSet(dup(&source.string_set));
    BoxShadow, false => PropertyValue::BoxShadow(lift_box_shadow(&source.box_shadow));
    ObjectPosition, false => PropertyValue::ObjectPosition(lift_css_position(source.object_position));
    Transform, false => PropertyValue::Transform(lift_transform(&source.transform));
    TransformOrigin, false => {
        PropertyValue::TransformOrigin(
            lift_css_position(source.transform_origin),
            Length::Px(source.transform_origin_z.0),
        )
    };
    BackgroundImage, false => {
        PropertyValue::BackgroundImage(layers(source, |layer| layer.image.clone()))
    };
    BackgroundRepeat, false => PropertyValue::BackgroundRepeat(layers(source, |layer| layer.repeat));
    BackgroundAttachment, false => {
        PropertyValue::BackgroundAttachment(layers(source, |layer| layer.attachment))
    };
    BackgroundClip, false => PropertyValue::BackgroundClip(layers(source, |layer| layer.clip));
    BackgroundOrigin, false => PropertyValue::BackgroundOrigin(layers(source, |layer| layer.origin));
    BackgroundPosition, false => {
        PropertyValue::BackgroundPosition(layers(source, |layer| lift_css_position(layer.position)))
    };
    BackgroundSize, false => {
        PropertyValue::BackgroundSize(layers(source, |layer| lift_background_size(layer.size)))
    };
    OutlineWidth, false => PropertyValue::OutlineWidth(Length::Px(source.outline.width().0)),
        initial => PropertyValue::OutlineWidth(Length::Px(BORDER_WIDTH_MEDIUM_PX));
    OutlineStyle, false => PropertyValue::OutlineStyle(source.outline.style());
    OutlineColor, false => PropertyValue::OutlineColor(source.outline.color);
    OutlineOffset, false => PropertyValue::OutlineOffset(Length::Px(source.outline_offset.0));
    TextDecorationColor, false => PropertyValue::TextDecorationColor(source.text_decoration_color);
    TextEmphasisColor, true => PropertyValue::TextEmphasisColor(source.text_emphasis_color);
}

/// The value the CSS-wide `keyword` gives the property `key` under the parent
/// style `inherited`, or `None` when the table does not cover `key`.
pub(crate) fn defaulting_value(
    key: PropertyKey,
    keyword: CssWideKeyword,
    inherited: &ComputedValues,
) -> Option<PropertyValue> {
    let from_parent = match keyword {
        CssWideKeyword::Inherit => true,
        CssWideKeyword::Initial => false,
        CssWideKeyword::Unset | CssWideKeyword::Revert | CssWideKeyword::RevertLayer => {
            is_inherited(key)
        }
    };
    if !from_parent && let Some(value) = initial_value(key) {
        return Some(value);
    }
    specified_from(key, if from_parent { inherited } else { &INITIAL })
}
