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
    Border, BorderColor, BorderStyle, BoxSizing, ContentComponent, CssColor, DisplayValue, Length,
    LengthOrAuto, LineHeight, Sides, TextAlign, empty_content_list, empty_counter_entries,
    empty_string_set_entries,
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
/// doc を参照 (inherited: color / font-family / font-size / font-weight / text_align / line_height、
/// non-inherited: background-color / display / counter-* / content / string-set /
/// running_templates / padding / margin / border / width / height / box_sizing)。
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
    /// `font-family` — 優先順位順。inherited、initial: `[Atom::from("serif")]`。
    pub font_family: Vec<Atom>,
    /// `font-size`。**inherited**、initial: 16px (spec は `medium`、実 px は
    /// UA 依存)。CSS Fonts 4 §2.5 "Font size: the font-size property"
    /// <https://www.w3.org/TR/css-fonts-4/#propdef-font-size>。
    pub font_size: Length,
    /// `font-weight`。inherited、initial: 400 (normal)。
    ///
    /// **常に resolve 済みの absolute weight** (`[1, 1000]`)。specified value 側の
    /// `bolder` / `lighter` sentinel ([`crate::property::FontWeightValue`]) は
    /// `crate::cascade::apply_value` が親の computed weight と CSS Fonts 4 §2.2
    /// の table から絶対値に解決してから書き込むため、この field に relative
    /// keyword が残ることはない。これは spec とも一致する — §2.2 の property
    /// table は `Computed value: a number, see below` と規定し、§2.2.1
    /// "Relative Weights" が "Specified values of `bolder` and `lighter`
    /// indicate weights relative to the weight of the parent element. The
    /// computed weight is calculated based on the inherited `font-weight`
    /// value" と規定している
    /// (<https://www.w3.org/TR/css-fonts-4/#relative-weights>)。
    ///
    /// なお `u16` 表現のため **computed value の fractional 精度は保持されない**
    /// (spec §2.2.2 は fractional weight を valid とする)。既知 divergence、
    /// 追跡: bd raikiri-spike-e52s。
    ///
    /// 型を `u16` のまま保つことは下流契約でもある: `raikiri-dom` の layout が
    /// `cv.font_weight as f32` で読み戻す (raikiri-spike-5iy + 17s8)。
    pub font_weight: u16,
    /// `line-height`。**inherited**、initial: [`LineHeight::Normal`]。
    /// CSS Inline 3 §5.1 "Line Spacing: the line-height property"
    /// <https://www.w3.org/TR/css-inline-3/#line-height-property>。
    ///
    /// `LineHeight::Number(n)` (unitless multiplier) と `LineHeight::Length(l)`
    /// は下流 (paint) で font-size context に対して resolve される。number 変種は
    /// spec §5.1 の "child inherits the specified value" special behavior により
    /// **cascade は raw value を保持** し、child の font-size で再乗算する責務を
    /// 下流に残す (raikiri-spike-0vv.9)。
    pub line_height: LineHeight,
    /// `display`。**non-inherited**、initial: `DisplayValue::Inline`
    /// (CSS Display 3 §2 <https://www.w3.org/TR/css-display-3/#propdef-display>)。
    /// Sprint 12 scope: `block` / `inline` / `inline-block` / `none`
    /// (raikiri-spike-0vv.4、詳細は [`DisplayValue`] doc)。
    pub display: DisplayValue,
    /// `counter-reset`。**non-inherited**、initial: empty list (CSS Lists 3 §3)。
    /// counter-name + initial value pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`pick_winners` value.clone、
    /// `apply_value` move) と inheritance walk clone (`resolve_inheritance` の
    /// stack push + `out[idx] = computed.clone()`) が **shallow (Arc bump)** に
    /// なる。`* { counter-reset: c0 c1 ... cN }` × M element の O(N × M) memory
    /// blow-up を単一 heap slot 共有で塞ぐ (raikiri-spike-d9y.2 SEC HIGH、d9y.1
    /// Content/StringSet pattern の踏襲)。`Arc<Vec<T>>: Deref<Target = Vec<T>>`
    /// により downstream の `.iter()` / `.len()` / `.is_empty()` は既存 pattern
    /// そのままで通る (dom/paint consumer 波及 0)。
    pub counter_reset: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-increment`。**non-inherited**、initial: empty list (CSS Lists 3 §3)。
    /// counter-name + increment pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::counter_reset`] と同 rationale
    /// (raikiri-spike-d9y.2)。
    pub counter_increment: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-set`。**non-inherited**、initial: empty list (CSS Lists 3 §3)。
    /// counter-name + value pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::counter_reset`] と同 rationale
    /// (raikiri-spike-d9y.2)。
    pub counter_set: Arc<Vec<(SmolStr, i32)>>,
    /// `content` の resolved 中間表現。**non-inherited**、initial: empty list
    /// (spec §2.1 "content" property の `normal` / `none` を空 list として扱う
    /// — 本 crate は cascade static side、pseudo-element 生成判断は下流 layer)。
    /// M5 gcpm-directive-emit (raikiri-spike-m5.1)。
    /// 下流 (raikiri-dom) が `raikiri_traits::ContentValueItem` に mapping する
    /// (raikiri-style は raikiri-traits に依存しない leaf crate = 3ps/94e Phase B、
    /// counter-* wire-through pattern を踏襲、raikiri-spike-s85)。
    /// See <https://www.w3.org/TR/css-content-3/#content-property>.
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`pick_winners` value.clone、
    /// `apply_value` move) と inheritance walk clone (`resolve_inheritance` の
    /// stack push + `out[idx] = computed.clone()`) が **shallow (Arc bump)** に
    /// なる。`* { content: "<large>" }` × N element の O(N × M) memory blow-up
    /// を単一 heap slot 共有で塞ぐ (raikiri-spike-d9y.1 SEC HIGH)。
    /// `Arc<Vec<T>>: Deref<Target = Vec<T>>` により downstream の `.iter()` /
    /// `.len()` / `.is_empty()` は既存 pattern そのままで通る (dom/paint
    /// consumer 波及 0)。
    pub content: Arc<Vec<ContentComponent>>,
    /// `string-set` の parse 結果 — `(name, content-list)` entry の列。
    /// **non-inherited**、initial: empty list (CSS GCPM 3 §3.1)。
    /// 名前解決と runtime `string()` 参照は下流 (raikiri-dom) 責務。
    /// See <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>.
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::content`] と同 rationale
    /// (raikiri-spike-d9y.1、`* { string-set: name "<large>" }` × N element の
    /// 同種 DoS 経路を塞ぐ)。
    pub string_set: Arc<Vec<(SmolStr, Vec<ContentComponent>)>>,
    /// `position: running(<custom-ident>)` の seed。**non-inherited**、initial:
    /// empty list。CSS GCPM 3 §1.2.1
    /// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>。
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
    pub text_align: TextAlign,
    /// `padding` — 4-side box-model padding。**non-inherited**、initial:
    /// `Sides::all(Length::Px(0.0))` (CSS Box 3 §6.1 initial "0")。
    ///
    /// - Physical longhands: [`padding-top`](https://www.w3.org/TR/css-box-3/#propdef-padding-top) /
    ///   `padding-right` / `padding-bottom` / `padding-left`。
    /// - Shorthand: [`padding` (§6.2)](https://www.w3.org/TR/css-box-3/#padding-shorthand)。
    ///
    /// Value grammar: `<length-percentage [0,∞]>` — non-negative constraint は
    /// parse-time enforce ([`crate::property::PropertyValue::PaddingTop`] doc 参照)。
    /// [`Sides<Length>`] は 5 [`Length`] variant (Px / Em / Rem / Percent / Pt) の
    /// authored value を保持、resolve は下流 (raikiri-dom apply_computed_to_style
    /// bridge、font-size context / containing block % / 96px-per-in DPI) 責務。
    /// (raikiri-spike-0vv.6)
    pub padding: Sides<Length>,
    /// `margin` 4-side quad (top / right / bottom / left)。**non-inherited**、
    /// initial: `0` on each side (`Sides::all(LengthOrAuto::Length(Length::Px(0.0)))`).
    ///
    /// Author CSS の box model 中核 property。raikiri-style は cascade static side
    /// に留まり、`LengthOrAuto::Length(Em/Rem/Percent/Pt)` の resolve は下流
    /// (raikiri-dom `apply_computed_to_style` bridge、future task) 責務 —
    /// font-size context / containing block % / DPI 変換で `taffy::LengthPercentageAuto`
    /// 相当に翻訳される。
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
    pub margin: Sides<LengthOrAuto>,
    /// `border` — 4-side box-model border (width / style / color × 4 side)。
    /// **non-inherited**、initial: 各 side `Border { width: Length::Px(3.0),
    /// style: BorderStyle::None, color: BorderColor::CurrentColor }`
    /// (`medium` UA-defined recommendation × `none` initial × `currentcolor`
    /// keyword、CSS Backgrounds 3 §5 initial 群、raikiri-spike-0vv.17)。
    ///
    /// - Physical longhands: [`border-top-width`](https://www.w3.org/TR/css-backgrounds-3/#border-width) /
    ///   `border-top-style` / `border-top-color` × 4 side。
    /// - Shorthand: [`border` (§5.4)](https://www.w3.org/TR/css-backgrounds-3/#border-shorthands)。
    ///
    /// # Value semantics
    ///
    /// - `Sides<Border>: Copy` により per-node write は bit-copy (Border は
    ///   f32/enum/BorderColor payload の POD 集合、BorderColor も Copy)。
    /// - `width` は [`Length`] variant (Px / Em / Rem / Pt) を保持。resolve は
    ///   下流責務 (font-size context / DPI 変換)。`<percentage>` は spec grammar
    ///   に含まれないため Length::Percent 変種は流入しない (advisor calibration
    ///   verified、[`crate::property::PropertyValue::BorderTopWidth`] doc 参照)。
    /// - `color` は cascade static side で [`BorderColor`] enum として保持し、
    ///   spec `currentcolor` keyword vs. 明示 `<color>` の specified-value
    ///   distinction を preserve する (CSS Backgrounds 3 §5.3
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
    /// - CSS Backgrounds 3 §5 "Borders":
    ///   [`border-width`](https://www.w3.org/TR/css-backgrounds-3/#border-width) /
    ///   [`border-style`](https://www.w3.org/TR/css-backgrounds-3/#border-style) /
    ///   [`border-color`](https://www.w3.org/TR/css-backgrounds-3/#border-color) /
    ///   [`border shorthand`](https://www.w3.org/TR/css-backgrounds-3/#border-shorthands)。
    ///
    /// (raikiri-spike-0vv.12)
    pub border: Sides<Border>,
    /// `width` — preferred physical horizontal size (writing-mode neutral な
    /// physical property、vertical writing mode では block axis に対応)。
    /// **non-inherited**、initial: [`LengthOrAuto::Auto`] (CSS Sizing 3 §3.1.1
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
    /// [`LengthOrAuto`] は 0vv.5 で margin longhand 用に導入した既存 shape (共通
    /// 型として本 property からも reuse、詳細は [`LengthOrAuto`] doc 参照)。
    /// `Length` variant の resolve (Em/Rem/Percent/Pt) は下流 (raikiri-dom
    /// `apply_computed_to_style` bridge、future task) 責務 — font-size context /
    /// containing block % / DPI 変換で `taffy::Style::size.width` 相当に翻訳
    /// される。`Auto` は CSS Sizing 3 の automatic size calculation
    /// (containing block width から margin/border/padding を差し引いた値を
    /// used-value に採る) として下流 layout で解決する — margin `auto` の余白
    /// 分配とは意味が異なる。(raikiri-spike-0vv.10)
    pub width: LengthOrAuto,
    /// `height` — preferred vertical size。**non-inherited**、initial:
    /// `LengthOrAuto::Auto` (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
    /// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>、
    /// spec 明記 "Initial: auto", "Inherited: no")。
    ///
    /// Sprint 17 seed scope (raikiri-spike-0vv.11) では `auto` + 非負
    /// `<length-percentage>` の 2 分岐のみ受理 — `min-content` / `max-content`
    /// / `fit-content(<length-percentage>)` は spec-valid だが milestone subset
    /// (g04 (b)) として parser 段で silent drop
    /// (`parse_height` doc 参照)。
    ///
    /// [`LengthOrAuto`] は sibling [`Self::width`] と同 shape を reuse
    /// (37n sibling、payload 型は共通)。resolve
    /// (`LengthOrAuto::Length(Percent(...))` → containing block % 換算、
    /// `LengthOrAuto::Auto` の実 layout 高さ計算) は下流 (raikiri-dom
    /// `apply_computed_to_style` bridge、future task) 責務 — 本 crate は
    /// cascade static side に留まり raw specified value を保持する。
    ///
    /// # Primary source
    ///
    /// - CSS Sizing 3 §3.1.1 "Preferred Size Properties":
    ///   [`height`](https://www.w3.org/TR/css-sizing-3/#preferred-size-properties)
    ///   — "Value: auto | `<length-percentage [0,∞]>` | min-content |
    ///   max-content | fit-content(`<length-percentage [0,∞]>`)",
    ///   "Initial: auto", "Applies to: all elements except non-replaced
    ///   inlines", "Inherited: no", "Percentages: relative to containing block".
    pub height: LengthOrAuto,
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
            font_family: vec![Atom::from("serif")],
            font_size: Length::Px(INITIAL_FONT_SIZE_PX),
            font_weight: 400,
            // CSS Inline 3 §5.1: line-height initial は `normal` (font metrics
            // ascent+descent 相当を paint 側で resolve、raikiri-spike-0vv.9)。
            line_height: LineHeight::Normal,
            display: DisplayValue::Inline,
            // CSS Lists 3 §3: counter-* initial is empty list (raikiri-spike-s85)
            // d9y.2: shared empty Arc slot — per-node allocation 回避
            // (advisor calibration、property.rs `empty_counter_entries` doc 参照)。
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            // CSS Content 3 §2.1: content initial (normal) は下流にとって「no
            // generated content」= empty list として扱う (raikiri-spike-m5.1)。
            // d9y.1: shared empty Arc slot — per-node allocation 回避
            // (advisor calibration、property.rs `empty_content_list` doc 参照)。
            content: empty_content_list(),
            // CSS GCPM 3 §3.1: string-set initial は empty list (raikiri-spike-m5.3)。
            // d9y.1: same shared-empty-Arc pattern。
            string_set: empty_string_set_entries(),
            // CSS GCPM 3 §1.2.1: position: running() seed initial は empty
            // (position の initial は `static`、running(name) 無し)。
            running_templates: Vec::new(),
            // CSS Text 3 §6.1: text-align initial is `start` (raikiri-spike-0vv.8)
            text_align: TextAlign::Start,
            // CSS Box 3 §6.1: padding initial = 0 (all 4 sides、raikiri-spike-0vv.6)。
            padding: Sides::all(Length::Px(0.0)),
            // CSS Box 3 §3.1: margin-* physical の initial は `0` (`Sides::all(0)`
            // で全 4 side に spread)。raikiri-spike-0vv.5。
            margin: Sides::all(LengthOrAuto::Length(Length::Px(0.0))),
            // CSS Backgrounds 3 §5.1/§5.2/§5.3: border initial は各 side で
            // width=medium (3px)、style=none、color=`currentcolor` keyword
            // ([`BorderColor::CurrentColor`]、raikiri-spike-0vv.12 seed +
            // raikiri-spike-0vv.17 で `CssColor::BLACK` placeholder から
            // enum variant へ格上げ、spec §5.3 initial 契約 fidelity)。
            border: Sides::all(Border {
                width: Length::Px(3.0),
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            }),
            // CSS Sizing 3 §3.1.1: width initial は `auto` (raikiri-spike-0vv.10)。
            width: LengthOrAuto::Auto,
            // CSS Sizing 3 §3.1.1: height initial は `auto` (raikiri-spike-0vv.11)。
            height: LengthOrAuto::Auto,
            // CSS Sizing 3 §3.3: box-sizing initial は `content-box`
            // (raikiri-spike-0vv.13)。
            box_sizing: BoxSizing::ContentBox,
        }
    }

    /// 親 node の computed values から child node の 「inheritance walk 開始値」
    /// を生成する。
    ///
    /// - **inherited** property は親からコピー
    /// - **non-inherited** property は `initial()` と同じ値を保持
    ///
    /// 各 property の inherited / non-inherited 分類は [`Self`] 定義の field
    /// doc comment を canonical source として参照する
    /// (現状 inherited: color / font-family / font-size / font-weight / text_align / line_height、
    /// non-inherited: background-color / display / counter-* / content /
    /// string-set / running_templates / padding / margin / border / width / height / box_sizing)。
    ///
    /// 新 property を追加する際は分類に応じてこの struct 直下の該当行を追加する
    /// (inherited なら parent からのコピー、non-inherited なら初期値を直接指定)。
    /// initial 値との drift を避けるため、対応する `initial()` の値も同時に更新
    /// すること。
    /// (spec §M1.4a、raikiri-spike-m1.22 (display) / raikiri-spike-0vv.7
    /// (background-color) / raikiri-spike-0vv.8 (text-align) /
    /// raikiri-spike-0vv.9 (line-height) / raikiri-spike-0vv.6 (padding) /
    /// raikiri-spike-0vv.13 (box-sizing))
    pub fn inherit_from(parent: &Self) -> Self {
        // 直接 struct literal で初期化する — Self::initial() 経由だと
        // font_family の Vec を 1 度 allocate → drop してから parent から
        // clone し直すことになり無駄 (roborev job 217 medium 対応)。
        Self {
            // inherited (親からコピー)
            color: parent.color,
            font_family: parent.font_family.clone(),
            font_size: parent.font_size,
            font_weight: parent.font_weight,
            // inherited (CSS Text 3 §6.1、raikiri-spike-0vv.8)。TextAlign は Copy。
            text_align: parent.text_align,
            // inherited (CSS Inline 3 §5.1 "Inheritance: Yes"、raikiri-spike-0vv.9)。
            // LineHeight は `Copy` (Length と同 shape) なので clone 不要。
            line_height: parent.line_height,
            // non-inherited (CSS Backgrounds 3 §2.2、initial: `transparent`、
            // raikiri-spike-0vv.7)
            background_color: CssColor::TRANSPARENT,
            // non-inherited (initial 値、CSS §9.2.4 initial value of display)
            display: DisplayValue::Inline,
            // non-inherited (CSS Lists 3 §3、raikiri-spike-s85)。
            // d9y.2: shared empty Arc slot (`empty_counter_entries`)、per-node
            // allocation 回避。inherit_from は child stack entry のたびに走る
            // ため、`Vec::new()` を直に書くと 1-doc あたり 3 × N 個の Vec
            // struct が生まれる (advisor calibration、d9y.1 content/string_set
            // pattern と同 rationale)。
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            // non-inherited (CSS Content 3 §2.1、raikiri-spike-m5.1)。
            // d9y.1: shared empty Arc slot (`empty_content_list`)、per-node
            // allocation 回避。inherit_from は child stack entry のたびに走る
            // ため、Arc::new(Vec::new()) を直に書くと 1-doc あたり O(N) 個の
            // small heap alloc regression になる (advisor calibration)。
            content: empty_content_list(),
            // non-inherited (CSS GCPM 3 §3.1、raikiri-spike-m5.3)。
            // d9y.1: same shared-empty-Arc pattern。
            string_set: empty_string_set_entries(),
            // non-inherited (CSS GCPM 3 §1.2.1、raikiri-spike-m5.4)
            running_templates: Vec::new(),
            // non-inherited (CSS Box 3 §6.1、raikiri-spike-0vv.6)。
            // `Sides<Length>: Copy` により per-node write は bit-copy。
            padding: Sides::all(Length::Px(0.0)),
            // non-inherited (CSS Box 3 §3.1 "Inherited: no")。initial 値と drift
            // しないよう `Self::initial()` と同 shape で 0 spread。raikiri-spike-0vv.5。
            margin: Sides::all(LengthOrAuto::Length(Length::Px(0.0))),
            // non-inherited (CSS Backgrounds 3 §5、raikiri-spike-0vv.12)。initial
            // 値と drift しないよう `Self::initial()` と同 shape で明示。`Sides<Border>:
            // Copy` により per-node write は bit-copy。raikiri-spike-0vv.17: color
            // は `BorderColor::CurrentColor` (spec §5.3 initial)。
            border: Sides::all(Border {
                width: Length::Px(3.0),
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            }),
            // non-inherited (CSS Sizing 3 §3.1.1 "Inherited: no"、initial: `auto`)。
            // 親が具体 width を持っていても child は Auto に戻る。raikiri-spike-0vv.10。
            width: LengthOrAuto::Auto,
            // non-inherited (CSS Sizing 3 §3.1.1 "Inherited: no"、
            // raikiri-spike-0vv.11)。`Self::initial()` と同 shape で `Auto`。
            // `LengthOrAuto: Copy` により per-node write は bit-copy。
            height: LengthOrAuto::Auto,
            // non-inherited (CSS Sizing 3 §3.3 "Inherited: no"、raikiri-spike-0vv.13)。
            // BoxSizing は Copy、initial 値を直接指定 (親からコピーしない — Verification #5
            // "parent border-box + child unset = child ContentBox" の pin)。
            box_sizing: BoxSizing::ContentBox,
        }
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
        assert_eq!(cv.font_family, vec![Atom::from("serif")]);
        // CSS Fonts 4 §2.5: font-size initial は `medium` = 本実装では 16px
        // (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>)。
        // **この 16.0 は意図的な literal** — `INITIAL_FONT_SIZE_PX` 参照に
        // 書き換えると同 const の誤編集を検出できなくなる (bd raikiri-spike-jaww、
        // 同 const の doc も参照)。
        assert_eq!(cv.font_size, Length::Px(16.0));
        assert_eq!(cv.font_weight, 400);
        // CSS Inline 3 §5.1: line-height initial は `normal` (raikiri-spike-0vv.9)
        assert_eq!(cv.line_height, LineHeight::Normal);
        assert_eq!(cv.display, DisplayValue::Inline);
        // CSS Lists 3 §3: counter-* initial は empty list (raikiri-spike-s85)
        assert!(cv.counter_reset.is_empty());
        assert!(cv.counter_increment.is_empty());
        assert!(cv.counter_set.is_empty());
        // CSS Content 3 §2.1 + CSS GCPM 3 §3.1 (raikiri-spike-m5.1 / m5.3)
        assert!(cv.content.is_empty());
        assert!(cv.string_set.is_empty());
        // CSS GCPM 3 §1.2.1 (raikiri-spike-m5.4): position initial は `static` →
        // running() seed 無し。
        assert!(cv.running_templates.is_empty());
        // CSS Text 3 §6.1 (raikiri-spike-0vv.8): text-align initial は `start`。
        assert_eq!(cv.text_align, TextAlign::Start);
        // CSS Box 3 §6.1 (raikiri-spike-0vv.6): padding initial = 0 (all 4 sides)。
        assert_eq!(cv.padding, Sides::all(Length::Px(0.0)));
        // CSS Box 3 §3.1 (raikiri-spike-0vv.5): margin initial は 0 on each side。
        assert_eq!(cv.margin, Sides::all(LengthOrAuto::Length(Length::Px(0.0))));
        // CSS Backgrounds 3 §5 (raikiri-spike-0vv.12 seed、raikiri-spike-0vv.17
        // で currentcolor へ格上げ): border initial は各 side {width: medium
        // (3px), style: none, color: `currentcolor` (BorderColor::CurrentColor)}。
        // hazard case 2 (author `color:red` + border-color 省略 → cascade static
        // side が initial 直行) の enum coverage — used-value resolution は
        // paint scope (bd raikiri-spike-q7qf) で `color` property に対して確定。
        assert_eq!(
            cv.border,
            Sides::all(Border {
                width: Length::Px(3.0),
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            })
        );
        // CSS Sizing 3 §3.1.1 (raikiri-spike-0vv.10): width initial は `auto`。
        assert_eq!(cv.width, LengthOrAuto::Auto);
        // CSS Sizing 3 §3.1.1 (raikiri-spike-0vv.11): height initial は `auto`。
        assert_eq!(cv.height, LengthOrAuto::Auto);
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

    // ── display + inherit_from (M1.4a、raikiri-spike-m1.22) ─────

    #[test]
    fn initial_display_is_inline() {
        // CSS §9.2.4: initial value of display is inline
        assert_eq!(ComputedValues::initial().display, DisplayValue::Inline);
    }

    #[test]
    fn inherit_from_copies_inherited_fields() {
        let parent = ComputedValues {
            color: CssColor {
                r: 200,
                g: 100,
                b: 50,
                a: 255,
            },
            // 0vv.7: non-inherited、literal fixture では明示 (child 側でも同じ initial に
            // 戻ることを確認する downstream test は inherit_from_leaves_background_color_at_initial 参照)。
            background_color: CssColor {
                r: 10,
                g: 20,
                b: 30,
                a: 255,
            },
            font_family: vec![Atom::from("sans-serif")],
            font_size: Length::Px(24.0),
            font_weight: 700,
            line_height: LineHeight::Number(1.5),
            display: DisplayValue::Block,
            // d9y.2: counter-* は Arc<Vec<..>>、fixture literal は Arc::new(vec![..]) で包む。
            counter_reset: Arc::new(vec![(SmolStr::new("chapter"), 3)]),
            counter_increment: Arc::new(vec![(SmolStr::new("section"), 2)]),
            counter_set: Arc::new(vec![(SmolStr::new("page"), 5)]),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: Vec::new(),
            // raikiri-spike-0vv.8: text-align は inherited、fixture では non-initial 値
            // (Center) を親に持たせて child が Start (initial) ではなく Center を
            // 引き継ぐことを assert する。
            text_align: TextAlign::Center,
            padding: Sides::all(Length::Px(0.0)),
            // 0vv.5: parent に explicit margin を持たせ、child が initial に落ちる
            // ことを他 non-inherited fixture (下の inherit_from_leaves_* 系) で pin。
            margin: Sides::all(LengthOrAuto::Length(Length::Px(12.0))),
            // 0vv.12: parent に non-initial border を持たせ、child が initial (medium
            // none currentcolor) に落ちることを他 non-inherited fixture
            // (inherit_from_leaves_border_at_initial) で pin。raikiri-spike-0vv.17:
            // color は `BorderColor::Resolved` variant で明示 color を保持。
            border: Sides::all(Border {
                width: Length::Px(5.0),
                style: BorderStyle::Solid,
                color: BorderColor::Resolved(CssColor {
                    r: 128,
                    g: 128,
                    b: 128,
                    a: 255,
                }),
            }),
            // 0vv.10: 同様 parent に explicit width を持たせ、child が initial
            // (Auto) に落ちる pin は inherit_from_leaves_width_at_initial 参照。
            width: LengthOrAuto::Length(Length::Px(200.0)),
            // 0vv.11: 同じく non-inherited fixture で parent に explicit value を
            // 持たせ、child が initial (Auto) に落ちることを pin
            // (inherit_from_leaves_height_at_initial 参照)。
            height: LengthOrAuto::Length(Length::Px(200.0)),
            // 0vv.13: parent に non-initial (BorderBox) を持たせ、child が initial
            // (ContentBox) に落ちることは `inherit_from_leaves_box_sizing_at_initial`
            // で pin する (Verification #5、CSS Sizing 3 §3.3 "Inherited: no")。
            box_sizing: BoxSizing::BorderBox,
        };
        let child = ComputedValues::inherit_from(&parent);
        // inherited: 親からコピー
        assert_eq!(child.color, parent.color);
        assert_eq!(child.font_family, parent.font_family);
        assert_eq!(child.font_size, parent.font_size);
        assert_eq!(child.font_weight, parent.font_weight);
        // CSS Text 3 §6.1: text-align は inherited (raikiri-spike-0vv.8)。
        assert_eq!(child.text_align, TextAlign::Center);
        // CSS Inline 3 §5.1: line-height は inherited (raikiri-spike-0vv.9)。
        // parent `Number(1.5)` は raw specified value のまま child へ渡る
        // (§5.1 special behavior、resolve は下流で child の font-size × 1.5)。
        assert_eq!(child.line_height, LineHeight::Number(1.5));
    }

    #[test]
    fn inherit_from_copies_line_height_length_variant() {
        // §5.1 のもう一方の branch: `<length-percentage>` の inherit も raw payload
        // をそのまま child に伝える (Percent は "element's own font-size" 相当を
        // 下流 paint が resolve、cascade は raw を保持)。
        let parent = ComputedValues {
            line_height: LineHeight::Length(Length::Px(24.0)),
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(child.line_height, LineHeight::Length(Length::Px(24.0)));
    }

    #[test]
    fn inherit_from_leaves_counter_properties_at_initial() {
        // CSS Lists 3 §3: counter-reset / counter-increment / counter-set は
        // non-inherited → 親が値を持っていても child は empty (initial) となる
        // (raikiri-spike-s85)
        // d9y.2: 親 fixture の counter-* は Arc<Vec<..>> になったため Arc::new でラップ。
        let parent = ComputedValues {
            counter_reset: Arc::new(vec![(SmolStr::new("chapter"), 3)]),
            counter_increment: Arc::new(vec![(SmolStr::new("section"), 2)]),
            counter_set: Arc::new(vec![(SmolStr::new("page"), 5)]),
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert!(child.counter_reset.is_empty());
        assert!(child.counter_increment.is_empty());
        assert!(child.counter_set.is_empty());
    }

    #[test]
    fn inherit_from_leaves_background_color_at_initial() {
        // CSS Backgrounds 3 §2.2: background-color は non-inherited (spec 明記
        // "Inheritance: no")。親が red でも child は initial (transparent) となる。
        // 37n sibling pattern (display / counter-* / content / string-set /
        // position の non-inheritance test 群を踏襲、raikiri-spike-0vv.7)。
        let parent = ComputedValues {
            background_color: CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(child.background_color, CssColor::TRANSPARENT);
    }

    #[test]
    fn inherit_from_leaves_display_at_initial() {
        // display は non-inherited → 親が Block でも child は Inline (initial)
        let parent = ComputedValues {
            display: DisplayValue::Block,
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(child.display, DisplayValue::Inline);
    }

    #[test]
    fn inherit_from_leaves_padding_at_initial() {
        // CSS Box 3 §6.1: padding は non-inherited。親が任意 padding を
        // 持っていても child は initial (`Sides::all(Length::Px(0.0))`)。
        // 37n sibling: display / counter-* / string_set / content non-inheritance
        // test を踏襲 (raikiri-spike-0vv.6)。
        let parent = ComputedValues {
            padding: Sides {
                top: Length::Px(10.0),
                right: Length::Percent(5.0),
                bottom: Length::Em(1.0),
                left: Length::Pt(12.0),
            },
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(child.padding, Sides::all(Length::Px(0.0)));
    }

    #[test]
    fn inherit_from_leaves_margin_at_initial() {
        // CSS Box 3 §3.1 "Inherited: no" — 親が margin を持っていても child は
        // initial (0 on each side) に戻る (raikiri-spike-0vv.5)。37n sibling:
        // display / counter-* / content / string_set / running_templates と同 shape。
        let parent = ComputedValues {
            margin: Sides {
                top: LengthOrAuto::Length(Length::Px(10.0)),
                right: LengthOrAuto::Auto,
                bottom: LengthOrAuto::Length(Length::Percent(50.0)),
                left: LengthOrAuto::Length(Length::Em(2.0)),
            },
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(
            child.margin,
            Sides::all(LengthOrAuto::Length(Length::Px(0.0)))
        );
    }

    #[test]
    fn inherit_from_leaves_border_at_initial() {
        // CSS Backgrounds 3 §5 "Inherited: no" — 親が border を持っていても
        // child は initial (各 side {medium, none, currentcolor}) に戻る
        // (raikiri-spike-0vv.12 seed、raikiri-spike-0vv.17 で `BorderColor`
        // enum へ格上げ)。37n sibling: display / counter-* / margin /
        // padding / running_templates と同 shape。hazard case 2 の
        // inheritance-path coverage: parent が author 明示 red border を
        // 持っていても child の cascade static side は `CurrentColor` に戻る
        // (paint scope は child 自身の `color` property で resolve する)。
        let parent = ComputedValues {
            border: Sides {
                top: Border {
                    width: Length::Px(10.0),
                    style: BorderStyle::Solid,
                    color: BorderColor::Resolved(CssColor {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    }),
                },
                right: Border {
                    width: Length::Em(2.0),
                    style: BorderStyle::Dashed,
                    color: BorderColor::Resolved(CssColor::BLACK),
                },
                bottom: Border {
                    width: Length::Px(1.0),
                    style: BorderStyle::Dotted,
                    color: BorderColor::Resolved(CssColor::TRANSPARENT),
                },
                left: Border {
                    width: Length::Pt(12.0),
                    style: BorderStyle::Double,
                    // Parent が明示 currentcolor を書いた state も 1 side で
                    // fixture 化し、child が initial (CurrentColor) に戻ることを
                    // pin (variant coverage — Resolved vs CurrentColor 両方が
                    // inherit_from を通過)。
                    color: BorderColor::CurrentColor,
                },
            },
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(
            child.border,
            Sides::all(Border {
                width: Length::Px(3.0),
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            })
        );
    }

    #[test]
    fn inherit_from_leaves_width_at_initial() {
        // CSS Sizing 3 §3.1.1 "Inherited: no" — 親が width: 100px を持っていても
        // child は initial (Auto) に戻る (raikiri-spike-0vv.10)。37n sibling:
        // display / counter-* / content / string_set / running_templates /
        // padding / margin と同 shape の non-inheritance pin。
        let parent = ComputedValues {
            width: LengthOrAuto::Length(Length::Px(100.0)),
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(child.width, LengthOrAuto::Auto);
    }

    #[test]
    fn inherit_from_leaves_height_at_initial() {
        // CSS Sizing 3 §3.1.1 "Inherited: no" — 親が height を持っていても child
        // は initial (`LengthOrAuto::Auto`) に戻る (raikiri-spike-0vv.11)。37n
        // sibling: display / counter-* / content / string_set / running_templates /
        // padding / margin と同 shape。
        //
        // Verification 6 (task doc) の中核 assertion。
        let parent = ComputedValues {
            height: LengthOrAuto::Length(Length::Px(100.0)),
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(child.height, LengthOrAuto::Auto);
    }

    #[test]
    fn inherit_from_leaves_box_sizing_at_initial() {
        // CSS Sizing 3 §3.3 "Inherited: no" — 親が box-sizing: border-box を
        // 持っていても child は initial (`BoxSizing::ContentBox`) に戻る
        // (raikiri-spike-0vv.13 Verification #5)。37n sibling: display /
        // counter-* / content / string_set / padding / margin と同 shape。
        let parent = ComputedValues {
            box_sizing: BoxSizing::BorderBox,
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(child.box_sizing, BoxSizing::ContentBox);
    }

    #[test]
    fn inherit_from_leaves_running_templates_at_initial() {
        // CSS GCPM 3 §1.2.1: position property は non-inherited (CSS Positioned
        // Layout 由来)。親が running(hdr) を持っていても child は initial (empty)。
        // 37n sibling pattern (string_set / content / counter-* non-inheritance
        // test を踏襲、raikiri-spike-m5.4)。
        let parent = ComputedValues {
            running_templates: vec![RunningTemplate {
                name: SmolStr::new("hdr"),
            }],
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert!(child.running_templates.is_empty());
    }
}
