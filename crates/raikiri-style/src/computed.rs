//! Per-node computed CSS values.
//!
//! Cascade + inheritance walk が populate。将来 ComputedValues → taffy::Style
//! + paint 用色情報の抽出 layer が入る予定。
//!
//! 現サポート property の一覧と inherited / non-inherited 分類は
//! [`ComputedValues`] 定義の field doc comment を参照。

use std::sync::Arc;

use smol_str::SmolStr;

use crate::Atom;
use crate::property::{
    AlignSelfValue, BorderColor, BorderStyle, BoxSizing, BreakBetween, BreakInside, ClearValue,
    ContentAlignmentValue, ContentComponent, CssColor, Direction, DisplayValue, FlexDirectionValue,
    FlexWrapValue, FloatValue, FontStyle, OverflowValue, OverflowWrap, OverflowXY,
    SelfAlignmentValue, Sides, TextAlign, TextDecorationColor, TextDecorationLine,
    TextDecorationStyle, TextTransform, VerticalAlign, Visibility, WhiteSpace, WordBreak,
    ZIndexValue, empty_content_list, empty_counter_entries, empty_string_set_entries,
    initial_font_family,
};
use crate::resolve::{
    ComputedBorder, ComputedFlexBasis, ComputedLength, ComputedLengthPercentage,
    ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal, ComputedLineHeight,
    ComputedTextShadow, empty_computed_text_shadow_list,
};

/// CSS spec 上の `font-size` initial value (`medium`) に対応する px 値。
///
/// CSS Fonts 4 §2.5 "Font size: the font-size property"
/// (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>) は "Initial: medium"
/// と規定し、`medium` の実 px は UA 依存。本実装は browser default の 16px を
/// 採る。
///
/// **非 test code で 16px を書く単一 source** —
/// [`ComputedValues::initial`] の `font_size` と
/// [`crate::resolve::ResolveContext::initial`] の `root_font_size` が参照する。
/// 両者は同一値でなければならず、独立に literal を持つと乖離を型検査で拾えない。
///
/// 一方「initial value が **16px そのものである**」ことの pin は test 側が
/// literal で持つ。**これらを「一貫性のため」本 const への参照に書き換えては
/// ならない** — 全体が自己参照になり、const の誤編集を何も検出できなくなる。
/// 該当 test は本 const を `20.0` 等に摂動すれば列挙できる (lib test が
/// fail-fast して doctest section まで到達しないので、
/// `cargo test -p raikiri-style` と `--doc` を別々に走らせること)。
pub(crate) const INITIAL_FONT_SIZE_PX: f32 = 16.0;

/// `position: running(<custom-ident>)` により登録された template の cascade-time seed。
///
/// CSS GCPM 3 §1.2.1 "The running() value"
/// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>: `position: running(name)`
/// された element は body flow から除去、`@page` margin box の
/// `content: element(name)` から参照される。
///
/// 本 struct は design doc §7.3 の **2-tier キャッシュ** の static side seed —
/// cascade で per-node に `name` を捕捉し、下流 (raikiri-dom) 側が subtree_root /
/// pre-cascaded style / dynamic flags を association する
/// (`ParsedRunningTemplate` — 本 crate は leaf、DOM node identity を持たない)。
///
/// `#[non_exhaustive]` により future field (e.g. `alternative_hint` 等の per-name
/// override) を non-breaking で追加可能。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningTemplate {
    /// `running(<name>)` の name (case-preserved smol str)。
    pub name: SmolStr,
}

/// Per-node computed style。現サポート property と inheritance 分類は下記 field
/// doc を参照 (inherited: color / font-family / font-size / font-weight / text_align / direction / line_height / font_style / text_transform / visibility / text_indent / word_break / overflow_wrap / letter_spacing / word_spacing / white_space / text_shadow、
/// non-inherited: background-color / display / counter-* / content / string-set /
/// running_templates / padding / margin / border / width / height / box_sizing /
/// overflow / text_decoration / vertical_align / z_index / float / clear)。
///
/// # 層
///
/// 本 struct が保持するのは **computed value 層**の値だけである。length を運ぶ
/// field は [`crate::resolve`] の `Computed*` 型で、`em` / `rem` / `pt` は既に
/// px へ絶対化されている (CSS Cascade 5 §7.2
/// <https://www.w3.org/TR/css-cascade-5/#inheriting> が「inheritance が運ぶのは
/// computed value である」と規定するため、この絶対化は inheritance より前に
/// 済んでいなければならない)。`<percentage>` は property ごとに扱いが異なる —
/// 各 field doc を参照。
///
/// 絶対化前の staging 表現は [`crate::specified::SpecifiedValues`]。cascade
/// winner の適用はそちらに対して行い、[`SpecifiedValues::finalize`] が本 struct
/// を produce する。
///
/// [`SpecifiedValues::finalize`]: crate::specified::SpecifiedValues::finalize
///
/// `#[non_exhaustive]` により future property (box-shadow 等) の追加が
/// non-breaking。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedValues {
    /// `color`。inherited、initial: opaque black。
    pub color: CssColor,
    /// `background-color`。**non-inherited**、initial: `transparent`
    /// (= [`CssColor::TRANSPARENT`])。CSS Backgrounds 3 §2.2 "Base Color:
    /// the background-color property"
    /// <https://www.w3.org/TR/css-backgrounds-3/#background-color>。
    pub background_color: CssColor,
    /// `font-family` — 優先順位順。inherited。CSS Fonts 4 §2.1
    /// <https://www.w3.org/TR/css-fonts-4/#font-family-prop> の spec 上の
    /// initial は "depends on user agent" — spec は具体的な family name を
    /// 規定しない ([`crate::property::initial_font_family`] doc 参照)。
    /// 本実装は `[Atom::from("serif")]` を採る。
    ///
    /// [`Arc<Vec<..>>`] wrap: inheritance walk clone
    /// (`SpecifiedValues::inherit_from` の `parent.font_family.clone()`、
    /// `resolve_inheritance` の stack push + `out[idx] = computed.clone()`) が
    /// **shallow (Arc bump)** になる。`font-family` は inherited property なので、
    /// [`Self::counter_reset`] 等 (non-inherited) とはコストの形が異なる —
    /// 「毎 node で initial にリセットする」コストではなく「inheritance walk が
    /// 毎 node で値を運ぶ」コストで、N-node document あたり O(N) の 1-element
    /// `Vec` malloc になっていた (out-of-diff pre-existing の perf 特性として発見)。
    /// `Arc<Vec<T>>: Deref<Target = Vec<T>>` により downstream の `.iter()` /
    /// `.len()` / `.is_empty()` は既存 pattern そのままで通る (dom/paint consumer
    /// 波及 0)。
    pub font_family: Arc<Vec<Atom>>,
    /// `font-size`。**inherited**、initial: 16px (spec は `medium`、実 px は
    /// UA 依存)。CSS Fonts 4 §2.5 "Font size: the font-size property"
    /// <https://www.w3.org/TR/css-fonts-4/#propdef-font-size> は
    /// "Computed value: an absolute length" と規定する。
    ///
    /// 型は [`ComputedLength`] — `em` (親基準) / `rem` (root 基準) / `<percentage>`
    /// (親基準) / `pt` は cascade の phase 2
    /// ([`crate::resolve::resolve_font_size`]) で px へ絶対化済み。
    pub font_size: ComputedLength,
    /// `font-weight`。inherited、initial: 400.0 (normal)。
    ///
    /// **常に resolve 済みの absolute weight** (`[1, 1000]`)。specified value 側の
    /// `bolder` / `lighter` sentinel ([`crate::property::FontWeightValue`]) は
    /// [`crate::cascade::apply_value`] が親の computed weight と CSS Fonts 4 §2.2
    /// の table から絶対値に解決してから書き込むため、この field に relative
    /// keyword が残ることはない。これは spec とも一致する — §2.2 の property
    /// table は `Computed value: a number, see below` と規定し、§2.2.1
    /// "Relative Weights" が "Specified values of `bolder` and `lighter`
    /// indicate weights relative to the weight of the parent element. The
    /// computed weight is calculated based on the inherited `font-weight`
    /// value" と規定している
    /// (<https://www.w3.org/TR/css-fonts-4/#relative-weights>)。
    ///
    /// 型は `f32` (旧 `u16` から格上げ) — §2.2.2 "Missing
    /// weights" <https://www.w3.org/TR/css-fonts-4/#missing-weights> "Fractional
    /// weights are valid" どおり computed value の fractional 精度を保持する。
    /// `u16` だった当時は整数化のため round-half-away-from-zero を要し、その丸めが
    /// §2.2.1 relative-weight table の行選択を変える 2 次被害を伴う既知
    /// divergence だった (本 field の型変更で解消)。
    ///
    /// `raikiri-dom` の layout はこの field を直接 `parley::FontWeight::new(f32)`
    /// に渡す (キャスト不要) —
    /// **渡す直前** に `sanitize_font_weight` (`crates/raikiri-dom/src/
    /// layout.rs`) が sink 境界 guard を掛ける。
    ///
    /// # `[1, 1000]` / finite は caller が維持する contract (本 field 自体に guard なし)
    ///
    /// 本 struct の field は全て `pub` であり、cascade を経由せず直接
    /// `ComputedValues { font_weight: ..., .. }` を構築することを妨げない
    /// (例: [`crate::page::cascade_page`] の継承元 root 引数)。`u16` だった頃は
    /// 非有限値がそもそも型で構成不可能だったが、`f32` 化
    /// でこの保証は「型」から「呼び出し元の値検証」に変わった — cascade を
    /// 経由する通常経路は `parse_font_weight` (private、
    /// [`crate::property::FontWeightValue`] の doc 参照) の `[1, 1000]` range
    /// guard により常に finite だが、直接構築はその guard を経ない。
    /// `NaN` / `±Inf` が渡った場合の [`crate::cascade::resolve_relative_weight`]
    /// (`bolder`/`lighter` 解決) の挙動は同関数の doc で characterize 済み。
    ///
    /// **本 field / `resolve_relative_weight` のどちらにも guard は追加しない**
    /// (guard は sink 境界に置く、resolve/computed 層の public surface は
    /// sanitize しない、という設計判断による — carve-out 2)。
    /// この field を直接読む他の consumer (raikiri-dom 以外、例:
    /// umbrella 経由の外部 consumer) は自分の sink 境界で同様の guard を
    /// 持つ責務を負う (同じ carve-out 2 の理屈)。
    pub font_weight: f32,
    /// `line-height`。**inherited**、initial: [`ComputedLineHeight::Normal`]。
    /// CSS Inline 3 §5.1 "Line Spacing: the line-height property"
    /// <https://www.w3.org/TR/css-inline-3/#line-height-property>。
    ///
    /// spec の "Computed value: the specified keyword, a number, or a computed
    /// `<length>` value" に 1:1 対応する 3 variant
    /// ([`ComputedLineHeight`])。**computed 層に percentage は存在しない** —
    /// `<percentage>` は宣言要素の computed font-size に対して cascade の
    /// phase 3 で絶対化される (§5.1 "Percentages: computed relative to 1em")。
    ///
    /// [`ComputedLineHeight::Number`] (unitless multiplier) は computed 層でも
    /// number のまま残る。これは §5.1 の "child inherits the specified value"
    /// special behavior に対応し、child の font-size で再乗算する責務を下流
    /// (paint) に残す。逆に
    /// [`ComputedLineHeight::Length`] は宣言要素で確定した px であり、
    /// child は**再解決せずそのまま継承する**。
    pub line_height: ComputedLineHeight,
    /// `display`。**non-inherited**、initial: `DisplayValue::Inline`
    /// (CSS Display 3 §2 <https://www.w3.org/TR/css-display-3/#propdef-display>)。
    /// 現状 `block` / `inline` / `inline-block` / `none`
    /// (詳細は [`DisplayValue`] doc)。
    pub display: DisplayValue,
    /// `counter-reset`。**non-inherited**。spec initial は `none`
    /// (CSS Lists 3 §4.1 <https://www.w3.org/TR/css-lists-3/#counter-reset>)、
    /// 本 impl はそれを空 list で表現する。
    /// counter-name + initial value pairs。counter tree の実 resolve に向けた
    /// pre-work であり、resolve 本体は将来別途実装する。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`apply_winners` の drain での
    /// `value.clone()`、`apply_value` move) と inheritance walk clone
    /// (`resolve_inheritance` の stack push + `out[idx] = computed.clone()`) が
    /// **shallow (Arc bump)** になる。
    /// `* { counter-reset: c0 c1 ... cN }` × M element の O(N × M) memory
    /// blow-up を単一 heap slot 共有で塞ぐ (security-relevant な DoS 対策、
    /// Content/StringSet と同 pattern の踏襲)。`Arc<Vec<T>>: Deref<Target = Vec<T>>`
    /// により downstream の `.iter()` / `.len()` / `.is_empty()` は既存 pattern
    /// そのままで通る (dom/paint consumer 波及 0)。
    pub counter_reset: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-increment`。**non-inherited**。spec initial は `none`
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#increment-set>)、
    /// 本 impl はそれを空 list で表現する。
    /// counter-name + increment pairs。counter tree の実 resolve に向けた
    /// pre-work であり、resolve 本体は将来別途実装する。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::counter_reset`] と同 rationale。
    pub counter_increment: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-set`。**non-inherited**。spec initial は `none`
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#increment-set>)、
    /// 本 impl はそれを空 list で表現する。
    /// counter-name + value pairs。counter tree の実 resolve に向けた
    /// pre-work であり、resolve 本体は将来別途実装する。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::counter_reset`] と同 rationale。
    pub counter_set: Arc<Vec<(SmolStr, i32)>>,
    /// `content` の resolved 中間表現。**non-inherited**。spec initial は
    /// `normal` (CSS Content 3 §1 propdef-content "Initial: normal") で、本 impl
    /// は `normal` / `none` をどちらも空 list で表現する (本 crate は cascade
    /// static side に留まり、pseudo-element 生成判断は下流 layer)。
    /// 将来の GCPM directive emit に向けた pre-work。
    /// 下流 (raikiri-dom) が `raikiri_traits::ContentValueItem` に mapping する
    /// (raikiri-style は raikiri-traits に依存しない leaf crate、counter-* の
    /// wire-through pattern を踏襲)。
    /// See <https://www.w3.org/TR/css-content-3/#content-property>.
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`apply_winners` の drain での
    /// `value.clone()`、`apply_value` move) と inheritance walk clone
    /// (`resolve_inheritance` の stack push + `out[idx] = computed.clone()`) が
    /// **shallow (Arc bump)** になる。
    /// `* { content: "<large>" }` × N element の O(N × M) memory blow-up
    /// を単一 heap slot 共有で塞ぐ (security-relevant な DoS 対策)。
    /// `Arc<Vec<T>>: Deref<Target = Vec<T>>` により downstream の `.iter()` /
    /// `.len()` / `.is_empty()` は既存 pattern そのままで通る (dom/paint
    /// consumer 波及 0)。
    pub content: Arc<Vec<ContentComponent>>,
    /// `string-set` の parse 結果 — `(name, content-list)` entry の列。
    /// **non-inherited**。spec initial は `none` (CSS GCPM 3 §1.1.1
    /// propdef-string-set "Initial: none")、本 impl はそれを空 list で表現する。
    /// 名前解決と runtime `string()` 参照は下流 (raikiri-dom) 責務。
    /// See <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>.
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::content`] と同 rationale
    /// (`* { string-set: name "<large>" }` × N element の同種 DoS 経路を塞ぐ)。
    pub string_set: Arc<Vec<(SmolStr, Vec<ContentComponent>)>>,
    /// `position: running(<custom-ident>)` の seed。**non-inherited**。spec
    /// initial は `position: static` (running() seed 無し、CSS GCPM 3 §1.2.1
    /// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>)、本 impl はそれを
    /// 空 list で表現する。
    ///
    /// 本 field は **per-node で常に 0 または 1 要素** (`position` は spec 上
    /// 単一値の property、element は最大 1 つの `running(name)` しか持たない):
    /// - `position: static` / 他 keyword / rule 無し → empty
    /// - `position: running(name)` → `[RunningTemplate { name }]`
    ///
    /// `Vec` shape を採るのは `content` / `string_set` と同じ
    /// SmolStr wire-through pattern の踏襲 (原則 1 前例主義)。下流 (raikiri-dom)
    /// が per-document `Vec<RunningTemplate>` を組み立てる際に per-node seed を
    /// concatenate する。design doc §7.3 の 2-tier キャッシュ static side に相当。
    pub running_templates: Vec<RunningTemplate>,
    /// `text-align`。**inherited**、initial: [`TextAlign::Start`]
    /// (CSS Text 3 §6.1 "Text Alignment: the text-align shorthand"
    /// <https://www.w3.org/TR/css-text-3/#text-align-property>)。
    ///
    /// spec 上 shorthand (text-align-all + text-align-last の 2 longhand を set)
    /// だが、現状は **shorthand as single field** convention (margin
    /// `Sides<T>` / content-normal-none-as-empty-list precedent) を踏襲して単一
    /// field に保持 (longhand 分離 §6.2 / §6.3 は将来 defer)。詳細は
    /// [`TextAlign`] doc-comment。
    ///
    /// sibling field: [`color`](Self::color) / [`font_family`](Self::font_family) /
    /// [`font_size`](Self::font_size) / [`font_weight`](Self::font_weight) と同じ
    /// **inherited** 系 — `inherit_from` の inherited block に配置し親から by-value
    /// copy (`TextAlign` は `Copy`)。
    ///
    /// **`MatchParent` が本 field の値として観測されることは無い** —
    /// cascade winner が `match-parent` でも、computed 層に届く前に
    /// [`crate::property::resolve_text_align_match_parent`] が親の
    /// [`Self::text_align`] + [`Self::direction`] を使って `Left` / `Right` /
    /// (親値のコピー) に解決する。解決は 2 箇所 — element 経路
    /// ([`crate::specified::SpecifiedValues::finalize`] /
    /// `finalize_as_root`) と page 経路
    /// ([`crate::cascade::resolve_against_inherited`]) — から同じ関数へ
    /// funnel する。
    pub text_align: TextAlign,
    /// `direction`。**inherited**、initial: [`Direction::Ltr`]
    /// (CSS Writing Modes 4 §2.1 "Specifying Directionality: the direction
    /// property" <https://www.w3.org/TR/css-writing-modes-4/#direction>)。
    /// Computed value = specified value (相対解決なし、[`Direction`] doc 参照)。
    ///
    /// sibling field: [`text_align`](Self::text_align) と同じ **inherited** 系 —
    /// `inherit_from` の inherited block に配置し親から by-value copy
    /// (`Direction` は `Copy`)。
    ///
    /// 追加理由: [`text_align`](Self::text_align) の
    /// `match-parent` 解決 (CSS Text 3 §6.1) が親の computed `direction` を
    /// 要求する。property 自体は CSS Paged Media 3 Appendix A
    /// page-property-list <https://www.w3.org/TR/css-page-3/#page-property-list>
    /// にも独立に載っており、`@page` context でも意味を持つ。
    pub direction: Direction,
    /// `text-indent` — first-line indentation of a block container.
    /// **inherited**、initial: [`ComputedLengthPercentage::Px`]`(0.0)` (CSS
    /// Text 3 §8.1 "First Line Indentation: the text-indent property"
    /// <https://www.w3.org/TR/css-text-3/#text-indent-property>: "Initial:
    /// 0", "Applies to: block containers", "Inherited: yes", "Percentages:
    /// refers to block container's own inline-axis inner size", "Computed
    /// value: computed `<length-percentage>` value, plus any specified
    /// keywords"). The full grammar is
    /// `<length-percentage> && hanging? && each-line?`.
    ///
    /// This field carries **only** the `<length-percentage>` component of
    /// the grammar — `hanging`/`each-line` are not implemented at this
    /// crate's scope ([`crate::property::PropertyValue::TextIndent`] doc's
    /// "Scope carving" section).
    ///
    /// Value shape mirrors [`Self::padding`] (`%` stays a computed
    /// `<percentage>`, resolved only at the used-value layer — downstream
    /// (taffy) responsibility, same [`ComputedLengthPercentage`] type) — but
    /// **not** the reference quantity: `padding`'s `%` resolves against the
    /// containing block's width (CSS Box 3 §4.1), while `text-indent`'s `%`
    /// resolves against the block container's own inline-axis inner size
    /// (the propdef's "Percentages" line above) — a different quantity, not
    /// merely different terminology for the same one. The other difference
    /// from `padding` is that this property **is** inherited: a node with no
    /// `text-indent` declaration of its own gets this value copied from its
    /// parent's already-absolutized computed value
    /// ([`crate::specified::SpecifiedValues::inherit_from`]'s
    /// `lift_length_percentage` seed), not reset to the initial `0`.
    pub text_indent: ComputedLengthPercentage,
    /// `padding` — 4-side box-model padding。**non-inherited**、initial:
    /// `Sides::all(ComputedLengthPercentage::Px(0.0))` — CSS Box 3 §4.1
    /// <https://www.w3.org/TR/css-box-3/#padding-physical> initial "0"。
    ///
    /// - Physical longhands: [`padding-top`](https://www.w3.org/TR/css-box-3/#propdef-padding-top) /
    ///   `padding-right` / `padding-bottom` / `padding-left`。
    /// - Shorthand: [`padding` (§4.2)](https://www.w3.org/TR/css-box-3/#padding-shorthand)。
    ///
    /// Value grammar: `<length-percentage [0,∞]>` — non-negative constraint は
    /// parse-time enforce ([`crate::property::PropertyValue::PaddingTop`] doc 参照)。
    /// "Computed value: a computed `<length-percentage>` value" のとおり
    /// `Px` / `Percent` の 2 形態 ([`ComputedLengthPercentage`]) を取る。
    /// **`<percentage>` は computed 層に残る** — 参照値 (containing block width)
    /// が used value 層でしか決まらないため (CSS Cascade 5 §4.5
    /// <https://www.w3.org/TR/css-cascade-5/#used>、raikiri では taffy 委譲)。
    pub padding: Sides<ComputedLengthPercentage>,
    /// `margin` 4-side quad (top / right / bottom / left)。**non-inherited**、
    /// initial: `0` on each side (`Sides::all(ComputedLengthPercentageOrAuto::Px(0.0))`).
    ///
    /// Author CSS の box model 中核 property。`em` / `rem` / `pt` は cascade の
    /// phase 3 で px に絶対化済み。`<percentage>` と `auto` は computed 層に残り
    /// (spec "Computed value: the keyword `auto` or a computed
    /// `<length-percentage>` value")、containing block に対する解決と余白分配は
    /// used value 層 = 下流 (taffy) の責務。
    ///
    /// # Primary sources (§ title + anchor)
    ///
    /// - CSS Box 3 §3.1 "Page-relative (Physical) Margin Properties":
    ///   [`margin-top` / `margin-right` / `margin-bottom` / `margin-left`](https://www.w3.org/TR/css-box-3/#margin-physical)
    ///   — "Value: `<length-percentage> | auto`", "Initial: 0", "Inherited: no",
    ///   "Applies to: all elements except internal table elements".
    /// - CSS Box 3 §3.2 "Margin Shorthand: the margin property":
    ///   [`margin`](https://www.w3.org/TR/css-box-3/#margin-shorthand) —
    ///   "Value: `<'margin-top'>{1,4}`", 1/2/3/4 value expansion rules.
    pub margin: Sides<ComputedLengthPercentageOrAuto>,
    /// `border` — 4-side box-model border (width / style / color × 4 side)。
    /// **non-inherited**、initial: 各 side が `width` =
    /// [`ComputedLength::ZERO`] / `style` = [`BorderStyle::None`] / `color` =
    /// [`BorderColor::CurrentColor`] (`width` / `style` は crate 内部 field、
    /// consumer からは [`ComputedBorder::width`] / [`ComputedBorder::style`]
    /// accessor 経由で読む)。
    ///
    /// specified の initial は width = `medium` (= 3px) だが、**computed 層では
    /// 0px** になる — CSS Backgrounds 3 §3.3
    /// <https://www.w3.org/TR/css-backgrounds-3/#border-width> の propdef が
    /// "Computed value: absolute length, snapped as a border width; zero if the
    /// border style is `none` or `hidden`" と規定し、initial の border-style が
    /// `none` であるため ([`crate::resolve::resolve_border`] が gate する)。
    ///
    /// - Physical longhands: [`border-top-width`](https://www.w3.org/TR/css-backgrounds-3/#border-width) /
    ///   `border-top-style` / `border-top-color` × 4 side。
    /// - Shorthand: [`border` (§3.4)](https://www.w3.org/TR/css-backgrounds-3/#border-shorthands)。
    ///
    /// # Value semantics
    ///
    /// - `Sides<ComputedBorder>: Copy` により per-node write は bit-copy
    ///   (ComputedBorder は f32/enum/BorderColor payload の POD 集合)。
    /// - `width` は [`ComputedLength`] (px)。`<percentage>` は spec grammar に
    ///   含まれないため computed 層でも length のみで足りる
    ///   ([`crate::property::PropertyValue::BorderTopWidth`] doc 参照)。
    /// - `color` は cascade static side で [`BorderColor`] enum として保持し、
    ///   spec `currentcolor` keyword vs. 明示 `<color>` の specified-value
    ///   distinction を preserve する (CSS Backgrounds 3 §3.1
    ///   <https://www.w3.org/TR/css-backgrounds-3/#border-color> initial:
    ///   currentcolor)。used-value resolution (currentcolor → 同 node の
    ///   computed `color` property lookup、CSS Color 3 §4.4) は paint scope
    ///   責務。
    ///
    /// # Non-goals
    ///
    /// - `border-image-*` sub-property (source/slice/width/outset/repeat) は
    ///   未着手、shorthand `border:` も border-image を reset しない
    ///   (spec deviation 明示、future 統合 task で対応)。
    /// - `border-{top,right,bottom,left}` 4-side single-side shorthand (例:
    ///   `border-top: 1px solid red`) は現状未対応、future 追加。
    /// - `currentcolor` の cascade-side enum 保持は解消済み。
    ///   used-value resolution (paint scope) は defer。
    ///
    /// # Primary sources
    ///
    /// - CSS Backgrounds 3 §3 "Borders":
    ///   [`border-width`](https://www.w3.org/TR/css-backgrounds-3/#border-width) /
    ///   [`border-style`](https://www.w3.org/TR/css-backgrounds-3/#border-style) /
    ///   [`border-color`](https://www.w3.org/TR/css-backgrounds-3/#border-color) /
    ///   [`border shorthand`](https://www.w3.org/TR/css-backgrounds-3/#border-shorthands)。
    pub border: Sides<ComputedBorder>,
    /// `width` — preferred physical horizontal size (writing-mode neutral な
    /// physical property、vertical writing mode では block axis に対応)。
    /// **non-inherited**、initial: [`ComputedLengthPercentageOrAuto::Auto`] (CSS Sizing 3 §3.1.1
    /// "Preferred Size Properties"
    /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
    ///
    /// spec value grammar は `auto | <length-percentage [0,∞]> | min-content |
    /// max-content | fit-content(<length-percentage>)` だが、現状は
    /// `auto` + non-negative `<length-percentage>` のみ受理する (min-content /
    /// max-content / fit-content() は未着手)。
    /// 負値は spec grammar `[0,∞]` violation として parse-time drop
    /// ([`crate::property::PropertyValue::Width`] doc + `parse_width` 参照)。
    ///
    /// spec の "Computed value: as specified, with `<length-percentage>` values
    /// computed" のとおり、length は px へ絶対化され percentage は残る
    /// ([`ComputedLengthPercentageOrAuto`]、margin と同型)。`Auto` は CSS
    /// Sizing 3 の automatic size calculation (containing block width から
    /// margin/border/padding を差し引いた値を used-value に採る) として下流
    /// layout で解決する — margin `auto` の余白分配とは意味が異なる。
    pub width: ComputedLengthPercentageOrAuto,
    /// `height` — preferred vertical size。**non-inherited**、initial:
    /// `ComputedLengthPercentageOrAuto::Auto` (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
    /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>、
    /// spec 明記 "Initial: auto", "Inherited: no")。
    ///
    /// 現状は `auto` + 非負
    /// `<length-percentage>` の 2 分岐のみ受理 — `min-content` / `max-content`
    /// / `fit-content(<length-percentage>)` は spec-valid だが未サポートとして
    /// parser 段で silent drop
    /// (`parse_height` doc 参照)。
    ///
    /// [`ComputedLengthPercentageOrAuto`] は sibling [`Self::width`] と同 shape を
    /// reuse (sibling field、payload 型は共通)。percentage の containing block 換算と
    /// `Auto` の実 layout 高さ計算は used value 層 = 下流 (taffy) 責務。
    ///
    /// # Primary source
    ///
    /// - CSS Sizing 3 §3.1.1 "Preferred Size Properties":
    ///   [`height`](https://www.w3.org/TR/css-sizing-3/#preferred-size-properties)
    ///   — "Value: auto | `<length-percentage [0,∞]>` | min-content |
    ///   max-content | fit-content(`<length-percentage [0,∞]>`)",
    ///   "Initial: auto", "Applies to: all elements except non-replaced
    ///   inlines", "Inherited: no", "Percentages: relative to containing block".
    pub height: ComputedLengthPercentageOrAuto,
    /// `box-sizing`。**non-inherited**、initial: [`BoxSizing::ContentBox`]
    /// (CSS Sizing 3 §3.3 "Box Edges for Sizing: the box-sizing property"
    /// <https://www.w3.org/TR/css-sizing-3/#box-sizing>、"Initial: `content-box`"
    /// / "Inherited: no")。computed value = specified keyword。
    ///
    /// spec note (§3.3): "The definition of the box-sizing property in this
    /// module supersedes the one in [CSS-UI-3]" — CSS-UI-3 の box-sizing
    /// 定義は本 module により supersede されるため、css-sizing-3 が authoritative
    /// source。
    ///
    /// sibling field: [`display`](Self::display) / [`background_color`](Self::background_color)
    /// と同じ **non-inherited** 系 — `inherit_from` の non-inherited block に
    /// 配置し initial 値を直接指定 (親からコピーしない)。
    ///
    /// # Downstream handoff (future scope、style-scope confined)
    ///
    /// 本 field は cascade static side seed のみ保持し、
    /// `apply_computed_to_style` bridge (dom scope、`taffy::Style::box_sizing`
    /// への翻訳) は future cross-scope task に defer (Non-goals)。
    pub box_sizing: BoxSizing,
    /// `overflow-x` + `overflow-y`. **non-inherited**, initial:
    /// [`OverflowXY::both`]`(`[`OverflowValue::Visible`]`)` (CSS Overflow
    /// Module Level 3 §3.1 "Overflow: the overflow-x, overflow-y,
    /// overflow-block, overflow-inline, and overflow properties"
    /// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>, "Initial:
    /// visible" / "Inherited: no").
    ///
    /// Computed value is **not** simply the specified keyword — CSS Overflow
    /// 3 §3.1 defines a cross-axis coupling ("The visible/clip values of
    /// overflow compute to auto/hidden (respectively) if one of overflow-x
    /// or overflow-y is neither visible nor clip"), applied by
    /// [`resolve_overflow`](crate::property::resolve_overflow) in phase 3.
    /// Bundled into one [`OverflowXY`] field (rather than two independent
    /// scalar fields) for the same reason [`Self::border`] bundles
    /// `border-*-width`/`border-*-style` into [`Sides<ComputedBorder>`] — the
    /// coupling needs both axes at once ([`OverflowXY`] doc).
    ///
    /// # Downstream handoff (future scope, style-scope confined)
    ///
    /// This field carries the cascade static side seed only, mirroring
    /// [`Self::box_sizing`] — the block-formatting-context establishment and
    /// float-clearing consequences of `overflow != visible` (CSS 2.1 §9.4.1 /
    /// §9.5) are dom/paint scope and deferred to a follow-up task (see the
    /// `overflow` UA rule comment in
    /// `crates/raikiri-html/src/ua/minimal.css`).
    pub overflow: OverflowXY,
    /// `text-decoration-line`. **non-inherited**, initial:
    /// [`TextDecorationLine::NONE`] (CSS Text Decoration Module Level 3
    /// §2.1 "Text Decoration Lines: the text-decoration-line property"
    /// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-line-property>,
    /// "Initial: none" / "Inherited: no"). Computed value = specified
    /// keyword(s) ([`TextDecorationLine`] doc — no length payload, so no
    /// relative resolution is needed).
    ///
    /// # Downstream handoff (future scope, style-scope confined)
    ///
    /// This field carries the cascade static side seed only, mirroring
    /// [`Self::box_sizing`] / [`Self::overflow`] — actually painting the
    /// decoration line is raikiri-paint scope and not yet wired
    /// (`crates/raikiri-paint/src/lib.rs`'s module doc lists "Text
    /// decoration (underline / line-through)" as a future milestone).
    pub text_decoration_line: TextDecorationLine,
    /// `text-decoration-style`. **non-inherited**, initial:
    /// [`TextDecorationStyle::Solid`] (CSS Text Decoration Module Level 3
    /// §2.2 "Text Decoration Style: the text-decoration-style property"
    /// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-style-property>,
    /// "Initial: solid" / "Inherited: no"). Computed value = specified
    /// keyword ([`TextDecorationStyle`] doc).
    ///
    /// Downstream handoff mirrors [`Self::text_decoration_line`] — the
    /// paint-side rendering of a non-`solid` style is not yet wired.
    pub text_decoration_style: TextDecorationStyle,
    /// `text-decoration-color`. **non-inherited**, initial:
    /// [`TextDecorationColor::CurrentColor`] (CSS Text Decoration Module
    /// Level 3 §2.3 "Text Decoration Color: the text-decoration-color
    /// property"
    /// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-color-property>,
    /// "Initial: currentcolor" / "Inherited: no"). Computed value =
    /// computed color ([`TextDecorationColor`] doc — used-value resolution
    /// of `currentcolor` is paint scope responsibility, mirroring
    /// [`BorderColor`]).
    ///
    /// Downstream handoff mirrors [`Self::text_decoration_line`].
    pub text_decoration_color: TextDecorationColor,
    /// `vertical-align`. **non-inherited**, initial:
    /// [`VerticalAlign::Baseline`] (CSS 2.1 §10.8.1 "Vertical alignment: the
    /// 'vertical-align' property"
    /// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>,
    /// "Initial: baseline" / "Inherited: no"). Computed value: the 6
    /// keywords (`baseline`/`sub`/`super`/`middle`/`text-top`/`text-bottom`)
    /// stay the specified keyword; [`VerticalAlign::Length`] absolutizes to
    /// `Length::Px` ([`crate::resolve::resolve_vertical_align`] doc).
    ///
    /// # Scope carving
    ///
    /// This field holds the subset described on [`VerticalAlign`]'s own
    /// "Scope carving" doc — `top` / `bottom` keywords and the
    /// `<percentage>` value form remain explicit follow-up, not represented
    /// by this field.
    ///
    /// # Same type at both the specified and computed layer
    ///
    /// Unlike most length-bearing fields in this crate (`flex_basis` /
    /// `letter_spacing` / `border`, which all use a dedicated `Computed*`
    /// type distinct from their specified-layer type), this field keeps the
    /// **same** [`VerticalAlign`] type [`crate::specified::SpecifiedValues::vertical_align`]
    /// carries. This is forced by raikiri-paint: its
    /// `vertical_align_shift_px` function takes this crate's
    /// [`VerticalAlign`] by value directly, so introducing a separate
    /// computed-only type here would require a raikiri-paint signature
    /// change this crate's scope does not include. See
    /// [`crate::resolve::resolve_vertical_align`] doc for how the
    /// [`VerticalAlign::Length`] variant is absolutized without changing
    /// this field's type.
    ///
    /// # Downstream handoff
    ///
    /// This field carries the cascade static side value only, mirroring
    /// [`Self::text_decoration_line`] — the baseline-shift amount
    /// calculation and glyph rendering is raikiri-paint scope, not this
    /// crate's. raikiri-paint consumes this field for `sub`/`super` (a
    /// used-font-size-relative pixel offset applied at glyph draw time);
    /// `baseline` continues to contribute no offset by definition. The
    /// other 3 keywords (`middle`/`text-top`/`text-bottom`) and the
    /// absolutized `Length` payload fall through raikiri-paint's own
    /// `#[non_exhaustive]` wildcard fallback to the same 0px shift until
    /// that crate's own shift-calculation work lands — this crate's job
    /// ends at carrying the (now, for `Length`, absolutized) value through.
    /// This does not by itself make `sub`/`super` content appear inline
    /// with its surrounding text — raikiri-dom has no inline formatting
    /// context yet (every element, `inline` included, lays out as its own
    /// block row), which is a separate, larger, pre-existing gap this
    /// field's wiring does not close.
    pub vertical_align: VerticalAlign,
    /// `font-style`. **inherited**, initial: [`FontStyle::Normal`] (CSS
    /// Fonts Module Level 4 §2.4 "Font style: the font-style property"
    /// <https://www.w3.org/TR/css-fonts-4/#font-style-prop>, "Initial:
    /// normal" / "Inherited: yes"). Computed value = specified keyword —
    /// see [`FontStyle`] doc's "Scope carving" section (the spec's
    /// angle-bearing computed-value branch is unreachable at this crate's
    /// scope, since `oblique <angle>?` is not implemented).
    ///
    /// # Scope carving (minimal scope)
    ///
    /// This field holds only the `normal | italic` subset of the
    /// property's full `normal | italic | left | right | oblique <angle
    /// [-90deg,90deg]>?` grammar — see [`FontStyle`] doc.
    pub font_style: FontStyle,
    /// `text-transform`. **inherited**, initial: [`TextTransform::None`]
    /// (CSS Text Module Level 3 §2.1 "Case Transforms: the text-transform
    /// property" <https://www.w3.org/TR/css-text-3/#text-transform-property>,
    /// "Initial: none" / "Inherited: yes"). Computed value = specified
    /// keyword.
    ///
    /// # Scope carving (minimal scope)
    ///
    /// This field holds only the `none | capitalize | uppercase |
    /// lowercase` subset of the property's full `none | [capitalize |
    /// uppercase | lowercase] || full-width || full-size-kana` grammar —
    /// see [`TextTransform`] doc.
    pub text_transform: TextTransform,
    /// `visibility`. **inherited**, initial: [`Visibility::Visible`] (CSS
    /// Display Module Level 3 §4 "Invisibility: the visibility property"
    /// <https://www.w3.org/TR/css-display-3/#visibility>, "Initial: visible"
    /// / "Inherited: yes"). Computed value = specified keyword — see
    /// [`Visibility`] doc's "Scope carving" section (the spec's
    /// formatting-context-specific space-saving effect for `collapse` is
    /// downstream layout scope, not represented by this field).
    pub visibility: Visibility,
    /// `z-index`. **non-inherited**, initial: [`ZIndexValue::Auto`] (CSS2
    /// §9.9.1 "Specifying the stack level: the 'z-index' property"
    /// <https://www.w3.org/TR/CSS2/visuren.html#z-index>, "Initial: auto" /
    /// "Inherited: no"). Computed value = specified value ([`ZIndexValue`]
    /// doc — no length payload, so no relative resolution is needed).
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only — no consumer reads it
    /// yet. [`ZIndexValue`] doc's "Scope carving" section explains why:
    /// this crate's `position` property does not implement the CSS2
    /// `relative`/`absolute`/`fixed`/`sticky` keywords that "positioned
    /// elements" (the propdef's "Applies to" clause) presupposes, so there
    /// is no stacking-context/paint-order consumer to wire up yet.
    pub z_index: ZIndexValue,
    /// `word-break`. **inherited**, initial: [`WordBreak::Normal`] (CSS
    /// Text Module Level 3 §5.1 "Breaking Rules for Letters: the
    /// word-break property"
    /// <https://www.w3.org/TR/css-text-3/#word-break-property>, "Initial:
    /// normal" / "Inherited: yes"). Computed value = specified keyword
    /// ([`WordBreak`] doc — no length payload, so no relative resolution
    /// is needed).
    ///
    /// # Scope carving (minimal scope)
    ///
    /// This field holds only the `normal | keep-all | break-all` subset of
    /// the property's full `normal | keep-all | break-all | break-word`
    /// grammar — the deprecated `break-word` value is not represented, see
    /// [`WordBreak`] doc's "Scope carving" section.
    pub word_break: WordBreak,
    /// `overflow-wrap` (legacy name alias: `word-wrap`). **inherited**,
    /// initial: [`OverflowWrap::Normal`] (CSS Text Module Level 3 §5.4
    /// "Overflow Wrapping: the overflow-wrap (word-wrap) property"
    /// <https://www.w3.org/TR/css-text-3/#overflow-wrap-property>,
    /// "Initial: normal" / "Inherited: yes"). Computed value = specified
    /// keyword ([`OverflowWrap`] doc — no length payload, so no relative
    /// resolution is needed).
    ///
    /// `word-wrap` is not a separate field — `parse_value` dispatches both
    /// names to this same field's [`PropertyKey::OverflowWrap`]
    /// ([`OverflowWrap`] doc's "legacy alias" section).
    ///
    /// [`PropertyKey::OverflowWrap`]: crate::property::PropertyKey::OverflowWrap
    pub overflow_wrap: OverflowWrap,
    /// `letter-spacing`. **inherited**, initial: [`ComputedLength::ZERO`]
    /// (CSS Text Module Level 3 §7.2 "Tracking: the letter-spacing
    /// property" <https://www.w3.org/TR/css-text-3/#letter-spacing-property>,
    /// "Initial: normal" / "Inherited: yes"). Computed value: an absolute
    /// length — the spec's `normal` keyword computes to zero (§7.2 "No
    /// additional spacing is applied. Computes to zero."), so unlike
    /// [`Self::line_height`] this field never needs to carry the keyword at
    /// the computed layer; [`crate::resolve::resolve_length_or_normal`]
    /// collapses `normal` to `0` before this field is populated.
    ///
    /// Values may be negative (§7.2: "Values may be negative, but there may
    /// be implementation-dependent limits.") — this field does not clamp.
    ///
    /// **Non-goal**: §7.2's legacy `getComputedStyle()` resolved-value rule
    /// ("a computed letter-spacing of zero yields a resolved value of
    /// `normal`") is a CSSOM serialization detail this crate has no surface
    /// for.
    pub letter_spacing: ComputedLength,
    /// `word-spacing`. **inherited**, initial: [`ComputedLength::ZERO`] (CSS
    /// Text Module Level 3 §7.1 "Word Spacing: the word-spacing property"
    /// <https://www.w3.org/TR/css-text-3/#word-spacing-property>, "Initial:
    /// normal" / "Inherited: yes"). Same computed-value shape as
    /// [`Self::letter_spacing`] — see that field's doc.
    pub word_spacing: ComputedLength,
    /// `break-before` (legacy shorthand: `page-break-before`).
    /// **non-inherited**, initial: [`BreakBetween::Auto`] (CSS
    /// Fragmentation Module Level 3 §3.1 "Breaks Between Boxes: the
    /// break-before and break-after properties"
    /// <https://www.w3.org/TR/css-break-3/#break-between>, "Initial: auto"
    /// / "Inherited: no"). Computed value = specified keyword
    /// ([`BreakBetween`] doc — no length payload, so no relative
    /// resolution is needed).
    ///
    /// `page-break-before` is not a separate field — `parse_value`
    /// dispatches both names to this same field's
    /// [`PropertyKey::BreakBefore`] ([`BreakBetween`] doc's "legacy
    /// shorthand" section).
    ///
    /// # Scope carving (minimal scope)
    ///
    /// This field holds only the `auto | avoid | avoid-page | page`
    /// subset of the property's full 12-keyword grammar — see
    /// [`BreakBetween`] doc's "Scope carving" section.
    ///
    /// [`PropertyKey::BreakBefore`]: crate::property::PropertyKey::BreakBefore
    pub break_before: BreakBetween,
    /// `break-after` (legacy shorthand: `page-break-after`). Same shape as
    /// [`Self::break_before`] — see that field's doc (CSS Fragmentation
    /// Module Level 3 §3.1, [`BreakBetween`] doc).
    ///
    /// [`PropertyKey::BreakAfter`]: crate::property::PropertyKey::BreakAfter
    pub break_after: BreakBetween,
    /// `break-inside` (legacy shorthand: `page-break-inside`).
    /// **non-inherited**, initial: [`BreakInside::Auto`] (CSS Fragmentation
    /// Module Level 3 §3.2 "Breaks Within Boxes: the break-inside
    /// property" <https://www.w3.org/TR/css-break-3/#break-within>,
    /// "Initial: auto" / "Inherited: no"). Computed value = specified
    /// keyword ([`BreakInside`] doc — no length payload, so no relative
    /// resolution is needed; a smaller, disjoint value set from
    /// [`Self::break_before`]/[`Self::break_after`]'s [`BreakBetween`]).
    ///
    /// `page-break-inside` is not a separate field — same dispatch shape
    /// as [`Self::break_before`]'s doc describes.
    ///
    /// # Scope carving (minimal scope)
    ///
    /// This field holds only the `auto | avoid | avoid-page` subset of the
    /// property's full 5-keyword grammar — see [`BreakInside`] doc's
    /// "Scope carving" section.
    pub break_inside: BreakInside,
    /// `float`. **non-inherited**, initial: [`FloatValue::None`] (CSS2
    /// §9.5.1 "Positioning the float: the 'float' property"
    /// <https://www.w3.org/TR/CSS2/visuren.html#propdef-float>, "Initial:
    /// none" / "Inherited: no"). Computed value = specified value
    /// ([`FloatValue`] doc — no length payload, so no relative resolution
    /// is needed).
    ///
    /// CSS2 §9.7's forced `display` recomputation when this field is not
    /// [`FloatValue::None`] has already been applied to [`Self::display`]
    /// by the time this field is populated — see
    /// [`crate::property::resolve_display_for_float`] doc.
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only. Actual float
    /// positioning, shrink-to-fit width, and line-box shortening (CSS2
    /// §9.5's exclusion-area algorithm) are layout-time behavior
    /// (raikiri-dom scope) — [`FloatValue`] doc's "Scope carving" section.
    pub float: FloatValue,
    /// `clear`. **non-inherited**, initial: [`ClearValue::None`] (CSS2
    /// §9.5.2 "Controlling flow next to floats: the 'clear' property"
    /// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>, "Initial:
    /// none" / "Inherited: no"). Computed value = specified value
    /// ([`ClearValue`] doc — no length payload, so no relative resolution
    /// is needed).
    ///
    /// # Scope carving
    ///
    /// This field carries the cascaded value only. Clearance computation
    /// and the vertical displacement it produces are layout-time behavior
    /// (raikiri-dom scope) — [`ClearValue`] doc's "Scope carving" section.
    pub clear: ClearValue,
    /// `white-space`. **inherited**, initial: [`WhiteSpace::Normal`] (CSS
    /// Text Module Level 3 §3 "White Space and Wrapping: the white-space
    /// property" <https://www.w3.org/TR/css-text-3/#white-space-property>,
    /// "Initial: normal" / "Inherited: yes"). Computed value = specified
    /// keyword.
    ///
    /// # Scope carving
    ///
    /// This field holds only the `normal | pre | nowrap | pre-wrap |
    /// pre-line` subset of the property's full `normal | pre | nowrap |
    /// pre-wrap | break-spaces | pre-line` grammar — see [`WhiteSpace`]
    /// doc. It also carries the cascaded value only — no consumer reads it
    /// yet, same as [`Self::z_index`] doc's "Scope carving" section: the
    /// white-space collapsing / line-wrapping algorithm the spec's keyword
    /// table describes belongs to a text layout / line-breaking consumer
    /// (raikiri-dom / raikiri-paint) this crate does not implement yet.
    pub white_space: WhiteSpace,
    /// `flex-direction`. **non-inherited**, initial:
    /// [`FlexDirectionValue::Row`] (CSS Flexible Box Layout Module Level 1
    /// §5.1 <https://www.w3.org/TR/css-flexbox-1/#flex-direction-property>,
    /// "Inherited: no"). Computed value = specified keyword ([`FlexDirectionValue`]
    /// doc — no length payload).
    pub flex_direction: FlexDirectionValue,
    /// `flex-wrap`. **non-inherited**, initial: [`FlexWrapValue::NoWrap`]
    /// (CSS Flexible Box Layout Module Level 1 §5.2
    /// <https://www.w3.org/TR/css-flexbox-1/#flex-wrap-property>, "Inherited:
    /// no"). Computed value = specified keyword.
    pub flex_wrap: FlexWrapValue,
    /// `flex-grow`. **non-inherited**, initial: `0.0` (CSS Flexible Box
    /// Layout Module Level 1 §7.2.1
    /// <https://www.w3.org/TR/css-flexbox-1/#flex-grow-property>, "Inherited:
    /// no"). Computed value = specified number ("`<number [0,∞]>`") —
    /// [`crate::property`]'s parser enforces `[0,∞]` **and** finiteness at
    /// parse time (this field has no downstream sink guard between here and
    /// `taffy::Style::flex_grow`, unlike geometry fields that pass through
    /// `raikiri-dom`'s `sanitize_taffy`).
    pub flex_grow: f32,
    /// `flex-shrink`. **non-inherited**, initial: `1.0` (CSS Flexible Box
    /// Layout Module Level 1 §7.2.2
    /// <https://www.w3.org/TR/css-flexbox-1/#flex-shrink-property>,
    /// "Inherited: no"). Same `[0,∞]` + finite parse-time enforcement as
    /// [`Self::flex_grow`].
    pub flex_shrink: f32,
    /// `flex-basis`. **non-inherited**, initial:
    /// [`ComputedFlexBasis::Auto`] (CSS Flexible Box Layout Module Level 1
    /// §7.2.3 <https://www.w3.org/TR/css-flexbox-1/#flex-basis-property>,
    /// "Inherited: no"). Computed value: specified keyword (`auto` /
    /// `content`) or a computed `<length-percentage>` value
    /// ([`ComputedFlexBasis`] doc).
    pub flex_basis: ComputedFlexBasis,
    /// `justify-content`. **non-inherited**, initial:
    /// [`ContentAlignmentValue::Normal`] (CSS Box Alignment Module Level 3
    /// §5.1 <https://www.w3.org/TR/css-align-3/#propdef-justify-content>,
    /// "Inherited: no"). Computed value = specified keyword(s)
    /// ([`ContentAlignmentValue`] doc — shared with [`Self::align_content`]).
    pub justify_content: ContentAlignmentValue,
    /// `align-content`. **non-inherited**, initial:
    /// [`ContentAlignmentValue::Normal`] (CSS Box Alignment Module Level 3
    /// §5.1 <https://www.w3.org/TR/css-align-3/#propdef-align-content>,
    /// "Inherited: no"). Computed value = specified keyword(s).
    pub align_content: ContentAlignmentValue,
    /// `align-items`. **non-inherited**, initial:
    /// [`SelfAlignmentValue::Normal`] (CSS Box Alignment Module Level 3
    /// §7.2 <https://www.w3.org/TR/css-align-3/#propdef-align-items>,
    /// "Inherited: no"). Computed value = specified keyword(s).
    pub align_items: SelfAlignmentValue,
    /// `align-self`. **non-inherited**, initial: [`AlignSelfValue::Auto`]
    /// (CSS Box Alignment Module Level 3 §6.2
    /// <https://www.w3.org/TR/css-align-3/#propdef-align-self>, "Inherited:
    /// no"). Computed value = specified keyword(s).
    pub align_self: AlignSelfValue,
    /// `row-gap`. **non-inherited**, initial:
    /// [`ComputedLengthPercentageOrNormal::Normal`] (CSS Box Alignment
    /// Module Level 3 §8.1
    /// <https://www.w3.org/TR/css-align-3/#propdef-row-gap>, "Inherited:
    /// no"). Computed value: specified keyword, else a computed
    /// `<length-percentage>` value ([`ComputedLengthPercentageOrNormal`] doc).
    pub row_gap: ComputedLengthPercentageOrNormal,
    /// `column-gap`. **non-inherited**, initial:
    /// [`ComputedLengthPercentageOrNormal::Normal`] (CSS Box Alignment
    /// Module Level 3 §8.1
    /// <https://www.w3.org/TR/css-align-3/#propdef-column-gap>, "Inherited:
    /// no"). Same shape as [`Self::row_gap`].
    pub column_gap: ComputedLengthPercentageOrNormal,
    /// `text-shadow`. **inherited**, initial: empty list (= `none`) (CSS
    /// Text Decoration Module Level 3 §4
    /// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property>,
    /// "Initial: none" / "Inherited: yes"). Computed value: "a list, each
    /// item consisting of three absolute lengths plus a computed color"
    /// ([`ComputedTextShadow`] doc — `currentcolor` stays symbolic, used-value
    /// resolution is paint scope responsibility, mirroring
    /// [`Self::text_decoration_color`]).
    ///
    /// # Downstream handoff (future scope, style-scope confined)
    ///
    /// This field carries the cascade static side seed only, mirroring
    /// [`Self::text_decoration_line`] — actually painting the shadow
    /// (including the blur approximation) is raikiri-paint scope and not yet
    /// wired.
    pub text_shadow: Arc<Vec<ComputedTextShadow>>,
}

impl ComputedValues {
    /// CSS spec に沿った initial value。cascade で何も matching しなかった root
    /// node と、inheritance chain の terminate に使う。
    pub fn initial() -> Self {
        Self {
            color: CssColor::BLACK,
            // CSS Backgrounds 3 §2.2: background-color initial は `transparent`。
            background_color: CssColor::TRANSPARENT,
            // shared Arc slot — per-node allocation 回避 (`initial_font_family`
            // doc 参照)。
            font_family: initial_font_family(),
            font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
            font_weight: 400.0,
            // CSS Inline 3 §5.1: line-height initial は `normal` (font metrics
            // ascent+descent 相当を paint 側で resolve)。
            line_height: ComputedLineHeight::Normal,
            display: DisplayValue::Inline,
            // CSS Lists 3 §4: counter-* の spec initial は `none`、本 impl は
            // 空 list で表現する (anchor は field doc 参照)。
            // shared empty Arc slot — per-node allocation 回避
            // (property.rs `empty_counter_entries` doc 参照)。
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            // CSS Content 3 §1: content の spec initial は `normal`。本 impl は下流に
            // とって「no generated content」= 空 list で表現する。
            // shared empty Arc slot — per-node allocation 回避
            // (property.rs `empty_content_list` doc 参照)。
            content: empty_content_list(),
            // CSS GCPM 3 §1.1.1: string-set の spec initial は `none`。本 impl は
            // それを空 list で表現する。
            // same shared-empty-Arc pattern。
            string_set: empty_string_set_entries(),
            // CSS GCPM 3 §1.2.1: position: running() seed initial は empty
            // (position の initial は `static`、running(name) 無し)。
            running_templates: Vec::new(),
            // CSS Text 3 §6.1: text-align initial is `start`
            text_align: TextAlign::Start,
            // CSS Writing Modes 4 §2.1: direction initial is `ltr`。
            direction: Direction::Ltr,
            // CSS Text 3 §8.1: text-indent initial is `0`。
            text_indent: ComputedLengthPercentage::Px(0.0),
            // CSS Box 3 §4.1: padding initial = 0 (all 4 sides)。
            padding: Sides::all(ComputedLengthPercentage::Px(0.0)),
            // CSS Box 3 §3.1: margin-* physical の initial は `0` (`Sides::all(0)`
            // で全 4 side に spread)。
            margin: Sides::all(ComputedLengthPercentageOrAuto::Px(0.0)),
            // CSS Backgrounds 3 §3.3/§3.2/§3.1: border initial は各 side で
            // style=none、color=`currentcolor` keyword
            // (`BorderColor::CurrentColor`、`CssColor::BLACK` placeholder から
            // enum variant へ格上げ、CSS Backgrounds 3 §3.1 initial 契約
            // fidelity — 上の margin 行の CSS Box 3 §3.1 とは別 spec の同番号
            // なので注意)。
            //
            // width は specified では `medium` (3px) だが **computed 層では 0px** —
            // §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width> の
            // "Computed value: … zero if the border style is `none` or `hidden`"
            // による (gate 実装は
            // `crate::resolve::resolve_border`)。specified 側の initial は
            // `crate::specified::SpecifiedValues::initial` が持つ。
            border: Sides::all(ComputedBorder {
                width: ComputedLength::ZERO,
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            }),
            // CSS Sizing 3 §3.1.1: width initial は `auto`。
            width: ComputedLengthPercentageOrAuto::Auto,
            // CSS Sizing 3 §3.1.1: height initial は `auto`。
            height: ComputedLengthPercentageOrAuto::Auto,
            // CSS Sizing 3 §3.3: box-sizing initial は `content-box`。
            box_sizing: BoxSizing::ContentBox,
            // CSS Overflow 3 §3.1: overflow-x/overflow-y initial は `visible`。
            // 両 axis が `visible` なので cross-axis
            // coupling (`resolve_overflow`) は initial state では no-op。
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
            // initial `normal` は computed 層で `0` (`ComputedLength::ZERO`)。
            letter_spacing: ComputedLength::ZERO,
            word_spacing: ComputedLength::ZERO,
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
            // CSS Flexible Box Layout Module Level 1 §5.1: flex-direction
            // initial は `row`。
            flex_direction: FlexDirectionValue::Row,
            // CSS Flexible Box Layout Module Level 1 §5.2: flex-wrap initial
            // は `nowrap`。
            flex_wrap: FlexWrapValue::NoWrap,
            // CSS Flexible Box Layout Module Level 1 §7.2.1/§7.2.2:
            // flex-grow initial は `0`、flex-shrink initial は `1`。
            flex_grow: 0.0,
            flex_shrink: 1.0,
            // CSS Flexible Box Layout Module Level 1 §7.2.3: flex-basis
            // initial は `auto`。
            flex_basis: ComputedFlexBasis::Auto,
            // CSS Box Alignment Module Level 3 §5.1: justify-content /
            // align-content initial は `normal`。
            justify_content: ContentAlignmentValue::Normal,
            align_content: ContentAlignmentValue::Normal,
            // CSS Box Alignment Module Level 3 §7.2: align-items initial は
            // `normal`。
            align_items: SelfAlignmentValue::Normal,
            // CSS Box Alignment Module Level 3 §6.2: align-self initial は
            // `auto`。
            align_self: AlignSelfValue::Auto,
            // CSS Box Alignment Module Level 3 §8.1: row-gap / column-gap
            // initial は `normal`。
            row_gap: ComputedLengthPercentageOrNormal::Normal,
            column_gap: ComputedLengthPercentageOrNormal::Normal,
            // CSS Text Decoration Module Level 3 §4: text-shadow initial
            // は `none` — shared empty Arc slot
            // (`empty_computed_text_shadow_list` doc 参照)。
            text_shadow: empty_computed_text_shadow_list(),
        }
    }

    /// 親 node の computed values から child node の「declaration が 1 つも無い
    /// 場合の computed values」を生成する。
    ///
    /// - **inherited** property は親からコピー
    /// - **non-inherited** property は `initial()` と同じ値を保持
    ///
    /// 各 property の inherited / non-inherited 分類は [`Self`] 定義の field
    /// doc comment を canonical source として参照する
    /// (現状 inherited: color / font-family / font-size / font-weight / text_align / direction / line_height / font_style / text_transform / visibility / text_indent / word_break / overflow_wrap / letter_spacing / word_spacing / white_space / text_shadow、
    /// non-inherited: background-color / display / counter-* / content /
    /// string-set / running_templates / padding / margin / border / width / height / box_sizing / overflow / text_decoration_line / text_decoration_style / text_decoration_color / vertical_align / z_index / break_before / break_after / break_inside)。
    ///
    /// # 実装 (delegation)
    ///
    /// cascade pipeline は本 method を使わない — winner の適用が staging 層
    /// ([`SpecifiedValues`]) に移ったため、`resolve_inheritance` は
    /// [`SpecifiedValues::inherit_from`] → [`SpecifiedValues::finalize`] を通る。
    /// 本 method はそこへ delegate する thin wrapper であり、**分類の実装は
    /// [`SpecifiedValues::inherit_from`] の 1 箇所だけに存在する** (新 property
    /// 追加時に 2 箇所を更新する必要はない)。
    ///
    /// delegation が恒等である根拠 — [`SpecifiedValues::inherit_from`] の出力に
    /// **font-relative な値は 1 つも含まれない**:
    ///
    /// - inherited な length 系 (`font_size` / `line_height`) は
    ///   [`lift_font_size`](crate::resolve::lift_font_size) /
    ///   [`lift_line_height`](crate::resolve::lift_line_height) が `Px` /
    ///   keyword / number にしか写さず、いずれも絶対化の**不動点**である。
    /// - non-inherited は全て initial value (`Px(0)` / `Auto` / `medium`+`none`)。
    ///
    /// 帰結として `finalize` は `em` / `rem` / `%` の arm を一度も踏まないので、
    /// **`ResolveContext` の中身は結果に影響しない** (`rem` の参照値が現れない)。
    /// 下で `parent.font_size` を渡しているのは形式上の要請にすぎず、
    /// `ResolveContext::initial()` でも同じ値になる。pin:
    /// `inherit_from_is_independent_of_resolve_context`。
    ///
    /// `border` だけは「specified の initial (`medium` = 3px) が computed 層で
    /// style gating により 0px に潰れる」変換を経るが、これは
    /// [`Self::initial`] の `border` と同じ値であり non-inherited の要求どおり。
    ///
    /// `finalize` は `parent` (`&Self` 全体) から
    /// `text_align` / `direction` も読んで `text-align: match-parent` を解決
    /// するが、こちらも恒等である —
    /// [`SpecifiedValues::inherit_from`] が `text_align` を親からそのまま
    /// コピーする
    /// ([`SpecifiedValues::text_align`](crate::specified::SpecifiedValues::text_align)
    /// の doc 参照) ので、渡される
    /// `self.text_align` は常に `parent.text_align` と等しく、`MatchParent`
    /// では**あり得ない** (computed 値が `MatchParent` を取らない invariant、
    /// [`crate::property::resolve_text_align_match_parent`] の debug_assert が
    /// pin する)。よって解決関数は常に "as specified" の pass-through 分岐を
    /// 通り、`child.text_align == parent.text_align` になる。
    ///
    /// [`SpecifiedValues`]: crate::specified::SpecifiedValues
    /// [`SpecifiedValues::inherit_from`]: crate::specified::SpecifiedValues::inherit_from
    /// [`SpecifiedValues::finalize`]: crate::specified::SpecifiedValues::finalize
    pub fn inherit_from(parent: &Self) -> Self {
        crate::specified::SpecifiedValues::inherit_from(parent).finalize(
            parent,
            &crate::resolve::ResolveContext::new(parent.font_size),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::TextShadowColor;

    #[test]
    fn initial_values_match_spec() {
        let cv = ComputedValues::initial();
        assert_eq!(cv.color, CssColor::BLACK);
        // CSS Backgrounds 3 §2.2: background-color initial は `transparent`
        // (= rgba(0, 0, 0, 0))
        assert_eq!(cv.background_color, CssColor::TRANSPARENT);
        // literal を保持する (`INITIAL_FONT_SIZE_PX` pin
        // と同じ理由 — `initial_font_family()` 参照に書き換えると自己参照になり
        // 同 helper の誤編集を検出できなくなる)。`*cv.font_family` で
        // `Arc<Vec<Atom>>` を `Vec<Atom>` に deref してから比較する。
        assert_eq!(*cv.font_family, vec![Atom::from("serif")]);
        // CSS Fonts 4 §2.5: font-size initial は `medium` = 本実装では 16px
        // (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>)。
        // **この 16.0 は意図的な literal** — `INITIAL_FONT_SIZE_PX` 参照に
        // 書き換えると同 const の誤編集を検出できなくなる (同 const の doc も参照)。
        assert_eq!(cv.font_size, ComputedLength(16.0));
        assert_eq!(cv.font_weight, 400.0);
        // CSS Inline 3 §5.1: line-height initial は `normal`
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
        assert_eq!(cv.display, DisplayValue::Inline);
        // CSS Lists 3 §4: counter-* の spec initial は `none`、本 impl では
        // empty list 表現 (<https://www.w3.org/TR/css-lists-3/#auto-numbering>)。
        assert!(cv.counter_reset.is_empty());
        assert!(cv.counter_increment.is_empty());
        assert!(cv.counter_set.is_empty());
        // CSS Content 3 §1 (content: Initial: normal) + CSS GCPM 3 §1.1.1
        // (string-set: Initial: none) — どちらも空 list 表現
        assert!(cv.content.is_empty());
        assert!(cv.string_set.is_empty());
        // CSS GCPM 3 §1.2.1: position initial は `static` →
        // running() seed 無し。
        assert!(cv.running_templates.is_empty());
        // CSS Text 3 §6.1: text-align initial は `start`。
        assert_eq!(cv.text_align, TextAlign::Start);
        // CSS Writing Modes 4 §2.1: direction initial は `ltr`。
        assert_eq!(cv.direction, Direction::Ltr);
        // CSS Fonts 4 §2.4: font-style initial は `normal`。
        assert_eq!(cv.font_style, FontStyle::Normal);
        // CSS Text Module Level 3 §2.1: text-transform initial は `none`。
        assert_eq!(cv.text_transform, TextTransform::None);
        // CSS Display 3 §4: visibility initial は `visible`。
        assert_eq!(cv.visibility, Visibility::Visible);
        // CSS Text 3 §8.1: text-indent initial は `0`。
        assert_eq!(cv.text_indent, ComputedLengthPercentage::Px(0.0));
        // CSS Text 3 §5.1: word-break initial は `normal`。
        assert_eq!(cv.word_break, WordBreak::Normal);
        // CSS Text 3 §5.4: overflow-wrap initial は `normal`。
        assert_eq!(cv.overflow_wrap, OverflowWrap::Normal);
        // CSS Box 3 §4.1: padding initial = 0 (all 4 sides)。
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
        // CSS Box 3 §3.1: margin initial は 0 on each side。
        assert_eq!(
            cv.margin,
            Sides::all(ComputedLengthPercentageOrAuto::Px(0.0))
        );
        // CSS Backgrounds 3 §3 (currentcolor へ格上げ済み): border initial は
        // 各 side {style: none, color: `currentcolor` (BorderColor::CurrentColor)}。
        // hazard case 2 (author `color:red` + border-color 省略 → cascade static
        // side が initial 直行) の enum coverage — used-value resolution は
        // paint scope で `color` property に対して確定。
        //
        // width は **0px** — specified の initial は `medium` (3px) だが CSS
        // Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>
        // の "Computed value: … zero if the border style is `none` or `hidden`"
        // により computed 層で潰れる。specified 側の
        // initial は `crate::specified` の
        // `initial_border_width_is_gated_to_zero_at_computed_layer` が pin する。
        assert_eq!(
            cv.border,
            Sides::all(ComputedBorder {
                width: ComputedLength(0.0),
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            })
        );
        // CSS Sizing 3 §3.1.1: width initial は `auto`。
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
        // CSS Sizing 3 §3.1.1: height initial は `auto`。
        assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Auto);
        // CSS Sizing 3 §3.3: box-sizing initial は `content-box`。
        assert_eq!(cv.box_sizing, BoxSizing::ContentBox);
        // CSS2 §9.9.1: z-index initial は `auto`。
        assert_eq!(cv.z_index, ZIndexValue::Auto);
        // CSS Fragmentation Module Level 3 §3.1 / §3.2: break-before /
        // break-after / break-inside initial は共に `auto`。
        assert_eq!(cv.break_before, BreakBetween::Auto);
        assert_eq!(cv.break_after, BreakBetween::Auto);
        assert_eq!(cv.break_inside, BreakInside::Auto);
        // CSS2 §9.5.1 / §9.5.2: float / clear の initial は共に `none`。
        assert_eq!(cv.float, FloatValue::None);
        assert_eq!(cv.clear, ClearValue::None);
        // CSS Text 3 §3: white-space initial は `normal`。
        assert_eq!(cv.white_space, WhiteSpace::Normal);
    }

    #[test]
    fn computed_values_is_send_and_clone() {
        fn assert_send<T: Send>() {}
        fn assert_clone<T: Clone>() {}
        assert_send::<ComputedValues>();
        assert_clone::<ComputedValues>();
    }

    // ── display initial ─────

    #[test]
    fn initial_display_is_inline() {
        // CSS Display 3 §2: display initial は `inline`
        // (anchor は `ComputedValues::display` field doc 側)。
        assert_eq!(ComputedValues::initial().display, DisplayValue::Inline);
    }

    // ── inherit_from (delegation の pin) ──

    /// 全 field が initial から離れた親 fixture。
    ///
    /// inherited / non-inherited のどちらの分岐が壊れても検出できるよう、
    /// **全 field を non-initial 値**にしてある (non-inherited が親から漏れれば
    /// initial との比較で落ち、inherited が initial に落ちれば親との比較で落ちる)。
    fn non_initial_parent() -> ComputedValues {
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
            line_height: ComputedLineHeight::Length(ComputedLength(30.0)),
            display: DisplayValue::Block,
            counter_reset: Arc::new(vec![(SmolStr::new("chapter"), 3)]),
            counter_increment: Arc::new(vec![(SmolStr::new("section"), 2)]),
            counter_set: Arc::new(vec![(SmolStr::new("page"), 5)]),
            content: Arc::new(vec![ContentComponent::Literal(SmolStr::new("x"))]),
            string_set: Arc::new(vec![(SmolStr::new("s"), Vec::new())]),
            running_templates: vec![RunningTemplate {
                name: SmolStr::new("hdr"),
            }],
            text_align: TextAlign::Center,
            // CSS Writing Modes 4 §2.1: `Rtl` — initial (`Ltr`) と異なる値
            // (non_initial_parent の趣旨どおり全 field を非 initial に)。
            direction: Direction::Rtl,
            // CSS Text 3 §8.1: initial (`0`) と異なる値
            // (non_initial_parent の趣旨どおり全 field を非 initial に)。
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
            // CSS Overflow 3 §3.1: `Hidden`/`Scroll` — non-initial (`visible`)
            // pair, and one that is also stable under `resolve_overflow`
            // (neither axis is `visible`/`clip`, so the cross-axis coupling
            // is a no-op here) so this fixture stays a plain "non-initial
            // parent", not an accidental probe of the coupling itself.
            overflow: OverflowXY {
                x: OverflowValue::Hidden,
                y: OverflowValue::Scroll,
            },
            // 3 field とも initial と異なる値
            // (non_initial_parent の趣旨どおり全 field を非 initial に)。
            text_decoration_line: TextDecorationLine::UNDERLINE,
            text_decoration_style: TextDecorationStyle::Wavy,
            text_decoration_color: TextDecorationColor::Resolved(CssColor::BLACK),
            // `Sub` — initial (`Baseline`) と異なる値 (non_initial_parent の
            // 趣旨どおり全 field を非 initial に)。
            vertical_align: VerticalAlign::Sub,
            // CSS Fonts 4 §2.4: `Italic` — initial (`Normal`) と異なる値
            // (non_initial_parent の趣旨どおり全 field を非 initial に)。
            font_style: FontStyle::Italic,
            // CSS Text Module Level 3 §2.1: `Uppercase` — initial (`None`)
            // と異なる値 (non_initial_parent の趣旨どおり全 field を非
            // initial に)。
            text_transform: TextTransform::Uppercase,
            // CSS Display 3 §4: `Hidden` — initial (`Visible`) と異なる値
            // (non_initial_parent の趣旨どおり全 field を非 initial に)。
            visibility: Visibility::Hidden,
            // CSS2 §9.9.1: `Integer(3)` — initial (`Auto`) と異なる値
            // (non_initial_parent の趣旨どおり全 field を非 initial に)。
            z_index: ZIndexValue::Integer(3),
            // CSS Text 3 §5.1: `KeepAll` — initial (`Normal`) と異なる値
            // (non_initial_parent の趣旨どおり全 field を非 initial に)。
            word_break: WordBreak::KeepAll,
            // CSS Text 3 §5.4: `Anywhere` — initial (`Normal`) と異なる値
            // (non_initial_parent の趣旨どおり全 field を非 initial に)。
            overflow_wrap: OverflowWrap::Anywhere,
            // CSS Text 3 §7.2 / §7.1: initial (`0`、`normal` の computed
            // value) と異なる値 (non_initial_parent の趣旨どおり全 field を
            // 非 initial に)。
            letter_spacing: ComputedLength(2.0),
            word_spacing: ComputedLength(4.0),
            // CSS Fragmentation Module Level 3 §3.1 / §3.2: initial
            // (`Auto`) と異なる値 (non_initial_parent の趣旨どおり全 field
            // を非 initial に)。
            break_before: BreakBetween::Page,
            break_after: BreakBetween::AvoidPage,
            break_inside: BreakInside::AvoidPage,
            // CSS2 §9.5.1 / §9.5.2: `Left`/`Both` — initial (`None`/`None`)
            // と異なる値 (non_initial_parent の趣旨どおり全 field を非
            // initial に)。
            float: FloatValue::Left,
            clear: ClearValue::Both,
            // CSS Text 3 §3: `Pre` — initial (`Normal`) と異なる値
            // (non_initial_parent の趣旨どおり全 field を非 initial に)。
            white_space: WhiteSpace::Pre,
            // CSS Flexible Box Layout Module Level 1 §5.1/§5.2: initial
            // (`Row`/`NoWrap`) と異なる値 (non_initial_parent の趣旨どおり
            // 全 field を非 initial に)。
            flex_direction: FlexDirectionValue::Column,
            flex_wrap: FlexWrapValue::Wrap,
            // CSS Flexible Box Layout Module Level 1 §7.2.1/§7.2.2: initial
            // (`0`/`1`) と異なる値。
            flex_grow: 2.0,
            flex_shrink: 3.0,
            // CSS Flexible Box Layout Module Level 1 §7.2.3: initial
            // (`auto`) と異なる値。
            flex_basis: ComputedFlexBasis::Px(50.0),
            // CSS Box Alignment Module Level 3 §5.1/§5.1/§7.2: initial
            // (`normal`) と異なる値。
            justify_content: ContentAlignmentValue::SpaceBetween,
            align_content: ContentAlignmentValue::Center,
            align_items: SelfAlignmentValue::FlexEnd,
            // CSS Box Alignment Module Level 3 §6.2: initial (`auto`) と
            // 異なる値。
            align_self: AlignSelfValue::Value(SelfAlignmentValue::Center),
            // CSS Box Alignment Module Level 3 §8.1: initial (`normal`) と
            // 異なる値。
            row_gap: ComputedLengthPercentageOrNormal::Px(6.0),
            column_gap: ComputedLengthPercentageOrNormal::Percent(10.0),
            // CSS Text Decoration Module Level 3 §4: initial (`none` =
            // 空 list) と異なる値 (non_initial_parent の趣旨どおり全 field を
            // 非 initial に)。
            text_shadow: Arc::new(vec![ComputedTextShadow {
                offset_x: ComputedLength(1.0),
                offset_y: ComputedLength(2.0),
                blur_radius: ComputedLength(3.0),
                color: TextShadowColor::Resolved(CssColor::BLACK),
            }]),
        }
    }

    /// `inherit_from` は inherited を親からコピーし、non-inherited を initial に
    /// 戻す。**`SpecifiedValues` への delegation が壊れたらここで落ちる。**
    ///
    /// field 単位で全 53 field を検査する — delegation は `finalize` を通るので、
    /// 絶対化側の regression (例: `lift_font_size` が不動点でなくなる、
    /// `resolve_border` の gating が消える) もここに現れる。
    #[test]
    fn inherit_from_copies_inherited_and_resets_non_inherited() {
        let parent = non_initial_parent();
        let child = ComputedValues::inherit_from(&parent);
        let initial = ComputedValues::initial();

        // inherited — 親からコピー (CSS Cascade 5 §7.2: inheritance が運ぶのは
        // computed value)。
        assert_eq!(child.color, parent.color);
        assert_eq!(child.font_family, parent.font_family);
        assert_eq!(child.font_size, parent.font_size);
        assert_eq!(child.font_weight, parent.font_weight);
        assert_eq!(child.text_align, parent.text_align);
        // CSS Writing Modes 4 §2.1: direction は inherited。
        assert_eq!(child.direction, parent.direction);
        // CSS Fonts 4 §2.4: font-style は inherited。
        assert_eq!(child.font_style, parent.font_style);
        // CSS Text Module Level 3 §2.1: text-transform は inherited。
        assert_eq!(child.text_transform, parent.text_transform);
        // CSS Display 3 §4: visibility は inherited。
        assert_eq!(child.visibility, parent.visibility);
        // CSS Text 3 §8.1: text-indent は inherited。
        assert_eq!(child.text_indent, parent.text_indent);
        // CSS Text 3 §5.1: word-break は inherited。
        assert_eq!(child.word_break, parent.word_break);
        // CSS Text 3 §5.4: overflow-wrap は inherited。
        assert_eq!(child.overflow_wrap, parent.overflow_wrap);
        // CSS Text 3 §7.2 / §7.1: letter-spacing / word-spacing は共に
        // inherited。
        assert_eq!(child.letter_spacing, parent.letter_spacing);
        assert_eq!(child.word_spacing, parent.word_spacing);
        // CSS Text 3 §3: white-space は inherited。
        assert_eq!(child.white_space, parent.white_space);
        // CSS Text Decoration Module Level 3 §4: text-shadow は inherited。
        assert_eq!(child.text_shadow, parent.text_shadow);
        // `line-height` の computed `<length>` は子で **再解決されない**
        // (CSS Inline 3: percentage は宣言要素で絶対化済)。
        assert_eq!(child.line_height, parent.line_height);

        // non-inherited — initial に戻る。
        assert_eq!(child.background_color, initial.background_color);
        assert_eq!(child.display, initial.display);
        assert!(child.counter_reset.is_empty());
        assert!(child.counter_increment.is_empty());
        assert!(child.counter_set.is_empty());
        assert!(child.content.is_empty());
        assert!(child.string_set.is_empty());
        assert!(child.running_templates.is_empty());
        assert_eq!(child.padding, initial.padding);
        assert_eq!(child.margin, initial.margin);
        // specified の initial border-width は `medium` (3px) だが computed 層では
        // style gating で 0px (CSS Backgrounds 3 §3.3)。delegation が
        // `resolve_border` を通っている証拠でもある。
        assert_eq!(child.border, initial.border);
        assert_eq!(child.border.top.width, ComputedLength::ZERO);
        assert_eq!(child.width, initial.width);
        assert_eq!(child.height, initial.height);
        assert_eq!(child.box_sizing, initial.box_sizing);
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
    }

    /// `inherit_from` の結果は `ResolveContext` の中身に依存しない。
    ///
    /// `ComputedValues::inherit_from` の doc が主張する invariant の pin —
    /// `SpecifiedValues::inherit_from` の出力に font-relative な値が 1 つも
    /// 含まれないので、`finalize` は `rem` arm を踏まず context を参照しない。
    /// 将来 lift 側が `Px` 以外を返すようになったら (= 不動点性が壊れたら)
    /// ここが落ちて delegation の前提が崩れたことを知らせる。
    #[test]
    fn inherit_from_is_independent_of_resolve_context() {
        use crate::resolve::ResolveContext;
        use crate::specified::SpecifiedValues;

        let parent = non_initial_parent();
        let expected = ComputedValues::inherit_from(&parent);
        for root_font_size in [
            ComputedLength(1.0),
            ComputedLength(16.0),
            ComputedLength(999.0),
        ] {
            let via_staging = SpecifiedValues::inherit_from(&parent)
                .finalize(&parent, &ResolveContext::new(root_font_size));
            assert_eq!(
                via_staging, expected,
                "inherit_from must not depend on the rem basis ({root_font_size:?})"
            );
        }
    }

    /// 親が initial なら child も initial (inheritance chain の terminate)。
    #[test]
    fn inherit_from_initial_parent_yields_initial() {
        assert_eq!(
            ComputedValues::inherit_from(&ComputedValues::initial()),
            ComputedValues::initial(),
        );
    }
}
