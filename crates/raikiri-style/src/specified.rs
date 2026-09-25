//! Cascade winner の **staging 表現** ([`SpecifiedValues`]) と、そこから
//! [`ComputedValues`] への絶対化 (phase 2 + phase 2.5 + phase 3 —
//! phase 2.5 は line-height の絶対化、後から追加)。
//!
//! # なぜ staging 表現が要るのか
//!
//! cascade winner (`PropertyValue`) が運ぶ length は **specified value** であり、
//! `em` / `rem` を含む。一方 [`ComputedValues`] は Phase 2 以降 **computed value 層**
//! の型だけを持つ。両者の間に位置し「winner を適用し終えたが、まだ絶対化して
//! いない」状態を表すのが本 module の [`SpecifiedValues`] である。
//!
//! この中間状態が独立に必要な理由は [`crate::resolve`] の module doc が述べる
//! とおり — `padding: 2em` の基準となる `font-size` は、その node の**全** winner を
//! 適用し終えた後にしか確定しない。winner を 1 つ適用するたびに絶対化する実装は
//! 適用順に依存してしまい成立しない (`font-size` が最後に適用されれば、それ以前に
//! 絶対化した `2em` は古い基準で焼き付いている)。**絶対化を winner 適用とは別
//! phase に分けることは必須の制約**である。
//!
//! なお cascade 段の winner 適用順そのものは **決定的**である —
//! `PropertyKey` discriminant を index にした slot 配列を昇順に走査するため。
//! 上の拘束は適用順の決定性とは独立に成り立つ: どの順に適用しようと
//! 「全 winner 適用後」でなければ基準 `font-size` は確定しない。

use std::sync::Arc;

use smol_str::SmolStr;

use crate::computed::{ChFontKey, ChLengthProvenance, ComputedValues, RunningTemplate};
use crate::property::{
    AlignSelfValue, BORDER_WIDTH_MEDIUM_PX, BackgroundAttachment, BackgroundImage,
    BackgroundRepeat, BackgroundRepeatKeyword, BackgroundSize, Border, BorderCollapseValue,
    BorderColor, BorderRadius, BorderSpacingValue, BorderStyle, BoxShadowItem, BoxSizing,
    BreakBetween, BreakInside, CaptionSideValue, ClearValue, ClipPath, ColumnCountValue,
    ColumnWidthValue, ContentAlignmentValue, ContentComponent, CssColor, CssPosition,
    CssPositionOffset, Direction, DisplayValue, EmptyCellsValue, FilterFunction, FlexBasisValue,
    FlexDirectionValue, FlexWrapValue, FloatValue, FontFamilyName, FontKerning,
    FontLanguageOverride, FontOpticalSizing, FontPaletteValue, FontStyle, FontSynthesisValue,
    FontVariantCaps, FontVariantEastAsian, FontVariantEmoji, FontVariantLigatures,
    FontVariantNumeric, FontVariantPosition, FontVariationSettings, GridAutoFlowValue,
    GridLineValue, GridTemplateAreasValue, GridTemplateTracks, GridTrackSize, HangingPunctuation,
    HyphenateCharacter, HyphenateLimitChars, Hyphens, Isolation, Length, LengthOrAuto,
    LengthOrNormal, LetterSpacingValue, LineBreak, LineHeight, ListStylePosition, ListStyleType,
    MaskImage, MixBlendMode, ObjectFit, Outline, OutlineColor, OutlineStyle, OverflowValue,
    OverflowWrap, OverflowXY, PageValue, PositionValue, RubyPosition, SelfAlignmentValue, Sides,
    TabSize, TableLayoutValue, TextAlign, TextAlignLast, TextAutospace, TextCombineUpright,
    TextDecorationColor, TextDecorationInset, TextDecorationLine, TextDecorationSkipInk,
    TextDecorationSkipSpaces, TextDecorationStyle, TextDecorationThickness, TextEmphasisHEdge,
    TextEmphasisPosition, TextEmphasisShape, TextEmphasisStyle, TextEmphasisVEdge,
    TextIndentLength, TextJustify, TextOrientation, TextShadowItem, TextSpacingTrim, TextTransform,
    TextUnderlineOffset, TextUnderlinePosition, TextWrapMode, TextWrapStyle, TransformFunction,
    UnicodeBidi, VerticalAlign, Visibility, VisualBox, WhiteSpace, WhiteSpaceCollapse, WordBreak,
    WordSpaceTransform, WordSpacingValue, WritingMode, ZIndexValue, empty_box_shadow_list,
    empty_content_list, empty_counter_entries, empty_filter_list, empty_quotes_entries,
    empty_string_set_entries, empty_text_shadow_list, empty_transform_list, initial_font_family,
    initial_grid_auto_track_list, resolve_display_for_float, resolve_overflow,
    resolve_text_align_internal_center, resolve_text_align_match_parent, resolve_writing_mode,
};
use crate::resolve::{
    ComputedBoxShadowItem, ComputedLength, ComputedLineHeight, ComputedTextIndent, ResolveContext,
    empty_computed_box_shadow_list, empty_computed_text_shadow_list, lift_border_spacing,
    lift_font_size, lift_letter_spacing, lift_line_height, lift_tab_size, lift_text_indent,
    lift_text_shadow_item, lift_word_spacing, resolve_background_image, resolve_background_size,
    resolve_border, resolve_border_radius, resolve_border_spacing, resolve_box_shadow_item,
    resolve_column_width, resolve_css_position, resolve_flex_basis, resolve_font_size,
    resolve_grid_auto_track_list, resolve_grid_template_tracks, resolve_length,
    resolve_length_percentage, resolve_length_percentage_or_auto,
    resolve_length_percentage_or_normal, resolve_length_percentage_with_ch, resolve_letter_spacing,
    resolve_letter_spacing_with_ch, resolve_line_height, resolve_margin_length_or_auto,
    resolve_outline, resolve_tab_size, resolve_text_decoration_inset,
    resolve_text_decoration_thickness, resolve_text_indent_calc, resolve_text_shadow_item,
    resolve_text_underline_offset, resolve_transform_function, resolve_vertical_align,
    resolve_word_spacing, resolve_word_spacing_with_ch, used_line_height_length,
};

/// Cascade winner を適用し終えたが、まだ絶対化していない per-node の値。
///
/// [`ComputedValues`] と同じ field 集合を持ち、**length を運ぶ field だけ**が
/// specified value 層の型 ([`Length`] / [`LengthOrAuto`] / [`LineHeight`] /
/// [`Border`]) のままになっている。
///
/// # property → 層の対応表
///
/// raikiri の cascade は property ごとに「どの層まで解決済か」が異なる。一律に
/// 「この struct は specified 型」と言えないので、対応表を型で表現したものが本
/// struct の field 型である:
///
/// | 層 | field |
/// |---|---|
/// | **specified 層のまま** (絶対化が phase 2 / phase 3 待ち) | `font_size` / `line_height` / `padding` / `margin` / `border` / `border_radius` / `box_shadow` / `outline` / `width` / `height` / `text_indent` / `text_decoration_inset` / `text_decoration_thickness` / `letter_spacing` / `word_spacing` / `tab_size` / `text_shadow` / `background_size` / `background_position` / `object_position` / `border_spacing` |
/// | **既に computed-equivalent** (絶対化する length を含まない) | `color` / `background_color` / `font_family` / `font_weight` / `display` / `list_style_type` / `list_style_position` / `counter_*` / `content` / `string_set` / `running_templates` / `text_align` / `direction` / `box_sizing` / `overflow` / `text_decoration_line` / `text_decoration_style` / `text_decoration_color` / `text_underline_position` / `text_emphasis_position` / `text_emphasis_style` / `text_emphasis_color` / `font_style` / `font_kerning` / `font_optical_sizing` / `font_variant_emoji` / `font_language_override` / `font_variant_ligatures` / `font_synthesis` / `font_variant_position` / `font_palette` / `font_variant_numeric` / `font_variant_east_asian` / `font_variant_caps` / `text_transform` / `text_combine_upright` / `text_orientation` / `unicode_bidi` / `visibility` / `z_index` / `word_break` / `overflow_wrap` / `break_before` / `break_after` / `break_inside` / `float` / `clear` / `white_space` / `white_space_collapse` / `hyphens` / `hyphenate_character` / `hyphenate_limit_chars` / `quotes` / `orphans` / `widows` / `background_repeat` / `background_attachment` / `background_clip` / `background_origin` / `background_image`\* / `object_fit` / `table_layout` / `border_collapse` / `caption_side` / `empty_cells` |
/// | **variant によって層が分かれる** (型は specified/computed で同じだが、一部 variant だけ絶対化を要る) | `vertical_align` — [`Self::vertical_align`] doc 参照 |
///
/// \* `background_image` は `None`/`Url(String)` の 2 variant では文字通り
/// この行の分類通りだが、`Gradient(..)` variant (CSS Images 4 §3) は
/// `<length-percentage>`/`<angle>` を含む — それでも表の分類上は「既に
/// computed-equivalent」に留める。`vertical_align` の行 (variant ごとに
/// phase 2/3 で解決 **される**) とは違い、`Gradient` の length/angle は
/// **どの phase でも絶対化されない** — 絶対化にはこの struct の scope 外の
/// 入力 (gradient box 自身の寸法) が要るため。詳細は
/// [`Self::background_image`] のフィールド doc。
///
/// `font_weight` が後者 (2 行目) にいるのは load-bearing な事実である —
/// `bolder` / `lighter` は [`crate::cascade::apply_value`] が**この struct へ書き込む
/// 時点で**親の computed weight に対して解決するので、`u16` で保持される
/// (下記「D5 invariant」節)。
///
/// `font_size` は「specified 層のまま」に留まるが、`larger` / `smaller`
/// (`<relative-size>`) は同じ D5 invariant を使って
/// **同じ書き込み時点**で親基準の絶対値に解決される — 結果は `Length::Px`
/// (specified 層の型としては通常の author px 指定と区別できない値) になるので
/// 表の分類は変わらない。詳細は下記「D5 invariant」節と
/// [`crate::cascade::apply_value`] の `FontSizeRelative` arm を参照。
///
/// page 経路には本 struct に相当する staging 型が無い —
/// [`crate::page::cascade_page`] は同じ 2 phase を `PropertyValue` の bag の上で
/// 直接走らせるので、中間状態は関数 local に閉じており public には出ない。
/// 両経路の phase 3 は [`crate::resolve`] の同じ関数群へ funnel する。
///
/// # D5 invariant — inherited field は**親の computed 値**で seed すること
///
/// [`Self::inherit_from`] は inherited property を親の [`ComputedValues`] から
/// seed する。これは単なる効率の話ではなく **正しさの要求**である:
/// [`crate::cascade::apply_value`] の `PropertyValue::FontWeight` arm は、
/// `bolder` / `lighter` を親の computed font-weight を基準に解決する。
/// [`Self::initial`] から seed すると、この基準を失う。
///
/// `font_size` follows the same rule: [`Self::inherit_from`] seeds it from
/// the parent's computed value through [`crate::resolve::lift_font_size`],
/// while the root uses the initial value.
///
/// # `text_align: match-parent` は D5 と**同型ではない**
///
/// `text-align: match-parent` も継承元依存の解決を要する点は `bolder` /
/// `lighter` と同じだが、D5 の read-before-write パターンは**使わない**。
/// D5 が安全なのは `apply_value` の `FontWeight` arm が自 field
/// (`self.font_weight`) だけを読み書きするからである。`match-parent` の解決は
/// **他 property (`direction`) の親の値**を要するため、同じ trick を使うと
/// 「自 node が `direction` winner を持つ場合、その適用順序次第で親ではなく
/// **自分の** direction を読んでしまう」レースが生まれる
/// ([`crate::cascade::resolve_inheritance`] の「winner の適用順に依存しない」
/// invariant への違反)。したがって本 struct の `text_align` field は D5 と
/// 同じく素朴に親からコピーするだけ ([`Self::inherit_from`] 参照) —
/// `match-parent` の解決は [`Self::finalize`] / [`Self::finalize_as_root`] が
/// 全 winner 適用**後**に、明示的に親の [`ComputedValues`] を受け取って行う
/// ([`crate::property::resolve_text_align_match_parent`] のドキュメント参照)。
///
/// # `#[non_exhaustive]`
///
/// [`ComputedValues`] と同じ判断 — future property の追加を source 互換にする。
/// crate 外からの構築は [`Self::initial`] / [`Self::inherit_from`] を使う。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct SpecifiedValues {
    /// [`ComputedValues::color`] の staging。層は computed-equivalent。
    pub color: CssColor,
    /// [`ComputedValues::background_color`] の staging。層は computed-equivalent。
    pub background_color: CssColor,
    /// [`ComputedValues::font_family`] の staging。層は computed-equivalent。
    pub font_family: Arc<Vec<FontFamilyName>>,
    /// `font-size` の **specified** value。phase 2 ([`resolve_font_size`]) で
    /// **親の** computed font-size を基準に絶対化される。
    pub font_size: Length,
    /// [`ComputedValues::font_weight`] の staging。**既に computed-equivalent**
    /// — 上記 D5 invariant を参照。型は `f32` (`u16` から格上げ、fractional
    /// weight を保持する)。
    pub font_weight: f32,
    /// `line-height` の **specified** value。phase 3 ([`resolve_line_height`]) で
    /// 自 node の computed font-size を基準に絶対化される。
    pub line_height: LineHeight,
    /// [`ComputedValues::display`] の staging。層は computed-equivalent。
    pub display: DisplayValue,
    /// [`ComputedValues::list_style_type`] の staging。inherited。
    pub list_style_type: ListStyleType,
    /// [`ComputedValues::list_style_position`] の staging。inherited。
    pub list_style_position: ListStylePosition,
    /// [`ComputedValues::list_style_image`] の staging。inherited。
    pub list_style_image: BackgroundImage,
    /// [`ComputedValues::counter_reset`] の staging。層は computed-equivalent。
    pub counter_reset: Arc<Vec<(SmolStr, i32)>>,
    /// [`ComputedValues::counter_increment`] の staging。層は computed-equivalent。
    pub counter_increment: Arc<Vec<(SmolStr, i32)>>,
    /// [`ComputedValues::counter_set`] の staging。層は computed-equivalent。
    pub counter_set: Arc<Vec<(SmolStr, i32)>>,
    /// [`ComputedValues::content`] の staging。層は computed-equivalent。
    pub content: Arc<Vec<ContentComponent>>,
    /// [`ComputedValues::string_set`] の staging。層は computed-equivalent。
    pub string_set: Arc<Vec<(SmolStr, Vec<ContentComponent>)>>,
    /// [`ComputedValues::running_templates`] の staging。層は computed-equivalent。
    pub running_templates: Vec<RunningTemplate>,
    /// `position` の staging — running_templates とは別に position keyword を保持 (relative 判定用)。
    pub position: PositionValue,
    /// [`ComputedValues::text_align`] の staging。層は computed-equivalent。
    /// `match-parent` はここでは**解決されない** — [`Self`] doc の
    /// "`text_align: match-parent` は D5 と同型ではない" 節参照。
    pub text_align: TextAlign,
    /// [`ComputedValues::hanging_punctuation`] staging. Inherited keyword;
    /// the line-layout consumer applies the implemented `first` subset.
    pub hanging_punctuation: HangingPunctuation,
    /// [`ComputedValues::text_autospace`](crate::computed::ComputedValues::text_autospace)
    /// の staging。keyword/flag set は computed-equivalent、inherited。
    pub text_autospace: TextAutospace,
    /// [`ComputedValues::word_space_transform`](crate::computed::ComputedValues::word_space_transform)
    /// の staging。inherited、initial `none`。値は specified のまま保持する。
    pub word_space_transform: WordSpaceTransform,
    /// [`ComputedValues::text_spacing_trim`](crate::computed::ComputedValues::text_spacing_trim)
    /// の staging。inherited; computed value is the specified keyword.
    pub text_spacing_trim: TextSpacingTrim,
    /// [`ComputedValues::text_justify`](crate::computed::ComputedValues::text_justify)
    /// の staging。keyword のため computed-equivalent、inherited。
    pub text_justify: TextJustify,
    /// [`ComputedValues::text_align_last`](crate::computed::ComputedValues::text_align_last)
    /// の staging。keyword のため computed-equivalent、inherited。
    pub text_align_last: TextAlignLast,
    /// [`ComputedValues::direction`] の staging。層は computed-equivalent。
    pub direction: Direction,
    /// [`ComputedValues::writing_mode`] の staging。**層は computed-equivalent
    /// ではない** — この field は 5 keyword とも specified 値をそのまま保持する
    /// (spec fidelity)。`vertical-rl`/`vertical-lr`/`sideways-rl`/`sideways-lr`
    /// → [`WritingMode::HorizontalTb`] の正規化は [`Self::absolutize_with`] が
    /// [`resolve_writing_mode`] 経由で行う (`text_align: match-parent` が
    /// [`Self`] でなく `finalize`/`absolutize_with` 側で解決されるのと同じ
    /// 「staging はまだ解決しない」形、[`WritingMode`] doc の Non-goal 節参照)。
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
    /// Source font for an inherited `ch` value.
    pub text_indent_ch_font: Option<ChFontKey>,
    /// Whether this value was inherited from an ancestor's `text-indent: ch`.
    pub text_indent_ch_inherited: bool,
    /// `text-indent`'s `hanging` flag staging. Inherited, initial `false`.
    pub text_indent_hanging: bool,
    /// `text-indent`'s `each-line` flag staging. Inherited, initial `false`.
    pub text_indent_each_line: bool,
    /// `padding` の **specified** value。phase 3
    /// ([`resolve_length_percentage`]) で絶対化される (percentage は素通し)。
    pub padding: Sides<Length>,
    /// `margin` の **specified** value。phase 3
    /// ([`resolve_margin_length_or_auto`]) で絶対化される。
    pub margin: Sides<LengthOrAuto>,
    /// `border` の **specified** value。phase 3 ([`resolve_border`]) で
    /// width が絶対化され、`none` / `hidden` の style gating も適用される。
    pub border: Sides<Border>,
    /// `border-radius` の **specified** value。phase 3 で四隅の length を
    /// 自 node の font-size / line-height 基準へ絶対化する。
    pub border_radius: BorderRadius,
    /// `box-shadow` の **specified** value。phase 3 で各 shadow の length を
    /// 絶対化する。property は non-inherited。
    pub box_shadow: Arc<Vec<BoxShadowItem>>,
    /// `outline` の **specified** value。phase 3 で width を絶対化し、
    /// `outline-style: none` の場合は computed width を 0 にする。
    pub outline: Outline,
    /// `outline-offset` の **specified** value。phase 3 で絶対化される
    /// (CSS UI 3 §4.5 <https://www.w3.org/TR/css-ui-3/#outline-offset>、
    /// initial `0`、non-inherited、`<length>` — 負値も受理)。
    pub outline_offset: Length,
    /// `width` の **specified** value。phase 3 で絶対化される。
    pub width: LengthOrAuto,
    /// `height` の **specified** value。phase 3 で絶対化される。
    pub height: LengthOrAuto,
    /// `max-width` の **specified** value。phase 3 で絶対化される。
    pub max_width: LengthOrAuto,
    /// `max-height` の **specified** value。phase 3 で絶対化される。
    pub max_height: LengthOrAuto,
    /// `min-width` の **specified** value。phase 3 で絶対化される。
    pub min_width: LengthOrAuto,
    /// `min-height` の **specified** value。phase 3 で絶対化される。
    pub min_height: LengthOrAuto,
    /// `min-block-size` の **specified** value。phase 3 で絶対化し、
    /// `writing-mode` に応じて physical min-width/min-height へ写像する。
    pub min_block_size: Option<LengthOrAuto>,
    /// `top` の **specified** value。phase 3 で絶対化される。
    pub top: LengthOrAuto,
    /// `right` の **specified** value。phase 3 で絶対化される。
    pub right: LengthOrAuto,
    /// `bottom` の **specified** value。phase 3 で絶対化される。
    pub bottom: LengthOrAuto,
    /// `left` の **specified** value。phase 3 で絶対化される。
    pub left: LengthOrAuto,
    /// [`ComputedValues::box_sizing`] の staging。層は computed-equivalent。
    pub box_sizing: BoxSizing,
    /// [`ComputedValues::overflow`] の staging。層は computed-equivalent
    /// (`OverflowValue` は length を運ばない) だが、cross-axis の
    /// computed-value coupling は**ここでは解決されない** — [`Self`] doc の
    /// "`text_align: match-parent` は D5 と同型ではない" 節と同じ理由で、
    /// [`resolve_overflow`] は phase 3 ([`Self::absolutize_with`]) が呼ぶ。
    pub overflow: OverflowXY,
    /// [`ComputedValues::text_decoration_line`] の staging。層は
    /// computed-equivalent (`TextDecorationLine` は length を運ばない)。
    pub text_decoration_line: TextDecorationLine,
    /// [`ComputedValues::text_decoration_style`] の staging。層は
    /// computed-equivalent (`TextDecorationStyle` は length を運ばない)。
    pub text_decoration_style: TextDecorationStyle,
    /// [`ComputedValues::text_decoration_color`] の staging。層は
    /// computed-equivalent (`TextDecorationColor` は length を運ばない、
    /// currentcolor の used-value resolution は paint scope 責務)。
    pub text_decoration_color: TextDecorationColor,
    /// [`ComputedValues::text_decoration_thickness`] の staging。`<length>` は
    /// phase 3 で自 node の font-size / line-height 基準へ絶対化する。
    pub text_decoration_thickness: TextDecorationThickness,
    /// [`ComputedValues::text_decoration_skip_ink`] の staging。inherited、
    /// keyword-only、computed-equivalent。
    pub text_decoration_skip_ink: TextDecorationSkipInk,
    /// [`ComputedValues::text_decoration_skip_spaces`] の staging。inherited、
    /// keyword-set-only、computed-equivalent。
    pub text_decoration_skip_spaces: TextDecorationSkipSpaces,
    /// [`ComputedValues::text_decoration_inset`] の staging。`em`/`rem` 等の
    /// length は phase 3 で自 node の font-size / line-height 基準へ絶対化する。
    pub text_decoration_inset: TextDecorationInset,
    /// [`ComputedValues::text_underline_offset`] の staging。inherited の
    /// length は phase 3 で declaring node の font-size 基準へ絶対化し、
    /// percentage は相対値のまま computed へ運ぶ。
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
    /// [`ComputedValues::vertical_align`] の staging。**型は
    /// [`ComputedValues::vertical_align`] と同じ** [`VerticalAlign`] だが、
    /// 層は field 一律ではない — `baseline`/`sub`/`super`/`middle`/
    /// `text-top`/`text-bottom` の 6 keyword は computed-equivalent (length
    /// を運ばない) な一方、[`VerticalAlign::Length`] / [`VerticalAlign::Calc`]
    /// は specified 層のまま (`em`/`rem` 等または mixed px/% を保持、絶対化は
    /// [`Self::absolutize_with`] の [`crate::resolve::resolve_vertical_align`]
    /// 呼び出しに委ねる) — 型を
    /// 分けない理由は同関数 doc 参照。
    pub vertical_align: VerticalAlign,
    /// [`ComputedValues::font_style`] の staging。層は computed-equivalent
    /// (`FontStyle` は length を運ばない — この crate の scope では)。
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
    /// [`ComputedValues::font_variant_caps`] の staging。層は
    /// computed-equivalent (`FontVariantCaps` は length を運ばない)。
    pub font_variant_caps: FontVariantCaps,
    /// [`ComputedValues::text_transform`] の staging。層は computed-equivalent
    /// (`TextTransform` は length を運ばない)。
    pub text_transform: TextTransform,
    /// [`ComputedValues::text_combine_upright`] の staging。層は computed-equivalent
    /// (`TextCombineUpright` は length を運ばない)。
    pub text_combine_upright: TextCombineUpright,
    /// [`ComputedValues::text_orientation`] の staging。層は computed-equivalent
    /// (`TextOrientation` は length を運ばない)。
    pub text_orientation: TextOrientation,
    /// [`ComputedValues::unicode_bidi`] の staging。層は computed-equivalent
    /// (`UnicodeBidi` は length を運ばない)。
    pub unicode_bidi: UnicodeBidi,
    /// [`ComputedValues::visibility`] の staging。層は computed-equivalent
    /// (`Visibility` は length を運ばない)。
    pub visibility: Visibility,
    /// [`ComputedValues::z_index`] の staging。層は computed-equivalent
    /// (`ZIndexValue` は length を運ばない)。
    pub z_index: ZIndexValue,
    /// [`ComputedValues::word_break`] の staging。層は computed-equivalent
    /// (`WordBreak` は length を運ばない)。
    pub word_break: WordBreak,
    /// [`ComputedValues::line_break`] の staging。層は computed-equivalent。
    pub line_break: LineBreak,
    /// [`ComputedValues::overflow_wrap`] の staging。層は computed-equivalent
    /// (`OverflowWrap` は length を運ばない)。`word-wrap` legacy alias もこの
    /// 同じ field に落ちる ([`ComputedValues::overflow_wrap`] doc 参照)。
    pub overflow_wrap: OverflowWrap,
    /// `letter-spacing` の **specified** value。phase 3
    /// ([`crate::resolve::resolve_letter_spacing`]) で computed CSS value と
    /// renderer-facing absolute fallback に解決される。
    pub letter_spacing: LetterSpacingValue,
    /// Authored `ch` factor retained through inheritance so the text-layout
    /// sink can replace the style fallback with a font metric.
    pub letter_spacing_ch_factor: Option<f32>,
    /// Font that declared an inherited `ch` [`Self::letter_spacing_ch_factor`],
    /// so descendants measure it with that font rather than their own.
    pub letter_spacing_ch_font: Option<ChFontKey>,
    /// `word-spacing` の **specified** value。CSS Text 4 length-percentage
    /// grammar is preserved through phase 3 for CSSOM computed-value exposure.
    pub word_spacing: WordSpacingValue,
    /// Authored `ch` factor retained through inheritance so the text-layout
    /// sink can replace the style-layer fallback with a font metric.
    pub word_spacing_ch_factor: Option<f32>,
    /// Font that declared an inherited `ch` [`Self::word_spacing_ch_factor`].
    pub word_spacing_ch_font: Option<ChFontKey>,
    /// `tab-size` の **specified** value。phase 3 ([`resolve_tab_size`]) で
    /// 自 node の computed font-size を基準に絶対化される (`<length>` 側
    /// のみ — `<number>` は絶対化不要、[`Self::flex_grow`] と同じ扱い)。
    pub tab_size: TabSize,
    /// [`ComputedValues::break_before`] の staging。層は computed-equivalent
    /// (`BreakBetween` は length を運ばない)。
    pub break_before: BreakBetween,
    /// [`ComputedValues::break_after`] の staging。層は computed-equivalent
    /// (`BreakBetween` は length を運ばない)。
    pub break_after: BreakBetween,
    /// [`ComputedValues::break_inside`] の staging。層は computed-equivalent
    /// (`BreakInside` は length を運ばない)。
    pub break_inside: BreakInside,
    /// `page` is non-inherited and selects the page type for the box that
    /// establishes the next class-A break point (CSS Paged Media 3 §8.1).
    pub page: PageValue,
    /// [`ComputedValues::float`] の staging。層は computed-equivalent
    /// (`FloatValue` は length を運ばない)。
    pub float: FloatValue,
    /// [`ComputedValues::clear`] の staging。層は computed-equivalent
    /// (`ClearValue` は length を運ばない)。
    pub clear: ClearValue,
    /// [`ComputedValues::white_space`] の staging。層は computed-equivalent
    /// (`WhiteSpace` は length を運ばない)。
    pub white_space: WhiteSpace,
    /// [`ComputedValues::white_space_collapse`] staging. Computed-equivalent:
    /// this keyword enum carries no lengths.
    pub white_space_collapse: WhiteSpaceCollapse,
    /// `text-wrap` wrapping component staging. Inherited, initial `wrap`.
    pub text_wrap: TextWrapMode,
    /// `text-wrap-style` staging. Inherited, initial `auto`.
    pub text_wrap_style: TextWrapStyle,
    /// [`ComputedValues::hyphens`] の staging。層は computed-equivalent
    /// (`Hyphens` は length を運ばない)。
    pub hyphens: Hyphens,
    /// [`ComputedValues::hyphenate_character`] の staging. Inherited string/keyword.
    pub hyphenate_character: HyphenateCharacter,
    /// [`ComputedValues::hyphenate_limit_chars`] の staging. Inherited computed triple.
    pub hyphenate_limit_chars: HyphenateLimitChars,
    /// [`ComputedValues::flex_direction`] の staging。層は computed-equivalent
    /// (`FlexDirectionValue` は length を運ばない)。
    pub flex_direction: FlexDirectionValue,
    /// [`ComputedValues::flex_wrap`] の staging。層は computed-equivalent
    /// (`FlexWrapValue` は length を運ばない)。
    pub flex_wrap: FlexWrapValue,
    /// [`ComputedValues::flex_grow`] の staging。層は computed-equivalent
    /// (`<number>` は絶対化不要 — [`ComputedValues::flex_grow`] doc 参照)。
    pub flex_grow: f32,
    /// [`ComputedValues::flex_shrink`] の staging。層は computed-equivalent。
    pub flex_shrink: f32,
    /// `flex-basis` の **specified** value。phase 3 ([`resolve_flex_basis`]) で
    /// 絶対化される (`auto`/`content` keyword は保持、`<length-percentage>`
    /// のみ絶対化) — [`Self::width`] と同じ絶対化 shape。
    pub flex_basis: FlexBasisValue,
    /// [`ComputedValues::order`] の staging。層は computed-equivalent
    /// (`<integer>` は絶対化不要 — [`Self::flex_grow`] と同じ扱い)。
    pub order: i32,
    /// [`ComputedValues::justify_content`] の staging。層は computed-equivalent
    /// (`ContentAlignmentValue` は length を運ばない)。
    pub justify_content: ContentAlignmentValue,
    /// [`ComputedValues::align_content`] の staging。層は computed-equivalent。
    pub align_content: ContentAlignmentValue,
    /// [`ComputedValues::align_items`] の staging。層は computed-equivalent
    /// (`SelfAlignmentValue` は length を運ばない)。
    pub align_items: SelfAlignmentValue,
    /// [`ComputedValues::align_self`] の staging。層は computed-equivalent。
    pub align_self: AlignSelfValue,
    /// `row-gap` の **specified** value。phase 3
    /// ([`resolve_length_percentage_or_normal`]) で絶対化される (`normal` は
    /// 保持、`<length-percentage>` のみ絶対化) — [`Self::letter_spacing`] と
    /// 同じ「specified 層のまま留まる」分類だが、`normal` の computed 表現が
    /// 異なる ([`crate::resolve::ComputedLengthPercentageOrNormal`] doc 参照)。
    pub row_gap: LengthOrNormal,
    /// `column-gap` の **specified** value。[`Self::row_gap`] と同じ絶対化 phase。
    pub column_gap: LengthOrNormal,
    /// [`ComputedValues::quotes`] の staging。層は computed-equivalent
    /// (length を運ばないため絶対化不要)。
    pub quotes: Arc<Vec<(SmolStr, SmolStr)>>,
    /// Whether an empty quotes list is the initial `auto` value.
    pub quotes_auto: bool,
    /// `text-shadow` の **specified** value。phase 3
    /// ([`resolve_text_shadow_item`]) で各 item の length 3 本が絶対化される
    /// — [`Self::padding`] と同じ「specified 層のまま留まる」分類だが、こちらは
    /// **inherited** かつ list-shaped (`Sides<T>` の 4 side ではなく可変長
    /// list) — [`ComputedValues::text_shadow`] doc 参照。`Self::inherit_from`
    /// は (`padding` のように initial へ再セットするのではなく) 親の
    /// computed 値を [`lift_text_shadow_item`] で lift して seed する
    /// (`Self::text_indent` と同じ扱い)。
    pub text_shadow: Arc<Vec<TextShadowItem>>,
    /// `grid-template-columns` の **specified** value。phase 3
    /// (track list 中の `<length-percentage>` のみ絶対化、`none` keyword は
    /// 保持) で絶対化される — [`Self::flex_basis`] と同じ「keyword or
    /// absolutize」shape だが対象が単一値ではなく track list 全体。
    pub grid_template_columns: GridTemplateTracks,
    /// `grid-template-rows` の **specified** value。[`Self::grid_template_columns`]
    /// と同じ絶対化 phase。
    pub grid_template_rows: GridTemplateTracks,
    /// [`ComputedValues::grid_template_areas`] の staging。層は
    /// computed-equivalent (spec の "Computed value: the keyword `none` or a
    /// list of string values" — track-sizing の `<length-percentage>` の
    /// ような絶対化対象を持たない、[`GridTemplateAreasValue`] doc 参照)。
    pub grid_template_areas: GridTemplateAreasValue,
    /// `grid-auto-columns` の **specified** value。phase 3 で
    /// [`Self::grid_template_columns`] と同じ track-size 絶対化を受ける。
    pub grid_auto_columns: Arc<Vec<GridTrackSize>>,
    /// `grid-auto-rows` の **specified** value。[`Self::grid_auto_columns`]
    /// と同じ絶対化 phase。
    pub grid_auto_rows: Arc<Vec<GridTrackSize>>,
    /// [`ComputedValues::grid_auto_flow`] の staging。層は computed-equivalent
    /// (`GridAutoFlowValue` は length を運ばない)。
    pub grid_auto_flow: GridAutoFlowValue,
    /// [`ComputedValues::grid_row_start`] の staging。層は computed-equivalent
    /// (`GridLineValue` は length を運ばない)。
    pub grid_row_start: GridLineValue,
    /// [`ComputedValues::grid_row_end`] の staging。層は computed-equivalent。
    pub grid_row_end: GridLineValue,
    /// [`ComputedValues::grid_column_start`] の staging。層は computed-equivalent。
    pub grid_column_start: GridLineValue,
    /// [`ComputedValues::grid_column_end`] の staging。層は computed-equivalent。
    pub grid_column_end: GridLineValue,
    /// [`ComputedValues::justify_items`] の staging。層は computed-equivalent
    /// (`SelfAlignmentValue` は length を運ばない)。
    pub justify_items: SelfAlignmentValue,
    /// [`ComputedValues::justify_self`] の staging。層は computed-equivalent。
    pub justify_self: AlignSelfValue,
    /// [`ComputedValues::orphans`] の staging。層は computed-equivalent
    /// (`<integer>` は length を運ばない)。
    pub orphans: i32,
    /// [`ComputedValues::widows`] の staging。[`Self::orphans`] と同じ層。
    pub widows: i32,
    /// [`ComputedValues::background_repeat`] の staging。層は
    /// computed-equivalent (`BackgroundRepeat` は length を運ばない)。
    pub background_repeat: BackgroundRepeat,
    /// [`ComputedValues::background_attachment`] の staging。層は
    /// computed-equivalent (`BackgroundAttachment` は length を運ばない)。
    pub background_attachment: BackgroundAttachment,
    /// [`ComputedValues::background_clip`] の staging。層は
    /// computed-equivalent (`VisualBox` は length を運ばない)。
    pub background_clip: VisualBox,
    /// [`ComputedValues::background_origin`] の staging。層は
    /// computed-equivalent (`VisualBox` は length を運ばない)。
    pub background_origin: VisualBox,
    /// `background-size` の **specified** value。phase 3 で各軸の
    /// `<length-percentage>` を絶対化する (`width`/`height` と同じ shape)。
    pub background_size: BackgroundSize,
    /// `background-position` の **specified** value。phase 3 で各 offset の
    /// `<length-percentage>` を絶対化する。
    pub background_position: CssPosition,
    /// [`ComputedValues::background_image`] の staging。`None`/`Url(String)`
    /// は computed-equivalent。`Gradient(..)` variant (CSS Images 4 §3) の
    /// `<length-percentage>` payload (`GradientColorStop::position`,
    /// `RadialSize::Circle`/`Ellipse`, `RadialGradient`/`ConicGradient` の
    /// `CssPosition`) は font-relative 部分を phase 3 で絶対化し、
    /// `<percentage>` は gradient box 寸法 (paint/used-value 層) が必要なため
    /// 素通し — `resolve_background_image` doc参照。`<angle>` は常に素通し。
    pub background_image: BackgroundImage,
    /// [`ComputedValues::object_fit`] の staging。層は computed-equivalent
    /// (`ObjectFit` は length を運ばない)。
    pub object_fit: ObjectFit,
    /// `object-position` の **specified** value。phase 3 で各 offset の
    /// `<length-percentage>` を絶対化する (`background_position` と同じ shape
    /// — 型自体も [`CssPosition`] を再利用する)。
    pub object_position: CssPosition,
    /// `opacity` の **specified** value — 範囲外の値も clamp せずそのまま
    /// 保持する ([`ComputedValues::opacity`] doc の "specified preserves,
    /// computed clamps" 節参照)。clamp は phase 3
    /// ([`Self::absolutize_with`]) が行う。
    pub opacity: f32,
    /// `isolation` の **specified** value — 常に keyword、絶対化不要な素通し
    /// field (`object_fit` と同じ shape)。
    pub isolation: Isolation,
    /// `mix-blend-mode` の **specified** value — `isolation` と同じ shape。
    pub mix_blend_mode: MixBlendMode,
    /// `mask-image` の **specified** value — `None`/`Url(String)` は
    /// computed-equivalent。`Gradient(..)` variant は `background_image` と
    /// 同じく `<length-percentage>` の font-relative 部分を phase 3 で絶対化
    /// し、`<percentage>` は素通し (`resolve_background_image` doc参照)。
    pub mask_image: MaskImage,
    /// `clip-path` の **specified** value — `mask_image` と同じ shape
    /// (埋め込まれた `<url>` は絶対化しない、[`ClipPath`] doc参照)。
    pub clip_path: ClipPath,
    /// `transform` の **specified** value。CSS Transforms Level 1 §4 の
    /// Computed value は "as specified, but with lengths made absolute" —
    /// `mask_image`/`filter` の "as specified" (絶対化不要) とは異なり、
    /// 埋め込まれた `Length` payload (`translate()`/`translateX()`/
    /// `translateY()` の non-percentage 側) を phase 3
    /// ([`Self::absolutize_with`]) で絶対化する — length 側は
    /// font-size/root-font-size 基準で `Px` へ、percentage 側は symbolic な
    /// `Percent` のまま残す ([`crate::resolve::resolve_length_percentage`]
    /// と同じ split、[`crate::property::TransformFunction`] doc参照)。
    /// `none` は空 list ([`empty_transform_list`]) で表現する。
    pub transform: Arc<Vec<TransformFunction>>,
    /// `filter` の **specified** value — `transform` と同じ shape
    /// (埋め込まれた `Length`/`Angle`/`f32` は絶対化しない、
    /// [`FilterFunction`] doc参照)。`none` は空 list
    /// ([`empty_filter_list`]) で表現する。
    pub filter: Arc<Vec<FilterFunction>>,
    /// `table-layout` の **specified** value — **non-inherited**、initial:
    /// [`TableLayoutValue::Auto`] (CSS Tables 3 §4
    /// <https://www.w3.org/TR/css-tables-3/#table-layout-property>)。
    /// computed value = specified keyword のため
    /// [`crate::computed::ComputedValues::table_layout`]
    /// の staging として素通しする ([`Self::float`] と同じ
    /// computed-equivalent 分類)。
    pub table_layout: TableLayoutValue,
    /// `border-collapse` の **specified** value — **inherited**、initial:
    /// [`BorderCollapseValue::Separate`] (CSS Tables 3 §6
    /// <https://www.w3.org/TR/css-tables-3/#border-collapse-property>)。
    /// computed value = specified keyword のため
    /// [`crate::computed::ComputedValues::border_collapse`]
    /// の staging として素通しする。**inherited** なので `Self::inherit_from`
    /// は親の computed 値を seed する ([`Self::text_indent`] と同じ扱いではなく
    /// 素朴なコピー — keyword のため lift 不要、[`Self::visibility`] と同じ)。
    pub border_collapse: BorderCollapseValue,
    /// `border-spacing` の **specified** value — **inherited**、initial:
    /// `0` (両軸 `0px`、CSS Tables 3 §6.1
    /// <https://www.w3.org/TR/css-tables-3/#border-spacing-property>)。
    /// computed value = two absolute lengths のため
    /// [`crate::computed::ComputedValues::border_spacing`]
    /// の staging として phase 3 で絶対化する ([`Self::tab_size`] の
    /// `Length` arm と同じ length-bearing staging 形)。
    /// **inherited** なので `Self::inherit_from` は親の computed 値を
    /// [`lift_border_spacing`] で seed する ([`Self::tab_size`] の
    /// `Length` arm と同じ lift 扱い)。
    pub border_spacing: BorderSpacingValue,
    /// `caption-side` の **specified** value — **inherited**、initial:
    /// [`CaptionSideValue::Top`] (CSS Tables 3 §7
    /// <https://www.w3.org/TR/css-tables-3/#caption-side-property>)。
    /// computed value = specified keyword のため
    /// [`crate::computed::ComputedValues::caption_side`]
    /// の staging として素通しする。**inherited** なので `Self::inherit_from`
    /// は親の computed 値を seed する (素朴なコピー — keyword のため
    /// lift 不要、[`Self::visibility`] と同じ)。
    pub caption_side: CaptionSideValue,
    /// `empty-cells` の **specified** value — **inherited**、initial:
    /// [`EmptyCellsValue::Show`] (CSS Tables 3 §8
    /// <https://www.w3.org/TR/css-tables-3/#empty-cells-property>)。
    /// computed value = specified keyword のため
    /// [`crate::computed::ComputedValues::empty_cells`]
    /// の staging として素通しする。**inherited** なので `Self::inherit_from`
    /// は親の computed 値を seed する (素朴なコピー — keyword のため
    /// lift 不要、[`Self::visibility`] と同じ)。
    pub empty_cells: EmptyCellsValue,
    /// `column-count` specified value; non-inherited.
    pub column_count: ColumnCountValue,
    /// `column-width` specified value; non-inherited.
    pub column_width: ColumnWidthValue,
}

impl SpecifiedValues {
    /// 全 property が CSS spec の initial value である staging 値。
    ///
    /// 各値の spec 根拠は [`ComputedValues::initial`] の field 単位 comment と
    /// [`ComputedValues`] の field doc を canonical source として参照する
    /// (本関数はその **specified 表現**であり、絶対化を通すと
    /// [`ComputedValues::initial`] に一致する — check:
    /// `initial_specified_finalizes_to_initial_computed`)。
    ///
    /// 親を持たない node (Document root / detached subtree の起点) の seed に
    /// 使う。
    pub fn initial() -> Self {
        Self {
            color: CssColor::BLACK,
            background_color: CssColor::TRANSPARENT,
            // shared Arc slot — per-node allocation 回避 (`initial_font_family`
            // doc 参照)。
            font_family: initial_font_family(),
            // CSS Fonts 4 §2.5: initial は `medium` (本実装では 16px)。
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
            // CSS Writing Modes 4 §2.1: direction initial は `ltr`。
            direction: Direction::Ltr,
            // CSS Writing Modes 4 §3.2: writing-mode initial は
            // `horizontal-tb`。
            writing_mode: WritingMode::HorizontalTb,
            ruby_position: RubyPosition::Over,
            // CSS Text 3 §8.1: text-indent initial は `0`。
            text_indent: TextIndentLength::Length(Length::Px(0.0)),
            text_indent_ch_factor: None,
            text_indent_ch_font: None,
            text_indent_ch_inherited: false,
            text_indent_hanging: false,
            text_indent_each_line: false,
            padding: Sides::all(Length::Px(0.0)),
            margin: Sides::all(LengthOrAuto::Length(Length::Px(0.0))),
            // CSS Backgrounds 3 §3.3 / §3.2 / §3.1: width=medium (3px) /
            // style=none / color=currentcolor。computed 層では style gating に
            // より width が 0px に潰れる (`resolve_border`)。
            // (§ 番号は spec の `data-level` 実測値に基づき 5.x から訂正済み。
            // crate 全域で一貫して 3.x を使う — Backgrounds 3 の §5.x は
            // border-image の節なので「一貫性のため」本 file を 5.x に
            // 戻してはならない。)
            border: Sides::all(INITIAL_BORDER),
            border_radius: BorderRadius {
                top_left: Length::Px(0.0),
                top_right: Length::Px(0.0),
                bottom_right: Length::Px(0.0),
                bottom_left: Length::Px(0.0),
            },
            box_shadow: empty_box_shadow_list(),
            outline: Outline {
                width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
                style: OutlineStyle::None,
                color: OutlineColor::Invert,
            },
            // CSS UI 3 §4.5: outline-offset initial は `0`.
            outline_offset: Length::Px(0.0),
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
            max_width: LengthOrAuto::Auto,
            max_height: LengthOrAuto::Auto,
            min_width: LengthOrAuto::Auto,
            min_height: LengthOrAuto::Auto,
            min_block_size: None,
            top: LengthOrAuto::Auto,
            right: LengthOrAuto::Auto,
            bottom: LengthOrAuto::Auto,
            left: LengthOrAuto::Auto,
            box_sizing: BoxSizing::ContentBox,
            // CSS Overflow 3 §3.1: overflow-x/overflow-y initial は
            // `visible`。
            overflow: OverflowXY::both(OverflowValue::Visible),
            // CSS Text Decoration Module Level 3 §2.1/§2.2/§2.3: initial は
            // それぞれ `none` / `solid` / `currentcolor`。
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
            // CSS 2.1 §10.8.1: vertical-align initial は `baseline`。
            vertical_align: VerticalAlign::Baseline,
            // CSS Fonts 4 §2.4: font-style initial は `normal`。
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
            // CSS Fonts Module Level 3 §6.6: font-variant-caps initial は
            // `normal`。
            font_variant_caps: FontVariantCaps::Normal,
            // CSS Text Module Level 3 §2.1: text-transform initial は `none`。
            text_transform: TextTransform::None,
            // CSS Writing Modes 3 §9.1: text-combine-upright initial is `none`.
            text_combine_upright: TextCombineUpright::None,
            // CSS Writing Modes 3 §5.1: text-orientation initial is `mixed`.
            text_orientation: TextOrientation::Mixed,
            // CSS Writing Modes 3 §2.2: unicode-bidi initial is `normal`.
            unicode_bidi: UnicodeBidi::Normal,
            // CSS Display 3 §4: visibility initial は `visible`。
            visibility: Visibility::Visible,
            // CSS2 §9.9.1: z-index initial は `auto`。
            z_index: ZIndexValue::Auto,
            // CSS Text 3 §5.1: word-break initial は `normal`。
            word_break: WordBreak::Normal,
            // CSS Text 3 §5.3: line-break initial は `auto`。
            line_break: LineBreak::Auto,
            // CSS Text 3 §5.4: overflow-wrap initial は `normal`。
            overflow_wrap: OverflowWrap::Normal,
            // CSS Text 3 §7.2 / §7.1: letter-spacing / word-spacing の
            // initial は共に `normal`。
            letter_spacing: LetterSpacingValue::Normal,
            letter_spacing_ch_factor: None,
            letter_spacing_ch_font: None,
            word_spacing: WordSpacingValue::Normal,
            word_spacing_ch_factor: None,
            word_spacing_ch_font: None,
            // CSS Text Module Level 3 §4.2: tab-size initial は `8`。
            tab_size: TabSize::Number(8.0),
            // CSS Fragmentation Module Level 3 §3.1 / §3.2: break-before /
            // break-after / break-inside initial は共に `auto`。
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
            // CSS Paged Media 3 §8.1: page initial is `auto` and is not inherited.
            page: PageValue::Auto,
            // CSS2 §9.5.1 / §9.5.2: float / clear の initial は共に `none`。
            float: FloatValue::None,
            clear: ClearValue::None,
            // CSS Text 3 §3: white-space initial は `normal`。
            white_space: WhiteSpace::Normal,
            // CSS Text 4 white-space-collapse initial value is `collapse`.
            white_space_collapse: WhiteSpaceCollapse::Collapse,
            text_wrap: TextWrapMode::Wrap,
            text_wrap_style: TextWrapStyle::Auto,
            // CSS Text 3 §5.3: hyphens initial は `manual`。
            hyphens: Hyphens::Manual,
            // CSS Text 4: hyphenate-character initial is `auto`.
            hyphenate_character: HyphenateCharacter::Auto,
            // CSS Text 4: hyphenate-limit-chars initial is `auto`.
            hyphenate_limit_chars: HyphenateLimitChars::INITIAL,
            // CSS Flexible Box Layout Module Level 1 §5.1/§5.2:
            // flex-direction initial は `row`、flex-wrap initial は `nowrap`。
            flex_direction: FlexDirectionValue::Row,
            flex_wrap: FlexWrapValue::NoWrap,
            // CSS Flexible Box Layout Module Level 1 §7.2.1/§7.2.2:
            // flex-grow initial は `0`、flex-shrink initial は `1`。
            flex_grow: 0.0,
            flex_shrink: 1.0,
            // CSS Flexible Box Layout Module Level 1 §7.2.3: flex-basis
            // initial は `auto`。
            flex_basis: FlexBasisValue::Auto,
            // CSS Flexible Box Layout Module Level 1 §4.2: order initial
            // は `0`。
            order: 0,
            // CSS Box Alignment Module Level 3 §5.1 (justify-content /
            // align-content) / §7.2 (align-items): initial は `normal`、
            // §6.2 (align-self) の initial は `auto`。
            justify_content: ContentAlignmentValue::Normal,
            align_content: ContentAlignmentValue::Normal,
            align_items: SelfAlignmentValue::Normal,
            align_self: AlignSelfValue::Auto,
            // CSS Box Alignment Module Level 3 §8.1: row-gap / column-gap
            // initial は `normal`。
            row_gap: LengthOrNormal::Normal,
            column_gap: LengthOrNormal::Normal,
            // CSS Content 3 §2.4.1: quotes の spec initial は "depends on
            // user agent"、本 impl は 独立実装方針によりそれを空 list で
            // 表現する (`ComputedValues::quotes` doc 参照)。
            quotes: empty_quotes_entries(),
            quotes_auto: true,
            // CSS Text Decoration Module Level 3 §4: text-shadow initial
            // は `none` — shared empty Arc slot (`empty_text_shadow_list`
            // doc 参照)。
            text_shadow: empty_text_shadow_list(),
            // CSS Grid Layout Module Level 1 §7.2/§7.3: grid-template-*
            // initial は共に `none`。
            grid_template_columns: GridTemplateTracks::None,
            grid_template_rows: GridTemplateTracks::None,
            grid_template_areas: GridTemplateAreasValue::None,
            // CSS Grid Layout Module Level 1 §7.6: grid-auto-columns /
            // grid-auto-rows initial は `auto`。
            grid_auto_columns: initial_grid_auto_track_list(),
            grid_auto_rows: initial_grid_auto_track_list(),
            // CSS Grid Layout Module Level 1 §7.7: grid-auto-flow initial は
            // `row`。
            grid_auto_flow: GridAutoFlowValue::Row,
            // CSS Grid Layout Module Level 1 §8.3: grid-row-start/-end /
            // grid-column-start/-end initial は共に `auto`。
            grid_row_start: GridLineValue::Auto,
            grid_row_end: GridLineValue::Auto,
            grid_column_start: GridLineValue::Auto,
            grid_column_end: GridLineValue::Auto,
            // CSS Box Alignment Module Level 3 §7.1/§6.1: justify-items /
            // justify-self initial (`PropertyValue::JustifyItems` doc の
            // "legacy は未対応" 節参照、justify-self は `auto`)。
            justify_items: SelfAlignmentValue::Normal,
            justify_self: AlignSelfValue::Auto,
            // CSS Fragmentation Module Level 3 §3.3: orphans / widows
            // initial は共に `2`。
            orphans: 2,
            widows: 2,
            // CSS Backgrounds and Borders 3 §2.4: background-repeat initial
            // は `repeat` (両軸)。
            background_repeat: BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::Repeat,
            },
            // CSS Backgrounds and Borders 3 §2.5: background-attachment
            // initial は `scroll`。
            background_attachment: BackgroundAttachment::Scroll,
            // CSS Backgrounds and Borders 3 §2.7: background-clip initial
            // は `border-box` — sibling `background_origin` (initial
            // `padding-box`) と異なる点に注意。
            background_clip: VisualBox::BorderBox,
            // background-origin initial は `padding-box`。
            background_origin: VisualBox::PaddingBox,
            // CSS Backgrounds and Borders 3 §2.9: background-size initial
            // は `auto` (= 両軸 `auto`、`BackgroundSize` doc の
            // "1 value のみ指定時" fill 規則とは無関係の spec 明示値)。
            background_size: BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            },
            // CSS Backgrounds and Borders 3 §2.6: background-position
            // initial は `0% 0%`。
            background_position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
                vertical: CssPositionOffset::Start(Length::Percent(0.0)),
            },
            // CSS Backgrounds and Borders 3 §2.3: background-image initial
            // は `none`。
            background_image: BackgroundImage::None,
            // CSS Images Module Level 3 §5.1: object-fit initial は `fill`。
            object_fit: ObjectFit::Fill,
            // CSS Images Module Level 3 §5.2: object-position initial は
            // `50% 50%` — `background-position` の `0% 0%` とは異なる点に
            // 注意。
            object_position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
                vertical: CssPositionOffset::Start(Length::Percent(50.0)),
            },
            // CSS Color 4 §3.3: opacity initial は `1`。
            opacity: 1.0,
            // CSS Compositing and Blending Level 1 §3.4.2: isolation
            // initial は `auto`。
            isolation: Isolation::Auto,
            // CSS Compositing and Blending Level 1 §3.4.1: mix-blend-mode
            // initial は `normal`。
            mix_blend_mode: MixBlendMode::Normal,
            // CSS Masking Level 1 §7.1: mask-image initial は `none`。
            mask_image: MaskImage::None,
            // CSS Masking Level 1 §5.1: clip-path initial は `none`。
            clip_path: ClipPath::None,
            // CSS Transforms Level 1 §4: transform initial は `none`
            // (空 list)。
            transform: empty_transform_list(),
            // CSS Filter Effects Level 1 §5: filter initial は `none`
            // (空 list)。
            filter: empty_filter_list(),
            // CSS Tables 3 §4: table-layout initial は `auto`
            // (non-inherited)。
            table_layout: TableLayoutValue::Auto,
            // CSS Tables 3 §6: border-collapse initial は `separate`
            // (inherited — 親を持つ node は `Self::inherit_from` が親値で
            // 上書きする)。
            border_collapse: BorderCollapseValue::Separate,
            // CSS Tables 3 §6.1: border-spacing initial は `0`
            // (inherited — 親を持つ node は `Self::inherit_from` が親値で
            // 上書きする、両軸 0px)。
            border_spacing: BorderSpacingValue {
                horizontal: Length::Px(0.0),
                vertical: Length::Px(0.0),
            },
            // CSS Tables 3 §7: caption-side initial は `top`
            // (inherited — 親を持つ node は `Self::inherit_from` が親値で
            // 上書きする)。
            caption_side: CaptionSideValue::Top,
            // CSS Tables 3 §8: empty-cells initial は `show`
            // (inherited — 親を持つ node は `Self::inherit_from` が親値で
            // 上書きする)。
            empty_cells: EmptyCellsValue::Show,
            column_count: ColumnCountValue::Auto,
            column_width: ColumnWidthValue::Auto,
        }
    }

    /// 親 node の [`ComputedValues`] から child node の staging 開始値を作る。
    ///
    /// - **inherited** property は親の computed 値から seed する。length を運ぶ
    ///   `font_size` / `line_height` / `text_indent` / `letter_spacing` /
    ///   `word_spacing` / `tab_size` は [`crate::resolve::lift_font_size`] /
    ///   [`crate::resolve::lift_line_height`] / [`crate::resolve::lift_text_indent`] /
    ///   [`crate::resolve::lift_letter_spacing`] / [`crate::resolve::lift_word_spacing`] /
    ///   [`crate::resolve::lift_tab_size`] で specified 表現に lift する (`Px` / `Percent` は
    ///   絶対化の不動点なので、phase 2 / phase 3 を通しても二重適用に
    ///   ならない — 各関数の doc 参照)。
    /// - **non-inherited** property は [`Self::initial`] と同じ値。
    ///
    /// 分類の canonical source は [`ComputedValues`] の field doc comment。
    /// 新 property を足すときは本関数と [`Self::initial`] と
    /// [`Self::finalize`] の 3 箇所を同時に更新する (前 2 者は field 網羅、
    /// 最後は絶対化の要否)。
    ///
    /// **分類の実装は本関数 1 箇所だけである** — public な
    /// [`ComputedValues::inherit_from`] は本関数 + [`Self::finalize`] へ delegate
    /// する thin wrapper なので、そちらに分類を写す必要はない。
    ///
    /// [`ComputedValues::inherit_from`]: crate::computed::ComputedValues::inherit_from
    ///
    /// **inherited field を [`Self::initial`] から seed してはならない** —
    /// [`Self`] doc の D5 invariant を参照。
    pub fn inherit_from(parent: &ComputedValues) -> Self {
        // **直接 struct literal で初期化する** — `..Self::initial()` 経由に
        // 「簡約」すると `font_family` の `Vec` を 1 度 allocate → drop してから
        // parent から clone し直すことになり無駄
        // (本 comment は `ComputedValues::inherit_from` から移設したもの)。
        // `font_family` は `Arc<Vec<FontFamilyName>>` 化されたため、この特定の
        // malloc→drop は解消済み
        // (`initial_font_family()` は shared slot の bump のみ) —
        // ただし本 directive (下記) はそれとは独立に立つ (per-field 網羅列挙が
        // 新規 property 追加時の audit friendliness を担う、将来 field が同種の
        // 生 heap payload を持てば再発しうる)。
        //
        // 本関数は per-node で走るので、この無駄は O(N) の alloc regression に
        // なる。**15 行削れるからと `..Self::initial()` に書き換えてはならない。**
        Self {
            // ── inherited: 親の computed 値から seed ────────────────────
            color: parent.color,
            font_family: parent.font_family.clone(),
            // computed `<length>` → specified `Px` の lift (lossless、不動点)。
            font_size: lift_font_size(parent.font_size),
            // D5: `bolder` / `lighter` はこの値を基準に解決される。
            font_weight: parent.font_weight,
            line_height: lift_line_height(parent.line_height),
            // CSS Lists 3 §3: both list-style longhands are inherited.
            list_style_type: parent.list_style_type.clone(),
            list_style_position: parent.list_style_position,
            list_style_image: parent.list_style_image.clone(),
            // `match-parent` はここでは解決しない (素朴なコピー) — 解決は
            // `finalize` / `finalize_as_root` が全 winner 適用後に親の
            // `ComputedValues` を明示的に受け取って行う (`Self` doc の
            // "D5 と同型ではない" 節)。
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
            // CSS Text 3 §6.2 / §6.1: いずれも inherited、keyword の素朴なコピー。
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
            text_indent_ch_font: parent.text_indent_ch_font.clone(),
            text_indent_ch_inherited: parent.text_indent_ch_inherited
                || parent.text_indent_ch_factor.is_some(),
            text_indent_hanging: parent.text_indent_hanging,
            text_indent_each_line: parent.text_indent_each_line,
            // CSS Fonts 4 §2.4: font-style は inherited。
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
            // CSS Fonts Module Level 3 §6.6: font-variant-caps は inherited。
            font_variant_caps: parent.font_variant_caps,
            // CSS Text Module Level 3 §2.1: text-transform は inherited。
            text_transform: parent.text_transform,
            // CSS Writing Modes 3 §9.1: text-combine-upright is inherited.
            text_combine_upright: parent.text_combine_upright,
            // CSS Writing Modes 3 §5.1: text-orientation is inherited.
            text_orientation: parent.text_orientation,
            // CSS Display 3 §4: visibility は inherited。
            visibility: parent.visibility,
            // CSS Text 3 §5.1: word-break は inherited。
            word_break: parent.word_break,
            // CSS Text 3 §5.3: line-break は inherited。
            line_break: parent.line_break,
            // CSS Text 3 §5.4: overflow-wrap は inherited。
            overflow_wrap: parent.overflow_wrap,
            // CSS Text 3 letter-spacing and CSS Text 4 word-spacing are inherited.
            // Both preserve computed percentages and mixed calcs through their
            // corresponding lift helpers; the absolute layout fallback remains
            // separate in `ComputedValues::word_spacing`.
            letter_spacing: lift_letter_spacing(parent.letter_spacing_computed),
            letter_spacing_ch_factor: parent.letter_spacing_ch_factor,
            letter_spacing_ch_font: parent.letter_spacing_ch_font.clone(),
            word_spacing: lift_word_spacing(parent.word_spacing_computed),
            word_spacing_ch_factor: parent.word_spacing_ch_factor,
            word_spacing_ch_font: parent.word_spacing_ch_font.clone(),
            // CSS Text Module Level 3 §4.2: tab-size は inherited。computed
            // `<number>` / `<length>` → specified 表現の lift
            // (`lift_line_height` と同型)。
            tab_size: lift_tab_size(parent.tab_size),
            // CSS Text 3 §3: white-space は inherited。
            white_space: parent.white_space,
            // CSS Text 4 white-space-collapse is inherited.
            white_space_collapse: parent.white_space_collapse,
            text_wrap: parent.text_wrap,
            text_wrap_style: parent.text_wrap_style,
            // CSS Tables 3 §6: border-collapse は inherited。keyword のため
            // lift 不要の素朴なコピー (`visibility` と同じ扱い)。
            border_collapse: parent.border_collapse,
            // CSS Tables 3 §6.1: border-spacing は inherited。computed
            // two-length → specified 表現の lift (`tab_size` の `Length`
            // arm と同型)。
            border_spacing: lift_border_spacing(parent.border_spacing),
            // CSS Tables 3 §7: caption-side は inherited。keyword のため
            // lift 不要の素朴なコピー (`visibility` と同じ扱い)。
            caption_side: parent.caption_side,
            // CSS Tables 3 §8: empty-cells は inherited。keyword のため
            // lift 不要の素朴なコピー (`visibility` と同じ扱い)。
            empty_cells: parent.empty_cells,
            // CSS Multi-column Layout 1: both longhands are non-inherited.
            column_count: ColumnCountValue::Auto,
            column_width: ColumnWidthValue::Auto,
            // CSS Text 3 §5.3: hyphens は inherited。
            hyphens: parent.hyphens,
            // CSS Text 4: hyphenate-character is inherited.
            hyphenate_character: parent.hyphenate_character.clone(),
            // CSS Text 4: hyphenate-limit-chars is inherited as computed values.
            hyphenate_limit_chars: parent.hyphenate_limit_chars,
            // CSS Content 3 §2.4.1: quotes は inherited。Arc reference-count increment のみ
            // (`ComputedValues::font_family` と同じ shape)。
            quotes: parent.quotes.clone(),
            quotes_auto: parent.quotes_auto,
            // CSS Fragmentation Module Level 3 §3.3: orphans / widows は共に
            // inherited。
            orphans: parent.orphans,
            widows: parent.widows,
            // CSS Text Decoration Module Level 3 §4: text-shadow は
            // inherited。computed `Arc<Vec<ComputedTextShadow>>` → specified
            // `Arc<Vec<TextShadowItem>>` の per-item lift (`lift_text_shadow_item`、
            // `Px` は不動点)。空 list は shared Arc slot を再利用 (per-node
            // allocation 回避、`Self::text_indent` 等の他 lift と違い list 全体を
            // map する必要があるため、空 check は他 non-inherited list property
            // (`content` 等) と同じ判断)。
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
                    })
                }
            },
            text_underline_position: parent.text_underline_position,
            text_emphasis_position: parent.text_emphasis_position,
            text_emphasis_style: parent.text_emphasis_style.clone(),
            text_emphasis_color: parent.text_emphasis_color,
            // ── non-inherited: initial 値 ───────────────────────────────
            // CSS Writing Modes 3 §2.2: unicode-bidi is non-inherited.
            unicode_bidi: UnicodeBidi::Normal,
            background_color: CssColor::TRANSPARENT,
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
                top_left: Length::Px(0.0),
                top_right: Length::Px(0.0),
                bottom_right: Length::Px(0.0),
                bottom_left: Length::Px(0.0),
            },
            box_shadow: empty_box_shadow_list(),
            outline: Outline {
                width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
                style: OutlineStyle::None,
                color: OutlineColor::Invert,
            },
            // CSS UI 3 §4.5: outline-offset は non-inherited, initial `0`.
            outline_offset: Length::Px(0.0),
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
            max_width: LengthOrAuto::Auto,
            max_height: LengthOrAuto::Auto,
            min_width: LengthOrAuto::Auto,
            min_height: LengthOrAuto::Auto,
            min_block_size: None,
            top: LengthOrAuto::Auto,
            right: LengthOrAuto::Auto,
            bottom: LengthOrAuto::Auto,
            left: LengthOrAuto::Auto,
            box_sizing: BoxSizing::ContentBox,
            // non-inherited (CSS Overflow 3 §3.1)。
            overflow: OverflowXY::both(OverflowValue::Visible),
            // non-inherited (CSS Text Decoration Module Level 3 §2.1/§2.2/
            // §2.3, all "Inherited: no")。
            text_decoration_line: TextDecorationLine::NONE,
            text_decoration_style: TextDecorationStyle::Solid,
            text_decoration_color: TextDecorationColor::CurrentColor,
            text_decoration_thickness: TextDecorationThickness::Auto,
            // non-inherited (CSS Text Decoration 4 §2.9.1 "Inherited: no")。
            text_decoration_inset: TextDecorationInset::Lengths {
                start: Length::Px(0.0),
                end: Length::Px(0.0),
            },
            // non-inherited (CSS 2.1 §10.8.1 "Inherited: no")。
            vertical_align: VerticalAlign::Baseline,
            // non-inherited (CSS2 §9.9.1 "Inherited: no")。
            z_index: ZIndexValue::Auto,
            // non-inherited (CSS Fragmentation Module Level 3 §3.1 / §3.2
            // "Inherited: no")。
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
            // `page` is non-inherited (CSS Paged Media 3 §8.1).
            page: PageValue::Auto,
            // non-inherited (CSS2 §9.5.1 / §9.5.2 "Inherited: no", both)。
            float: FloatValue::None,
            clear: ClearValue::None,
            // non-inherited (CSS Flexible Box Layout Module Level 1 §5.1/
            // §5.2/§7.2.1/§7.2.2/§7.2.3, all "Inherited: no")。
            flex_direction: FlexDirectionValue::Row,
            flex_wrap: FlexWrapValue::NoWrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: FlexBasisValue::Auto,
            order: 0,
            // non-inherited (CSS Box Alignment Module Level 3 §5.1/§5.1/
            // §7.2/§6.2, all "Inherited: no")。
            justify_content: ContentAlignmentValue::Normal,
            align_content: ContentAlignmentValue::Normal,
            align_items: SelfAlignmentValue::Normal,
            align_self: AlignSelfValue::Auto,
            // non-inherited (CSS Box Alignment Module Level 3 §8.1,
            // "Inherited: no")。
            row_gap: LengthOrNormal::Normal,
            column_gap: LengthOrNormal::Normal,
            // non-inherited (CSS Grid Layout Module Level 1 §7.2/§7.3/§7.6/
            // §7.7/§8.3, all "Inherited: no")。
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
            // "Inherited: no")。
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
            // non-inherited (CSS Filter Effects Level 1 §5 "Inherited: no").
            filter: empty_filter_list(),
            // non-inherited (CSS Tables 3 §4 "Inherited: no")、initial
            // `auto` — child は winner が無ければ常にこの値に戻る
            // (`float` と同じ扱い)。
            table_layout: TableLayoutValue::Auto,
            // non-inherited 側に置くのは `table_layout` のみ —
            // `border_collapse` は inherited のため上記 inherited 節で
            // seed 済み (`visibility` と同じ配置)。
        }
    }

    /// **親を持つ** node を絶対化して [`ComputedValues`] にする
    /// (**phase 2 → phase 3**)。
    ///
    /// `parent` は親要素の [`ComputedValues`] (font-size は phase 2 の基準、
    /// `text_align` + `direction` は `match-parent` 解決の基準 —
    /// `parent_font_size: ComputedLength` から `&ComputedValues`
    /// に広げた、下記「引数を広げた理由」節参照)。`ctx.root_font_size` は
    /// root element の computed font-size。root element 自身には
    /// [`Self::finalize_as_root`] を使うこと (`rem` の基準が違う上、
    /// `match-parent` も "computes to start" の別ルールになる)。
    ///
    /// The parent provides the computed values needed for inherited
    /// properties such as `text-align: match-parent`. Lengths are resolved in
    /// the documented cascade phases: relative font-size values use the
    /// parent basis, while later relative values use this node's computed
    /// font size. The root uses [`Self::finalize_as_root`] because `rem` and
    /// `match-parent` have distinct root rules (CSS Text 3 §6.1).
    ///
    pub fn finalize(self, parent: &ComputedValues, ctx: &ResolveContext) -> ComputedValues {
        // `parent` の line-height 基準 (CSS Values 4 §6.1.1 の自己参照条項、
        // font-size (phase 2) の `lh` にも要るように
        // なった — line-height (phase 2.5) の `lh` と**同じ**基準を使い回す)。
        // `parent` はこの `finalize` 呼び出しに入る**前**に (tree walk の親→子
        // 順で) 既に確定済みなので、font-size (phase 2) より先に求めても
        // phase 2 → 2.5 の順序は崩れない — 崩れるとしたら「自 node の」
        // font-size 確定前に「自 node の」line-height を求めるケースだけで、
        // これは親の値の話であり無関係 (`mod@crate::resolve` の module doc
        // 「想定される 4 段階」節参照)。
        let parent_line_height_basis =
            used_line_height_length(parent.line_height, parent.font_size);
        // phase 2: font-size を **親基準** で絶対化する (CSS Values 4 §6.1.1
        // parent-metrics 条項)。`lh` は上記 `parent_line_height_basis`、`rlh`
        // は tree-global な `ctx.root_line_height` を参照する
        // (`resolve_font_size` doc の「`lh` / `rlh` の自己参照」節が
        // canonical)。
        let font_size = resolve_font_size(
            self.font_size,
            parent.font_size,
            parent_line_height_basis,
            ctx,
        );
        // text-align: match-parent の解決 (CSS Text 3 §6.1)。親を持つ node の
        // 分岐 — root element の "computes to start" は `finalize_as_root` 側。
        let text_align = resolve_text_align_internal_center(
            resolve_text_align_match_parent(self.text_align, parent.text_align, parent.direction),
            parent.text_align,
        );
        // phase 2.5: line-height を絶対化する。
        // `lh` (自己参照、親基準) / `rlh` (tree-global、`ctx.root_line_height`
        // 基準) の判断根拠は `resolve_line_height` doc が canonical。
        // この基準は自 node の
        // padding 等 (phase 3) には使わない — それらは `absolutize_with` 内で
        // 改めて**自 node の**基準 (`used_line_height_length(line_height,
        // font_size)`) を求める。
        let line_height =
            resolve_line_height(self.line_height, font_size, parent_line_height_basis, ctx);
        // phase 3: 残りを **自 node の** font-size / line-height 基準で絶対化する。
        self.absolutize_with(font_size, line_height, text_align, ctx)
    }

    /// **親を持たない** node (root element) を絶対化する。
    ///
    /// root element では `rem` の基準が phase 2 と phase 3 で**異なる**。
    /// CSS Values 4 §6.1.1 "Font-relative Lengths"
    /// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) の verbatim:
    ///
    /// > When used in the value of any font-\* property **on the element they
    /// > refer to**, the font-relative lengths resolve against the computed
    /// > metrics of the parent element—or against the computed metrics
    /// > corresponding to the initial values of the font and line-height
    /// > properties, if the element has no parent.
    ///
    /// - **phase 2** (`font-size`) — `font-size` は font-\* property であり、
    ///   `rem` は定義上 root element を指す (§6.1.1 `rem`: "Equal to the computed
    ///   value of the em unit on the root element."
    ///   <https://www.w3.org/TR/css-values-4/#rem>)。すなわち root element 上の
    ///   `font-size: Nrem` は「自分を指す font-relative length を font-\*
    ///   property に使う」case なので上記条項が発火し、**initial value (16px)
    ///   基準**になる。`em` も同じ条項で initial 基準 (親が無いため)。
    /// - **phase 3** (それ以外) — `padding` 等は font-\* property ではないので
    ///   条項は発火せず、`rem` は素の定義どおり **root element の computed
    ///   font-size** = phase 2 で確定した自分の font-size 基準になる。
    ///   (`em` も "the element on which it is used" の定義どおり自 font-size 基準
    ///   — 同 §の "The other font-relative lengths continue to resolve against
    ///   the element's own metrics when used in line-height." と整合。)
    ///
    /// この非対称は `html { font-size: 20px; padding: 2rem }` で観測できる —
    /// `font-size` は 20px、`padding` は 40px (16px × 2 = 32px では**ない**)。
    /// check: [`mod@crate::cascade`] の
    /// `rem_on_root_element_box_property_uses_own_font_size`。
    ///
    /// `line-height` **自身の値**に現れる font-relative unit も同じ非対称を
    /// 持つ — 同 §の "The other font-relative lengths continue to resolve
    /// against the element's own metrics when used in line-height." により
    /// `em` 等は自 font-size 基準のまま (phase 2.5、`finalize` 側と同じ)。
    /// **`lh` / `rlh` だけが例外** — spec 原文と両者の非対称の判断根拠は
    /// [`resolve_line_height`] doc が canonical (既知の drift 前例により
    /// 要約に留める)。
    /// 結論だけ述べると、root element には親が無いので `line-height: 1lh`
    /// / `1rlh` は常に「initial values」(`line-height: normal`) 基準に
    /// 帰着し、`normal` は font metrics が無い限り絶対長化できない
    /// (`cap`/`rcap` と同じ wall) — root element 上のこの自己参照は常に
    /// unresolved になる。
    ///
    /// root element の **box property** (`padding: 1rlh` 等) は上記の
    /// 自己参照条項の対象外 — こちらは phase 3 の話で、`rlh` の素の定義
    /// (「root element の `lh`」) どおり **自分の**確定済 line-height を
    /// 参照してよい。本関数はそのために phase 2.5 で自分の line-height を
    /// 確定させてから [`ResolveContext::with_root_line_height`] を組み立てる
    /// (下記 body 参照)。
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
    /// # 引数を取らない理由と、その前提
    ///
    /// 両 phase の基準がいずれも本関数の中で決まるので、呼び出し側が渡し間違える
    /// 余地が無い。**ただしこれは「呼び出し側が本当に親を持たない node にしか
    /// 本関数を使わない」ことが前提**である — 親の computed font-size を捨てて
    /// initial に固定するのが正しいのは §6.1.1 の "if the element has no parent"
    /// が成立するときだけ。[`crate::cascade::resolve_inheritance`] はその invariant
    /// を `debug_assert` で check している (同関数の `None` arm の comment 参照)。
    /// 同じ「親を持たない」前提が `text-align: match-parent` → `start` にも
    /// 適用される — raikiri のモデルでは「element 祖先が無い」ことを
    /// `root_ctx == None` で判定しており (合成 DOM では複数 element が
    /// 各々この意味で「root」になり得る、`resolve_inheritance` の doc 参照)、
    /// それがそのまま「match-parent の親が無い」の判定基準でもある。
    pub fn finalize_as_root(self) -> ComputedValues {
        // phase 2: 親が無いので initial values 基準。
        //
        // `em` / `rem` の根拠は上記 doc の CSS Values 4 §6.1.1 parent-metrics
        // 条項。**`<percentage>` は §6.1.1 の対象ではない** — 同条項の主語は
        // "the font-relative lengths" であり percentage を含まない。
        // `html { font-size: 150% }` → 24px の根拠は CSS Fonts 4 §2.5
        // <https://www.w3.org/TR/css-fonts-4/#font-size-prop> の
        // "Percentages: refer to parent element's font size" と「親が居ない場合は
        // initial values を基準にする」の**組み合わせによる導出**であって、
        // どちらの § の明文でもない (page 経路の同型の導出は
        // `cascade::resolve_against_inherited` の `FontSize` arm comment に
        // 同じ区別で書いてある)。
        //
        // 結果として 3 unit すべて 16px 基準になる。
        //
        // `lh`: root element には親が無いので
        // self-reference basis は常に `None` (= "initial values" =
        // `line-height: normal` = 解決不能、下記 phase 2.5 の `lh`/`rlh` と
        // 同じ判断)。`rlh` は `ResolveContext::initial()` の
        // `root_line_height: None` がそのまま同じ結果になる — root 自身の
        // `font-size: 1rlh` も自己参照 (`rlh` の素の定義上「自分自身」を
        // 参照するのは宣言要素が root のときだけ、`resolve_font_size` doc
        // 参照)。
        let font_size = resolve_font_size(
            self.font_size,
            ComputedLength(crate::computed::INITIAL_FONT_SIZE_PX),
            None,
            &ResolveContext::initial(),
        );
        // CSS Text 3 §6.1: "Computes to start when specified on the root
        // element." — 親の text_align / direction を一切参照しない、この
        // 関数に閉じた特別扱い (上記 doc 節参照)。
        let text_align = match self.text_align {
            TextAlign::MatchParent | TextAlign::Inherit => TextAlign::Start,
            TextAlign::InternalCenter => TextAlign::Center,
            other => other,
        };
        // phase 2.5: root element には親が無いので
        // `line-height` 自身の値に現れる `lh`/`rlh` の自己参照基準は常に
        // `None` (= "initial values" = `normal`、上記 doc 節)。それ以外の
        // font-relative unit (`em` 等) は自 font-size 基準のまま (`finalize`
        // と同じ `resolve_line_height` 呼び出し形)。
        let line_height = resolve_line_height(
            self.line_height,
            font_size,
            None,
            &ResolveContext::initial(),
        );
        // phase 3 の `ctx`: `rem` の基準は「root element の computed
        // font-size」= 自分、`rlh` の基準も同様「root element の確定済
        // line-height」= 自分 (box property は自己参照条項の対象外、上記
        // doc 節)。`used_line_height_length` は
        // `crate::cascade::resolve_inheritance` が子へ配る `child_ctx` と
        // 同じ導出 — 両者の一致は `mod@crate::cascade` の
        // `rlh_on_root_element_matches_child_root_line_height_basis` が check する。
        let own_line_height = used_line_height_length(line_height, font_size);
        let ctx = ResolveContext::with_root_line_height(font_size, own_line_height);
        self.absolutize_with(font_size, line_height, text_align, &ctx)
    }

    /// phase 3 — 自 node の確定済 computed `font-size` / `line-height` を基準に
    /// 残りを絶対化する。
    ///
    /// `parent` を **意図的に受け取らない** ([`Self::finalize`] doc の担保 1)。
    /// `text_align` は呼び手 ([`Self::finalize`] / [`Self::finalize_as_root`])
    /// が既に `match-parent` を解決した後の値 — 本関数はそれを素通しするだけで、
    /// 自身は解決ロジックを持たない (両呼び手の分岐が異なるため、本関数に
    /// 共通化すると root 判定を関数内に持ち込むことになり、上記「引数を取らない
    /// 理由」の局所性が崩れる)。
    ///
    /// `line_height` も同様に呼び手が phase 2.5 で確定させた**自 node の**
    /// [`ComputedLineHeight`] — 本関数は
    /// それを [`used_line_height_length`] で絶対長へ変換し、
    /// `padding`/`margin`/`border`/`width`/`height` の `lh` 解決基準
    /// (`own_line_height`) として使う。呼び手が
    /// `line_height` を自分で計算する (本関数の内部で
    /// `resolve_line_height` を呼ばない) のは、`padding: 1lh` 等が
    /// **既に確定した**自 node の line-height を必要とし、`font_size` と
    /// 同じく「先に確定させて引数で渡す」形にしないと参照順序を守れない
    /// ためである。
    fn absolutize_with(
        self,
        font_size: ComputedLength,
        line_height: ComputedLineHeight,
        text_align: TextAlign,
        ctx: &ResolveContext,
    ) -> ComputedValues {
        // `padding`/`margin`/`border`/`width`/`height` の `1lh` 解決基準
        // — `rlh` は `ctx.root_line_height` (tree-global)
        // を使うので、本 local はここでしか要らない。
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
            TextIndentLength::Calc(calc) => (resolve_text_indent_calc(calc, font_size), None),
        };
        // The font-metric ch fallback is only for a plain length with recorded
        // ch provenance. A calc keeps and resolves its own percent/px/em terms.
        let text_indent_ch_factor = match self.text_indent {
            TextIndentLength::Length(_) => self
                .text_indent_ch_factor
                .or(authored_text_indent_ch_factor),
            TextIndentLength::Calc(_) => None,
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
        let letter_spacing_ch_font = declaring_ch_font(
            letter_spacing.ch_factor,
            self.letter_spacing_ch_factor,
            &self.letter_spacing_ch_font,
        );
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
        ComputedValues {
            color: self.color,
            background_color: self.background_color,
            font_family: self.font_family,
            font_size,
            font_weight: self.font_weight,
            line_height,
            // CSS2 §9.7: `float` の cascaded value に応じて `display` の
            // computed value を強制変換する same-node coupling
            // (`resolve_display_for_float` doc 参照、`overflow`
            // cross-axis coupling と同じ phase 3 の位置)。
            display: resolve_display_for_float(self.display, self.float),
            list_style_type: self.list_style_type,
            list_style_position: self.list_style_position,
            list_style_image: self.list_style_image,
            counter_reset: self.counter_reset,
            counter_increment: self.counter_increment,
            counter_set: self.counter_set,
            content: self.content,
            string_set: self.string_set,
            running_templates: self.running_templates,
            position: self.position,
            // 呼び手が既に match-parent を解決した後の値 (関数 doc 参照)。
            text_align,
            // CSS Text 3 §8.2.1: inherited keyword, no relative resolution.
            hanging_punctuation: self.hanging_punctuation,
            // CSS Text 4: inherited keyword/flag set, no relative resolution.
            text_autospace: self.text_autospace,
            // CSS Text 4: inherited keyword combination; no resolution required.
            word_space_transform: self.word_space_transform,
            // CSS Text 4: inherited keyword; computed value is specified value.
            text_spacing_trim: self.text_spacing_trim,
            // keyword の素通し (解決不要)。
            text_justify: self.text_justify,
            text_align_last: self.text_align_last,
            // computed value = specified value、相対解決なし (`Direction` doc
            // 参照) — 自 node の winner 適用結果をそのまま素通し。
            direction: self.direction,
            // `vertical-rl`/`vertical-lr`/`sideways-rl`/`sideways-lr` の 4
            // keyword を renderer-facing `writing_mode` field では `HorizontalTb`
            // に正規化する。CSSOM computed keyword は `cssom_writing_mode` に
            // 別途保持するため、この fallback は layout 用に限定される。
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
            text_indent_ch_font,
            text_indent_ch_inherited: self.text_indent_ch_inherited,
            // Flags pass through untouched (no absolutization needed).
            text_indent_hanging: self.text_indent_hanging,
            text_indent_each_line: self.text_indent_each_line,
            padding: self
                .padding
                .map(|l| resolve_length_percentage(l, font_size, own_line_height, ctx)),
            padding_ch,
            // `margin` は `width`/`height` と型を共有するが、`Lh`/`Rlh`
            // 解決不能時の fallback は違う (`resolve_margin_length_or_auto`
            // doc 参照 — Finding A)。
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
            max_height: resolve_length_percentage_or_auto(
                self.max_height,
                font_size,
                own_line_height,
                ctx,
            ),
            min_width,
            min_height,
            min_block_size: physical_min_block,
            top: resolve_length_percentage_or_auto(self.top, font_size, own_line_height, ctx),
            right: resolve_length_percentage_or_auto(self.right, font_size, own_line_height, ctx),
            bottom: resolve_length_percentage_or_auto(self.bottom, font_size, own_line_height, ctx),
            left: resolve_length_percentage_or_auto(self.left, font_size, own_line_height, ctx),
            box_sizing: self.box_sizing,
            // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.7/§2.8 — computed value =
            // specified keyword(s), no length payload (`TextDecorationLine`
            // arm と同じ shape、下記参照)。
            background_repeat: self.background_repeat,
            background_attachment: self.background_attachment,
            background_clip: self.background_clip,
            background_origin: self.background_origin,
            // CSS Backgrounds and Borders 3 §2.9/§2.6 — `<length-percentage>`
            // を含むため `width`/`height` と同じ shape で絶対化する。
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
            // CSS Backgrounds and Borders 3 §2.3 / CSS Images 4 §3 —
            // `None`/`Url(String)` は computed-equivalent。`Gradient(..)` の
            // `<length-percentage>` payload は font-relative 部分を自 node の
            // `font-size`/`own_line_height` 基準で絶対化し、`<percentage>` は
            // gradient box 寸法が必要なため素通し
            // (`resolve_background_image` doc参照)。`<angle>` は常に素通し。
            background_image: resolve_background_image(
                self.background_image,
                font_size,
                own_line_height,
                ctx,
            ),
            // CSS Images Module Level 3 §5.1 — computed value = specified
            // keyword, no length payload (`background_repeat` arm と同じ
            // shape)。
            object_fit: self.object_fit,
            // CSS Images Module Level 3 §5.2 — `<length-percentage>` を含む
            // ため `background_position` と同じ shape で絶対化する (同じ
            // `resolve_css_position` を再利用)。
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
            // computed value = specified keyword(s)/color
            // (`TextDecorationLine`/`TextDecorationStyle`/`TextDecorationColor`
            // docs 参照、length を運ばないため相対解決なし)。
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
            // 6 keyword (`baseline`/`sub`/`super`/`middle`/`text-top`/
            // `text-bottom`) は computed value = specified keyword。
            // `VerticalAlign::Length` / `VerticalAlign::Calc` は own node の
            // `font_size` / `own_line_height` 基準で絶対化する
            // (`resolve_vertical_align` doc 参照 — percentage term は
            // `line-height: normal` 時 `0px` として扱う)。
            vertical_align: resolve_vertical_align(
                self.vertical_align,
                font_size,
                own_line_height,
                ctx,
            ),
            // computed value = specified keyword (`FontStyle` doc 参照、
            // この crate の scope では angle-bearing branch が unreachable
            // なため相対解決なし) — 自 node の winner 適用結果をそのまま素通し。
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
            font_variation_settings: self.font_variation_settings.canonicalized(),
            // computed value = specified keyword (`FontVariantCaps` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            font_variant_caps: self.font_variant_caps,
            // computed value = specified keyword (`TextTransform` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            text_transform: self.text_transform,
            // computed value = specified keyword (`TextCombineUpright` doc 参照).
            text_combine_upright: self.text_combine_upright,
            // computed value = specified keyword (`TextOrientation` doc 参照).
            text_orientation: self.text_orientation,
            // computed value = specified value (`UnicodeBidi` doc 参照).
            unicode_bidi: self.unicode_bidi,
            // computed value = specified keyword (`Visibility` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            visibility: self.visibility,
            // computed value = specified value (`ZIndexValue` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            z_index: self.z_index,
            // computed value = specified keyword (`WordBreak` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            word_break: self.word_break,
            // computed value = specified keyword (`LineBreak` doc).
            line_break: self.line_break,
            // computed value = specified keyword (`OverflowWrap` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。`word-wrap` legacy alias も同じ field に落ちる。
            overflow_wrap: self.overflow_wrap,
            // `letter-spacing` / `word-spacing` の `1lh` 解決基準も他の box
            // property と同じ `own_line_height` (CSS Text 3 §7.2 / §7.1 は
            // `normal` を `0` に潰す以外 line-height 基準の特別扱いを持たない)。
            letter_spacing: letter_spacing.value,
            letter_spacing_computed,
            // Preserve the `ch` provenance through inheritance for the
            // font-metric-aware text-layout consumer.
            letter_spacing_ch_factor: self.letter_spacing_ch_factor.or(letter_spacing.ch_factor),
            letter_spacing_ch_font,
            word_spacing: word_spacing.value,
            word_spacing_computed,
            word_spacing_ch_factor: self.word_spacing_ch_factor.or(word_spacing.ch_factor),
            word_spacing_ch_font,
            // `tab-size` の `1lh` 解決基準も他の box property と同じ
            // `own_line_height` (CSS Text Module Level 3 §4.2 は line-height
            // 基準の特別扱いを持たない)。
            tab_size: resolve_tab_size(self.tab_size, font_size, own_line_height, ctx),
            // computed value = specified keyword (`BreakBetween` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            break_before: self.break_before,
            break_after: self.break_after,
            // computed value = specified keyword (`BreakInside` doc 参照、
            // 同上)。
            break_inside: self.break_inside,
            // computed value = specified value (`FloatValue`/`ClearValue`
            // doc 参照、length を運ばないため相対解決なし) — 自 node の
            // winner 適用結果をそのまま素通し。`display` への影響は上の
            // `display` field 自体の代入式が担う (`resolve_display_for_float`)。
            float: self.float,
            clear: self.clear,
            // computed value = specified keyword (`WhiteSpace` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            white_space: self.white_space,
            // CSS Text 4 white-space-collapse computes to the specified keyword.
            white_space_collapse: self.white_space_collapse,
            text_wrap: self.text_wrap,
            text_wrap_style: self.text_wrap_style,
            // computed value = specified keyword (`Hyphens` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            hyphens: self.hyphens,
            // CSS Text 4 defines an inherited `auto | <string>` computed value.
            hyphenate_character: self.hyphenate_character.clone(),
            // This integer/`auto` triple is already in computed form.
            hyphenate_limit_chars: self.hyphenate_limit_chars,
            // computed value = specified keyword (`FlexDirectionValue` /
            // `FlexWrapValue` docs 参照、length を運ばないため相対解決なし) —
            // 自 node の winner 適用結果をそのまま素通し。
            flex_direction: self.flex_direction,
            flex_wrap: self.flex_wrap,
            // computed value = specified number (CSS Flexible Box Layout
            // Module Level 1 §7.2.1/§7.2.2 参照、`<number>` は絶対化不要) —
            // 自 node の winner 適用結果をそのまま素通し。
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            // `flex-basis` — `width`/`height` と同じ絶対化 shape
            // (`resolve_flex_basis` doc 参照)。
            flex_basis: resolve_flex_basis(self.flex_basis, font_size, own_line_height, ctx),
            // computed value = specified integer (CSS Flexible Box Layout
            // Module Level 1 §4.2 参照、`<integer>` は絶対化不要) —
            // 自 node の winner 適用結果をそのまま素通し。
            order: self.order,
            // computed value = specified keyword(s) (`ContentAlignmentValue`
            // / `SelfAlignmentValue` / `AlignSelfValue` docs 参照、length を
            // 運ばないため相対解決なし)。
            justify_content: self.justify_content,
            align_content: self.align_content,
            align_items: self.align_items,
            align_self: self.align_self,
            // `row-gap` / `column-gap` — `padding` と似た絶対化 shape だが
            // `normal` keyword を保持する (`resolve_length_percentage_or_normal`
            // doc 参照)。
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
            // computed value = specified value (`ComputedValues::quotes` doc
            // 参照、length を運ばないため相対解決なし) — 自 node の winner
            // 適用結果 (または inherit_from で継承した親値) をそのまま素通し。
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
            // `grid-template-columns`/`-rows` — track list 全体の
            // `<length-percentage>` を絶対化する (`resolve_grid_template_tracks`
            // doc 参照)。
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
            // computed value = specified value (`GridTemplateAreasValue` doc
            // 参照、spec の "list of string values" — track-sizing の
            // `<length-percentage>` のような絶対化対象を持たない) — 自 node
            // の winner 適用結果をそのまま素通し。
            grid_template_areas: self.grid_template_areas,
            // `grid-auto-columns`/`-rows` — `grid_template_columns` と同じ
            // track-size 絶対化。
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
            // computed value = specified keyword(s) (`GridAutoFlowValue` /
            // `GridLineValue` docs 参照、length を運ばないため相対解決なし) —
            // 自 node の winner 適用結果をそのまま素通し。
            grid_auto_flow: self.grid_auto_flow,
            grid_row_start: self.grid_row_start,
            grid_row_end: self.grid_row_end,
            grid_column_start: self.grid_column_start,
            grid_column_end: self.grid_column_end,
            // computed value = specified keyword(s) (`SelfAlignmentValue` /
            // `AlignSelfValue` docs 参照、length を運ばないため相対解決なし)。
            justify_items: self.justify_items,
            justify_self: self.justify_self,
            // computed value = specified integer (CSS Fragmentation Module
            // Level 3 §3.3、length を運ばないため相対解決なし) — 自 node の
            // winner 適用結果 (または inherit_from で継承した親値) をそのまま
            // 素通し。
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
            // CSS Compositing and Blending Level 1 §3.4.2: always a
            // keyword, no phase-3 transform — 素通し (`object_fit` と同じ
            // shape)。
            isolation: self.isolation,
            // CSS Compositing and Blending Level 1 §3.4.1: same shape as
            // `isolation` above.
            mix_blend_mode: self.mix_blend_mode,
            // CSS Masking Level 1 §7.1: `None`/`Url(String)` は
            // computed-equivalent。`Gradient(..)` の `<length-percentage>`
            // payload は `background_image` と同じく font-relative 部分を
            // phase 3 で絶対化し、`<percentage>` は素通し
            // (`resolve_background_image` doc参照)。
            mask_image: resolve_background_image(self.mask_image, font_size, own_line_height, ctx),
            // CSS Masking Level 1 §5.1: same shape as `mask_image` above
            // (`ClipPath` doc's scope note).
            clip_path: self.clip_path,
            // CSS Transforms Level 1 §4: Computed value is "as specified,
            // but with lengths made absolute" — `translate()`/
            // `translateX()`/`translateY()`'s `<length-percentage>` payload
            // の length 側だけを絶対化し percentage 側は `Percent` のまま残す
            // (`resolve_length_percentage` と同じ split、`background-position`/
            // `object-position` の `resolve_css_position` と同型)。
            // `matrix()` の 6 `<number>` slot と `rotate()`/`skew()` 系の
            // `<angle>` slot は対象外 — 前者は fully resolved、後者は
            // spec 上正規化されない `<angle>` (`crate::property::Angle` doc)。
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
            // CSS Filter Effects Level 1 §5: unlike `transform` above,
            // this property's own Computed value is plain "as specified"
            // — no absolutization is spec-required here at all
            // (`FilterFunction` doc's "Range restriction is reject, not
            // clamp" section already establishes this same "as specified"
            // fact for a different purpose) — 素通し。
            filter: self.filter,
            // computed value = specified keyword (`TableLayoutValue` doc
            // 参照、length を運ばないため相対解決なし) — 自 node の winner
            // 適用結果 (non-inherited のため inherit seed は常に initial)
            // をそのまま素通し。
            table_layout: self.table_layout,
            // computed value = specified keyword (`BorderCollapseValue` doc
            // 参照、同上) — 自 node の winner 適用結果 (または inherit_from
            // で継承した親値) をそのまま素通し。
            border_collapse: self.border_collapse,
            // computed value = two absolute lengths (`BorderSpacingValue` doc
            // 参照) — 自 node の winner 適用結果 (または inherit_from で
            // lift した親値) を自 node の font 基準で絶対化
            // (`tab_size` の `Length` arm と同型)。
            border_spacing: resolve_border_spacing(
                self.border_spacing,
                font_size,
                own_line_height,
                ctx,
            ),
            // computed value = specified keyword (`CaptionSideValue` doc
            // 参照、length を運ばないため相対解決なし) — 自 node の winner
            // 適用結果 (または inherit_from で継承した親値) をそのまま素通し。
            caption_side: self.caption_side,
            // computed value = specified keyword (`EmptyCellsValue` doc
            // 参照、同上) — 自 node の winner 適用結果 (または inherit_from
            // で継承した親値) をそのまま素通し。
            empty_cells: self.empty_cells,
            // CSS Multi-column Layout 1: non-inherited count passes through;
            // width is absolutized against the element's own font metrics.
            column_count: self.column_count,
            column_width: resolve_column_width(self.column_width, font_size, own_line_height, ctx),
            custom_properties: crate::computed::empty_custom_properties(),
            local_custom_properties: crate::computed::empty_custom_properties(),
        }
    }
}

/// `border-*` の specified initial value (1 side 分)。
///
/// CSS Backgrounds 3 — width は `medium`
/// (§3.3 "Line Thickness: the border-width properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-width> が本文で
/// "The thin, medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// respectively." と**規範的に**定める)、style は `none`
/// (§3.2 "Line Patterns: the border-style properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>)、color は
/// `currentcolor` (§3.1 "Line Colors: the border-color properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-color>)。
///
/// **computed 層の initial は本値ではない** — style が `none` なので
/// [`resolve_border`] の gating により width が 0px に潰れる
/// ([`ComputedValues::initial`] 参照)。
///
/// `pub(crate)` なのは page 経路の phase 3 ([`crate::page::cascade_page`]) が
/// `border-*-style` **未宣言**時の gating 基準として `.style` を読むため。
/// CSS Paged Media 3 §6 "Page Properties"
/// <https://www.w3.org/TR/css-page-3/#page-properties> の "both the page context
/// and the margin context have a computed value for every property" により、
/// 未宣言 property の computed value は initial 値であり、`border-*-style` は
/// non-inherited なので継承値ではなくここが唯一の source になる。
/// **「initial の border-style は `none`」を page.rs 側で literal 再掲しない**
/// ための共有である。width の `3.0` は独立 literal ではなく
/// [`crate::property::BORDER_WIDTH_MEDIUM_PX`] を参照する — 単一 source の
/// 詳細は同 const の doc 参照。
pub(crate) const INITIAL_BORDER: Border = Border {
    width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
    style: BorderStyle::None,
    color: BorderColor::CurrentColor,
};

#[cfg(test)]
mod tests;
