//! Per-node computed CSS values.
//!
//! Cascade + inheritance walk が populate。M1.6 で ComputedValues → taffy::Style
//! + paint 用色情報の抽出 layer が入る予定。
//!
//! 現サポート property の一覧と inherited / non-inherited 分類は
//! [`ComputedValues`] 定義の field doc comment を参照。

use std::sync::Arc;

use smol_str::SmolStr;

use crate::Atom;
use crate::property::{
    BorderColor, BorderStyle, BoxSizing, ContentComponent, CssColor, Direction, DisplayValue,
    Sides, TextAlign, empty_content_list, empty_counter_entries, empty_string_set_entries,
    initial_font_family,
};
use crate::resolve::{
    ComputedBorder, ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto,
    ComputedLineHeight,
};

/// CSS spec 上の `font-size` initial value (`medium`) に対応する px 値。
///
/// CSS Fonts 4 §2.5 "Font size: the font-size property"
/// (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>) は "Initial: medium"
/// と規定し、`medium` の実 px は UA 依存。本実装は browser default の 16px を
/// 採る。
///
/// **非 test code で 16px を書く単一 source** (bd raikiri-spike-jaww) —
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
/// doc を参照 (inherited: color / font-family / font-size / font-weight / text_align / direction / line_height、
/// non-inherited: background-color / display / counter-* / content / string-set /
/// running_templates / padding / margin / border / width / height / box_sizing)。
///
/// # 層 (bd decision raikiri-spike-082k)
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
    /// (raikiri-spike-0vv.7)
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
    /// `Vec` malloc になっていた (raikiri-spike-no7b、d9y.1/d9y.2 pattern の踏襲、
    /// origin: raikiri-spike-zpui §8.2 perf lens、out-of-diff pre-existing)。
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
    /// 型は `f32` (旧 `u16`、bd raikiri-spike-e52s で格上げ) — §2.2.2 "Missing
    /// weights" <https://www.w3.org/TR/css-fonts-4/#missing-weights> "Fractional
    /// weights are valid" どおり computed value の fractional 精度を保持する。
    /// `u16` だった当時は整数化のため round-half-away-from-zero を要し、その丸めが
    /// §2.2.1 relative-weight table の行選択を変える 2 次被害を伴う既知
    /// divergence だった (旧 tracking: bd raikiri-spike-e52s、本 field の
    /// 型変更で解消)。
    ///
    /// `raikiri-dom` の layout はこの field を直接 `parley::FontWeight::new(f32)`
    /// に渡す (キャスト不要、raikiri-spike-5iy + 17s8 + e52s) —
    /// **渡す直前** に `sanitize_font_weight` (`crates/raikiri-dom/src/
    /// layout.rs`、bd raikiri-spike-sxd7) が sink 境界 guard を掛ける。
    ///
    /// # `[1, 1000]` / finite は caller が維持する contract (本 field 自体に guard なし)
    ///
    /// 本 struct の field は全て `pub` であり、cascade を経由せず直接
    /// `ComputedValues { font_weight: ..., .. }` を構築することを妨げない
    /// (例: [`crate::page::cascade_page`] の継承元 root 引数)。`u16` だった頃は
    /// 非有限値がそもそも型で構成不可能だったが、`f32` 化 (bd raikiri-spike-e52s)
    /// でこの保証は「型」から「呼び出し元の値検証」に変わった — cascade を
    /// 経由する通常経路は `parse_font_weight` (private、
    /// [`crate::property::FontWeightValue`] の doc 参照) の `[1, 1000]` range
    /// guard により常に finite だが、直接構築はその guard を経ない。
    /// `NaN` / `±Inf` が渡った場合の [`crate::cascade::resolve_relative_weight`]
    /// (`bolder`/`lighter` 解決) の挙動は同関数の doc で characterize 済み。
    ///
    /// **本 field / `resolve_relative_weight` のどちらにも guard は追加しない**
    /// (bd raikiri-spike-kfl7 precedent: guard は sink 境界に置く、resolve/
    /// computed 層の public surface は sanitize しない — carve-out 2)。
    /// この field を直接読む他の consumer (raikiri-dom 以外、例:
    /// umbrella 経由の外部 consumer) は自分の sink 境界で同様の guard を
    /// 持つ責務を負う (kfl7 carve-out 2 と同じ理屈、bd raikiri-spike-3ea3
    /// が fulgur consumer 境界の guard 方針を追跡)。
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
    /// (paint) に残す (raikiri-spike-0vv.9)。逆に
    /// [`ComputedLineHeight::Length`] は宣言要素で確定した px であり、
    /// child は**再解決せずそのまま継承する**。
    pub line_height: ComputedLineHeight,
    /// `display`。**non-inherited**、initial: `DisplayValue::Inline`
    /// (CSS Display 3 §2 <https://www.w3.org/TR/css-display-3/#propdef-display>)。
    /// Sprint 12 scope: `block` / `inline` / `inline-block` / `none`
    /// (raikiri-spike-0vv.4、詳細は [`DisplayValue`] doc)。
    pub display: DisplayValue,
    /// `counter-reset`。**non-inherited**。spec initial は `none`
    /// (CSS Lists 3 §4.1 <https://www.w3.org/TR/css-lists-3/#counter-reset>)、
    /// 本 impl はそれを空 list で表現する。
    /// counter-name + initial value pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`apply_winners` の drain での
    /// `value.clone()`、`apply_value` move) と inheritance walk clone
    /// (`resolve_inheritance` の stack push + `out[idx] = computed.clone()`) が
    /// **shallow (Arc bump)** になる。
    /// `* { counter-reset: c0 c1 ... cN }` × M element の O(N × M) memory
    /// blow-up を単一 heap slot 共有で塞ぐ (raikiri-spike-d9y.2 SEC HIGH、d9y.1
    /// Content/StringSet pattern の踏襲)。`Arc<Vec<T>>: Deref<Target = Vec<T>>`
    /// により downstream の `.iter()` / `.len()` / `.is_empty()` は既存 pattern
    /// そのままで通る (dom/paint consumer 波及 0)。
    pub counter_reset: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-increment`。**non-inherited**。spec initial は `none`
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#increment-set>)、
    /// 本 impl はそれを空 list で表現する。
    /// counter-name + increment pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::counter_reset`] と同 rationale
    /// (raikiri-spike-d9y.2)。
    pub counter_increment: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-set`。**non-inherited**。spec initial は `none`
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#increment-set>)、
    /// 本 impl はそれを空 list で表現する。
    /// counter-name + value pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::counter_reset`] と同 rationale
    /// (raikiri-spike-d9y.2)。
    pub counter_set: Arc<Vec<(SmolStr, i32)>>,
    /// `content` の resolved 中間表現。**non-inherited**。spec initial は
    /// `normal` (CSS Content 3 §1 propdef-content "Initial: normal") で、本 impl
    /// は `normal` / `none` をどちらも空 list で表現する (本 crate は cascade
    /// static side に留まり、pseudo-element 生成判断は下流 layer)。
    /// M5 gcpm-directive-emit (raikiri-spike-m5.1)。
    /// 下流 (raikiri-dom) が `raikiri_traits::ContentValueItem` に mapping する
    /// (raikiri-style は raikiri-traits に依存しない leaf crate = 3ps/94e Phase B、
    /// counter-* wire-through pattern を踏襲、raikiri-spike-s85)。
    /// See <https://www.w3.org/TR/css-content-3/#content-property>.
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`apply_winners` の drain での
    /// `value.clone()`、`apply_value` move) と inheritance walk clone
    /// (`resolve_inheritance` の stack push + `out[idx] = computed.clone()`) が
    /// **shallow (Arc bump)** になる。
    /// `* { content: "<large>" }` × N element の O(N × M) memory blow-up
    /// を単一 heap slot 共有で塞ぐ (raikiri-spike-d9y.1 SEC HIGH)。
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
    /// (raikiri-spike-d9y.1、`* { string-set: name "<large>" }` × N element の
    /// 同種 DoS 経路を塞ぐ)。
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
    /// `Vec` shape を採るのは m5.1 `content` / m5.3 `string_set` と同じ
    /// SmolStr wire-through pattern の踏襲 (原則 1 前例主義)。下流 (raikiri-dom)
    /// が per-document `Vec<RunningTemplate>` を組み立てる際に per-node seed を
    /// concatenate する。design doc §7.3 の 2-tier キャッシュ static side に相当。
    /// (raikiri-spike-m5.4)
    pub running_templates: Vec<RunningTemplate>,
    /// `text-align`。**inherited**、initial: [`TextAlign::Start`]
    /// (CSS Text 3 §6.1 "Text Alignment: the text-align shorthand"
    /// <https://www.w3.org/TR/css-text-3/#text-align-property>)。
    ///
    /// spec 上 shorthand (text-align-all + text-align-last の 2 longhand を set)
    /// だが、Sprint 12 seed では **shorthand as single field** convention (margin
    /// `Sides<T>` / content-normal-none-as-empty-list precedent) を踏襲して単一
    /// field に保持 (**g04 (b) milestone subset**、longhand 分離 §6.2 / §6.3 は
    /// 後続 task で defer)。詳細は [`TextAlign`] doc-comment。
    ///
    /// 37n sibling: [`color`](Self::color) / [`font_family`](Self::font_family) /
    /// [`font_size`](Self::font_size) / [`font_weight`](Self::font_weight) と同じ
    /// **inherited** 系 — `inherit_from` の inherited block に配置し親から by-value
    /// copy (`TextAlign` は `Copy`)。
    /// (raikiri-spike-0vv.8)
    ///
    /// **`MatchParent` が本 field の値として観測されることは無い** —
    /// cascade winner が `match-parent` でも、computed 層に届く前に
    /// [`crate::property::resolve_text_align_match_parent`] が親の
    /// [`Self::text_align`] + [`Self::direction`] を使って `Left` / `Right` /
    /// (親値のコピー) に解決する (raikiri-spike-l3wg、origin:
    /// raikiri-spike-ygl0 §8.2 spec lens F1)。解決は 2 箇所 — element 経路
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
    /// 37n sibling: [`text_align`](Self::text_align) と同じ **inherited** 系 —
    /// `inherit_from` の inherited block に配置し親から by-value copy
    /// (`Direction` は `Copy`)。
    ///
    /// 追加理由 (raikiri-spike-l3wg): [`text_align`](Self::text_align) の
    /// `match-parent` 解決 (CSS Text 3 §6.1) が親の computed `direction` を
    /// 要求する。property 自体は CSS Paged Media 3 Appendix A
    /// page-property-list <https://www.w3.org/TR/css-page-3/#page-property-list>
    /// にも独立に載っており、`@page` context でも意味を持つ。
    pub direction: Direction,
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
    /// (raikiri-spike-0vv.6)
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
    ///
    /// raikiri-spike-0vv.5。
    pub margin: Sides<ComputedLengthPercentageOrAuto>,
    /// `border` — 4-side box-model border (width / style / color × 4 side)。
    /// **non-inherited**、initial: 各 side が `width` =
    /// [`ComputedLength::ZERO`] / `style` = [`BorderStyle::None`] / `color` =
    /// [`BorderColor::CurrentColor`] (`width` / `style` は crate 内部 field、
    /// consumer からは [`ComputedBorder::width`] / [`ComputedBorder::style`]
    /// accessor 経由で読む、bd raikiri-spike-9jmt)。
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
    ///   含まれないため computed 層でも length のみで足りる (advisor calibration
    ///   verified、[`crate::property::PropertyValue::BorderTopWidth`] doc 参照)。
    /// - `color` は cascade static side で [`BorderColor`] enum として保持し、
    ///   spec `currentcolor` keyword vs. 明示 `<color>` の specified-value
    ///   distinction を preserve する (CSS Backgrounds 3 §3.1
    ///   <https://www.w3.org/TR/css-backgrounds-3/#border-color> initial:
    ///   currentcolor)。used-value resolution (currentcolor → 同 node の
    ///   computed `color` property lookup、CSS Color 3 §4.4) は paint scope
    ///   責務 (bd raikiri-spike-q7qf)。
    ///
    /// # Non-goals (raikiri-spike-0vv.12 initial、raikiri-spike-0vv.17 部分解消)
    ///
    /// - `border-image-*` sub-property (source/slice/width/outset/repeat) は
    ///   Epic 未着手、shorthand `border:` も border-image を reset しない
    ///   (spec deviation 明示、future 統合 task で対応)。
    /// - `border-{top,right,bottom,left}` 4-side single-side shorthand (例:
    ///   `border-top: 1px solid red`) は本 task では未対応、future 追加。
    /// - `currentcolor` の cascade-side enum 保持は 0vv.17 で解消。
    ///   used-value resolution (paint scope) は bd raikiri-spike-q7qf に defer。
    ///
    /// # Primary sources
    ///
    /// - CSS Backgrounds 3 §3 "Borders":
    ///   [`border-width`](https://www.w3.org/TR/css-backgrounds-3/#border-width) /
    ///   [`border-style`](https://www.w3.org/TR/css-backgrounds-3/#border-style) /
    ///   [`border-color`](https://www.w3.org/TR/css-backgrounds-3/#border-color) /
    ///   [`border shorthand`](https://www.w3.org/TR/css-backgrounds-3/#border-shorthands)。
    ///
    /// (raikiri-spike-0vv.12)
    pub border: Sides<ComputedBorder>,
    /// `width` — preferred physical horizontal size (writing-mode neutral な
    /// physical property、vertical writing mode では block axis に対応)。
    /// **non-inherited**、initial: [`ComputedLengthPercentageOrAuto::Auto`] (CSS Sizing 3 §3.1.1
    /// "Preferred Size Properties"
    /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
    ///
    /// spec value grammar は `auto | <length-percentage [0,∞]> | min-content |
    /// max-content | fit-content(<length-percentage>)` だが、Sprint 17 では
    /// `auto` + non-negative `<length-percentage>` のみ受理する (min-content /
    /// max-content / fit-content() は g04 (b) milestone subset、Epic 未着手)。
    /// 負値は spec grammar `[0,∞]` violation として parse-time drop
    /// ([`crate::property::PropertyValue::Width`] doc + `parse_width` 参照)。
    ///
    /// spec の "Computed value: as specified, with `<length-percentage>` values
    /// computed" のとおり、length は px へ絶対化され percentage は残る
    /// ([`ComputedLengthPercentageOrAuto`]、margin と同型)。`Auto` は CSS
    /// Sizing 3 の automatic size calculation (containing block width から
    /// margin/border/padding を差し引いた値を used-value に採る) として下流
    /// layout で解決する — margin `auto` の余白分配とは意味が異なる。
    /// (raikiri-spike-0vv.10)
    pub width: ComputedLengthPercentageOrAuto,
    /// `height` — preferred vertical size。**non-inherited**、initial:
    /// `ComputedLengthPercentageOrAuto::Auto` (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
    /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>、
    /// spec 明記 "Initial: auto", "Inherited: no")。
    ///
    /// Sprint 17 seed scope (raikiri-spike-0vv.11) では `auto` + 非負
    /// `<length-percentage>` の 2 分岐のみ受理 — `min-content` / `max-content`
    /// / `fit-content(<length-percentage>)` は spec-valid だが milestone subset
    /// (g04 (b)) として parser 段で silent drop
    /// (`parse_height` doc 参照)。
    ///
    /// [`ComputedLengthPercentageOrAuto`] は sibling [`Self::width`] と同 shape を
    /// reuse (37n sibling、payload 型は共通)。percentage の containing block 換算と
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
    /// 37n sibling: [`display`](Self::display) / [`background_color`](Self::background_color)
    /// と同じ **non-inherited** 系 — `inherit_from` の non-inherited block に
    /// 配置し initial 値を直接指定 (親からコピーしない)。
    ///
    /// # Downstream handoff (future scope、style-scope confined)
    ///
    /// 本 field は cascade static side seed のみ保持し、
    /// `apply_computed_to_style` bridge (dom scope、`taffy::Style::box_sizing`
    /// への翻訳) は future cross-scope task に defer
    /// (bd raikiri-spike-0vv.13 Non-goals)。
    ///
    /// (raikiri-spike-0vv.13)
    pub box_sizing: BoxSizing,
}

impl ComputedValues {
    /// CSS spec に沿った initial value。cascade で何も matching しなかった root
    /// node と、inheritance chain の terminate に使う。
    pub fn initial() -> Self {
        Self {
            color: CssColor::BLACK,
            // CSS Backgrounds 3 §2.2: background-color initial は `transparent`
            // (raikiri-spike-0vv.7)。
            background_color: CssColor::TRANSPARENT,
            // d9y.1/d9y.2 pattern踏襲 (raikiri-spike-no7b): shared Arc slot —
            // per-node allocation 回避 (`initial_font_family` doc 参照)。
            font_family: initial_font_family(),
            font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
            font_weight: 400.0,
            // CSS Inline 3 §5.1: line-height initial は `normal` (font metrics
            // ascent+descent 相当を paint 側で resolve、raikiri-spike-0vv.9)。
            line_height: ComputedLineHeight::Normal,
            display: DisplayValue::Inline,
            // CSS Lists 3 §4: counter-* の spec initial は `none`、本 impl は
            // 空 list で表現する (raikiri-spike-s85、anchor は field doc 参照)。
            // d9y.2: shared empty Arc slot — per-node allocation 回避
            // (advisor calibration、property.rs `empty_counter_entries` doc 参照)。
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            // CSS Content 3 §1: content の spec initial は `normal`。本 impl は下流に
            // とって「no generated content」= 空 list で表現する (raikiri-spike-m5.1)。
            // d9y.1: shared empty Arc slot — per-node allocation 回避
            // (advisor calibration、property.rs `empty_content_list` doc 参照)。
            content: empty_content_list(),
            // CSS GCPM 3 §1.1.1: string-set の spec initial は `none`。本 impl は
            // それを空 list で表現する (raikiri-spike-m5.3)。
            // d9y.1: same shared-empty-Arc pattern。
            string_set: empty_string_set_entries(),
            // CSS GCPM 3 §1.2.1: position: running() seed initial は empty
            // (position の initial は `static`、running(name) 無し)。
            running_templates: Vec::new(),
            // CSS Text 3 §6.1: text-align initial is `start` (raikiri-spike-0vv.8)
            text_align: TextAlign::Start,
            // CSS Writing Modes 4 §2.1: direction initial is `ltr` (raikiri-spike-l3wg)。
            direction: Direction::Ltr,
            // CSS Box 3 §4.1: padding initial = 0 (all 4 sides、raikiri-spike-0vv.6)。
            padding: Sides::all(ComputedLengthPercentage::Px(0.0)),
            // CSS Box 3 §3.1: margin-* physical の initial は `0` (`Sides::all(0)`
            // で全 4 side に spread)。raikiri-spike-0vv.5。
            margin: Sides::all(ComputedLengthPercentageOrAuto::Px(0.0)),
            // CSS Backgrounds 3 §3.3/§3.2/§3.1: border initial は各 side で
            // style=none、color=`currentcolor` keyword
            // ([`BorderColor::CurrentColor`]、raikiri-spike-0vv.12 seed +
            // raikiri-spike-0vv.17 で `CssColor::BLACK` placeholder から
            // enum variant へ格上げ、CSS Backgrounds 3 §3.1 initial 契約
            // fidelity — 上の margin 行の CSS Box 3 §3.1 とは別 spec の同番号
            // なので注意)。
            //
            // width は specified では `medium` (3px) だが **computed 層では 0px** —
            // §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width> の
            // "Computed value: … zero if the border style is `none` or `hidden`"
            // による (bd raikiri-spike-zls8、gate 実装は
            // `crate::resolve::resolve_border`)。specified 側の initial は
            // `crate::specified::SpecifiedValues::initial` が持つ。
            border: Sides::all(ComputedBorder {
                width: ComputedLength::ZERO,
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            }),
            // CSS Sizing 3 §3.1.1: width initial は `auto` (raikiri-spike-0vv.10)。
            width: ComputedLengthPercentageOrAuto::Auto,
            // CSS Sizing 3 §3.1.1: height initial は `auto` (raikiri-spike-0vv.11)。
            height: ComputedLengthPercentageOrAuto::Auto,
            // CSS Sizing 3 §3.3: box-sizing initial は `content-box`
            // (raikiri-spike-0vv.13)。
            box_sizing: BoxSizing::ContentBox,
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
    /// (現状 inherited: color / font-family / font-size / font-weight / text_align / direction / line_height、
    /// non-inherited: background-color / display / counter-* / content /
    /// string-set / running_templates / padding / margin / border / width / height / box_sizing)。
    ///
    /// # 実装 (bd raikiri-spike-zls8 以降は delegation)
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
    /// `finalize` は `parent` (raikiri-spike-l3wg 以降 `&Self` 全体) から
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

    #[test]
    fn initial_values_match_spec() {
        let cv = ComputedValues::initial();
        assert_eq!(cv.color, CssColor::BLACK);
        // CSS Backgrounds 3 §2.2: background-color initial は `transparent`
        // (= rgba(0, 0, 0, 0)、raikiri-spike-0vv.7)
        assert_eq!(cv.background_color, CssColor::TRANSPARENT);
        // literal を保持する (bd raikiri-spike-jaww の `INITIAL_FONT_SIZE_PX` pin
        // と同じ理由 — `initial_font_family()` 参照に書き換えると自己参照になり
        // 同 helper の誤編集を検出できなくなる)。`*cv.font_family` で
        // `Arc<Vec<Atom>>` を `Vec<Atom>` に deref してから比較する。
        assert_eq!(*cv.font_family, vec![Atom::from("serif")]);
        // CSS Fonts 4 §2.5: font-size initial は `medium` = 本実装では 16px
        // (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>)。
        // **この 16.0 は意図的な literal** — `INITIAL_FONT_SIZE_PX` 参照に
        // 書き換えると同 const の誤編集を検出できなくなる (bd raikiri-spike-jaww、
        // 同 const の doc も参照)。
        assert_eq!(cv.font_size, ComputedLength(16.0));
        assert_eq!(cv.font_weight, 400.0);
        // CSS Inline 3 §5.1: line-height initial は `normal` (raikiri-spike-0vv.9)
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
        assert_eq!(cv.display, DisplayValue::Inline);
        // CSS Lists 3 §4: counter-* の spec initial は `none`、本 impl では
        // empty list 表現 (<https://www.w3.org/TR/css-lists-3/#auto-numbering>、
        // raikiri-spike-s85)。
        assert!(cv.counter_reset.is_empty());
        assert!(cv.counter_increment.is_empty());
        assert!(cv.counter_set.is_empty());
        // CSS Content 3 §1 (content: Initial: normal) + CSS GCPM 3 §1.1.1
        // (string-set: Initial: none) — どちらも空 list 表現 (m5.1 / m5.3)
        assert!(cv.content.is_empty());
        assert!(cv.string_set.is_empty());
        // CSS GCPM 3 §1.2.1 (raikiri-spike-m5.4): position initial は `static` →
        // running() seed 無し。
        assert!(cv.running_templates.is_empty());
        // CSS Text 3 §6.1 (raikiri-spike-0vv.8): text-align initial は `start`。
        assert_eq!(cv.text_align, TextAlign::Start);
        // CSS Writing Modes 4 §2.1 (raikiri-spike-l3wg): direction initial は `ltr`。
        assert_eq!(cv.direction, Direction::Ltr);
        // CSS Box 3 §4.1 (raikiri-spike-0vv.6): padding initial = 0 (all 4 sides)。
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
        // CSS Box 3 §3.1 (raikiri-spike-0vv.5): margin initial は 0 on each side。
        assert_eq!(
            cv.margin,
            Sides::all(ComputedLengthPercentageOrAuto::Px(0.0))
        );
        // CSS Backgrounds 3 §3 (raikiri-spike-0vv.12 seed、raikiri-spike-0vv.17
        // で currentcolor へ格上げ): border initial は各 side {style: none,
        // color: `currentcolor` (BorderColor::CurrentColor)}。
        // hazard case 2 (author `color:red` + border-color 省略 → cascade static
        // side が initial 直行) の enum coverage — used-value resolution は
        // paint scope (bd raikiri-spike-q7qf) で `color` property に対して確定。
        //
        // width は **0px** — specified の initial は `medium` (3px) だが CSS
        // Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>
        // の "Computed value: … zero if the border style is `none` or `hidden`"
        // により computed 層で潰れる (bd raikiri-spike-zls8)。specified 側の
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
        // CSS Sizing 3 §3.1.1 (raikiri-spike-0vv.10): width initial は `auto`。
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
        // CSS Sizing 3 §3.1.1 (raikiri-spike-0vv.11): height initial は `auto`。
        assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Auto);
        // CSS Sizing 3 §3.3 (raikiri-spike-0vv.13): box-sizing initial は `content-box`。
        assert_eq!(cv.box_sizing, BoxSizing::ContentBox);
    }

    #[test]
    fn computed_values_is_send_and_clone() {
        fn assert_send<T: Send>() {}
        fn assert_clone<T: Clone>() {}
        assert_send::<ComputedValues>();
        assert_clone::<ComputedValues>();
    }

    // ── display initial (M1.4a、raikiri-spike-m1.22) ─────

    #[test]
    fn initial_display_is_inline() {
        // CSS Display 3 §2: display initial は `inline`
        // (anchor は [`ComputedValues::display`] field doc 側)。
        assert_eq!(ComputedValues::initial().display, DisplayValue::Inline);
    }

    // ── inherit_from (delegation の pin、bd raikiri-spike-zls8 §8.2 debt D1) ──

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
        }
    }

    /// `inherit_from` は inherited を親からコピーし、non-inherited を initial に
    /// 戻す。**`SpecifiedValues` への delegation が壊れたらここで落ちる。**
    ///
    /// field 単位で全 21 field を検査する — delegation は `finalize` を通るので、
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
        // CSS Writing Modes 4 §2.1 (raikiri-spike-l3wg): direction は inherited。
        assert_eq!(child.direction, parent.direction);
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
