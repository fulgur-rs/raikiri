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
//! **computed 値も [`Length`] で運ばれる** (bd raikiri-spike-sshp)。
//! canonical な説明は [`Length`] の doc の「本型は『specified 層』を意味しない
//! — 層は出所で決まる」節、page 経路が保証する内容は
//! [`crate::page::PageCascadeResult::declarations`] の doc が canonical。
//! **本節は要約に留め、規則の中身をここに書き足さないこと** — 以前ここには
//! 「書き換えるときは必ずあちらと揃えること」と書いてあったが、その手運用は
//! 実際に 2 度 drift した (bd raikiri-spike-awjx)。現在は `page::tests` の
//! `page_declarations_carry_no_specified_layer_residue` (raikiri-spike-l3wg
//! 以前は `page_declarations_carry_exactly_one_specified_layer_residue`、
//! `text-align: match-parent` が唯一の specified 層残滓だった) が保証内容を
//! 機械的に pin している。
//!
//! bd decision raikiri-spike-082k (Option A) / bd task raikiri-spike-i5bs
//! (Phase 1 = additive)。
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
//! できない (適用順は property 間で保証されない)。これは decision 082k の
//! 拘束事項である。
//!
//! 本 module の関数群は、この制約を守るのに必要な材料を signature に持つ —
//! いずれも cascade の winner 集合に触らない純関数で、基準となる font-size を
//! **引数で受け取る**。ただし signature が保証するのは
//! **「本 module の関数自体が winner を適用しない」「基準が呼び出し側から明示的に
//! 供給される」の 2 点だけ**である。
//!
//! **順序は型で縛られていない。** 引数はただの [`ComputedLength`] なので、winner を
//! 1 つ適用するたびに本 module の関数を呼び、親の font-size や phase 2 前の中間値を
//! 基準として渡す誤実装は**普通に書ける** (型検査は通る)。すなわち decision 082k の
//! 拘束事項は本 module では**規約として**守るものであり、下の doctest がその規約
//! である。
//!
//! **cascade pipeline 側は規約に頼っていない** (bd raikiri-spike-zls8 の判断):
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
//! # 想定される 3 phase の呼び出し順序
//!
//! ```
//! use raikiri_style::{
//!     ComputedLength, ComputedLengthPercentage, ResolveContext, resolve_font_size,
//!     resolve_length_percentage,
//! };
//! use raikiri_style::property::Length;
//!
//! // 親の computed font-size (inheritance が運んできた computed value)。
//! let parent_font_size = ComputedLength(16.0);
//! let ctx = ResolveContext::new(ComputedLength(16.0));
//!
//! // phase 1: cascade winner を specified 表現のまま staging する (順不同)。
//! let specified_font_size = Length::Em(1.5);
//! let specified_padding_top = Length::Em(2.0);
//!
//! // phase 2: font-size を **親基準** で絶対化する。
//! let font_size = resolve_font_size(specified_font_size, parent_font_size, &ctx);
//! assert_eq!(font_size, ComputedLength(24.0));
//!
//! // phase 3: 残りを **自 node の確定済 font-size** 基準で絶対化する。
//! let padding_top = resolve_length_percentage(specified_padding_top, font_size, &ctx);
//! assert_eq!(padding_top, ComputedLengthPercentage::Px(48.0));
//! ```
//!
//! # `#[non_exhaustive]` の方針 — 本 module の computed 型群に限る判断
//!
//! **crate-wide の規則ではない。** specified 層の [`Length`] / [`LengthOrAuto`] /
//! [`LineHeight`] / [`Border`] を含む `property.rs` の公開 enum は sum 型でも
//! `#[non_exhaustive]` を付ける (37n sibling convention)。本 module の
//! computed 型群だけがそこから外れる — 理由は下記の **explicit trade** であって
//! 「sum 型だから」という形の性質ではない。
//!
//! - [`ComputedLengthPercentage`] / [`ComputedLengthPercentageOrAuto`] /
//!   [`ComputedLineHeight`] — **付けない** (下記 trade。下流に網羅 match を
//!   強制する)。
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
//! **calc()** の 3 形態を取る。したがって Epic 5 (css-variables-and-math) で
//! `Calc` variant は**確実に増える**。これは以下の **explicit trade** である。
//!
//! - **得るもの**: 今すぐ下流で網羅 match が書けること。`raikiri-dom` の
//!   `layout.rs` にある defensive な `_ => length(0.0)` を削除でき、
//!   fail-quiet の class が型検査で閉じる。
//! - **払うもの**: Epic 5 で `Calc` variant を追加する際、raikiri-style /
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

use crate::computed::INITIAL_FONT_SIZE_PX;
use crate::property::{Border, BorderColor, BorderStyle, Length, LengthOrAuto, LineHeight};

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
/// 同士の加算なので対象外 (bd raikiri-spike-i5bs NOTES 訂正 1)。
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
/// let p = resolve_length_percentage(Length::Percent(50.0), font_size, &ctx);
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
/// let lh = resolve_line_height(LineHeight::Length(Length::Percent(150.0)), font_size, &ctx);
/// assert_eq!(lh, ComputedLineHeight::Length(ComputedLength(30.0)));
///
/// // `<number>` は素通し (子が自分の font-size に掛ける)。
/// let n = resolve_line_height(LineHeight::Number(1.5), font_size, &ctx);
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
///     resolve_border(initial, ComputedLength(20.0), &ctx).width(),
///     ComputedLength::ZERO,
/// );
///
/// // style が visible なら specified width がそのまま絶対化される。
/// let mut specified = initial;
/// specified.style = BorderStyle::Solid;
/// let computed = resolve_border(specified, ComputedLength(20.0), &ctx);
/// assert_eq!(computed.width(), ComputedLength(3.0));
/// // style / color は specified keyword をそのまま運ぶ。
/// assert_eq!(computed.style(), specified.style);
/// assert_eq!(computed.color, specified.color);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedBorder {
    /// 絶対化済みの border width。
    ///
    /// `style` が [`BorderStyle::None`] / [`BorderStyle::Hidden`] のとき
    /// [`resolve_border`] は本 field を必ず `ComputedLength::ZERO` にする
    /// (上記 propdef の gating)。crate 外からの直接書き換えでこの対応関係を
    /// 崩せないよう `pub(crate)` に絞り、read-only accessor [`Self::width`]
    /// のみを公開する (bd raikiri-spike-9jmt)。
    pub(crate) width: ComputedLength,
    /// `border-*-style` — computed 層でも specified keyword。
    ///
    /// `width` と対で `pub(crate)` に絞り、read-only accessor
    /// [`Self::style`] のみを公開する (bd raikiri-spike-9jmt)。
    pub(crate) style: BorderStyle,
    /// `border-*-color` — `currentcolor` keyword を保持したまま computed 層に
    /// 残る (used-value 解決は paint 責務)。
    pub color: BorderColor,
}

impl ComputedBorder {
    /// 絶対化済みの border width への read-only accessor。
    ///
    /// [`Self::style`] が [`BorderStyle::None`] / [`BorderStyle::Hidden`] の
    /// ときは必ず `ComputedLength::ZERO` — [`resolve_border`] が gate する
    /// (bd raikiri-spike-9jmt)。
    pub fn width(&self) -> ComputedLength {
        self.width
    }

    /// `border-*-style` の computed value への read-only accessor
    /// (bd raikiri-spike-9jmt)。
    pub fn style(&self) -> BorderStyle {
        self.style
    }
}

// ---------------------------------------------------------------------------
// ResolveContext
// ---------------------------------------------------------------------------

/// 絶対化に必要な document-global の参照値。
///
/// 現状は `rem` の参照値 (root element の computed font-size) のみ。
///
/// # Primary source (§ title + anchor)
///
/// CSS Values 4 §6.1.1 "Font-relative Lengths"
/// (<https://www.w3.org/TR/css-values-4/#rem>): `rem` — "Equal to the computed
/// value of the em unit on the root element."
///
/// `#[non_exhaustive]` (module doc 参照) — struct 自体は future field を source
/// 互換で追加できる。下流からの struct literal 構築は
/// [`ResolveContext::new`] を使う。
///
/// **ただし `new` は positional なので `#[non_exhaustive]` の source 互換は
/// constructor まで及ばない** — viewport-relative unit (`vw` / `vh`、
/// bd raikiri-spike-2x8) の viewport size を足す時点で、`new` の signature 変更
/// (breaking) か第 2 constructor (`with_viewport()` 等) / builder のいずれかが
/// 強制される。field 追加を本当に非破壊にしたいなら後者を採ること。
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
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolveContext {
    /// root element の computed font-size。`rem` の参照値。
    ///
    /// 型は [`ComputedLength`] — 本 field が保持するのは**絶対化済の computed
    /// `<length>`** であり、[`resolve_font_size`] の戻り値をそのまま格納できる。
    pub root_font_size: ComputedLength,
}

impl ResolveContext {
    /// root element の computed font-size を指定して構築する。
    pub fn new(root_font_size: ComputedLength) -> Self {
        Self { root_font_size }
    }

    /// root element の computed font-size が未確定な段階で使う initial context。
    ///
    /// `root_font_size` は `font-size` の initial value (16px) —
    /// [`crate::computed::ComputedValues::initial`] の `font_size` と同一値。
    ///
    /// root element 自身の **`font-size: Nrem`** もこの値を参照する: CSS Values 4
    /// §6.1.1 "Font-relative Lengths"
    /// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) の
    /// "When used in the value of any font-* property on the element they refer
    /// to, the font-relative lengths resolve against the computed metrics of the
    /// parent element—or against the computed metrics corresponding to the
    /// initial values of the font and line-height properties, if the element has
    /// no parent." により、root element では initial value 基準になる。
    ///
    /// **root element の box property (`padding` 等) は対象外** — 上記条項は
    /// "any font-* property" に限定されており、`padding: 2rem` の `rem` は素の
    /// 定義どおり root element の computed font-size を参照する。すなわち root
    /// element でも phase 3 では本 context ではなく
    /// `ResolveContext::new(自 font-size)` を使う
    /// ([`SpecifiedValues::finalize_as_root`] が実装している)。
    ///
    /// [`SpecifiedValues::finalize_as_root`]: crate::specified::SpecifiedValues::finalize_as_root
    pub fn initial() -> Self {
        Self {
            root_font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
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
/// 式は `v * 4.0 / 3.0` の形 (乗算を先) で書く — `raikiri-dom` の `layout.rs` の
/// 既存 bridge (`computed_length_percentage_to_taffy_length_percentage` 他) と
/// **同一の評価順**にし、同じ authored value に対して両者が bit 単位で同じ f32
/// を返すことを保つため。
/// f32 は結合則を満たさないので `v * (4.0 / 3.0)` に「簡約」してはならない。
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

/// `font-size` の specified value を絶対化する (**phase 2** — 親基準)。
///
/// `parent_font_size` は**親要素の** computed font-size。親がない (root element)
/// 場合は `font-size` の initial value (16px、
/// [`ComputedLength`]`(16.0)`) を渡す。
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
///
/// `ex` / `rex` / `ch` / `rch` / `ic` / `ric` は style 層に実 font metrics が
/// 無いため常に spec の unknown-metric fallback を使う — 根拠は各 variant
/// ([`Length::Ex`] 等) の doc、`font-size` 自身が font-* property のため
/// **親** 基準になる理由は上記 parent-metrics 条項 (`em` と同じ扱い)。
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
/// ないので initial values 基準)。
///
/// cascade pipeline ではこの contract を
/// [`SpecifiedValues::finalize_as_root`] が守る (bd raikiri-spike-zls8) —
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
/// let child = resolve_font_size(Length::Em(1.5), ComputedLength(16.0), &ctx);
/// assert_eq!(child, ComputedLength(24.0));
/// let grandchild = resolve_font_size(Length::Em(1.5), child, &ctx);
/// assert_eq!(grandchild, ComputedLength(36.0));
/// ```
pub fn resolve_font_size(
    specified: Length,
    parent_font_size: ComputedLength,
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
        // ex / ch: unknown-metric fallback = 0.5em ([`Length::Ex`] /
        // [`Length::Ch`] doc)。font-size 自身の値なので基準は親
        // (上記 parent-metrics 条項、`em` と同じ)。
        Length::Ex(v) | Length::Ch(v) => ComputedLength(parent_font_size.0 * v * 0.5),
        // ic: unknown-metric fallback = 1em ([`Length::Ic`] doc)。
        Length::Ic(v) => ComputedLength(parent_font_size.0 * v),
        // rex / rch: root 版の同じ fallback、基準は root_font_size (`rem` と同じ)。
        Length::Rex(v) | Length::Rch(v) => ComputedLength(ctx.root_font_size.0 * v * 0.5),
        Length::Ric(v) => ComputedLength(ctx.root_font_size.0 * v),
        // CSS Fonts 4 `font-size` propdef: "Percentages: refer to parent
        // element's font size" — font-size は §5.5.1 の「percentage は
        // percentage のまま computed される」原則の明示的な例外。
        Length::Percent(p) => ComputedLength(parent_font_size.0 * p / 100.0),
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
/// 本関数の in-crate consumer は 2 つある — `border-*-width`
/// ([`resolve_border`]) と `line-height` の `<length>` 成分
/// ([`resolve_line_height`])。両者ともに `Length::Percent` を渡さない:
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
/// これは設計文書 §4.6 が Option A で削除するとした下流 (`raikiri-dom`
/// `layout.rs`) の catch-all とは別物である (あちらは bd raikiri-spike-zls8 で
/// 実際に削除済) — あちらは **computed 層**の型を
/// match して `Em` / `Rem` という **spec-valid な入力**を黙って 0px に潰す
/// (= fail-quiet)。本 arm は **specified 層の [`Length`]** に対するもので、
/// 潰れる入力が grammar 上存在しない。
pub(crate) fn resolve_length(
    specified: Length,
    font_size: ComputedLength,
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
        // ([`resolve_font_size`] の同 arm と同じ 0.5em / 1em、基準のみ自要素)。
        Length::Ex(v) | Length::Ch(v) => ComputedLength(font_size.0 * v * 0.5),
        Length::Ic(v) => ComputedLength(font_size.0 * v),
        Length::Rex(v) | Length::Rch(v) => ComputedLength(ctx.root_font_size.0 * v * 0.5),
        Length::Ric(v) => ComputedLength(ctx.root_font_size.0 * v),
        Length::Percent(_) => ComputedLength::ZERO,
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
pub fn resolve_length_percentage(
    specified: Length,
    font_size: ComputedLength,
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
        // ex / ch / ic: [`resolve_length`] と同じ fallback ratio。
        Length::Ex(v) | Length::Ch(v) => ComputedLengthPercentage::Px(font_size.0 * v * 0.5),
        Length::Ic(v) => ComputedLengthPercentage::Px(font_size.0 * v),
        Length::Rex(v) | Length::Rch(v) => {
            ComputedLengthPercentage::Px(ctx.root_font_size.0 * v * 0.5)
        }
        Length::Ric(v) => ComputedLengthPercentage::Px(ctx.root_font_size.0 * v),
        Length::Percent(p) => ComputedLengthPercentage::Percent(p),
    }
}

/// `<length-percentage> | auto` を取る property (`margin-*` / `width` /
/// `height`) の specified value を絶対化する (**phase 3** — 自 node 基準)。
///
/// `Auto` は computed 層でも keyword のまま。`Percent` の扱いは
/// [`resolve_length_percentage`] と同じ (素通し、used value 層で解決)。
pub fn resolve_length_percentage_or_auto(
    specified: LengthOrAuto,
    font_size: ComputedLength,
    ctx: &ResolveContext,
) -> ComputedLengthPercentageOrAuto {
    match specified {
        LengthOrAuto::Auto => ComputedLengthPercentageOrAuto::Auto,
        LengthOrAuto::Length(len) => match resolve_length_percentage(len, font_size, ctx) {
            ComputedLengthPercentage::Px(v) => ComputedLengthPercentageOrAuto::Px(v),
            ComputedLengthPercentage::Percent(p) => ComputedLengthPercentageOrAuto::Percent(p),
        },
    }
}

/// `line-height` の specified value を絶対化する (**phase 3** — 自 node 基準)。
///
/// - `normal` / `<number>` は素通し。`<number>` を computed 層に残すのは spec 上
///   load-bearing な distinction (子は number を inherit して**自分の**
///   font-size に掛ける)。
/// - `<percentage>` は **自要素の** computed font-size に対して絶対化する —
///   CSS Inline 3 §5.1 (<https://www.w3.org/TR/css-inline-3/#propdef-line-height>)
///   "Percentages: computed relative to 1em" + CSS Values 4 §6.1.1 `em`
///   (<https://www.w3.org/TR/css-values-4/#em>) "Equal to the computed value of
///   the font-size property of the element on which it is used."
/// - `<length>` は `resolve_length` と同じ規則で絶対化する。
pub fn resolve_line_height(
    specified: LineHeight,
    font_size: ComputedLength,
    ctx: &ResolveContext,
) -> ComputedLineHeight {
    match specified {
        LineHeight::Normal => ComputedLineHeight::Normal,
        LineHeight::Number(n) => ComputedLineHeight::Number(n),
        LineHeight::Length(len) => ComputedLineHeight::Length(match len {
            // CSS Inline 3 §5.1 "Percentages: computed relative to 1em" —
            // percentage は宣言要素の computed font-size で絶対化される
            // (`resolve_length` の grammar-unreachable な 0px arm には
            // 落とさない)。
            Length::Percent(p) => ComputedLength(font_size.0 * p / 100.0),
            other => resolve_length(other, font_size, ctx),
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
/// **computed 層**の要求である。
///
/// **spec tension (silently 解決しない)**: 同 §3.3 の非規範 Note は "Although the
/// initial width is medium, the initial style is none; therefore the used initial
/// width is 0." と **used** 層で述べる一方、規範な propdef table は **computed**
/// 層を指定している。Note は非規範なので propdef table が governs。
///
/// **本関数は gate の単一 source である (element 経路 / page 経路の両方)** —
/// `raikiri-dom` の `layout.rs` は Sprint 18 まで同じ gating を used 層
/// (`used_border_width` helper) で 1 層遅れて行っていたが、bd raikiri-spike-zls8
/// が `layout.rs` を [`ComputedBorder`] consumer に migrate した際に削除した。
/// 下流に同じ判定を再実装してはならない (spec 規則の二重実装は片方だけ直す
/// drift を生む)。
///
/// page 経路 (`@page`) は `PropertyValue` の bag を運ぶが、bd raikiri-spike-sshp
/// 以降 [`crate::page::cascade_page`] の phase 3 が `border-*-width` longhand を
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
/// 例外を作らない)。`border-image-*` は Epic 未着手
/// (`ComputedValues::border` doc の Non-goals) なので現状 gate 位置の再検討は
/// 不要だが、着手時には両 section を読み直すこと。
pub fn resolve_border(
    specified: Border,
    font_size: ComputedLength,
    ctx: &ResolveContext,
) -> ComputedBorder {
    // `matches!` + else 枝: 未知の future `BorderStyle` variant は「visible な
    // style」側に落として specified width を透過させる (spec 上 visible な style
    // が追加されたときに width が黙って 0 にならないよう fail-safe に倒す)。
    // (下流の `layout.rs` は本 gate の結果を受け取るだけで再判定しない。)
    let width = if matches!(specified.style, BorderStyle::None | BorderStyle::Hidden) {
        ComputedLength::ZERO
    } else {
        resolve_length(specified.width, font_size, ctx)
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
/// assert_eq!(resolve_font_size(lifted, ComputedLength(16.0), &ctx), inherited);
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
///     &ctx,
/// );
/// assert_eq!(declared, ComputedLineHeight::Length(ComputedLength(30.0)));
///
/// // 子 (font-size 10px) は 30px を **そのまま** 継承する (15px ではない)。
/// let child = resolve_line_height(lift_line_height(declared), ComputedLength(10.0), &ctx);
/// assert_eq!(child, ComputedLineHeight::Length(ComputedLength(30.0)));
/// ```
pub fn lift_line_height(computed: ComputedLineHeight) -> LineHeight {
    match computed {
        ComputedLineHeight::Normal => LineHeight::Normal,
        ComputedLineHeight::Number(n) => LineHeight::Number(n),
        ComputedLineHeight::Length(l) => LineHeight::Length(Length::Px(l.0)),
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
            resolve_font_size(Length::Px(18.0), ComputedLength(16.0), &CTX),
            ComputedLength(18.0),
        );
    }

    /// `1pt = 1/72in`、`1in = 96px` → `12pt = 16px`
    /// (CSS Values 4 §6.2 <https://www.w3.org/TR/css-values-4/#absolute-lengths>)。
    #[test]
    fn font_size_pt_converts_at_96px_per_inch() {
        assert_eq!(
            resolve_font_size(Length::Pt(12.0), ComputedLength(16.0), &CTX),
            ComputedLength(16.0),
        );
    }

    /// `font-size` の `em` は **親** の computed font-size 基準
    /// (CSS Values 4 §6.1.1 parent-metrics 条項)。
    #[test]
    fn font_size_em_resolves_against_parent_font_size() {
        assert_eq!(
            resolve_font_size(Length::Em(1.5), ComputedLength(16.0), &CTX),
            ComputedLength(24.0),
        );
    }

    /// `em` の compounding: 16px → 1.5em → 1.5em = 24px → 36px
    /// (decision raikiri-spike-082k Rationale 1 (i))。
    #[test]
    fn font_size_em_compounds_across_two_levels() {
        let child = resolve_font_size(Length::Em(1.5), ComputedLength(16.0), &CTX);
        let grandchild = resolve_font_size(Length::Em(1.5), child, &CTX);
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
            resolve_font_size(Length::Rem(2.0), ComputedLength(64.0), &ctx),
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
            resolve_font_size(Length::Rem(2.0), ComputedLength(INITIAL_FONT_SIZE_PX), &ctx),
            ComputedLength(32.0),
        );
    }

    /// `font-size` の `<percentage>` は親の font-size 基準で **length になる**
    /// (CSS Fonts 4 `font-size` propdef "Percentages: refer to parent element's
    /// font size" — CSS Values 4 §5.5.1 の明示的例外)。
    #[test]
    fn font_size_percent_resolves_against_parent_font_size() {
        assert_eq!(
            resolve_font_size(Length::Percent(150.0), ComputedLength(16.0), &CTX),
            ComputedLength(24.0),
        );
    }

    /// `ex` / `ch` は style 層に real font metrics が無いため常に spec の
    /// unknown-metric fallback (`0.5em`) を使う ([`Length::Ex`] / [`Length::Ch`]
    /// doc)。`font-size` 上では他 font-relative unit と同じく **親** 基準
    /// (self-reference avoidance、bd raikiri-spike-2x8)。
    #[test]
    fn font_size_ex_and_ch_resolve_against_parent_font_size_with_half_em_fallback() {
        assert_eq!(
            resolve_font_size(Length::Ex(2.0), ComputedLength(16.0), &CTX),
            ComputedLength(16.0), // 2 * 0.5 * 16
        );
        assert_eq!(
            resolve_font_size(Length::Ch(2.0), ComputedLength(16.0), &CTX),
            ComputedLength(16.0),
        );
    }

    /// `ic` の unknown-metric fallback は `1em` ([`Length::Ic`] doc)。
    #[test]
    fn font_size_ic_resolves_against_parent_font_size_with_one_em_fallback() {
        assert_eq!(
            resolve_font_size(Length::Ic(1.5), ComputedLength(16.0), &CTX),
            ComputedLength(24.0),
        );
    }

    /// `rex` / `rch` / `ric` は root element 基準 (`rem` と同じ、親の font-size
    /// には依存しない)。
    #[test]
    fn font_size_r_prefixed_font_relative_units_resolve_against_root_font_size() {
        let ctx = ResolveContext::new(ComputedLength(20.0));
        assert_eq!(
            resolve_font_size(Length::Rex(2.0), ComputedLength(64.0), &ctx),
            ComputedLength(20.0), // 2 * 0.5 * 20 (親 64px は無視)
        );
        assert_eq!(
            resolve_font_size(Length::Rch(2.0), ComputedLength(64.0), &ctx),
            ComputedLength(20.0),
        );
        assert_eq!(
            resolve_font_size(Length::Ric(2.0), ComputedLength(64.0), &ctx),
            ComputedLength(40.0),
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
            resolve_font_size(Length::In(1.0), ComputedLength(16.0), &CTX),
            ComputedLength(96.0),
        );
        assert_eq!(
            resolve_font_size(Length::Cm(1.0), ComputedLength(16.0), &CTX),
            ComputedLength(96.0 / 2.54),
        );
        assert_eq!(
            resolve_font_size(Length::Mm(1.0), ComputedLength(16.0), &CTX),
            ComputedLength(96.0 / 2.54 / 10.0),
        );
        assert_eq!(
            resolve_font_size(Length::Q(1.0), ComputedLength(16.0), &CTX),
            ComputedLength(96.0 / 2.54 / 40.0),
        );
        assert_eq!(
            resolve_font_size(Length::Pc(1.0), ComputedLength(16.0), &CTX),
            ComputedLength(96.0 / 6.0),
        );
    }

    // -----------------------------------------------------------------
    // phase 3: `<length>` (border-width) の絶対化 (自 node 基準)
    // -----------------------------------------------------------------

    #[test]
    fn length_px_and_pt_are_absolute() {
        assert_eq!(
            resolve_length(Length::Px(3.0), ComputedLength(16.0), &CTX),
            ComputedLength(3.0),
        );
        assert_eq!(
            resolve_length(Length::Pt(9.0), ComputedLength(16.0), &CTX),
            ComputedLength(12.0),
        );
    }

    /// `font-size` 以外の property の `em` は **自要素** の computed font-size
    /// 基準 (CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#em>)。
    #[test]
    fn length_em_resolves_against_own_font_size() {
        assert_eq!(
            resolve_length(Length::Em(2.0), ComputedLength(20.0), &CTX),
            ComputedLength(40.0),
        );
    }

    #[test]
    fn length_rem_resolves_against_root_font_size() {
        let ctx = ResolveContext::new(ComputedLength(10.0));
        assert_eq!(
            resolve_length(Length::Rem(2.5), ComputedLength(64.0), &ctx),
            ComputedLength(25.0),
        );
    }

    /// `border-*-width` の grammar (`<line-width>`) は `<percentage>` を含まない
    /// ため parse 段で drop される。到達不能 arm の全域性のみを pin する。
    #[test]
    fn length_percent_is_grammar_unreachable_and_falls_to_zero() {
        assert_eq!(
            resolve_length(Length::Percent(50.0), ComputedLength(20.0), &CTX),
            ComputedLength::ZERO,
        );
    }

    /// `font-size` 以外 (= `resolve_length` の呼び出し先である `border-*-width`
    /// や `line-height` の `<length>` 成分) では `ex` / `ch` / `ic` は
    /// **自要素** の computed font-size 基準になる — `resolve_font_size` の
    /// 同 unit テスト (親基準) との非対称を pin する
    /// ([`Length::Ex`] doc の parent-metrics 条項)。
    #[test]
    fn length_ex_ch_ic_resolve_against_own_font_size() {
        assert_eq!(
            resolve_length(Length::Ex(2.0), ComputedLength(20.0), &CTX),
            ComputedLength(20.0), // 2 * 0.5 * 20
        );
        assert_eq!(
            resolve_length(Length::Ch(2.0), ComputedLength(20.0), &CTX),
            ComputedLength(20.0),
        );
        assert_eq!(
            resolve_length(Length::Ic(2.0), ComputedLength(20.0), &CTX),
            ComputedLength(40.0),
        );
    }

    #[test]
    fn length_r_prefixed_font_relative_units_resolve_against_root_font_size() {
        let ctx = ResolveContext::new(ComputedLength(10.0));
        assert_eq!(
            resolve_length(Length::Rex(2.5), ComputedLength(64.0), &ctx),
            ComputedLength(12.5), // 2.5 * 0.5 * 10 (自 font-size 64px は無視)
        );
        assert_eq!(
            resolve_length(Length::Ric(2.5), ComputedLength(64.0), &ctx),
            ComputedLength(25.0),
        );
    }

    /// CSS Values 4 §6.2 換算表 — `border-*-width` 経由 (`resolve_length`) でも
    /// `resolve_font_size` と同じ変換になることを pin
    /// (`width: 1in` → 96px、issue 本文の verification 対象)。
    #[test]
    fn length_additional_absolute_units_convert_per_spec_table() {
        assert_eq!(
            resolve_length(Length::In(1.0), ComputedLength(20.0), &CTX),
            ComputedLength(96.0),
        );
        assert_eq!(
            resolve_length(Length::Pc(1.0), ComputedLength(20.0), &CTX),
            ComputedLength(96.0 / 6.0),
        );
        assert_eq!(
            resolve_length(Length::Mm(1.0), ComputedLength(20.0), &CTX),
            ComputedLength(96.0 / 2.54 / 10.0),
        );
        assert_eq!(
            resolve_length(Length::Q(1.0), ComputedLength(20.0), &CTX),
            ComputedLength(96.0 / 2.54 / 40.0),
        );
        assert_eq!(
            resolve_length(Length::Cm(1.0), ComputedLength(20.0), &CTX),
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
            resolve_length_percentage(Length::Px(10.0), fs, &CTX),
            ComputedLengthPercentage::Px(10.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Pt(6.0), fs, &CTX),
            ComputedLengthPercentage::Px(8.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Em(2.0), fs, &CTX),
            ComputedLengthPercentage::Px(40.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Rem(0.5), fs, &CTX),
            ComputedLengthPercentage::Px(8.0),
        );
    }

    /// [`resolve_length_percentage`] (`padding-*` の絶対化関数) 側でも
    /// bd raikiri-spike-2x8 で追加した全 unit を直接 exercise する
    /// (`resolve_font_size` / `resolve_length` の同 unit test とは別 site —
    /// 3 関数それぞれが独立した match を持つため、patch coverage は
    /// 関数単位で見る)。
    #[test]
    fn length_percentage_absolutizes_additional_units() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_length_percentage(Length::Ex(2.0), fs, &CTX),
            ComputedLengthPercentage::Px(20.0), // 2 * 0.5 * 20
        );
        assert_eq!(
            resolve_length_percentage(Length::Ch(2.0), fs, &CTX),
            ComputedLengthPercentage::Px(20.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Ic(1.5), fs, &CTX),
            ComputedLengthPercentage::Px(30.0),
        );
        let ctx = ResolveContext::new(ComputedLength(10.0));
        assert_eq!(
            resolve_length_percentage(Length::Rex(2.0), fs, &ctx),
            ComputedLengthPercentage::Px(10.0), // 2 * 0.5 * 10 (自 20px は無視)
        );
        assert_eq!(
            resolve_length_percentage(Length::Rch(2.0), fs, &ctx),
            ComputedLengthPercentage::Px(10.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Ric(2.0), fs, &ctx),
            ComputedLengthPercentage::Px(20.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Cm(1.0), fs, &CTX),
            ComputedLengthPercentage::Px(96.0 / 2.54),
        );
        assert_eq!(
            resolve_length_percentage(Length::Mm(1.0), fs, &CTX),
            ComputedLengthPercentage::Px(96.0 / 2.54 / 10.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Q(1.0), fs, &CTX),
            ComputedLengthPercentage::Px(96.0 / 2.54 / 40.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::In(1.0), fs, &CTX),
            ComputedLengthPercentage::Px(96.0),
        );
        assert_eq!(
            resolve_length_percentage(Length::Pc(1.0), fs, &CTX),
            ComputedLengthPercentage::Px(96.0 / 6.0),
        );
    }

    /// `<length-percentage>` の percentage は computed 層に **percentage のまま**
    /// 残る (CSS Values 4 §5.5.1 / CSS Box 3 `padding-top` "Computed value: a
    /// computed `<length-percentage>` value")。authored 数値をそのまま保持し
    /// `/ 100` もしない。
    #[test]
    fn length_percentage_percent_passes_through_unchanged() {
        assert_eq!(
            resolve_length_percentage(Length::Percent(50.0), ComputedLength(20.0), &CTX),
            ComputedLengthPercentage::Percent(50.0),
        );
    }

    // -----------------------------------------------------------------
    // phase 3: `<length-percentage> | auto` (margin / width / height)
    // -----------------------------------------------------------------

    #[test]
    fn length_percentage_or_auto_keeps_auto() {
        assert_eq!(
            resolve_length_percentage_or_auto(LengthOrAuto::Auto, ComputedLength(16.0), &CTX),
            ComputedLengthPercentageOrAuto::Auto,
        );
    }

    #[test]
    fn length_percentage_or_auto_absolutizes_and_passes_percent() {
        let fs = ComputedLength(20.0);
        assert_eq!(
            resolve_length_percentage_or_auto(LengthOrAuto::Length(Length::Em(1.5)), fs, &CTX),
            ComputedLengthPercentageOrAuto::Px(30.0),
        );
        assert_eq!(
            resolve_length_percentage_or_auto(
                LengthOrAuto::Length(Length::Percent(25.0)),
                fs,
                &CTX
            ),
            ComputedLengthPercentageOrAuto::Percent(25.0),
        );
    }

    // -----------------------------------------------------------------
    // phase 3: line-height
    // -----------------------------------------------------------------

    #[test]
    fn line_height_normal_passes_through() {
        assert_eq!(
            resolve_line_height(LineHeight::Normal, ComputedLength(20.0), &CTX),
            ComputedLineHeight::Normal,
        );
    }

    /// `<number>` は computed 層でも number のまま (CSS Inline 3 §5.1
    /// "Computed value: the specified keyword, a number, or a computed
    /// `<length>` value")。
    #[test]
    fn line_height_number_passes_through() {
        assert_eq!(
            resolve_line_height(LineHeight::Number(1.5), ComputedLength(20.0), &CTX),
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
                &CTX
            ),
            ComputedLineHeight::Length(ComputedLength(24.0)),
        );
        assert_eq!(
            resolve_line_height(
                LineHeight::Length(Length::Px(24.0)),
                ComputedLength(20.0),
                &CTX
            ),
            ComputedLineHeight::Length(ComputedLength(24.0)),
        );
    }

    // -----------------------------------------------------------------
    // phase 3: border
    // -----------------------------------------------------------------

    #[test]
    fn border_absolutizes_width_and_carries_style_and_color() {
        let mut specified = SpecifiedValues::initial().border.top;
        specified.style = BorderStyle::Solid;
        let computed = resolve_border(specified, ComputedLength(20.0), &CTX);
        // specified `medium` = 3px (CSS Backgrounds 3 §3.3: thin/medium/thick は
        // 1px/3px/5px に**規範的に**等価)。
        assert_eq!(computed.width, ComputedLength(3.0));
        assert_eq!(computed.style, specified.style);
        assert_eq!(computed.color, specified.color);
    }

    /// `ComputedBorder::width()` / `::style()` accessor 本体を実行する pin
    /// (bd raikiri-spike-9jmt)。上の test は同一モジュール内なので
    /// `pub(crate)` field に直接アクセスし、accessor 関数本体そのものは
    /// 経由しない。crate 外視点から accessor を叩く doctest ([`ComputedBorder`]
    /// 型 doc 内) はあるが、この repo の toolchain (stable 固定、
    /// `cargo llvm-cov` に `--doctests` 未指定) では doctest はカバレッジ計測
    /// 対象に入らないため、本 test が accessor 本体を計測対象として実行する。
    #[test]
    fn computed_border_accessors_read_the_gated_fields() {
        let mut specified = SpecifiedValues::initial().border.top;
        specified.style = BorderStyle::Solid;
        let computed = resolve_border(specified, ComputedLength(20.0), &CTX);
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
                resolve_border(b, ComputedLength(20.0), &CTX).width,
                ComputedLength::ZERO,
            );
        }
        b.style = BorderStyle::Solid;
        assert_eq!(
            resolve_border(b, ComputedLength(20.0), &CTX).width,
            ComputedLength(5.0),
        );
    }

    #[test]
    fn border_em_width_resolves_against_own_font_size() {
        let mut specified = SpecifiedValues::initial().border.top;
        specified.width = Length::Em(0.5);
        specified.style = BorderStyle::Solid;
        let computed = resolve_border(specified, ComputedLength(20.0), &CTX);
        assert_eq!(computed.width, ComputedLength(10.0));
    }

    /// bd raikiri-spike-2x8 で追加した absolute unit (`pc`) も
    /// `border-*-width` の style gating (この module doc / [`resolve_border`]
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
            resolve_border(specified, ComputedLength(20.0), &CTX).width,
            ComputedLength(16.0),
        );
        specified.style = BorderStyle::None;
        assert_eq!(
            resolve_border(specified, ComputedLength(20.0), &CTX).width,
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
            resolve_font_size(lifted, ComputedLength(16.0), &CTX),
            inherited,
        );
        // 基準を変えても不変 (= 二重適用が起きない)。
        assert_eq!(
            resolve_font_size(
                lifted,
                ComputedLength(100.0),
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
            &CTX,
        );
        assert_eq!(declared, ComputedLineHeight::Length(ComputedLength(30.0)));

        let lifted = lift_line_height(declared);
        assert_eq!(lifted, LineHeight::Length(Length::Px(30.0)));

        // 子の font-size が 10px でも 15px にはならない。
        let child = resolve_line_height(lifted, ComputedLength(10.0), &CTX);
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
                &CTX
            ),
            ComputedLineHeight::Number(1.5),
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
    /// 集約版 (`SpecifiedValues::finalize` 全体) は `crate::specified` の
    /// `initial_specified_finalizes_to_initial_computed` が持つ。こちらは
    /// **どの関数が壊れたか**を局所化するための per-function 粒度。
    #[test]
    fn initial_length_fields_absolutize_to_spec_initials() {
        let initial = SpecifiedValues::initial();
        let fs = ComputedLength(INITIAL_FONT_SIZE_PX);

        assert_eq!(initial.padding, Sides::all(Length::Px(0.0)));
        assert_eq!(
            resolve_length_percentage(initial.padding.top, fs, &CTX),
            ComputedLengthPercentage::Px(0.0),
        );
        assert_eq!(
            resolve_length_percentage_or_auto(initial.margin.top, fs, &CTX),
            ComputedLengthPercentageOrAuto::Px(0.0),
        );
        assert_eq!(
            resolve_length_percentage_or_auto(initial.width, fs, &CTX),
            ComputedLengthPercentageOrAuto::Auto,
        );
        assert_eq!(
            resolve_length_percentage_or_auto(initial.height, fs, &CTX),
            ComputedLengthPercentageOrAuto::Auto,
        );
        // CSS Backgrounds 3 §3.3 "Computed value: … zero if the border style is
        // none or hidden" — initial style は `none` なので computed width は 0px
        // (specified の `medium` = 3px は style gating で潰れる)。
        assert_eq!(initial.border.left.width, Length::Px(3.0));
        assert_eq!(
            resolve_border(initial.border.left, fs, &CTX).width,
            ComputedLength::ZERO,
        );
        assert_eq!(
            resolve_line_height(initial.line_height, fs, &CTX),
            ComputedLineHeight::Normal,
        );
        assert_eq!(
            resolve_font_size(initial.font_size, fs, &CTX),
            ComputedLength(INITIAL_FONT_SIZE_PX),
        );
    }
}
