//! CSS property value 型と per-property parser。
//!
//! M1.4 では color / font-family / font-size / font-weight の 4 property のみ。
//! 認識できない property name / invalid value は `parse_value` が `None` を返す
//! (spec 準拠の silent drop、caller である rule.rs で declaration ごと drop)。
//!
//! `parse_value` は rule.rs の `DeclParser::parse_value` から呼ばれる。

use std::sync::{Arc, OnceLock};

use cssparser::color::{clamp_unit_f32, parse_hash_color, parse_named_color};
use cssparser::{ParseError, Parser, Token};
use smol_str::SmolStr;

use crate::Atom;

/// 空 `<content-list>` を表す shared Arc — cascade で全 node が持ちうる
/// initial / inherit_from の default 値を per-node 新規 allocate せず、
/// 単一 heap slot を bump-share するための helper。
///
/// `raikiri-spike-d9y.1` (SEC HIGH cascade memory DoS fix) の副作用として
/// `ComputedValues.content` / `.string_set` は `Arc<Vec<..>>` に wrap したが、
/// `Arc::new(Vec::new())` を every node で呼ぶと N-node document あたり
/// 2N の small heap allocation regression になる (advisor calibration)。
/// `OnceLock` で **process 全体で 1 個** の empty Arc を保持し、
/// [`empty_content_list`] / [`empty_string_set_entries`] が各 initial spot で
/// clone (Arc bump only) する。
///
/// 空 `Vec::new()` は allocation 0 だが `Vec` struct 自体の 24 bytes が per-node
/// に生まれる — Arc 化により 8-byte pointer に置き換わり、指す先は shared。
pub(crate) fn empty_content_list() -> Arc<Vec<ContentComponent>> {
    static EMPTY: OnceLock<Arc<Vec<ContentComponent>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// `string-set` entry の 1 要素 — `(<custom-ident> name, content-list)` pair
/// を owned Vec で保持。alias 化により clippy::type_complexity を satisfy し、
/// 下段の `Arc<Vec<StringSetEntry>>` shape を局所化する。
pub(crate) type StringSetEntry = (SmolStr, Vec<ContentComponent>);

/// 空 `string-set` entries を表す shared Arc — [`empty_content_list`] と同じ
/// pattern (per-node empty allocation regression 回避、d9y.1)。
pub(crate) fn empty_string_set_entries() -> Arc<Vec<StringSetEntry>> {
    static EMPTY: OnceLock<Arc<Vec<StringSetEntry>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// 空 `counter-*` entries を表す shared Arc — 3 property
/// (`counter-reset` / `counter-increment` / `counter-set`) 全てで単一 slot を
/// 共有する ([`Vec<(SmolStr, i32)>`] は同一型のため helper を分ける必要無し)。
///
/// `raikiri-spike-d9y.2` (SEC HIGH cascade memory DoS fix) の副作用 helper。
/// counter-* は non-inherited (CSS Lists 3 §3、`counter-reset` を含む全 3 property)
/// のため、`ComputedValues::inherit_from` が child stack entry のたびに empty 値で
/// 初期化する。生 `Vec::new()` を使うと per-node で 3 個の `Vec` struct
/// (24 bytes × 3) が生まれ N-node document あたり O(N) の overhead になるため、
/// [`empty_content_list`] / [`empty_string_set_entries`] と同じ `OnceLock` 保持の
/// shared Arc を使う (advisor calibration precedent)。
pub(crate) fn empty_counter_entries() -> Arc<Vec<(SmolStr, i32)>> {
    static EMPTY: OnceLock<Arc<Vec<(SmolStr, i32)>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// RGBA color (0-255 per channel、`a` は 255 = fully opaque)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CssColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl CssColor {
    /// Opaque black — `<color>` initial value に相当。
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
}

/// CSS length or length-percentage value (Author CSS seed for m4+ box model).
///
/// 各 variant は authored value (raw number as written) を保持し、resolve は
/// 下流責務 (font-size context / containing block % / DPI 変換)。sibling arm
/// convention (37n): [`Length::Px`] が `Px(16.0)` = `16px` の pattern を確立、
/// 他 variant も authored value をそのまま保持する (`Em(1.2)` = `1.2em`、
/// `Percent(50.0)` = `50%` の literal 数字を格納)。
///
/// Downstream match は必ず wildcard arm を持つこと (`#[non_exhaustive]` 属性、
/// 変数追加が既存 pattern-match を break しない forward-compat 契約)。既存 sibling
/// site: `crates/raikiri-dom/src/layout.rs:143` `preshape_text` が M1.4 時点から
/// `_ => Err(LayoutError::Internal { ... })` の defensive wildcard を持つ。
///
/// # Primary sources (§ title + anchor)
///
/// - CSS Values 4 §6.1.1 "Font-relative Lengths":
///   [`em`](https://www.w3.org/TR/css-values-4/#em) —
///   "Equal to the computed value of the font-size property of the element on
///   which it is used." /
///   [`rem`](https://www.w3.org/TR/css-values-4/#rem) —
///   "Equal to the computed value of the em unit on the root element."
/// - CSS Values 4 §5.5 "Percentages":
///   [`<percentage>`](https://www.w3.org/TR/css-values-4/#percentages) —
///   "Percentage values are denoted by &lt;percentage&gt;, and indicates a value
///   that is some fraction of another reference value."
/// - CSS Values 4 §6.2 "Absolute Lengths":
///   [`pt`](https://www.w3.org/TR/css-values-4/#absolute-lengths) —
///   `1pt = 1/72 in`, CSS で `1in = 96px` の pixel-relative absolute unit。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// Absolute pixel length。`10px` → `Px(10.0)`。
    Px(f32),
    /// Font-relative length: `em` — 使用要素の computed `font-size` に対する倍率。
    /// `1.2em` → `Em(1.2)`。Resolve は下流 (font-size stack を辿る)。
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#em>).
    Em(f32),
    /// Font-relative length: `rem` — root element の computed `font-size` に対する倍率。
    /// `1rem` → `Rem(1.0)`。
    ///
    /// Spec: CSS Values 4 §6.1.1 Font-relative Lengths
    /// (<https://www.w3.org/TR/css-values-4/#rem>).
    Rem(f32),
    /// Percentage — `<length-percentage>` 文脈で reference value に対する比率。
    /// `50%` → `Percent(50.0)` (authored number をそのまま格納、divide-by-100 なし)。
    ///
    /// Spec: CSS Values 4 §5.5 Percentages
    /// (<https://www.w3.org/TR/css-values-4/#percentages>).
    Percent(f32),
    /// Absolute length: `pt` — 1pt = 1/72 in, CSS で 1in = 96px。
    /// `12pt` → `Pt(12.0)`、resolve 時 `12 * 96 / 72 = 16px` 相当。
    ///
    /// Spec: CSS Values 4 §6.2 Absolute Lengths
    /// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>).
    Pt(f32),
}

/// `<counter-style>` の parse 結果。
///
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>
/// で `counter()` / `counters()` の optional 第 3 引数、CSS Content 3 §2.6
/// で `target-counter()` / `target-counters()` の optional 末尾引数として現れる。
/// spec default = `decimal` (`counter-style?` omitted 時)。
///
/// M5 static-side scope では named style を SmolStr で pass-through する
/// (`decimal-leading-zero`, `upper-alpha`, `lower-roman` 等の解釈は下流責務、
/// runtime resolve で counter tree を format する際に効く)。
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CounterStyle {
    /// `decimal` — spec default (`<counter-style>?` omitted も同一 variant)。
    #[default]
    Decimal,
    /// `decimal` 以外の named counter-style。値は case-preserved の smol str。
    Named(SmolStr),
}

/// `string()` の第 2 引数 `[ first | start | last | first-except ]?`。
///
/// CSS Content 3 §2.7.2 "Inserting Named Strings: the string() function"
/// <https://www.w3.org/TR/css-content-3/#string-function>。
/// spec default = `first` (per §2.7.2 "if the second argument is omitted").
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StringFetchMode {
    /// `first` — spec default。
    #[default]
    First,
    /// `start`。
    Start,
    /// `last`。
    Last,
    /// `first-except`。
    FirstExcept,
}

/// `target-text()` の第 2 引数 `[ content | before | after | first-letter ]?`。
///
/// CSS Content 3 §2.6.3 "The target-text() function"
/// <https://www.w3.org/TR/css-content-3/#target-text>。
/// spec default = `content` (per §2.6.3 "The default value is `content`").
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentPart {
    /// `content` — spec default (element の string value)。
    #[default]
    Content,
    /// `before` — `::before` pseudo-element の string value。
    Before,
    /// `after` — `::after` pseudo-element の string value。
    After,
    /// `first-letter` — `::first-letter` pseudo-element の string。
    FirstLetter,
}

/// `content()` function の引数 `[ text | before | after | first-letter ]?`。
///
/// CSS GCPM 3 §1.1.1.1 "The content() function"
/// <https://www.w3.org/TR/css-gcpm-3/#content-list> の verbatim production:
/// `content() = content([text | before | after | first-letter])`。
/// spec default = `text` (per §1.1.1.1 の `text` dt/dd: "This is the default
/// value"、および `h2 { string-set: heading content() }` の bare 例)。
///
/// NB: sibling [`ContentPart`] (target-text() 用) と keyword 集合が重なるが、
/// `text` vs `content` の spec spelling divergence があるため型を分ける
/// (StringFetchMode / ContentPart と同じ per-function 専用 enum 慣行、
/// reviewer:spec: `content(content)` を silently accept してはならない)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentTextKeyword {
    /// `text` — spec default (element の string value、`white-space: normal`
    /// 相当で決定)。
    #[default]
    Text,
    /// `before` — `::before` pseudo-element の string value。
    Before,
    /// `after` — `::after` pseudo-element の string value。
    After,
    /// `first-letter` — `::first-letter` pseudo-element の string。
    FirstLetter,
}

/// [`parse_content_list_items`] の list vocabulary mode selector。
///
/// CSS Content 3 §2 <https://www.w3.org/TR/css-content-3/#content-list3> と
/// CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#content-list> は同名
/// `<content-list>` production を持つが、後者は前者の narrower な local 再定義
/// (GCPM 3 は `Link defaults` に CSS Content 3 を含めず、§1.1.1 L82 で自前に
/// `<content-list> = [ <string> | <counter()> | <counters()> | <content()> |
/// <attr()> ]+` を dfn する)。property ごとに受理される function 集合が違うため、
/// dispatch 時に mode で分岐する ([`StringFetchMode`] / [`ContentPart`] /
/// [`ContentTextKeyword`] と同じ per-context 専用 enum 慣行、raikiri-spike-6s1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContentListMode {
    /// CSS Content 3 §2 broad `<content-list>` — `content` property 用。
    /// 受理: `<string>` bare literal / `counter()` / `counters()` / `string()` /
    /// `attr()` / `target-counter()` / `target-counters()` / `target-text()` /
    /// `content()`。
    CssContent3,
    /// CSS GCPM 3 §1.1.1 narrow local `<content-list>` — `string-set` 用。
    /// 受理: `<string>` bare literal / `counter()` / `counters()` / `content()` /
    /// `attr()`。**明示 reject**: `string()` (bare `<string>` literal とは別),
    /// `target-counter()`, `target-counters()`, `target-text()` (GCPM 3 §1.1.1
    /// L82 verbatim grammar より導出、cascade で declaration drop → shadow 効果を
    /// spec 準拠に一致させる)。
    GcpmStringSet,
}

/// `content` property の value item — cascade static side の中間表現。
///
/// design doc §7.1 の `raikiri_traits::ContentValueItem` に 1:1 mapping する
/// (下流 raikiri-dom が runtime resolve 時に翻訳)。raikiri-style は raikiri-traits
/// に依存しない leaf crate = 94e/3ps Phase B により、counter-* wire-through
/// pattern (raikiri-spike-s85) と同様に **local** な intermediate type で保持し、
/// downstream 側で shared trait type にマッピングする。
///
/// Variants は spec の function grammar 順:
/// - Literal: bare `<string>` (§2.1)
/// - Counter / Counters: CSS Lists 3 §4.7
///   <https://www.w3.org/TR/css-lists-3/#counter-functions>
/// - String: CSS Content 3 §2.7.2 <https://www.w3.org/TR/css-content-3/#string-function>
/// - Attr: CSS Content 3 §2.1 <https://www.w3.org/TR/css-content-3/#strings>
/// - Target*: CSS Content 3 §2.6.1-3
///   <https://www.w3.org/TR/css-content-3/#target-counter>,
///   <https://www.w3.org/TR/css-content-3/#target-counters>,
///   <https://www.w3.org/TR/css-content-3/#target-text>
/// - Content: CSS GCPM 3 §1.1.1.1
///   <https://www.w3.org/TR/css-gcpm-3/#content-list> (raikiri-spike-5ri)
///
/// URL は raw `String` として保持 (raikiri-style は `url` crate に依存しない —
/// runtime resolve 段で `url::Url` へ parse する consumer 責務)。
///
/// (raikiri-spike-m5.1)
///
/// # `#[non_exhaustive]` semantics (fulgur / downstream consumer 向け verbatim)
///
/// enum-level `#[non_exhaustive]` は downstream の `match` に `_ =>` arm を
/// 強制することで新 variant 追加を forward-compatible にするが、**既存 variant
/// の tuple constructor 呼び出しは block しない**。ゆえに既存 variant の
/// payload **type** 変更は downstream の constructor を compile-break させる。
///
/// Sprint 9 hardening (bd raikiri-spike-d9y.1、SEC HIGH cascade memory DoS fix)
/// では [`Literal`](Self::Literal) の payload を `String` → [`SmolStr`] に
/// 変更した (bd raikiri-spike-q3f wall/umbrella formal declare)。SmolStr は
/// `Deref<Target = str>` を提供するため、pattern-match で payload を **読む**
/// consumer は `match cc { ContentComponent::Literal(s) => &*s, .. }` や
/// `s.as_str()` / `s.len()` などの `&str` API がそのまま動作する。**construct**
/// する consumer のみ `ContentComponent::Literal("foo".into())` を
/// `ContentComponent::Literal(SmolStr::new("foo"))` (または `.into()` が有効な
/// context では対応する `From` impl) に書き換える。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentComponent {
    /// `<string>` bare literal (`content: "hello"`)。
    ///
    /// [`SmolStr`] は 22 bytes 以下を inline、超過分は内部 `Arc<str>` 保存で
    /// clone が O(1) bump になる (raikiri-spike-d9y.1 の DoS 直系 attack vector
    /// `content: "<large>"` に対する secondary defense、primary は outer
    /// [`PropertyValue::Content`] の [`Arc<Vec<..>>`] wrap)。
    ///
    /// **Consumer 向け** (bd raikiri-spike-q3f wall/umbrella formal declare):
    /// pre-d9y.1 の payload は `String` だった。SmolStr は `Deref<Target = str>`
    /// を提供するので、read-side (`&*s` / `s.as_str()` / `s.len()` / `for c in s.chars()`)
    /// は透過的に継続動作する。construct-side のみ `SmolStr::new("foo")` (または
    /// `SmolStr::from(String)`) へ書き換える。enum-level docstring
    /// §`#[non_exhaustive]` semantics も参照。
    Literal(SmolStr),
    /// `counter(<counter-name>, <counter-style>?)`。
    Counter { name: SmolStr, style: CounterStyle },
    /// `counters(<counter-name>, <string>, <counter-style>?)`。
    Counters {
        name: SmolStr,
        separator: String,
        style: CounterStyle,
    },
    /// `string(<custom-ident>, [ first | start | last | first-except ]?)`。
    String {
        name: SmolStr,
        fetch: StringFetchMode,
    },
    /// `attr(<attribute-name>)` (§2.1、type/fallback は M5+ scope)。
    Attr { name: SmolStr },
    /// `target-counter([<string>|<url>], <counter-name>, <counter-style>?)`。
    TargetCounter {
        url: String,
        name: SmolStr,
        style: CounterStyle,
    },
    /// `target-counters([<string>|<url>], <counter-name>, <string>, <counter-style>?)`。
    TargetCounters {
        url: String,
        name: SmolStr,
        separator: String,
        style: CounterStyle,
    },
    /// `target-text([<string>|<url>], [ content | before | after | first-letter ]?)`。
    TargetText { url: String, part: ContentPart },
    /// `content([ text | before | after | first-letter ]?)` — GCPM 3 §1.1.1.1
    /// <https://www.w3.org/TR/css-gcpm-3/#content-list>。
    /// 現要素 (または擬似要素) の string value を named string に挿入する用途で、
    /// `<content-list>` の一員として `string-set` および `content` property の
    /// content-list 内で受理される。keyword 省略時は spec default `Text`。
    /// runtime resolve は raikiri-dom 責務 (m5.1 wire-through pattern)。
    Content { keyword: ContentTextKeyword },
}

/// `display` property の value。M1.4a scope では `block` / `inline` のみ。
///
/// spec §M1.4a Non-goals: `table*`, `flex`, `grid`, `none` 等は M6+。
/// (raikiri-spike-m1.22)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayValue {
    Block,
    Inline,
}

/// `position` property の value — M5 static-side scope では `static` (default) と
/// GCPM `running(<custom-ident>)` のみ受理する。
///
/// CSS GCPM 3 §1.2.1 "The running() value"
/// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>: `position: running(name)`
/// は element を normal flow から取り除き、`element()` 経由で page margin box に
/// 配置可能な template として登録する。
///
/// `relative` / `absolute` / `fixed` / `sticky` は M5+ scope 外、silent drop
/// (parse_position が `None`)。`static` を明示的に variant 化しているのは、
/// 先行の `position: running(x)` を later cascade で上書き無効化する用途
/// (`.foo { position: running(hdr) } .foo.reset { position: static }` の
/// 後者が winner になったとき、`apply_value` は no-op、`inherit_from` 起点で
/// 空 `running_templates` が残る)。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PositionValue {
    /// `static` — spec default、running() を suppress。
    ///
    /// `Default` は derive しない — 本 crate の convention は "derive `Default`
    /// iff `.default()` が call される" (37n sibling [`DisplayValue`] と同じ、
    /// spec default は初期化側 [`crate::computed::ComputedValues::initial`] が
    /// 直接指定する)。
    Static,
    /// `running(<custom-ident>)`。`<custom-ident>` は case-preserved の smol str。
    Running(SmolStr),
}

/// M1.4 でサポートする property の resolved value。
///
/// 認識できない property (`background-color` / `margin` / ...) や invalid value
/// (`font-size: 1em` — em 未対応) は parser 段で `None` に落として rule から
/// silently 除外される。
///
/// # `#[non_exhaustive]` semantics (fulgur / downstream consumer 向け verbatim)
///
/// enum-level `#[non_exhaustive]` は downstream の `match` を forward-compatible
/// にする (新 variant 追加時 `_ =>` arm が必ず求められる) が、**既存 variant の
/// tuple constructor 呼び出しは block しない**。したがって variant の payload
/// **type** が変わると constructor 側は普通に compile-break する。
///
/// Sprint 9 hardening (bd raikiri-spike-d9y.1、SEC HIGH cascade memory DoS fix)
/// では正にこの break が発生し、Sprint 10 wall/umbrella formal declare
/// (bd raikiri-spike-q3f) で以下を fulgur consumer 向け migration 対象として
/// 表明する:
///
/// - [`Content`](Self::Content): `Content(Vec<ContentComponent>)` →
///   `Content(Arc<Vec<ContentComponent>>)`
/// - [`StringSet`](Self::StringSet): payload の outer `Vec<..>` を `Arc<Vec<..>>` に
///
/// Sprint 9 Wave 2 hardening (bd raikiri-spike-d9y.2、同 SEC HIGH の counter-*
/// 拡張) は Sprint 10 時点で consumer live impact 0 だが同 pattern:
///
/// - [`CounterReset`](Self::CounterReset) / [`CounterIncrement`](Self::CounterIncrement) /
///   [`CounterSet`](Self::CounterSet): `Vec<(SmolStr, i32)>` → `Arc<Vec<(SmolStr, i32)>>`
///
/// Pattern-match で payload を **読む** consumer は `Arc<Vec<T>>` の
/// `Deref<Target = Vec<T>>` → `Deref<Target = [T]>` chain により、`match` arm
/// で `PropertyValue::Content(components) => components.iter()` のような使い方が
/// **透過的に継続動作** する (`&Arc<Vec<T>>` は autoderef で `&[T]` として使える)。
/// 一方、`PropertyValue::Content(vec![...])` のように payload を **construct** する
/// 場合は `PropertyValue::Content(Arc::new(vec![...]))` への書き換えが必要。
/// 詳細は `docs/superpowers/specs/2026-07-20-raikiri-0.1-to-0.2-migration.md`
/// を参照。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// `color: <color>` — inherited、initial: black。
    Color(CssColor),
    /// `font-family: <family-name>#` — inherited、initial: `[Atom::from("serif")]`。
    FontFamily(Vec<Atom>),
    /// `font-size: <length>` — inherited、initial: 16px。
    FontSize(Length),
    /// `font-weight: <integer>` — inherited、initial: 400。
    FontWeight(u16),
    /// `display: <block-or-inline>` — non-inherited、initial: inline
    /// (spec §M1.4a、raikiri-spike-m1.22)。
    Display(DisplayValue),
    /// `counter-reset: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited、initial: empty list (CSS Lists 3 §3)。
    /// missing integer は 0 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`pick_winners` の
    /// `value.clone()`) + inheritance walk clone (`resolve_inheritance` の
    /// `stack.push((child, computed.clone()))` + `out[idx] = computed.clone()`)
    /// が **shallow (Arc bump only)** になる。counter-* は non-inherited のため
    /// child は inherit_from で shared empty slot に落ちるが、winner までの経路
    /// (parent stack entry + cascaded candidates 蓄積) は deep-clone 経由だった。
    /// `* { counter-reset: c0 c1 ... cN }` × M element で O(N × M) → O(N + M)
    /// (raikiri-spike-d9y.2 SEC HIGH、d9y.1 Content/StringSet pattern の踏襲)。
    CounterReset(Arc<Vec<(SmolStr, i32)>>),
    /// `counter-increment: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited、initial: empty list (CSS Lists 3 §3)。
    /// missing integer は 1 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::CounterReset`] と同 rationale
    /// (raikiri-spike-d9y.2)。
    CounterIncrement(Arc<Vec<(SmolStr, i32)>>),
    /// `counter-set: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited、initial: empty list (CSS Lists 3 §3)。
    /// missing integer は 0 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::CounterReset`] と同 rationale
    /// (raikiri-spike-d9y.2)。
    CounterSet(Arc<Vec<(SmolStr, i32)>>),
    /// `content: normal | none | <content-list>` — non-inherited、initial:
    /// empty list (spec の `normal` / `none` を空 list として扱う、pseudo-element
    /// 生成判断は下流 layer)。M5 gcpm-directive-emit static-side
    /// (raikiri-spike-m5.1)、CSS Content 3 §2.1
    /// <https://www.w3.org/TR/css-content-3/#content-property>。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone + inheritance walk stack
    /// entry clone + per-node write が **shallow (Arc bump only)** になる。
    /// `* { content: "<large>" }` × N element の O(N × M) memory blow-up を
    /// 単一 heap slot 共有で塞ぐ (raikiri-spike-d9y.1 SEC HIGH)。
    ///
    /// **Consumer 向け** (bd raikiri-spike-q3f wall/umbrella formal declare):
    /// pattern-match で payload を **読む** 場合は `Arc<Vec<T>>` の deref chain
    /// (Vec → slice) により従来の `PropertyValue::Content(components) =>
    /// components.iter().for_each(..)` がそのまま動作する。**construct** する
    /// 場合のみ `PropertyValue::Content(Arc::new(vec![..]))` の書き換えが必要。
    /// enum-level docstring §`#[non_exhaustive]` semantics も参照。
    Content(Arc<Vec<ContentComponent>>),
    /// `string-set: none | [ <custom-ident> <content-list> ]#` — non-inherited、
    /// initial: empty list。各 entry は `(name, content-list)` pair。
    /// CSS GCPM 3 §3.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>、
    /// `<content-list>` は CSS Content 3 §2 (m5.1 で parser 実装済)。
    /// 名前解決と runtime string() 参照は下流 (raikiri-dom) 責務。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::Content`] と同じ理由 —
    /// `* { string-set: name "<large>" }` × N element 経路の同種 DoS を塞ぐ
    /// (raikiri-spike-d9y.1)。
    ///
    /// **Consumer 向け** (bd raikiri-spike-q3f wall/umbrella formal declare):
    /// [`Self::Content`] と同じく outer `Arc` は read-side は deref 透過、
    /// construct-side (`PropertyValue::StringSet(vec![(name, items)])`) のみ
    /// `PropertyValue::StringSet(Arc::new(vec![..]))` に書き換える。inner
    /// `Vec<ContentComponent>` は Arc 化しない (per-entry share の hit率 が
    /// 想定できないため、outer 単段で d9y.1 の攻撃経路を塞ぐ設計)。
    StringSet(Arc<Vec<(SmolStr, Vec<ContentComponent>)>>),
    /// `position: static | running(<custom-ident>)` — non-inherited、initial:
    /// `static`。M5 static-side ε (raikiri-spike-m5.4)。
    /// CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax>。
    /// M5 scope では `running()` seed emit のみが下流に伝わる —
    /// `Static` は `apply_value` で no-op (先行 `running()` を上書き suppress
    /// する discriminant 用途、spec default に相当)。
    /// `relative` / `absolute` / `fixed` / `sticky` は M5+ scope 外、parser 段で drop。
    Position(PositionValue),
}

/// Property key (cascade で "同一 property を勝ち取る" ための discriminant)。
///
/// cascade.rs の per-node winner selection、および page.rs の
/// [`cascade_page`](crate::page::cascade_page) が [`PageCascadeResult`] の
/// map key に使う。`PropertyValue` の variant tag を stateless に抜き出したもので
/// 追加情報を持たないため public に露出する (raikiri-spike-m4.1、[`PageCascadeResult`]
/// が `pub` 型を要求するため — clippy `private_interfaces` 対応)。
///
/// [`PageCascadeResult`]: crate::page::PageCascadeResult
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropertyKey {
    Color,
    FontFamily,
    FontSize,
    FontWeight,
    Display,
    CounterReset,
    CounterIncrement,
    CounterSet,
    Content,
    StringSet,
    Position,
}

impl PropertyValue {
    /// この value が属する property key を返す。
    ///
    /// cascade winner selection で "同一 property を勝ち取る" ための discriminant として、
    /// また `@page` cascade 結果 map の key として使う。
    pub fn key(&self) -> PropertyKey {
        match self {
            PropertyValue::Color(_) => PropertyKey::Color,
            PropertyValue::FontFamily(_) => PropertyKey::FontFamily,
            PropertyValue::FontSize(_) => PropertyKey::FontSize,
            PropertyValue::FontWeight(_) => PropertyKey::FontWeight,
            PropertyValue::Display(_) => PropertyKey::Display,
            PropertyValue::CounterReset(_) => PropertyKey::CounterReset,
            PropertyValue::CounterIncrement(_) => PropertyKey::CounterIncrement,
            PropertyValue::CounterSet(_) => PropertyKey::CounterSet,
            PropertyValue::Content(_) => PropertyKey::Content,
            PropertyValue::StringSet(_) => PropertyKey::StringSet,
            PropertyValue::Position(_) => PropertyKey::Position,
        }
    }
}

/// Property name + Parser から `PropertyValue` を produce。
/// 認識できない name / invalid value は `None`。
pub(crate) fn parse_value(name: &str, input: &mut Parser<'_, '_>) -> Option<PropertyValue> {
    // ascii-lowercase 比較で property name を dispatch。
    let normalized_name = name.to_ascii_lowercase();
    match normalized_name.as_str() {
        "color" => parse_color(input).map(PropertyValue::Color),
        "font-family" => parse_font_family(input).map(PropertyValue::FontFamily),
        "font-size" => parse_font_size(input).map(PropertyValue::FontSize),
        "font-weight" => parse_font_weight(input).map(PropertyValue::FontWeight),
        "display" => parse_display(input).map(PropertyValue::Display),
        // CSS Lists 3 §3 counter properties (raikiri-spike-s85、M5 pre-work)。
        // spec default: reset = 0、increment = 1、set = 0。
        // Arc wrap は raikiri-spike-d9y.2 の cascade memory DoS fix (per-element
        // clone を shallow bump 化)、空 list は 3 property 共通 shared Arc slot
        // (`empty_counter_entries`) に落として per-node allocation regression を
        // 避ける (d9y.1 Content/StringSet precedent と同 pattern)。
        "counter-reset" => parse_counter_property(input, 0).map(|v| {
            if v.is_empty() {
                PropertyValue::CounterReset(empty_counter_entries())
            } else {
                PropertyValue::CounterReset(Arc::new(v))
            }
        }),
        "counter-increment" => parse_counter_property(input, 1).map(|v| {
            if v.is_empty() {
                PropertyValue::CounterIncrement(empty_counter_entries())
            } else {
                PropertyValue::CounterIncrement(Arc::new(v))
            }
        }),
        "counter-set" => parse_counter_property(input, 0).map(|v| {
            if v.is_empty() {
                PropertyValue::CounterSet(empty_counter_entries())
            } else {
                PropertyValue::CounterSet(Arc::new(v))
            }
        }),
        // CSS Content 3 §2.1 content property (raikiri-spike-m5.1、M5 gcpm-directive-emit static side)。
        // Arc wrap は raikiri-spike-d9y.1 の cascade memory DoS fix (per-element clone を
        // shallow bump 化)、empty list は shared Arc slot に落として per-node allocation
        // regression を避ける (advisor calibration)。
        "content" => parse_content(input).map(|v| {
            if v.is_empty() {
                PropertyValue::Content(empty_content_list())
            } else {
                PropertyValue::Content(Arc::new(v))
            }
        }),
        // CSS GCPM 3 §3.1 string-set (raikiri-spike-m5.3、M5 static-side β)。
        // Arc wrap は raikiri-spike-d9y.1、同 rationale。
        "string-set" => parse_string_set(input).map(|v| {
            if v.is_empty() {
                PropertyValue::StringSet(empty_string_set_entries())
            } else {
                PropertyValue::StringSet(Arc::new(v))
            }
        }),
        // CSS GCPM 3 §1.2.1 position: running() (raikiri-spike-m5.4、M5 static-side ε)。
        // M5 scope では `static` + `running(<custom-ident>)` のみ受理、
        // `relative` / `absolute` / `fixed` / `sticky` は silent drop (M5+ scope 外)。
        "position" => parse_position(input).map(PropertyValue::Position),
        _ => None,
    }
}

/// `<color>` を parse する。
///
/// cssparser 0.37 は (0.36 までと異なり) 汎用 `Color` enum / `Color::parse` を
/// 提供しない — それは別 crate `cssparser-color` 側に移った。ここでは
/// `cssparser::color` に残っている building block (`parse_hash_color` /
/// `parse_named_color`) と、`rgb()` / `rgba()` function の手動 parse で
/// hex / named / rgb() の 3 形式をカバーする (m1.4 scope)。
fn parse_color(input: &mut Parser<'_, '_>) -> Option<CssColor> {
    let token = input.next().ok()?.clone();
    match token {
        Token::Hash(ref value) | Token::IDHash(ref value) => {
            let (r, g, b, alpha) = parse_hash_color(value.as_bytes()).ok()?;
            Some(CssColor {
                r,
                g,
                b,
                a: clamp_unit_f32(alpha),
            })
        }
        Token::Ident(ref name) => {
            let (r, g, b) = parse_named_color(name).ok()?;
            Some(CssColor { r, g, b, a: 255 })
        }
        Token::Function(ref name)
            if name.eq_ignore_ascii_case("rgb") || name.eq_ignore_ascii_case("rgba") =>
        {
            input.parse_nested_block(parse_rgb_function).ok()
        }
        _ => None,
    }
}

/// `rgb( <integer> , <integer> , <integer> [, <number>]? )` の中身 (関数呼び出しの
/// 括弧内) を parse する。`parse_nested_block` の caller 側で `rgb(` / `rgba(` の
/// function token は既に consume 済み。
fn parse_rgb_function<'i>(input: &mut Parser<'i, '_>) -> Result<CssColor, ParseError<'i, ()>> {
    let r = clamp_channel(input.expect_integer()?);
    input.expect_comma()?;
    let g = clamp_channel(input.expect_integer()?);
    input.expect_comma()?;
    let b = clamp_channel(input.expect_integer()?);
    let a = if input.try_parse(|input| input.expect_comma()).is_ok() {
        clamp_unit_f32(input.expect_number()?)
    } else {
        255
    };
    Ok(CssColor { r, g, b, a })
}

fn clamp_channel(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// `font-family: <family-name>#` を parse する。
///
/// comma-separated な family-name の list。各 family-name は quoted string
/// (`"Times New Roman"`) か、unquoted identifier の連続 (`Times New Roman` =
/// 3 ident が空白区切りで 1 family、CSS4 で有効) のいずれか。
///
/// 末尾で comma が続かなければ loop を止め、残り input (`!important` 等) は
/// 手を付けずに downstream (caller の `parse_important` / `expect_exhausted`)
/// に委ねる — `!` を garbage として拒否しないための Finding 3 対応。
fn parse_font_family(input: &mut Parser<'_, '_>) -> Option<Vec<Atom>> {
    let mut families = Vec::new();
    loop {
        // Try quoted string first (e.g. "Times New Roman")
        let family = if let Ok(s) = input.try_parse(|i| i.expect_string().cloned()) {
            Atom::from(s.as_ref())
        } else if let Ok(first) = input.try_parse(|i| i.expect_ident().cloned()) {
            // Unquoted ident sequence: `Times New Roman` = 3 idents joined by space
            let mut buf = first.as_ref().to_string();
            while let Ok(next) = input.try_parse(|i| i.expect_ident().cloned()) {
                buf.push(' ');
                buf.push_str(next.as_ref());
            }
            Atom::from(buf.as_str())
        } else {
            return None;
        };
        families.push(family);
        // Consume comma or stop (leaves remaining input alone)
        if input.try_parse(|i| i.expect_comma()).is_err() {
            break;
        }
    }
    if families.is_empty() {
        None
    } else {
        Some(families)
    }
}

/// `<length>` / `<length-percentage>` の共通 parser。1 token を consume する。
///
/// Grammar reference: CSS Values 4 §5 <https://www.w3.org/TR/css-values-4/#lengths>
/// / §5.5 <https://www.w3.org/TR/css-values-4/#percentages>.
///
/// # Mode selector
///
/// `allow_percentage` で受理集合を分岐:
/// - `false` → `<length>` mode: dimension unit のみ受理 (`px` / `em` / `rem` / `pt`)。
/// - `true` → `<length-percentage>` mode: 上記 4 unit + `%` token を受理。
///
/// 未対応 unit (`vw` / `vh` / `ch` / `ex` / `cm` / `mm` / `in` / `pc` / `Q` /
/// `cap` / `rcap` / `ic` / `ric` / `lh` / `rlh`) は spec-valid だが本 milestone
/// scope 外 (g04 category (b) milestone subset、defer 先 Sprint 13+ style backlog、
/// 未起票 — Epic 1 planner 判定)。`0` bare (unitless zero) も後続 milestone
/// (現行 behavior 踏襲、`parse_font_size` の existing test は unitless zero を
/// 受理しない spec-strict 挙動)。
///
/// # Sign / range
///
/// 本 helper は sign / range check を行わない — property ごとに要件が異なるため
/// (font-size は non-negative、margin は negative 許容、etc.)。caller 側で
/// post-filter する ([`parse_font_size`] は `>= 0.0` の Px-only guard を持つ)。
///
/// # Forward-provisioning
///
/// `allow_percentage=true` mode は本 task では caller 未使用 (font-size は
/// length-only)。以下 blocked task で consume 予定:
/// - `raikiri-spike-0vv.5` margin longhand + shorthand parse
/// - `raikiri-spike-0vv.6` padding longhand + shorthand parse
/// - `raikiri-spike-0vv.9` line-height parse
///
/// これら margin/padding/line-height の grammar は spec で `<length-percentage>`
/// (percentage 受理側)、共通 helper 化により重複 dimension unit dispatch を回避。
fn parse_length_value(input: &mut Parser<'_, '_>, allow_percentage: bool) -> Option<Length> {
    match input.next().ok()? {
        Token::Dimension { value, unit, .. } => match unit.to_ascii_lowercase().as_str() {
            "px" => Some(Length::Px(*value)),
            "em" => Some(Length::Em(*value)),
            "rem" => Some(Length::Rem(*value)),
            "pt" => Some(Length::Pt(*value)),
            // (b) milestone subset — 他 CSS Values 4 unit は未対応、silent drop。
            _ => None,
        },
        Token::Percentage { unit_value, .. } if allow_percentage => {
            // cssparser 0.37 tokenizer は `50%` を `unit_value = 0.5` として emit
            // (`value / 100.0`)、Length::Percent は authored number (50.0) を保持する
            // ため × 100.0 で戻す。
            Some(Length::Percent(*unit_value * 100.0))
        }
        _ => None,
    }
}

/// `font-size: <length>` を parse する。
///
/// Grammar: `<'font-size'> = <length> | <percentage> | ...` (CSS Fonts 4
/// <https://www.w3.org/TR/css-fonts-4/#font-size-prop>) のうち、本 milestone は
/// **non-negative `<length>` unit の `px` のみ**を受理 (g04 category (b) milestone
/// subset、em/rem/pt/% は Author CSS seed [`parse_length_value`] helper 側で認識
/// されるが font-size 経路では post-filter で drop、defer 先 未起票 — 次 planner が
/// font-size context resolve と bundle 判定)。
///
/// [`Length::Em`] / [`Length::Rem`] / [`Length::Pt`] は resolve に font stack context
/// を要し、[`Length::Percent`] は parent font-size context (spec §6.1.1) を要する。
/// 現在 raikiri-style の cascade static side はこれら context を持たないため、
/// helper 経由で parse 成功しても本 property では None に落として declaration drop。
fn parse_font_size(input: &mut Parser<'_, '_>) -> Option<Length> {
    // helper を <length> mode で呼び、Px の non-negative case のみ受理。
    match parse_length_value(input, false)? {
        Length::Px(v) if v >= 0.0 => Some(Length::Px(v)),
        // (b) milestone subset: Em/Rem/Pt は spec-valid だが font-size context resolve
        // 未実装のため drop、negative Px も spec 上 invalid のため drop。
        _ => None,
    }
}

fn parse_font_weight(input: &mut Parser<'_, '_>) -> Option<u16> {
    // integer literal (100..=900) のみ、keyword は drop。
    match input.next().ok()? {
        Token::Number {
            int_value: Some(v), ..
        } if *v >= 100 && *v <= 900 => Some(*v as u16),
        _ => None,
    }
}

/// `display: <ident>` を parse する。
///
/// M1.4a scope では `block` / `inline` のみ受理、他 keyword (`flex`,
/// `grid`, `none`, `table*` 等) は silent drop (`None`)。
/// ASCII case-insensitive で ident を比較する (CSS spec 準拠)。
fn parse_display(input: &mut Parser<'_, '_>) -> Option<DisplayValue> {
    let ident = input.next().ok()?;
    match ident {
        Token::Ident(name) if name.eq_ignore_ascii_case("block") => Some(DisplayValue::Block),
        Token::Ident(name) if name.eq_ignore_ascii_case("inline") => Some(DisplayValue::Inline),
        _ => None,
    }
}

/// `counter-reset` / `counter-increment` / `counter-set` の value を parse する。
///
/// Grammar (CSS Lists 3 §3):
///   `<counter-name> = <custom-ident>` — CSS-wide keyword (inherit / initial /
///   unset / revert / revert-layer) + `default` + `none` を除く任意 ident。
///   `[ <counter-name> <integer>? ]+ | none`。
///
/// `default_number`: 各 property の spec default (reset=0、increment=1、set=0)。
///
/// `none` を top-level alternative として先に処理。以降は ident + optional
/// integer を LL(1) で peel。ident が reserved keyword、または最初の token が
/// ident でない (`counter-reset: 123 abc` 等) 場合は None を返し、rule.rs 側の
/// silent-drop で declaration が丸ごと落ちる。
///
/// 途中 ident (`chapter none`) が reserved の場合は `try_parse` の rewind で
/// 未消費のまま loop を抜け、caller の `expect_exhausted` (rule.rs)
/// が leftover token を検出して declaration を drop する。
fn parse_counter_property(
    input: &mut Parser<'_, '_>,
    default_number: i32,
) -> Option<Vec<(SmolStr, i32)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut result = Vec::new();
    loop {
        // reserved keyword を counter-name として受理しない (spec §3、`<custom-ident>`
        // の除外リスト)。try_parse の rewind で reserved 検出時は unconsumed に戻す。
        let name = match input.try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
            let ident = i.expect_ident()?.clone();
            if is_reserved_counter_name(&ident) {
                Err(i.new_custom_error(()))
            } else {
                Ok(SmolStr::new(ident.as_ref()))
            }
        }) {
            Ok(name) => name,
            Err(_) => break,
        };
        // optional trailing `<integer>` (missing → property-specific default)。
        let value = input
            .try_parse(|i| i.expect_integer())
            .unwrap_or(default_number);
        result.push((name, value));
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// `<counter-name>` = `<custom-ident>` の除外リスト (CSS Lists 3 §3 + CSS Values 4)。
///
/// CSS-wide keyword + `default` (Counter Styles L3) + `none` (top-level alternative)
/// を弾く。case-insensitive 比較。
fn is_reserved_counter_name(ident: &str) -> bool {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default" | "none"
    )
}

/// `content: normal | none | <content-list>` を parse する
/// (CSS Content 3 §2.1 <https://www.w3.org/TR/css-content-3/#content-property>)。
///
/// `normal` / `none` は spec で意味が異なる (pseudo-element の生成/非生成) が、
/// 本 crate は cascade static side に留まり生成判断は下流に委ねるため、両者を
/// 空 `Vec` に落として区別を持たない (§7.1 downstream mapping で必要になれば
/// 変異させる)。counter-* precedent (raikiri-spike-s85) と同じ shape。
///
/// items+ loop は `<string>` literal と function token (`counter(...)` 等) を
/// 順次 peel する。認識できない token に当たった時点で loop を break、caller
/// の `expect_exhausted` (rule.rs) が leftover を検知して declaration ごと drop。
///
/// `alt text` (spec `... [/ <string>...]?`) は M5 pre-work scope 外、`/` 以降は
/// unconsumed のまま caller に返す (現状 rule.rs の `expect_exhausted` により
/// declaration drop、alt text 対応時に本関数を extend)。
fn parse_content(input: &mut Parser<'_, '_>) -> Option<Vec<ContentComponent>> {
    // `normal` / `none` = 空 list (top-level alternative)。
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(Vec::new());
    }
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let items = parse_content_list_items(input, ContentListMode::CssContent3);
    if items.is_empty() { None } else { Some(items) }
}

/// `<content-list>` の items+ loop 部分。
///
/// `<string>` bare literal と function token (`counter(...)` / `string(...)` /
/// `target-*()` / `attr(...)` / `content(...)`) を順次 peel。認識できない
/// token に当たった時点で break — 呼び出し側が leftover を検知して drop する。
///
/// `content` property (`parse_content`) と `string-set` property
/// (`parse_string_set`) の両方から call されるが、GCPM 3 §1.1.1 は string-set
/// 向けに CSS Content 3 §2 の broad list を narrower に再定義しているため、
/// `mode` パラメータで受理 function 集合を分岐する:
/// - [`ContentListMode::CssContent3`] — content property (CSS Content 3 §2
///   <https://www.w3.org/TR/css-content-3/#content-list3>)。全 8 function を受理。
/// - [`ContentListMode::GcpmStringSet`] — string-set property (CSS GCPM 3
///   §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#content-list>)。`string()` と
///   `target-counter()` / `target-counters()` / `target-text()` は spec grammar
///   に含まれず reject (bare `<string>` literal は両 mode で受理)。
///
/// bare literal 分岐は spec 上両 mode で共通 (どちらの `<content-list>` grammar
/// も `<string>` を top-level alternative に含む) なので mode 判定なし。分岐は
/// [`parse_content_function`] の match arm で mode guard を掛ける。
///
/// (raikiri-spike-m5.1 で導入、raikiri-spike-6s1 で mode-parameterize)
fn parse_content_list_items(
    input: &mut Parser<'_, '_>,
    mode: ContentListMode,
) -> Vec<ContentComponent> {
    let mut items = Vec::new();
    loop {
        // bare `<string>` literal — 両 mode 共通 (mode gate 不要)。
        if let Ok(s) = input.try_parse(|i| i.expect_string_cloned()) {
            items.push(ContentComponent::Literal(SmolStr::new(s.as_ref())));
            continue;
        }
        // function — mode に応じて `string()` / `target-*()` を reject する
        // 判定は `parse_content_function` の match arm side で実施。
        let parsed = input.try_parse(|i| -> Result<ContentComponent, ParseError<'_, ()>> {
            let name = i.expect_function()?.clone();
            i.parse_nested_block(|inner| {
                parse_content_function(name.as_ref(), mode, inner)
                    .ok_or_else(|| inner.new_custom_error(()))
            })
        });
        match parsed {
            Ok(c) => items.push(c),
            Err(_) => break,
        }
    }
    items
}

/// `string-set: none | [ <custom-ident> <content-list> ]#` を parse する
/// (CSS GCPM 3 §3.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>)。
///
/// `none` を top-level alternative として先に処理し、以降は
/// `(name, content-list)` entry を comma-separated で peel する。
///
/// `<custom-ident>` は CSS-wide keyword + `default` (css-values-4 §3.6 が
/// 将来の CSS-wide keyword 用に予約) + `none` (top-level alt、gcpm-3 §3.1) を弾く。
///
/// ## Entry separator の strict 化 (raikiri-spike-1ll)
///
/// `#` (comma-separated multiplier、CSS Values 4 §3.3
/// <https://www.w3.org/TR/css-values-4/#mult-comma>) は entry 間に comma を
/// 要求する一方、**trailing comma を許容しない**。従って comma を consume した
/// 直後の loop iteration では次 entry の name parse **必須** — 失敗すれば
/// `#` production 全体が spec-invalid、declaration drop = `None`。
///
/// 初回 iteration で name parse が失敗する case (`string-set: ,`,
/// `string-set: "x"` 等 name 不在) も含めて `.ok()?` で一律に `None` 上位伝播
/// する。この strict `?` propagation は sibling
/// [`parse_optional_counter_style`] (raikiri-spike-zik) と同 principle。
///
/// `<content-list>` は 1+ items 必須 (CSS Content 3 §2)。name の後に 1 item も
/// peel できなければ malformed → `None` (declaration drop)。
fn parse_string_set(input: &mut Parser<'_, '_>) -> Option<Vec<(SmolStr, Vec<ContentComponent>)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut entries = Vec::new();
    loop {
        // <custom-ident> — CSS-wide keyword + `default` + `none` を弾く。
        // 既存 `is_reserved_custom_ident` (css-wide + default) と、property-specific
        // top-level alternative の `none` reject を組み合わせる (m5.1 の
        // `is_reserved_custom_ident` docstring の想定 usage)。
        //
        // `.ok()?` で strict 上位伝播: (a) 初回 iteration で name 不在 = `#`
        // production 0 entries、(b) 直前 iteration で bottom `expect_comma` が
        // succeed した直後 = trailing comma、の 2 case を一律 `None` に落とす。
        let name = input
            .try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
                let ident = i.expect_ident()?.clone();
                if is_reserved_custom_ident(&ident) || ident.eq_ignore_ascii_case("none") {
                    Err(i.new_custom_error(()))
                } else {
                    Ok(SmolStr::new(ident.as_ref()))
                }
            })
            .ok()?;
        // <content-list> は 1+ items 必須。0 items → declaration drop。
        // GCPM 3 §1.1.1 narrow local <content-list> = `string()` と `target-*()`
        // を受理しない (raikiri-spike-6s1、詳細は `ContentListMode` doc)。
        let items = parse_content_list_items(input, ContentListMode::GcpmStringSet);
        if items.is_empty() {
            return None;
        }
        entries.push((name, items));
        // 次 entry の separator: comma で継続、他 token で loop を抜ける
        // (caller `expect_exhausted` が leftover token を drop)。break 到達時は
        // 直前の push で entries 非空 — なので tail は無条件 `Some(entries)`。
        if input.try_parse(|i| i.expect_comma()).is_err() {
            break;
        }
    }

    // break 到達 = 直前の push を経ている、`.ok()?` 経路以外で loop を抜ける
    // 唯一の exit なので `entries` は必ず 1+。
    Some(entries)
}

/// Dispatch on function name (ASCII-case-insensitive、spec identifier 慣行)。
/// 未知の function name または引数 parse 失敗は `None` — caller の
/// `parse_nested_block` が custom error に変換する。
///
/// `mode` は property ごとの `<content-list>` 語彙を選ぶ (詳細は
/// [`ContentListMode`] doc):
/// - [`ContentListMode::CssContent3`] (`content` property, CSS Content 3 §2)
///   では全 arm を許可。
/// - [`ContentListMode::GcpmStringSet`] (`string-set` property, CSS GCPM 3
///   §1.1.1) では `string` / `target-counter` / `target-counters` /
///   `target-text` arm を match guard で外し fall-through で `None` を返す
///   (= declaration drop、caller の `parse_string_set` が `<content-list>` 0
///   items → `None`)。`counter` / `counters` / `content` / `attr` は両 mode で
///   spec grammar に含まれるため gate なし。
///
/// 各 `parse_*_fn` は自身では `expect_exhausted` を呼ばない —
/// [`parse_content`] 側の `parse_nested_block` が内部で
/// [`Parser::parse_entirely`] を経由し、closure 成功後の余剰 token を
/// exhaustion check で拒否する ([`parse_rgb_function`] と同じ規約)。
fn parse_content_function(
    name: &str,
    mode: ContentListMode,
    input: &mut Parser<'_, '_>,
) -> Option<ContentComponent> {
    match name.to_ascii_lowercase().as_str() {
        "string" if matches!(mode, ContentListMode::CssContent3) => parse_string_fn(input),
        "counter" => parse_counter_fn(input),
        "counters" => parse_counters_fn(input),
        "attr" => parse_attr_fn(input),
        "target-counter" if matches!(mode, ContentListMode::CssContent3) => {
            parse_target_counter_fn(input)
        }
        "target-counters" if matches!(mode, ContentListMode::CssContent3) => {
            parse_target_counters_fn(input)
        }
        "target-text" if matches!(mode, ContentListMode::CssContent3) => {
            parse_target_text_fn(input)
        }
        "content" => parse_content_fn(input),
        _ => None,
    }
}

/// `<custom-ident>` (CSS Values 4 §3.6): CSS-wide keyword + `default` + `none` を
/// 除いた任意 ident。case-preserving、smol str で保持。
///
/// counter-name / string-name / target-* の name 引数で共通に使う。
fn parse_custom_ident(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_custom_ident(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// `<custom-ident>` 除外リスト (CSS Values 4 §3.6)。
///
/// CSS-wide keyword (`inherit` / `initial` / `unset` / `revert` /
/// `revert-layer`) と `default` のみを弾く。`none` はここでは除外せず、
/// より狭い grammar (`<counter-name>` 等) の追加除外は個別の predicate
/// (例 [`is_reserved_counter_name`]) で行う。case-insensitive 比較。
fn is_reserved_custom_ident(ident: &str) -> bool {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default"
    )
}

/// `string(<custom-ident> [, [ first | start | last | first-except ]? ])`。
/// CSS Content 3 §2.7.2 <https://www.w3.org/TR/css-content-3/#string-function>。
fn parse_string_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_custom_ident(input)?;
    let fetch = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_string_fetch(input)?
    } else {
        StringFetchMode::default()
    };
    Some(ContentComponent::String { name, fetch })
}

fn parse_string_fetch(input: &mut Parser<'_, '_>) -> Option<StringFetchMode> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "first" => Some(StringFetchMode::First),
        "start" => Some(StringFetchMode::Start),
        "last" => Some(StringFetchMode::Last),
        "first-except" => Some(StringFetchMode::FirstExcept),
        _ => None,
    }
}

/// `<counter-name>` (CSS Lists 3 §4
/// <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>):
/// `<custom-ident>` から `none` を追加除外した production。
/// spec 原文: "A <counter-name> name cannot match the keyword `none`; such an
/// identifier is invalid as a <counter-name>"。
///
/// counter() / counters() (§4.7) の first argument、および
/// counter-reset / counter-increment / counter-set property (§3) の name 引数で
/// 使う。後者は既に [`parse_counter_property`] が [`is_reserved_counter_name`]
/// 経由で reject 済 — 本 helper は前者を同じ predicate に揃えるための wrapper
/// (raikiri-spike-afv — codex final for m5.1)。
fn parse_counter_name(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_counter_name(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// `counter(<counter-name>, <counter-style>?)`。
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>。
/// first argument grammar は §4 `<counter-name>`
/// (<https://www.w3.org/TR/css-lists-3/#typedef-counter-name>) —
/// `<custom-ident>` から `none` を追加除外。
fn parse_counter_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_counter_name(input)?;
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::Counter { name, style })
}

/// `counters(<counter-name>, <string>, <counter-style>?)`。
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>。
/// first argument grammar は §4 `<counter-name>`
/// (<https://www.w3.org/TR/css-lists-3/#typedef-counter-name>) —
/// `<custom-ident>` から `none` を追加除外。
fn parse_counters_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_counter_name(input)?;
    input.expect_comma().ok()?;
    let separator = input.expect_string().ok()?.as_ref().to_string();
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::Counters {
        name,
        separator,
        style,
    })
}

/// optional trailing `, <counter-style>`。省略時は spec default `decimal`
/// (CSS Lists 3 §4.7 `counter()` / `counters()` の末尾引数
/// <https://www.w3.org/TR/css-lists-3/#counter-functions>、CSS Content 3 §2.6.1-2
/// `target-counter()` / `target-counters()` の末尾引数
/// <https://www.w3.org/TR/css-content-3/#target-counter>)。
///
/// grammar は `<counter-style>?` — `,` を先行させる時は ident 必須。
/// `,` を consume 後に ident 不在 (`counter(chapter,)` 等の trailing-comma)
/// は spec-invalid、`None` 上位伝播で declaration ごと drop する
/// (sibling [`parse_string_fetch`] / [`parse_content_part`] と同じ strict
/// `?` propagation、raikiri-spike-zik で silent Decimal fallback を除去)。
fn parse_optional_counter_style(input: &mut Parser<'_, '_>) -> Option<CounterStyle> {
    if input.try_parse(|i| i.expect_comma()).is_ok() {
        // comma consumed — ident 必須。失敗は None として上位伝播。
        let ident = input.expect_ident().ok()?.clone();
        Some(counter_style_from_ident(ident.as_ref()))
    } else {
        Some(CounterStyle::default())
    }
}

fn counter_style_from_ident(ident: &str) -> CounterStyle {
    if ident.eq_ignore_ascii_case("decimal") {
        CounterStyle::Decimal
    } else {
        CounterStyle::Named(SmolStr::new(ident))
    }
}

/// `attr(<attribute-name>)`。CSS Content 3 §2.1
/// <https://www.w3.org/TR/css-content-3/#strings>。
///
/// M5 static-side scope: type / fallback (attr(x string, "default") 等) は defer。
fn parse_attr_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = input.expect_ident().ok()?.clone();
    Some(ContentComponent::Attr {
        name: SmolStr::new(name.as_ref()),
    })
}

/// target-* の第 1 引数 `[ <string> | <url> ]` を raw String として抽出。
/// `url("...")` / `url(...)` / bare `"..."` を統一的に受ける
/// (cssparser の `expect_url_or_string` を使用)。
fn parse_target_url(input: &mut Parser<'_, '_>) -> Option<String> {
    input
        .expect_url_or_string()
        .ok()
        .map(|s| s.as_ref().to_string())
}

/// `target-counter([<string>|<url>], <custom-ident>, <counter-style>?)`。
/// CSS Content 3 §2.6.1 <https://www.w3.org/TR/css-content-3/#target-counter>。
fn parse_target_counter_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    input.expect_comma().ok()?;
    let name = parse_custom_ident(input)?;
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::TargetCounter { url, name, style })
}

/// `target-counters([<string>|<url>], <custom-ident>, <string>, <counter-style>?)`。
/// CSS Content 3 §2.6.2 <https://www.w3.org/TR/css-content-3/#target-counters>。
fn parse_target_counters_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    input.expect_comma().ok()?;
    let name = parse_custom_ident(input)?;
    input.expect_comma().ok()?;
    let separator = input.expect_string().ok()?.as_ref().to_string();
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::TargetCounters {
        url,
        name,
        separator,
        style,
    })
}

/// `target-text([<string>|<url>], [ content | before | after | first-letter ]?)`。
/// CSS Content 3 §2.6.3 <https://www.w3.org/TR/css-content-3/#target-text>。
fn parse_target_text_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    let part = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_content_part(input)?
    } else {
        ContentPart::default()
    };
    Some(ContentComponent::TargetText { url, part })
}

/// `position: static | running(<custom-ident>)` を parse する
/// (CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax>)。
///
/// M5 static-side ε (raikiri-spike-m5.4) の scope:
/// - `static` — [`PositionValue::Static`]、`inherit_from` の初期状態と一致するため
///   apply_value が no-op でも問題ない。cascade winner selection では
///   先行 `running(...)` を上書き suppress する identity 用途
///   (advisor calibration: standalone-static test だけでは実効性が問えない)。
/// - `running(<custom-ident>)` — [`PositionValue::Running`]、apply_value が
///   1-item `RunningTemplate` を computed.running_templates に seed する。
/// - 他 keyword (`relative` / `absolute` / `fixed` / `sticky`) は M5+ scope 外、
///   silent drop = `None`。
///
/// `<custom-ident>` の除外は m5.3 string-set と同じ規約:
/// [`is_reserved_custom_ident`] (CSS-wide keyword + `default`) に加えて
/// `none` を弾く。`none` は position property の他 spec-defined keyword
/// では無いが、custom-ident としては予約 alternative の慣行を残しつつ、
/// runtime resolve で `element(none)` 参照を誤って matching させないためのガード
/// (reviewer:spec interpretation point、m5.3 の `none` reject と同じ扱い)。
fn parse_position(input: &mut Parser<'_, '_>) -> Option<PositionValue> {
    // `static` は M5 scope で唯一受理する non-running keyword。
    if input
        .try_parse(|i| i.expect_ident_matching("static"))
        .is_ok()
    {
        return Some(PositionValue::Static);
    }
    // `running(<custom-ident>)`。function name は ASCII case-insensitive、
    // 中身の custom-ident は case-preserving で SmolStr に格納。
    let running = input.try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
        let fn_name = i.expect_function()?.clone();
        if !fn_name.eq_ignore_ascii_case("running") {
            return Err(i.new_custom_error(()));
        }
        i.parse_nested_block(|inner| -> Result<SmolStr, ParseError<'_, ()>> {
            let ident = inner.expect_ident()?.clone();
            if is_reserved_custom_ident(&ident) || ident.eq_ignore_ascii_case("none") {
                return Err(inner.new_custom_error(()));
            }
            Ok(SmolStr::new(ident.as_ref()))
        })
    });
    running.ok().map(PositionValue::Running)
}

fn parse_content_part(input: &mut Parser<'_, '_>) -> Option<ContentPart> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "content" => Some(ContentPart::Content),
        "before" => Some(ContentPart::Before),
        "after" => Some(ContentPart::After),
        "first-letter" => Some(ContentPart::FirstLetter),
        _ => None,
    }
}

/// `content([ text | before | after | first-letter ]?)`。
/// CSS GCPM 3 §1.1.1.1 <https://www.w3.org/TR/css-gcpm-3/#content-list>。
///
/// bare `content()` (spec 例 `h2 { string-set: heading content() }`) は
/// spec default `text` を意味する。target-text() の第 2 引数と違い、keyword は
/// paren 直下に置かれる (comma を先行させない)。
///
/// GCPM 3 §1.1.1 の narrow `<content-list>` (string-set 側) と CSS Content 3
/// §2 の broad `<content-list>` (content property 側) の **両方** に含まれる
/// 5 alt の 1 つのため、[`ContentListMode`] mode gate なし = 両 property 共通で
/// 受理される (raikiri-spike-6s1 で mode dispatch を導入した後もこの arm は
/// unconditional のまま)。
fn parse_content_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let keyword = if input.is_exhausted() {
        ContentTextKeyword::default()
    } else {
        parse_content_text_keyword(input)?
    };
    Some(ContentComponent::Content { keyword })
}

fn parse_content_text_keyword(input: &mut Parser<'_, '_>) -> Option<ContentTextKeyword> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "text" => Some(ContentTextKeyword::Text),
        "before" => Some(ContentTextKeyword::Before),
        "after" => Some(ContentTextKeyword::After),
        "first-letter" => Some(ContentTextKeyword::FirstLetter),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cssparser::ParserInput;

    fn parse(source: &str, name: &str) -> Option<PropertyValue> {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        parse_value(name, &mut parser)
    }

    #[test]
    fn color_parse_hex() {
        assert_eq!(
            parse("#ff0000", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn color_parse_named() {
        assert_eq!(
            parse("red", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn color_parse_rgb() {
        assert_eq!(
            parse("rgb(255, 0, 0)", "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn color_parse_invalid_returns_none() {
        assert_eq!(parse("bogus", "color"), None);
        assert_eq!(parse("", "color"), None);
    }

    #[test]
    fn font_size_parse_px() {
        assert_eq!(
            parse("16px", "font-size"),
            Some(PropertyValue::FontSize(Length::Px(16.0)))
        );
    }

    #[test]
    fn font_size_rejects_em_and_keyword() {
        assert_eq!(parse("1em", "font-size"), None);
        assert_eq!(parse("medium", "font-size"), None);
    }

    #[test]
    fn font_size_rejects_negative() {
        // spec: font-size は non-negative <length> のみ。
        assert_eq!(parse("-10px", "font-size"), None);
        assert_eq!(parse("-0.5px", "font-size"), None);
    }

    #[test]
    fn font_size_accepts_zero() {
        assert_eq!(
            parse("0px", "font-size"),
            Some(PropertyValue::FontSize(Length::Px(0.0)))
        );
    }

    #[test]
    fn font_family_parse_comma_list() {
        let got = parse(r#"Arial, "Times New Roman", serif"#, "font-family");
        let expected = Some(PropertyValue::FontFamily(vec![
            Atom::from("Arial"),
            Atom::from("Times New Roman"),
            Atom::from("serif"),
        ]));
        assert_eq!(got, expected);
    }

    #[test]
    fn font_family_unquoted_multi_word_single_family() {
        // CSS4: unquoted multi-word family name = ident sequence joined by space。
        let got = parse("Times New Roman", "font-family");
        let expected = Some(PropertyValue::FontFamily(vec![Atom::from(
            "Times New Roman",
        )]));
        assert_eq!(got, expected);
    }

    #[test]
    fn font_weight_parse_integer() {
        assert_eq!(
            parse("400", "font-weight"),
            Some(PropertyValue::FontWeight(400))
        );
        assert_eq!(
            parse("700", "font-weight"),
            Some(PropertyValue::FontWeight(700))
        );
    }

    #[test]
    fn font_weight_rejects_keyword() {
        assert_eq!(parse("bold", "font-weight"), None);
        assert_eq!(parse("normal", "font-weight"), None);
    }

    #[test]
    fn unknown_property_returns_none() {
        assert_eq!(parse("100px", "margin"), None);
        assert_eq!(parse("red", "background-color"), None);
    }

    // ── Display (M1.4a、raikiri-spike-m1.22) ─────────────────────

    #[test]
    fn display_parse_block() {
        assert_eq!(
            parse("block", "display"),
            Some(PropertyValue::Display(DisplayValue::Block))
        );
    }

    #[test]
    fn display_parse_inline() {
        assert_eq!(
            parse("inline", "display"),
            Some(PropertyValue::Display(DisplayValue::Inline))
        );
    }

    #[test]
    fn display_rejects_unknown_ident() {
        // spec §M1.4a: block と inline 以外の値 (flex, grid, none, table, ...) は
        // M6+ 対応、現状は silent drop (None を返す)
        assert_eq!(parse("flex", "display"), None);
        assert_eq!(parse("grid", "display"), None);
        assert_eq!(parse("none", "display"), None);
        assert_eq!(parse("table", "display"), None);
    }

    #[test]
    fn display_rejects_non_ident() {
        assert_eq!(parse("16px", "display"), None);
        assert_eq!(parse("100", "display"), None);
    }

    #[test]
    fn display_is_case_insensitive() {
        // CSS spec: property value keyword は ASCII case-insensitive
        assert_eq!(
            parse("BLOCK", "display"),
            Some(PropertyValue::Display(DisplayValue::Block))
        );
        assert_eq!(
            parse("Inline", "display"),
            Some(PropertyValue::Display(DisplayValue::Inline))
        );
    }

    // ── counter-* (CSS Lists 3 §3、raikiri-spike-s85 M5 pre-work) ──

    // d9y.2: `PropertyValue::Counter*(Arc<Vec<..>>)` に wrap したため、
    // literal test 比較用に Arc<Vec<..>> を返す helper に切り替え
    // (d9y.1 content/string_set helper と同 pattern)。
    fn counter_pairs(pairs: &[(&str, i32)]) -> Arc<Vec<(SmolStr, i32)>> {
        Arc::new(
            pairs
                .iter()
                .map(|(name, value)| (SmolStr::new(name), *value))
                .collect(),
        )
    }

    #[test]
    fn counter_reset_single_name_defaults_to_zero() {
        // spec: reset の default は 0
        assert_eq!(
            parse("chapter", "counter-reset"),
            Some(PropertyValue::CounterReset(counter_pairs(&[(
                "chapter", 0
            )])))
        );
    }

    #[test]
    fn counter_reset_multiple_names_with_mixed_ints() {
        // 2 番目に integer が付く → 1 番目は default 0、2 番目は 3
        assert_eq!(
            parse("chapter section 3", "counter-reset"),
            Some(PropertyValue::CounterReset(counter_pairs(&[
                ("chapter", 0),
                ("section", 3)
            ])))
        );
    }

    #[test]
    fn counter_reset_none_returns_empty_vec() {
        // spec: `none` は空リストと同等 (top-level alternative)
        // d9y.2: empty case は shared Arc slot (`empty_counter_entries`) を使う。
        assert_eq!(
            parse("none", "counter-reset"),
            Some(PropertyValue::CounterReset(empty_counter_entries()))
        );
    }

    #[test]
    fn counter_reset_rejects_number_first() {
        // 先頭が number → ident が来るまで peel できず empty → None (drop)
        // spec §3: `<counter-name> = <custom-ident>` (数値は counter-name ではない)
        assert_eq!(parse("123 abc", "counter-reset"), None);
    }

    #[test]
    fn counter_increment_single_name_defaults_to_one() {
        // spec: increment の default は 1
        assert_eq!(
            parse("chapter", "counter-increment"),
            Some(PropertyValue::CounterIncrement(counter_pairs(&[(
                "chapter", 1
            )])))
        );
    }

    #[test]
    fn counter_increment_mixed_int_and_default() {
        // `chapter 2 section` → chapter=2、section=default(1)
        assert_eq!(
            parse("chapter 2 section", "counter-increment"),
            Some(PropertyValue::CounterIncrement(counter_pairs(&[
                ("chapter", 2),
                ("section", 1)
            ])))
        );
    }

    #[test]
    fn counter_increment_accepts_negative_integer() {
        // spec §3: <integer> — negative も valid (counter を decrement する用途)
        assert_eq!(
            parse("chapter -1", "counter-increment"),
            Some(PropertyValue::CounterIncrement(counter_pairs(&[(
                "chapter", -1
            )])))
        );
    }

    #[test]
    fn counter_increment_none_returns_empty_vec() {
        // d9y.2: empty case は shared Arc slot を使う。
        assert_eq!(
            parse("none", "counter-increment"),
            Some(PropertyValue::CounterIncrement(empty_counter_entries()))
        );
    }

    #[test]
    fn counter_set_defaults_to_zero() {
        // spec: set の default は 0
        assert_eq!(
            parse("page 5 note", "counter-set"),
            Some(PropertyValue::CounterSet(counter_pairs(&[
                ("page", 5),
                ("note", 0)
            ])))
        );
    }

    #[test]
    fn counter_set_none_returns_empty_vec() {
        // d9y.2: empty case は shared Arc slot を使う。
        assert_eq!(
            parse("none", "counter-set"),
            Some(PropertyValue::CounterSet(empty_counter_entries()))
        );
    }

    #[test]
    fn counter_reset_is_case_insensitive_on_none() {
        // CSS spec: keyword `none` は ASCII case-insensitive
        // d9y.2: empty case は shared Arc slot を使う。
        assert_eq!(
            parse("NONE", "counter-reset"),
            Some(PropertyValue::CounterReset(empty_counter_entries()))
        );
    }

    #[test]
    fn counter_reset_rejects_reserved_css_wide_keyword_as_name() {
        // spec §3: <counter-name> excludes CSS-wide keywords + `default`。
        // 先頭 ident が `inherit` → try_parse rewind で empty result → None。
        assert_eq!(parse("inherit", "counter-reset"), None);
        assert_eq!(parse("initial", "counter-reset"), None);
        assert_eq!(parse("unset", "counter-reset"), None);
        assert_eq!(parse("revert", "counter-reset"), None);
        assert_eq!(parse("default", "counter-reset"), None);
    }

    #[test]
    fn counter_reset_accepts_negative_integer() {
        // CSS Values 3 §5.1: <integer> は負値を含む。
        // increment だけでなく reset / set も同一 grammar。
        assert_eq!(
            parse("chapter -5", "counter-reset"),
            Some(PropertyValue::CounterReset(counter_pairs(&[(
                "chapter", -5
            )])))
        );
    }

    #[test]
    fn counter_set_accepts_negative_integer() {
        // 同上 (parity with reset/increment negative-integer coverage)。
        assert_eq!(
            parse("page -3", "counter-set"),
            Some(PropertyValue::CounterSet(counter_pairs(&[("page", -3)])))
        );
    }

    // ── content property (CSS Content 3 §2、raikiri-spike-m5.1) ──
    //
    // task 9 verification items = spec-derived (9y9(a))。task 記述の
    // `raikiri_traits::ContentValueItem` は下流 (raikiri-dom) mapping 先。
    // raikiri-style は raikiri-traits に依存しない leaf crate (94e/3ps Phase B)
    // のため、s85 counter-* precedent に倣い local `ContentComponent` を emit
    // する (原則 1: 前例主義)。Symbol → SmolStr、Url → String へ substitution。

    fn content_items(source: &str) -> Vec<ContentComponent> {
        match parse(source, "content") {
            // d9y.1: PropertyValue::Content(Arc<Vec<..>>) を expose するため
            // (*v).clone() で Vec を deref-clone。tests は既存 shape のまま検証。
            Some(PropertyValue::Content(v)) => (*v).clone(),
            other => panic!("expected PropertyValue::Content, got {other:?}"),
        }
    }

    #[test]
    fn content_parse_string_function() {
        // Verification 1: content: string(my_str)
        // → ContentComponent::String { name: "my_str", fetch: default (First) }
        let items = content_items("string(my_str)");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::String {
                name: SmolStr::new("my_str"),
                fetch: StringFetchMode::First,
            }
        );
    }

    #[test]
    fn content_parse_counter_function() {
        // Verification 2: content: counter(chapter)
        // → ContentComponent::Counter { name: "chapter", style: default (Decimal) }
        let items = content_items("counter(chapter)");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn content_parse_counters_function() {
        // Verification 3: content: counters(section, ".")
        // → ContentComponent::Counters { name, separator: ".", style: default }
        let items = content_items(r#"counters(section, ".")"#);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::Counters {
                name: SmolStr::new("section"),
                separator: String::from("."),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn content_parse_target_counter_function() {
        // Verification 4: content: target-counter(url("#anchor"), page)
        // → ContentComponent::TargetCounter { url: "#anchor", name: "page", style: default }
        let items = content_items(r##"target-counter(url("#anchor"), page)"##);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::TargetCounter {
                url: String::from("#anchor"),
                name: SmolStr::new("page"),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn content_parse_target_counters_function() {
        // Verification 5: content: target-counters(url("#anchor"), section, ".")
        // → ContentComponent::TargetCounters { url, name, separator, style: default }
        let items = content_items(r##"target-counters(url("#anchor"), section, ".")"##);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::TargetCounters {
                url: String::from("#anchor"),
                name: SmolStr::new("section"),
                separator: String::from("."),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn content_parse_target_text_first_letter() {
        // Verification 6: content: target-text(url("#anchor"), first-letter)
        // → ContentComponent::TargetText { url, part: ContentPart::FirstLetter }
        //
        // NB: task description の "content-first-letter" は spec (§2.6.3
        // `[ content | before | after | first-letter ]?`) と食い違うため、
        // spec-correct な `first-letter` を採用 (reviewer:spec 9y9(c) の
        // task-own-claim verification で task 側の書き振りが訂正対象)。
        let items = content_items(r##"target-text(url("#anchor"), first-letter)"##);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::TargetText {
                url: String::from("#anchor"),
                part: ContentPart::FirstLetter,
            }
        );
    }

    #[test]
    fn content_parse_attr_function() {
        // Verification 7: content: attr(href) → ContentComponent::Attr { name: "href" }
        let items = content_items("attr(href)");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ContentComponent::Attr {
                name: SmolStr::new("href"),
            }
        );
    }

    #[test]
    fn content_parse_literal_string() {
        // Verification 8: content: "hello" → ContentComponent::Literal("hello")
        let items = content_items(r#""hello""#);
        assert_eq!(
            items,
            vec![ContentComponent::Literal(SmolStr::new("hello"))]
        );
    }

    #[test]
    fn content_parse_mixed_sequence_preserves_order() {
        // Verification 9: content: "Chapter " counter(chapter) ": " string(chapter_title)
        // → 4-item Vec in order
        let items = content_items(r#""Chapter " counter(chapter) ": " string(chapter_title)"#);
        assert_eq!(items.len(), 4, "expected 4 items, got {items:?}");
        assert_eq!(
            items[0],
            ContentComponent::Literal(SmolStr::new("Chapter "))
        );
        assert_eq!(
            items[1],
            ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
        assert_eq!(items[2], ContentComponent::Literal(SmolStr::new(": ")));
        assert_eq!(
            items[3],
            ContentComponent::String {
                name: SmolStr::new("chapter_title"),
                fetch: StringFetchMode::First,
            }
        );
    }

    // ── content property edge cases (spec-derived、guard rails) ──

    #[test]
    fn content_normal_returns_empty_list() {
        // spec §2.1: `normal` は「content が明示されない場合と同じ」= 空 list として保持。
        // pseudo-element generation 判断は下流で行う。
        assert_eq!(
            parse("normal", "content"),
            Some(PropertyValue::Content(empty_content_list()))
        );
    }

    #[test]
    fn content_none_returns_empty_list() {
        // spec §2.1: `none` — 本 crate では `normal` と同じく空 list に落とす。
        assert_eq!(
            parse("none", "content"),
            Some(PropertyValue::Content(empty_content_list()))
        );
    }

    #[test]
    fn content_string_with_fetch_last_keyword() {
        // spec §2.7.2 の string() 第 2 引数 keyword を全て受理することを smoke で pin。
        let items = content_items("string(head, last)");
        assert_eq!(
            items,
            vec![ContentComponent::String {
                name: SmolStr::new("head"),
                fetch: StringFetchMode::Last,
            }]
        );
    }

    #[test]
    fn content_target_text_default_part_is_content() {
        // spec §2.6.3: 第 2 引数省略時 default は `content`。
        let items = content_items(r##"target-text(url("#a"))"##);
        assert_eq!(
            items,
            vec![ContentComponent::TargetText {
                url: String::from("#a"),
                part: ContentPart::Content,
            }]
        );
    }

    #[test]
    fn content_counter_rejects_none_name() {
        // spec CSS Lists 3 §4 <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>:
        // "A <counter-name> name cannot match the keyword `none`; such an identifier
        // is invalid as a <counter-name>". §4.7 counter() の first argument が
        // <counter-name> production のため `counter(none)` は declaration drop。
        // counter-reset/increment/set (property.rs 既存) と一貫、Chrome/FF と一致。
        // (raikiri-spike-afv — codex final for m5.1)
        assert_eq!(parse("counter(none)", "content"), None);
    }

    #[test]
    fn content_counters_rejects_none_name() {
        // spec CSS Lists 3 §4 / §4.7: counters() の first argument も
        // <counter-name> production、`none` は invalid。
        // (raikiri-spike-afv — codex final for m5.1)
        assert_eq!(parse(r#"counters(none, ".")"#, "content"), None);
    }

    #[test]
    fn content_counter_with_named_style_preserves_ident() {
        // spec CSS Lists 3 §4.7: 第 2 引数 `<counter-style>` は decimal 以外の
        // named style も受ける。下流 (raikiri-dom) が解釈するため raw ident 保持。
        let items = content_items("counter(chapter, upper-alpha)");
        assert_eq!(
            items,
            vec![ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Named(SmolStr::new("upper-alpha")),
            }]
        );
    }

    #[test]
    fn content_rejects_unknown_function() {
        // 未知 function は認識できず、items 開始 token として peel 失敗。
        // 先頭 token が unknown function だと empty items → None (drop)。
        assert_eq!(parse("bogus(x)", "content"), None);
    }

    #[test]
    fn content_case_insensitive_function_name() {
        // spec: function name は ASCII case-insensitive。
        let items = content_items("COUNTER(chapter)");
        assert_eq!(
            items,
            vec![ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }]
        );
    }

    #[test]
    fn content_key_maps_to_content_property_key() {
        // PropertyValue::Content → PropertyKey::Content (cascade winner 選択の
        // discriminant integrity、既存 sibling counter-* と同じ pattern)。
        let cv = PropertyValue::Content(empty_content_list());
        assert_eq!(cv.key(), PropertyKey::Content);
    }

    // ── parse_optional_counter_style trailing-comma strict reject (raikiri-spike-zik) ──
    //
    // CSS Lists 3 §4.7 `counter(<counter-name>, <counter-style>?)` /
    // CSS Content 3 §2.6.1-2 `target-counter()` / `target-counters()` は
    // `<counter-style>?` — `,` を先行させる時は ident 必須。trailing-comma
    // (`counter(chapter,)` 等) は spec-invalid → declaration ごと drop すべき。
    // sibling `parse_string_fetch` / `parse_content_part` は既に strict `?`
    // propagation、`parse_optional_counter_style` のみ silent Decimal fallback
    // していた regression を pin する。

    #[test]
    fn content_counter_rejects_trailing_comma() {
        // `counter(chapter,)` — comma 消費後に ident 不在。spec-invalid、
        // declaration drop = None (Chrome/Firefox と同挙動)。
        assert_eq!(parse("counter(chapter,)", "content"), None);
    }

    #[test]
    fn content_counters_rejects_trailing_comma() {
        // `counters(chapter, ".",)` — separator string 後の trailing comma。
        assert_eq!(parse(r#"counters(chapter, ".",)"#, "content"), None);
    }

    #[test]
    fn content_target_counter_rejects_trailing_comma() {
        // `target-counter(url("#a"), page,)` — name 後の trailing comma。
        // target-counter/target-counters は parse_optional_counter_style を
        // 経由 (parse_target_counter_fn / parse_target_counters_fn) するため同じ pattern で drop。
        assert_eq!(
            parse(r##"target-counter(url("#a"), page,)"##, "content"),
            None
        );
    }

    #[test]
    fn content_target_counters_rejects_trailing_comma() {
        // `target-counters(url("#a"), section, ".",)` — separator 後の trailing。
        assert_eq!(
            parse(r##"target-counters(url("#a"), section, ".",)"##, "content"),
            None
        );
    }

    #[test]
    fn content_string_rejects_trailing_comma() {
        // 対照実験 (現行 strict の維持確認): `string(foo,)` は
        // `parse_string_fetch` が `?` 経由で伝播、既に None。
        assert_eq!(parse("string(foo,)", "content"), None);
    }

    #[test]
    fn content_target_text_rejects_trailing_comma() {
        // 対照実験: `target-text(url("#a"),)` は `parse_content_part` が
        // `?` 経由で伝播、既に None。
        assert_eq!(parse(r##"target-text(url("#a"),)"##, "content"), None);
    }

    #[test]
    fn content_counter_accepts_bare_default() {
        // `counter(chapter)` — trailing comma 無しの正常 case、Decimal default
        // で Some を返す (silent fallback を strict にしても正常 path は変えない)。
        let items = content_items("counter(chapter)");
        assert_eq!(
            items,
            vec![ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }]
        );
    }

    // ── string-set (CSS GCPM 3 §3.1、raikiri-spike-m5.3) ──
    //
    // grammar: `none | [ <custom-ident> <content-list> ]#` — 各 entry は
    // (name, content-list) pair、m5.1 の `ContentComponent` + `parse_content_list_items`
    // を reuse。task description の "4-item Vec" は entry name の分を content 側に
    // 誤って含めた結果、実態は 3-item (name は tuple の第 1 要素)。

    fn string_set_entries(source: &str) -> Vec<(SmolStr, Vec<ContentComponent>)> {
        match parse(source, "string-set") {
            // d9y.1: PropertyValue::StringSet(Arc<Vec<..>>)、content_items と同 pattern。
            Some(PropertyValue::StringSet(v)) => (*v).clone(),
            other => panic!("expected PropertyValue::StringSet, got {other:?}"),
        }
    }

    #[test]
    fn string_set_single_entry_with_literal() {
        // Verification 1: string-set: my_str "hello"
        // → [(SmolStr("my_str"), [Literal("hello")])]
        let entries = string_set_entries(r#"my_str "hello""#);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, SmolStr::new("my_str"));
        assert_eq!(
            entries[0].1,
            vec![ContentComponent::Literal(SmolStr::new("hello"))]
        );
    }

    #[test]
    fn string_set_mixed_content_list_preserves_order() {
        // Verification 2 (raikiri-spike-6s1 で adjust):
        // string-set: chapter_title counter(chapter) ": " attr(title)
        //
        // 先頭 `chapter_title` は entry name (tuple 第 1 要素)。content-list は
        // 残りの `counter(chapter) ": " attr(title)` = 3 items。
        //
        // NB: m5.3 の原 test は末尾に `string(chapter_title)` を置いていたが、
        // GCPM 3 §1.1.1 narrow list は `string()` function を含まないため
        // raikiri-spike-6s1 で `attr()` (GCPM narrow list の 5 alt の 1 つ) に
        // swap。テストの主意 (mixed content-list の order 保持) は保つ。
        let entries = string_set_entries(r#"chapter_title counter(chapter) ": " attr(title)"#);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, SmolStr::new("chapter_title"));
        assert_eq!(entries[0].1.len(), 3);
        assert_eq!(
            entries[0].1[0],
            ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
        assert_eq!(
            entries[0].1[1],
            ContentComponent::Literal(SmolStr::new(": "))
        );
        assert_eq!(
            entries[0].1[2],
            ContentComponent::Attr {
                name: SmolStr::new("title"),
            }
        );
    }

    #[test]
    fn string_set_comma_separated_multi_entry() {
        // Verification 3: string-set: a "x", b "y" → 2 entries
        let entries = string_set_entries(r#"a "x", b "y""#);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, SmolStr::new("a"));
        assert_eq!(
            entries[0].1,
            vec![ContentComponent::Literal(SmolStr::new("x"))]
        );
        assert_eq!(entries[1].0, SmolStr::new("b"));
        assert_eq!(
            entries[1].1,
            vec![ContentComponent::Literal(SmolStr::new("y"))]
        );
    }

    #[test]
    fn string_set_none_returns_empty_vec() {
        // spec §3.1: top-level `none` = empty list
        assert_eq!(
            parse("none", "string-set"),
            Some(PropertyValue::StringSet(empty_string_set_entries()))
        );
    }

    #[test]
    fn string_set_rejects_reserved_css_wide_keyword_as_name() {
        // spec §3.1 + CSS Values 4 §3.6: `<custom-ident>` は CSS-wide keyword 除外。
        // 先頭 ident が `inherit` → try_parse rewind で entries 空 → None。
        //
        // NB: 先頭が `none` の場合は top-level alternative の branch を先に
        // 通って `Some(empty)` を返し、leftover は下流 `expect_exhausted` で
        // declaration drop (rule.rs level)。この case は parse_value 単体では
        // 検証しない — advisor calibration。
        assert_eq!(parse("inherit \"x\"", "string-set"), None);
        assert_eq!(parse("initial \"x\"", "string-set"), None);
        assert_eq!(parse("unset \"x\"", "string-set"), None);
        assert_eq!(parse("revert \"x\"", "string-set"), None);
        assert_eq!(parse("default \"x\"", "string-set"), None);
    }

    #[test]
    fn string_set_rejects_name_without_content_list() {
        // spec §3.1 + Content 3 §2: `<content-list>` は 1+ items 必須。
        // name だけで items 0 → declaration drop (None)。
        assert_eq!(parse("my_str", "string-set"), None);
    }

    #[test]
    fn string_set_is_case_insensitive_on_none() {
        // CSS spec: keyword `none` は ASCII case-insensitive
        assert_eq!(
            parse("NONE", "string-set"),
            Some(PropertyValue::StringSet(empty_string_set_entries()))
        );
    }

    #[test]
    fn string_set_key_maps_to_string_set_property_key() {
        // PropertyValue::StringSet → PropertyKey::StringSet (cascade winner 選択の
        // discriminant integrity、既存 sibling counter-* / content と同じ pattern)。
        let v = PropertyValue::StringSet(empty_string_set_entries());
        assert_eq!(v.key(), PropertyKey::StringSet);
    }

    // ── string-set trailing-comma strict reject (raikiri-spike-1ll) ──
    //
    // `#` (comma-separated multiplier、CSS Values 4 §3.3
    // <https://www.w3.org/TR/css-values-4/#mult-comma>) は trailing comma を
    // 許容しない。GCPM 3 §3.1 <string-set-value> = `[ <custom-ident>
    // <content-list> ]#` は entry 間 comma 必須 + trailing comma 禁止。
    //
    // m5.3 の初期実装は separator loop で `try_parse(expect_comma).is_err() {
    // break }` していたため、trailing comma を silently 受理していた (comma を
    // consume 後 next iteration で name parse fail → break → 既存 entries を
    // Some で返す)。zik と同 principle の `.ok()?` propagation で strict 化。

    #[test]
    fn string_set_rejects_trailing_comma_single_entry() {
        // `string-set: a "x",` → trailing comma → declaration drop。
        // pre-fix は Some([(a, [Literal("x")])]) を silently 返していた。
        assert_eq!(parse(r#"a "x","#, "string-set"), None);
    }

    #[test]
    fn string_set_rejects_trailing_comma_two_entries() {
        // `string-set: a "x", b "y",` → trailing comma → declaration drop。
        // 内部 comma 1 個は valid separator、末尾 comma のみが `#` 違反。
        assert_eq!(parse(r#"a "x", b "y","#, "string-set"), None);
    }

    #[test]
    fn string_set_rejects_trailing_comma_three_entries() {
        // 3 entries + trailing comma — chain 越しの一貫 strict reject を pin。
        assert_eq!(parse(r#"a "x", b "y", c "z","#, "string-set"), None);
    }

    #[test]
    fn string_set_rejects_missing_entry_after_comma() {
        // `string-set: a "x", b` → comma 後 name は取れるが `<content-list>`
        // が 0 items (`parse_content_list_items` empty) → declaration drop。
        // trailing-comma 系とは reject 経路が異なる (items-empty) 独立 pin。
        assert_eq!(parse(r#"a "x", b"#, "string-set"), None);
    }

    #[test]
    fn string_set_accepts_missing_comma_single_leftover_entry() {
        // `string-set: a "x" b "y"` は separator comma 欠如。iter 1 で
        // (a, ["x"]) push 後、bottom expect_comma fail → break、leftover
        // `b "y"` は本 helper (parse_value 直呼び、caller expect_exhausted
        // 経由なし) では drop されず 1 entry の Some として観測される。
        // 実 caller (rule.rs) は expect_exhausted で declaration drop する
        // — 本 test は parse_string_set の break exit が Some (`.ok()?`
        // 経路と混同しない) であることを pin する目的、trailing-comma fix の
        // non-regression coverage。
        let entries = string_set_entries(r#"a "x" b "y""#);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, SmolStr::new("a"));
        assert_eq!(
            entries[0].1,
            vec![ContentComponent::Literal(SmolStr::new("x"))]
        );
    }

    // ── string-set narrow <content-list> gate (CSS GCPM 3 §1.1.1、raikiri-spike-6s1) ──
    //
    // GCPM 3 §1.1.1 L82 verbatim: <content-list> = [ <string> | <counter()> |
    // <counters()> | <content()> | <attr()> ]+ — CSS Content 3 §2 broad list を
    // string-set 用に narrower 再定義。`string()` (function、bare literal とは別)
    // および `target-counter()` / `target-counters()` / `target-text()` は
    // spec grammar に含まれず、`ContentListMode::GcpmStringSet` mode dispatch で
    // reject する (parse_content_list_items が 0 items → parse_string_set →
    // None → declaration drop、bd description の cascade shadow 例
    // `p.hi { string-set: title target-counter(url("#x"), page); }` の spec 準拠
    // 挙動 = .hi rule drop → parser layer で確認)。
    //
    // 一方 content property (CssContent3 mode) はこれら全てを引き続き受理する
    // (下の content_parse_* 系 pin test 群で non-regression 検証)。

    #[test]
    fn string_set_rejects_string_fn() {
        // GCPM 3 §1.1.1 L82 は `string()` function を narrow list から除外。
        // bare `<string>` literal (`"..."`) と混同しないよう function 側のみ reject。
        assert_eq!(parse("title string(x)", "string-set"), None);
    }

    #[test]
    fn string_set_rejects_target_counter_fn() {
        // GCPM 3 §1.1.1 L82 は `target-counter()` を narrow list から除外。
        // bd description の cascade shadow 主要例、declaration drop → cascade で
        // 先行の spec-valid rule が winner になる shape。
        assert_eq!(
            parse(r##"title target-counter(url("#a"), page)"##, "string-set"),
            None
        );
    }

    #[test]
    fn string_set_rejects_target_counters_fn() {
        // GCPM 3 §1.1.1 L82 は `target-counters()` を narrow list から除外。
        assert_eq!(
            parse(
                r##"title target-counters(url("#a"), section, ".")"##,
                "string-set"
            ),
            None
        );
    }

    #[test]
    fn string_set_rejects_target_text_fn() {
        // GCPM 3 §1.1.1 L82 は `target-text()` を narrow list から除外。
        assert_eq!(
            parse(r##"title target-text(url("#a"))"##, "string-set"),
            None
        );
    }

    // ── content() function (CSS GCPM 3 §1.1.1.1、raikiri-spike-5ri) ──
    //
    // grammar (spec verbatim, line 758 of TR/css-gcpm-3/):
    //   content() = content([text | before | after | first-letter])
    // 4 keyword、default `text`。GCPM 3 §1.1.1 の narrow `<content-list>` と
    // CSS Content 3 §2 の broad `<content-list>` の両方に含まれるため、string-set
    // および content property 双方の content-list 内で受理される
    // (raikiri-spike-6s1 で `ContentListMode` mode dispatch を導入した後も
    // `content()` arm は両 mode で unconditional accept)。
    //
    // pre-fix reproduction: `string-set: title content(text)` は m5.1/m5.3 で
    // silent drop していた (parse_content_function match arm 欠如 →
    // parse_content_list_items break → 0 items → parse_string_set None →
    // declaration drop)。arm 追加で Some を返すことを pin する。

    #[test]
    fn string_set_content_text_reproduces_pre_fix_drop() {
        // bd raikiri-spike-5ri description の主要 repro case:
        // pre-fix では declaration drop = None、post-fix では
        // (title, [Content{keyword: Text}]) を含む Some を返す。
        let entries = string_set_entries("title content(text)");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, SmolStr::new("title"));
        assert_eq!(
            entries[0].1,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            }]
        );
    }

    #[test]
    fn content_content_fn_explicit_text_keyword() {
        // §1.1.1.1: `content(text)` は element の string value (default と同義だが
        // 明示的 keyword 保持で downstream の分岐余地を残す)。
        let items = content_items("content(text)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            }]
        );
    }

    #[test]
    fn content_content_fn_default_keyword_on_empty_parens() {
        // §1.1.1.1 の spec 例 `h2 { string-set: heading content() }` — bare
        // `content()` は default `text` を意味する。
        let items = content_items("content()");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            }]
        );
    }

    #[test]
    fn content_content_fn_before_keyword() {
        // §1.1.1.1 の spec 例 `h1 { string-set: header content(before) ':' content(text); }`
        // で使われる `before` keyword。
        let items = content_items("content(before)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Before,
            }]
        );
    }

    #[test]
    fn content_content_fn_after_keyword() {
        let items = content_items("content(after)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::After,
            }]
        );
    }

    #[test]
    fn content_content_fn_first_letter_keyword() {
        let items = content_items("content(first-letter)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::FirstLetter,
            }]
        );
    }

    #[test]
    fn content_content_fn_rejects_unknown_keyword() {
        // §1.1.1.1 の grammar は `[text | before | after | first-letter]` の 4
        // alternative のみ。それ以外の ident (spec 上存在しない `marker` 等) は
        // parse_content_text_keyword が None を返し、上位伝播で
        // parse_content_list_items が break、declaration drop = None。
        // (`marker` は list-item pseudo に関する別 concept、content() には出現しない)
        assert_eq!(parse("content(marker)", "content"), None);
        assert_eq!(parse("content(bogus)", "content"), None);
    }

    #[test]
    fn content_content_fn_rejects_target_text_keyword() {
        // §1.1.1.1 は `text` alternative を持つ (target-text() §2.6.3 は `content`)。
        // spec spelling divergence — `content(content)` は spec-invalid、reject。
        // 混同 (sibling ContentPart 再利用) を防ぐ regression pin。
        assert_eq!(parse("content(content)", "content"), None);
    }

    #[test]
    fn content_content_fn_case_insensitive_keyword() {
        // spec 慣行: keyword は ASCII case-insensitive。
        let items = content_items("content(TEXT)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            }]
        );
    }

    #[test]
    fn content_content_fn_case_insensitive_function_name() {
        // parse_content_function は既存 arm と同じく ASCII case-insensitive dispatch。
        let items = content_items("CONTENT(before)");
        assert_eq!(
            items,
            vec![ContentComponent::Content {
                keyword: ContentTextKeyword::Before,
            }]
        );
    }

    #[test]
    fn content_content_fn_rejects_extra_argument() {
        // grammar は single-argument。余剰 token は
        // parse_nested_block 内 parse_entirely が拒否し declaration drop。
        assert_eq!(parse("content(text, extra)", "content"), None);
        assert_eq!(parse("content(text before)", "content"), None);
    }

    #[test]
    fn string_set_content_fn_mixed_with_other_items() {
        // §1.1.1.1 の spec 例:
        //   h1 { string-set: header content(before) ':' content(text); }
        // → (header, [Content{Before}, Literal(":"), Content{Text}]) 3 items。
        let entries = string_set_entries(r#"header content(before) ":" content(text)"#);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, SmolStr::new("header"));
        assert_eq!(
            entries[0].1,
            vec![
                ContentComponent::Content {
                    keyword: ContentTextKeyword::Before,
                },
                ContentComponent::Literal(SmolStr::new(":")),
                ContentComponent::Content {
                    keyword: ContentTextKeyword::Text,
                },
            ]
        );
    }

    // ── position: running() (CSS GCPM 3 §1.2.1、raikiri-spike-m5.4) ──
    //
    // Verification items 1-6 は task description 由来 (bd raikiri-spike-m5.4)、
    // canonical shape は bd raikiri-spike-376 amended。sibling は s85 (counter)
    // / m5.1 (content) / m5.3 (string-set) の SmolStr wire-through pattern。

    #[test]
    fn position_parse_running_header() {
        // Verification 1: position: running(header)
        // → PropertyValue::Position(PositionValue::Running("header"))
        assert_eq!(
            parse("running(header)", "position"),
            Some(PropertyValue::Position(PositionValue::Running(
                SmolStr::new("header")
            )))
        );
    }

    #[test]
    fn position_parse_running_footer() {
        // Verification 2: 別 name の smoke — SmolStr::new が生きていることを pin。
        assert_eq!(
            parse("running(footer)", "position"),
            Some(PropertyValue::Position(PositionValue::Running(
                SmolStr::new("footer")
            )))
        );
    }

    #[test]
    fn position_parse_static() {
        // Verification 5 baseline: position: static → PositionValue::Static。
        // apply_value は no-op、running_templates は inherit_from の initial
        // (空 Vec) が残る = cascade winner が earlier running(...) を suppress する
        // ID 用途 (cascade.rs 側の `static_position_wins_over_running` で検証)。
        assert_eq!(
            parse("static", "position"),
            Some(PropertyValue::Position(PositionValue::Static))
        );
    }

    #[test]
    fn position_running_case_insensitive_function_name() {
        // Verification 4: function name は ASCII case-insensitive (CSS spec 慣行)、
        // custom-ident は case-preserving。
        assert_eq!(
            parse("RUNNING(header)", "position"),
            Some(PropertyValue::Position(PositionValue::Running(
                SmolStr::new("header")
            )))
        );
    }

    #[test]
    fn position_running_rejects_none_custom_ident() {
        // Verification 6: `running(none)` reject。`none` は position property
        // spec-defined keyword ではないが、runtime resolve で `element(none)` 参照が
        // silent match するのを避けるため custom-ident としても弾く (m5.3 string-set
        // と同じ規約、reviewer:spec interpretation point)。
        assert_eq!(parse("running(none)", "position"), None);
    }

    #[test]
    fn position_running_rejects_reserved_css_wide_keyword() {
        // spec CSS Values 4 §3.6: <custom-ident> は CSS-wide keyword + `default`
        // 除外。position: running(inherit) 等は declaration drop。
        assert_eq!(parse("running(inherit)", "position"), None);
        assert_eq!(parse("running(initial)", "position"), None);
        assert_eq!(parse("running(unset)", "position"), None);
        assert_eq!(parse("running(revert)", "position"), None);
        assert_eq!(parse("running(default)", "position"), None);
    }

    #[test]
    fn position_rejects_missing_custom_ident() {
        // spec §1.2.1: `running() = running( <custom-ident> )` — argument 必須。
        // 空 argument は malformed、declaration drop。
        assert_eq!(parse("running()", "position"), None);
    }

    #[test]
    fn position_rejects_out_of_scope_keywords() {
        // M5+ scope: relative / absolute / fixed / sticky は本 crate では
        // 認識せず None を返す (spec-correct: invalid → drop)。
        assert_eq!(parse("relative", "position"), None);
        assert_eq!(parse("absolute", "position"), None);
        assert_eq!(parse("fixed", "position"), None);
        assert_eq!(parse("sticky", "position"), None);
    }

    #[test]
    fn position_rejects_running_with_extra_arg() {
        // `running(a, b)` — parse_nested_block が parse_entirely 経由で
        // 余剰 token を検知し、declaration drop になる。
        assert_eq!(parse("running(a, b)", "position"), None);
    }

    // ── parse_length_value helper (raikiri-spike-0vv.3) ────────────────────
    //
    // helper 単体を叩く共通 fixture — property dispatcher (`parse_value`) を経由せず
    // 5 unit sample (`px` / `em` / `rem` / `%` / `pt`) の parse を直接 verify する。
    // `parse_font_size` 経由 test は上流に既存 (`font_size_parse_px` 等)、そちらは
    // px-only post-filter を verify するので分離する。

    fn parse_length(source: &str, allow_percentage: bool) -> Option<Length> {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        parse_length_value(&mut parser, allow_percentage)
    }

    #[test]
    fn parse_length_value_accepts_px() {
        assert_eq!(parse_length("10px", false), Some(Length::Px(10.0)));
        // length-percentage mode でも px 受理 (mode 非依存)。
        assert_eq!(parse_length("10px", true), Some(Length::Px(10.0)));
    }

    #[test]
    fn parse_length_value_accepts_em() {
        // CSS Values 4 §6.1.1 em (https://www.w3.org/TR/css-values-4/#em):
        // authored `1.2em` を Length::Em(1.2) にそのまま保持 (resolve は下流責務)。
        assert_eq!(parse_length("1.2em", false), Some(Length::Em(1.2)));
    }

    #[test]
    fn parse_length_value_accepts_rem() {
        // CSS Values 4 §6.1.1 rem (https://www.w3.org/TR/css-values-4/#rem):
        // root element の font-size 基準、authored value を Length::Rem に格納。
        assert_eq!(parse_length("1rem", false), Some(Length::Rem(1.0)));
    }

    #[test]
    fn parse_length_value_accepts_pt() {
        // CSS Values 4 §6.2 absolute lengths (https://www.w3.org/TR/css-values-4/#absolute-lengths):
        // 1pt = 1/72 in, 1in = 96px、resolve 側で 12pt → 16px 相当に変換。
        assert_eq!(parse_length("12pt", false), Some(Length::Pt(12.0)));
    }

    #[test]
    fn parse_length_value_accepts_percentage_when_allowed() {
        // CSS Values 4 §5.5 (https://www.w3.org/TR/css-values-4/#percentages):
        // `<length-percentage>` mode でのみ受理。cssparser `unit_value = 0.5` を
        // × 100.0 で authored `50` に戻して Length::Percent(50.0) に格納。
        assert_eq!(parse_length("50%", true), Some(Length::Percent(50.0)));
    }

    #[test]
    fn parse_length_value_rejects_percentage_in_length_only_mode() {
        // `<length>` mode (font-size 等) では `%` は grammar 外、None を返す。
        assert_eq!(parse_length("50%", false), None);
    }

    #[test]
    fn parse_length_value_rejects_unsupported_unit() {
        // (b) milestone subset: `vw` / `ch` / `cm` / `in` / `Q` 等は本 helper で silent drop。
        assert_eq!(parse_length("10vw", false), None);
        assert_eq!(parse_length("10ch", true), None);
        assert_eq!(parse_length("1in", false), None);
    }

    #[test]
    fn parse_length_value_rejects_unitless_zero() {
        // 現行 behavior: unitless zero は Dimension token にならず (Number token)、
        // 本 helper の Dimension arm に落ちず None。既存 `parse_font_size` 挙動と一致。
        assert_eq!(parse_length("0", false), None);
        assert_eq!(parse_length("0", true), None);
    }

    #[test]
    fn parse_length_value_rejects_non_numeric_token() {
        assert_eq!(parse_length("medium", false), None);
        assert_eq!(parse_length("", false), None);
    }

    #[test]
    fn parse_length_value_preserves_negative_sign() {
        // helper は sign check を行わない — property ごとに要件が異なるため
        // (font-size は non-negative post-filter、margin は negative 許容)。
        assert_eq!(parse_length("-5px", false), Some(Length::Px(-5.0)));
        assert_eq!(parse_length("-1em", false), Some(Length::Em(-1.0)));
    }

    #[test]
    fn parse_length_value_unit_dispatch_case_insensitive() {
        // CSS spec: unit identifier は ASCII case-insensitive。
        assert_eq!(parse_length("10PX", false), Some(Length::Px(10.0)));
        assert_eq!(parse_length("1.5EM", false), Some(Length::Em(1.5)));
        assert_eq!(parse_length("2Rem", false), Some(Length::Rem(2.0)));
        assert_eq!(parse_length("14Pt", false), Some(Length::Pt(14.0)));
    }

    #[test]
    fn position_key_maps_to_position_property_key() {
        // PropertyValue::Position → PropertyKey::Position (cascade winner 選択の
        // discriminant integrity、既存 sibling counter-* / content / string-set と
        // 同じ pattern)。
        let v = PropertyValue::Position(PositionValue::Static);
        assert_eq!(v.key(), PropertyKey::Position);
        let v = PropertyValue::Position(PositionValue::Running(SmolStr::new("hdr")));
        assert_eq!(v.key(), PropertyKey::Position);
    }
}
