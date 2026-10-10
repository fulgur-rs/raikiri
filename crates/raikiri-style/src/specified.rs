//! The **staging representation** ([`SpecifiedValues`]) of cascade winners, and
//! their absolutization into [`ComputedValues`] (phase 2 + phase 2.5 + phase 3;
//! phase 2.5, line-height absolutization, was added later).
//!
//! # Why a staging representation is needed
//!
//! Lengths carried by cascade winners (`PropertyValue`) are **specified values**
//! and can contain `em` or `rem`. By contrast, [`ComputedValues`] contains only
//! **computed-value types** from phase 2 onward. [`SpecifiedValues`] represents
//! the state between them: all winners have been applied, but their values have
//! not yet been absolutized.
//!
//! As explained in the module docs for [`crate::resolve`], the reference
//! `font-size` for `padding: 2em` is known only **after every winner** for the
//! node has been applied. Absolutizing each winner as it is applied would make
//! the result depend on application order: if `font-size` were applied last,
//! the earlier `2em` would already have been fixed against the old reference.
//! **Absolutization must be a separate phase after applying the winners.**
//!
//! The cascade's winner application order itself is **deterministic**: it
//! traverses a slot array indexed by `PropertyKey` discriminant in ascending
//! order. The constraint above holds regardless of that order. The reference
//! `font-size` cannot be final until every winner has been applied.

use std::sync::Arc;

use smol_str::SmolStr;

use crate::computed::{
    ChFontKey, ChLengthProvenance, ComputedValues, RunningTemplate, VerticalLogicalSize,
};
use crate::property::{
    AlignSelfValue, BORDER_WIDTH_MEDIUM_PX, BackgroundAttachment, BackgroundImage,
    BackgroundRepeat, BackgroundRepeatKeyword, BackgroundSize, Border, BorderCollapseValue,
    BorderColor, BorderRadius, BorderSpacingValue, BorderStyle, BoxShadowItem, BoxSizing,
    BreakBetween, BreakInside, CaptionSideValue, ClearValue, ClipPath, ColumnCountValue,
    ColumnFillValue, ColumnSpanValue, ColumnWidthValue, ContentAlignmentValue, ContentComponent,
    CssColor, CssPosition, CssPositionOffset, Direction, DisplayValue, EmptyCellsValue,
    FilterFunction, FlexBasisValue, FlexDirectionValue, FlexWrapValue, FloatValue, FontFamilyName,
    FontFeatureSettings, FontKerning, FontLanguageOverride, FontOpticalSizing, FontPaletteValue,
    FontStyle, FontSynthesisValue, FontVariantCaps, FontVariantEastAsian, FontVariantEmoji,
    FontVariantLigatures, FontVariantNumeric, FontVariantPosition, FontVariationSettings,
    GridAutoFlowValue, GridLineValue, GridTemplateAreasValue, GridTemplateTracks, GridTrackSize,
    HangingPunctuation, HyphenateCharacter, HyphenateLimitChars, Hyphens, Isolation, Length,
    LengthOrAuto, LengthOrNormal, LetterSpacingValue, LineBreak, LineHeight, ListStylePosition,
    ListStyleType, MaskImage, MixBlendMode, ObjectFit, Outline, OutlineColor, OutlineStyle,
    OverflowValue, OverflowWrap, OverflowXY, PageValue, PositionValue, RubyPosition,
    SelfAlignmentValue, Sides, TabSize, TableLayoutValue, TextAlign, TextAlignLast, TextAutospace,
    TextCombineUpright, TextDecorationColor, TextDecorationInset, TextDecorationLine,
    TextDecorationSkipInk, TextDecorationSkipSpaces, TextDecorationStyle, TextDecorationThickness,
    TextEmphasisHEdge, TextEmphasisPosition, TextEmphasisShape, TextEmphasisStyle,
    TextEmphasisVEdge, TextIndentLength, TextJustify, TextOrientation, TextOverflowValue,
    TextShadowItem, TextSpacingTrim, TextTransform, TextUnderlineOffset, TextUnderlinePosition,
    TextWrapMode, TextWrapStyle, TransformFunction, UnicodeBidi, VerticalAlign, Visibility,
    VisualBox, WhiteSpace, WhiteSpaceCollapse, WordBreak, WordSpaceTransform, WordSpacingValue,
    WritingMode, ZIndexValue, empty_box_shadow_list, empty_content_list, empty_counter_entries,
    empty_filter_list, empty_quotes_entries, empty_string_set_entries, empty_text_shadow_list,
    empty_transform_list, initial_font_family, initial_grid_auto_track_list,
    resolve_display_for_float, resolve_overflow, resolve_text_align_internal_center,
    resolve_text_align_match_parent, resolve_writing_mode,
};
use crate::resolve::{
    ComputedBoxShadowItem, ComputedLength, ComputedLineHeight, ComputedTextIndent, ResolveContext,
    calc_ch_factor, calc_ch_offset, empty_computed_box_shadow_list,
    empty_computed_text_shadow_list, lift_border_spacing, lift_font_size, lift_letter_spacing,
    lift_line_height, lift_tab_size, lift_text_indent, lift_text_shadow_item, lift_word_spacing,
    resolve_background_image, resolve_background_size, resolve_border, resolve_border_radius,
    resolve_border_spacing, resolve_box_shadow_item, resolve_column_width, resolve_css_position,
    resolve_flex_basis, resolve_font_size, resolve_grid_auto_track_list,
    resolve_grid_template_tracks, resolve_length, resolve_length_percentage,
    resolve_length_percentage_or_auto, resolve_length_percentage_or_normal,
    resolve_length_percentage_with_ch, resolve_letter_spacing, resolve_letter_spacing_with_ch,
    resolve_line_height, resolve_margin_length_or_auto, resolve_outline, resolve_tab_size,
    resolve_text_decoration_inset, resolve_text_decoration_thickness, resolve_text_indent_calc,
    resolve_text_shadow_item, resolve_text_underline_offset, resolve_transform_function,
    resolve_vertical_align, resolve_word_spacing, resolve_word_spacing_with_ch,
    used_line_height_length,
};

/// Per-node values after applying cascade winners but before absolutization.
///
/// Has the same set of fields as [`ComputedValues`], but **only fields carrying
/// lengths** retain specified-value types ([`Length`] / [`LengthOrAuto`] /
/// [`LineHeight`] / [`Border`]).
///
/// # Property-to-layer mapping
///
/// Different properties reach different resolution layers during Raikiri's
/// cascade. The field types below encode that mapping; the whole struct cannot
/// simply be called a collection of specified values:
///
/// | Layer | Fields |
/// | --- | --- |
/// | **Still specified** (awaiting phase 2 or phase 3 absolutization) | `font_size` / `line_height` / `padding` / `margin` / `border` / `border_radius` / `box_shadow` / `outline` / `width` / `height` / `text_indent` / `text_decoration_inset` / `text_decoration_thickness` / `letter_spacing` / `word_spacing` / `tab_size` / `text_shadow` / `background_size` / `background_position` / `object_position` / `border_spacing` |
/// | **Already computed-equivalent** (no lengths to absolutize) | `color` / `background_color` / `font_family` / `font_weight` / `display` / `list_style_type` / `list_style_position` / `counter_*` / `content` / `string_set` / `running_templates` / `text_align` / `direction` / `box_sizing` / `overflow` / `text_decoration_line` / `text_decoration_style` / `text_decoration_color` / `text_underline_position` / `text_emphasis_position` / `text_emphasis_style` / `text_emphasis_color` / `font_style` / `font_kerning` / `font_optical_sizing` / `font_variant_emoji` / `font_language_override` / `font_variant_ligatures` / `font_synthesis` / `font_variant_position` / `font_palette` / `font_variant_numeric` / `font_variant_east_asian` / `font_variant_caps` / `text_transform` / `text_combine_upright` / `text_orientation` / `unicode_bidi` / `visibility` / `z_index` / `word_break` / `overflow_wrap` / `break_before` / `break_after` / `break_inside` / `float` / `clear` / `white_space` / `white_space_collapse` / `hyphens` / `hyphenate_character` / `hyphenate_limit_chars` / `quotes` / `orphans` / `widows` / `background_repeat` / `background_attachment` / `background_clip` / `background_origin` / `background_image`\* / `object_fit` / `table_layout` / `border_collapse` / `caption_side` / `empty_cells` |
/// | **Variant-dependent layer** (the type is the same in both layers, but some variants require absolutization) | `vertical_align` — see [`Self::vertical_align`] |
///
/// \* For `background_image`, the `None` and `Url(String)` variants fit the
/// computed-equivalent classification. The `Gradient(..)` variant (CSS Images 4
/// §3) also contains `<length-percentage>` and `<angle>`, yet remains classified
/// here as computed-equivalent. Unlike the `vertical_align` variants, which
/// **are resolved** in phase 2/3, gradient lengths and angles **are never
/// absolutized in these phases**: doing so requires the gradient box's own
/// dimensions, which are outside this struct's scope. See the field docs for
/// [`Self::background_image`].
///
/// The position of `font_weight` in the second row matters: `bolder` and
/// `lighter` are resolved against the parent's computed weight **when**
/// [`crate::cascade::apply_value`] **writes to this struct**. Thus the value
/// is held as a numeric weight (see "D5 invariant" below).
///
/// `font_size` remains in the specified layer, but `larger` and `smaller`
/// (`<relative-size>`) use that same D5 invariant: **at write time**, they are
/// resolved to an absolute value relative to the parent. The resulting
/// `Length::Px` is indistinguishable from an ordinary author-specified px
/// length at the type level, so its classification stays unchanged. See the
/// "D5 invariant" section below and the `FontSizeRelative` arm of
/// [`crate::cascade::apply_value`].
///
/// The page path has no equivalent staging struct. Instead,
/// [`crate::page::cascade_page`] runs the same two phases directly over a bag
/// of `PropertyValue`s. Its intermediate state stays local to that function
/// and is not public. Phase 3 in both paths uses the same functions in
/// [`crate::resolve`].
///
/// # D5 invariant — seed inherited fields from the **parent's computed values**
///
/// [`Self::inherit_from`] seeds inherited properties from the parent's
/// [`ComputedValues`]. This is **required for correctness**, not just an
/// optimization: the `PropertyValue::FontWeight` arm of
/// [`crate::cascade::apply_value`] resolves `bolder` and `lighter` relative
/// to the parent's computed font weight. Seeding from [`Self::initial`]
/// would lose that reference.
///
/// `font_size` follows the same rule: [`Self::inherit_from`] seeds it from
/// the parent's computed value through [`crate::resolve::lift_font_size`],
/// while the root uses the initial value.
///
/// # `text_align: match-parent` differs from D5
///
/// Like `bolder` and `lighter`, `text-align: match-parent` needs information
/// from the parent, but it **does not** use D5's read-before-write pattern.
/// D5 is safe because the `FontWeight` arm of `apply_value` reads and writes
/// only its own field (`self.font_weight`). Resolving `match-parent` requires
/// the parent's value of **another property (`direction`)**. Reusing that
/// trick would introduce a race: if this node has a `direction` winner, its
/// application order could cause the code to read the node's **own** direction
/// rather than its parent's. That would violate the application-order
/// independence invariant of [`crate::cascade::walk_from`]. Thus
/// `text_align`, like the D5 fields, is simply copied from the parent here
/// (see [`Self::inherit_from`]). After **all** winners have been applied,
/// [`Self::finalize`] / [`Self::finalize_as_root`] resolve `match-parent` using
/// an explicit reference to the parent's [`ComputedValues`] (see the docs for
/// [`crate::property::resolve_text_align_match_parent`]).
///
/// # `#[non_exhaustive]`
///
/// As with [`ComputedValues`], this makes future property additions source-compatible.
/// External callers should construct values via [`Self::initial`] or [`Self::inherit_from`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct SpecifiedValues {
    /// Staging value for [`ComputedValues::color`]; computed-equivalent.
    pub color: CssColor,
    /// Staging value for [`ComputedValues::background_color`]; computed-equivalent.
    pub background_color: CssColor,
    /// Symbolic background color retained for explicit inheritance.
    pub(crate) background_color_expression: Option<SmolStr>,
    /// Staging value for [`ComputedValues::font_family`]; computed-equivalent.
    pub font_family: Arc<Vec<FontFamilyName>>,
    /// **Specified** `font-size`; phase 2 ([`resolve_font_size`]) absolutizes it
    /// against the **parent's** computed font size.
    pub font_size: Length,
    /// Staging value for [`ComputedValues::font_weight`]; **already
    /// computed-equivalent** (see the D5 invariant above). The `f32` type
    /// replaces `u16` to retain fractional weights.
    pub font_weight: f32,
    /// **Specified** `line-height`; phase 3 ([`resolve_line_height`])
    /// absolutizes it against this node's computed font size.
    pub line_height: LineHeight,
    /// Staging value for [`ComputedValues::display`]; computed-equivalent.
    pub display: DisplayValue,
    /// Staging value for [`ComputedValues::list_style_type`]; inherited.
    pub list_style_type: ListStyleType,
    /// Staging value for [`ComputedValues::list_style_position`]; inherited.
    pub list_style_position: ListStylePosition,
    /// Staging value for [`ComputedValues::list_style_image`]; inherited.
    pub list_style_image: BackgroundImage,
    /// Staging value for [`ComputedValues::counter_reset`]; computed-equivalent.
    pub counter_reset: Arc<Vec<(SmolStr, i32)>>,
    /// Staging value for [`ComputedValues::counter_increment`]; computed-equivalent.
    pub counter_increment: Arc<Vec<(SmolStr, i32)>>,
    /// Staging value for [`ComputedValues::counter_set`]; computed-equivalent.
    pub counter_set: Arc<Vec<(SmolStr, i32)>>,
    /// Staging value for [`ComputedValues::content`]; computed-equivalent.
    pub content: Arc<Vec<ContentComponent>>,
    /// Staging value for [`ComputedValues::string_set`]; computed-equivalent.
    pub string_set: Arc<Vec<(SmolStr, Vec<ContentComponent>)>>,
    /// Staging value for [`ComputedValues::running_templates`]; computed-equivalent.
    pub running_templates: Vec<RunningTemplate>,
    /// Staging value for `position`; retains its keyword separately from
    /// running_templates to detect relative positioning.
    pub position: PositionValue,
    /// Staging value for [`ComputedValues::text_align`]; computed-equivalent.
    /// `match-parent` is **not resolved here**. See "`text_align: match-parent`
    /// differs from D5" in the docs for [`Self`].
    pub text_align: TextAlign,
    /// [`ComputedValues::hanging_punctuation`] staging. The inherited keyword
    /// set is preserved for the line-layout consumer.
    pub hanging_punctuation: HangingPunctuation,
    /// Staging value for
    /// [`ComputedValues::text_autospace`](crate::computed::ComputedValues::text_autospace);
    /// its keyword/flag set is computed-equivalent and inherited.
    pub text_autospace: TextAutospace,
    /// Staging value for
    /// [`ComputedValues::word_space_transform`](crate::computed::ComputedValues::word_space_transform);
    /// inherited, initially `none`, and retained as specified.
    pub word_space_transform: WordSpaceTransform,
    /// Staging value for
    /// [`ComputedValues::text_spacing_trim`](crate::computed::ComputedValues::text_spacing_trim);
    /// inherited; its computed value is the specified keyword.
    pub text_spacing_trim: TextSpacingTrim,
    /// Staging value for
    /// [`ComputedValues::text_justify`](crate::computed::ComputedValues::text_justify);
    /// inherited and computed-equivalent because it is a keyword.
    pub text_justify: TextJustify,
    /// Staging value for
    /// [`ComputedValues::text_align_last`](crate::computed::ComputedValues::text_align_last);
    /// inherited and computed-equivalent because it is a keyword.
    pub text_align_last: TextAlignLast,
    /// Staging value for [`ComputedValues::direction`]; computed-equivalent.
    pub direction: Direction,
    /// Staging value for [`ComputedValues::writing_mode`]; **not yet
    /// computed-equivalent**. This field retains all five keywords exactly as
    /// specified. [`Self::absolutize_with`] normalizes `vertical-rl`,
    /// `vertical-lr`, `sideways-rl`, and `sideways-lr` to
    /// [`WritingMode::HorizontalTb`] via [`resolve_writing_mode`]. As with
    /// `text-align: match-parent`, resolution belongs to `finalize` /
    /// `absolutize_with`, not to staging (see the Non-goal section of the
    /// [`WritingMode`] docs and the [`Self`] docs).
    pub writing_mode: WritingMode,
    /// `ruby-position` — inherited annotation placement.
    pub ruby_position: RubyPosition,
    /// `text-indent`'s specified `<length-percentage>`, including a deferred
    /// linear `calc()`. Phase 3 resolves its `em` term against this element's
    /// computed font size; percentages remain unresolved. This property is
    /// inherited, so [`Self::inherit_from`] lifts the parent's computed value
    /// into the staging type. CSS Text 3 §8.1
    /// <https://www.w3.org/TR/css-text-3/#text-indent-property>.
    pub text_indent: TextIndentLength,
    /// Authored `ch` factor retained through inheritance for the layout sink.
    pub text_indent_ch_factor: Option<f32>,
    /// Absolute part (px) of a `ch`-bearing `calc()` for `text-indent`, paired with
    /// the factor: a font-aware consumer resolves it as
    /// `factor * advance + offset`. Zero for a plain `Nch` value.
    pub text_indent_ch_offset: f32,
    /// Source font for an inherited `ch` value.
    pub text_indent_ch_font: Option<ChFontKey>,
    /// Whether this value was inherited from an ancestor's `text-indent: ch`.
    pub text_indent_ch_inherited: bool,
    /// `text-indent`'s `hanging` flag staging. Inherited, initial `false`.
    pub text_indent_hanging: bool,
    /// `text-indent`'s `each-line` flag staging. Inherited, initial `false`.
    pub text_indent_each_line: bool,
    /// **Specified** `padding`; phase 3 ([`resolve_length_percentage`]) absolutizes lengths but
    /// leaves percentages unchanged.
    pub padding: Sides<Length>,
    /// **Specified** `margin`; phase 3 ([`resolve_margin_length_or_auto`]) absolutizes its lengths.
    pub margin: Sides<LengthOrAuto>,
    /// **Specified** `border`; phase 3 ([`resolve_border`]) absolutizes its width and applies
    /// style gating for `none` and `hidden`.
    pub border: Sides<Border>,
    /// **Specified** `border-radius`; phase 3 absolutizes all corner lengths against this node's
    /// font size and line height.
    pub border_radius: BorderRadius,
    /// **Specified** `box-shadow`; phase 3 absolutizes each shadow's lengths. This property is
    /// not inherited.
    pub box_shadow: Arc<Vec<BoxShadowItem>>,
    /// **Specified** `outline`; phase 3 absolutizes its width and sets the computed width to zero
    /// for `outline-style: none`.
    pub outline: Outline,
    /// **Specified** `outline-offset`; phase 3 absolutizes it (CSS UI 3 §4.5
    /// <https://www.w3.org/TR/css-ui-3/#outline-offset>). It is a non-inherited `<length>` with
    /// initial value `0`; negatives are allowed.
    pub outline_offset: Length,
    /// **Specified** `width`; absolutized in phase 3.
    pub width: LengthOrAuto,
    /// **Specified** `height`; absolutized in phase 3.
    pub height: LengthOrAuto,
    /// **Specified** `max-width`; absolutized in phase 3.
    pub max_width: LengthOrAuto,
    /// **Specified** `max-height`; absolutized in phase 3.
    pub max_height: LengthOrAuto,
    /// **Specified** `min-width`; absolutized in phase 3.
    pub min_width: LengthOrAuto,
    /// **Specified** `min-height`; absolutized in phase 3.
    pub min_height: LengthOrAuto,
    /// **Specified** `min-block-size`; phase 3 absolutizes it and maps it onto physical
    /// min-width/min-height according to `writing-mode`.
    pub min_block_size: Option<LengthOrAuto>,
    /// **Specified** `inline-size`; phase 3 maps it onto physical `width` or
    /// `height` according to `writing-mode`, then absolutizes it.
    pub inline_size: Option<LengthOrAuto>,
    /// **Specified** `block-size`; mapped onto the axis perpendicular to
    /// [`Self::inline_size`] in phase 3.
    pub block_size: Option<LengthOrAuto>,
    /// Cascade precedence of the winning `width` / `height` / `inline-size` /
    /// `block-size` declarations. Phase 3 lets the logical and physical
    /// declaration that map onto the same axis compete in cascade order.
    pub(crate) preferred_size_precedence: PreferredSizePrecedence,
    /// **Specified** `top`; absolutized in phase 3.
    pub top: LengthOrAuto,
    /// **Specified** `right`; absolutized in phase 3.
    pub right: LengthOrAuto,
    /// **Specified** `bottom`; absolutized in phase 3.
    pub bottom: LengthOrAuto,
    /// **Specified** `left`; absolutized in phase 3.
    pub left: LengthOrAuto,
    /// Staging value for [`ComputedValues::box_sizing`]; computed-equivalent.
    pub box_sizing: BoxSizing,
    /// Staging value for [`ComputedValues::overflow`]. `OverflowValue` has no lengths, so the
    /// layer is computed-equivalent, but cross-axis computed-value coupling is **not resolved
    /// here**. For the same reason as "`text_align: match-parent` differs from D5" in the
    /// [`Self`] docs, phase 3 ([`Self::absolutize_with`]) calls [`resolve_overflow`].
    pub overflow: OverflowXY,
    /// Staging value for [`ComputedValues::text_decoration_line`]; computed-equivalent because
    /// `TextDecorationLine` has no lengths.
    pub text_decoration_line: TextDecorationLine,
    /// Staging value for [`ComputedValues::text_decoration_style`]; computed-equivalent because
    /// `TextDecorationStyle` has no lengths.
    pub text_decoration_style: TextDecorationStyle,
    /// Staging value for [`ComputedValues::text_decoration_color`]; computed-equivalent because
    /// `TextDecorationColor` has no lengths. Resolving currentcolor at used-value time belongs to
    /// painting.
    pub text_decoration_color: TextDecorationColor,
    /// Staging value for [`ComputedValues::text_decoration_thickness`]; phase 3 absolutizes
    /// `<length>` against this node's font size / line height.
    pub text_decoration_thickness: TextDecorationThickness,
    /// Staging value for [`ComputedValues::text_decoration_skip_ink`]; inherited, keyword-only,
    /// and computed-equivalent.
    pub text_decoration_skip_ink: TextDecorationSkipInk,
    /// Staging value for [`ComputedValues::text_decoration_skip_spaces`]; inherited,
    /// keyword-set-only, and computed-equivalent.
    pub text_decoration_skip_spaces: TextDecorationSkipSpaces,
    /// Staging value for [`ComputedValues::text_decoration_inset`]; phase 3 absolutizes lengths
    /// such as `em`/`rem` against this node's font size and line height.
    pub text_decoration_inset: TextDecorationInset,
    /// Staging value for [`ComputedValues::text_underline_offset`]; phase 3 absolutizes inherited
    /// lengths against the declaring node's font size, while percentages remain relative in the
    /// computed value.
    pub text_underline_offset: TextUnderlineOffset,
    /// [`ComputedValues::text_underline_position`] staging. Inherited keyword
    /// set; no relative-value resolution is required.
    pub text_underline_position: TextUnderlinePosition,
    /// [`ComputedValues::text_emphasis_position`] staging. Inherited keyword
    /// value; no relative-value resolution is required.
    pub text_emphasis_position: TextEmphasisPosition,
    /// [`ComputedValues::text_emphasis_style`] staging. Inherited shape/fill
    /// or string value; no relative-value resolution is required.
    pub text_emphasis_style: TextEmphasisStyle,
    /// [`ComputedValues::text_emphasis_color`] staging. Inherited `currentColor`
    /// or resolved color value; no relative-value resolution is required.
    pub text_emphasis_color: TextDecorationColor,
    /// Staging value for [`ComputedValues::vertical_align`]. Its type is the same
    /// [`VerticalAlign`] as [`ComputedValues::vertical_align`], but its resolution layer depends
    /// on the variant. The six keywords `baseline`, `sub`, `super`, `middle`, `text-top`, and
    /// `text-bottom` are computed-equivalent (no lengths); [`VerticalAlign::Length`] and
    /// [`VerticalAlign::Calc`] remain specified and retain `em`/`rem` or mixed px/% until
    /// [`Self::absolutize_with`] calls [`crate::resolve::resolve_vertical_align`]. See that
    /// function's docs for why the types are not split.
    pub vertical_align: VerticalAlign,
    /// Staging value for [`ComputedValues::font_style`]; computed-equivalent because `FontStyle`
    /// carries no lengths within this crate's scope.
    pub font_style: FontStyle,
    /// [`ComputedValues::font_kerning`] staging; inherited keyword, computed-equivalent.
    pub font_kerning: FontKerning,
    /// [`ComputedValues::font_optical_sizing`] staging; inherited keyword, computed-equivalent.
    pub font_optical_sizing: FontOpticalSizing,
    /// [`ComputedValues::font_variant_emoji`] staging; inherited keyword, computed-equivalent.
    pub font_variant_emoji: FontVariantEmoji,
    /// [`ComputedValues::font_language_override`] staging; inherited keyword/string, computed-equivalent.
    pub font_language_override: FontLanguageOverride,
    /// [`ComputedValues::font_variant_ligatures`] staging; inherited keyword, computed-equivalent.
    pub font_variant_ligatures: FontVariantLigatures,
    /// [`ComputedValues::font_synthesis`] staging; inherited computed keyword set.
    pub font_synthesis: FontSynthesisValue,
    /// [`ComputedValues::font_variant_position`] staging; inherited computed keyword.
    pub font_variant_position: FontVariantPosition,
    /// [`ComputedValues::font_palette`] staging; inherited computed keyword or identifier.
    pub font_palette: FontPaletteValue,
    /// [`ComputedValues::font_variant_numeric`] staging; inherited computed keyword set.
    pub font_variant_numeric: FontVariantNumeric,
    /// [`ComputedValues::font_variant_east_asian`] staging; inherited computed value.
    pub font_variant_east_asian: FontVariantEastAsian,
    /// Specified sequence; preserves authored order and duplicates until finalization.
    /// Inherited values come from the parent computed value.
    pub font_variation_settings: FontVariationSettings,
    /// Specified OpenType features; inherited values come from the parent computed value.
    pub font_feature_settings: FontFeatureSettings,
    /// Staging value for [`ComputedValues::font_variant_caps`]; computed-equivalent because
    /// `FontVariantCaps` carries no lengths.
    pub font_variant_caps: FontVariantCaps,
    /// Staging value for [`ComputedValues::text_transform`]; computed-equivalent because
    /// `TextTransform` carries no lengths.
    pub text_transform: TextTransform,
    /// Staging value for [`ComputedValues::text_combine_upright`]; computed-equivalent because
    /// `TextCombineUpright` carries no lengths.
    pub text_combine_upright: TextCombineUpright,
    /// Staging value for [`ComputedValues::text_orientation`]; computed-equivalent because
    /// `TextOrientation` carries no lengths.
    pub text_orientation: TextOrientation,
    /// Staging value for [`ComputedValues::unicode_bidi`]; computed-equivalent because
    /// `UnicodeBidi` carries no lengths.
    pub unicode_bidi: UnicodeBidi,
    /// Staging value for [`ComputedValues::visibility`]; computed-equivalent because `Visibility`
    /// carries no lengths.
    pub visibility: Visibility,
    /// Staging value for [`ComputedValues::z_index`]; computed-equivalent because `ZIndexValue`
    /// carries no lengths.
    pub z_index: ZIndexValue,
    /// Staging value for [`ComputedValues::word_break`]; computed-equivalent because `WordBreak`
    /// carries no lengths.
    pub word_break: WordBreak,
    /// Staging value for [`ComputedValues::line_break`]; computed-equivalent.
    pub line_break: LineBreak,
    /// Staging value for [`ComputedValues::overflow_wrap`]; computed-equivalent because
    /// `OverflowWrap` carries no lengths. The legacy `word-wrap` alias also maps to this field
    /// (see the [`ComputedValues::overflow_wrap`] docs).
    pub overflow_wrap: OverflowWrap,
    /// **Specified** `letter-spacing`; phase 3 ([`crate::resolve::resolve_letter_spacing`])
    /// resolves it into a computed CSS value and a renderer-facing absolute fallback.
    pub letter_spacing: LetterSpacingValue,
    /// Authored `ch` factor retained through inheritance so the text-layout
    /// sink can replace the style fallback with a font metric.
    pub letter_spacing_ch_factor: Option<f32>,
    /// Absolute part (px) of a `ch`-bearing `calc()` for `letter-spacing`, paired with
    /// the factor: a font-aware consumer resolves it as
    /// `factor * advance + offset`. Zero for a plain `Nch` value.
    pub letter_spacing_ch_offset: f32,
    /// Font that declared an inherited `ch` [`Self::letter_spacing_ch_factor`],
    /// so descendants measure it with that font rather than their own.
    pub letter_spacing_ch_font: Option<ChFontKey>,
    /// **Specified** `word-spacing`; its CSS Text 4 length-percentage grammar remains available
    /// through phase 3 for CSSOM computed-value exposure.
    pub word_spacing: WordSpacingValue,
    /// Authored `ch` factor retained through inheritance so the text-layout
    /// sink can replace the style-layer fallback with a font metric.
    pub word_spacing_ch_factor: Option<f32>,
    /// Absolute part (px) of a `ch`-bearing `calc()` for `word-spacing`, paired with
    /// the factor: a font-aware consumer resolves it as
    /// `factor * advance + offset`. Zero for a plain `Nch` value.
    pub word_spacing_ch_offset: f32,
    /// Font that declared an inherited `ch` [`Self::word_spacing_ch_factor`].
    pub word_spacing_ch_font: Option<ChFontKey>,
    /// **Specified** `tab-size`; phase 3 ([`resolve_tab_size`]) absolutizes `<length>` against
    /// this node's computed font size. `<number>` needs no absolutization, as with
    /// [`Self::flex_grow`].
    pub tab_size: TabSize,
    /// Staging value for [`ComputedValues::break_before`]; computed-equivalent because
    /// `BreakBetween` carries no lengths.
    pub break_before: BreakBetween,
    /// Staging value for [`ComputedValues::break_after`]; computed-equivalent because
    /// `BreakBetween` carries no lengths.
    pub break_after: BreakBetween,
    /// Staging value for [`ComputedValues::break_inside`]; computed-equivalent because
    /// `BreakInside` carries no lengths.
    pub break_inside: BreakInside,
    /// `page` is non-inherited and selects the page type for the box that
    /// establishes the next class-A break point (CSS Paged Media 3 §8.1).
    pub page: PageValue,
    /// Staging value for [`ComputedValues::float`]; computed-equivalent because `FloatValue`
    /// carries no lengths.
    pub float: FloatValue,
    /// Staging value for [`ComputedValues::clear`]; computed-equivalent because `ClearValue`
    /// carries no lengths.
    pub clear: ClearValue,
    /// Staging value for [`ComputedValues::white_space`]; computed-equivalent because
    /// `WhiteSpace` carries no lengths.
    pub white_space: WhiteSpace,
    /// [`ComputedValues::white_space_collapse`] staging. Computed-equivalent:
    /// this keyword enum carries no lengths.
    pub white_space_collapse: WhiteSpaceCollapse,
    /// `text-wrap` wrapping component staging. Inherited, initial `wrap`.
    pub text_wrap: TextWrapMode,
    /// `text-wrap-style` staging. Inherited, initial `auto`.
    pub text_wrap_style: TextWrapStyle,
    /// `white-space-collapse` after the legacy `white-space` keyword and the
    /// longhand are settled by cascade order. Inherited. The CSSOM does not
    /// read it; it serializes the declared fields above.
    pub effective_white_space_collapse: WhiteSpaceCollapse,
    /// `text-wrap-mode` after the legacy `white-space` keyword and the
    /// longhands are settled by cascade order. Inherited. The CSSOM does not
    /// read it.
    pub effective_text_wrap_mode: TextWrapMode,
    /// Staging value for [`ComputedValues::hyphens`]; computed-equivalent because `Hyphens`
    /// carries no lengths.
    pub hyphens: Hyphens,
    /// Staging value for [`ComputedValues::hyphenate_character`]. Inherited string/keyword.
    pub hyphenate_character: HyphenateCharacter,
    /// Staging value for [`ComputedValues::hyphenate_limit_chars`]. Inherited computed triple.
    pub hyphenate_limit_chars: HyphenateLimitChars,
    /// Staging value for [`ComputedValues::flex_direction`]; computed-equivalent because
    /// `FlexDirectionValue` carries no lengths.
    pub flex_direction: FlexDirectionValue,
    /// Staging value for [`ComputedValues::flex_wrap`]; computed-equivalent because
    /// `FlexWrapValue` carries no lengths.
    pub flex_wrap: FlexWrapValue,
    /// Staging value for [`ComputedValues::flex_grow`]; computed-equivalent because `<number>`
    /// needs no absolutization (see the [`ComputedValues::flex_grow`] docs).
    pub flex_grow: f32,
    /// Staging value for [`ComputedValues::flex_shrink`]; computed-equivalent.
    pub flex_shrink: f32,
    /// **Specified** `flex-basis`; phase 3 ([`resolve_flex_basis`]) preserves `auto` and
    /// `content` but absolutizes `<length-percentage>`, as with [`Self::width`].
    pub flex_basis: FlexBasisValue,
    /// Staging value for [`ComputedValues::order`]; computed-equivalent because `<integer>` needs
    /// no absolutization, as with [`Self::flex_grow`].
    pub order: i32,
    /// Staging value for [`ComputedValues::justify_content`]; computed-equivalent because
    /// `ContentAlignmentValue` carries no lengths.
    pub justify_content: ContentAlignmentValue,
    /// Staging value for [`ComputedValues::align_content`]; computed-equivalent.
    pub align_content: ContentAlignmentValue,
    /// Staging value for [`ComputedValues::align_items`]; computed-equivalent because
    /// `SelfAlignmentValue` carries no lengths.
    pub align_items: SelfAlignmentValue,
    /// Staging value for [`ComputedValues::align_self`]; computed-equivalent.
    pub align_self: AlignSelfValue,
    /// **Specified** `row-gap`; phase 3 ([`resolve_length_percentage_or_normal`]) preserves
    /// `normal` and absolutizes `<length-percentage>`. Like [`Self::letter_spacing`], it remains
    /// in the specified layer, but its representation of computed `normal` differs (see
    /// [`crate::resolve::ComputedLengthPercentageOrNormal`]).
    pub row_gap: LengthOrNormal,
    /// **Specified** `column-gap`; absolutized in the same phase as [`Self::row_gap`].
    pub column_gap: LengthOrNormal,
    /// Staging value for [`ComputedValues::quotes`]; computed-equivalent because it has no
    /// lengths to absolutize.
    pub quotes: Arc<Vec<(SmolStr, SmolStr)>>,
    /// Whether an empty quotes list is the initial `auto` value.
    pub quotes_auto: bool,
    /// **Specified** `text-shadow`; phase 3 ([`resolve_text_shadow_item`]) absolutizes three
    /// lengths in each item. Like [`Self::padding`], this remains specified, but unlike `padding`
    /// it is **inherited** and is a variable-length list rather than four `Sides<T>`. See the
    /// [`ComputedValues::text_shadow`] docs. `Self::inherit_from` seeds it by lifting the
    /// parent's computed values with [`lift_text_shadow_item`], rather than resetting it to
    /// initial, as for `padding` (the same approach as `Self::text_indent`).
    pub text_shadow: Arc<Vec<TextShadowItem>>,
    /// **Specified** `grid-template-columns`; phase 3 absolutizes `<length-percentage>` in its
    /// track list but preserves `none`. This follows the "keyword or absolutize" pattern of
    /// [`Self::flex_basis`] across an entire track list instead of one value.
    pub grid_template_columns: GridTemplateTracks,
    /// **Specified** `grid-template-rows`; follows the same absolutization phase as
    /// [`Self::grid_template_columns`].
    pub grid_template_rows: GridTemplateTracks,
    /// Staging value for [`ComputedValues::grid_template_areas`]; computed-equivalent: the spec
    /// defines its computed value as the keyword `none` or a list of strings. Unlike track
    /// sizing, it has no `<length-percentage>` to absolutize (see [`GridTemplateAreasValue`]).
    pub grid_template_areas: GridTemplateAreasValue,
    /// **Specified** `grid-auto-columns`; phase 3 applies the same track-size absolutization as
    /// for [`Self::grid_template_columns`].
    pub grid_auto_columns: Arc<Vec<GridTrackSize>>,
    /// **Specified** `grid-auto-rows`; follows the same absolutization phase as
    /// [`Self::grid_auto_columns`].
    pub grid_auto_rows: Arc<Vec<GridTrackSize>>,
    /// Staging value for [`ComputedValues::grid_auto_flow`]; computed-equivalent because
    /// `GridAutoFlowValue` carries no lengths.
    pub grid_auto_flow: GridAutoFlowValue,
    /// Staging value for [`ComputedValues::grid_row_start`]; computed-equivalent because
    /// `GridLineValue` carries no lengths.
    pub grid_row_start: GridLineValue,
    /// Staging value for [`ComputedValues::grid_row_end`]; computed-equivalent.
    pub grid_row_end: GridLineValue,
    /// Staging value for [`ComputedValues::grid_column_start`]; computed-equivalent.
    pub grid_column_start: GridLineValue,
    /// Staging value for [`ComputedValues::grid_column_end`]; computed-equivalent.
    pub grid_column_end: GridLineValue,
    /// Staging value for [`ComputedValues::justify_items`]; computed-equivalent because
    /// `SelfAlignmentValue` carries no lengths.
    pub justify_items: SelfAlignmentValue,
    /// Staging value for [`ComputedValues::justify_self`]; computed-equivalent.
    pub justify_self: AlignSelfValue,
    /// Staging value for [`ComputedValues::orphans`]; computed-equivalent because `<integer>`
    /// carries no lengths.
    pub orphans: i32,
    /// Staging value for [`ComputedValues::widows`]; same layer as [`Self::orphans`].
    pub widows: i32,
    /// Staging value for [`ComputedValues::background_repeat`]; computed-equivalent because
    /// `BackgroundRepeat` carries no lengths.
    pub background_repeat: BackgroundRepeat,
    /// Staging value for [`ComputedValues::background_attachment`]; computed-equivalent because
    /// `BackgroundAttachment` carries no lengths.
    pub background_attachment: BackgroundAttachment,
    /// Staging value for [`ComputedValues::background_clip`]; computed-equivalent because
    /// `VisualBox` carries no lengths.
    pub background_clip: VisualBox,
    /// Staging value for [`ComputedValues::background_origin`]; computed-equivalent because
    /// `VisualBox` carries no lengths.
    pub background_origin: VisualBox,
    /// **Specified** `background-size`; phase 3 absolutizes `<length-percentage>` on each axis,
    /// as for `width` / `height`.
    pub background_size: BackgroundSize,
    /// **Specified** `background-position`; phase 3 absolutizes `<length-percentage>` in each offset.
    pub background_position: CssPosition,
    /// Staging value for [`ComputedValues::background_image`]. `None` and `Url(String)` are
    /// computed-equivalent. Phase 3 absolutizes the font-relative part of `<length-percentage>`
    /// in `Gradient(..)` (CSS Images 4 §3), including `GradientColorStop::position`,
    /// `RadialSize::Circle` / `Ellipse`, and `CssPosition` in `RadialGradient` / `ConicGradient`.
    /// Percentages pass through because they need the gradient box dimensions at paint /
    /// used-value time (see the `resolve_background_image` docs). Angles always pass through.
    pub background_image: BackgroundImage,
    /// Staging value for [`ComputedValues::object_fit`]; computed-equivalent because `ObjectFit`
    /// carries no lengths.
    pub object_fit: ObjectFit,
    /// **Specified** `object-position`; phase 3 absolutizes `<length-percentage>` in each offset,
    /// as for `background_position`, reusing the [`CssPosition`] type.
    pub object_position: CssPosition,
    /// **Specified** `opacity`; retains even out-of-range values without clamping (see "specified
    /// preserves, computed clamps" in the [`ComputedValues::opacity`] docs). Phase 3
    /// ([`Self::absolutize_with`]) clamps it.
    pub opacity: f32,
    /// **Specified** `isolation`; always a keyword. It passes through without absolutization, as
    /// with `object_fit`.
    pub isolation: Isolation,
    /// **Specified** `mix-blend-mode`; same pass-through shape as `isolation`.
    pub mix_blend_mode: MixBlendMode,
    /// **Specified** `mask-image`; `None` / `Url(String)` are computed-equivalent. As for
    /// `background_image`, phase 3 absolutizes font-relative parts of `<length-percentage>` in
    /// `Gradient(..)` but leaves percentages unchanged (see the `resolve_background_image` docs).
    pub mask_image: MaskImage,
    /// **Specified** `clip-path`; has the same shape as `mask_image`. Embedded `<url>` values are
    /// not absolutized (see [`ClipPath`]).
    pub clip_path: ClipPath,
    /// **Specified** `transform`. CSS Transforms Level 1 §4 says its computed value is "as
    /// specified, but with lengths made absolute". Unlike `mask_image` / `filter` ("as
    /// specified", requiring no absolutization), phase 3 ([`Self::absolutize_with`]) absolutizes
    /// the non-percentage `Length` payload of `translate()`, `translateX()`, and `translateY()`
    /// against font size / root font size into `Px`. Percentages remain symbolic `Percent`, as
    /// with [`crate::resolve::resolve_length_percentage`] (see
    /// [`crate::property::TransformFunction`]). `none` is represented by an empty list
    /// ([`empty_transform_list`]).
    pub transform: Arc<Vec<TransformFunction>>,
    /// Border-box origin, non-inherited, initial `50% 50%` (CSS Transforms 1 §5).
    pub transform_origin: CssPosition,
    /// Z origin; retained even though rendering currently supports only 2D.
    pub transform_origin_z: Length,
    /// **Specified** `filter`; has the same list shape as `transform`, but embedded `Length`,
    /// `Angle`, and `f32` values are not absolutized (see [`FilterFunction`]). `none` is
    /// represented by an empty list ([`empty_filter_list`]).
    pub filter: Arc<Vec<FilterFunction>>,
    /// **Specified** `table-layout`; **non-inherited**, initially [`TableLayoutValue::Auto`] (CSS
    /// Tables 3 §4 <https://www.w3.org/TR/css-tables-3/#table-layout-property>). Because the
    /// computed value is the specified keyword, it passes through as staging for
    /// [`crate::computed::ComputedValues::table_layout`], in the same computed-equivalent
    /// category as [`Self::float`].
    pub table_layout: TableLayoutValue,
    /// **Specified** `text-overflow`; **non-inherited**, initially [`TextOverflowValue::Clip`]
    /// (CSS Overflow 3 §5.1). The computed value is the specified keyword, as for
    /// [`Self::table_layout`].
    pub text_overflow: TextOverflowValue,
    /// **Specified** `border-collapse`; **inherited**, initially
    /// [`BorderCollapseValue::Separate`] (CSS Tables 3 §6
    /// <https://www.w3.org/TR/css-tables-3/#border-collapse-property>). Its computed value is the
    /// specified keyword, so staging for [`crate::computed::ComputedValues::border_collapse`]
    /// passes through unchanged. `Self::inherit_from` seeds it from the parent's computed value
    /// by a direct copy, as for [`Self::visibility`], not by lifting it like
    /// [`Self::text_indent`].
    pub border_collapse: BorderCollapseValue,
    /// **Specified** `border-spacing`; **inherited**, initially `0` (`0px` on both axes; CSS
    /// Tables 3 §6.1 <https://www.w3.org/TR/css-tables-3/#border-spacing-property>). Its computed
    /// value has two absolute lengths, so phase 3 absolutizes staging for
    /// [`crate::computed::ComputedValues::border_spacing`], as for the length-bearing arm of
    /// [`Self::tab_size`]. Since it is inherited, `Self::inherit_from` seeds it by lifting the
    /// parent's computed value with [`lift_border_spacing`], again like the `Length` arm of
    /// [`Self::tab_size`].
    pub border_spacing: BorderSpacingValue,
    /// **Specified** `caption-side`; **inherited**, initially [`CaptionSideValue::Top`] (CSS
    /// Tables 3 §7 <https://www.w3.org/TR/css-tables-3/#caption-side-property>). The computed
    /// value is the specified keyword, so staging for
    /// [`crate::computed::ComputedValues::caption_side`] passes through unchanged.
    /// `Self::inherit_from` seeds it by directly copying the parent's computed value, with no
    /// lift needed for a keyword, as for [`Self::visibility`].
    pub caption_side: CaptionSideValue,
    /// **Specified** `empty-cells`; **inherited**, initially [`EmptyCellsValue::Show`] (CSS
    /// Tables 3 §8 <https://www.w3.org/TR/css-tables-3/#empty-cells-property>). The computed
    /// value is the specified keyword, so staging for
    /// [`crate::computed::ComputedValues::empty_cells`] passes through unchanged.
    /// `Self::inherit_from` seeds it by directly copying the parent's computed value, with no
    /// lift needed for a keyword, as for [`Self::visibility`].
    pub empty_cells: EmptyCellsValue,
    /// `column-count` specified value; non-inherited.
    pub column_count: ColumnCountValue,
    /// `column-fill` specified value; non-inherited, initial `balance`.
    pub column_fill: ColumnFillValue,
    /// Non-inherited `column-span`, initially `none`.
    pub column_span: ColumnSpanValue,
    /// `column-width` specified value; non-inherited.
    pub column_width: ColumnWidthValue,
    /// Non-inherited column rule; resolves without contributing to box geometry.
    pub column_rule: Border,
}

/// Cascade sort key of a winning declaration: origin and importance rank,
/// element attachment and layer, specificity, source order, then position among the element's candidates
/// (CSS Cascading 4 §6.1).
pub(crate) type CascadePrecedence = (u8, (bool, u32), u32, u32, usize);

/// [`CascadePrecedence`] of each winning preferred-size declaration; `None`
/// when no declaration won for that property.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PreferredSizePrecedence {
    pub(crate) width: Option<CascadePrecedence>,
    pub(crate) height: Option<CascadePrecedence>,
    pub(crate) inline_size: Option<CascadePrecedence>,
    pub(crate) block_size: Option<CascadePrecedence>,
}

impl SpecifiedValues {
    /// Inherit marker text with the UA defaults required by CSS Lists 3 §3.1.1.
    pub(crate) fn inherit_marker_from(parent: &ComputedValues) -> Self {
        let mut marker = Self::inherit_from(parent);
        marker.unicode_bidi = crate::property::UnicodeBidi::Isolate;
        marker.font_variant_numeric = FontVariantNumeric::initial();
        marker.font_variant_numeric.tabular_nums = true;
        marker.white_space = WhiteSpace::Pre;
        marker.white_space_collapse = WhiteSpaceCollapse::Preserve;
        marker.text_wrap = TextWrapMode::Nowrap;
        marker.effective_white_space_collapse = WhiteSpaceCollapse::Preserve;
        marker.effective_text_wrap_mode = TextWrapMode::Nowrap;
        marker.text_transform = TextTransform::None;
        marker
    }

    /// Staging values with the CSS-specified initial value for every property.
    ///
    /// The field-by-field comments of [`ComputedValues::initial`] and field
    /// docs of [`ComputedValues`] are the canonical sources for these initial
    /// values. This function provides their **specified representation**;
    /// absolutizing it produces [`ComputedValues::initial`] (checked by
    /// `initial_specified_finalizes_to_initial_computed`).
    ///
    /// Used to seed nodes with no parent (the document root or the start of
    /// a detached subtree).
    pub fn initial() -> Self {
        Self {
            color: CssColor::BLACK,
            background_color: CssColor::TRANSPARENT,
            background_color_expression: None,
            // Shared Arc slot avoids an allocation per node (see the `initial_font_family` docs).
            font_family: initial_font_family(),
            // CSS Fonts 4 §2.5: the initial value is `medium` (16px here).
            font_size: Length::Px(crate::computed::INITIAL_FONT_SIZE_PX),
            font_weight: 400.0,
            line_height: LineHeight::Normal,
            display: DisplayValue::Inline,
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            list_style_image: BackgroundImage::None,
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: Vec::new(),
            position: PositionValue::Static,
            text_align: TextAlign::Start,
            // CSS Text 3 §8.2.1: hanging-punctuation initial is `none`.
            hanging_punctuation: HangingPunctuation::None,
            // CSS Text 4: text-autospace initial is `normal`.
            text_autospace: TextAutospace::Normal,
            // CSS Text 4: word-space-transform initial is `none`.
            word_space_transform: WordSpaceTransform::None,
            // CSS Text 4: text-spacing-trim initial is `normal`.
            text_spacing_trim: TextSpacingTrim::Normal,
            // CSS Text 3 §6.2: text-justify initial is `auto`.
            text_justify: TextJustify::Auto,
            // CSS Text 3 §6.1: text-align-last initial is `auto`.
            text_align_last: TextAlignLast::Auto,
            // CSS Writing Modes 4 §2.1: direction is initially `ltr`.
            direction: Direction::Ltr,
            // CSS Writing Modes 4 §3.2: writing-mode is initially `horizontal-tb`.
            writing_mode: WritingMode::HorizontalTb,
            ruby_position: RubyPosition::Over,
            // CSS Text 3 §8.1: text-indent is initially `0`.
            text_indent: TextIndentLength::Length(Length::Px(0.0)),
            text_indent_ch_factor: None,
            text_indent_ch_offset: 0.0,
            text_indent_ch_font: None,
            text_indent_ch_inherited: false,
            text_indent_hanging: false,
            text_indent_each_line: false,
            padding: Sides::all(Length::Px(0.0)),
            margin: Sides::all(LengthOrAuto::Length(Length::Px(0.0))),
            // CSS Backgrounds 3 §3.3 / §3.2 / §3.1: width=medium (3px) / style=none /
            // color=currentcolor. Style gating reduces the width to 0px in the computed layer
            // (`resolve_border`). These section numbers were corrected from 5.x using the spec's
            // actual `data-level` values. Keep 3.x throughout the crate: §5.x of Backgrounds 3
            // covers border-image, so do not restore 5.x here merely for "consistency".
            border: Sides::all(INITIAL_BORDER),
            border_radius: BorderRadius {
                top_left: Length::Px(0.0).into(),
                top_right: Length::Px(0.0).into(),
                bottom_right: Length::Px(0.0).into(),
                bottom_left: Length::Px(0.0).into(),
            },
            box_shadow: empty_box_shadow_list(),
            outline: Outline {
                width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
                style: OutlineStyle::None,
                color: OutlineColor::Invert,
            },
            // CSS UI 3 §4.5: outline-offset is initially `0`.
            outline_offset: Length::Px(0.0),
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
            max_width: LengthOrAuto::Auto,
            max_height: LengthOrAuto::Auto,
            min_width: LengthOrAuto::Auto,
            min_height: LengthOrAuto::Auto,
            min_block_size: None,
            inline_size: None,
            preferred_size_precedence: PreferredSizePrecedence::default(),
            block_size: None,
            top: LengthOrAuto::Auto,
            right: LengthOrAuto::Auto,
            bottom: LengthOrAuto::Auto,
            left: LengthOrAuto::Auto,
            box_sizing: BoxSizing::ContentBox,
            // CSS Overflow 3 §3.1: overflow-x and overflow-y are initially `visible`.
            overflow: OverflowXY::both(OverflowValue::Visible),
            // CSS Text Decoration Module Level 3 §2.1/§2.2/§2.3: the respective initial values
            // are `none`, `solid`, and `currentcolor`.
            text_decoration_line: TextDecorationLine::NONE,
            text_decoration_style: TextDecorationStyle::Solid,
            text_decoration_color: TextDecorationColor::CurrentColor,
            text_decoration_thickness: TextDecorationThickness::Auto,
            // CSS Text Decoration 4: text-decoration-skip-ink initial is `auto`.
            text_decoration_skip_ink: TextDecorationSkipInk::Auto,
            // CSS Text Decoration 4: text-decoration-skip-spaces initial is `start end`.
            text_decoration_skip_spaces: TextDecorationSkipSpaces::StartEnd,
            // CSS Text Decoration 4 §2.9.1: initial is `0` and non-inherited.
            text_decoration_inset: TextDecorationInset::Lengths {
                start: Length::Px(0.0),
                end: Length::Px(0.0),
            },
            text_underline_offset: TextUnderlineOffset::Auto,
            text_underline_position: TextUnderlinePosition::AUTO,
            text_emphasis_position: TextEmphasisPosition::Position {
                vertical: TextEmphasisVEdge::Over,
                horizontal: Some(TextEmphasisHEdge::Right),
            },
            text_emphasis_style: TextEmphasisStyle::None,
            text_emphasis_color: TextDecorationColor::CurrentColor,
            // CSS 2.1 §10.8.1: vertical-align is initially `baseline`.
            vertical_align: VerticalAlign::Baseline,
            // CSS Fonts 4 §2.4: font-style is initially `normal`.
            font_style: FontStyle::Normal,
            font_kerning: FontKerning::Auto,
            // CSS Fonts 4 §8.1: font-optical-sizing initial is `auto`.
            font_optical_sizing: FontOpticalSizing::Auto,
            // CSS Fonts 4 §9.3: font-variant-emoji initial is `normal`.
            font_variant_emoji: FontVariantEmoji::Normal,
            // CSS Fonts 4 §6.13: font-language-override initial is `normal`.
            font_language_override: FontLanguageOverride::Normal,
            // CSS Fonts 4 §6.4: font-variant-ligatures initial is `normal`.
            font_variant_ligatures: FontVariantLigatures::Normal,
            font_synthesis: FontSynthesisValue::initial(),
            font_variant_position: FontVariantPosition::Normal,
            font_palette: FontPaletteValue::Normal,
            font_variant_numeric: FontVariantNumeric::initial(),
            font_variant_east_asian: FontVariantEastAsian::initial(),
            font_variation_settings: FontVariationSettings::Normal,
            font_feature_settings: FontFeatureSettings::Normal,
            // CSS Fonts Module Level 3 §6.6: font-variant-caps is initially `normal`.
            font_variant_caps: FontVariantCaps::Normal,
            // CSS Text Module Level 3 §2.1: text-transform is initially `none`.
            text_transform: TextTransform::None,
            // CSS Writing Modes 3 §9.1: text-combine-upright initial is `none`.
            text_combine_upright: TextCombineUpright::None,
            // CSS Writing Modes 3 §5.1: text-orientation initial is `mixed`.
            text_orientation: TextOrientation::Mixed,
            // CSS Writing Modes 3 §2.2: unicode-bidi initial is `normal`.
            unicode_bidi: UnicodeBidi::Normal,
            // CSS Display 3 §4: visibility is initially `visible`.
            visibility: Visibility::Visible,
            // CSS2 §9.9.1: z-index is initially `auto`.
            z_index: ZIndexValue::Auto,
            // CSS Text 3 §5.1: word-break is initially `normal`.
            word_break: WordBreak::Normal,
            // CSS Text 3 §5.3: line-break is initially `auto`.
            line_break: LineBreak::Auto,
            // CSS Text 3 §5.4: overflow-wrap is initially `normal`.
            overflow_wrap: OverflowWrap::Normal,
            // CSS Text 3 §7.2 / §7.1: both letter-spacing and word-spacing are initially `normal`.
            letter_spacing: LetterSpacingValue::Normal,
            letter_spacing_ch_factor: None,
            letter_spacing_ch_offset: 0.0,
            letter_spacing_ch_font: None,
            word_spacing: WordSpacingValue::Normal,
            word_spacing_ch_factor: None,
            word_spacing_ch_offset: 0.0,
            word_spacing_ch_font: None,
            // CSS Text Module Level 3 §4.2: tab-size is initially `8`.
            tab_size: TabSize::Number(8.0),
            // CSS Fragmentation Module Level 3 §3.1 / §3.2: break-before, break-after, and
            // break-inside are initially `auto`.
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
            // CSS Paged Media 3 §8.1: page initial is `auto` and is not inherited.
            page: PageValue::Auto,
            // CSS2 §9.5.1 / §9.5.2: float and clear are both initially `none`.
            float: FloatValue::None,
            clear: ClearValue::None,
            // CSS Text 3 §3: white-space is initially `normal`.
            white_space: WhiteSpace::Normal,
            // CSS Text 4 white-space-collapse initial value is `collapse`.
            white_space_collapse: WhiteSpaceCollapse::Collapse,
            text_wrap: TextWrapMode::Wrap,
            text_wrap_style: TextWrapStyle::Auto,
            effective_white_space_collapse: WhiteSpaceCollapse::Collapse,
            effective_text_wrap_mode: TextWrapMode::Wrap,
            // CSS Text 3 §5.3: hyphens is initially `manual`.
            hyphens: Hyphens::Manual,
            // CSS Text 4: hyphenate-character initial is `auto`.
            hyphenate_character: HyphenateCharacter::Auto,
            // CSS Text 4: hyphenate-limit-chars initial is `auto`.
            hyphenate_limit_chars: HyphenateLimitChars::INITIAL,
            // CSS Flexible Box Layout Module Level 1 §5.1/§5.2: flex-direction is initially
            // `row`; flex-wrap is initially `nowrap`.
            flex_direction: FlexDirectionValue::Row,
            flex_wrap: FlexWrapValue::NoWrap,
            // CSS Flexible Box Layout Module Level 1 §7.2.1/§7.2.2: flex-grow is initially `0`;
            // flex-shrink is initially `1`.
            flex_grow: 0.0,
            flex_shrink: 1.0,
            // CSS Flexible Box Layout Module Level 1 §7.2.3: flex-basis is initially `auto`.
            flex_basis: FlexBasisValue::Auto,
            // CSS Flexible Box Layout Module Level 1 §4.2: order is initially `0`.
            order: 0,
            // CSS Box Alignment Module Level 3 §5.1 (justify-content / align-content) and §7.2
            // (align-items): the initial value is `normal`; for align-self (§6.2) it is `auto`.
            justify_content: ContentAlignmentValue::Normal,
            align_content: ContentAlignmentValue::Normal,
            align_items: SelfAlignmentValue::Normal,
            align_self: AlignSelfValue::Auto,
            // CSS Box Alignment Module Level 3 §8.1: row-gap and column-gap are initially `normal`.
            row_gap: LengthOrNormal::Normal,
            column_gap: LengthOrNormal::Normal,
            // CSS Content 3 §2.4.1: the specified initial quotes value "depends on user agent".
            // Following the independent-implementation policy, we represent it as an empty list
            // (see the `ComputedValues::quotes` docs).
            quotes: empty_quotes_entries(),
            quotes_auto: true,
            // CSS Text Decoration Module Level 3 §4: text-shadow is initially `none`; use the
            // shared empty Arc slot (see the `empty_text_shadow_list` docs).
            text_shadow: empty_text_shadow_list(),
            // CSS Grid Layout Module Level 1 §7.2/§7.3: all grid-template-* properties are
            // initially `none`.
            grid_template_columns: GridTemplateTracks::None,
            grid_template_rows: GridTemplateTracks::None,
            grid_template_areas: GridTemplateAreasValue::None,
            // CSS Grid Layout Module Level 1 §7.6: grid-auto-columns and grid-auto-rows are
            // initially `auto`.
            grid_auto_columns: initial_grid_auto_track_list(),
            grid_auto_rows: initial_grid_auto_track_list(),
            // CSS Grid Layout Module Level 1 §7.7: grid-auto-flow is initially `row`.
            grid_auto_flow: GridAutoFlowValue::Row,
            // CSS Grid Layout Module Level 1 §8.3: grid-row-start/-end and grid-column-start/-end
            // are initially `auto`.
            grid_row_start: GridLineValue::Auto,
            grid_row_end: GridLineValue::Auto,
            grid_column_start: GridLineValue::Auto,
            grid_column_end: GridLineValue::Auto,
            // CSS Box Alignment Module Level 3 §7.1/§6.1: the initial justify-items /
            // justify-self values. See "legacy is unsupported" in the
            // `PropertyValue::JustifyItems` docs; justify-self is `auto`.
            justify_items: SelfAlignmentValue::Normal,
            justify_self: AlignSelfValue::Auto,
            // CSS Fragmentation Module Level 3 §3.3: orphans and widows are both initially `2`.
            orphans: 2,
            widows: 2,
            // CSS Backgrounds and Borders 3 §2.4: background-repeat is initially `repeat` on both axes.
            background_repeat: BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::Repeat,
            },
            // CSS Backgrounds and Borders 3 §2.5: background-attachment is initially `scroll`.
            background_attachment: BackgroundAttachment::Scroll,
            // CSS Backgrounds and Borders 3 §2.7: background-clip is initially `border-box`,
            // unlike its sibling background_origin (initially `padding-box`).
            background_clip: VisualBox::BorderBox,
            // background-origin is initially `padding-box`.
            background_origin: VisualBox::PaddingBox,
            // CSS Backgrounds and Borders 3 §2.9: background-size is initially `auto` on both
            // axes. This is an explicit spec value, unrelated to the single-value fill rule in
            // the `BackgroundSize` docs.
            background_size: BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            },
            // CSS Backgrounds and Borders 3 §2.6: background-position is initially `0% 0%`.
            background_position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
                vertical: CssPositionOffset::Start(Length::Percent(0.0)),
            },
            // CSS Backgrounds and Borders 3 §2.3: background-image is initially `none`.
            background_image: BackgroundImage::None,
            // CSS Images Module Level 3 §5.1: object-fit is initially `fill`.
            object_fit: ObjectFit::Fill,
            // CSS Images Module Level 3 §5.2: object-position is initially `50% 50%`, unlike
            // background-position (`0% 0%`).
            object_position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
                vertical: CssPositionOffset::Start(Length::Percent(50.0)),
            },
            // CSS Color 4 §3.3: opacity is initially `1`.
            opacity: 1.0,
            // CSS Compositing and Blending Level 1 §3.4.2: isolation is initially `auto`.
            isolation: Isolation::Auto,
            // CSS Compositing and Blending Level 1 §3.4.1: mix-blend-mode is initially `normal`.
            mix_blend_mode: MixBlendMode::Normal,
            // CSS Masking Level 1 §7.1: mask-image is initially `none`.
            mask_image: MaskImage::None,
            // CSS Masking Level 1 §5.1: clip-path is initially `none`.
            clip_path: ClipPath::None,
            // CSS Transforms Level 1 §4: transform is initially `none` (an empty list).
            transform: empty_transform_list(),
            transform_origin: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
                vertical: CssPositionOffset::Start(Length::Percent(50.0)),
            },
            transform_origin_z: Length::Px(0.0),
            // CSS Filter Effects Level 1 §5: filter is initially `none` (an empty list).
            filter: empty_filter_list(),
            // CSS Tables 3 §4: table-layout is initially `auto` and is not inherited.
            table_layout: TableLayoutValue::Auto,
            // CSS Overflow 3 §5.1: text-overflow is initially `clip` and is not inherited.
            text_overflow: TextOverflowValue::Clip,
            // CSS Tables 3 §6: border-collapse is initially `separate`. For nodes with a parent,
            // `Self::inherit_from` replaces it with the inherited value.
            border_collapse: BorderCollapseValue::Separate,
            // CSS Tables 3 §6.1: border-spacing is initially `0` on both axes (0px). For nodes
            // with a parent, `Self::inherit_from` replaces it with the inherited value.
            border_spacing: BorderSpacingValue {
                horizontal: Length::Px(0.0),
                vertical: Length::Px(0.0),
            },
            // CSS Tables 3 §7: caption-side is initially `top`. For nodes with a parent,
            // `Self::inherit_from` replaces it with the inherited value.
            caption_side: CaptionSideValue::Top,
            // CSS Tables 3 §8: empty-cells is initially `show`. For nodes with a parent,
            // `Self::inherit_from` replaces it with the inherited value.
            empty_cells: EmptyCellsValue::Show,
            column_count: ColumnCountValue::Auto,
            column_fill: ColumnFillValue::Balance,
            column_span: ColumnSpanValue::None,
            column_rule: INITIAL_BORDER,
            column_width: ColumnWidthValue::Auto,
        }
    }

    /// Build a child node's initial staging values from its parent's
    /// [`ComputedValues`].
    ///
    /// - Seed **inherited** properties from the parent's computed values.
    ///   Length-bearing `font_size`, `line_height`, `text_indent`,
    ///   `letter_spacing`, `word_spacing`, and `tab_size` are lifted to
    ///   specified representations by [`crate::resolve::lift_font_size`],
    ///   [`crate::resolve::lift_line_height`],
    ///   [`crate::resolve::lift_text_indent`],
    ///   [`crate::resolve::lift_letter_spacing`],
    ///   [`crate::resolve::lift_word_spacing`], and
    ///   [`crate::resolve::lift_tab_size`]. Since `Px` and `Percent` are fixed
    ///   points of absolutization, passing through phase 2/3 does not apply
    ///   the conversion twice (see each function's docs).
    /// - Set **non-inherited** properties to the values in [`Self::initial`].
    ///
    /// The field docs of [`ComputedValues`] define the canonical
    /// classification. When adding a property, update this function,
    /// [`Self::initial`], and [`Self::finalize`] together: the first two cover
    /// all fields, and the last handles any required absolutization.
    ///
    /// **This function is the only implementation of the classification.**
    /// The public [`ComputedValues::inherit_from`] is a thin wrapper around
    /// this function and [`Self::finalize`]; do not duplicate the classification.
    ///
    /// [`ComputedValues::inherit_from`]: crate::computed::ComputedValues::inherit_from
    ///
    /// **Do not seed inherited fields from [`Self::initial`].** See the D5
    /// invariant in the [`Self`] docs.
    pub fn inherit_from(parent: &ComputedValues) -> Self {
        // Initialize with an explicit struct literal. "Simplifying" this to `..Self::initial()`
        // would allocate and then drop a `font_family` Vec before cloning the parent's value
        // (this comment moved here from `ComputedValues::inherit_from`). Since `font_family`
        // became `Arc<Vec<FontFamilyName>>`, that particular allocation/drop no longer occurs:
        // `initial_font_family()` only increments the shared slot. The rule still stands
        // independently: enumerating fields makes additions easier to audit, and a future field
        // may again carry an owned heap allocation. This runs once per node, so needless
        // allocations would regress by O(N). **Do not replace the literal with
        // `..Self::initial()` just to remove 15 lines.**
        Self {
            // ── inherited: seed from the parent's computed values ───────────────
            color: parent.color,
            font_family: parent.font_family.clone(),
            // Losslessly lift a computed `<length>` to specified `Px`, a fixed point.
            font_size: lift_font_size(parent.font_size),
            // D5: `bolder` and `lighter` resolve against this value.
            font_weight: parent.font_weight,
            line_height: lift_line_height(parent.line_height),
            // CSS Lists 3 §3: both list-style longhands are inherited.
            list_style_type: parent.list_style_type.clone(),
            list_style_position: parent.list_style_position,
            list_style_image: parent.list_style_image.clone(),
            // Do not resolve `match-parent` here; simply copy it. After all winners are applied,
            // `finalize` or `finalize_as_root` resolves it using an explicit parent
            // `ComputedValues` (see "differs from D5" in the `Self` docs).
            text_align: parent.text_align,
            // CSS Text Decoration 4: text-decoration-skip-ink is inherited.
            text_decoration_skip_ink: parent.text_decoration_skip_ink,
            // CSS Text Decoration 4: text-decoration-skip-spaces is inherited.
            text_decoration_skip_spaces: parent.text_decoration_skip_spaces,
            // CSS Text 3 §8.2.1: hanging-punctuation is inherited.
            hanging_punctuation: parent.hanging_punctuation,
            // CSS Text 4: text-autospace is inherited.
            text_autospace: parent.text_autospace,
            // CSS Text 4: word-space-transform is inherited.
            word_space_transform: parent.word_space_transform,
            // CSS Text 4: text-spacing-trim is inherited.
            text_spacing_trim: parent.text_spacing_trim,
            // CSS Text 3 §6.2 / §6.1: both are inherited keywords copied directly.
            text_justify: parent.text_justify,
            text_align_last: parent.text_align_last,
            direction: parent.direction,
            // CSS Writing Modes 4 §3.2: writing-mode is inherited. Seed from
            // the parent's preserved computed keyword; `parent.writing_mode`
            // remains the separate renderer-facing horizontal-tb fallback.
            writing_mode: parent.cssom_writing_mode,
            ruby_position: parent.ruby_position,
            // CSS Text 3 §8.1: text-indent is inherited. Lift mixed calc
            // terms without reapplying the parent's em basis to the child.
            text_indent: lift_text_indent(parent.text_indent),
            text_indent_ch_factor: parent.text_indent_ch_factor,
            text_indent_ch_offset: parent.text_indent_ch_offset,
            text_indent_ch_font: parent.text_indent_ch_font.clone(),
            text_indent_ch_inherited: parent.text_indent_ch_inherited
                || parent.text_indent_ch_factor.is_some(),
            text_indent_hanging: parent.text_indent_hanging,
            text_indent_each_line: parent.text_indent_each_line,
            // CSS Fonts 4 §2.4: font-style is inherited.
            font_style: parent.font_style,
            font_kerning: parent.font_kerning,
            // CSS Fonts 4 §8.1: font-optical-sizing is inherited.
            font_optical_sizing: parent.font_optical_sizing,
            // CSS Fonts 4 §9.3: font-variant-emoji is inherited.
            font_variant_emoji: parent.font_variant_emoji,
            // CSS Fonts 4 §6.13: font-language-override is inherited.
            font_language_override: parent.font_language_override.clone(),
            // CSS Fonts 4 §6.4: font-variant-ligatures is inherited.
            font_variant_ligatures: parent.font_variant_ligatures,
            font_synthesis: parent.font_synthesis,
            font_variant_position: parent.font_variant_position,
            font_palette: parent.font_palette.clone(),
            font_variant_numeric: parent.font_variant_numeric,
            font_variant_east_asian: parent.font_variant_east_asian,
            font_variation_settings: parent.font_variation_settings.clone(),
            font_feature_settings: parent.font_feature_settings.clone(),
            // CSS Fonts Module Level 3 §6.6: font-variant-caps is inherited.
            font_variant_caps: parent.font_variant_caps,
            // CSS Text Module Level 3 §2.1: text-transform is inherited.
            text_transform: parent.text_transform,
            // CSS Writing Modes 3 §9.1: text-combine-upright is inherited.
            text_combine_upright: parent.text_combine_upright,
            // CSS Writing Modes 3 §5.1: text-orientation is inherited.
            text_orientation: parent.text_orientation,
            // CSS Display 3 §4: visibility is inherited.
            visibility: parent.visibility,
            // CSS Text 3 §5.1: word-break is inherited.
            word_break: parent.word_break,
            // CSS Text 3 §5.3: line-break is inherited.
            line_break: parent.line_break,
            // CSS Text 3 §5.4: overflow-wrap is inherited.
            overflow_wrap: parent.overflow_wrap,
            // CSS Text 3 letter-spacing and CSS Text 4 word-spacing are inherited.
            // Both preserve computed percentages and mixed calcs through their
            // corresponding lift helpers; the absolute layout fallback remains
            // separate in `ComputedValues::word_spacing`.
            letter_spacing: lift_letter_spacing(parent.letter_spacing_computed),
            letter_spacing_ch_factor: parent.letter_spacing_ch_factor,
            letter_spacing_ch_offset: parent.letter_spacing_ch_offset,
            letter_spacing_ch_font: parent.letter_spacing_ch_font.clone(),
            word_spacing: lift_word_spacing(parent.word_spacing_computed),
            word_spacing_ch_factor: parent.word_spacing_ch_factor,
            word_spacing_ch_offset: parent.word_spacing_ch_offset,
            word_spacing_ch_font: parent.word_spacing_ch_font.clone(),
            // CSS Text Module Level 3 §4.2: tab-size is inherited. Lift a computed `<number>` or
            // `<length>` into its specified representation, as for `lift_line_height`.
            tab_size: lift_tab_size(parent.tab_size),
            // CSS Text 3 §3: white-space is inherited.
            white_space: parent.white_space,
            // CSS Text 4 white-space-collapse is inherited.
            white_space_collapse: parent.white_space_collapse,
            text_wrap: parent.text_wrap,
            text_wrap_style: parent.text_wrap_style,
            // Both effective halves are inherited; a declaration on this
            // element overrides them in `apply_winners`.
            effective_white_space_collapse: parent.effective_white_space_collapse,
            effective_text_wrap_mode: parent.effective_text_wrap_mode,
            // CSS Tables 3 §6: border-collapse is an inherited keyword; copy it directly without
            // a lift, as for visibility.
            border_collapse: parent.border_collapse,
            // CSS Tables 3 §6.1: border-spacing is inherited. Lift the two computed lengths into
            // specified values, as for the `Length` arm of tab_size.
            border_spacing: lift_border_spacing(parent.border_spacing),
            // CSS Tables 3 §7: caption-side is an inherited keyword; copy it directly without a
            // lift, as for visibility.
            caption_side: parent.caption_side,
            // CSS Tables 3 §8: empty-cells is an inherited keyword; copy it directly without a
            // lift, as for visibility.
            empty_cells: parent.empty_cells,
            // CSS Multi-column Layout 1: all three multicol properties are non-inherited.
            column_count: ColumnCountValue::Auto,
            column_fill: ColumnFillValue::Balance,
            column_span: ColumnSpanValue::None,
            column_rule: INITIAL_BORDER,
            column_width: ColumnWidthValue::Auto,
            // CSS Text 3 §5.3: hyphens is inherited.
            hyphens: parent.hyphens,
            // CSS Text 4: hyphenate-character is inherited.
            hyphenate_character: parent.hyphenate_character.clone(),
            // CSS Text 4: hyphenate-limit-chars is inherited as computed values.
            hyphenate_limit_chars: parent.hyphenate_limit_chars,
            // CSS Content 3 §2.4.1: quotes is inherited; only increment the Arc reference count,
            // as for `ComputedValues::font_family`.
            quotes: parent.quotes.clone(),
            quotes_auto: parent.quotes_auto,
            // CSS Fragmentation Module Level 3 §3.3: both orphans and widows are inherited.
            orphans: parent.orphans,
            widows: parent.widows,
            // CSS Text Decoration Module Level 3 §4: text-shadow is inherited. Lift each item in
            // its `Arc<Vec<TextShadowItem>>` with `lift_text_shadow_item` (`Px` remains fixed).
            // Reuse the shared Arc slot for an empty list instead of allocating per node. Unlike
            // lifts such as `Self::text_indent`, this maps an entire list, so the empty-list
            // check follows the approach for other list properties such as content.
            text_shadow: if parent.text_shadow.is_empty() {
                empty_text_shadow_list()
            } else {
                Arc::new(
                    parent
                        .text_shadow
                        .iter()
                        .map(|c| lift_text_shadow_item(*c))
                        .collect(),
                )
            },
            // CSS Text Decoration 4 §2.8: text-underline-offset is inherited.
            text_underline_offset: match parent.text_underline_offset {
                crate::resolve::ComputedTextUnderlineOffset::Auto => TextUnderlineOffset::Auto,
                crate::resolve::ComputedTextUnderlineOffset::Length(value) => {
                    TextUnderlineOffset::Length(Length::Px(value.px()))
                }
                crate::resolve::ComputedTextUnderlineOffset::Percent(percent) => {
                    TextUnderlineOffset::Length(Length::Percent(percent))
                }
                crate::resolve::ComputedTextUnderlineOffset::Calc(value) => {
                    TextUnderlineOffset::Calc(crate::property::LengthPercentageCalc {
                        percent: value.percent,
                        px: value.px,
                        em: 0.0,
                        ch: 0.0,
                    })
                }
            },
            text_underline_position: parent.text_underline_position,
            text_emphasis_position: parent.text_emphasis_position,
            text_emphasis_style: parent.text_emphasis_style.clone(),
            text_emphasis_color: parent.text_emphasis_color,
            // ── non-inherited: use initial values ──────────────────────────────────
            // CSS Writing Modes 3 §2.2: unicode-bidi is non-inherited.
            unicode_bidi: UnicodeBidi::Normal,
            background_color: CssColor::TRANSPARENT,
            background_color_expression: None,
            display: DisplayValue::Inline,
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: Vec::new(),
            position: PositionValue::Static,
            padding: Sides::all(Length::Px(0.0)),
            margin: Sides::all(LengthOrAuto::Length(Length::Px(0.0))),
            border: Sides::all(INITIAL_BORDER),
            border_radius: BorderRadius {
                top_left: Length::Px(0.0).into(),
                top_right: Length::Px(0.0).into(),
                bottom_right: Length::Px(0.0).into(),
                bottom_left: Length::Px(0.0).into(),
            },
            box_shadow: empty_box_shadow_list(),
            outline: Outline {
                width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
                style: OutlineStyle::None,
                color: OutlineColor::Invert,
            },
            // CSS UI 3 §4.5: outline-offset is non-inherited, initially `0`.
            outline_offset: Length::Px(0.0),
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
            max_width: LengthOrAuto::Auto,
            max_height: LengthOrAuto::Auto,
            min_width: LengthOrAuto::Auto,
            min_height: LengthOrAuto::Auto,
            min_block_size: None,
            inline_size: None,
            preferred_size_precedence: PreferredSizePrecedence::default(),
            block_size: None,
            top: LengthOrAuto::Auto,
            right: LengthOrAuto::Auto,
            bottom: LengthOrAuto::Auto,
            left: LengthOrAuto::Auto,
            box_sizing: BoxSizing::ContentBox,
            // non-inherited (CSS Overflow 3 §3.1).
            overflow: OverflowXY::both(OverflowValue::Visible),
            // non-inherited (CSS Text Decoration Module Level 3 §2.1/§2.2/
            // §2.3, all "Inherited: no").
            text_decoration_line: TextDecorationLine::NONE,
            text_decoration_style: TextDecorationStyle::Solid,
            text_decoration_color: TextDecorationColor::CurrentColor,
            text_decoration_thickness: TextDecorationThickness::Auto,
            // non-inherited (CSS Text Decoration 4 §2.9.1 "Inherited: no").
            text_decoration_inset: TextDecorationInset::Lengths {
                start: Length::Px(0.0),
                end: Length::Px(0.0),
            },
            // non-inherited (CSS 2.1 §10.8.1 "Inherited: no").
            vertical_align: VerticalAlign::Baseline,
            // non-inherited (CSS2 §9.9.1 "Inherited: no").
            z_index: ZIndexValue::Auto,
            // non-inherited (CSS Fragmentation Module Level 3 §3.1 / §3.2
            // "Inherited: no").
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
            // `page` is non-inherited (CSS Paged Media 3 §8.1).
            page: PageValue::Auto,
            // non-inherited (CSS2 §9.5.1 / §9.5.2 "Inherited: no", both).
            float: FloatValue::None,
            clear: ClearValue::None,
            // non-inherited (CSS Flexible Box Layout Module Level 1 §5.1/
            // §5.2/§7.2.1/§7.2.2/§7.2.3, all "Inherited: no").
            flex_direction: FlexDirectionValue::Row,
            flex_wrap: FlexWrapValue::NoWrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: FlexBasisValue::Auto,
            order: 0,
            // non-inherited (CSS Box Alignment Module Level 3 §5.1/§5.1/
            // §7.2/§6.2, all "Inherited: no").
            justify_content: ContentAlignmentValue::Normal,
            align_content: ContentAlignmentValue::Normal,
            align_items: SelfAlignmentValue::Normal,
            align_self: AlignSelfValue::Auto,
            // non-inherited (CSS Box Alignment Module Level 3 §8.1,
            // "Inherited: no").
            row_gap: LengthOrNormal::Normal,
            column_gap: LengthOrNormal::Normal,
            // non-inherited (CSS Grid Layout Module Level 1 §7.2/§7.3/§7.6/
            // §7.7/§8.3, all "Inherited: no").
            grid_template_columns: GridTemplateTracks::None,
            grid_template_rows: GridTemplateTracks::None,
            grid_template_areas: GridTemplateAreasValue::None,
            grid_auto_columns: initial_grid_auto_track_list(),
            grid_auto_rows: initial_grid_auto_track_list(),
            grid_auto_flow: GridAutoFlowValue::Row,
            grid_row_start: GridLineValue::Auto,
            grid_row_end: GridLineValue::Auto,
            grid_column_start: GridLineValue::Auto,
            grid_column_end: GridLineValue::Auto,
            // non-inherited (CSS Box Alignment Module Level 3 §7.1/§6.1,
            // "Inherited: no").
            justify_items: SelfAlignmentValue::Normal,
            justify_self: AlignSelfValue::Auto,
            // non-inherited (CSS Backgrounds and Borders 3 §2.3/§2.4/§2.5/§2.6/
            // §2.7/§2.8/§2.9, all "Inherited: no") — child starts from spec
            // initial, same as `background_color` above.
            background_repeat: BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::Repeat,
            },
            background_attachment: BackgroundAttachment::Scroll,
            background_clip: VisualBox::BorderBox,
            background_origin: VisualBox::PaddingBox,
            background_size: BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            },
            background_position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
                vertical: CssPositionOffset::Start(Length::Percent(0.0)),
            },
            background_image: BackgroundImage::None,
            // non-inherited (CSS Images Module Level 3 §5.1/§5.2, both
            // "Inherited: no") — child starts from spec initial, same as
            // `background_repeat` above.
            object_fit: ObjectFit::Fill,
            object_position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
                vertical: CssPositionOffset::Start(Length::Percent(50.0)),
            },
            // non-inherited (CSS Color 4 §3.3 "Inherited: no").
            opacity: 1.0,
            // non-inherited (CSS Compositing and Blending Level 1 §3.4.2
            // "Inherited: no").
            isolation: Isolation::Auto,
            // non-inherited (CSS Compositing and Blending Level 1 §3.4.1
            // "Inherited: no").
            mix_blend_mode: MixBlendMode::Normal,
            // non-inherited (CSS Masking Level 1 §7.1 "Inherited: no").
            mask_image: MaskImage::None,
            // non-inherited (CSS Masking Level 1 §5.1 "Inherited: no").
            clip_path: ClipPath::None,
            // non-inherited (CSS Transforms Level 1 §4 "Inherited: no").
            transform: empty_transform_list(),
            transform_origin: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
                vertical: CssPositionOffset::Start(Length::Percent(50.0)),
            },
            transform_origin_z: Length::Px(0.0),
            // non-inherited (CSS Filter Effects Level 1 §5 "Inherited: no").
            filter: empty_filter_list(),
            // non-inherited (CSS Tables 3 §4 "Inherited: no"), initially
            // `auto`: without a winner, the child always reverts to this value, as for float.
            table_layout: TableLayoutValue::Auto,
            // non-inherited (CSS Overflow 3 §5.1), initially `clip`.
            text_overflow: TextOverflowValue::Clip,
            // Only table_layout and text_overflow belong in the non-inherited group. border_collapse is inherited
            // and was seeded in the inherited section above, like visibility.
        }
    }

    /// Absolutize a node **with a parent** into [`ComputedValues`]
    /// (**phase 2 → phase 3**).
    ///
    /// `parent` is the parent's [`ComputedValues`]. Its font size supplies the
    /// phase 2 reference; its `text_align` and `direction` resolve
    /// `match-parent` (see "Why the argument was widened" below for the change
    /// from `parent_font_size: ComputedLength` to `&ComputedValues`).
    /// `ctx.root_font_size` is the root element's computed font size. Use
    /// [`Self::finalize_as_root`] for the root itself: it has a different `rem`
    /// reference and the separate "computes to start" rule for `match-parent`.
    ///
    /// The parent provides the computed values needed for inherited
    /// properties such as `text-align: match-parent`. Lengths are resolved in
    /// the documented cascade phases: relative font-size values use the
    /// parent basis, while later relative values use this node's computed
    /// font size. The root uses [`Self::finalize_as_root`] because `rem` and
    /// `match-parent` have distinct root rules (CSS Text 3 §6.1).
    ///
    pub fn finalize(self, parent: &ComputedValues, ctx: &ResolveContext) -> ComputedValues {
        // Find the parent's line-height reference (CSS Values 4 §6.1.1 self-reference rule). Both
        // `lh` in phase 2 font-size and `lh` in phase 2.5 line-height use this same parent
        // reference. The parent is already resolved before this call in the parent-to-child tree
        // walk. Calculating its line height ahead of this node's font size does not violate phase
        // 2 → 2.5 order; only calculating **this node's own** line height before its own font
        // size would do that. See "The expected four stages" in the `mod@crate::resolve` module
        // docs.
        let parent_line_height_basis =
            used_line_height_length(parent.line_height, parent.font_size);
        // Phase 2: absolutize font-size against the **parent** (the parent-metrics clause of CSS
        // Values 4 §6.1.1). `lh` uses `parent_line_height_basis`; `rlh` uses the tree-global
        // `ctx.root_line_height`. See the self-reference section for `lh` / `rlh` in the
        // `resolve_font_size` docs.
        let font_size = resolve_font_size(
            self.font_size,
            parent.font_size,
            parent_line_height_basis,
            ctx,
        );
        // Resolve text-align: match-parent (CSS Text 3 §6.1) for a node with a parent. The root's
        // "computes to start" branch is in `finalize_as_root`.
        let text_align = resolve_text_align_internal_center(
            resolve_text_align_match_parent(self.text_align, parent.text_align, parent.direction),
            parent.text_align,
        );
        // Phase 2.5: absolutize line-height. The `resolve_line_height` docs define why
        // self-referential `lh` uses the parent, while `rlh` uses the tree-global
        // `ctx.root_line_height`. Do not use that reference for this node's padding and other
        // phase 3 values: `absolutize_with` calculates **this node's own** reference with
        // `used_line_height_length(line_height, font_size)`.
        let line_height =
            resolve_line_height(self.line_height, font_size, parent_line_height_basis, ctx);
        // Phase 3: absolutize everything else against **this node's** font size and line height.
        self.absolutize_with(font_size, line_height, text_align, ctx)
    }

    /// Absolutize a node **without a parent** (the root element).
    ///
    /// On the root element, `rem` uses **different** references in phases 2
    /// and 3. CSS Values 4 §6.1.1 "Font-relative Lengths"
    /// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) says:
    ///
    /// > When used in the value of any font-\* property **on the element they
    /// > refer to**, the font-relative lengths resolve against the computed
    /// > metrics of the parent element—or against the computed metrics
    /// > corresponding to the initial values of the font and line-height
    /// > properties, if the element has no parent.
    ///
    /// - **Phase 2** (`font-size`): this is a font-\* property, and by
    ///   definition `rem` refers to the root element (§6.1.1: "Equal to the
    ///   computed value of the em unit on the root element."
    ///   <https://www.w3.org/TR/css-values-4/#rem>). Thus `font-size: Nrem` on
    ///   the root uses a font-relative length on the very element it refers
    ///   to, triggering the rule above: use the **initial value (16px)**.
    ///   `em` also uses the initial value because there is no parent.
    /// - **Phase 3** (other properties): `padding` and other box properties
    ///   are not font-\* properties. The special rule does not apply, so `rem`
    ///   uses its ordinary definition: the **root's computed font size**,
    ///   which is its own font size from phase 2. `em` likewise uses the font
    ///   size of "the element on which it is used", consistent with the same
    ///   section's statement that "The other font-relative lengths continue
    ///   to resolve against the element's own metrics when used in
    ///   line-height."
    ///
    /// For `html { font-size: 20px; padding: 2rem }`, the font size is 20px
    /// and the padding is 40px, **not** 16px × 2 = 32px. This is checked by
    /// `rem_on_root_element_box_property_uses_own_font_size` under
    /// [`mod@crate::cascade`].
    ///
    /// Font-relative units in the value of `line-height` **itself** have
    /// the same distinction. As the same section says, "The other
    /// font-relative lengths continue to resolve against the element's own
    /// metrics when used in line-height." Thus `em` and similar units still
    /// use its own font size in phase 2.5, as in `finalize`.
    /// **`lh` / `rlh` are exceptions.** The [`resolve_line_height`] docs are
    /// canonical for the spec text and their differing behavior; we only
    /// summarize here because previous summaries have drifted. With no
    /// parent, `line-height: 1lh` / `1rlh` on the root always falls back to
    /// the "initial values" (`line-height: normal`). Without font metrics,
    /// `normal` cannot become an absolute length (the same obstacle as
    /// `cap` / `rcap`), so this self-reference remains unresolved.
    ///
    /// A **box property** on the root, such as `padding: 1rlh`, is not
    /// covered by that self-reference rule: in phase 3 it may use `rlh`'s
    /// ordinary definition ("the root element's `lh`"), referring to its
    /// **own** resolved line height. This function therefore determines the
    /// root line height in phase 2.5 before constructing
    /// [`ResolveContext::with_root_line_height`] (see the body below).
    ///
    /// # `text-align: match-parent` on the root element
    ///
    /// CSS Text 3 §6.1 `#valdef-text-align-match-parent` verbatim continues
    /// past the parent-direction clause with: "Computes to start when
    /// specified on the root element." This is a **different** rule from the
    /// [`Self::finalize`] branch — it does not consult any parent's
    /// `text_align` / `direction` at all, because there is no parent to
    /// consult. [`crate::property::resolve_text_align_match_parent`] only
    /// implements the has-a-parent half; this function implements the
    /// no-parent half locally, matching the same "which entry point runs"
    /// split the `rem` basis already uses below.
    ///
    /// # Why this takes no arguments, and its precondition
    ///
    /// Both phases determine their reference values inside this function, so
    /// callers cannot pass the wrong values. **This relies on callers using
    /// it only for nodes that truly have no parent.** Discarding a parent's
    /// computed font size and using the initial value is valid only under
    /// §6.1.1's "if the element has no parent" rule.
    /// [`crate::cascade::walk_from`] checks this invariant with a
    /// `debug_assert` (see its `None` arm). The same no-parent precondition
    /// applies when `text-align: match-parent` becomes `start`: Raikiri uses
    /// `root_ctx == None` to detect the absence of an element ancestor.
    /// In a synthetic DOM, several elements can each be "roots" in this
    /// sense (see the `walk_from` docs); that same criterion decides
    /// whether `match-parent` has a parent.
    ///
    /// Viewport-percentage lengths resolve against the default viewport of
    /// [`ResolveContext::initial`]; use [`Self::finalize_as_root_in_viewport`]
    /// to give the actual one.
    pub fn finalize_as_root(self) -> ComputedValues {
        let ctx = ResolveContext::initial();
        self.finalize_as_root_in_viewport(ctx.viewport_width, ctx.viewport_height)
    }

    /// [`Self::finalize_as_root`] with a `width` × `height` CSS px viewport,
    /// the basis of the viewport-percentage lengths.
    ///
    /// The viewport is the only reference value this function takes from its
    /// caller: unlike the font-relative bases, it does not come from the
    /// element tree.
    pub fn finalize_as_root_in_viewport(self, width: f32, height: f32) -> ComputedValues {
        // Phase 2: with no parent, use the initial values. The `em` / `rem` reference follows the
        // parent-metrics clause of CSS Values 4 §6.1.1 quoted above. **That clause does not cover
        // `<percentage>`**: its subject is "the font-relative lengths", not percentages. The 24px
        // result for `html { font-size: 150% }` instead follows by combining CSS Fonts 4 §2.5
        // <https://www.w3.org/TR/css-fonts-4/#font-size-prop> ("Percentages: refer to parent
        // element's font size") with the use of initial values when there is no parent. Neither
        // section says this result explicitly. The `FontSize` arm of
        // `cascade::resolve_against_inherited` makes the same distinction for pages. All three
        // units consequently use 16px here. For `lh`, the root has no parent, so its
        // self-reference is always `None` ("initial values" = `line-height: normal`, which cannot
        // be resolved), as with phase 2.5 below. `rlh` likewise sees `root_line_height: None`
        // from `ResolveContext::initial()`: `font-size: 1rlh` on the root refers to itself under
        // the ordinary definition of `rlh` (only the root can be its own referent; see the
        // `resolve_font_size` docs).
        let initial = ResolveContext::initial()
            .with_viewport(width, height)
            .with_vertical_root(self.writing_mode != WritingMode::HorizontalTb);
        let font_size = resolve_font_size(
            self.font_size,
            ComputedLength(crate::computed::INITIAL_FONT_SIZE_PX),
            None,
            &initial,
        );
        // CSS Text 3 §6.1: "Computes to start when specified on the root element." This special
        // case does not consult any parent text_align or direction and stays local to this
        // function (see the docs above).
        let text_align = match self.text_align {
            TextAlign::MatchParent | TextAlign::Inherit => TextAlign::Start,
            TextAlign::InternalCenter => TextAlign::Center,
            other => other,
        };
        // Phase 2.5: with no parent, `lh` / `rlh` in the value of `line-height` itself always use
        // the self-reference basis `None` ("initial values" = `normal`; see above). Other
        // font-relative units, such as `em`, still use this element's own font size through the
        // same `resolve_line_height` call as in `finalize`.
        let line_height = resolve_line_height(self.line_height, font_size, None, &initial);
        // Phase 3 context: `rem` uses the root element's computed font size (its own); `rlh`
        // similarly uses its resolved line height. Box properties are outside the self-reference
        // rule above. `used_line_height_length` derives the same value as the `child_ctx` that
        // `crate::cascade::walk_from` supplies to children.
        // `rlh_on_root_element_matches_child_root_line_height_basis` under `mod@crate::cascade`
        // checks that the two agree.
        let own_line_height = used_line_height_length(line_height, font_size);
        let ctx = ResolveContext::with_root_line_height(font_size, own_line_height)
            .with_viewport(width, height)
            .with_vertical_root(self.writing_mode != WritingMode::HorizontalTb);
        self.absolutize_with(font_size, line_height, text_align, &ctx)
    }

    /// Phase 3: absolutize remaining values against this node's resolved
    /// computed `font-size` and `line-height`.
    ///
    /// This function **intentionally takes no `parent`** (see guarantee 1 in
    /// the [`Self::finalize`] docs). Its caller ([`Self::finalize`] or
    /// [`Self::finalize_as_root`]) has already resolved `match-parent` in
    /// `text_align`; this function passes it through without duplicating the
    /// resolution logic. The callers have different branches, so moving that
    /// logic here would require root detection and weaken the locality
    /// described under "Why this takes no arguments" above.
    ///
    /// Similarly, `line_height` is **this node's** [`ComputedLineHeight`],
    /// resolved by the caller in phase 2.5. [`used_line_height_length`]
    /// converts it to the absolute `own_line_height` used to resolve `lh` on
    /// padding, margin, border, width and height. The caller computes it
    /// first (rather than calling `resolve_line_height` here) because
    /// `padding: 1lh` needs the **already resolved** line height of this
    /// node. Passing it in, like `font_size`, preserves that order.
    /// Fold `inline-size` / `block-size` into `width` / `height` and return
    /// their mapping for a vertical writing mode.
    ///
    /// Layout runs vertical content on horizontal axes except where it lays
    /// out vertical lines itself, so `width` / `height` keep the horizontal
    /// mapping. In `vertical-rl` / `vertical-lr`, the returned physical
    /// `(width, height)` follows CSS Logical Properties 1 §4.1
    /// (<https://www.w3.org/TR/css-logical-1/#dimension-properties>): the
    /// inline size is the physical height. A declared logical size replaces
    /// the physical size on the same axis, matching `min-block-size` below.
    fn map_logical_preferred_sizes(&mut self) -> Option<(LengthOrAuto, LengthOrAuto)> {
        let inline_size = self.inline_size.take();
        let block_size = self.block_size.take();
        let precedence = self.preferred_size_precedence;
        if inline_size.is_none()
            && block_size.is_none()
            && precedence.inline_size.is_none()
            && precedence.block_size.is_none()
        {
            return None;
        }
        // CSS Logical Properties 1 §4: a logical property and the physical
        // property it maps to share one computed value, so the one later in
        // cascade order wins.
        let pick = |physical: LengthOrAuto,
                    physical_rank: Option<CascadePrecedence>,
                    logical: Option<LengthOrAuto>,
                    logical_rank: Option<CascadePrecedence>| {
            // A winner invalid at computed-value time still takes its place
            // in cascade order and computes to the initial `auto`.
            if logical_rank.is_some() && logical_rank >= physical_rank {
                logical.unwrap_or(LengthOrAuto::Auto)
            } else if physical_rank.is_some() {
                physical
            } else {
                logical.unwrap_or(physical)
            }
        };
        let vertical = matches!(
            self.writing_mode,
            WritingMode::VerticalRl | WritingMode::VerticalLr
        )
        .then(|| {
            (
                pick(
                    self.width,
                    precedence.width,
                    block_size,
                    precedence.block_size,
                ),
                pick(
                    self.height,
                    precedence.height,
                    inline_size,
                    precedence.inline_size,
                ),
            )
        });
        self.width = pick(
            self.width,
            precedence.width,
            inline_size,
            precedence.inline_size,
        );
        self.height = pick(
            self.height,
            precedence.height,
            block_size,
            precedence.block_size,
        );
        vertical
    }

    fn absolutize_with(
        mut self,
        font_size: ComputedLength,
        line_height: ComputedLineHeight,
        text_align: TextAlign,
        ctx: &ResolveContext,
    ) -> ComputedValues {
        let vertical_logical_size = self.map_logical_preferred_sizes();
        // Reference for `1lh` on padding, margin, border, width and height. `rlh` uses the
        // tree-global `ctx.root_line_height`, so this local reference is only needed here.
        let own_line_height = used_line_height_length(line_height, font_size);
        let letter_spacing =
            resolve_letter_spacing_with_ch(self.letter_spacing, font_size, own_line_height, ctx);
        let letter_spacing_computed =
            resolve_letter_spacing(self.letter_spacing, font_size, own_line_height, ctx);
        let word_spacing =
            resolve_word_spacing_with_ch(self.word_spacing, font_size, own_line_height, ctx);
        let word_spacing_computed =
            resolve_word_spacing(self.word_spacing, font_size, own_line_height, ctx);
        let (text_indent, authored_text_indent_ch_factor) = match self.text_indent {
            TextIndentLength::Length(length) => {
                let resolved =
                    resolve_length_percentage_with_ch(length, font_size, own_line_height, ctx);
                let value = match resolved.value {
                    crate::resolve::ComputedLengthPercentage::Px(px) => ComputedTextIndent::Px(px),
                    crate::resolve::ComputedLengthPercentage::Percent(percent) => {
                        ComputedTextIndent::Percent(percent)
                    }
                };
                (value, resolved.ch_factor)
            }
            TextIndentLength::Calc(calc) => (
                resolve_text_indent_calc(calc, font_size),
                calc_ch_factor(calc),
            ),
        };
        // Font-metric `ch` provenance comes from a plain `Nch` length or from
        // a `ch` term inside a calc; an inherited value keeps the ancestor's.
        let text_indent_ch_factor = self
            .text_indent_ch_factor
            .or(authored_text_indent_ch_factor);
        // A calc authored on this node contributes its own absolute part; an
        // inherited one keeps the ancestor's, and a plain `Nch` has none.
        let text_indent_ch_offset = match self.text_indent {
            TextIndentLength::Calc(calc) if authored_text_indent_ch_factor.is_some() => {
                calc_ch_offset(calc, font_size)
            }
            _ if authored_text_indent_ch_factor.is_some() => 0.0,
            _ => self.text_indent_ch_offset,
        };
        let own_ch_font = || ChFontKey {
            family: self.font_family.clone(),
            size: font_size,
            weight: self.font_weight,
            style: self.font_style,
        };
        // A `ch` value declared on this node measures with its own font; one
        // inherited from an ancestor keeps measuring with the ancestor's font.
        let declaring_ch_font =
            |authored: Option<f32>, factor: Option<f32>, inherited: &Option<ChFontKey>| {
                if authored.is_some() {
                    Some(own_ch_font())
                } else if factor.is_some() {
                    inherited.clone()
                } else {
                    None
                }
            };
        let letter_spacing_ch_offset = match self.letter_spacing {
            LetterSpacingValue::Calc(calc) if letter_spacing.ch_factor.is_some() => {
                calc_ch_offset(calc, font_size)
            }
            _ if letter_spacing.ch_factor.is_some() => 0.0,
            _ => self.letter_spacing_ch_offset,
        };
        let letter_spacing_ch_font = declaring_ch_font(
            letter_spacing.ch_factor,
            self.letter_spacing_ch_factor,
            &self.letter_spacing_ch_font,
        );
        let word_spacing_ch_offset = match self.word_spacing {
            LetterSpacingValue::Calc(calc) if word_spacing.ch_factor.is_some() => {
                calc_ch_offset(calc, font_size)
            }
            _ if word_spacing.ch_factor.is_some() => 0.0,
            _ => self.word_spacing_ch_offset,
        };
        let word_spacing_ch_font = declaring_ch_font(
            word_spacing.ch_factor,
            self.word_spacing_ch_factor,
            &self.word_spacing_ch_font,
        );
        let text_indent_ch_font = if authored_text_indent_ch_factor.is_some() {
            Some(own_ch_font())
        } else if text_indent_ch_factor.is_some() {
            self.text_indent_ch_font.clone()
        } else {
            None
        };
        let ch_provenance = |length: Length| match length {
            Length::Ch(factor) if factor.is_finite() => Some(ChLengthProvenance {
                factor,
                font: own_ch_font(),
            }),
            _ => None,
        };
        let size_ch = |size: LengthOrAuto| match size {
            LengthOrAuto::Length(length) => ch_provenance(length),
            _ => None,
        };
        let vertical_logical_size =
            vertical_logical_size.map(|(width, height)| VerticalLogicalSize {
                width: resolve_length_percentage_or_auto(width, font_size, own_line_height, ctx),
                width_ch: size_ch(width),
                height: resolve_length_percentage_or_auto(height, font_size, own_line_height, ctx),
                height_ch: size_ch(height),
            });
        let width_ch = match self.width {
            LengthOrAuto::Length(length) => ch_provenance(length),
            _ => None,
        };
        let height_ch = match self.height {
            LengthOrAuto::Length(length) => ch_provenance(length),
            _ => None,
        };
        let padding_ch = self.padding.map(ch_provenance);
        let margin_ch = self.margin.map(|value| match value {
            LengthOrAuto::Length(length) => ch_provenance(length),
            _ => None,
        });
        let (text_decoration_inset_start_ch, text_decoration_inset_end_ch) =
            match self.text_decoration_inset {
                TextDecorationInset::Auto => (None, None),
                TextDecorationInset::Lengths { start, end } => {
                    (ch_provenance(start), ch_provenance(end))
                }
            };
        let min_width =
            resolve_length_percentage_or_auto(self.min_width, font_size, own_line_height, ctx);
        let min_height =
            resolve_length_percentage_or_auto(self.min_height, font_size, own_line_height, ctx);
        let physical_min_block = self
            .min_block_size
            .map(|value| resolve_length_percentage_or_auto(value, font_size, own_line_height, ctx));
        let (min_width, min_height) = match self.writing_mode {
            WritingMode::HorizontalTb => (min_width, physical_min_block.unwrap_or(min_height)),
            WritingMode::VerticalRl
            | WritingMode::VerticalLr
            | WritingMode::SidewaysRl
            | WritingMode::SidewaysLr => (physical_min_block.unwrap_or(min_width), min_height),
        };
        let max_width_ch = size_ch(self.max_width);
        let max_height_ch = size_ch(self.max_height);
        let min_block_size_ch = self.min_block_size.and_then(size_ch);
        let min_width_ch = size_ch(self.min_width);
        let min_height_ch = size_ch(self.min_height);
        let (min_width_ch, min_height_ch) = match self.writing_mode {
            WritingMode::HorizontalTb => (
                min_width_ch,
                if self.min_block_size.is_some() {
                    min_block_size_ch.clone()
                } else {
                    min_height_ch
                },
            ),
            WritingMode::VerticalRl
            | WritingMode::VerticalLr
            | WritingMode::SidewaysRl
            | WritingMode::SidewaysLr => (
                if self.min_block_size.is_some() {
                    min_block_size_ch.clone()
                } else {
                    min_width_ch
                },
                min_height_ch,
            ),
        };
        // CSS2 §9.7: same-node coupling forces the computed `display` value to depend on the
        // cascaded `float` value. See the `resolve_display_for_float` docs; this happens in
        // phase 3 like overflow cross-axis coupling.
        let display = resolve_display_for_float(self.display, self.float);
        let mut running_templates = std::mem::take(&mut self.running_templates);
        ComputedValues {
            color: self.color,
            background_color: self
                .background_color_expression
                .as_ref()
                .and_then(|source| crate::property::resolve_contextual_color(source, self.color))
                .unwrap_or(self.background_color),
            background_color_expression: self.background_color_expression,
            font_family: self.font_family,
            font_size,
            font_weight: self.font_weight,
            line_height,
            display: running_display(&mut running_templates, display),
            list_style_type: self.list_style_type,
            list_style_position: self.list_style_position,
            list_style_image: self.list_style_image,
            counter_reset: self.counter_reset,
            counter_increment: self.counter_increment,
            counter_set: self.counter_set,
            content: self.content,
            string_set: self.string_set,
            running_templates,
            position: self.position,
            // The caller has already resolved match-parent (see function docs).
            text_align,
            // CSS Text 3 §8.2.1: inherited keyword, no relative resolution.
            hanging_punctuation: self.hanging_punctuation,
            // CSS Text 4: inherited keyword/flag set, no relative resolution.
            text_autospace: self.text_autospace,
            // CSS Text 4: inherited keyword combination; no resolution required.
            word_space_transform: self.word_space_transform,
            // CSS Text 4: inherited keyword; computed value is specified value.
            text_spacing_trim: self.text_spacing_trim,
            // Pass this keyword through; no resolution needed.
            text_justify: self.text_justify,
            text_align_last: self.text_align_last,
            // The computed value is the specified value (see `Direction` docs). Pass through this
            // node's winner unchanged; no relative resolution.
            direction: self.direction,
            // Normalize `vertical-rl`, `vertical-lr`, `sideways-rl`, and `sideways-lr` to
            // `HorizontalTb` in the renderer-facing writing_mode field. Retain the CSSOM computed
            // keyword separately in cssom_writing_mode; this fallback affects layout only.
            writing_mode: resolve_writing_mode(self.writing_mode),
            cssom_writing_mode: self.writing_mode,
            ruby_position: self.ruby_position,
            // `text-indent` — same absolutization shape as `padding` (`%` is
            // passed through, `em`/`rem`/`pt`/etc. resolve against the own
            // `font_size`/`own_line_height` basis established above), but
            // this field is **inherited** — a child with no winner of its
            // own gets this value from `Self::inherit_from`'s
            // `lift_text_indent(parent.text_indent)` seed instead of
            // resetting to the initial `0` (`Self::padding` is
            // non-inherited and always resets, `Self::text_align` sibling
            // comment above shows the inherited counterpart pattern).
            text_indent,
            text_indent_ch_factor,
            text_indent_ch_offset,
            text_indent_ch_font,
            text_indent_ch_inherited: self.text_indent_ch_inherited,
            // Flags pass through untouched (no absolutization needed).
            text_indent_hanging: self.text_indent_hanging,
            text_indent_each_line: self.text_indent_each_line,
            padding: self
                .padding
                .map(|l| resolve_length_percentage(l, font_size, own_line_height, ctx)),
            padding_ch,
            // Although `margin` shares a type with `width` and `height`, it has a different
            // fallback when `Lh` / `Rlh` cannot be resolved (see Finding A in the
            // `resolve_margin_length_or_auto` docs).
            margin: self
                .margin
                .map(|l| resolve_margin_length_or_auto(l, font_size, own_line_height, ctx)),
            margin_ch,
            border: self
                .border
                .map(|b| resolve_border(b, font_size, own_line_height, ctx)),
            border_radius: resolve_border_radius(
                self.border_radius,
                font_size,
                own_line_height,
                ctx,
            ),
            box_shadow: if self.box_shadow.is_empty() {
                empty_computed_box_shadow_list()
            } else {
                Arc::new(
                    self.box_shadow
                        .iter()
                        .map(|item| resolve_box_shadow_item(*item, font_size, own_line_height, ctx))
                        .collect::<Vec<ComputedBoxShadowItem>>(),
                )
            },
            outline: resolve_outline(self.outline, font_size, own_line_height, ctx),
            outline_offset: resolve_length(self.outline_offset, font_size, own_line_height, ctx),
            width: resolve_length_percentage_or_auto(self.width, font_size, own_line_height, ctx),
            width_ch,
            height: resolve_length_percentage_or_auto(self.height, font_size, own_line_height, ctx),
            height_ch,
            max_width: resolve_length_percentage_or_auto(
                self.max_width,
                font_size,
                own_line_height,
                ctx,
            ),
            max_width_ch,
            max_height: resolve_length_percentage_or_auto(
                self.max_height,
                font_size,
                own_line_height,
                ctx,
            ),
            max_height_ch,
            min_width,
            min_width_ch,
            min_height,
            min_height_ch,
            min_block_size: physical_min_block,
            min_block_size_ch,
            vertical_logical_size,
            top: resolve_length_percentage_or_auto(self.top, font_size, own_line_height, ctx),
            right: resolve_length_percentage_or_auto(self.right, font_size, own_line_height, ctx),
            bottom: resolve_length_percentage_or_auto(self.bottom, font_size, own_line_height, ctx),
            left: resolve_length_percentage_or_auto(self.left, font_size, own_line_height, ctx),
            box_sizing: self.box_sizing,
            // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.7/§2.8: the computed values are
            // specified keywords with no lengths, like the TextDecorationLine arm below.
            background_repeat: self.background_repeat,
            background_attachment: self.background_attachment,
            background_clip: self.background_clip,
            background_origin: self.background_origin,
            // CSS Backgrounds and Borders 3 §2.9/§2.6: these contain `<length-percentage>`, so
            // absolutize them like width / height.
            background_size: resolve_background_size(
                self.background_size,
                font_size,
                own_line_height,
                ctx,
            ),
            background_position: resolve_css_position(
                self.background_position,
                font_size,
                own_line_height,
                ctx,
            ),
            // CSS Backgrounds and Borders 3 §2.3 / CSS Images 4 §3: `None` and `Url(String)` are
            // computed-equivalent. For `Gradient(..)`, absolutize font-relative parts of
            // `<length-percentage>` against this node's font_size / own_line_height. Percentages
            // need gradient box dimensions and pass through (see the `resolve_background_image`
            // docs); angles also pass through.
            background_image: resolve_background_image(
                self.background_image,
                font_size,
                own_line_height,
                ctx,
            ),
            // CSS Images Module Level 3 §5.1: the computed value is the specified keyword with no
            // lengths, like background_repeat.
            object_fit: self.object_fit,
            transform_origin: resolve_css_position(
                self.transform_origin,
                font_size,
                own_line_height,
                ctx,
            ),
            transform_origin_z: crate::resolve::resolve_length(
                self.transform_origin_z,
                font_size,
                own_line_height,
                ctx,
            ),
            // CSS Images Module Level 3 §5.2: this contains `<length-percentage>`, so absolutize
            // it like background_position using the same resolve_css_position function.
            object_position: resolve_css_position(
                self.object_position,
                font_size,
                own_line_height,
                ctx,
            ),
            // CSS Overflow 3 §3.1 cross-axis computed-value coupling
            // — same-node sibling dependency, resolved
            // here (phase 3) once both `overflow-x`/`overflow-y` winners are
            // known, mirroring the `border-*-style` -> `border-*-width` gate
            // a few fields up (`resolve_border`). See `resolve_overflow` doc.
            overflow: resolve_overflow(self.overflow),
            // The computed value is the specified keyword(s) or color; no relative resolution is
            // needed because these carry no lengths (see the TextDecorationLine,
            // TextDecorationStyle, and TextDecorationColor docs).
            text_decoration_line: self.text_decoration_line,
            text_decoration_style: self.text_decoration_style,
            text_decoration_color: self.text_decoration_color,
            text_decoration_thickness: resolve_text_decoration_thickness(
                self.text_decoration_thickness,
                font_size,
                own_line_height,
                ctx,
            ),
            // CSS Text Decoration 4: inherited keyword; no resolution required.
            text_decoration_skip_ink: self.text_decoration_skip_ink,
            // CSS Text Decoration 4: inherited keyword set; no resolution required.
            text_decoration_skip_spaces: self.text_decoration_skip_spaces,
            text_decoration_inset: resolve_text_decoration_inset(
                self.text_decoration_inset,
                font_size,
                own_line_height,
                ctx,
            ),
            text_decoration_inset_start_ch,
            text_decoration_inset_end_ch,
            text_underline_offset: resolve_text_underline_offset(
                self.text_underline_offset,
                font_size,
                own_line_height,
                ctx,
            ),
            text_underline_position: self.text_underline_position,
            text_emphasis_position: self.text_emphasis_position,
            // CSS Text Decoration 3 §3.1 resolves a fill-only mark against
            // typographic writing mode. Use the raw specified mode here,
            // before the renderer-facing computed mode is normalized.
            text_emphasis_style: match &self.text_emphasis_style {
                TextEmphasisStyle::DefaultShape { fill } => TextEmphasisStyle::Shape {
                    fill: *fill,
                    shape: match self.writing_mode {
                        WritingMode::HorizontalTb => TextEmphasisShape::Circle,
                        WritingMode::VerticalRl
                        | WritingMode::VerticalLr
                        | WritingMode::SidewaysRl
                        | WritingMode::SidewaysLr => TextEmphasisShape::Sesame,
                    },
                },
                style => style.clone(),
            },
            text_emphasis_color: self.text_emphasis_color,
            // The six keywords (`baseline`, `sub`, `super`, `middle`, `text-top`, `text-bottom`)
            // are already computed values. Absolutize VerticalAlign::Length and
            // VerticalAlign::Calc against this node's font_size / own_line_height (see
            // resolve_vertical_align); with `line-height: normal`, the percentage term is treated
            // as `0px`.
            vertical_align: resolve_vertical_align(
                self.vertical_align,
                font_size,
                own_line_height,
                ctx,
            ),
            // The computed value is the specified keyword (see FontStyle docs). No relative
            // resolution is needed because angle-bearing variants are unreachable within this
            // crate. Pass through this node's winner.
            font_style: self.font_style,
            font_kerning: self.font_kerning,
            // Computed value is the specified keyword; no relative resolution.
            font_optical_sizing: self.font_optical_sizing,
            // Computed value is the specified keyword; no rendering behavior.
            font_variant_emoji: self.font_variant_emoji,
            // Computed value is the specified string or keyword; no font selection.
            font_language_override: self.font_language_override.clone(),
            // Computed value is the specified keyword; no shaping behavior.
            font_variant_ligatures: self.font_variant_ligatures,
            font_synthesis: self.font_synthesis,
            font_variant_position: self.font_variant_position,
            font_palette: self.font_palette.clone(),
            font_variant_numeric: self.font_variant_numeric,
            font_variant_east_asian: self.font_variant_east_asian,
            // An inherited list is already in computed order, which the list
            // records, so it passes through shared without a walk over it.
            font_variation_settings: self.font_variation_settings.canonicalized(),
            font_feature_settings: self.font_feature_settings.canonicalized(),
            // The computed value is the specified keyword (see FontVariantCaps docs); with no
            // lengths, no relative resolution is needed. Pass through this node's winner.
            font_variant_caps: self.font_variant_caps,
            // The computed value is the specified keyword (see TextTransform docs); with no
            // lengths, no relative resolution is needed. Pass through this node's winner.
            text_transform: self.text_transform,
            // The computed value is the specified keyword (see TextCombineUpright docs).
            text_combine_upright: self.text_combine_upright,
            // The computed value is the specified keyword (see TextOrientation docs).
            text_orientation: self.text_orientation,
            // The computed value is the specified value (see UnicodeBidi docs).
            unicode_bidi: self.unicode_bidi,
            // The computed value is the specified keyword (see Visibility docs); with no lengths,
            // no relative resolution is needed. Pass through this node's winner.
            visibility: self.visibility,
            // The computed value is the specified value (see ZIndexValue docs); with no lengths,
            // no relative resolution is needed. Pass through this node's winner.
            z_index: self.z_index,
            // The computed value is the specified keyword (see WordBreak docs); with no lengths,
            // no relative resolution is needed. Pass through this node's winner.
            word_break: self.word_break,
            // computed value = specified keyword (`LineBreak` doc).
            line_break: self.line_break,
            // The computed value is the specified keyword (see OverflowWrap docs); with no
            // lengths, no relative resolution is needed. Pass through this node's winner. The
            // legacy `word-wrap` alias maps here too.
            overflow_wrap: self.overflow_wrap,
            // `1lh` in letter-spacing and word-spacing uses own_line_height like any box property
            // (CSS Text 3 §7.2 / §7.1 special-cases only the conversion of `normal` to `0`).
            letter_spacing: letter_spacing.value,
            letter_spacing_computed,
            // Preserve the `ch` provenance through inheritance for the
            // font-metric-aware text-layout consumer.
            letter_spacing_ch_factor: self.letter_spacing_ch_factor.or(letter_spacing.ch_factor),
            letter_spacing_ch_offset,
            letter_spacing_ch_font,
            word_spacing: word_spacing.value,
            word_spacing_computed,
            word_spacing_ch_factor: self.word_spacing_ch_factor.or(word_spacing.ch_factor),
            word_spacing_ch_offset,
            word_spacing_ch_font,
            // `1lh` in tab-size uses own_line_height like any box property (CSS Text Module Level
            // 3 §4.2 specifies no special line-height reference).
            tab_size: resolve_tab_size(self.tab_size, font_size, own_line_height, ctx),
            // The computed value is the specified keyword (see BreakBetween docs); with no
            // lengths, no relative resolution is needed. Pass through this node's winner.
            break_before: self.break_before,
            break_after: self.break_after,
            // The computed value is the specified keyword (see BreakInside docs); likewise, no
            // relative resolution is needed.
            break_inside: self.break_inside,
            // The computed values are the specified values (see FloatValue / ClearValue docs); no
            // lengths need resolution. Pass through this node's winners.
            // resolve_display_for_float handles their effect on display in the display assignment
            // above.
            float: self.float,
            clear: self.clear,
            // The computed value is the specified keyword (see WhiteSpace docs); with no lengths,
            // no relative resolution is needed. Pass through this node's winner.
            white_space: self.white_space,
            // CSS Text 4 white-space-collapse computes to the specified keyword.
            white_space_collapse: self.white_space_collapse,
            text_wrap: self.text_wrap,
            text_wrap_style: self.text_wrap_style,
            effective_white_space_collapse: self.effective_white_space_collapse,
            effective_text_wrap_mode: self.effective_text_wrap_mode,
            // The computed value is the specified keyword (see Hyphens docs); with no lengths, no
            // relative resolution is needed. Pass through this node's winner.
            hyphens: self.hyphens,
            // CSS Text 4 defines an inherited `auto | <string>` computed value.
            hyphenate_character: self.hyphenate_character.clone(),
            // This integer/`auto` triple is already in computed form.
            hyphenate_limit_chars: self.hyphenate_limit_chars,
            // The computed values are the specified keywords (see FlexDirectionValue /
            // FlexWrapValue docs); no lengths need resolution. Pass through this node's winners.
            flex_direction: self.flex_direction,
            flex_wrap: self.flex_wrap,
            // The computed value is the specified number (CSS Flexible Box Layout Module Level 1
            // §7.2.1/§7.2.2); `<number>` needs no absolutization. Pass through this node's
            // winners.
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            // Absolutize flex-basis like width / height (see resolve_flex_basis docs).
            flex_basis: resolve_flex_basis(self.flex_basis, font_size, own_line_height, ctx),
            // The computed value is the specified integer (CSS Flexible Box Layout Module Level 1
            // §4.2); `<integer>` needs no absolutization. Pass through this node's winner.
            order: self.order,
            // The computed values are the specified keywords (see ContentAlignmentValue /
            // SelfAlignmentValue / AlignSelfValue docs); with no lengths, no relative resolution
            // is needed.
            justify_content: self.justify_content,
            align_content: self.align_content,
            align_items: self.align_items,
            align_self: self.align_self,
            // Absolutize row-gap and column-gap similarly to padding, but retain the `normal`
            // keyword (see resolve_length_percentage_or_normal docs).
            row_gap: resolve_length_percentage_or_normal(
                self.row_gap,
                font_size,
                own_line_height,
                ctx,
            ),
            column_gap: resolve_length_percentage_or_normal(
                self.column_gap,
                font_size,
                own_line_height,
                ctx,
            ),
            // The computed value is the specified value (see ComputedValues::quotes docs); it has
            // no lengths to resolve. Pass through this node's winner or its inherited value.
            quotes: self.quotes,
            quotes_auto: self.quotes_auto,
            // `text-shadow` — each item's 3 lengths absolutized against this
            // node's own `font_size`/`own_line_height` basis
            // (`resolve_text_shadow_item`), `<color>` passed through
            // unchanged (no length). Empty list (`none`) reuses the shared
            // computed-layer empty Arc slot rather than allocating.
            text_shadow: if self.text_shadow.is_empty() {
                empty_computed_text_shadow_list()
            } else {
                Arc::new(
                    self.text_shadow
                        .iter()
                        .map(|item| {
                            resolve_text_shadow_item(*item, font_size, own_line_height, ctx)
                        })
                        .collect(),
                )
            },
            // Absolutize `<length-percentage>` in the entire track list of grid-template-columns
            // and grid-template-rows (see resolve_grid_template_tracks docs).
            grid_template_columns: resolve_grid_template_tracks(
                self.grid_template_columns,
                font_size,
                own_line_height,
                ctx,
            ),
            grid_template_rows: resolve_grid_template_tracks(
                self.grid_template_rows,
                font_size,
                own_line_height,
                ctx,
            ),
            // The computed value is the specified value (see GridTemplateAreasValue docs). The
            // spec calls it a "list of string values", with no `<length-percentage>` to
            // absolutize as in track sizing. Pass through this node's winner.
            grid_template_areas: self.grid_template_areas,
            // Apply the same track-size absolutization to grid-auto-columns / -rows as to
            // grid_template_columns.
            grid_auto_columns: resolve_grid_auto_track_list(
                &self.grid_auto_columns,
                font_size,
                own_line_height,
                ctx,
            ),
            grid_auto_rows: resolve_grid_auto_track_list(
                &self.grid_auto_rows,
                font_size,
                own_line_height,
                ctx,
            ),
            // The computed values are the specified keywords (see GridAutoFlowValue /
            // GridLineValue docs); with no lengths, no relative resolution is needed. Pass
            // through this node's winners.
            grid_auto_flow: self.grid_auto_flow,
            grid_row_start: self.grid_row_start,
            grid_row_end: self.grid_row_end,
            grid_column_start: self.grid_column_start,
            grid_column_end: self.grid_column_end,
            // The computed values are the specified keywords (see SelfAlignmentValue /
            // AlignSelfValue docs); no lengths need resolution.
            justify_items: self.justify_items,
            justify_self: self.justify_self,
            // The computed value is the specified integer (CSS Fragmentation Module Level 3
            // §3.3); with no lengths, no relative resolution is needed. Pass through this node's
            // winner or inherited value.
            orphans: self.orphans,
            widows: self.widows,
            // CSS Color 4 §3.3: "Opacity values outside the range `[0, 1]`
            // are not invalid, and are preserved in specified values, but
            // are clamped to the range `[0, 1]` in computed values." — the
            // one place this crate performs that clamp (`ComputedValues::opacity`
            // doc's "specified preserves, computed clamps" note). `f32::clamp`
            // correctly maps the `+Inf`/`-Inf` a huge literal (`opacity:
            // 1e40`/`opacity: -1e40`) can produce to `1.0`/`0.0` without
            // panicking (only NaN bounds panic, and neither bound here is
            // NaN). When `self` was built through the ordinary parse ->
            // cascade pipeline, `self.opacity` also never carries NaN by
            // the time it reaches here: `property.rs`'s numeric-token
            // acquisition already corrects the one cssparser artifact that
            // could otherwise produce it (a huge-*exponent* literal like
            // `opacity: 0e999`, `property.rs` module doc's "Numeric-token
            // NaN stabilization" section), and `parse_opacity_value`'s
            // `!is_nan()` guard — narrower than `is_finite()` specifically
            // so `+Inf`/`-Inf` still reach this clamp — remains as
            // defense-in-depth on top of that (`parse_opacity_value` doc's
            // "`!is_nan()` guard" section is canonical). But
            // `self.opacity` is a public field on a `pub fn` — a caller
            // that builds a `SpecifiedValues` directly and assigns `NaN`
            // here bypasses that parse-time guard entirely, and
            // `f32::clamp` passes a NaN `self` through unchanged (only a
            // NaN *bound* panics); this arm does not protect against that
            // direct-construction case, only against the pipeline's own
            // out-of-range values.
            opacity: self.opacity.clamp(0.0, 1.0),
            // CSS Compositing and Blending Level 1 §3.4.2: always a keyword; pass through without
            // a phase 3 transform, as for object_fit.
            isolation: self.isolation,
            // CSS Compositing and Blending Level 1 §3.4.1: same shape as
            // `isolation` above.
            mix_blend_mode: self.mix_blend_mode,
            // CSS Masking Level 1 §7.1: `None` and `Url(String)` are computed-equivalent. As for
            // background_image, phase 3 absolutizes font-relative parts of `<length-percentage>`
            // in `Gradient(..)` but leaves percentages unchanged (see resolve_background_image
            // docs).
            mask_image: resolve_background_image(self.mask_image, font_size, own_line_height, ctx),
            // CSS Masking Level 1 §5.1: same shape as `mask_image` above
            // (`ClipPath` doc's scope note).
            clip_path: self.clip_path,
            // CSS Transforms Level 1 §4: its computed value is "as specified, but with lengths
            // made absolute". Absolutize only the length part of `<length-percentage>` in
            // translate(), translateX(), and translateY(); leave the percentage part as Percent.
            // This is the same split as resolve_length_percentage and the same pattern as
            // resolve_css_position for background-position / object-position. Exclude the six
            // `<number>` slots of matrix() (already resolved) and the `<angle>` slots of rotate()
            // and skew() (the spec does not normalize angles; see `crate::property::Angle` docs).
            transform: if self.transform.is_empty() {
                crate::resolve::empty_computed_transform_list()
            } else {
                Arc::new(
                    self.transform
                        .iter()
                        .map(|f| resolve_transform_function(*f, font_size, own_line_height, ctx))
                        .collect(),
                )
            },
            // CSS Filter Effects Level 1 §5: unlike transform above, its computed value is
            // plainly "as specified", so the spec requires no absolutization here (see the "Range
            // restriction is reject, not clamp" section of the FilterFunction docs for this same
            // "as specified" fact in another context). Pass through unchanged.
            filter: self.filter,
            // The computed value is the specified keyword (see TableLayoutValue docs); no lengths
            // need resolution. Pass through this node's winner or the initial value used as its
            // non-inherited seed.
            table_layout: self.table_layout,
            text_overflow: self.text_overflow,
            // The computed value is the specified keyword (see BorderCollapseValue docs);
            // likewise, pass through this node's winner or the inherited parent value.
            border_collapse: self.border_collapse,
            // The computed value is two absolute lengths (see BorderSpacingValue docs).
            // Absolutize this node's winner or the lifted inherited value against this node's
            // font reference, as for the Length arm of tab_size.
            border_spacing: resolve_border_spacing(
                self.border_spacing,
                font_size,
                own_line_height,
                ctx,
            ),
            // The computed value is the specified keyword (see CaptionSideValue docs); no lengths
            // need resolution. Pass through this node's winner or inherited parent value.
            caption_side: self.caption_side,
            // The computed value is the specified keyword (see EmptyCellsValue docs); likewise,
            // pass through this node's winner or inherited parent value.
            empty_cells: self.empty_cells,
            // CSS Multi-column Layout 1: non-inherited count and fill pass through;
            // width is absolutized against the element's own font metrics.
            column_count: self.column_count,
            column_fill: self.column_fill,
            column_span: self.column_span,
            column_rule: {
                let mut rule = resolve_border(self.column_rule, font_size, own_line_height, ctx);
                // CSS Values 4: snap positive subpixel widths up and larger
                // widths down before centering the producer's rule rectangle.
                let width = rule.width.px();
                rule.width = ComputedLength(if width > 0.0 {
                    width.floor().max(1.0)
                } else {
                    0.0
                });
                rule
            },
            column_width: resolve_column_width(self.column_width, font_size, own_line_height, ctx),
            custom_properties: crate::computed::empty_custom_properties(),
            local_custom_properties: crate::computed::empty_custom_properties(),
        }
    }
}

/// The specified initial `border-*` value for one side.
///
/// CSS Backgrounds 3 defines width as `medium` (§3.3, "Line Thickness: the
/// border-width properties", <https://www.w3.org/TR/css-backgrounds-3/#border-width>).
/// The normative text says, "The thin, medium, and thick keywords are
/// equivalent to 1px, 3px, and 5px, respectively." Style is `none` (§3.2,
/// "Line Patterns: the border-style properties",
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>); color is
/// `currentcolor` (§3.1, "Line Colors: the border-color properties",
/// <https://www.w3.org/TR/css-backgrounds-3/#border-color>).
///
/// **The computed initial value differs**: with style `none`, gating in
/// [`resolve_border`] reduces the width to 0px (see
/// [`ComputedValues::initial`]).
///
/// This is `pub(crate)` because phase 3 of the page path
/// ([`crate::page::cascade_page`]) reads `.style` as the gating reference
/// when `border-*-style` is **undeclared**. CSS Paged Media 3 §6, "Page
/// Properties" (<https://www.w3.org/TR/css-page-3/#page-properties>), says
/// "both the page context and the margin context have a computed value for
/// every property". An undeclared property therefore uses its initial
/// computed value. Since `border-*-style` is non-inherited, this is the sole
/// source, rather than a value inherited from a parent. Sharing this constant
/// avoids duplicating the literal rule "initial border-style is `none`" in
/// page.rs. The width `3.0` comes from
/// [`crate::property::BORDER_WIDTH_MEDIUM_PX`], not another literal; see that
/// constant's docs for the single-source rationale.
pub(crate) const INITIAL_BORDER: Border = Border {
    width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
    style: BorderStyle::None,
    color: BorderColor::CurrentColor,
};

/// The computed `display` of an element with `position: running(<name>)`.
///
/// CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax>:
/// "The element is removed from the normal flow, and is available to place
/// in a page margin box using `element()`." The element therefore generates
/// no box where it stands, and the display it would have had is recorded on
/// its template for the margin-box layout. Elements without a running
/// template keep `display`.
fn running_display(templates: &mut [RunningTemplate], display: DisplayValue) -> DisplayValue {
    if templates.is_empty() {
        return display;
    }
    for template in templates.iter_mut() {
        template.display = display;
    }
    DisplayValue::None
}

#[cfg(test)]
mod tests;
