//! Computed value 層の value 型 + specified → computed の絶対化 (absolutization)。
//!
//! 本 module は **層の分離**を担う。本 module の絶対化関数群は
//! [`crate::property`] の [`Length`] / [`LengthOrAuto`] / [`LineHeight`] /
//! [`Border`] を **入力**に取り、`Computed*` 型群を **出力**する。すなわち
//! `Computed*` 型は「computed value 層である」ことを型で表明する。
//! [`crate::computed::ComputedValues`] は per-node の集約 struct として
//! `computed.rs` に残る (本 module は value 型と絶対化関数のみ)。
//!
//! **逆は成り立たない — [`Length`] 等は「specified 層である」ことを表明しない。**
//! 本 module の入力に現れるときは specified 層だが、型そのものが層を決めるわけ
//! ではなく、**層は値の出所で決まる**。実際 page 経路
//! ([`crate::page::cascade_page`]) は `PropertyValue` の bag を運ぶので
//! **computed 値も [`Length`] で運ばれる**。
//! canonical な説明は [`Length`] の doc の「本型は『specified 層』を意味しない
//! — 層は出所で決まる」節、page 経路が保証する内容は
//! [`crate::page::PageCascadeResult::declarations`] の doc が canonical。
//! **本節は要約に留め、規則の中身をここに書き足さないこと** — 以前ここには
//! 「書き換えるときは必ずあちらと揃えること」と書いてあったが、その手運用は
//! 実際に 2 度 drift した。現在は `page::tests` の
//! `page_declarations_carry_no_specified_layer_residue` (以前は
//! `page_declarations_carry_exactly_one_specified_layer_residue` という名前で、
//! `text-align: match-parent` が唯一の specified 層残滓だった) が保証内容を
//! 機械的に pin している。
//!
//! # なぜ絶対化が独立 phase なのか
//!
//! CSS Cascade 5 §7.2 "Inheritance"
//! (<https://www.w3.org/TR/css-cascade-5/#inheriting>) は
//! "The inherited value of a property on an element is the computed value of the
//! property on the element's parent element." と規定する。すなわち inheritance が
//! 運ぶのは **computed value** であり、`em` / `rem` は inheritance の時点で既に
//! 絶対化されていなければならない。
//!
//! かつ絶対化は **cascade winner の適用とは別 phase** でなければならない —
//! `padding: 2em` の基準となる `font-size` は、同 node の全 winner を適用し終えた
//! 後にしか確定しないため、winner を 1 つずつ適用する途中で絶対化することは
//! できない (適用順は property 間で保証されない)。
//!
//! 本 module の関数群は、この制約を守るのに必要な材料を signature に持つ —
//! いずれも cascade の winner 集合に触らない純関数で、基準となる font-size を
//! **引数で受け取る**。ただし signature が保証するのは
//! **「本 module の関数自体が winner を適用しない」「基準が呼び出し側から明示的に
//! 供給される」の 2 点だけ**である。
//!
//! **順序は型で縛られていない。** 引数はただの [`ComputedLength`] なので、winner を
//! 1 つ適用するたびに本 module の関数を呼び、親の font-size や phase 2 前の中間値を
//! 基準として渡す誤実装は**普通に書ける** (型検査は通る)。すなわち上記の制約は
//! 本 module では**規約として**守るものであり、下の doctest がその規約である。
//!
//! **cascade pipeline 側は規約に頼っていない**:
//! 絶対化の入口を [`SpecifiedValues::finalize`] /
//! [`SpecifiedValues::finalize_as_root`] の 2 つに絞り、phase 3 を
//! `parent_font_size` を受け取らない private 関数に閉じ込めてある。
//! `OwnFontSize` / `ParentFontSize` newtype による型 level の enforcement は
//! **採らなかった** — 守る距離が各 entry point の 2 行しかない一方、本 module の
//! public 関数とその doctest 全体の signature churn を伴うため。
//!
//! [`SpecifiedValues::finalize`]: crate::specified::SpecifiedValues::finalize
//! [`SpecifiedValues::finalize_as_root`]: crate::specified::SpecifiedValues::finalize_as_root
//!
//! # 想定される 4 段階 (phase 1 / 2 / 2.5 / 3) の呼び出し順序
//!
//! phase 2.5 (line-height の絶対化) は後から追加された —
//! `padding: 2lh` のような box property が `1lh` を使うには、自 node の
//! line-height が **先に**確定していなければならない (font-size が phase 2 で
//! 先に確定するのと同じ理由)。
//!
//! `parent_line_height_basis` (**親要素の**確定済み used line-height) は
//! phase 2 (`font-size` の `lh` 自己参照、
//! [`resolve_font_size`] doc 参照) にも必要になった。**これは phase 2 → 2.5 の
//! 順序を逆転させるものではない** — `parent_line_height_basis` が指すのは
//! **自 node の** phase 2.5 の結果ではなく、**親 node** の (別の再帰呼び出しで
//! 既に確定済みの) phase 2.5 の結果である。tree walk は親を子より先に処理する
//! ため、この値は自 node の phase 2 に入る**前**から手元にある。したがって
//! 呼び手は単に「`parent_line_height_basis` を求める式を、自 node の phase 2
//! 呼び出しより前に書く」だけでよい (下記例、[`SpecifiedValues::finalize`] の
//! 実装も同形)。
//!
//! ```
//! use raikiri_style::{
//!     ComputedLength, ComputedLengthPercentage, ComputedLineHeight, ResolveContext,
//!     resolve_font_size, resolve_length_percentage, resolve_line_height,
//!     used_line_height_length,
//! };
//! use raikiri_style::property::{Length, LineHeight};
//!
//! // 親の computed font-size / line-height (inheritance が運んできた computed
//! // value — 親 node は既に処理済みなので、この 2 つは自 node の処理に入る
//! // 前から確定している)。
//! let parent_font_size = ComputedLength(16.0);
//! let parent_line_height = ComputedLineHeight::Normal;
//! let ctx = ResolveContext::new(ComputedLength(16.0));
//!
//! // 親の line-height 基準 (`lh` の自己参照、`font-size` と `line-height` の
//! // 両方が使う) — 親が既に確定済みなので、自 node の phase 2 より前に求まる。
//! let parent_line_height_basis = used_line_height_length(parent_line_height, parent_font_size);
//!
//! // phase 1: cascade winner を specified 表現のまま staging する (順不同)。
//! let specified_font_size = Length::Em(1.5);
//! let specified_line_height = LineHeight::Number(1.5);
//! let specified_padding_top = Length::Lh(2.0);
//!
//! // phase 2: font-size を **親基準** で絶対化する。
//! let font_size = resolve_font_size(
//!     specified_font_size,
//!     parent_font_size,
//!     parent_line_height_basis,
//!     &ctx,
//! );
//! assert_eq!(font_size, ComputedLength(24.0));
//!
//! // phase 2.5: line-height を絶対化する。`<number>` は自 node の (今確定した)
//! // font-size 基準、`lh`/`rlh` の自己参照基準は親の line-height
//! // (`resolve_line_height` doc 参照) — ここでは `<number>` なので後者は未使用。
//! let line_height =
//!     resolve_line_height(specified_line_height, font_size, parent_line_height_basis, &ctx);
//! assert_eq!(line_height, ComputedLineHeight::Number(1.5));
//!
//! // phase 3: 残りを **自 node の確定済 font-size / line-height** 基準で絶対化する。
//! // `padding: 2lh` の基準は phase 2.5 が確定した own line-height (1.5 * 24px = 36px)。
//! let own_line_height = used_line_height_length(line_height, font_size);
//! let padding_top =
//!     resolve_length_percentage(specified_padding_top, font_size, own_line_height, &ctx);
//! assert_eq!(padding_top, ComputedLengthPercentage::Px(72.0)); // 2 * 36
//! ```
//!
//! # `#[non_exhaustive]` の方針 — 本 module の computed 型群に限る判断
//!
//! **crate-wide の規則ではない。** specified 層の [`Length`] / [`LengthOrAuto`] /
//! [`LineHeight`] / [`Border`] を含む `property.rs` の公開 enum は sum 型でも
//! `#[non_exhaustive]` を付ける (この crate 内の類似 enum に共通の convention)。
//! 本 module の
//! computed 型群だけがそこから外れる — 理由は下記の **explicit trade** であって
//! 「sum 型だから」という形の性質ではない。
//!
//! - [`ComputedLengthPercentage`] / [`ComputedLengthPercentageOrAuto`] /
//!   [`ComputedLineHeight`] / [`ComputedTabSize`] — **付けない** (下記 trade。
//!   下流に網羅 match を強制する)。
//! - [`ComputedBorder`] / [`ResolveContext`] — **付ける**。field 追加は下流の
//!   match を fail-quiet にしないので、source 互換を取る方が純粋に得。
//! - [`ComputedLength`] — **付けない**。`ComputedLength(16.0)` の位置構築を
//!   下流に許すため (`#[non_exhaustive]` はそれを禁じる)。
//!
//! 付けない判断は **spec が variant 数を閉じているからではない**。
//! CSS Values 4 §5.6.1 "Computation and Combination of Percentage and Dimension
//! Mixes" (<https://www.w3.org/TR/css-values-4/#combine-mixed>) は verbatim で
//!
//! > The computed value of a percentage-dimension mix is defined as
//! > - a computed dimension if the percentage component is zero or is defined
//! >   specifically to compute to a dimension value
//! > - a computed percentage if the dimension component is zero
//! > - a computed calc() expression otherwise
//!
//! と規定しており、computed `<length-percentage>` は px / percentage /
//! **calc()** の 3 形態を取る。したがって将来 css-variables-and-math 対応が
//! 入れば `Calc` variant は**確実に増える**。これは以下の **explicit trade**
//! である。
//!
//! - **得るもの**: 今すぐ下流で網羅 match が書けること。`raikiri-dom` の
//!   `layout.rs` にある defensive な `_ => length(0.0)` を削除でき、
//!   fail-quiet の class が型検査で閉じる。
//! - **払うもの**: 将来 `Calc` variant を追加する際、raikiri-style /
//!   raikiri-dom / raikiri を跨ぐ coordinated breaking change が 1 回発生する。
//! - **取る理由**: `calc()` は下流が**必ず対応すべき**形態なので、compile error
//!   で強制通知する方が、`#[non_exhaustive]` にして黙って 0px に落とすより安全。
//!
//! CSS Values 4 §10.11 "Computed Value"
//! (<https://www.w3.org/TR/css-values-4/#calc-computed-value>) は裏側も
//! 規定する — "Where percentages are not resolved at computed-value time, they
//! are not resolved in math functions, e.g. `calc(100% - 100% + 1px)` resolves to
//! `calc(0% + 1px)`, not to `1px`." すなわち percentage を computed 層に残す
//! property では calc() 形態が computed value として**残る**。
//!
//! なお `#[non_exhaustive]` が提供するのは **source 互換** (下流が既存 match に
//! 新 arm を書き足さずに済む) であり、再 compile の回避ではない — dependency が
//! 変われば下流の再 compile は当然発生する。
//!
//! [`ComputedLength`] はこの trade の対象外 — 詳細は同型の doc を参照。

use std::sync::{Arc, OnceLock};

use crate::computed::INITIAL_FONT_SIZE_PX;
use crate::property::{
    Border, BorderColor, BorderStyle, FlexBasisValue, Length, LengthOrAuto, LengthOrNormal,
    LineHeight, TabSize, TextShadowColor, TextShadowItem, VerticalAlign,
};

// ---------------------------------------------------------------------------
// computed value 層の value 型
// ---------------------------------------------------------------------------

/// Computed `<length>` — **px 単位の絶対長**。
///
/// `font-size` / `border-*-width` のように grammar が `<percentage>` を取らない
/// property の computed value に使う。
///
/// # Primary sources (§ title + anchor)
///
/// - CSS Values 4 §6 "Distance Units: the `<length>` type"
///   (<https://www.w3.org/TR/css-values-4/#lengths>): "The computed value of a
///   length (computed length) is the specified length resolved to an absolute
///   length, and its unit is not distinguished: it can be represented by any
///   absolute length unit (but will be serialized using its canonical unit,
///   px)." → 本型は「px で表現する」選択を取る。
/// - CSS Fonts 4 §2.5 "Font size: the font-size property"
///   (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>):
///   "Computed value: an absolute length"。
///
/// # `#[non_exhaustive]` を付けない理由 (module doc の trade の対象外)
///
/// 本型は単一 f32 payload の newtype であり、`calc()` 導入後も表現が変わらない。
/// `font-size: calc(1em + 2px)` は computed 時に完全な `<length>` へ解決される —
/// 根拠は上記 CSS Values 4 §6 (「computed length は絶対長へ resolve される」) と
/// CSS Values 4 §10.11 "Computed Value"
/// (<https://www.w3.org/TR/css-values-4/#calc-computed-value>): "The computed
/// value of a math function is its calculation tree simplified, using all the
/// information available at computed value time. (Such as the em to px ratio,
/// how to resolve percentages in some properties, etc.)"
///
/// **§5.6.1 `#combine-mixed` は本主張の根拠にならない** — 同 section が規定する
/// のは *percentage 成分と dimension 成分の混合*であり、`1em + 2px` は dimension
/// 同士の加算なので対象外。
///
/// 加えて本型は `pub f32` 1 field の tuple struct であり、`ComputedLength(16.0)`
/// という位置構築を下流に許したい (`#[non_exhaustive]` はそれを禁じる) —
/// module doc の product 型の扱いとはこの点で異なる。
///
/// ```
/// use raikiri_style::ComputedLength;
///
/// let fs = ComputedLength(16.0);
/// assert_eq!(fs.px(), 16.0);
/// assert_eq!(ComputedLength::ZERO, ComputedLength(0.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedLength(pub f32);

impl ComputedLength {
    /// `0px`。padding / margin / border-width の spec initial value に対応する。
    pub const ZERO: Self = Self(0.0);

    /// px 値を取り出す。
    pub fn px(self) -> f32 {
        self.0
    }
}

/// Computed `<length-percentage>` — px か percentage。
///
/// `padding-*` のように grammar が `<length-percentage>` を取り、percentage の
/// 参照値が **used value 層**で決まる (containing block width) property に使う。
/// percentage は computed 層に**そのまま残る**。
///
/// # Primary sources (§ title + anchor)
///
/// - CSS Values 4 §5.5.1 "Computation and Combination of `<percentage>`"
///   (<https://www.w3.org/TR/css-values-4/#combine-percentages>): "Unless
///   otherwise specified (such as in font-size, which computes its
///   `<percentage>` values to `<length>`), the computed value of a percentage is
///   the specified percentage."
/// - CSS Box 3 `padding-top` propdef
///   (<https://www.w3.org/TR/css-box-3/#propdef-padding-top>):
///   "Value: `<length-percentage [0,∞]>`" / "Computed value: a computed
///   `<length-percentage>` value"。
/// - CSS Cascade 5 §4.5 "Used Values"
///   (<https://www.w3.org/TR/css-cascade-5/#used>) — containing block width への
///   解決は used value 層 (raikiri では taffy の責務)。
///
/// [`Percent`](Self::Percent) は **authored 数値をそのまま**保持する
/// (`50%` → `Percent(50.0)`、`/ 100.0` しない) — specified 層の
/// [`Length::Percent`] と同じ convention。
///
/// `#[non_exhaustive]` を付けない判断とその trade は
/// [module doc](crate::resolve) を参照。
///
/// ```
/// use raikiri_style::{
///     ComputedLength, ComputedLengthPercentage, ResolveContext, resolve_length_percentage,
/// };
/// use raikiri_style::property::Length;
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(20.0);
///
/// // percentage は絶対化せず素通し (参照値は used value 層で決まる)。
/// let p = resolve_length_percentage(Length::Percent(50.0), font_size, None, &ctx);
/// assert_eq!(p, ComputedLengthPercentage::Percent(50.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedLengthPercentage {
    /// 絶対化済みの px 長。
    Px(f32),
    /// Percentage — authored 数値をそのまま保持 (`50%` → `Percent(50.0)`)。
    Percent(f32),
}

/// Computed `<length-percentage> | auto`。
///
/// `margin-*` / `width` / `height` の computed value に使う。
///
/// # Primary sources (§ title + anchor)
///
/// - CSS Box 3 `margin-top` propdef
///   (<https://www.w3.org/TR/css-box-3/#propdef-margin-top>):
///   "Value: `<length-percentage> | auto`" / "Computed value: the keyword auto
///   or a computed `<length-percentage>` value"。
/// - CSS Sizing 3 §3.1.1 "Preferred Size Properties: the width and height
///   properties"
///   (<https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>):
///   "Value: auto | `<length-percentage>` | min-content | max-content |
///   fit-content(`<length-percentage>`)" / "Computed value: as specified, with
///   `<length-percentage>` values computed" — 後者が本型 (length は絶対化、
///   percentage は素通し) の直接の根拠。
///   (`min-content` / `max-content` / `fit-content()` は specified 層の
///   [`LengthOrAuto`] が未対応 — 既存 gap、本 module の scope 外。)
///
/// [`Auto`](Self::Auto) の意味は property 依存 (margin は available space の
/// 分配、width / height は automatic size calculation) — specified 層の
/// [`LengthOrAuto::Auto`] と同じく variant 側は property-agnostic に保つ。
///
/// `#[non_exhaustive]` を付けない判断とその trade は
/// [module doc](crate::resolve) を参照。
///
/// ```
/// use raikiri_style::{
///     ComputedLength, ComputedLengthPercentageOrAuto, ResolveContext,
///     resolve_length_percentage_or_auto,
/// };
/// use raikiri_style::property::LengthOrAuto;
///
/// let ctx = ResolveContext::initial();
/// let auto = resolve_length_percentage_or_auto(
///     LengthOrAuto::Auto,
///     ComputedLength(16.0),
///     None,
///     &ctx,
/// );
/// assert_eq!(auto, ComputedLengthPercentageOrAuto::Auto);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedLengthPercentageOrAuto {
    /// 絶対化済みの px 長。
    Px(f32),
    /// Percentage — authored 数値をそのまま保持 (`50%` → `Percent(50.0)`)。
    Percent(f32),
    /// `auto` keyword。
    Auto,
}

/// Computed `flex-basis`。
///
/// CSS Flexible Box Layout Module Level 1 §7.2.3
/// (<https://www.w3.org/TR/css-flexbox-1/#flex-basis-property>): "Computed
/// value: specified keyword or a computed `<length-percentage>` value" —
/// `auto` / `content` は computed 層でも keyword のまま、それ以外は
/// [`ComputedLengthPercentage`] と同じ shape (`Px` / `Percent`) に絶対化する。
/// [`ComputedLengthPercentageOrAuto`] の `content`-keyword 版に相当する。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedFlexBasis {
    /// 絶対化済みの px 長。
    Px(f32),
    /// Percentage — authored 数値をそのまま保持。
    Percent(f32),
    /// `auto` keyword。
    Auto,
    /// `content` keyword ([`crate::property::FlexBasisValue`] doc の
    /// "`content` と `auto` の意味差" 節参照)。
    Content,
}

/// Computed `row-gap` / `column-gap`。
///
/// CSS Box Alignment Module Level 3 §8.1 propdef `row-gap`/`column-gap`
/// (<https://www.w3.org/TR/css-align-3/#propdef-row-gap>): "Computed value:
/// specified keyword, else a computed `<length-percentage>` value" — `normal`
/// は computed 層でも keyword のまま残る点が [`ComputedLength`] へ潰す
/// `letter-spacing`/`word-spacing` の `normal` ([`resolve_length_or_normal`]
/// doc 参照) と**異なる** (gap は "Computes to: normal" ではなく "specified
/// keyword" — spec 文言の違いをそのまま型に反映)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedLengthPercentageOrNormal {
    /// 絶対化済みの px 長。
    Px(f32),
    /// Percentage — authored 数値をそのまま保持。
    Percent(f32),
    /// `normal` keyword — spec initial value。
    Normal,
}

/// Computed `line-height`。
///
/// # Primary source (§ title + anchor)
///
/// CSS Inline 3 §5.1 "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#propdef-line-height>) の property
/// definition table は verbatim で
///
/// > Value: normal | `<number [0,∞]>` | `<length-percentage [0,∞]>`
/// > Percentages: computed relative to 1em
/// > Computed value: the specified keyword, a number, or a computed `<length>`
/// > value
///
/// と規定する。すなわち **computed 層に percentage は存在しない** —
/// `<percentage>` は「1em に対する比率」= 自要素の computed font-size
/// (CSS Values 4 §6.1.1 `em` <https://www.w3.org/TR/css-values-4/#em>
/// "Equal to the computed value of the font-size property of the element on
/// which it is used.") に対して computed 時に絶対化される。本型が
/// `Percent` variant を持たないのはこの Computed value 行の 3 形態に 1:1 で
/// 対応させた結果である。
///
/// [`Number`](Self::Number) が computed 層でも number のまま残るのは spec 上
/// load-bearing な distinction — 子は number を inherit して**自分の**
/// font-size に掛ける。
///
/// `#[non_exhaustive]` を付けない判断とその trade は
/// [module doc](crate::resolve) を参照。
///
/// ```
/// use raikiri_style::{ComputedLength, ComputedLineHeight, ResolveContext, resolve_line_height};
/// use raikiri_style::property::{Length, LineHeight};
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(20.0);
///
/// // `<percentage>` は自要素の computed font-size に対して絶対化される。
/// let lh = resolve_line_height(LineHeight::Length(Length::Percent(150.0)), font_size, None, &ctx);
/// assert_eq!(lh, ComputedLineHeight::Length(ComputedLength(30.0)));
///
/// // `<number>` は素通し (子が自分の font-size に掛ける)。
/// let n = resolve_line_height(LineHeight::Number(1.5), font_size, None, &ctx);
/// assert_eq!(n, ComputedLineHeight::Number(1.5));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedLineHeight {
    /// `normal` keyword — computed 層でも keyword のまま (font metrics に基づく
    /// 解決は paint 責務)。
    Normal,
    /// `<number>` — unitless multiplier。computed 層でも number のまま。
    Number(f32),
    /// 絶対化済みの `<length>`。
    Length(ComputedLength),
}

/// Computed `tab-size`。
///
/// # Primary source (§ title + anchor)
///
/// CSS Text Module Level 3 §4.2 "Tab Character Size: the tab-size property"
/// (<https://www.w3.org/TR/css-text-3/#tab-size-property>) propdef table:
///
/// > Value: `<number [0,∞]> | <length [0,∞]>`
/// > Initial: 8
/// > Percentages: N/A
/// > Computed value: the specified number or absolute length
///
/// [`Number`](Self::Number) が computed 層でも number のまま残るのは
/// [`ComputedLineHeight::Number`] と同じ理由 — spec 本文 "A `<number>`
/// represents the measure as a multiple of the advance width of the space
/// character ... of the nearest block container ancestor" という font
/// metric 依存の解決を、本 crate がまだ持たない downstream text layout
/// consumer に委ねる ([`crate::property::TabSize`] doc 参照)。
///
/// `#[non_exhaustive]` を付けない判断とその trade は
/// [module doc](crate::resolve) を参照。
///
/// ```
/// use raikiri_style::{ComputedLength, ComputedTabSize, ResolveContext, resolve_tab_size};
/// use raikiri_style::property::{Length, TabSize};
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(20.0);
///
/// // `<length>` は自要素の computed font-size に対して絶対化される。
/// let ts = resolve_tab_size(TabSize::Length(Length::Em(2.0)), font_size, None, &ctx);
/// assert_eq!(ts, ComputedTabSize::Length(ComputedLength(40.0)));
///
/// // `<number>` は素通し (downstream consumer が自分の font metrics に掛ける)。
/// let n = resolve_tab_size(TabSize::Number(4.0), font_size, None, &ctx);
/// assert_eq!(n, ComputedTabSize::Number(4.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedTabSize {
    /// `<number>` — computed 層でも number のまま。
    Number(f32),
    /// 絶対化済みの `<length>`。
    Length(ComputedLength),
}

/// Computed `border-*` (1 side 分の width / style / color)。
///
/// specified 層の [`Border`] と同じ shape で、`width` のみ
/// [`ComputedLength`] に置き換わる。`style` / `color` は computed 層でも
/// specified keyword を保つ ([`BorderColor::CurrentColor`] の
/// used-value 解決は paint 責務)。
///
/// # Primary source (§ title + anchor)
///
/// CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) の propdef table:
///
/// - "Value: `<line-width>`" (`<line-width> = <length [0,∞]> | thin | medium |
///   thick`) — grammar に `<percentage>` を含まないため、width は `<length>`
///   のみ ([`ComputedLength`]) で足りる。
/// - "Computed value: absolute length, snapped as a border width; **zero if the
///   border style is `none` or `hidden`**" — style gating が **computed 層**の
///   要求であることの根拠。[`resolve_border`] がこれを実装する。
///   (`snapped as a border width` = device pixel への snap は未実装、
///   本 module の scope 外。)
///
/// `#[non_exhaustive]` を付ける (module doc 参照) — future field
/// (`border-image-*` の cascade 統合など) を source 互換で追加できる。specified
/// 層の [`Border`] と同じ判断。
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext, SpecifiedValues, resolve_border};
/// use raikiri_style::property::BorderStyle;
///
/// let ctx = ResolveContext::initial();
/// // specified 層の initial border (computed 層の initial は下記のとおり 0px)。
/// let initial = SpecifiedValues::initial().border.top;
///
/// // specified の border-width `medium` = 3px — CSS Backgrounds 3 §3.3 は
/// // "The thin, medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// // respectively." と**規範的に**等価を定めている (UA 裁量ではない。UA 依存なのは
/// // font-size の `medium` で、CSS Fonts 4 §2.5.1 "Absolute Size Keyword Mapping"
/// // の別 keyword)。
/// // ただし initial の border-style は `none` なので computed value は 0px。
/// assert_eq!(
///     resolve_border(initial, ComputedLength(20.0), None, &ctx).width(),
///     ComputedLength::ZERO,
/// );
///
/// // style が visible なら specified width がそのまま絶対化される。
/// let mut specified = initial;
/// specified.style = BorderStyle::Solid;
/// let computed = resolve_border(specified, ComputedLength(20.0), None, &ctx);
/// assert_eq!(computed.width(), ComputedLength(3.0));
/// // style / color は specified keyword をそのまま運ぶ。
/// assert_eq!(computed.style(), specified.style);
/// assert_eq!(computed.color, specified.color);
/// // `computed.style()` 呼び出しと `computed.color` 直接読み出しは、下記
/// // write-path pin (`# write 経路が無いことの compile-fail pin` 節) の
/// // non-vacuous control を兼ねる。
/// ```
///
/// # write 経路が無いことの compile-fail pin
///
/// `width` / `style` はいずれも `pub(crate)` に絞ってある (各 field doc
/// 参照)。この narrowing が保たれ続けることは prose の主張のままだと将来の
/// regression (rename 時の見落とし等) で静かに崩れうる。
/// [`crate::rule::Declaration`] の `value` field で確立した技法 (同型の
/// doc comment 参照) をここに転用する。
///
/// `ComputedBorder` にはすでに `#[non_exhaustive]` が付いているため、struct
/// literal 構築や `..base` functional-update による fence は width / style
/// 単独の visibility を discriminate **できない** — 発生するエラーは常に
/// non_exhaustive 由来の `E0639` であり、両 field が将来 `pub` に戻っても
/// compile-fail し続けてしまう (`Declaration` の doc が指摘する同種の
/// vacuous pin と同じ構造。ただしあちらは「将来 non_exhaustive が付いたら」
/// という risk だったのに対し、こちらは non_exhaustive が既に付いている現在
/// の事実であり、非 struct-literal 系 fence を最初から作らない理由になる)。
///
/// そのため struct literal fence は作らず、[`resolve_border`] が返す
/// **所有権のある**値への直接 field 代入だけを使う。`resolve_border` が
/// 参照ではなく値そのものを返すため (上の主 doctest 参照)、`Declaration` の
/// doc が踏んだ confound (`declarations()` が `&[_]` を返すので
/// `.clone()` を挟まないと代入が常に `E0594` (immutable な参照への代入) で
/// vacuous-compile-fail する) は **そもそも発生しない** — 借用を経由しない
/// ので、代入の成否は各 field 自身の visibility だけで決まる:
///
/// ```compile_fail
/// use raikiri_style::{ComputedLength, ResolveContext, SpecifiedValues, resolve_border};
///
/// let ctx = ResolveContext::initial();
/// let specified = SpecifiedValues::initial().border.top;
/// let mut computed = resolve_border(specified, ComputedLength(20.0), None, &ctx);
/// computed.width = ComputedLength(999.0);
/// ```
///
/// ```compile_fail
/// use raikiri_style::property::BorderStyle;
/// use raikiri_style::{ComputedLength, ResolveContext, SpecifiedValues, resolve_border};
///
/// let ctx = ResolveContext::initial();
/// let specified = SpecifiedValues::initial().border.top;
/// let mut computed = resolve_border(specified, ComputedLength(20.0), None, &ctx);
/// computed.style = BorderStyle::Solid;
/// ```
///
/// # 上 2 fence の non-visibility 部分の non-vacuous control
///
/// 新しい control doctest はここには追加しない — 上の主 doctest (本 struct
/// doc 冒頭) がすでに同じ ingredient (`resolve_border` /
/// `SpecifiedValues::initial` / `ComputedLength` / `BorderStyle`) を使い、
/// `.width()` / `.style()` accessor 呼び出しに加えて `color` field
/// (`computed.color` / `specified.color` の比較、今 `pub`)
/// への直接読み出しまで行った上で compile が通ることを assert している。
/// ingredient が drift (rename / shape 変更) すれば、まずそちらが
/// (compile_fail ではなく通常の doctest として) 落ちるので、上 2 fence が
/// 「意図した理由」で compile-fail し続けているかどうかの drift 検知は
/// そちらに委ねる。
///
/// ただし、この control は `resolve_border` が値ではなく参照を返すよう
/// 変わった場合の drift を検知しない (`.width()` / `.color` はどちらの
/// 戻り値型でも同じく compile が通るため) — その変更が width/style の
/// 可視性緩和と同時に起きると 2 fence は `E0594` で compile-fail し続け、
/// vacuous 化に気付けない。`resolve_border` の戻り値型を変える際は本 doc
/// を書き直すこと。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedBorder {
    /// 絶対化済みの border width。
    ///
    /// `style` が [`BorderStyle::None`] / [`BorderStyle::Hidden`] のとき
    /// [`resolve_border`] は本 field を必ず `ComputedLength::ZERO` にする
    /// (上記 propdef の gating)。crate 外からの直接書き換えでこの対応関係を
    /// 崩せないよう `pub(crate)` に絞り、read-only accessor [`Self::width`]
    /// のみを公開する。
    pub(crate) width: ComputedLength,
    /// `border-*-style` — computed 層でも specified keyword。
    ///
    /// `width` と対で `pub(crate)` に絞り、read-only accessor
    /// [`Self::style`] のみを公開する。
    pub(crate) style: BorderStyle,
    /// `border-*-color` — `currentcolor` keyword を保持したまま computed 層に
    /// 残る (used-value 解決は paint 責務)。
    pub color: BorderColor,
}

impl ComputedBorder {
    /// 絶対化済みの border width への read-only accessor。
    ///
    /// [`Self::style`] が [`BorderStyle::None`] / [`BorderStyle::Hidden`] の
    /// ときは必ず `ComputedLength::ZERO` — [`resolve_border`] が gate する。
    pub fn width(&self) -> ComputedLength {
        self.width
    }

    /// `border-*-style` の computed value への read-only accessor。
    pub fn style(&self) -> BorderStyle {
        self.style
    }
}

/// `text-shadow` の 1 shadow entry の computed value。
///
/// CSS Text Decoration Module Level 3 §4
/// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property> の
/// Computed value: "a list, each item consisting of three absolute lengths
/// plus a computed color"。[`TextShadowItem`] (specified 層、[`crate::property`])
/// の length 3 本 (`offset_x`/`offset_y`/`blur_radius`) を [`ComputedLength`]
/// に絶対化したもの — `color` は [`ComputedBorder::color`] / used-value
/// resolution が paint scope 責務な点も含め同じ扱い ([`TextShadowColor`] doc
/// 参照)。
///
/// `#[non_exhaustive]` — sibling [`ComputedBorder`] と同じ判断 (future field
/// の non-breaking 追加)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedTextShadow {
    /// 絶対化済みの `offset-x`。
    pub offset_x: ComputedLength,
    /// 絶対化済みの `offset-y`。
    pub offset_y: ComputedLength,
    /// 絶対化済みの `blur-radius`。省略時 (specified 層で `Length::Px(0.0)`
    /// に eager fill 済み、[`TextShadowItem`] doc 参照) は `ComputedLength::ZERO`。
    pub blur_radius: ComputedLength,
    /// `<color>` — `currentcolor` keyword を保持したまま computed 層に残る
    /// (used-value 解決は paint 責務、[`TextShadowColor`] doc 参照)。
    pub color: TextShadowColor,
}

/// 空 `text-shadow` list (`none`) を表す computed 層の shared Arc —
/// [`crate::property::empty_text_shadow_list`] の computed-layer counterpart
/// (同じ `OnceLock` shared-slot pattern、per-node allocation regression 回避)。
/// specified 層 (`Arc<Vec<TextShadowItem>>`) と computed 層
/// (`Arc<Vec<ComputedTextShadow>>`) は phase 3 で型が変わる (length が
/// [`Length`] → [`ComputedLength`] に絶対化される) ため、別 slot が要る。
pub(crate) fn empty_computed_text_shadow_list() -> Arc<Vec<ComputedTextShadow>> {
    static EMPTY: OnceLock<Arc<Vec<ComputedTextShadow>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// [`TextShadowItem`] (specified) を絶対化して [`ComputedTextShadow`] にする
/// (**phase 3** — 自 node 基準)。
///
/// 3 本の length はいずれも `<length>` (percentage 不可、[`TextShadowItem`]
/// doc 参照) なので、percentage 対応の [`resolve_length_percentage`] ではなく
/// percentage 非対応の [`resolve_length`] へ delegate する
/// ([`resolve_length_or_normal`] が `letter-spacing`/`word-spacing` の
/// `<length>` 成分を同じ理由で [`resolve_length`] に委譲するのと同型)。
/// `color` は length を運ばないため素通し。
pub fn resolve_text_shadow_item(
    specified: TextShadowItem,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedTextShadow {
    ComputedTextShadow {
        offset_x: resolve_length(specified.offset_x, font_size, own_line_height, ctx),
        offset_y: resolve_length(specified.offset_y, font_size, own_line_height, ctx),
        blur_radius: resolve_length(specified.blur_radius, font_size, own_line_height, ctx),
        color: specified.color,
    }
}

/// 親の computed `text-shadow` 1 item を specified 表現に **lift** する
/// (inheritance seed 用)。
///
/// [`lift_length_or_normal`] と同じ lossless / 不動点性 — [`resolve_length`]
/// の `Px` arm は identity なので、lift した値を phase 3 に再度通しても
/// 二重適用にならない。`color` は length を運ばないため素通し。
pub fn lift_text_shadow_item(computed: ComputedTextShadow) -> TextShadowItem {
    TextShadowItem {
        offset_x: Length::Px(computed.offset_x.0),
        offset_y: Length::Px(computed.offset_y.0),
        blur_radius: Length::Px(computed.blur_radius.0),
        color: computed.color,
    }
}

// ---------------------------------------------------------------------------
// ResolveContext
// ---------------------------------------------------------------------------

/// 絶対化に必要な document-global の参照値。
///
/// `rem` の参照値 (root element の computed font-size) と、`rlh` の参照値
/// (root element の computed line-height を [`used_line_height_length`] で
/// 絶対長に変換した値、`normal` で解決不能なら `None`) の 2 つ
/// (後者は後から追加された)。
///
/// # Primary source (§ title + anchor)
///
/// CSS Values 4 §6.1.1 "Font-relative Lengths"
/// (<https://www.w3.org/TR/css-values-4/#rem>): `rem` — "Equal to the computed
/// value of the em unit on the root element." /
/// (<https://www.w3.org/TR/css-values-4/#rlh>): `rlh` — "Equal to the value of
/// the lh unit on the root element."
///
/// `#[non_exhaustive]` (module doc 参照) — struct 自体は future field を source
/// 互換で追加できる。下流からの struct literal 構築は [`ResolveContext::new`] /
/// [`ResolveContext::with_root_line_height`] を使う。
///
/// **ただし `new` は positional なので `#[non_exhaustive]` の source 互換は
/// constructor まで及ばない。** `root_line_height` の追加
/// はこの trade-off の実例 — `new` の signature を破壊せず、`root_line_height`
/// を明示したい呼び手のためだけに [`ResolveContext::with_root_line_height`] を
/// 第 2 constructor として追加した (`new` は `root_line_height: None` 固定の
/// 薄い wrapper のまま)。将来また field が増える場合も同じ判断 (breaking な
/// `new` signature 変更ではなく第 2 constructor / builder) を踏襲すること。
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext};
///
/// // root element の font-size が確定する前 (および root element 自身の
/// // `font-size: Nrem`) は initial value 基準。
/// assert_eq!(ResolveContext::initial().root_font_size, ComputedLength(16.0));
/// assert_eq!(
///     ResolveContext::new(ComputedLength(20.0)).root_font_size,
///     ComputedLength(20.0),
/// );
/// // `new` は `root_line_height` を明示しない既存呼び手向けの薄い wrapper —
/// // `rlh` の参照値は常に「未確定」(`None`) になる。
/// assert_eq!(ResolveContext::new(ComputedLength(20.0)).root_line_height, None);
/// assert_eq!(
///     ResolveContext::with_root_line_height(ComputedLength(20.0), Some(ComputedLength(24.0)))
///         .root_line_height,
///     Some(ComputedLength(24.0)),
/// );
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolveContext {
    /// root element の computed font-size。`rem` の参照値。
    ///
    /// 型は [`ComputedLength`] — 本 field が保持するのは**絶対化済の computed
    /// `<length>`** であり、[`resolve_font_size`] の戻り値をそのまま格納できる。
    pub root_font_size: ComputedLength,
    /// root element の `lh` 値 — `rlh` の参照値。
    ///
    /// [`used_line_height_length`] が root element の
    /// (computed line-height, computed font-size) から導く**絶対化済の
    /// px 長**、または `normal` で解決不能なら `None`。`None` は
    /// [`ComputedLineHeight::Normal`] と同じ「font metrics が style 層に無い」
    /// wall を表す (`cap`/`rcap` と同じ、[`Length::Lh`] doc 参照) — `0` や
    /// 他の数値で代用しない (cleanroom: 根拠のない比率を捏造しない)。
    ///
    /// [`Length::Lh`]: crate::property::Length::Lh
    pub root_line_height: Option<ComputedLength>,
}

impl ResolveContext {
    /// root element の computed font-size を指定して構築する。
    ///
    /// `root_line_height` は `None` (未確定) — `rlh` を要する呼び手は
    /// [`Self::with_root_line_height`] を使うこと。既存呼び手 (`rlh` を扱わない
    /// tree) の non-breaking な移行のためにこの thin wrapper を残す
    /// (struct doc の `#[non_exhaustive]` trade-off節 参照)。
    pub fn new(root_font_size: ComputedLength) -> Self {
        Self {
            root_font_size,
            root_line_height: None,
        }
    }

    /// root element の computed font-size **と** `rlh` の参照値を指定して
    /// 構築する。
    ///
    /// `root_line_height` は呼び手が [`used_line_height_length`] で
    /// あらかじめ絶対化した値 (root element の computed line-height が
    /// `normal` で解決不能なら `None`) を渡す。
    pub fn with_root_line_height(
        root_font_size: ComputedLength,
        root_line_height: Option<ComputedLength>,
    ) -> Self {
        Self {
            root_font_size,
            root_line_height,
        }
    }

    /// root element の computed font-size が未確定な段階で使う initial context。
    ///
    /// `root_font_size` は `font-size` の initial value (16px) —
    /// [`crate::computed::ComputedValues::initial`] の `font_size` と同一値。
    /// `root_line_height` も同様に未確定 (`None`) — root element の line-height
    /// も同じ「親が無い」条項に従い initial value (`normal`) 基準になるため、
    /// 本 context の下では `rlh` は常に解決不能 (下記 doc および
    /// [`SpecifiedValues::finalize_as_root`] 参照)。
    ///
    /// root element 自身の **`font-size: Nrem`** もこの値を参照する: CSS Values 4
    /// §6.1.1 "Font-relative Lengths"
    /// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) の
    /// "When used in the value of any font-* property on the element they refer
    /// to, the font-relative lengths resolve against the computed metrics of the
    /// parent element—or against the computed metrics corresponding to the
    /// initial values of the font and line-height properties, if the element has
    /// no parent." により、root element では initial value 基準になる。同 §は
    /// `lh`/`rlh` にも同条項の類似規定を及ぼすが、両者の非対称
    /// (`lh` は自己参照として扱う、`rlh` は tree-global 定数として扱う) の
    /// 判断根拠は [`resolve_line_height`] doc が canonical
    /// (前述の drift 前例により要約に留める) — 結論だけ述べると、root element の
    /// `line-height: 1lh` / `1rlh` はどちらも「initial line-height
    /// (`normal`)」基準に帰着し、常に unresolved になる。
    ///
    /// **root element の box property (`padding` 等) は対象外** — 上記条項は
    /// "any font-* property" / "the line-height property" に限定されており、
    /// `padding: 2rem` の `rem` や `padding: 1rlh` の `rlh` は素の定義どおり
    /// root element の computed font-size / line-height を参照する。すなわち
    /// root element でも phase 3 では本 context ではなく
    /// `ResolveContext::with_root_line_height(自 font-size, 自 rlh 基準)` を使う
    /// ([`SpecifiedValues::finalize_as_root`] が実装している)。
    ///
    /// [`SpecifiedValues::finalize_as_root`]: crate::specified::SpecifiedValues::finalize_as_root
    pub fn initial() -> Self {
        Self {
            root_font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
            root_line_height: None,
        }
    }
}

// ---------------------------------------------------------------------------
// 絶対化関数群 (specified → computed)
// ---------------------------------------------------------------------------

/// `pt` → px。`1pt = 1/72in`、CSS で `1in = 96px` なので `1pt = 96/72px = 4/3px`。
///
/// CSS Values 4 §6.2 "Absolute Lengths"
/// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>): "All of the
/// absolute length units are compatible, and px is their canonical unit."
///
/// 式は `v * 4.0 / 3.0` の形 (乗算を先) で書く — f32 は結合則を満たさないため
/// `v * (4.0 / 3.0)` に「簡約」すると異なる bit パターンの f32 になる。
/// **この式の形を変えてはならない。**
///
/// (以前は `raikiri-dom` の `layout.rs` bridge helper
/// 群 (padding / width-height / margin の3関数) にも同じ変換 (`Length::Pt(v)`
/// を受けて `v * 4.0 / 3.0` する arm、返り値の wrapper 型は関数ごとに
/// `LengthPercentage` / `Dimension` / `LengthPercentageAuto` と異なる) が
/// 存在し、それらと bit 単位で一致させることもこの評価順を選ぶ理由の
/// 一つだった。その後 bridge の引数を computed 層の型に切り替えた際に
/// その arm は3関数とも削除され — pt は cascade phase 3 で既に px に
/// 絶対化済みのため bridge に届かない — cross-check 対象は今は存在しない。
/// f32 非結合性という理由だけでこの式の形は独立に正しい。)
fn pt_to_px(v: f32) -> f32 {
    v * 4.0 / 3.0
}

/// `in` → px。CSS Values 4 §6.2 "Absolute Lengths"
/// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) 換算表 verbatim:
/// "1in = 2.54cm = 96px"。
fn in_to_px(v: f32) -> f32 {
    v * 96.0
}

/// `cm` → px。CSS Values 4 §6.2 換算表 verbatim: "1cm = 96px/2.54"。
fn cm_to_px(v: f32) -> f32 {
    v * 96.0 / 2.54
}

/// `mm` → px。CSS Values 4 §6.2 換算表 verbatim: "1mm = 1/10th of 1cm"。
/// spec の連鎖定義どおり [`cm_to_px`] を経由する (`px` と直接の等価式が
/// spec に無いため — spec が与えるのは `cm` / `in` 起点の比のみ)。
fn mm_to_px(v: f32) -> f32 {
    cm_to_px(v) / 10.0
}

/// `Q` (quarter-millimeter) → px。CSS Values 4 §6.2 換算表 verbatim:
/// "1Q = 1/40th of 1cm"。[`mm_to_px`] と同じ理由で [`cm_to_px`] を経由する。
fn q_to_px(v: f32) -> f32 {
    cm_to_px(v) / 40.0
}

/// `pc` (pica) → px。CSS Values 4 §6.2 換算表 verbatim: "1pc = 1/6th of 1in"。
/// [`mm_to_px`] / [`q_to_px`] と同じ理由で [`in_to_px`] を経由する。
fn pc_to_px(v: f32) -> f32 {
    in_to_px(v) / 6.0
}

/// すでに絶対化済みの [`ComputedLineHeight`] (自要素の、または root element の)
/// を、`lh` / `rlh` 単位の乗数として使える**絶対長**に変換する。
///
/// # Primary source (§ title + anchor)
///
/// CSS Values 4 §6.1.1 "Font-relative Lengths" [`lh`](https://www.w3.org/TR/css-values-4/#lh)
/// verbatim: "Equal to the computed value of the line-height property of the
/// element on which it is used, converting normal to an absolute length by
/// using only the metrics of the first available font."
///
/// - [`ComputedLineHeight::Length`] — 既に絶対長なのでそのまま返す。
/// - [`ComputedLineHeight::Number`] — `<number>` の used value は「この
///   要素自身の font-size に掛けたもの」(CSS Inline 3 §5.1
///   <https://www.w3.org/TR/css-inline-3/#propdef-line-height> の unitless
///   multiplier semantics — line box の高さ計算がまさにこの積を使う)。
/// - [`ComputedLineHeight::Normal`] — **`None`**。`normal` を絶対長化するには
///   "the metrics of the first available font" (real font ascent/descent) が
///   要るが、`raikiri-style` は style 層に font instance を持たない — `cap`/
///   `rcap` が同じ理由で spin out された wall と同じもの
///   ([`crate::property::Length::Lh`] doc 参照)。spec はここに font-size 比の
///   fallback を与えていないため、`ex`/`ch`/`ic` のような比率を捏造しては
///   ならない (cleanroom)。呼び手が消費 property ごとの fallback を選ぶ
///   ([`resolve_length_percentage`] 等の doc 参照)。
///
/// `normal` は `line-height` の **initial value** — この関数が `None` を返す
/// のは edge case ではなく、`lh`/`rlh` を使う要素の**大半**で起こる common
/// case である。
///
/// ```
/// use raikiri_style::{ComputedLength, ComputedLineHeight, used_line_height_length};
///
/// let font_size = ComputedLength(20.0);
///
/// // <length> はそのまま。
/// assert_eq!(
///     used_line_height_length(ComputedLineHeight::Length(ComputedLength(30.0)), font_size),
///     Some(ComputedLength(30.0)),
/// );
/// // <number> は自要素の font-size に掛ける。
/// assert_eq!(
///     used_line_height_length(ComputedLineHeight::Number(1.5), font_size),
///     Some(ComputedLength(30.0)),
/// );
/// // `normal` — font metrics が無いので解決不能。
/// assert_eq!(
///     used_line_height_length(ComputedLineHeight::Normal, font_size),
///     None,
/// );
/// ```
pub fn used_line_height_length(
    line_height: ComputedLineHeight,
    font_size: ComputedLength,
) -> Option<ComputedLength> {
    match line_height {
        ComputedLineHeight::Normal => None,
        ComputedLineHeight::Number(n) => Some(ComputedLength(font_size.0 * n)),
        ComputedLineHeight::Length(l) => Some(l),
    }
}

/// `lh` / `rlh` の authored multiplier に、[`used_line_height_length`] が
/// 返した基準を掛ける共通 helper。基準が `None` (`normal` で解決不能) なら
/// `None` を素通しし、各 `resolve_*` 関数が自分の consumer property に
/// 応じた fallback (`0px` / `Auto` / `normal`) を選ぶ。
fn resolve_lh_multiplier(v: f32, basis: Option<ComputedLength>) -> Option<ComputedLength> {
    basis.map(|b| ComputedLength(b.0 * v))
}

/// `font-size` の specified value を絶対化する (**phase 2** — 親基準)。
///
/// `parent_font_size` は**親要素の** computed font-size。親がない (root element)
/// 場合は `font-size` の initial value (16px、
/// [`ComputedLength`]`(16.0)`) を渡す。
///
/// `self_reference_basis` は**親要素の** used line-height ([`Length::Lh`]
/// の解決に使う — 下記 `Nlh` 行)。呼び手が
/// [`used_line_height_length`]`(parent.line_height, parent.font_size)` で
/// あらかじめ絶対長化したもの。`normal` で解決不能、または親がない (root
/// element) 場合は `None`。命名は [`resolve_line_height`] の同名引数と揃えた
/// — 両者とも「自己参照 (`lh`) を解決するための親基準」という同じ役割を持つ
/// (呼び手側のローカル変数名は `parent_line_height_basis` のままで構わない —
/// 呼び手が計算したものを指す名前と、この関数が受け取る引数の名前は別の命名
/// 領域であり、揃えるべきは後者と sibling 関数の対応する引数)。
///
/// # 単位ごとの解決
///
/// | specified | computed | 根拠 |
/// |---|---|---|
/// | `Npx` | `N` px | identity |
/// | `Npt` / `Ncm` / `Nmm` / `NQ` / `Nin` / `Npc` | 換算表どおり | CSS Values 4 §6.2 <https://www.w3.org/TR/css-values-4/#absolute-lengths> |
/// | `Nem` / `Nex` / `Nch` | `parent_font_size * N` (`ex`/`ch` は `* 0.5` 追加) | 下記 parent-metrics 条項 + [`Length::Ex`] / [`Length::Ch`] doc の fallback |
/// | `Nic` | `parent_font_size * N` | 下記 parent-metrics 条項 + [`Length::Ic`] doc の fallback |
/// | `Nrem` / `Nrex` / `Nrch` / `Nric` | `ctx.root_font_size * N` (`rex`/`rch` は `* 0.5` 追加) | CSS Values 4 §6.1.1 `rem` <https://www.w3.org/TR/css-values-4/#rem> |
/// | `N%` | `parent_font_size * N / 100` | CSS Fonts 4 `font-size` propdef "Percentages: refer to parent element's font size" <https://www.w3.org/TR/css-fonts-4/#propdef-font-size> |
/// | `Nlh` | `self_reference_basis * N`、基準が `None` なら [`INITIAL_FONT_SIZE_PX`] | 下記「`lh` / `rlh` の自己参照」節 |
/// | `Nrlh` | `ctx.root_line_height * N`、基準が `None` なら [`INITIAL_FONT_SIZE_PX`] | 同上 |
///
/// `ex` / `rex` / `ch` / `rch` / `ic` / `ric` は style 層に実 font metrics が
/// 無いため常に spec の unknown-metric fallback を使う — 根拠は各 variant
/// ([`Length::Ex`] 等) の doc、`font-size` 自身が font-* property のため
/// **親** 基準になる理由は上記 parent-metrics 条項 (`em` と同じ扱い)。
///
/// [`INITIAL_FONT_SIZE_PX`]: crate::computed::INITIAL_FONT_SIZE_PX
///
/// # `lh` / `rlh` の自己参照
///
/// CSS Values 4 §6.1.1 "Font-relative Lengths"
/// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) verbatim:
/// "Similarly, when lh or rlh units are used in the value of the line-height
/// property or font-\* properties on the element they refer to, they resolve
/// against the computed line-height and font metrics of the parent
/// element—or the computed metrics corresponding to the initial values of
/// the font and line-height properties, if the element has no parent."
/// `font-size` はまさにこの font-\* property であり、この条項が発火する。
///
/// [`resolve_line_height`] doc の「`Length::Lh` — 自己参照」/「`Length::Rlh`
/// — 自己参照ではなく tree-global 定数」節と**同じ判断**をここでも採る
/// (spec 引用が両者を "lh or rlh" と並べて一箇所に述べているため、`line-height`
/// 自身の自己参照解決とここで判断を変える理由がない — 一貫性を優先する):
///
/// - `lh` の素の定義 ("the element on which it is used") は使用要素自身を
///   常に指すため、`font-size` に使われた `lh` は常に自己参照になる。
///   fallback 基準は引用のとおり **親** — `self_reference_basis` 引数。
/// - `rlh` の素の定義 ("the lh unit on the root element") は宣言要素の位置に
///   依存しない tree-global 定数であり、自己参照になるのは宣言要素自身が
///   root element のときだけ ([`crate::specified::SpecifiedValues::finalize_as_root`]
///   が `ctx` に [`ResolveContext::initial`] を渡すことで `ctx.root_line_height`
///   を必然的に `None` にし、この一点をカバーする)。root **ではない**要素の
///   `font-size: 1rlh` は既に確定済みの別 node (root) の値を参照するだけで
///   自己参照ではないため、他の box property 上の `rlh` ([`resolve_length`] 等)
///   と同じく `ctx.root_line_height` を直接使う — `self_reference_basis`
///   ではなく `ctx.root_line_height` を読むのはこのため。
///
/// 基準が `None` (`normal` で解決不能、または root element で親が無い) の
/// ときは **`font-size` 自身の spec initial** (`medium` = [`INITIAL_FONT_SIZE_PX`]、
/// CSS Fonts 4 `font-size` propdef "Initial: medium") に倒す —
/// [`resolve_line_height`] の `Length::Lh` arm が解決不能なとき `line-height`
/// 自身の spec initial `normal` ([`ComputedLineHeight::Normal`]) に倒すのと
/// 同じ「解決できない宣言を、宣言されなかったのと同じ値に倒す」方針。
/// 本関数のような**単一 property 専用の** resolver は、汎用 resolver
/// ([`resolve_length`] / [`resolve_length_percentage`] — `border-*-width` /
/// `padding` など**複数** property で共有される) と違い、fallback 先として
/// 自分の consumer property の真の spec initial を直接返せる立場にある —
/// [`resolve_length_percentage_or_auto`] が `width`/`height` の fallback を
/// 汎用な `resolve_length_percentage` の `0px` に丸めず `Auto` (両者の真の
/// spec initial) に intercept するのと同じ判断。汎用 resolver 側が一律 `0px`
/// に倒すのは border-width の spec initial (`medium` = 3px) と一致しない
/// **既知の compromise** ([`resolve_length`] doc の「border-width: 1lh の
/// 0px fallback — 未解決の設計妥協」節) であって
/// 「単一 property 専用 resolver でも 0px に倒すべき」という一般原則ではない
/// — `resolve_font_size` はこの関数が `font-size` の唯一の consumer なので、
/// その compromise を持ち込む理由がない。
///
/// # Primary sources (§ title + anchor)
///
/// - CSS Values 4 §6.1.1 "Font-relative Lengths"
///   (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>): "When used
///   in the value of any font-* property on the element they refer to, the
///   font-relative lengths resolve against the computed metrics of the parent
///   element—or against the computed metrics corresponding to the initial values
///   of the font and line-height properties, if the element has no parent." →
///   `font-size` の `em` は **親基準** (自 font-size を参照すると self-reference
///   になるため)。
/// - CSS Fonts 4 §2.5 "Font size: the font-size property"
///   (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>):
///   "Percentages: refer to parent element's font size" /
///   "Computed value: an absolute length"。
///
/// # Caller contract
///
/// **root element の `font-size` を絶対化するときは [`ResolveContext::initial`]
/// を渡すこと。** `Rem` / `Rex` / `Rch` / `Ric` arm は `ctx.root_font_size` を
/// 無条件に参照するため、tree 全体で同一の `ResolveContext::new(root_font_size)`
/// を使い回すと `html { font-size: 2rem }` (同様に `2rex` / `2rch` / `2ric`) が
/// 自己参照になる (CSS Values 4 §6.1.1 の parent-metrics 条項 — root には親が
/// ないので initial values 基準)。**同じ理由で `self_reference_basis` にも
/// `None` を渡すこと** (root には親が無いので self-reference basis は
/// 「initial values」= `line-height: normal` = 解決不能)。
///
/// cascade pipeline ではこの contract を
/// [`SpecifiedValues::finalize_as_root`] が守る —
/// end-to-end の pin は [`mod@crate::cascade`] の
/// `rem_on_root_element_resolves_against_initial_font_size` /
/// `rem_below_root_element_resolves_against_root_computed_font_size` /
/// `rem_on_root_element_box_property_uses_own_font_size` の 3 本。
/// 本関数を直接呼ぶ code はこの contract を自分で守ること。
///
/// [`SpecifiedValues::finalize_as_root`]: crate::specified::SpecifiedValues::finalize_as_root
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext, resolve_font_size};
/// use raikiri_style::property::Length;
///
/// let ctx = ResolveContext::initial();
///
/// // em compounding: 16px → 1.5em → 1.5em = 24px → 36px
/// let child = resolve_font_size(Length::Em(1.5), ComputedLength(16.0), None, &ctx);
/// assert_eq!(child, ComputedLength(24.0));
/// let grandchild = resolve_font_size(Length::Em(1.5), child, None, &ctx);
/// assert_eq!(grandchild, ComputedLength(36.0));
///
/// // `1lh` resolves against the *parent's* used line-height
/// // — not the declaring element's own font-size. Parent: font-size 16px,
/// // `line-height: 1.5` (unitless) → used line-height 24px.
/// let self_reference_basis = Some(ComputedLength(24.0));
/// let font_size =
///     resolve_font_size(Length::Lh(2.0), ComputedLength(16.0), self_reference_basis, &ctx);
/// assert_eq!(font_size, ComputedLength(48.0));
/// ```
pub fn resolve_font_size(
    specified: Length,
    parent_font_size: ComputedLength,
    self_reference_basis: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLength {
    match specified {
        Length::Px(v) => ComputedLength(v),
        Length::Pt(v) => ComputedLength(pt_to_px(v)),
        Length::Cm(v) => ComputedLength(cm_to_px(v)),
        Length::Mm(v) => ComputedLength(mm_to_px(v)),
        Length::Q(v) => ComputedLength(q_to_px(v)),
        Length::In(v) => ComputedLength(in_to_px(v)),
        Length::Pc(v) => ComputedLength(pc_to_px(v)),
        Length::Em(v) => ComputedLength(parent_font_size.0 * v),
        Length::Rem(v) => ComputedLength(ctx.root_font_size.0 * v),
        // ex / ch: unknown-metric fallback = 0.5em (`Length::Ex` /
        // `Length::Ch` doc)。font-size 自身の値なので基準は親
        // (上記 parent-metrics 条項、`em` と同じ)。
        Length::Ex(v) | Length::Ch(v) => ComputedLength(parent_font_size.0 * v * 0.5),
        // ic: unknown-metric fallback = 1em (`Length::Ic` doc)。
        Length::Ic(v) => ComputedLength(parent_font_size.0 * v),
        // rex / rch: root 版の同じ fallback、基準は root_font_size (`rem` と同じ)。
        Length::Rex(v) | Length::Rch(v) => ComputedLength(ctx.root_font_size.0 * v * 0.5),
        Length::Ric(v) => ComputedLength(ctx.root_font_size.0 * v),
        // CSS Fonts 4 `font-size` propdef: "Percentages: refer to parent
        // element's font size" — font-size は §5.5.1 の「percentage は
        // percentage のまま computed される」原則の明示的な例外。
        Length::Percent(p) => ComputedLength(parent_font_size.0 * p / 100.0),
        // `lh` / `rlh` — 上記「`lh` / `rlh` の自己参照」
        // 節。基準が `None` のときの fallback は `font-size` 自身の spec
        // initial (`INITIAL_FONT_SIZE_PX`) — `resolve_length` /
        // `resolve_length_percentage` の汎用 `0px` fallback とは**意図的に
        // 異なる** (同節参照)。
        Length::Lh(v) => resolve_lh_multiplier(v, self_reference_basis)
            .unwrap_or(ComputedLength(INITIAL_FONT_SIZE_PX)),
        Length::Rlh(v) => resolve_lh_multiplier(v, ctx.root_line_height)
            .unwrap_or(ComputedLength(INITIAL_FONT_SIZE_PX)),
    }
}

/// `<length>` のみを取る property (grammar に `<percentage>` を含まないもの) の
/// specified value を絶対化する (**phase 3** — 自 node 基準)。
///
/// `font_size` は**自要素の** computed font-size (phase 2 で確定した値)。
/// `font-size` 自身の絶対化には [`resolve_font_size`] を使うこと (基準が親)。
///
/// # `Length::Percent` は grammar-unreachable
///
/// 本関数の in-crate consumer は 3 つある — `border-*-width`
/// ([`resolve_border`])、`line-height` の `<length>` 成分
/// ([`resolve_line_height`])、`letter-spacing` / `word-spacing`
/// ([`resolve_length_or_normal`])。いずれも `Length::Percent` を渡さない:
///
/// - `border-*-width`: CSS Backgrounds 3 §3.3 "Line Thickness: the
///   border-width properties"
///   (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) の grammar は
///   `<line-width> = <length [0,∞]> | thin | medium | thick` で `<percentage>`
///   を含まないため、percentage を含む declaration は parse 段で invalid として
///   drop される (`parse_border_width_side`)。
/// - `line-height`: [`resolve_line_height`] が `Length::Percent` を**本関数へ
///   delegate する前に intercept** して自要素の font-size で絶対化する
///   (CSS Inline 3 §5.1 "Percentages: computed relative to 1em")。
/// - `letter-spacing` / `word-spacing`: `border-*-width` と同じ shape — CSS
///   Text 3 §7.2 / §7.1 (`letter-spacing`
///   <https://www.w3.org/TR/css-text-3/#letter-spacing-property> /
///   `word-spacing` <https://www.w3.org/TR/css-text-3/#word-spacing-property>)
///   の grammar `normal | <length>` も `<percentage>` を含まない
///   ("Percentages: N/A" / "n/a")、percentage を含む declaration は parse 段で
///   invalid として drop される (`parse_letter_or_word_spacing`、
///   `allow_percentage=false`)。
///
/// **後者は invariant であり、grammar による保証ではない** —
/// [`resolve_line_height`] の match を「全 variant を本関数に delegate する」形に
/// 簡約すると `line-height: 150%` が下記 0px arm を踏んで黙って潰れる。簡約して
/// はならない。
///
/// 到達した場合は computed 層で意味を持たない値なので `0px` に落とす —
/// spec initial 相当の保守的な値であり、fail-quiet を許すためではなく
/// 「grammar 上ありえない入力に対する全域性」のための arm である。
///
/// これは設計文書 §4.6 が削除するとした下流 (`raikiri-dom`
/// `layout.rs`) の catch-all とは別物である (あちらは実際に削除済) —
/// あちらは **computed 層**の型を
/// match して `Em` / `Rem` という **spec-valid な入力**を黙って 0px に潰す
/// (= fail-quiet)。本 arm は **specified 層の [`Length`]** に対するもので、
/// 潰れる入力が grammar 上存在しない。
///
/// # `Length::Lh` / `Length::Rlh`
///
/// `own_line_height` は**呼び手が [`used_line_height_length`] であらかじめ
/// 絶対化した**、この関数が絶対化中の property を持つ要素**自身**の
/// line-height 基準 (`border-*-width` の呼び手 [`resolve_border`] がそう渡す)。
/// `rlh` は tree-global な `ctx.root_line_height` を参照する — [`Length::Lh`]
/// doc の「自己参照」節が対象とするのは `line-height` 自身の値としての
/// lh/rlh のみで、本関数は `line-height` の `<length>` 成分を delegate されて
/// も (`resolve_line_height` 参照) その delegation 自体が **すでに `Length::Lh`
/// / `Length::Rlh` を除外した後**なので、本関数の Lh/Rlh arm が「自己参照」
/// 問題を踏むことはない。
///
/// # `border-*-width: 1lh` の `0px` fallback — 未解決の設計妥協 (Finding B)
///
/// 基準が `None` (`normal` で解決不能、cap/rcap と同じ wall) のときは `0px` に
/// 倒す。**これは上記の `Percent` arm ("grammar 上ありえない入力") と同じ
/// 理由ではない** — `Length::Lh` / `Length::Rlh` は `border-*-width` の
/// grammar 上ふつうに到達しうる入力であり、"到達しない" という全域性の
/// 話ではなく、実際に踏まれうる値が `0px` に落ちるという意味のある挙動である。
///
/// CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) の border-width
/// 自身の spec initial は `medium` (= 3px、本 crate では
/// [`crate::specified::INITIAL_BORDER`] が既に扱う) であり、`0` は
/// `border-style` が `none`/`hidden` のときの gated 結果 (`resolve_border`
/// が別途処理する) であって、`lh` の解決可能性とは無関係。すなわち
/// `border-top-style: solid; border-top-width: 1lh` を `line-height: normal`
/// 下で書くと、意図しない**不可視**の border (`0px`) になる —
/// `resolve_length_percentage` の `Px(0.0)` fallback (`padding` の真の spec
/// initial と一致する) と違い、こちらの `0px` は border-width の spec
/// initial とも一致しない、単なる「他に選びようがなかった値」である。
///
/// **`letter-spacing` / `word-spacing` はこの不一致を持たない** —
/// [`resolve_length_or_normal`] 経由でこの fallback を踏む場合 (`1lh` を
/// `line-height: normal` 下で書いた場合)、`0px` は CSS Text 3 §7.2/§7.1 が
/// 定める `normal` の computed value そのもの ("Computes to zero.") と一致する
/// — `padding` の `Px(0.0)` fallback と同じ側であり、border-width の
/// "他に選びようがなかった値" 側ではない。
///
/// **`vertical-align: <length>` も同じ側 (`letter-spacing`/`word-spacing`
/// 寄り)** — [`resolve_vertical_align`] 経由でこの fallback を踏む場合
/// (`1lh` を `line-height: normal` 下で書いた場合)、`0px` は CSS 2.1
/// §10.8.1 の `<length>` 自身の spec verbatim ("The value `0cm` means the
/// same as `baseline`.") と一致する — `0px` shift = `baseline` と同じ
/// 効果であり、border-width の "他に選びようがなかった値" 側ではない。
///
/// この不整合は認識した上で **今回は直さない** — root 原因は
/// [`used_line_height_length`] doc の "normal" wall そのもの (real font
/// metrics が style 層に無い) であり、根本修正 (`ComputedLength` に
/// "unresolved" を表す手段を持たせる等) は今後の別途対応の範囲として扱う。
/// border-width 固有の「`medium` 相当へ倒す」代替案 (style gate 済みの
/// `resolve_border` が既に持つ判定ロジックを再利用できる見込みはある) も
/// その対応の中で検討することとし、本関数では `Percent` arm と
/// 同じコードパスに相乗りしない独立した設計判断として `0px` を明示的に
/// 選んでいる — 比率を捏造しない (cleanroom) という一線だけは守るが、
/// この `0px` 自体が border-width の正しい fallback だと主張するものではない。
pub(crate) fn resolve_length(
    specified: Length,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLength {
    match specified {
        Length::Px(v) => ComputedLength(v),
        Length::Pt(v) => ComputedLength(pt_to_px(v)),
        Length::Cm(v) => ComputedLength(cm_to_px(v)),
        Length::Mm(v) => ComputedLength(mm_to_px(v)),
        Length::Q(v) => ComputedLength(q_to_px(v)),
        Length::In(v) => ComputedLength(in_to_px(v)),
        Length::Pc(v) => ComputedLength(pc_to_px(v)),
        Length::Em(v) => ComputedLength(font_size.0 * v),
        Length::Rem(v) => ComputedLength(ctx.root_font_size.0 * v),
        // ex / ch / ic: 自要素基準の unknown-metric fallback
        // (`resolve_font_size` の同 arm と同じ 0.5em / 1em、基準のみ自要素)。
        Length::Ex(v) | Length::Ch(v) => ComputedLength(font_size.0 * v * 0.5),
        Length::Ic(v) => ComputedLength(font_size.0 * v),
        Length::Rex(v) | Length::Rch(v) => ComputedLength(ctx.root_font_size.0 * v * 0.5),
        Length::Ric(v) => ComputedLength(ctx.root_font_size.0 * v),
        Length::Percent(_) => ComputedLength::ZERO,
        Length::Lh(v) => resolve_lh_multiplier(v, own_line_height).unwrap_or(ComputedLength::ZERO),
        Length::Rlh(v) => {
            resolve_lh_multiplier(v, ctx.root_line_height).unwrap_or(ComputedLength::ZERO)
        }
    }
}

/// `normal | <length>` を取る property (`letter-spacing` / `word-spacing`) の
/// specified value を絶対化する (**phase 3** — 自 node 基準)。
///
/// `Normal` は常に [`ComputedLength::ZERO`] — CSS Text 3 §7.1
/// (<https://www.w3.org/TR/css-text-3/#word-spacing-property>) / §7.2
/// (<https://www.w3.org/TR/css-text-3/#letter-spacing-property>) がいずれも
/// "No additional spacing is applied. Computes to zero." と明記する。
/// `Length` 側は [`resolve_length`] へそのまま delegate する
/// ([`LengthOrNormal`] は percentage を持たないため、`resolve_length_percentage`
/// ではなく percentage 非対応の [`resolve_length`] が正しい delegate 先)。
///
/// [`ComputedLineHeight::Normal`] とは異なり、computed 層で keyword を保持
/// **しない** — 両 property とも spec の "Computed value" が "an absolute
/// length" であり、"normal" 自体は computed value の選択肢に含まれない
/// (`line-height` の "Computed value: … normal" とはこの点で異なる)。
pub fn resolve_length_or_normal(
    specified: LengthOrNormal,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLength {
    match specified {
        LengthOrNormal::Normal => ComputedLength::ZERO,
        LengthOrNormal::Length(l) => resolve_length(l, font_size, own_line_height, ctx),
    }
}

/// `tab-size` の specified value を絶対化する (**phase 3** — 自 node 基準)。
///
/// CSS Text Module Level 3 §4.2 propdef: "Computed value: the specified
/// number or absolute length" — [`TabSize::Number`] は素通し
/// ([`ComputedTabSize`] doc / [`crate::property::TabSize`] doc の scope
/// carving 節参照: 実際の tab stop advance の解決は font metric に依存した
/// downstream consumer の仕事)。[`TabSize::Length`] 側は [`resolve_length`]
/// へそのまま delegate する ([`TabSize`] は percentage を持たないため、
/// [`resolve_length_percentage`] ではなく percentage 非対応の
/// [`resolve_length`] が正しい delegate 先 — [`resolve_length_or_normal`]
/// と同型)。
pub fn resolve_tab_size(
    specified: TabSize,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedTabSize {
    match specified {
        TabSize::Number(n) => ComputedTabSize::Number(n),
        TabSize::Length(l) => {
            ComputedTabSize::Length(resolve_length(l, font_size, own_line_height, ctx))
        }
    }
}

/// `vertical-align: baseline | sub | super | middle | text-top |
/// text-bottom | <length>` の specified value を絶対化する (**phase 3** —
/// 自 node 基準)。
///
/// CSS 2.1 §10.8.1 propdef: "Computed value: for `<percentage>` and
/// `<length>` the absolute length, otherwise as specified" — 6 keyword は
/// computed 層でもそのまま keyword、`<length>` だけが絶対化対象。
///
/// # 戻り値が [`VerticalAlign`] 自身であること (別の `ComputedVerticalAlign`
/// 型を新設しない理由)
///
/// [`FlexBasisValue`]/[`ComputedFlexBasis`] のような specified/computed
/// 型分離パターンをここでは**採らない** — `raikiri-paint` 側 (`walk.rs` の
/// `vertical_align_shift_px`) が [`crate::computed::ComputedValues::vertical_align`]
/// の型として [`VerticalAlign`] を直接引数に取っており、別の computed 専用
/// 型へ差し替えると raikiri-paint 側の signature 変更を要求してしまう。本
/// crate の scope はこの property の raikiri-paint 側 wiring には一切
/// 触れないことなので、[`Length`] を絶対化した上で同じ [`VerticalAlign`]
/// enum の [`VerticalAlign::Length`] variant へ詰め直して返す —
/// [`crate::page`] の `fb` helper が [`ComputedFlexBasis`] を
/// [`FlexBasisValue`] へ詰め直すのと構造は同じだが、そちら側の型変換
/// (`Computed* → specified 型`) を経由せず、絶対化前後で常に同じ型のまま
/// 完結する点が異なる。
pub fn resolve_vertical_align(
    specified: VerticalAlign,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> VerticalAlign {
    match specified {
        VerticalAlign::Baseline
        | VerticalAlign::Sub
        | VerticalAlign::Super
        | VerticalAlign::Middle
        | VerticalAlign::TextTop
        | VerticalAlign::TextBottom => specified,
        VerticalAlign::Length(l) => VerticalAlign::Length(Length::Px(
            resolve_length(l, font_size, own_line_height, ctx).px(),
        )),
    }
}

/// `<length-percentage>` を取る property (`padding-*`) の specified value を
/// 絶対化する (**phase 3** — 自 node 基準)。
///
/// `Percent` は **絶対化せず素通し** — CSS Values 4 §5.5.1
/// (<https://www.w3.org/TR/css-values-4/#combine-percentages>) の
/// "the computed value of a percentage is the specified percentage" のとおり、
/// containing block width への解決は used value 層 (CSS Cascade 5 §4.5
/// <https://www.w3.org/TR/css-cascade-5/#used>、raikiri では taffy) の責務。
///
/// # `Length::Lh` / `Length::Rlh`
///
/// `own_line_height` は[`resolve_length`]の同名引数と同じ契約 — 呼び手が
/// [`used_line_height_length`] であらかじめ絶対化した、この property を持つ
/// 要素自身の line-height 基準。基準が `None` (`normal` で解決不能) のときは
/// `padding` の spec initial value である **`0`** に倒す (CSS Box 3 §4
/// <https://www.w3.org/TR/css-box-3/#padding-physical> "Initial: 0") —
/// これは font-metrics の比率を捏造した値ではなく、「この crate の style 層
/// では解決できない宣言を、宣言されなかったのと同じ値に倒す」という
/// per-property fallback である。**cascade の正式な declaration-drop
/// (次点候補への fall-through) とは異なる** — winner 選択は既に完了して
/// おり、本関数はその 1 件だけを initial 相当に倒す。
pub fn resolve_length_percentage(
    specified: Length,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthPercentage {
    match specified {
        Length::Px(v) => ComputedLengthPercentage::Px(v),
        Length::Pt(v) => ComputedLengthPercentage::Px(pt_to_px(v)),
        Length::Cm(v) => ComputedLengthPercentage::Px(cm_to_px(v)),
        Length::Mm(v) => ComputedLengthPercentage::Px(mm_to_px(v)),
        Length::Q(v) => ComputedLengthPercentage::Px(q_to_px(v)),
        Length::In(v) => ComputedLengthPercentage::Px(in_to_px(v)),
        Length::Pc(v) => ComputedLengthPercentage::Px(pc_to_px(v)),
        Length::Em(v) => ComputedLengthPercentage::Px(font_size.0 * v),
        Length::Rem(v) => ComputedLengthPercentage::Px(ctx.root_font_size.0 * v),
        // ex / ch / ic: `resolve_length` と同じ fallback ratio。
        Length::Ex(v) | Length::Ch(v) => ComputedLengthPercentage::Px(font_size.0 * v * 0.5),
        Length::Ic(v) => ComputedLengthPercentage::Px(font_size.0 * v),
        Length::Rex(v) | Length::Rch(v) => {
            ComputedLengthPercentage::Px(ctx.root_font_size.0 * v * 0.5)
        }
        Length::Ric(v) => ComputedLengthPercentage::Px(ctx.root_font_size.0 * v),
        Length::Percent(p) => ComputedLengthPercentage::Percent(p),
        Length::Lh(v) => ComputedLengthPercentage::Px(
            resolve_lh_multiplier(v, own_line_height)
                .map(ComputedLength::px)
                .unwrap_or(0.0),
        ),
        Length::Rlh(v) => ComputedLengthPercentage::Px(
            resolve_lh_multiplier(v, ctx.root_line_height)
                .map(ComputedLength::px)
                .unwrap_or(0.0),
        ),
    }
}

/// `<length-percentage> | auto` を取る property (**`width` / `height` /
/// `flex-basis`** — `margin-*` は [`resolve_margin_length_or_auto`] を
/// 使うこと、下記 "Finding A" 節参照) の specified value を絶対化する
/// (**phase 3** — 自 node 基準)。
///
/// `flex-basis` ([`resolve_flex_basis`] 経由) が 3 人目の caller なのは
/// `<'width'>` reuse (CSS Flexible Box Layout Module Level 1 §7.2.3、
/// [`crate::property::FlexBasisValue`] doc 参照) の直接の帰結 — spec 上の
/// propdef grammar が文字通り `width` の grammar を再利用しているため、`auto`
/// / `Lh`/`Rlh` 解決不能時の fallback も `width`/`height` と同じ `Auto` が
/// 正しい (flex-basis の spec initial も `auto`、"宣言されなかったのと同じ値に
/// 倒す" という本関数の設計方針がそのまま適用できる — `margin-*` を除外する
/// 理由とは無関係な独立の一致)。
///
/// `Auto` は computed 層でも keyword のまま。`Percent` の扱いは
/// [`resolve_length_percentage`] と同じ (素通し、used value 層で解決)。
///
/// # `Length::Lh` / `Length::Rlh` の解決不能 fallback は `Auto`
///
/// [`resolve_length_percentage`] へ丸ごと delegate**しない** — 基準
/// (`own_line_height` / `ctx.root_line_height`) が `None` (`normal` で解決
/// 不能) のとき、[`resolve_length_percentage`] は `Px(0.0)` を返すが、`width`/
/// `height` の spec initial は `auto` であって `0` ではない (CSS Sizing 3
/// §3.1.1 <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
/// `Px(0.0)` を返すと spec に反するため、本関数は `Lh`/`Rlh` を intercept して
/// 解決不能な場合 `Auto` を返す — 「解決できない宣言は、宣言されなかったのと
/// 同じ値に倒す」という [`resolve_length_percentage`] と同じ設計方針を、
/// `width`/`height` にとって真の spec initial である `Auto` に合わせて
/// 適用したもの。
///
/// # Finding A — `margin-*` は本関数を使わない
///
/// 当初 `margin-*` もこの関数の consumer に含めていたが、spec 指摘により訂正した:
/// margin の spec initial (CSS Box 3 §3.1
/// <https://www.w3.org/TR/css-box-3/#margin-physical> "Initial: 0") は
/// **definite length `0`** であって `auto` ではない — `width`/`height` とは
/// 逆に `Px(0.0)` こそが margin の真の spec initial である。加えて `auto` は
/// margin では「available space を分配する」という**実際のレイアウト動作**
/// (taffy の auto-margin centering、`raikiri-dom/src/layout.rs` の
/// `length_percentage_auto_to_taffy` 参照) を引き起こす spec keyword であり、
/// 単なる「無指定を表す中立値」ではない。本関数の `Auto` fallback を margin
/// にも適用すると、`line-height: normal` という common case
/// (`<div style="line-height: normal; margin-top: 1lh">`) で spec に無い
/// 具体的なレイアウト挙動を勝手に発火させてしまう。
pub fn resolve_length_percentage_or_auto(
    specified: LengthOrAuto,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthPercentageOrAuto {
    match specified {
        LengthOrAuto::Auto => ComputedLengthPercentageOrAuto::Auto,
        LengthOrAuto::Length(Length::Lh(v)) => match resolve_lh_multiplier(v, own_line_height) {
            Some(c) => ComputedLengthPercentageOrAuto::Px(c.px()),
            None => ComputedLengthPercentageOrAuto::Auto,
        },
        LengthOrAuto::Length(Length::Rlh(v)) => {
            match resolve_lh_multiplier(v, ctx.root_line_height) {
                Some(c) => ComputedLengthPercentageOrAuto::Px(c.px()),
                None => ComputedLengthPercentageOrAuto::Auto,
            }
        }
        LengthOrAuto::Length(len) => {
            match resolve_length_percentage(len, font_size, own_line_height, ctx) {
                ComputedLengthPercentage::Px(v) => ComputedLengthPercentageOrAuto::Px(v),
                ComputedLengthPercentage::Percent(p) => ComputedLengthPercentageOrAuto::Percent(p),
            }
        }
    }
}

/// `flex-basis: content | <'width'>` の specified value を絶対化する
/// (**phase 3** — 自 node 基準)。
///
/// `content` はそのまま keyword として素通し ([`ComputedFlexBasis::Content`]、
/// [`crate::property::FlexBasisValue`] doc の scope carving 節参照)。`auto` /
/// `<length-percentage>` 側は `<'width'>` reuse の通り
/// [`resolve_length_percentage_or_auto`] と全く同じ shape (`Lh`/`Rlh` 解決
/// 不能時の `Auto` fallback を含む) — 実装を複製せず delegate する。
pub fn resolve_flex_basis(
    specified: FlexBasisValue,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedFlexBasis {
    match specified {
        FlexBasisValue::Content => ComputedFlexBasis::Content,
        FlexBasisValue::Auto => ComputedFlexBasis::Auto,
        FlexBasisValue::Length(len) => {
            match resolve_length_percentage_or_auto(
                LengthOrAuto::Length(len),
                font_size,
                own_line_height,
                ctx,
            ) {
                ComputedLengthPercentageOrAuto::Auto => ComputedFlexBasis::Auto,
                ComputedLengthPercentageOrAuto::Px(v) => ComputedFlexBasis::Px(v),
                ComputedLengthPercentageOrAuto::Percent(p) => ComputedFlexBasis::Percent(p),
            }
        }
    }
}

/// `row-gap` / `column-gap`: `normal | <length-percentage [0,∞]>` の
/// specified value を絶対化する (**phase 3** — 自 node 基準)。
///
/// `normal` は computed 層でも keyword のまま残る
/// ([`ComputedLengthPercentageOrNormal`] doc 参照、`letter-spacing`/
/// `word-spacing` の `normal → ComputedLength::ZERO`
/// ([`resolve_length_or_normal`]) とは異なる spec 文言のため意図的に
/// 別関数)。`<length-percentage>` 側は [`resolve_length_percentage`] へ
/// delegate (percentage は素通し — used value 層は下流 (taffy) 責務)。
pub fn resolve_length_percentage_or_normal(
    specified: LengthOrNormal,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthPercentageOrNormal {
    match specified {
        LengthOrNormal::Normal => ComputedLengthPercentageOrNormal::Normal,
        LengthOrNormal::Length(len) => {
            match resolve_length_percentage(len, font_size, own_line_height, ctx) {
                ComputedLengthPercentage::Px(v) => ComputedLengthPercentageOrNormal::Px(v),
                ComputedLengthPercentage::Percent(p) => {
                    ComputedLengthPercentageOrNormal::Percent(p)
                }
            }
        }
    }
}

/// `<length-percentage> | auto` を取る **`margin-*`専用** の absolutization
/// (Finding A)。
///
/// [`resolve_length_percentage_or_auto`] と shape は同じ (`Auto` keyword は
/// そのまま、`<length-percentage>` は [`resolve_length_percentage`] に
/// delegate) だが、**`Lh`/`Rlh` 専用の intercept を持たない** — その結果、
/// 解決不能 (`normal`) なときの fallback は [`resolve_length_percentage`]
/// がそのまま返す `Px(0.0)` になる。これは margin の spec initial (CSS Box 3
/// §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical> "Initial: 0")
/// そのものであり、`resolve_length_percentage_or_auto` が `width`/`height`
/// のために返す `Auto` (margin にとっては spec 上根拠のない値かつ、taffy の
/// auto-margin centering という実際のレイアウト動作を誘発する) とは意図的に
/// 異なる。共有関数 [`resolve_length_percentage_or_auto`] 自体の fallback は
/// 変えない — 外部から見える public API の挙動を、根拠の無い margin 側の
/// 都合で `width`/`height` の呼び手ごと変えるのは影響範囲が広すぎる
/// (この関数は pub なので margin/width/height 以外の将来の呼び手が居ても
/// 安全なよう、変更は margin 専用の本関数に閉じる)。
pub fn resolve_margin_length_or_auto(
    specified: LengthOrAuto,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthPercentageOrAuto {
    match specified {
        LengthOrAuto::Auto => ComputedLengthPercentageOrAuto::Auto,
        LengthOrAuto::Length(len) => {
            match resolve_length_percentage(len, font_size, own_line_height, ctx) {
                ComputedLengthPercentage::Px(v) => ComputedLengthPercentageOrAuto::Px(v),
                ComputedLengthPercentage::Percent(p) => ComputedLengthPercentageOrAuto::Percent(p),
            }
        }
    }
}

/// `line-height` の specified value を絶対化する (**phase 3 / phase 2.5** —
/// 自 node 基準。呼び手の doc "phase 2.5" 節参照 —
/// [`crate::specified::SpecifiedValues::finalize`] /
/// [`crate::specified::SpecifiedValues::finalize_as_root`])。
///
/// - `normal` / `<number>` は素通し。`<number>` を computed 層に残すのは spec 上
///   load-bearing な distinction (子は number を inherit して**自分の**
///   font-size に掛ける)。
/// - `<percentage>` は **自要素の** computed font-size に対して絶対化する —
///   CSS Inline 3 §5.1 (<https://www.w3.org/TR/css-inline-3/#propdef-line-height>)
///   "Percentages: computed relative to 1em" + CSS Values 4 §6.1.1 `em`
///   (<https://www.w3.org/TR/css-values-4/#em>) "Equal to the computed value of
///   the font-size property of the element on which it is used."
/// - `<length>` (`Lh` / `Rlh` を除く) は [`resolve_length`] と同じ規則で
///   絶対化する。
///
/// # `Length::Lh` — 自己参照
///
/// `line-height: 1lh` は「自分の computed line-height」を自分の値として
/// 使う自己参照になる — `lh` の素の定義 ("the element on which it is used")
/// は常に「使用要素自身」を指すため、この自己参照は**あらゆる要素**で起こる。
/// CSS Values 4 §6.1.1 "Font-relative Lengths"
/// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) verbatim:
/// "Similarly, when lh or rlh units are used in the value of the line-height
/// property or font-\* properties on the element they refer to, they resolve
/// against the computed line-height and font metrics of the parent
/// element—or the computed metrics corresponding to the initial values of
/// the font and line-height properties, if the element has no parent."
///
/// `self_reference_basis` は呼び手があらかじめ [`used_line_height_length`]
/// で絶対化した**親の** line-height (親が無い root element では `None` —
/// 「initial values」= `line-height: normal` は解決不能なので `None` が
/// そのまま正しい基準になる、[`ResolveContext::initial`] doc 参照)。基準が
/// `None` のときは `line-height` 自身の spec initial value である
/// **`normal`** ([`ComputedLineHeight::Normal`]) に倒す —
/// [`resolve_length_percentage`] の `0px` fallback と同じ「解決できない
/// 宣言を宣言前の状態に倒す」方針を、`line-height` にとって最も自然な
/// 「無指定」状態に適用したもの。
///
/// # `Length::Rlh` — 自己参照ではなく tree-global 定数 (`Lh` と非対称)
///
/// 上記引用は "lh or rlh" と両方を並べているが、**`rlh` はこの crate では
/// 自己参照として扱わない** — `rlh` の素の定義 ("Equal to the value of the
/// lh unit **on the root element**") は宣言要素の位置に依存しない tree-global
/// な定数であり、循環参照が起こり得るのは宣言要素自身が root element の
/// ときだけ ([`crate::specified::SpecifiedValues::finalize_as_root`] が
/// その一点をカバーする — root では `ctx` に
/// [`ResolveContext::initial`] を渡すため `ctx.root_line_height` は
/// 必然的に `None`)。root **ではない**要素の `line-height: 1rlh` は
/// 既に確定済みの別 node (root) の値を参照するだけで自己参照ではないため、
/// 他の box property 上の `rlh` と同じく `ctx.root_line_height` を直接
/// 使う — `self_reference_basis` (**親**の line-height) を使うと `rlh` の
/// 素の定義に反する誤った基準 (親の line-height) を使ってしまう。
/// 引用文の "Similarly" は「自己参照が起こり得る場面では同じ fallback 構造を
/// 使う」ことを述べているに過ぎず、`rlh` について「循環しない場面でも親を
/// 参照せよ」と読むのは `rlh` 自身の定義と矛盾するため採らない。
pub fn resolve_line_height(
    specified: LineHeight,
    font_size: ComputedLength,
    self_reference_basis: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLineHeight {
    match specified {
        LineHeight::Normal => ComputedLineHeight::Normal,
        LineHeight::Number(n) => ComputedLineHeight::Number(n),
        LineHeight::Length(Length::Lh(v)) => match resolve_lh_multiplier(v, self_reference_basis) {
            Some(c) => ComputedLineHeight::Length(c),
            None => ComputedLineHeight::Normal,
        },
        LineHeight::Length(Length::Rlh(v)) => {
            match resolve_lh_multiplier(v, ctx.root_line_height) {
                Some(c) => ComputedLineHeight::Length(c),
                None => ComputedLineHeight::Normal,
            }
        }
        LineHeight::Length(len) => ComputedLineHeight::Length(match len {
            // CSS Inline 3 §5.1 "Percentages: computed relative to 1em" —
            // percentage は宣言要素の computed font-size で絶対化される
            // (`resolve_length` の grammar-unreachable な 0px arm には
            // 落とさない)。
            Length::Percent(p) => ComputedLength(font_size.0 * p / 100.0),
            // `Lh` / `Rlh` は上の arm で既に払い出し済み — ここに来る `other`
            // が Lh/Rlh になることはない (`own_line_height: None` は死に引数)。
            other => resolve_length(other, font_size, None, ctx),
        }),
    }
}

/// `border-*` 1 side 分の specified value を絶対化する (**phase 3** — 自 node
/// 基準)。
///
/// `width` を絶対化し、`style` / `color` は specified keyword をそのまま運ぶ。
///
/// # style gating は computed 層の要求
///
/// `style` が `none` / `hidden` のとき `width` は **0px** になる。CSS Backgrounds
/// 3 §3.3 "Line Thickness: the border-width properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) の propdef table が
/// "Computed value: absolute length, snapped as a border width; zero if the
/// border style is `none` or `hidden`" と規定するとおり、これは used 層ではなく
/// **computed 層**の要求である (**TR 版**; version marker は下記の
/// "version marker" 節参照)。
///
/// **spec tension (silently 解決しない)**: 同 §3.3 の非規範 Note は "Although the
/// initial width is medium, the initial style is none; therefore the used initial
/// width is 0." と **used** 層で述べる一方、規範な propdef table は **computed**
/// 層を指定している。Note は非規範なので propdef table が governs。
///
/// **version marker**: 上記は TR
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) の記述。ED
/// (<https://drafts.csswg.org/css-backgrounds-3/#border-width>) は CSSWG
/// [Issue 11494](https://github.com/w3c/csswg-drafts/issues/11494) により
/// この gate を **computed → resolved/used 層へ移動**する規定変更を経ている
/// (computed value 行から
/// "zero if the border style is `none` or `hidden`" 節が消え、代わりに
/// "The resolved value for the border-width properties is the used value.
/// If the border-style corresponding to a given border-width is none or
/// hidden, then the used width is 0." が本文に追加されている)。**本関数の
/// gate 位置は TR に従っており、ED には未追随** — TR が現行 Recommendation-
/// track の版であり、ED 追随は W3C process 上いつになるか不明なため
/// (緊急度は低いと判断済: 現行の paged-media path では resolved value は
/// 両版とも 0 になるため observable な差は無く、animation の
/// "by computed value" 補間の起点のみが異なる)。TR に追随する既定の gate 位置
/// (computed 層、本関数) が変わる場合は raikiri-dom 側の `used_border_width`
/// 相当を復活させる判断が要る — used 層に戻すのはこの issue の対象外であり、
/// 決定なしに変更しないこと。
///
/// **本関数は gate の単一 source である (element 経路 / page 経路の両方)** —
/// `raikiri-dom` の `layout.rs` は以前、同じ gating を used 層
/// (`used_border_width` helper) で 1 層遅れて行っていたが、`layout.rs` を
/// [`ComputedBorder`] consumer に migrate した際に削除した。
/// 下流に同じ判定を再実装してはならない (spec 規則の二重実装は片方だけ直す
/// drift を生む)。
///
/// page 経路 (`@page`) は `PropertyValue` の bag を運ぶが、
/// [`crate::page::cascade_page`] の phase 3 が `border-*-width` longhand を
/// [`Border`] に組み直して**本関数へ funnel する** — `matches!(style, None |
/// Hidden)` を page 側で書き直してはならない。longhand には color が無いので
/// placeholder を渡すが、本関数は width の判定に color を読まない。
/// `border-*-style` **未宣言**時の基準は [`crate::specified::INITIAL_BORDER`] の
/// `style` (= `none`) であり、CSS Paged Media 3 §6 "Page Properties"
/// <https://www.w3.org/TR/css-page-3/#page-properties> の "both the page context
/// and the margin context have a computed value for every property" が根拠。
///
/// # CAVEAT: border-image
///
/// CSS Backgrounds 3 §3.2 "Line Patterns: the border-style properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-style>) の `none` 定義は
/// "No border. Color and width are ignored (i.e., the border has width 0). Note
/// this means that the initial value of `border-image-width` will also resolve to
/// zero." であり、§3.3 の Computed value 行と整合する (border-image に対する
/// 例外を作らない)。`border-image-*` は未着手
/// (`ComputedValues::border` doc の Non-goals) なので現状 gate 位置の再検討は
/// 不要だが、着手時には両 section を読み直すこと。
/// `own_line_height` — 呼び手が [`used_line_height_length`]
/// であらかじめ絶対化した、この border を持つ要素自身の line-height 基準。
/// `border-*-width: 1lh` の resolve に使う ([`resolve_length`] の同名引数と
/// 同じ契約)。`None` (`normal` で解決不能) のときは `resolve_length` が
/// `0px` に倒す。
pub fn resolve_border(
    specified: Border,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedBorder {
    // `matches!` + else 枝: 未知の future `BorderStyle` variant は「visible な
    // style」側に落として specified width を透過させる (spec 上 visible な style
    // が追加されたときに width が黙って 0 にならないよう fail-safe に倒す)。
    // (下流の `layout.rs` は本 gate の結果を受け取るだけで再判定しない。)
    let width = if matches!(specified.style, BorderStyle::None | BorderStyle::Hidden) {
        ComputedLength::ZERO
    } else {
        resolve_length(specified.width, font_size, own_line_height, ctx)
    };
    ComputedBorder {
        width,
        style: specified.style,
        color: specified.color,
    }
}

// ---------------------------------------------------------------------------
// computed → specified の lift (inheritance seed 用)
// ---------------------------------------------------------------------------

/// 親の computed `font-size` を specified 表現に **lift** する
/// (inheritance seed 用)。
///
/// 3 phase 構成では phase 1 (winner の staging) の入力が specified 型なので、
/// inheritance で運ばれてきた親の computed value を specified 表現に戻す必要が
/// ある。CSS Values 4 §6 (<https://www.w3.org/TR/css-values-4/#lengths>) が
/// computed length を「任意の絶対単位で表現してよい」としており、px として
/// 表現するのは**値の恒等変換**なので lossless。
///
/// かつ `Px` は絶対化の**不動点** ([`resolve_font_size`] の `Px` arm は identity)
/// なので、lift した値を **phase 2** (font-size の絶対化) に通しても二重適用に
/// ならない。`font-size` の絶対化は phase 2 であり phase 3 ではない —
/// phase 3 (`resolve_length` 系) が基準として受け取るのは、この phase 2 が
/// 確定させた自 node の computed font-size である。
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext, lift_font_size, resolve_font_size};
///
/// let ctx = ResolveContext::initial();
/// let inherited = ComputedLength(24.0);
///
/// // lift → 絶対化 の round trip は恒等 (Px が不動点)。
/// let lifted = lift_font_size(inherited);
/// assert_eq!(resolve_font_size(lifted, ComputedLength(16.0), None, &ctx), inherited);
/// ```
pub fn lift_font_size(computed: ComputedLength) -> Length {
    Length::Px(computed.0)
}

/// 親の computed `line-height` を specified 表現に **lift** する
/// (inheritance seed 用)。
///
/// [`lift_font_size`] と同じ lossless 性が [`ComputedLineHeight`] の **3 variant
/// すべて**で成立する。
///
/// - `Length(ComputedLength(30.0))` → `Length(Length::Px(30.0))`。
///   phase 3 を通しても `30px` のまま — すなわち **font-size がより小さい子は
///   30px をそのまま継承し、percentage を再解決しない**。これは CSS Inline 3
///   §5.1 (<https://www.w3.org/TR/css-inline-3/#propdef-line-height>) の
///   `Computed value: … a computed <length> value` が要求する挙動である
///   (percentage は宣言要素で絶対化され、子はその length を継承する)。
///   **「子の font-size で再 resolve すべき」ではない。**
/// - `Number(1.5)` は素通しで、子自身の font-size に掛かる (spec 上
///   load-bearing な distinction)。
/// - `Normal` は素通し。keyword のまま継承され、used value 層で解決される。
///
/// ```
/// use raikiri_style::{
///     ComputedLength, ComputedLineHeight, ResolveContext, lift_line_height,
///     resolve_line_height,
/// };
/// use raikiri_style::property::{Length, LineHeight};
///
/// let ctx = ResolveContext::initial();
///
/// // 宣言要素 (font-size 20px) で `line-height: 150%` を絶対化 → 30px
/// let declared = resolve_line_height(
///     LineHeight::Length(Length::Percent(150.0)),
///     ComputedLength(20.0),
///     None,
///     &ctx,
/// );
/// assert_eq!(declared, ComputedLineHeight::Length(ComputedLength(30.0)));
///
/// // 子 (font-size 10px) は 30px を **そのまま** 継承する (15px ではない)。
/// let child = resolve_line_height(lift_line_height(declared), ComputedLength(10.0), None, &ctx);
/// assert_eq!(child, ComputedLineHeight::Length(ComputedLength(30.0)));
/// ```
pub fn lift_line_height(computed: ComputedLineHeight) -> LineHeight {
    match computed {
        ComputedLineHeight::Normal => LineHeight::Normal,
        ComputedLineHeight::Number(n) => LineHeight::Number(n),
        ComputedLineHeight::Length(l) => LineHeight::Length(Length::Px(l.0)),
    }
}

/// 親の computed `<length-percentage>` を specified 表現に **lift** する
/// (inheritance seed 用) — [`lift_font_size`] / [`lift_line_height`] と
/// 同じ役目を [`ComputedLengthPercentage`] に対して果たす。
///
/// [`lift_font_size`] と同じ lossless 性が両 variant で成立する:
///
/// - `Px(v)` → `Length::Px(v)`。`Px` は絶対化の不動点
///   ([`resolve_length_percentage`] の `Px` arm は identity) なので、lift
///   した値を phase 3 に通しても二重適用にならない。
/// - `Percent(p)` → `Length::Percent(p)`。[`resolve_length_percentage`] の
///   `Percent` arm も identity ("the computed value of a percentage is the
///   specified percentage", CSS Values 4 §5.5.1
///   <https://www.w3.org/TR/css-values-4/#combine-percentages>) — 子は親の
///   `%` をそのまま継承し、containing block 基準の再解決はしない (used
///   value 層 = 下流 layout の責務、[`ComputedLengthPercentage`] doc 参照)。
///
/// 現在の唯一の consumer は `text-indent` (CSS Text 3 §8.1、**inherited**
/// `<length-percentage>` property) — [`crate::specified::SpecifiedValues::inherit_from`]
/// がこの関数で親の `ComputedValues::text_indent` を子の staging へ seed する。
///
/// ```
/// use raikiri_style::{
///     ComputedLength, ComputedLengthPercentage, ResolveContext, lift_length_percentage,
///     resolve_length_percentage,
/// };
///
/// let ctx = ResolveContext::initial();
/// let inherited = ComputedLengthPercentage::Px(40.0);
///
/// // lift → 絶対化 の round trip は恒等 (Px が不動点)。
/// let lifted = lift_length_percentage(inherited);
/// assert_eq!(
///     resolve_length_percentage(lifted, ComputedLength(10.0), None, &ctx),
///     inherited
/// );
///
/// // `Percent` も同じく恒等 — containing block 基準は used value 層まで
/// // 再解決しない。
/// let inherited_pct = ComputedLengthPercentage::Percent(10.0);
/// let lifted_pct = lift_length_percentage(inherited_pct);
/// assert_eq!(
///     resolve_length_percentage(lifted_pct, ComputedLength(10.0), None, &ctx),
///     inherited_pct
/// );
/// ```
pub fn lift_length_percentage(computed: ComputedLengthPercentage) -> Length {
    match computed {
        ComputedLengthPercentage::Px(v) => Length::Px(v),
        ComputedLengthPercentage::Percent(p) => Length::Percent(p),
    }
}

/// 親の computed `letter-spacing` / `word-spacing` を specified 表現に
/// **lift** する (inheritance seed 用)。
///
/// [`lift_font_size`] と同じ lossless / fixed-point 性 — [`resolve_length_or_normal`]
/// の `Length` branch は [`resolve_length`] へ delegate し、その `Px` arm は
/// identity (`ComputedLength(v) -> Length::Px(v)` を素通し) なので、lift した
/// 値を phase 3 に再度通しても二重適用にならない。
///
/// [`lift_line_height`] とは異なり `Normal` を復元しない — `letter-spacing: normal`
/// / `word-spacing: normal` の computed value は spec 上すでに `0` (an absolute
/// length、[`resolve_length_or_normal`] doc 参照) であり、`Normal` keyword は
/// computed 層に一切現れないため、"lift 元" の情報として残っていない
/// ([`ComputedLineHeight`] が `Normal` variant を保持し続けるのとの違いは
/// [`resolve_length_or_normal`] doc 参照)。
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext, lift_length_or_normal, resolve_length_or_normal};
/// use raikiri_style::property::LengthOrNormal;
///
/// let ctx = ResolveContext::initial();
/// let inherited = ComputedLength(2.0);
///
/// // lift → 絶対化 の round trip は恒等 (Px が不動点)。
/// let lifted = lift_length_or_normal(inherited);
/// assert_eq!(
///     resolve_length_or_normal(lifted, ComputedLength(16.0), None, &ctx),
///     inherited
/// );
/// assert_eq!(lifted, LengthOrNormal::Length(raikiri_style::property::Length::Px(2.0)));
/// ```
pub fn lift_length_or_normal(computed: ComputedLength) -> LengthOrNormal {
    LengthOrNormal::Length(Length::Px(computed.0))
}

/// 親の [`ComputedTabSize`] を specified 表現 ([`TabSize`]) に **lift** する
/// (inheritance seed 用) — [`lift_line_height`] と同じ lossless / 不動点性。
///
/// - `Number(n)` → `TabSize::Number(n)`。[`resolve_tab_size`] の `Number`
///   arm は identity なので不動点。
/// - `Length(l)` → `TabSize::Length(Length::Px(l.px()))`。
///   [`resolve_length`] の `Px` arm は identity なので不動点
///   ([`lift_font_size`] と同じ根拠)。
///
/// ```
/// use raikiri_style::{
///     ComputedLength, ComputedTabSize, ResolveContext, lift_tab_size, resolve_tab_size,
/// };
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(16.0);
///
/// // lift → 絶対化 の round trip は恒等 (Px が不動点)。
/// let inherited = ComputedTabSize::Length(ComputedLength(32.0));
/// let lifted = lift_tab_size(inherited);
/// assert_eq!(resolve_tab_size(lifted, font_size, None, &ctx), inherited);
///
/// // `Number` も素通しのまま不動点。
/// let inherited_number = ComputedTabSize::Number(4.0);
/// assert_eq!(
///     resolve_tab_size(lift_tab_size(inherited_number), font_size, None, &ctx),
///     inherited_number
/// );
/// ```
pub fn lift_tab_size(computed: ComputedTabSize) -> TabSize {
    match computed {
        ComputedTabSize::Number(n) => TabSize::Number(n),
        ComputedTabSize::Length(l) => TabSize::Length(Length::Px(l.px())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::Sides;
    use crate::specified::SpecifiedValues;

    /// `root_font_size` = 16px の共通 context (`rem` の参照値)。
    /// `Rem` を含む test の期待値はこの 16px に依存する — 変更すると落ちる。
    /// 自 node / 親の font-size は各 test が引数で個別に渡す。
    const CTX: ResolveContext = ResolveContext {
        root_font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
        root_line_height: None,
    };

    // -----------------------------------------------------------------
    // ComputedLength / ResolveContext の基本契約
    // -----------------------------------------------------------------

    #[test]
    fn computed_length_zero_and_px_accessor() {
        assert_eq!(ComputedLength::ZERO, ComputedLength(0.0));
        assert_eq!(ComputedLength(12.5).px(), 12.5);
    }

    #[test]
    fn resolve_context_new_stores_root_font_size() {
        assert_eq!(
            ResolveContext::new(ComputedLength(20.0)).root_font_size,
            ComputedLength(20.0),
        );
    }

    // -----------------------------------------------------------------
    // phase 2: font-size の絶対化 (親基準)
    // -----------------------------------------------------------------

    #[test]
    fn font_size_px_is_identity() {
        assert_eq!(
            resolve_font_size(Length::Px(18.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(18.0),
        );
    }

    /// `1pt = 1/72in`、`1in = 96px` → `12pt = 16px`
    /// (CSS Values 4 §6.2 <https://www.w3.org/TR/css-values-4/#absolute-lengths>)。
    #[test]
    fn font_size_pt_converts_at_96px_per_inch() {
        assert_eq!(
            resolve_font_size(Length::Pt(12.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(16.0),
        );
    }

    /// `font-size` の `em` は **親** の computed font-size 基準
    /// (CSS Values 4 §6.1.1 parent-metrics 条項)。
    #[test]
    fn font_size_em_resolves_against_parent_font_size() {
        assert_eq!(
            resolve_font_size(Length::Em(1.5), ComputedLength(16.0), None, &CTX),
            ComputedLength(24.0),
        );
    }

    /// `em` の compounding: 16px → 1.5em → 1.5em = 24px → 36px。
    #[test]
    fn font_size_em_compounds_across_two_levels() {
        let child = resolve_font_size(Length::Em(1.5), ComputedLength(16.0), None, &CTX);
        let grandchild = resolve_font_size(Length::Em(1.5), child, None, &CTX);
        assert_eq!(child, ComputedLength(24.0));
        assert_eq!(grandchild, ComputedLength(36.0));
    }

    /// `rem` は root element の computed font-size 基準
    /// (CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>)。
    /// 親の font-size には**依存しない**。
    #[test]
    fn font_size_rem_resolves_against_root_font_size() {
        let ctx = ResolveContext::new(ComputedLength(20.0));
        assert_eq!(
            resolve_font_size(Length::Rem(2.0), ComputedLength(64.0), None, &ctx),
            ComputedLength(40.0),
        );
    }

    /// root element 自身の `font-size: Nrem` は initial value (16px) 基準
    /// (CSS Values 4 §6.1.1 parent-metrics 条項 —
    /// root element には親がないため initial values を参照する)。
    #[test]
    fn font_size_rem_on_root_element_uses_initial_font_size() {
        let ctx = ResolveContext::initial();
        assert_eq!(
            resolve_font_size(
                Length::Rem(2.0),
                ComputedLength(INITIAL_FONT_SIZE_PX),
                None,
                &ctx
            ),
            ComputedLength(32.0),
        );
    }

    /// `font-size` の `<percentage>` は親の font-size 基準で **length になる**
    /// (CSS Fonts 4 `font-size` propdef "Percentages: refer to parent element's
    /// font size" — CSS Values 4 §5.5.1 の明示的例外)。
    #[test]
    fn font_size_percent_resolves_against_parent_font_size() {
        assert_eq!(
            resolve_font_size(Length::Percent(150.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(24.0),
        );
    }

    /// `ex` / `ch` は style 層に real font metrics が無いため常に spec の
    /// unknown-metric fallback (`0.5em`) を使う (`Length::Ex` / `Length::Ch`
    /// doc)。`font-size` 上では他 font-relative unit と同じく **親** 基準
    /// (self-reference avoidance)。
    #[test]
    fn font_size_ex_and_ch_resolve_against_parent_font_size_with_half_em_fallback() {
        assert_eq!(
            resolve_font_size(Length::Ex(2.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(16.0), // 2 * 0.5 * 16
        );
        assert_eq!(
            resolve_font_size(Length::Ch(2.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(16.0),
        );
    }

    /// `ic` の unknown-metric fallback は `1em` (`Length::Ic` doc)。
    #[test]
    fn font_size_ic_resolves_against_parent_font_size_with_one_em_fallback() {
        assert_eq!(
            resolve_font_size(Length::Ic(1.5), ComputedLength(16.0), None, &CTX),
            ComputedLength(24.0),
        );
    }

    /// `rex` / `rch` / `ric` は root element 基準 (`rem` と同じ、親の font-size
    /// には依存しない)。
    #[test]
    fn font_size_r_prefixed_font_relative_units_resolve_against_root_font_size() {
        let ctx = ResolveContext::new(ComputedLength(20.0));
        assert_eq!(
            resolve_font_size(Length::Rex(2.0), ComputedLength(64.0), None, &ctx),
            ComputedLength(20.0), // 2 * 0.5 * 20 (親 64px は無視)
        );
        assert_eq!(
            resolve_font_size(Length::Rch(2.0), ComputedLength(64.0), None, &ctx),
            ComputedLength(20.0),
        );
        assert_eq!(
            resolve_font_size(Length::Ric(2.0), ComputedLength(64.0), None, &ctx),
            ComputedLength(40.0),
        );
    }

    /// `font-size: 1lh` resolves against the **parent's** used line-height
    /// (CSS Values 4 §6.1.1's self-reference clause,
    /// same判断 as `resolve_line_height`'s `Length::Lh` arm). The `parent`
    /// argument to `resolve_font_size` (16px, unrelated) is deliberately
    /// different from `parent_line_height_basis` (30px) so a bug that
    /// conflates "parent's font-size" with "parent's line-height" would be
    /// caught.
    #[test]
    fn font_size_lh_resolves_against_parent_line_height_basis() {
        assert_eq!(
            resolve_font_size(
                Length::Lh(2.0),
                ComputedLength(16.0),
                Some(ComputedLength(30.0)),
                &CTX,
            ),
            ComputedLength(60.0), // 2 * 30
        );
    }

    /// `font-size: 1rlh` resolves against `ctx.root_line_height` — a
    /// tree-global constant, **not** `parent_line_height_basis` (mirrors
    /// `resolve_line_height`'s `Length::Rlh`
    /// arm and its "not self-referential for non-root elements" rationale).
    /// `parent_line_height_basis` is deliberately set to a different value
    /// (30px) than `ctx.root_line_height` (50px) so a bug that swaps the two
    /// bases would be caught.
    #[test]
    fn font_size_rlh_resolves_against_root_line_height_not_parent() {
        let ctx =
            ResolveContext::with_root_line_height(ComputedLength(16.0), Some(ComputedLength(50.0)));
        assert_eq!(
            resolve_font_size(
                Length::Rlh(2.0),
                ComputedLength(16.0),
                Some(ComputedLength(30.0)), // must NOT be read by the `Rlh` arm
                &ctx,
            ),
            ComputedLength(100.0), // 2 * 50, not 2 * 30
        );
    }

    /// `font-size: 1lh` / `1rlh` fall back to `font-size`'s own spec initial
    /// (`medium` = `INITIAL_FONT_SIZE_PX`) when their basis is unresolvable
    /// (`normal` with no font metrics — the same wall as `cap`/`rcap`, or a
    /// root element with no parent) — **not** `0px`. Unlike the generic,
    /// multi-property `resolve_length`/`resolve_length_percentage` (whose
    /// flat `0px` fallback is a known compromise for `border-width`, tracked
    /// separately), `resolve_font_size` is a
    /// dedicated single-property resolver and can fall back to its own true
    /// initial directly — same convention as `resolve_line_height`'s `Lh`
    /// arm falling back to `line-height`'s own initial `normal`.
    #[test]
    fn font_size_lh_and_rlh_fall_back_to_initial_when_basis_is_unresolved() {
        assert_eq!(
            resolve_font_size(Length::Lh(2.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(INITIAL_FONT_SIZE_PX),
        );
        assert_eq!(
            resolve_font_size(Length::Rlh(2.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(INITIAL_FONT_SIZE_PX),
        );
    }

    /// CSS Values 4 §6.2 "Absolute Lengths" 換算表 verbatim: `1in = 96px` /
    /// `1cm = 96px/2.54` / `1mm = 1/10th of 1cm` / `1Q = 1/40th of 1cm` /
    /// `1pc = 1/6th of 1in`。expected 側は decimal literal ではなく spec と同じ
    /// 式で書く — `96.0/2.54` に正確な 10 進表現は無いため、実装の評価順
    /// (`cm_to_px` / `pc_to_px` 経由の連鎖) と揃えて f32 rounding を bit 単位で
    /// 一致させる。
    #[test]
    fn font_size_additional_absolute_units_convert_per_spec_table() {
        assert_eq!(
            resolve_font_size(Length::In(1.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(96.0),
        );
        assert_eq!(
            resolve_font_size(Length::Cm(1.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(96.0 / 2.54),
        );
        assert_eq!(
            resolve_font_size(Length::Mm(1.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(96.0 / 2.54 / 10.0),
        );
        assert_eq!(
            resolve_font_size(Length::Q(1.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(96.0 / 2.54 / 40.0),
        );
        assert_eq!(
            resolve_font_size(Length::Pc(1.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(96.0 / 6.0),
        );
    }

    // -----------------------------------------------------------------
    // phase 3: `<length>` (border-width) の絶対化 (自 node 基準)
    // -----------------------------------------------------------------

    #[test]
    fn length_px_and_pt_are_absolute() {
        assert_eq!(
            resolve_length(Length::Px(3.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(3.0),
        );
        assert_eq!(
            resolve_length(Length::Pt(9.0), ComputedLength(16.0), None, &CTX),
            ComputedLength(12.0),
        );
    }

    /// `font-size` 以外の property の `em` は **自要素** の computed font-size
    /// 基準 (CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#em>)。
    #[test]
    fn length_em_resolves_against_own_font_size() {
        assert_eq!(
            resolve_length(Length::Em(2.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(40.0),
        );
    }

    #[test]
    fn length_rem_resolves_against_root_font_size() {
        let ctx = ResolveContext::new(ComputedLength(10.0));
        assert_eq!(
            resolve_length(Length::Rem(2.5), ComputedLength(64.0), None, &ctx),
            ComputedLength(25.0),
        );
    }

    /// `border-*-width` の grammar (`<line-width>`) は `<percentage>` を含まない
    /// ため parse 段で drop される。到達不能 arm の全域性のみを pin する。
    #[test]
    fn length_percent_is_grammar_unreachable_and_falls_to_zero() {
        assert_eq!(
            resolve_length(Length::Percent(50.0), ComputedLength(20.0), None, &CTX),
            ComputedLength::ZERO,
        );
    }

    /// `font-size` 以外 (= `resolve_length` の呼び出し先である `border-*-width`
    /// や `line-height` の `<length>` 成分) では `ex` / `ch` / `ic` は
    /// **自要素** の computed font-size 基準になる — `resolve_font_size` の
    /// 同 unit テスト (親基準) との非対称を pin する
    /// (`Length::Ex` doc の parent-metrics 条項)。
    #[test]
    fn length_ex_ch_ic_resolve_against_own_font_size() {
        assert_eq!(
            resolve_length(Length::Ex(2.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(20.0), // 2 * 0.5 * 20
        );
        assert_eq!(
            resolve_length(Length::Ch(2.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(20.0),
        );
        assert_eq!(
            resolve_length(Length::Ic(2.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(40.0),
        );
    }

    #[test]
    fn length_r_prefixed_font_relative_units_resolve_against_root_font_size() {
        let ctx = ResolveContext::new(ComputedLength(10.0));
        assert_eq!(
            resolve_length(Length::Rex(2.5), ComputedLength(64.0), None, &ctx),
            ComputedLength(12.5), // 2.5 * 0.5 * 10 (自 font-size 64px は無視)
        );
        assert_eq!(
            resolve_length(Length::Ric(2.5), ComputedLength(64.0), None, &ctx),
            ComputedLength(25.0),
        );
    }

    /// CSS Values 4 §6.2 換算表 — `border-*-width` 経由 (`resolve_length`) でも
    /// `resolve_font_size` と同じ変換になることを pin
    /// (`width: 1in` → 96px、issue 本文の verification 対象)。
    #[test]
    fn length_additional_absolute_units_convert_per_spec_table() {
        assert_eq!(
            resolve_length(Length::In(1.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(96.0),
        );
        assert_eq!(
            resolve_length(Length::Pc(1.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(96.0 / 6.0),
        );
        assert_eq!(
            resolve_length(Length::Mm(1.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(96.0 / 2.54 / 10.0),
        );
        assert_eq!(
            resolve_length(Length::Q(1.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(96.0 / 2.54 / 40.0),
        );
        assert_eq!(
            resolve_length(Length::Cm(1.0), ComputedLength(20.0), None, &CTX),
            ComputedLength(96.0 / 2.54),
        );
    }

    // -----------------------------------------------------------------
    // phase 3: `<length-percentage>` (padding) の絶対化
    // -----------------------------------------------------------------

    #[test]
    fn length_percentage_absolutizes_lengths() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_length_percentage(Length::Px(10.0), fs, None, &CTX),
            ComputedLengthPercentage::Px(10.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Pt(6.0), fs, None, &CTX),
            ComputedLengthPercentage::Px(8.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Em(2.0), fs, None, &CTX),
            ComputedLengthPercentage::Px(40.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Rem(0.5), fs, None, &CTX),
            ComputedLengthPercentage::Px(8.0),
        );
    }

    /// `resolve_length_percentage` (`padding-*` の絶対化関数) 側でも
    /// 追加した全 unit を直接 exercise する
    /// (`resolve_font_size` / `resolve_length` の同 unit test とは別 site —
    /// 3 関数それぞれが独立した match を持つため、patch coverage は
    /// 関数単位で見る)。
    #[test]
    fn length_percentage_absolutizes_additional_units() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_length_percentage(Length::Ex(2.0), fs, None, &CTX),
            ComputedLengthPercentage::Px(20.0), // 2 * 0.5 * 20
        );
        assert_eq!(
            resolve_length_percentage(Length::Ch(2.0), fs, None, &CTX),
            ComputedLengthPercentage::Px(20.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Ic(1.5), fs, None, &CTX),
            ComputedLengthPercentage::Px(30.0),
        );
        let ctx = ResolveContext::new(ComputedLength(10.0));
        assert_eq!(
            resolve_length_percentage(Length::Rex(2.0), fs, None, &ctx),
            ComputedLengthPercentage::Px(10.0), // 2 * 0.5 * 10 (自 20px は無視)
        );
        assert_eq!(
            resolve_length_percentage(Length::Rch(2.0), fs, None, &ctx),
            ComputedLengthPercentage::Px(10.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Ric(2.0), fs, None, &ctx),
            ComputedLengthPercentage::Px(20.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Cm(1.0), fs, None, &CTX),
            ComputedLengthPercentage::Px(96.0 / 2.54),
        );
        assert_eq!(
            resolve_length_percentage(Length::Mm(1.0), fs, None, &CTX),
            ComputedLengthPercentage::Px(96.0 / 2.54 / 10.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Q(1.0), fs, None, &CTX),
            ComputedLengthPercentage::Px(96.0 / 2.54 / 40.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::In(1.0), fs, None, &CTX),
            ComputedLengthPercentage::Px(96.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Pc(1.0), fs, None, &CTX),
            ComputedLengthPercentage::Px(96.0 / 6.0),
        );
    }

    /// `padding: 1lh` — own line-height が解決済 (`Some`) なら乗数として使う。
    #[test]
    fn length_percentage_lh_multiplies_own_line_height_basis() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_length_percentage(Length::Lh(1.5), fs, Some(ComputedLength(24.0)), &CTX),
            ComputedLengthPercentage::Px(36.0), // 1.5 * 24
        );
    }

    /// `padding: 1lh` — own line-height が `normal` で解決不能 (`None`) の
    /// ときは padding の spec initial value `0` に倒す (cleanroom: 比率を
    /// 捏造しない、`resolve_length_percentage` doc 参照)。
    #[test]
    fn length_percentage_lh_falls_back_to_zero_when_unresolvable() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_length_percentage(Length::Lh(1.5), fs, None, &CTX),
            ComputedLengthPercentage::Px(0.0),
        );
    }

    /// `padding: 1rlh` — tree-global な `ctx.root_line_height` を基準にする
    /// (own line-height ではない)。
    #[test]
    fn length_percentage_rlh_multiplies_root_line_height_basis() {
        let fs = ComputedLength(20.0);
        let ctx = ResolveContext::with_root_line_height(
            ComputedLength(16.0),
            Some(ComputedLength(19.2)), // root: line-height: normal 相当ではなく既知の px
        );
        assert_eq!(
            resolve_length_percentage(Length::Rlh(2.0), fs, Some(ComputedLength(999.0)), &ctx),
            ComputedLengthPercentage::Px(38.4), // own_line_height (999) は無視、root だけ使う
        );
    }

    /// `<length-percentage>` の percentage は computed 層に **percentage のまま**
    /// 残る (CSS Values 4 §5.5.1 / CSS Box 3 `padding-top` "Computed value: a
    /// computed `<length-percentage>` value")。authored 数値をそのまま保持し
    /// `/ 100` もしない。
    #[test]
    fn length_percentage_percent_passes_through_unchanged() {
        assert_eq!(
            resolve_length_percentage(Length::Percent(50.0), ComputedLength(20.0), None, &CTX),
            ComputedLengthPercentage::Percent(50.0),
        );
    }

    // -----------------------------------------------------------------
    // phase 3: `<length-percentage> | auto` (margin / width / height)
    // -----------------------------------------------------------------

    #[test]
    fn length_percentage_or_auto_keeps_auto() {
        assert_eq!(
            resolve_length_percentage_or_auto(LengthOrAuto::Auto, ComputedLength(16.0), None, &CTX),
            ComputedLengthPercentageOrAuto::Auto,
        );
    }

    #[test]
    fn length_percentage_or_auto_absolutizes_and_passes_percent() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_length_percentage_or_auto(
                LengthOrAuto::Length(Length::Em(1.5)),
                fs,
                None,
                &CTX
            ),
            ComputedLengthPercentageOrAuto::Px(30.0),
        );
        assert_eq!(
            resolve_length_percentage_or_auto(
                LengthOrAuto::Length(Length::Percent(25.0)),
                fs,
                None,
                &CTX
            ),
            ComputedLengthPercentageOrAuto::Percent(25.0),
        );
    }

    /// `width: 1lh` (`resolve_length_percentage_or_auto` — `width`/`height`
    /// only since Finding A) —
    /// resolvable な own line-height なら乗数として使う。
    /// `resolve_length_percentage_or_auto` は `Lh`/`Rlh` を
    /// `resolve_length_percentage` へ delegate**しない** (fallback が違う、
    /// 次のテスト参照) が、resolvable な場合の数値は一致する。
    #[test]
    fn length_percentage_or_auto_lh_multiplies_own_line_height_basis() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_length_percentage_or_auto(
                LengthOrAuto::Length(Length::Lh(1.5)),
                fs,
                Some(ComputedLength(24.0)),
                &CTX,
            ),
            ComputedLengthPercentageOrAuto::Px(36.0),
        );
    }

    /// `width: 1lh` — own line-height が `normal` で解決不能なら **`Auto`**
    /// に倒す (`width`/`height`'s spec initial, CSS Sizing 3 §3.1.1) —
    /// `resolve_length_percentage`'s `0px` fallback とは異なる。**margin
    /// はもう本関数を通らない** (Finding A) —
    /// margin の同型テストは `resolve_margin_length_or_auto_lh_falls_back_to_zero_when_unresolvable`
    /// を参照。
    #[test]
    fn length_percentage_or_auto_lh_falls_back_to_auto_when_unresolvable() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_length_percentage_or_auto(
                LengthOrAuto::Length(Length::Lh(1.5)),
                fs,
                None,
                &CTX
            ),
            ComputedLengthPercentageOrAuto::Auto,
        );
        let ctx_no_root_lh = ResolveContext::new(ComputedLength(16.0));
        assert_eq!(
            resolve_length_percentage_or_auto(
                LengthOrAuto::Length(Length::Rlh(1.0)),
                fs,
                Some(ComputedLength(999.0)), // own line-height は rlh に無関係
                &ctx_no_root_lh,
            ),
            ComputedLengthPercentageOrAuto::Auto,
        );
    }

    /// `margin-top: 1lh` (Finding A)
    /// — resolvable な own line-height なら乗数として使う。Numerically
    /// identical to `resolve_length_percentage_or_auto`'s answer when
    /// resolvable — only the unresolvable fallback differs (next test).
    #[test]
    fn resolve_margin_length_or_auto_lh_multiplies_own_line_height_basis() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_margin_length_or_auto(
                LengthOrAuto::Length(Length::Lh(1.5)),
                fs,
                Some(ComputedLength(24.0)),
                &CTX,
            ),
            ComputedLengthPercentageOrAuto::Px(36.0),
        );
    }

    /// The Finding A regression pin: `margin-top: 1lh` / `1rlh` under
    /// `line-height: normal` (unresolvable) must compute to **`Px(0.0)`**
    /// — margin's actual spec initial (CSS Box 3 §3.1) — not `Auto`
    /// (`resolve_length_percentage_or_auto`'s fallback, which is correct
    /// for `width`/`height` but was wrongly shared with `margin` before this
    /// fix; `Auto` triggers real taffy auto-margin layout, not a neutral
    /// "unspecified" value, per `resolve_margin_length_or_auto`'s doc).
    #[test]
    fn resolve_margin_length_or_auto_lh_falls_back_to_zero_when_unresolvable() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_margin_length_or_auto(LengthOrAuto::Length(Length::Lh(1.5)), fs, None, &CTX),
            ComputedLengthPercentageOrAuto::Px(0.0),
        );
        let ctx_no_root_lh = ResolveContext::new(ComputedLength(16.0));
        assert_eq!(
            resolve_margin_length_or_auto(
                LengthOrAuto::Length(Length::Rlh(1.0)),
                fs,
                Some(ComputedLength(999.0)), // own line-height は rlh に無関係
                &ctx_no_root_lh,
            ),
            ComputedLengthPercentageOrAuto::Px(0.0),
        );
    }

    /// `margin: auto` itself must still pass through as `Auto` —
    /// `resolve_margin_length_or_auto` only changes the `Lh`/`Rlh`
    /// unresolvable fallback, not the literal `auto` keyword's own meaning.
    #[test]
    fn resolve_margin_length_or_auto_keeps_auto_keyword() {
        assert_eq!(
            resolve_margin_length_or_auto(LengthOrAuto::Auto, ComputedLength(16.0), None, &CTX),
            ComputedLengthPercentageOrAuto::Auto,
        );
    }

    /// `margin-top: 50%` — percentage still passes through un-absolutized,
    /// same as `resolve_length_percentage_or_auto` (used-value layer input).
    #[test]
    fn resolve_margin_length_or_auto_keeps_percent() {
        assert_eq!(
            resolve_margin_length_or_auto(
                LengthOrAuto::Length(Length::Percent(50.0)),
                ComputedLength(16.0),
                None,
                &CTX,
            ),
            ComputedLengthPercentageOrAuto::Percent(50.0),
        );
    }

    // -----------------------------------------------------------------
    // phase 3: line-height
    // -----------------------------------------------------------------

    #[test]
    fn line_height_normal_passes_through() {
        assert_eq!(
            resolve_line_height(LineHeight::Normal, ComputedLength(20.0), None, &CTX),
            ComputedLineHeight::Normal,
        );
    }

    /// `<number>` は computed 層でも number のまま (CSS Inline 3 §5.1
    /// "Computed value: the specified keyword, a number, or a computed
    /// `<length>` value")。
    #[test]
    fn line_height_number_passes_through() {
        assert_eq!(
            resolve_line_height(LineHeight::Number(1.5), ComputedLength(20.0), None, &CTX),
            ComputedLineHeight::Number(1.5),
        );
    }

    /// `<percentage>` は **自要素** の computed font-size に対して絶対化される
    /// (CSS Inline 3 §5.1 "Percentages: computed relative to 1em")。
    /// `150% × 20px = 30px`。
    #[test]
    fn line_height_percent_resolves_against_own_font_size() {
        assert_eq!(
            resolve_line_height(
                LineHeight::Length(Length::Percent(150.0)),
                ComputedLength(20.0),
                None,
                &CTX
            ),
            ComputedLineHeight::Length(ComputedLength(30.0)),
        );
    }

    #[test]
    fn line_height_length_absolutizes_font_relative_units() {
        assert_eq!(
            resolve_line_height(
                LineHeight::Length(Length::Em(1.2)),
                ComputedLength(20.0),
                None,
                &CTX
            ),
            ComputedLineHeight::Length(ComputedLength(24.0)),
        );
        assert_eq!(
            resolve_line_height(
                LineHeight::Length(Length::Px(24.0)),
                ComputedLength(20.0),
                None,
                &CTX
            ),
            ComputedLineHeight::Length(ComputedLength(24.0)),
        );
    }

    /// `line-height: 1lh` is self-referential (CSS Values 4 §6.1.1, spec
    /// quote + `lh`/`rlh` asymmetry rationale canonically documented on
    /// `resolve_line_height`). When the parent's
    /// own line-height is resolvable, `lh` multiplies by it — `own
    /// font_size` (the 2nd arg) and `ctx.root_line_height` are **not**
    /// consulted at all for this case, only `self_reference_basis` is.
    #[test]
    fn line_height_lh_resolves_against_parent_self_reference_basis() {
        let parent_basis = Some(ComputedLength(24.0)); // parent's used line-height
        let ctx_with_unrelated_root = ResolveContext::with_root_line_height(
            ComputedLength(16.0),
            Some(ComputedLength(999.0)), // must not leak into `lh`'s answer
        );
        assert_eq!(
            resolve_line_height(
                LineHeight::Length(Length::Lh(1.5)),
                ComputedLength(999.0), // own font-size, irrelevant here
                parent_basis,
                &ctx_with_unrelated_root,
            ),
            ComputedLineHeight::Length(ComputedLength(36.0)),
        );
    }

    /// `rlh`, unlike `lh`, is **not** treated as self-referential in this
    /// crate — its own definition ("the lh unit on the root element") is a
    /// tree-global constant, not something that depends on the declaring
    /// element (see `resolve_line_height`'s doc, "`Length::Rlh` — 自己参照
    /// ではなく tree-global 定数" section, for why the literal "Similarly,
    /// lh or rlh" spec wording is not followed for `rlh` on non-root
    /// elements). So `line-height: 1rlh` on a *non-root* element reads
    /// `ctx.root_line_height`, **not** `self_reference_basis` (the parent's
    /// line-height) — this is the discriminating test: parent and root
    /// bases are deliberately different values.
    #[test]
    fn line_height_rlh_resolves_against_root_not_parent_self_reference_basis() {
        let parent_basis = Some(ComputedLength(999.0)); // must not leak into `rlh`'s answer
        let ctx = ResolveContext::with_root_line_height(
            ComputedLength(16.0),
            Some(ComputedLength(24.0)), // root's used line-height
        );
        assert_eq!(
            resolve_line_height(
                LineHeight::Length(Length::Rlh(0.5)),
                ComputedLength(999.0),
                parent_basis,
                &ctx,
            ),
            ComputedLineHeight::Length(ComputedLength(12.0)), // 0.5 * 24 (root), not 0.5 * 999
        );
    }

    /// When the parent's own line-height is `normal` (unresolvable — no font
    /// metrics in the style layer, same wall as `cap`/`rcap`) or there is no
    /// parent (`self_reference_basis: None`, root element case — CSS Values
    /// 4 §6.1.1's "if the element has no parent" clause reduces to `normal`
    /// there too, see `SpecifiedValues::finalize_as_root` doc), `1lh` falls
    /// back to `line-height`'s own spec initial value `normal` rather than
    /// inventing a length (this is the per-property
    /// fallback chosen for the "normal" wall, distinct from
    /// `resolve_length_percentage`'s `0px` / `resolve_length_percentage_or_auto`'s
    /// `Auto`, because `normal` is what "unspecified" actually means for
    /// this property).
    #[test]
    fn line_height_lh_falls_back_to_normal_when_self_reference_basis_unresolvable() {
        assert_eq!(
            resolve_line_height(
                LineHeight::Length(Length::Lh(1.5)),
                ComputedLength(20.0),
                None,
                &CTX,
            ),
            ComputedLineHeight::Normal,
        );
    }

    /// Same fallback for `rlh`, but keyed off `ctx.root_line_height` instead
    /// of `self_reference_basis` — this is what makes root's own
    /// self-referential `1rlh` (`self_reference_basis` irrelevant there,
    /// `finalize_as_root` never passes it) *and* a normal-rooted document's
    /// descendants both land on `normal` without special-casing which node
    /// is which.
    #[test]
    fn line_height_rlh_falls_back_to_normal_when_root_line_height_unresolvable() {
        assert_eq!(
            resolve_line_height(
                LineHeight::Length(Length::Rlh(1.5)),
                ComputedLength(20.0),
                Some(ComputedLength(999.0)), // parent basis must not rescue rlh
                &CTX,                        // CTX.root_line_height == None
            ),
            ComputedLineHeight::Normal,
        );
    }

    // -----------------------------------------------------------------
    // phase 3: border
    // -----------------------------------------------------------------

    #[test]
    fn border_absolutizes_width_and_carries_style_and_color() {
        let mut specified = SpecifiedValues::initial().border.top;
        specified.style = BorderStyle::Solid;
        let computed = resolve_border(specified, ComputedLength(20.0), None, &CTX);
        // specified `medium` = 3px (CSS Backgrounds 3 §3.3: thin/medium/thick は
        // 1px/3px/5px に**規範的に**等価)。
        assert_eq!(computed.width, ComputedLength(3.0));
        assert_eq!(computed.style, specified.style);
        assert_eq!(computed.color, specified.color);
    }

    /// `border-*-width: 1lh` / `1rlh` — resolvable な
    /// 基準なら乗数、`None` (`normal` で解決不能) なら `resolve_length` の
    /// grammar-unreachable `Percent` arm と同じ `0px` に倒す。
    #[test]
    fn border_width_resolves_lh_and_rlh() {
        let mut specified = SpecifiedValues::initial().border.top;
        specified.style = BorderStyle::Solid;
        specified.width = Length::Lh(2.0);
        assert_eq!(
            resolve_border(
                specified,
                ComputedLength(20.0),
                Some(ComputedLength(10.0)),
                &CTX
            )
            .width,
            ComputedLength(20.0), // 2 * 10
        );
        // `own_line_height: None` (normal で解決不能) → 0px。
        assert_eq!(
            resolve_border(specified, ComputedLength(20.0), None, &CTX).width,
            ComputedLength::ZERO,
        );

        let mut rlh_specified = specified;
        rlh_specified.width = Length::Rlh(1.5);
        let ctx =
            ResolveContext::with_root_line_height(ComputedLength(16.0), Some(ComputedLength(20.0)));
        assert_eq!(
            // own_line_height (999) は `rlh` に無関係 — `ctx.root_line_height` だけ使う。
            resolve_border(
                rlh_specified,
                ComputedLength(20.0),
                Some(ComputedLength(999.0)),
                &ctx
            )
            .width,
            ComputedLength(30.0), // 1.5 * 20
        );
    }

    /// `ComputedBorder::width()` / `::style()` accessor 本体を実行する pin。
    /// 上の test は同一モジュール内なので
    /// `pub(crate)` field に直接アクセスし、accessor 関数本体そのものは
    /// 経由しない。crate 外視点から accessor を叩く doctest (`ComputedBorder`
    /// 型 doc 内) はあるが、この repo の toolchain (stable 固定、
    /// `cargo llvm-cov` に `--doctests` 未指定) では doctest はカバレッジ計測
    /// 対象に入らないため、本 test が accessor 本体を計測対象として実行する。
    #[test]
    fn computed_border_accessors_read_the_gated_fields() {
        let mut specified = SpecifiedValues::initial().border.top;
        specified.style = BorderStyle::Solid;
        let computed = resolve_border(specified, ComputedLength(20.0), None, &CTX);
        assert_eq!(computed.width(), ComputedLength(3.0));
        assert_eq!(computed.style(), BorderStyle::Solid);
    }

    /// CSS Backgrounds 3 §3.3 "Computed value: … zero if the border style is
    /// none or hidden" — style gating は **computed 層**の要求。
    #[test]
    fn border_width_is_zero_when_style_is_none_or_hidden() {
        let mut b = SpecifiedValues::initial().border.top;
        b.width = Length::Px(5.0);
        for style in [BorderStyle::None, BorderStyle::Hidden] {
            b.style = style;
            assert_eq!(
                resolve_border(b, ComputedLength(20.0), None, &CTX).width,
                ComputedLength::ZERO,
            );
        }
        b.style = BorderStyle::Solid;
        assert_eq!(
            resolve_border(b, ComputedLength(20.0), None, &CTX).width,
            ComputedLength(5.0),
        );
    }

    #[test]
    fn border_em_width_resolves_against_own_font_size() {
        let mut specified = SpecifiedValues::initial().border.top;
        specified.width = Length::Em(0.5);
        specified.style = BorderStyle::Solid;
        let computed = resolve_border(specified, ComputedLength(20.0), None, &CTX);
        assert_eq!(computed.width, ComputedLength(10.0));
    }

    /// 追加した absolute unit (`pc`) も
    /// `border-*-width` の style gating (この module doc / `resolve_border`
    /// doc の "spec tension" 節) と組み合わさって正しく解決する — `1pc = 16px`
    /// (CSS Values 4 §6.2)。`style: none` では新 unit も他 unit と同じく 0px に
    /// gate される (regression pin: この gate は絶対化の**後**に効くため、
    /// unit を増やしても gate 自体の網羅性は変わらない)。
    #[test]
    fn border_pc_width_is_absolutized_and_still_gated_by_style() {
        let mut specified = SpecifiedValues::initial().border.top;
        specified.width = Length::Pc(1.0);
        specified.style = BorderStyle::Solid;
        assert_eq!(
            resolve_border(specified, ComputedLength(20.0), None, &CTX).width,
            ComputedLength(16.0),
        );
        specified.style = BorderStyle::None;
        assert_eq!(
            resolve_border(specified, ComputedLength(20.0), None, &CTX).width,
            ComputedLength::ZERO,
        );
    }

    // -----------------------------------------------------------------
    // lift (computed → specified) の losslessness / 不動点性
    // -----------------------------------------------------------------

    /// `Px` は絶対化の不動点なので、lift → 絶対化の round trip は恒等。
    /// 基準 font-size を変えても結果が変わらないことを pin する。
    #[test]
    fn lift_font_size_is_fixed_point_under_absolutization() {
        let inherited = ComputedLength(24.0);
        let lifted = lift_font_size(inherited);
        assert_eq!(lifted, Length::Px(24.0));
        assert_eq!(
            resolve_font_size(lifted, ComputedLength(16.0), None, &CTX),
            inherited,
        );
        // 基準を変えても不変 (= 二重適用が起きない)。
        assert_eq!(
            resolve_font_size(
                lifted,
                ComputedLength(100.0),
                None,
                &ResolveContext::new(ComputedLength(100.0)),
            ),
            inherited,
        );
    }

    /// `line-height: 150%` は **宣言要素** で絶対化され、子はその length を
    /// 継承する (CSS Inline 3 §5.1)。子の font-size で **再 resolve しない** —
    /// この test は「percentage を再解決すべき」という誤修正を検出する
    /// regression pin。
    #[test]
    fn lift_line_height_length_does_not_re_resolve_percentage_in_child() {
        let declared = resolve_line_height(
            LineHeight::Length(Length::Percent(150.0)),
            ComputedLength(20.0),
            None,
            &CTX,
        );
        assert_eq!(declared, ComputedLineHeight::Length(ComputedLength(30.0)));

        let lifted = lift_line_height(declared);
        assert_eq!(lifted, LineHeight::Length(Length::Px(30.0)));

        // 子の font-size が 10px でも 15px にはならない。
        let child = resolve_line_height(lifted, ComputedLength(10.0), None, &CTX);
        assert_eq!(child, ComputedLineHeight::Length(ComputedLength(30.0)));
    }

    /// `<number>` は lift でも素通しし、子自身の font-size に掛かる余地を残す。
    #[test]
    fn lift_line_height_number_and_normal_pass_through() {
        assert_eq!(
            lift_line_height(ComputedLineHeight::Number(1.5)),
            LineHeight::Number(1.5),
        );
        assert_eq!(
            lift_line_height(ComputedLineHeight::Normal),
            LineHeight::Normal,
        );
        // Number は子の font-size で resolve されずに number のまま運ばれる。
        assert_eq!(
            resolve_line_height(
                lift_line_height(ComputedLineHeight::Number(1.5)),
                ComputedLength(10.0),
                None,
                &CTX
            ),
            ComputedLineHeight::Number(1.5),
        );
    }

    // -----------------------------------------------------------------
    // lift_length_percentage (text-indent inheritance seed)
    // -----------------------------------------------------------------

    #[test]
    fn lift_length_percentage_px_is_fixed_point_under_absolutization() {
        let lifted = lift_length_percentage(ComputedLengthPercentage::Px(40.0));
        assert_eq!(lifted, Length::Px(40.0));
        assert_eq!(
            resolve_length_percentage(lifted, ComputedLength(10.0), None, &CTX),
            ComputedLengthPercentage::Px(40.0),
        );
    }

    /// `%` は containing block 依存の used value 層まで再解決しない —
    /// lift → 絶対化の round trip でも `%` のまま運ばれることを pin。
    #[test]
    fn lift_length_percentage_percent_does_not_resolve_against_child_font_size() {
        let lifted = lift_length_percentage(ComputedLengthPercentage::Percent(10.0));
        assert_eq!(lifted, Length::Percent(10.0));
        assert_eq!(
            resolve_length_percentage(lifted, ComputedLength(10.0), None, &CTX),
            ComputedLengthPercentage::Percent(10.0),
        );
    }

    /// `resolve_flex_basis`'s `FlexBasisValue::Content` arm — pass-through
    /// keyword, no `<length-percentage>` machinery involved
    /// ([`resolve_flex_basis`] doc's "content はそのまま keyword として素通し"
    /// note).
    #[test]
    fn resolve_flex_basis_content_is_pass_through_keyword() {
        assert_eq!(
            resolve_flex_basis(FlexBasisValue::Content, ComputedLength(20.0), None, &CTX),
            ComputedFlexBasis::Content,
        );
    }

    /// `resolve_flex_basis`'s nested `Lh`/`Rlh`-unresolvable fallback
    /// (delegated to [`resolve_length_percentage_or_auto`], sibling to
    /// `border_width_resolves_lh_and_rlh` above) — `own_line_height: None`
    /// (`normal` で解決不能) makes `1lh` fall back to `Auto`, same as the
    /// `<'width'>` shape [`resolve_flex_basis`]'s doc says it reuses.
    #[test]
    fn resolve_flex_basis_falls_back_to_auto_when_lh_unresolvable() {
        assert_eq!(
            resolve_flex_basis(
                FlexBasisValue::Length(Length::Lh(2.0)),
                ComputedLength(20.0),
                None,
                &CTX
            ),
            ComputedFlexBasis::Auto,
        );
    }

    /// [`resolve_vertical_align`]'s 6 bare-keyword variants are pass-through
    /// — no length payload, nothing for phase 3 to absolutize (same shape
    /// as [`resolve_flex_basis`]'s `Content` arm above).
    #[test]
    fn resolve_vertical_align_keywords_are_pass_through() {
        for va in [
            VerticalAlign::Baseline,
            VerticalAlign::Sub,
            VerticalAlign::Super,
            VerticalAlign::Middle,
            VerticalAlign::TextTop,
            VerticalAlign::TextBottom,
        ] {
            assert_eq!(
                resolve_vertical_align(va, ComputedLength(20.0), None, &CTX),
                va,
            );
        }
    }

    /// `<length>` absolutizes against the declaring node's own `font-size`
    /// (`em`) — the same basis `letter-spacing`/`word-spacing` use via
    /// [`resolve_length_or_normal`].
    #[test]
    fn resolve_vertical_align_length_absolutizes_em() {
        assert_eq!(
            resolve_vertical_align(
                VerticalAlign::Length(Length::Em(2.0)),
                ComputedLength(10.0),
                None,
                &CTX
            ),
            VerticalAlign::Length(Length::Px(20.0)),
        );
    }

    /// §10.8.1 spec verbatim ("Raise (positive value) or lower (negative
    /// value)") — negative `<length>` absolutizes without a non-negative
    /// filter, same as `letter-spacing`/`margin-*`.
    #[test]
    fn resolve_vertical_align_length_preserves_negative_sign() {
        assert_eq!(
            resolve_vertical_align(
                VerticalAlign::Length(Length::Px(-6.0)),
                ComputedLength(10.0),
                None,
                &CTX
            ),
            VerticalAlign::Length(Length::Px(-6.0)),
        );
    }

    /// `own_line_height: None` (`normal` で解決不能) makes `1lh` fall back
    /// to `Px(0.0)` — same [`resolve_length`] "Finding B" fallback
    /// `resolve_flex_basis_falls_back_to_auto_when_lh_unresolvable` above
    /// pins, but landing on the *benign* side of that doc's distinction:
    /// `0px` shift here matches CSS 2.1 §10.8.1's own spec verbatim for
    /// `<length>` ("The value `0cm` means the same as `baseline`."), not an
    /// arbitrary "no better option" value (`resolve_length` doc's "Finding
    /// B" section).
    #[test]
    fn resolve_vertical_align_length_falls_back_to_zero_when_lh_unresolvable() {
        assert_eq!(
            resolve_vertical_align(
                VerticalAlign::Length(Length::Lh(2.0)),
                ComputedLength(20.0),
                None,
                &CTX
            ),
            VerticalAlign::Length(Length::Px(0.0)),
        );
    }

    // -----------------------------------------------------------------
    // specified initial → computed initial (per-function 粒度の drift 検出)
    // -----------------------------------------------------------------

    /// specified 層の initial value を各絶対化関数に個別に通した結果が、
    /// spec の computed initial (padding / margin = 0px、width / height = auto、
    /// border-width = 0px、line-height = normal、font-size = 16px) になることを
    /// pin する。
    ///
    /// 集約版 (`SpecifiedValues::finalize` 全体) は `crate::specified` の // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// `initial_specified_finalizes_to_initial_computed` が持つ。こちらは
    /// **どの関数が壊れたか**を局所化するための per-function 粒度。
    #[test]
    fn initial_length_fields_absolutize_to_spec_initials() {
        let initial = SpecifiedValues::initial();
        let fs = ComputedLength(INITIAL_FONT_SIZE_PX);

        assert_eq!(initial.padding, Sides::all(Length::Px(0.0)));
        assert_eq!(
            resolve_length_percentage(initial.padding.top, fs, None, &CTX),
            ComputedLengthPercentage::Px(0.0),
        );
        assert_eq!(initial.text_indent, Length::Px(0.0));
        assert_eq!(
            resolve_length_percentage(initial.text_indent, fs, None, &CTX),
            ComputedLengthPercentage::Px(0.0),
        );
        assert_eq!(
            resolve_length_percentage_or_auto(initial.margin.top, fs, None, &CTX),
            ComputedLengthPercentageOrAuto::Px(0.0),
        );
        assert_eq!(
            resolve_length_percentage_or_auto(initial.width, fs, None, &CTX),
            ComputedLengthPercentageOrAuto::Auto,
        );
        assert_eq!(
            resolve_length_percentage_or_auto(initial.height, fs, None, &CTX),
            ComputedLengthPercentageOrAuto::Auto,
        );
        // CSS Backgrounds 3 §3.3 "Computed value: … zero if the border style is
        // none or hidden" — initial style は `none` なので computed width は 0px
        // (specified の `medium` = 3px は style gating で潰れる)。
        assert_eq!(initial.border.left.width, Length::Px(3.0));
        assert_eq!(
            resolve_border(initial.border.left, fs, None, &CTX).width,
            ComputedLength::ZERO,
        );
        assert_eq!(
            resolve_line_height(initial.line_height, fs, None, &CTX),
            ComputedLineHeight::Normal,
        );
        assert_eq!(
            resolve_font_size(initial.font_size, fs, None, &CTX),
            ComputedLength(INITIAL_FONT_SIZE_PX),
        );
    }
}
