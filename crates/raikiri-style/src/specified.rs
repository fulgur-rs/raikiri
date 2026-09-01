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
//! `PropertyKey` discriminant を index にした slot 配列を昇順に走査するため
//! (以前は `HashMap` iteration 順で非決定的だった)。
//! 上の拘束は適用順の決定性とは独立に成り立つ: どの順に適用しようと
//! 「全 winner 適用後」でなければ基準 `font-size` は確定しない。

use std::sync::Arc;

use smol_str::SmolStr;

use crate::Atom;
use crate::computed::{ComputedValues, RunningTemplate};
use crate::property::{
    AlignSelfValue, BORDER_WIDTH_MEDIUM_PX, Border, BorderColor, BorderStyle, BoxSizing,
    BreakBetween, BreakInside, ClearValue, ContentAlignmentValue, ContentComponent, CssColor,
    Direction, DisplayValue, FlexBasisValue, FlexDirectionValue, FlexWrapValue, FloatValue,
    FontStyle, FontVariantCaps, GridAutoFlowValue, GridLineValue, GridTemplateAreasValue,
    GridTemplateTracks, GridTrackSize, Hyphens, Length, LengthOrAuto, LengthOrNormal, LineHeight,
    OverflowValue, OverflowWrap, OverflowXY, SelfAlignmentValue, Sides, TabSize, TextAlign,
    TextDecorationColor, TextDecorationLine, TextDecorationStyle, TextShadowItem, TextTransform,
    VerticalAlign, Visibility, WhiteSpace, WordBreak, ZIndexValue, empty_content_list,
    empty_counter_entries, empty_quotes_entries, empty_string_set_entries, empty_text_shadow_list,
    initial_font_family, initial_grid_auto_track_list, resolve_display_for_float, resolve_overflow,
    resolve_text_align_match_parent,
};
use crate::resolve::{
    ComputedLength, ComputedLineHeight, ResolveContext, empty_computed_text_shadow_list,
    lift_font_size, lift_length_or_normal, lift_length_percentage, lift_line_height, lift_tab_size,
    lift_text_shadow_item, resolve_border, resolve_flex_basis, resolve_font_size,
    resolve_grid_auto_track_list, resolve_grid_template_tracks, resolve_length_or_normal,
    resolve_length_percentage, resolve_length_percentage_or_auto,
    resolve_length_percentage_or_normal, resolve_line_height, resolve_margin_length_or_auto,
    resolve_tab_size, resolve_text_shadow_item, resolve_vertical_align, used_line_height_length,
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
/// | **specified 層のまま** (絶対化が phase 2 / phase 3 待ち) | `font_size` / `line_height` / `padding` / `margin` / `border` / `width` / `height` / `text_indent` / `letter_spacing` / `word_spacing` / `tab_size` / `text_shadow` |
/// | **既に computed-equivalent** (絶対化する length を含まない) | `color` / `background_color` / `font_family` / `font_weight` / `display` / `counter_*` / `content` / `string_set` / `running_templates` / `text_align` / `direction` / `box_sizing` / `overflow` / `text_decoration_line` / `text_decoration_style` / `text_decoration_color` / `font_style` / `font_variant_caps` / `text_transform` / `visibility` / `z_index` / `word_break` / `overflow_wrap` / `break_before` / `break_after` / `break_inside` / `float` / `clear` / `white_space` / `hyphens` / `quotes` / `orphans` / `widows` |
/// | **variant によって層が分かれる** (型は specified/computed で同じだが、一部 variant だけ絶対化を要る) | `vertical_align` — [`Self::vertical_align`] doc 参照 |
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
/// [`crate::cascade::apply_value`] の `PropertyValue::FontWeight` arm は
/// `apply_value` 中の read-modify-write で、書き込み前の
/// `self.font_weight` が**親の computed font-weight である**ことに依拠して
/// `bolder` / `lighter` を解決する。[`Self::initial`] から seed すると
/// `bolder` が常に 400 起点になり、**compile error にも既存 test の失敗にも
/// ならずに**壊れる。
/// pin: `bolder_resolves_against_parent_computed_weight_through_staging`。
///
/// `font_size` も同じ invariant に依拠する (`FontSizeRelative` arm が
/// 加わった) — [`Self::inherit_from`] は
/// `font_size` を [`crate::resolve::lift_font_size`] 経由で seed し、この
/// 関数は常に `Length::Px` を返す (`Px` は絶対化の不動点、同関数 doc 参照)。
/// [`Self::initial`] も `font_size: Length::Px(INITIAL_FONT_SIZE_PX)` で
/// 同じく `Px`。したがって `apply_value` の `FontSizeRelative` arm が
/// 書き込み前に読む `self.font_size` は**必ず「親 (または root では initial)
/// の computed font-size」の `Length::Px` 表現である** — `font_weight`
/// (`u16`、単位を持たない) とは表現型が違うが保証の形は同じ。
/// [`Self::initial`] から seed する実装に変えると `larger` が常に
/// `INITIAL_FONT_SIZE_PX` (16px) 起点になり、`bolder` と同じ壊れ方をする。
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
    pub font_family: Arc<Vec<Atom>>,
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
    /// [`ComputedValues::text_align`] の staging。層は computed-equivalent。
    /// `match-parent` はここでは**解決されない** — [`Self`] doc の
    /// "`text_align: match-parent` は D5 と同型ではない" 節参照。
    pub text_align: TextAlign,
    /// [`ComputedValues::direction`] の staging。層は computed-equivalent。
    pub direction: Direction,
    /// `text-indent` の **specified** value。phase 3
    /// ([`resolve_length_percentage`]) で絶対化される (percentage は素通し) —
    /// [`Self::padding`] と同じ絶対化 shape だが、こちらは **inherited**
    /// なので `Self::inherit_from` は (`padding` のように initial へ
    /// 再セットするのではなく) 親の computed 値を [`lift_length_percentage`]
    /// で lift して seed する。CSS Text 3 §8.1
    /// <https://www.w3.org/TR/css-text-3/#text-indent-property>。
    pub text_indent: Length,
    /// `padding` の **specified** value。phase 3
    /// ([`resolve_length_percentage`]) で絶対化される (percentage は素通し)。
    pub padding: Sides<Length>,
    /// `margin` の **specified** value。phase 3
    /// ([`resolve_margin_length_or_auto`]) で絶対化される。
    pub margin: Sides<LengthOrAuto>,
    /// `border` の **specified** value。phase 3 ([`resolve_border`]) で
    /// width が絶対化され、`none` / `hidden` の style gating も適用される。
    pub border: Sides<Border>,
    /// `width` の **specified** value。phase 3 で絶対化される。
    pub width: LengthOrAuto,
    /// `height` の **specified** value。phase 3 で絶対化される。
    pub height: LengthOrAuto,
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
    /// [`ComputedValues::vertical_align`] の staging。**型は
    /// [`ComputedValues::vertical_align`] と同じ** [`VerticalAlign`] だが、
    /// 層は field 一律ではない — `baseline`/`sub`/`super`/`middle`/
    /// `text-top`/`text-bottom` の 6 keyword は computed-equivalent (length
    /// を運ばない) な一方、[`VerticalAlign::Length`] は specified 層のまま
    /// (`em`/`rem` 等を保持、絶対化は [`Self::absolutize_with`] の
    /// [`crate::resolve::resolve_vertical_align`] 呼び出しに委ねる) — 型を
    /// 分けない理由は同関数 doc 参照。
    pub vertical_align: VerticalAlign,
    /// [`ComputedValues::font_style`] の staging。層は computed-equivalent
    /// (`FontStyle` は length を運ばない — この crate の scope では)。
    pub font_style: FontStyle,
    /// [`ComputedValues::font_variant_caps`] の staging。層は
    /// computed-equivalent (`FontVariantCaps` は length を運ばない)。
    pub font_variant_caps: FontVariantCaps,
    /// [`ComputedValues::text_transform`] の staging。層は computed-equivalent
    /// (`TextTransform` は length を運ばない)。
    pub text_transform: TextTransform,
    /// [`ComputedValues::visibility`] の staging。層は computed-equivalent
    /// (`Visibility` は length を運ばない)。
    pub visibility: Visibility,
    /// [`ComputedValues::z_index`] の staging。層は computed-equivalent
    /// (`ZIndexValue` は length を運ばない)。
    pub z_index: ZIndexValue,
    /// [`ComputedValues::word_break`] の staging。層は computed-equivalent
    /// (`WordBreak` は length を運ばない)。
    pub word_break: WordBreak,
    /// [`ComputedValues::overflow_wrap`] の staging。層は computed-equivalent
    /// (`OverflowWrap` は length を運ばない)。`word-wrap` legacy alias もこの
    /// 同じ field に落ちる ([`ComputedValues::overflow_wrap`] doc 参照)。
    pub overflow_wrap: OverflowWrap,
    /// `letter-spacing` の **specified** value。phase 3
    /// ([`resolve_length_or_normal`]) で絶対化される — `padding`/`margin` と
    /// 同じ「specified 層のまま留まる」分類 (`Normal` variant が `em`/`rem`
    /// を含む `Length` と disjoint ではないため、`font_style` 等の
    /// computed-equivalent 分類には入らない)。
    pub letter_spacing: LengthOrNormal,
    /// `word-spacing` の **specified** value。[`Self::letter_spacing`] と
    /// 同じ絶対化 phase・同じ分類理由 ([`LengthOrNormal`] を共有する
    /// sibling property、両者の spec 根拠は同 type の doc 参照)。
    pub word_spacing: LengthOrNormal,
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
    /// [`ComputedValues::float`] の staging。層は computed-equivalent
    /// (`FloatValue` は length を運ばない)。
    pub float: FloatValue,
    /// [`ComputedValues::clear`] の staging。層は computed-equivalent
    /// (`ClearValue` は length を運ばない)。
    pub clear: ClearValue,
    /// [`ComputedValues::white_space`] の staging。層は computed-equivalent
    /// (`WhiteSpace` は length を運ばない)。
    pub white_space: WhiteSpace,
    /// [`ComputedValues::hyphens`] の staging。層は computed-equivalent
    /// (`Hyphens` は length を運ばない)。
    pub hyphens: Hyphens,
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
}

impl SpecifiedValues {
    /// 全 property が CSS spec の initial value である staging 値。
    ///
    /// 各値の spec 根拠は [`ComputedValues::initial`] の field 単位 comment と
    /// [`ComputedValues`] の field doc を canonical source として参照する
    /// (本関数はその **specified 表現**であり、絶対化を通すと
    /// [`ComputedValues::initial`] に一致する — pin:
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
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: Vec::new(),
            text_align: TextAlign::Start,
            // CSS Writing Modes 4 §2.1: direction initial は `ltr`。
            direction: Direction::Ltr,
            // CSS Text 3 §8.1: text-indent initial は `0`。
            text_indent: Length::Px(0.0),
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
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
            box_sizing: BoxSizing::ContentBox,
            // CSS Overflow 3 §3.1: overflow-x/overflow-y initial は
            // `visible`。
            overflow: OverflowXY::both(OverflowValue::Visible),
            // CSS Text Decoration Module Level 3 §2.1/§2.2/§2.3: initial は
            // それぞれ `none` / `solid` / `currentcolor`。
            text_decoration_line: TextDecorationLine::NONE,
            text_decoration_style: TextDecorationStyle::Solid,
            text_decoration_color: TextDecorationColor::CurrentColor,
            // CSS 2.1 §10.8.1: vertical-align initial は `baseline`。
            vertical_align: VerticalAlign::Baseline,
            // CSS Fonts 4 §2.4: font-style initial は `normal`。
            font_style: FontStyle::Normal,
            // CSS Fonts Module Level 3 §6.6: font-variant-caps initial は
            // `normal`。
            font_variant_caps: FontVariantCaps::Normal,
            // CSS Text Module Level 3 §2.1: text-transform initial は `none`。
            text_transform: TextTransform::None,
            // CSS Display 3 §4: visibility initial は `visible`。
            visibility: Visibility::Visible,
            // CSS2 §9.9.1: z-index initial は `auto`。
            z_index: ZIndexValue::Auto,
            // CSS Text 3 §5.1: word-break initial は `normal`。
            word_break: WordBreak::Normal,
            // CSS Text 3 §5.4: overflow-wrap initial は `normal`。
            overflow_wrap: OverflowWrap::Normal,
            // CSS Text 3 §7.2 / §7.1: letter-spacing / word-spacing の
            // initial は共に `normal`。
            letter_spacing: LengthOrNormal::Normal,
            word_spacing: LengthOrNormal::Normal,
            // CSS Text Module Level 3 §4.2: tab-size initial は `8`。
            tab_size: TabSize::Number(8.0),
            // CSS Fragmentation Module Level 3 §3.1 / §3.2: break-before /
            // break-after / break-inside initial は共に `auto`。
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
            // CSS2 §9.5.1 / §9.5.2: float / clear の initial は共に `none`。
            float: FloatValue::None,
            clear: ClearValue::None,
            // CSS Text 3 §3: white-space initial は `normal`。
            white_space: WhiteSpace::Normal,
            // CSS Text 3 §5.3: hyphens initial は `manual`。
            hyphens: Hyphens::Manual,
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
            // user agent"、本 impl は cleanroom 方針によりそれを空 list で
            // 表現する (`ComputedValues::quotes` doc 参照)。
            quotes: empty_quotes_entries(),
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
        }
    }

    /// 親 node の [`ComputedValues`] から child node の staging 開始値を作る。
    ///
    /// - **inherited** property は親の computed 値から seed する。length を運ぶ
    ///   `font_size` / `line_height` / `text_indent` / `letter_spacing` /
    ///   `word_spacing` / `tab_size` は [`lift_font_size`] / [`lift_line_height`] /
    ///   [`lift_length_percentage`] / [`lift_length_or_normal`] /
    ///   [`lift_tab_size`] で specified 表現に lift する (`Px` / `Percent` は
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
        // `font_family` は `Arc<Vec<Atom>>` 化されたため、この特定の
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
            // `match-parent` はここでは解決しない (素朴なコピー) — 解決は
            // `finalize` / `finalize_as_root` が全 winner 適用後に親の
            // `ComputedValues` を明示的に受け取って行う (`Self` doc の
            // "D5 と同型ではない" 節)。
            text_align: parent.text_align,
            direction: parent.direction,
            // CSS Text 3 §8.1: text-indent は inherited。computed
            // `<length-percentage>` → specified `Length` の lift (lossless、
            // `Px` / `Percent` どちらも不動点、`lift_length_percentage` doc
            // 参照)。
            text_indent: lift_length_percentage(parent.text_indent),
            // CSS Fonts 4 §2.4: font-style は inherited。
            font_style: parent.font_style,
            // CSS Fonts Module Level 3 §6.6: font-variant-caps は inherited。
            font_variant_caps: parent.font_variant_caps,
            // CSS Text Module Level 3 §2.1: text-transform は inherited。
            text_transform: parent.text_transform,
            // CSS Display 3 §4: visibility は inherited。
            visibility: parent.visibility,
            // CSS Text 3 §5.1: word-break は inherited。
            word_break: parent.word_break,
            // CSS Text 3 §5.4: overflow-wrap は inherited。
            overflow_wrap: parent.overflow_wrap,
            // CSS Text 3 §7.2 / §7.1: letter-spacing / word-spacing は共に
            // inherited。computed `<length>` → specified `Px` の lift
            // (`lift_font_size` と同じ lossless / 不動点性、
            // `lift_length_or_normal` doc 参照)。
            letter_spacing: lift_length_or_normal(parent.letter_spacing),
            word_spacing: lift_length_or_normal(parent.word_spacing),
            // CSS Text Module Level 3 §4.2: tab-size は inherited。computed
            // `<number>` / `<length>` → specified 表現の lift
            // (`lift_line_height` と同型)。
            tab_size: lift_tab_size(parent.tab_size),
            // CSS Text 3 §3: white-space は inherited。
            white_space: parent.white_space,
            // CSS Text 3 §5.3: hyphens は inherited。
            hyphens: parent.hyphens,
            // CSS Content 3 §2.4.1: quotes は inherited。Arc bump のみ
            // (`ComputedValues::font_family` と同じ shape)。
            quotes: parent.quotes.clone(),
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
            // ── non-inherited: initial 値 ───────────────────────────────
            background_color: CssColor::TRANSPARENT,
            display: DisplayValue::Inline,
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: Vec::new(),
            padding: Sides::all(Length::Px(0.0)),
            margin: Sides::all(LengthOrAuto::Length(Length::Px(0.0))),
            border: Sides::all(INITIAL_BORDER),
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
            box_sizing: BoxSizing::ContentBox,
            // non-inherited (CSS Overflow 3 §3.1)。
            overflow: OverflowXY::both(OverflowValue::Visible),
            // non-inherited (CSS Text Decoration Module Level 3 §2.1/§2.2/
            // §2.3, all "Inherited: no")。
            text_decoration_line: TextDecorationLine::NONE,
            text_decoration_style: TextDecorationStyle::Solid,
            text_decoration_color: TextDecorationColor::CurrentColor,
            // non-inherited (CSS 2.1 §10.8.1 "Inherited: no")。
            vertical_align: VerticalAlign::Baseline,
            // non-inherited (CSS2 §9.9.1 "Inherited: no")。
            z_index: ZIndexValue::Auto,
            // non-inherited (CSS Fragmentation Module Level 3 §3.1 / §3.2
            // "Inherited: no")。
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
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
    /// # 引数を `&ComputedValues` に広げた理由
    ///
    /// 当初 `parent_font_size: ComputedLength` だけを受け取っていたが、
    /// `text-align: match-parent` の解決 (CSS Text 3 §6.1) が親の
    /// `text_align` + `direction` も要求するようになった。3 つの scalar
    /// 引数に分割する案 (`parent_font_size, parent_text_align,
    /// parent_direction`) も検討したが、将来また別の inherited property が
    /// 「親の computed 値」を要求するたびに引数が増える形になるため、
    /// [`crate::cascade::resolve_against_inherited`] が既に取っている
    /// `inherited: &ComputedValues` の shape に揃えた —呼び手 (`cascade.rs` の
    /// `resolve_inheritance`) は `parent_computed` をそのまま渡すだけになる。
    ///
    /// この形は同一 node の winner 適用順序に関する懸念を持ち込まない —
    /// `parent` は呼び手が**この node の staging (`self`) とは別に**保持して
    /// いる、既に確定済みの親の [`ComputedValues`] であり、`self` (自 node の
    /// staging、`direction` winner が上書き済みかもしれない) とは無関係な
    /// 参照である。[`crate::property::resolve_text_align_match_parent`] の
    /// doc が説明する「なぜ `apply_value` ではなく `finalize` か」の根拠は
    /// まさにこの分離にある。
    ///
    /// # phase 順序の担保
    ///
    /// phase 2 と phase 3 は基準が違う (親の font-size vs. 自 node の font-size)
    /// にもかかわらず両方 [`ComputedLength`] なので、型検査だけでは取り違えを
    /// 防げない。本実装が実際に担保するのは次の 3 点であり、それ以上ではない:
    ///
    /// 1. **phase 3 の本体は `parent` を名前として持たない** — 後半は
    ///    private な `Self::absolutize_with` に閉じており、その signature に
    ///    `parent` が無いので、**その body の中では**取り違えが書けない。
    ///
    ///    **本関数の body については同じことが言えない** — `parent.font_size` と
    ///    `font_size` はどちらも [`ComputedLength`] として同一 scope に居るので
    ///    `self.absolutize_with(parent.font_size, ..., ctx)` のような取り違えは
    ///    compile する。「取り違えは書けない」という capability claim は本関数には
    ///    成り立たない。`text_align` 解決も同型の risk を持つ —
    ///    `resolve_text_align_match_parent(self.text_align, parent.text_align,
    ///    parent.direction)` の 2 番目と 3 番目の引数は異なる型
    ///    (`TextAlign` / `Direction`) なので取り違えれば compile error になるが、
    ///    `self.text_align` と `parent.text_align` はどちらも `TextAlign` なので
    ///    その 2 つの取り違えは compile する。
    /// 2. **cascade pipeline から見た絶対化の入口は本関数と
    ///    [`Self::finalize_as_root`] の 2 つだけ** なので、phase 2 → phase 3 の
    ///    順序と基準の受け渡しは各 2 行に局所化されている。
    ///    **call site を実際に守っているのはこの局所性であって claim 1 ではない。**
    /// 3. **root / 非 root の `rem` 基準の違いが entry point の名前になっている**
    ///    ので、呼び出し側は `ResolveContext` を組み立てる判断をしない。同様に
    ///    `match-parent` の「親あり」/「親なし (root)」分岐も entry point の
    ///    選択そのもの (`finalize` vs `finalize_as_root`) に埋め込まれている。
    ///
    /// 逆に担保**していない**こと: [`crate::resolve`] の絶対化関数群は個別に
    /// public なので、本関数を経由せず誤った基準で呼ぶ code は依然として書ける。
    /// `OwnFontSize` / `ParentFontSize` newtype による型 level の enforcement は
    /// 採らなかった — 守る距離が各関数の 2 行しかない一方、public 関数 8 本と
    /// その doctest の signature churn を伴うため。
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
        let text_align =
            resolve_text_align_match_parent(self.text_align, parent.text_align, parent.direction);
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
    /// pin: [`mod@crate::cascade`] の
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
    /// を `debug_assert` で pin している (同関数の `None` arm の comment 参照)。
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
            TextAlign::MatchParent => TextAlign::Start,
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
        // `rlh_on_root_element_matches_child_root_line_height_basis` が pin する。
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
            counter_reset: self.counter_reset,
            counter_increment: self.counter_increment,
            counter_set: self.counter_set,
            content: self.content,
            string_set: self.string_set,
            running_templates: self.running_templates,
            // 呼び手が既に match-parent を解決した後の値 (関数 doc 参照)。
            text_align,
            // computed value = specified value、相対解決なし (`Direction` doc
            // 参照) — 自 node の winner 適用結果をそのまま素通し。
            direction: self.direction,
            // `text-indent` — same absolutization shape as `padding` (`%` is
            // passed through, `em`/`rem`/`pt`/etc. resolve against the own
            // `font_size`/`own_line_height` basis established above), but
            // this field is **inherited** — a child with no winner of its
            // own gets this value from `Self::inherit_from`'s
            // `lift_length_percentage(parent.text_indent)` seed instead of
            // resetting to the initial `0` (`Self::padding` is
            // non-inherited and always resets, `Self::text_align` sibling
            // comment above shows the inherited counterpart pattern).
            text_indent: resolve_length_percentage(
                self.text_indent,
                font_size,
                own_line_height,
                ctx,
            ),
            padding: self
                .padding
                .map(|l| resolve_length_percentage(l, font_size, own_line_height, ctx)),
            // `margin` は `width`/`height` と型を共有するが、`Lh`/`Rlh`
            // 解決不能時の fallback は違う (`resolve_margin_length_or_auto`
            // doc 参照 — Finding A)。
            margin: self
                .margin
                .map(|l| resolve_margin_length_or_auto(l, font_size, own_line_height, ctx)),
            border: self
                .border
                .map(|b| resolve_border(b, font_size, own_line_height, ctx)),
            width: resolve_length_percentage_or_auto(self.width, font_size, own_line_height, ctx),
            height: resolve_length_percentage_or_auto(self.height, font_size, own_line_height, ctx),
            box_sizing: self.box_sizing,
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
            // 6 keyword (`baseline`/`sub`/`super`/`middle`/`text-top`/
            // `text-bottom`) は computed value = specified keyword、
            // `VerticalAlign::Length` だけ own node の `font_size`/
            // `own_line_height` 基準で絶対化する (`resolve_vertical_align`
            // doc 参照)。
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
            // computed value = specified keyword (`FontVariantCaps` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            font_variant_caps: self.font_variant_caps,
            // computed value = specified keyword (`TextTransform` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            text_transform: self.text_transform,
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
            // computed value = specified keyword (`OverflowWrap` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。`word-wrap` legacy alias も同じ field に落ちる。
            overflow_wrap: self.overflow_wrap,
            // `letter-spacing` / `word-spacing` の `1lh` 解決基準も他の box
            // property と同じ `own_line_height` (CSS Text 3 §7.2 / §7.1 は
            // `normal` を `0` に潰す以外 line-height 基準の特別扱いを持たない)。
            letter_spacing: resolve_length_or_normal(
                self.letter_spacing,
                font_size,
                own_line_height,
                ctx,
            ),
            word_spacing: resolve_length_or_normal(
                self.word_spacing,
                font_size,
                own_line_height,
                ctx,
            ),
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
            // computed value = specified keyword (`Hyphens` doc 参照、
            // length を運ばないため相対解決なし) — 自 node の winner 適用結果を
            // そのまま素通し。
            hyphens: self.hyphens,
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
            custom_properties: crate::computed::empty_custom_properties(),
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
mod tests {
    use super::*;
    use crate::computed::INITIAL_FONT_SIZE_PX;
    use crate::property::TextShadowColor;
    use crate::resolve::{
        ComputedBorder, ComputedFlexBasis, ComputedGridTemplateTracks, ComputedGridTrackBreadth,
        ComputedGridTrackList, ComputedGridTrackListComponent, ComputedGridTrackSize,
        ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal,
        ComputedLineHeight, ComputedTabSize, ComputedTextShadow,
    };

    /// `root_font_size` = 16px の共通 context。
    const CTX: ResolveContext = ResolveContext {
        root_font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
        root_line_height: None,
    };

    /// `finalize` の `parent: &ComputedValues` として渡す、font-size だけ
    /// 差し替えた fixture (旧 `PARENT_FS: ComputedLength` の後継)。`text_align` / `direction` は本 module の phase 2/3 length 系
    /// test では無関係なので initial (`Start` / `Ltr`) のまま。
    fn parent_with_font_size(px: f32) -> ComputedValues {
        ComputedValues {
            font_size: ComputedLength(px),
            ..ComputedValues::initial()
        }
    }

    // -----------------------------------------------------------------
    // initial の 2 表現が一致する (drift 検出)
    // -----------------------------------------------------------------

    /// specified 層の initial を絶対化すると computed 層の initial に一致する。
    ///
    /// 両辺は独立に literal を持つ 2 本の struct literal なので自己参照ではない
    /// — 片方だけを書き換える drift を捕らえる。以前 `resolve.rs` に
    /// 置かれていた drift test (完全 tautology 化したため削除) の後継。
    #[test]
    fn initial_specified_finalizes_to_initial_computed() {
        assert_eq!(
            SpecifiedValues::initial()
                .finalize(&ComputedValues::initial(), &ResolveContext::initial()),
            ComputedValues::initial(),
        );
    }

    /// specified の border initial は `medium` (3px) / `none` / `currentcolor` で、
    /// computed 層では style gating により width が 0px に潰れる
    /// (CSS Backgrounds 3 §3.3 "Computed value: … zero if the border style is
    /// `none` or `hidden`")。
    #[test]
    fn initial_border_width_is_gated_to_zero_at_computed_layer() {
        assert_eq!(SpecifiedValues::initial().border.top.width, Length::Px(3.0));
        assert_eq!(
            ComputedValues::initial().border.top.width,
            ComputedLength::ZERO
        );
    }

    // -----------------------------------------------------------------
    // inherit_from — inherited / non-inherited の分類
    // -----------------------------------------------------------------

    fn parent_fixture() -> ComputedValues {
        ComputedValues {
            color: CssColor {
                r: 200,
                g: 100,
                b: 50,
                a: 255,
            },
            background_color: CssColor {
                r: 10,
                g: 20,
                b: 30,
                a: 255,
            },
            font_family: Arc::new(vec![Atom::from("sans-serif")]),
            font_size: ComputedLength(24.0),
            font_weight: 700.0,
            line_height: ComputedLineHeight::Number(1.5),
            display: DisplayValue::Block,
            counter_reset: Arc::new(vec![(SmolStr::new("chapter"), 3)]),
            counter_increment: Arc::new(vec![(SmolStr::new("section"), 2)]),
            counter_set: Arc::new(vec![(SmolStr::new("page"), 5)]),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: vec![RunningTemplate {
                name: SmolStr::new("hdr"),
            }],
            text_align: TextAlign::Center,
            direction: Direction::Rtl,
            text_indent: ComputedLengthPercentage::Px(9.0),
            padding: Sides::all(ComputedLengthPercentage::Px(7.0)),
            margin: Sides::all(ComputedLengthPercentageOrAuto::Px(12.0)),
            border: Sides::all(ComputedBorder {
                width: ComputedLength(5.0),
                style: BorderStyle::Solid,
                color: BorderColor::Resolved(CssColor::BLACK),
            }),
            width: ComputedLengthPercentageOrAuto::Px(200.0),
            height: ComputedLengthPercentageOrAuto::Px(200.0),
            box_sizing: BoxSizing::BorderBox,
            overflow: OverflowXY {
                x: OverflowValue::Hidden,
                y: OverflowValue::Scroll,
            },
            text_decoration_line: TextDecorationLine::UNDERLINE,
            text_decoration_style: TextDecorationStyle::Wavy,
            text_decoration_color: TextDecorationColor::Resolved(CssColor::BLACK),
            vertical_align: VerticalAlign::Sub,
            font_style: FontStyle::Italic,
            font_variant_caps: FontVariantCaps::SmallCaps,
            text_transform: TextTransform::Uppercase,
            visibility: Visibility::Hidden,
            z_index: ZIndexValue::Integer(3),
            word_break: WordBreak::KeepAll,
            overflow_wrap: OverflowWrap::Anywhere,
            letter_spacing: ComputedLength(2.0),
            word_spacing: ComputedLength(4.0),
            tab_size: ComputedTabSize::Length(ComputedLength(11.0)),
            break_before: BreakBetween::Page,
            break_after: BreakBetween::AvoidPage,
            break_inside: BreakInside::AvoidPage,
            float: FloatValue::Left,
            clear: ClearValue::Both,
            white_space: WhiteSpace::Pre,
            hyphens: Hyphens::None,
            flex_direction: FlexDirectionValue::Column,
            flex_wrap: FlexWrapValue::Wrap,
            flex_grow: 2.0,
            flex_shrink: 3.0,
            flex_basis: ComputedFlexBasis::Px(50.0),
            justify_content: ContentAlignmentValue::SpaceBetween,
            align_content: ContentAlignmentValue::Center,
            align_items: SelfAlignmentValue::FlexEnd,
            align_self: AlignSelfValue::Value(SelfAlignmentValue::Center),
            row_gap: ComputedLengthPercentageOrNormal::Px(6.0),
            column_gap: ComputedLengthPercentageOrNormal::Percent(10.0),
            quotes: Arc::new(vec![(SmolStr::new("«"), SmolStr::new("»"))]),
            text_shadow: Arc::new(vec![ComputedTextShadow {
                offset_x: ComputedLength(1.0),
                offset_y: ComputedLength(2.0),
                blur_radius: ComputedLength(3.0),
                color: TextShadowColor::Resolved(CssColor::BLACK),
            }]),
            grid_template_columns: ComputedGridTemplateTracks::List(Arc::new(
                ComputedGridTrackList {
                    line_names: vec![vec![], vec![]],
                    components: vec![ComputedGridTrackListComponent::Size(
                        ComputedGridTrackSize::Breadth(ComputedGridTrackBreadth::Px(100.0)),
                    )],
                },
            )),
            grid_template_rows: ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![ComputedGridTrackListComponent::Size(
                    ComputedGridTrackSize::Breadth(ComputedGridTrackBreadth::Percent(50.0)),
                )],
            })),
            grid_template_areas: GridTemplateAreasValue::Areas(Arc::new(
                crate::property::GridTemplateAreas {
                    row_strings: vec!["a".into()],
                    areas: vec![crate::property::GridTemplateAreaEntry {
                        name: "a".into(),
                        row_start: 1,
                        row_end: 2,
                        column_start: 1,
                        column_end: 2,
                    }],
                    row_count: 1,
                    column_count: 1,
                },
            )),
            grid_auto_columns: Arc::new(vec![ComputedGridTrackSize::Breadth(
                ComputedGridTrackBreadth::MinContent,
            )]),
            grid_auto_rows: Arc::new(vec![ComputedGridTrackSize::Breadth(
                ComputedGridTrackBreadth::MaxContent,
            )]),
            grid_auto_flow: GridAutoFlowValue::ColumnDense,
            grid_row_start: GridLineValue::Line(2),
            grid_row_end: GridLineValue::Span(3),
            grid_column_start: GridLineValue::Named("foo".into()),
            grid_column_end: GridLineValue::NamedLine("bar".into(), 2),
            justify_items: SelfAlignmentValue::Center,
            justify_self: AlignSelfValue::Value(SelfAlignmentValue::End),
            orphans: 5,
            widows: 7,
            custom_properties: crate::computed::empty_custom_properties(),
        }
    }

    #[test]
    fn inherit_from_copies_inherited_fields() {
        let parent = parent_fixture();
        let child = SpecifiedValues::inherit_from(&parent);
        assert_eq!(child.color, parent.color);
        assert_eq!(child.font_family, parent.font_family);
        // `inherit_from` の `parent.font_family.clone()` は Arc bump —
        // deep-clone regression なら ptr_eq が false になる (`Arc::ptr_eq`
        // behavioral-proxy methodology、`mod@crate::cascade` test 群と同型)。
        assert!(Arc::ptr_eq(&child.font_family, &parent.font_family));
        assert_eq!(child.font_weight, 700.0);
        // CSS Text 3 §6.1: text-align は inherited。
        assert_eq!(child.text_align, TextAlign::Center);
        // CSS Writing Modes 4 §2.1: direction は inherited。
        assert_eq!(child.direction, Direction::Rtl);
        // CSS Fonts 4 §2.4: font-style は inherited。
        assert_eq!(child.font_style, FontStyle::Italic);
        // CSS Fonts Module Level 3 §6.6: font-variant-caps は inherited。
        assert_eq!(child.font_variant_caps, FontVariantCaps::SmallCaps);
        // CSS Text Module Level 3 §2.1: text-transform は inherited。
        assert_eq!(child.text_transform, TextTransform::Uppercase);
        // CSS Display 3 §4: visibility は inherited。
        assert_eq!(child.visibility, Visibility::Hidden);
        // CSS Text 3 §8.1: text-indent は inherited — computed → specified
        // の lift (`lift_length_percentage`)。
        assert_eq!(child.text_indent, Length::Px(9.0));
        // CSS Text 3 §5.1: word-break は inherited。
        assert_eq!(child.word_break, WordBreak::KeepAll);
        // CSS Text 3 §5.4: overflow-wrap は inherited。
        assert_eq!(child.overflow_wrap, OverflowWrap::Anywhere);
        // CSS Text 3 §3: white-space は inherited。
        assert_eq!(child.white_space, WhiteSpace::Pre);
        // CSS Text 3 §5.3: hyphens は inherited。
        assert_eq!(child.hyphens, Hyphens::None);
        // computed → specified の lift (px 表現)。
        assert_eq!(child.font_size, Length::Px(24.0));
        assert_eq!(child.line_height, LineHeight::Number(1.5));
        // CSS Text 3 §7.2 / §7.1: letter-spacing / word-spacing は共に
        // inherited。computed → specified の lift (px 表現、`lift_font_size`
        // と同型)。
        assert_eq!(
            child.letter_spacing,
            LengthOrNormal::Length(Length::Px(2.0))
        );
        assert_eq!(child.word_spacing, LengthOrNormal::Length(Length::Px(4.0)));
        // CSS Text Module Level 3 §4.2: tab-size は inherited。computed
        // `<length>` → specified `Px` の lift (`lift_tab_size` 経由、
        // `lift_font_size` と同型)。
        assert_eq!(child.tab_size, TabSize::Length(Length::Px(11.0)));
        // CSS Content 3 §2.4.1: quotes は inherited。`inherit_from` の
        // `parent.quotes.clone()` は Arc bump —
        // deep-clone regression なら ptr_eq が false になる (`font_family`
        // 同 assertion と同じ methodology)。
        assert_eq!(child.quotes, parent.quotes);
        assert!(Arc::ptr_eq(&child.quotes, &parent.quotes));
        // CSS Text Decoration Module Level 3 §4: text-shadow は
        // inherited。computed → specified の per-item lift
        // (`lift_text_shadow_item`、px 表現) — `<color>` は素通し。
        assert_eq!(
            *child.text_shadow,
            vec![TextShadowItem {
                offset_x: Length::Px(1.0),
                offset_y: Length::Px(2.0),
                blur_radius: Length::Px(3.0),
                color: TextShadowColor::Resolved(CssColor::BLACK),
            }]
        );
        // CSS Fragmentation Module Level 3 §3.3: orphans / widows は共に
        // inherited。
        assert_eq!(child.orphans, 5);
        assert_eq!(child.widows, 7);
    }

    #[test]
    fn inherit_from_leaves_non_inherited_fields_at_initial() {
        let parent = parent_fixture();
        let child = SpecifiedValues::inherit_from(&parent);
        let initial = SpecifiedValues::initial();
        assert_eq!(child.background_color, CssColor::TRANSPARENT);
        assert_eq!(child.display, DisplayValue::Inline);
        assert!(child.counter_reset.is_empty());
        assert!(child.counter_increment.is_empty());
        assert!(child.counter_set.is_empty());
        assert!(child.content.is_empty());
        assert!(child.string_set.is_empty());
        assert!(child.running_templates.is_empty());
        assert_eq!(child.padding, initial.padding);
        assert_eq!(child.margin, initial.margin);
        assert_eq!(child.border, initial.border);
        assert_eq!(child.width, LengthOrAuto::Auto);
        assert_eq!(child.height, LengthOrAuto::Auto);
        assert_eq!(child.box_sizing, BoxSizing::ContentBox);
        // CSS Overflow 3 §3.1: overflow-x/overflow-y は
        // non-inherited。
        assert_eq!(child.overflow, initial.overflow);
        // CSS Text Decoration Module Level 3 §2.1/§2.2/§2.3:
        // text-decoration-line/-style/-color は non-inherited。
        assert_eq!(child.text_decoration_line, initial.text_decoration_line);
        assert_eq!(child.text_decoration_style, initial.text_decoration_style);
        assert_eq!(child.text_decoration_color, initial.text_decoration_color);
        // CSS 2.1 §10.8.1: vertical-align は non-inherited。
        assert_eq!(child.vertical_align, initial.vertical_align);
        // CSS2 §9.9.1: z-index は non-inherited。
        assert_eq!(child.z_index, initial.z_index);
        // CSS Fragmentation Module Level 3 §3.1 / §3.2: break-before /
        // break-after / break-inside は non-inherited。
        assert_eq!(child.break_before, initial.break_before);
        assert_eq!(child.break_after, initial.break_after);
        assert_eq!(child.break_inside, initial.break_inside);
        // CSS2 §9.5.1 / §9.5.2: float / clear は共に non-inherited。
        assert_eq!(child.float, initial.float);
        assert_eq!(child.clear, initial.clear);
        // CSS Flexible Box Layout Module Level 1 §5.1/§5.2/§7.2.1/§7.2.2/
        // §7.2.3: flex-* は non-inherited。
        assert_eq!(child.flex_direction, initial.flex_direction);
        assert_eq!(child.flex_wrap, initial.flex_wrap);
        assert_eq!(child.flex_grow, initial.flex_grow);
        assert_eq!(child.flex_shrink, initial.flex_shrink);
        assert_eq!(child.flex_basis, initial.flex_basis);
        // CSS Box Alignment Module Level 3 §5.1 (justify-content /
        // align-content) / §7.2 (align-items) / §6.2 (align-self): all
        // non-inherited。
        assert_eq!(child.justify_content, initial.justify_content);
        assert_eq!(child.align_content, initial.align_content);
        assert_eq!(child.align_items, initial.align_items);
        assert_eq!(child.align_self, initial.align_self);
        // CSS Box Alignment Module Level 3 §8.1: row-gap / column-gap は
        // non-inherited。
        assert_eq!(child.row_gap, initial.row_gap);
        assert_eq!(child.column_gap, initial.column_gap);
        // CSS Grid Layout Module Level 1 §7.2/§7.3/§7.6/§7.7/§8.3: grid-*
        // は全て non-inherited。
        assert_eq!(child.grid_template_columns, initial.grid_template_columns);
        assert_eq!(child.grid_template_rows, initial.grid_template_rows);
        assert_eq!(child.grid_template_areas, initial.grid_template_areas);
        assert_eq!(child.grid_auto_columns, initial.grid_auto_columns);
        assert_eq!(child.grid_auto_rows, initial.grid_auto_rows);
        assert_eq!(child.grid_auto_flow, initial.grid_auto_flow);
        assert_eq!(child.grid_row_start, initial.grid_row_start);
        assert_eq!(child.grid_row_end, initial.grid_row_end);
        assert_eq!(child.grid_column_start, initial.grid_column_start);
        assert_eq!(child.grid_column_end, initial.grid_column_end);
        // CSS Box Alignment Module Level 3 §7.1/§6.1: justify-items /
        // justify-self は共に non-inherited。
        assert_eq!(child.justify_items, initial.justify_items);
        assert_eq!(child.justify_self, initial.justify_self);
    }

    /// `line-height: 150%` を親が宣言していた場合、親の computed は
    /// `Length(ComputedLength(px))` であり、子は**その length をそのまま**継承する
    /// (CSS Inline 3 §5.1 — percentage は宣言要素で絶対化される)。
    #[test]
    fn inherit_from_lifts_computed_line_height_length_without_re_resolving() {
        let parent = ComputedValues {
            line_height: ComputedLineHeight::Length(ComputedLength(30.0)),
            ..ComputedValues::initial()
        };
        let child = SpecifiedValues::inherit_from(&parent);
        assert_eq!(child.line_height, LineHeight::Length(Length::Px(30.0)));
        // 子の font-size が 10px でも 15px にはならない。
        let computed = child.finalize(&parent_with_font_size(10.0), &CTX);
        assert_eq!(
            computed.line_height,
            ComputedLineHeight::Length(ComputedLength(30.0))
        );
    }

    // -----------------------------------------------------------------
    // finalize — phase 2 / phase 3 の基準
    // -----------------------------------------------------------------

    /// phase 2 の `em` は **親** の font-size 基準、phase 3 の `em` は
    /// **自 node の (phase 2 で確定した)** font-size 基準
    /// (CSS Values 4 §6.1.1)。両者を取り違えると padding が 32px になる。
    #[test]
    fn finalize_uses_parent_font_size_for_font_size_and_own_for_the_rest() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Em(2.0); // 親 16px → 32px
        sv.padding = Sides::all(Length::Em(1.0)); // 自 32px → 32px
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.font_size, ComputedLength(32.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(32.0));
    }

    /// 追加した `ex` も `em` と同じ parent/own 非対称を
    /// 持つ (unknown-metric fallback `0.5em`、`Length::Ex` doc)。数値は
    /// `finalize_uses_parent_font_size_for_font_size_and_own_for_the_rest`
    /// と揃える (`32px` / `32px`) — multiplier を変えて `ex` の `0.5` 係数を
    /// 通しても同じ基準規則になることを示す。padding 側に **親** (16px) を
    /// 誤って使うと `16 * 0.5 * 2 = 16px` になり、`32px` にならないため
    /// parent/own の取り違えを検出できる。
    #[test]
    fn finalize_resolves_ex_against_parent_for_font_size_and_own_for_padding() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Ex(4.0); // 親 16px 基準 → 0.5 * 4 * 16 = 32px
        sv.padding = Sides::all(Length::Ex(2.0)); // 自 (phase 2 で確定した) 32px 基準 → 0.5 * 2 * 32 = 32px
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.font_size, ComputedLength(32.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(32.0));
    }

    /// `vertical-align: <length>` absolutizes against the declaring node's
    /// own (phase-2-resolved) `font-size` — same basis `letter-spacing`/
    /// `word-spacing` use (`resolve_vertical_align` doc).
    #[test]
    fn finalize_resolves_vertical_align_length_against_own_font_size() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Px(20.0);
        sv.vertical_align = VerticalAlign::Length(Length::Em(1.5)); // 1.5 * 20 = 30px
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.vertical_align, VerticalAlign::Length(Length::Px(30.0)));
    }

    /// The 6 bare keywords (`baseline`/`sub`/`super`/`middle`/`text-top`/
    /// `text-bottom`) pass through `finalize` unchanged — no relative
    /// resolution needed (`VerticalAlign` doc's "Scope carving" section).
    #[test]
    fn finalize_passes_vertical_align_keywords_through_unchanged() {
        let mut sv = SpecifiedValues::initial();
        sv.vertical_align = VerticalAlign::Middle;
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.vertical_align, VerticalAlign::Middle);
    }

    /// `padding: 1lh` needs the **already-resolved own** line-height as its
    /// basis — this is exactly the phase-3 reordering
    /// this design describes: `absolutize_with` must capture `line_height`
    /// into a local *before* resolving `padding`/`margin`/`border`/`width`/
    /// `height`, or this would be unable to read it at all.
    #[test]
    fn finalize_resolves_lh_against_own_line_height_for_padding() {
        let mut sv = SpecifiedValues::initial();
        sv.line_height = LineHeight::Number(2.0); // 自 font-size (16px, inherited) 基準 → used 32px
        sv.padding = Sides::all(Length::Lh(1.5)); // 1.5 * 32 = 48px
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.line_height, ComputedLineHeight::Number(2.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(48.0));
    }

    /// When the own line-height is unresolvable (`normal`, the initial value
    /// — the common case, not an edge case), `1lh` falls back to padding's
    /// own spec initial `0` rather than a fabricated length (cleanroom: see
    /// `crate::resolve::resolve_length_percentage` doc for why). // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    #[test]
    fn finalize_resolves_lh_falls_back_to_zero_when_line_height_normal() {
        let mut sv = SpecifiedValues::initial(); // line_height stays `normal`
        sv.padding = Sides::all(Length::Lh(1.5));
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(0.0));
    }

    /// `<percentage>` は property ごとに扱いが違う: `font-size` は length に
    /// なり、`padding` / `margin` / `width` / `height` は computed 層に
    /// percentage のまま残る (CSS Values 4 §5.5.1 + CSS Box 3 の各 propdef)。
    #[test]
    fn finalize_keeps_box_percentages_and_resolves_font_size_percentage() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Percent(150.0);
        sv.padding = Sides::all(Length::Percent(25.0));
        sv.margin = Sides::all(LengthOrAuto::Length(Length::Percent(10.0)));
        sv.width = LengthOrAuto::Length(Length::Percent(50.0));
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.font_size, ComputedLength(24.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Percent(25.0));
        assert_eq!(
            cv.margin.bottom,
            ComputedLengthPercentageOrAuto::Percent(10.0)
        );
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Percent(50.0));
    }

    /// `rem` は phase 2 / phase 3 のどちらでも `ctx.root_font_size` 基準
    /// (CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>)。
    #[test]
    fn finalize_resolves_rem_against_context_root_font_size() {
        let ctx = ResolveContext::new(ComputedLength(20.0));
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Rem(2.0);
        sv.margin = Sides::all(LengthOrAuto::Length(Length::Rem(0.5)));
        let cv = sv.finalize(&parent_with_font_size(64.0), &ctx);
        assert_eq!(cv.font_size, ComputedLength(40.0));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(10.0));
    }

    // -----------------------------------------------------------------
    // finalize — text-align: match-parent (CSS Text 3 §6.1)
    // -----------------------------------------------------------------

    #[test]
    fn finalize_resolves_match_parent_against_parent_text_align_and_direction() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::MatchParent;
        let parent = ComputedValues {
            text_align: TextAlign::Start,
            direction: Direction::Rtl,
            ..ComputedValues::initial()
        };
        let cv = sv.finalize(&parent, &CTX);
        // Start + Rtl → Right (CSS Text 3 §6.1 verbatim table).
        assert_eq!(cv.text_align, TextAlign::Right);
    }

    /// **The test that pins the whole design of this module's `match-parent`
    /// handling**: a node that declares *both* `direction: rtl` and
    /// `text-align: match-parent` must still resolve against the *parent's*
    /// direction, not its own. If `finalize` (or a future refactor) ever
    /// starts reading `self.direction` instead of `parent.direction` for this
    /// resolution, this is the test that catches it — every other test in
    /// this module has `self.direction == parent.direction` and would stay
    /// green.
    ///
    /// Spec citation: CSS Text 3 §6.1 `#valdef-text-align-match-parent`
    /// says "interpreted against **the parent's** direction value" — not the
    /// element's own. See `crate::property::resolve_text_align_match_parent` // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// doc for why this can't be resolved in `cascade::apply_value` (the
    /// same-node winner-order hazard between the `direction` and `text-align`
    /// `PropertyKey` slots).
    #[test]
    fn finalize_match_parent_uses_parent_direction_not_own_direction_winner() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::MatchParent;
        // Own winner for `direction` already applied to the staging value —
        // simulates `direction: rtl` being cascaded on *this* node.
        sv.direction = Direction::Rtl;

        // Parent disagrees: Ltr.
        let parent = ComputedValues {
            text_align: TextAlign::Start,
            direction: Direction::Ltr,
            ..ComputedValues::initial()
        };
        let cv = sv.finalize(&parent, &CTX);
        // Must resolve against the *parent's* Ltr (→ Left), not the node's
        // own Rtl (which would give Right).
        assert_eq!(cv.text_align, TextAlign::Left);
        // The node's own `direction` winner is unaffected — it is a wholly
        // separate property and still flows through to the child's computed
        // value normally.
        assert_eq!(cv.direction, Direction::Rtl);
    }

    #[test]
    fn finalize_copies_non_start_end_parent_text_align_verbatim() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::MatchParent;
        let parent = ComputedValues {
            text_align: TextAlign::Center,
            ..ComputedValues::initial()
        };
        let cv = sv.finalize(&parent, &CTX);
        assert_eq!(cv.text_align, TextAlign::Center);
    }

    #[test]
    fn finalize_leaves_non_match_parent_text_align_untouched() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::Center;
        let parent = ComputedValues {
            text_align: TextAlign::Start,
            direction: Direction::Rtl,
            ..ComputedValues::initial()
        };
        let cv = sv.finalize(&parent, &CTX);
        assert_eq!(cv.text_align, TextAlign::Center);
    }

    /// CSS Text 3 §6.1 verbatim: "Computes to start when specified on the
    /// root element." — the parent-direction table does **not** apply here,
    /// even if the node itself declares a `direction`.
    #[test]
    fn finalize_as_root_resolves_match_parent_to_start() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::MatchParent;
        sv.direction = Direction::Rtl;
        assert_eq!(sv.finalize_as_root().text_align, TextAlign::Start);
    }

    #[test]
    fn finalize_as_root_leaves_non_match_parent_text_align_untouched() {
        let mut sv = SpecifiedValues::initial();
        sv.text_align = TextAlign::Right;
        assert_eq!(sv.finalize_as_root().text_align, TextAlign::Right);
    }

    /// `direction` itself is a plain inherited pass-through — no
    /// `match-parent`-style resolution, symmetric with `TextAlign::Center`
    /// et al.
    #[test]
    fn finalize_passes_direction_through_unchanged() {
        let mut sv = SpecifiedValues::initial();
        sv.direction = Direction::Rtl;
        let cv = sv.clone().finalize(&ComputedValues::initial(), &CTX);
        assert_eq!(cv.direction, Direction::Rtl);
        assert_eq!(sv.finalize_as_root().direction, Direction::Rtl);
    }

    /// root element では `rem` の基準が phase 2 と phase 3 で異なる
    /// (`SpecifiedValues::finalize_as_root` の doc に spec verbatim)。
    ///
    /// - `font-size: 2rem` → **32px** (initial 16px 基準 — font-\* property 上の
    ///   自己参照 unit なので parent-metrics 条項が発火する)
    /// - `padding: 2rem` → **40px** (自 font-size 20px 基準 — box property は
    ///   条項の対象外で `rem` は素の定義「root element の computed font-size」)
    #[test]
    fn finalize_as_root_uses_initial_for_font_size_and_own_for_box_properties() {
        let mut fs_case = SpecifiedValues::initial();
        fs_case.font_size = Length::Rem(2.0);
        assert_eq!(fs_case.finalize_as_root().font_size, ComputedLength(32.0));

        let mut box_case = SpecifiedValues::initial();
        box_case.font_size = Length::Px(20.0);
        box_case.padding = Sides::all(Length::Rem(2.0));
        let cv = box_case.finalize_as_root();
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(40.0));
    }

    /// root element の `font-size: Nem` も親が無いので initial 16px 基準。
    #[test]
    fn finalize_as_root_resolves_em_font_size_against_initial() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Em(1.5);
        assert_eq!(sv.finalize_as_root().font_size, ComputedLength(24.0));
    }

    /// root element の **box property** の `1rlh` は自分の確定済 line-height
    /// を基準にする (`rem_on_root_element_box_property_uses_own_font_size`
    /// の `rem` と同じ非対称の `rlh` 版。self-reference 条項の対象は
    /// `line-height` 自身の値だけで、`padding` はその対象外)。
    /// `finalize_as_root` は own line-height を phase 2.5 で確定させてから
    /// `ResolveContext::with_root_line_height` を組み立てる — この test は
    /// その配線がここまで届くことを直接 pin する。
    #[test]
    fn finalize_as_root_resolves_rlh_using_own_line_height_basis() {
        let mut sv = SpecifiedValues::initial();
        sv.font_size = Length::Px(20.0);
        sv.line_height = LineHeight::Number(2.0); // own font-size 20px → used 40px
        sv.padding = Sides::all(Length::Rlh(1.5)); // 1.5 * 40 = 60px
        let cv = sv.finalize_as_root();
        assert_eq!(cv.line_height, ComputedLineHeight::Number(2.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(60.0));
    }

    /// root element には親が無いので、`line-height` 自身の値としての
    /// `1lh`/`1rlh` (自己参照) は常に「initial values」= `normal` 基準に
    /// 帰着し、常に unresolved になる (CSS Values 4 §6.1.1 "if the element
    /// has no parent" — `finalize_as_root` doc 参照)。上の test と対で、
    /// 「box property の rlh は自分の line-height を使う」「line-height 自身の
    /// lh/rlh は self-reference で常に normal」の 2 つの非対称を区別する。
    #[test]
    fn finalize_as_root_line_height_self_reference_is_always_normal() {
        let mut sv = SpecifiedValues::initial();
        sv.line_height = LineHeight::Length(Length::Lh(1.0));
        assert_eq!(
            sv.finalize_as_root().line_height,
            ComputedLineHeight::Normal
        );

        let mut sv_rlh = SpecifiedValues::initial();
        sv_rlh.line_height = LineHeight::Length(Length::Rlh(1.0));
        assert_eq!(
            sv_rlh.finalize_as_root().line_height,
            ComputedLineHeight::Normal
        );
    }

    /// 全 4 side が独立に絶対化される (`Sides::map` が side を取り違えない)。
    #[test]
    fn finalize_absolutizes_each_side_independently() {
        let mut sv = SpecifiedValues::initial();
        sv.padding = Sides {
            top: Length::Px(1.0),
            right: Length::Em(1.0),
            bottom: Length::Pt(3.0),
            left: Length::Percent(5.0),
        };
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(1.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(16.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(4.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Percent(5.0));
    }

    /// `text-shadow` の list-shaped phase 3 — 直上
    /// `finalize_absolutizes_each_side_independently` の `Sides<Length>`
    /// precedent を可変長 list に一般化したもの。各 item の
    /// 3 length (`offset_x`/`offset_y`/`blur_radius`) が own-node font-size
    /// basis で独立に絶対化される (`resolve_text_shadow_item`)、`<color>` は
    /// 素通し ([`TextShadowColor`] doc)。
    #[test]
    fn finalize_absolutizes_each_text_shadow_item_independently() {
        let mut sv = SpecifiedValues::initial();
        sv.text_shadow = Arc::new(vec![
            TextShadowItem {
                offset_x: Length::Em(1.0),
                offset_y: Length::Rem(2.0),
                blur_radius: Length::Pt(3.0),
                color: TextShadowColor::CurrentColor,
            },
            TextShadowItem {
                offset_x: Length::Px(4.0),
                offset_y: Length::Px(5.0),
                blur_radius: Length::Px(0.0),
                color: TextShadowColor::Resolved(CssColor::BLACK),
            },
        ]);
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert_eq!(
            *cv.text_shadow,
            vec![
                ComputedTextShadow {
                    // 1em * own font-size (16px, `SpecifiedValues::initial`).
                    offset_x: ComputedLength(16.0),
                    // 2rem * root font-size (16px, `CTX`).
                    offset_y: ComputedLength(32.0),
                    // 3pt = 3 * 4/3 px = 4px.
                    blur_radius: ComputedLength(4.0),
                    color: TextShadowColor::CurrentColor,
                },
                ComputedTextShadow {
                    offset_x: ComputedLength(4.0),
                    offset_y: ComputedLength(5.0),
                    blur_radius: ComputedLength::ZERO,
                    color: TextShadowColor::Resolved(CssColor::BLACK),
                },
            ]
        );
    }

    /// 空 list (`none`) は allocation せず shared computed-empty-Arc slot を
    /// 再利用する — [`Self::absolutize_with`] の `text_shadow` arm doc 参照。
    #[test]
    fn finalize_empty_text_shadow_list_reuses_shared_computed_empty_arc() {
        let sv = SpecifiedValues::initial();
        assert!(sv.text_shadow.is_empty());
        let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
        assert!(cv.text_shadow.is_empty());
        assert!(Arc::ptr_eq(
            &cv.text_shadow,
            &ComputedValues::initial().text_shadow
        ));
    }
}
