//! CSS property value 型と per-property parser。
//!
//! M1.4 では color / font-family / font-size / font-weight の 4 property のみ。
//! 認識できない property name / invalid value は `parse_value` が `None` を返す
//! (spec 準拠の silent drop、caller である rule.rs で declaration ごと drop)。
//!
//! `parse_value` は rule.rs の `DeclParser::parse_value` から呼ばれる。

use cssparser::color::{clamp_unit_f32, parse_hash_color, parse_named_color};
use cssparser::{ParseError, Parser, Token};
use smol_str::SmolStr;

use crate::Atom;

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

/// CSS length。M1.4 では pixel (`<length>` = px リテラル) のみ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// Absolute pixel length。
    Px(f32),
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
///
/// URL は raw `String` として保持 (raikiri-style は `url` crate に依存しない —
/// runtime resolve 段で `url::Url` へ parse する consumer 責務)。
///
/// (raikiri-spike-m5.1)
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentComponent {
    /// `<string>` bare literal (`content: "hello"`)。
    Literal(String),
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
    /// `running(<custom-ident>)`。<custom-ident> は case-preserved の smol str。
    Running(SmolStr),
}

/// M1.4 でサポートする property の resolved value。
///
/// 認識できない property (`background-color` / `margin` / ...) や invalid value
/// (`font-size: 1em` — em 未対応) は parser 段で `None` に落として rule から
/// silently 除外される。
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
    CounterReset(Vec<(SmolStr, i32)>),
    /// `counter-increment: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited、initial: empty list (CSS Lists 3 §3)。
    /// missing integer は 1 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    CounterIncrement(Vec<(SmolStr, i32)>),
    /// `counter-set: [ <counter-name> <integer>? ]+ | none` —
    /// non-inherited、initial: empty list (CSS Lists 3 §3)。
    /// missing integer は 0 に default (spec default)。M5 pre-work (raikiri-spike-s85)。
    CounterSet(Vec<(SmolStr, i32)>),
    /// `content: normal | none | <content-list>` — non-inherited、initial:
    /// empty list (spec の `normal` / `none` を空 list として扱う、pseudo-element
    /// 生成判断は下流 layer)。M5 gcpm-directive-emit static-side
    /// (raikiri-spike-m5.1)、CSS Content 3 §2.1
    /// <https://www.w3.org/TR/css-content-3/#content-property>。
    Content(Vec<ContentComponent>),
    /// `string-set: none | [ <custom-ident> <content-list> ]#` — non-inherited、
    /// initial: empty list。各 entry は `(name, content-list)` pair。
    /// CSS GCPM 3 §3.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>、
    /// `<content-list>` は CSS Content 3 §2 (m5.1 で parser 実装済)。
    /// 名前解決と runtime string() 参照は下流 (raikiri-dom) 責務。
    StringSet(Vec<(SmolStr, Vec<ContentComponent>)>),
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
        "counter-reset" => parse_counter_property(input, 0).map(PropertyValue::CounterReset),
        "counter-increment" => {
            parse_counter_property(input, 1).map(PropertyValue::CounterIncrement)
        }
        "counter-set" => parse_counter_property(input, 0).map(PropertyValue::CounterSet),
        // CSS Content 3 §2.1 content property (raikiri-spike-m5.1、M5 gcpm-directive-emit static side)
        "content" => parse_content(input).map(PropertyValue::Content),
        // CSS GCPM 3 §3.1 string-set (raikiri-spike-m5.3、M5 static-side β)
        "string-set" => parse_string_set(input).map(PropertyValue::StringSet),
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

fn parse_font_size(input: &mut Parser<'_, '_>) -> Option<Length> {
    // <length> = px リテラルのみ (m1.4 scope)。
    match input.next().ok()? {
        Token::Dimension { value, unit, .. }
            if unit.eq_ignore_ascii_case("px") && *value >= 0.0 =>
        {
            Some(Length::Px(*value))
        }
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

    let items = parse_content_list_items(input);
    if items.is_empty() { None } else { Some(items) }
}

/// `<content-list> = [ <string> | <counter> | <string()> | <attr()> | <target> ]+`
/// の items+ loop 部分 (CSS Content 3 §2)。
///
/// `<string>` literal と function token (`counter(...)` / `string(...)` /
/// `target-*()` / `attr(...)` 等) を順次 peel。認識できない token に当たった
/// 時点で break — 呼び出し側が leftover を検知して drop する。
///
/// `parse_content` (`content` property) と `parse_string_set` (`string-set`
/// property) が共有 (m5.1 で content 用に導入、m5.3 で string-set が reuse)。
fn parse_content_list_items(input: &mut Parser<'_, '_>) -> Vec<ContentComponent> {
    let mut items = Vec::new();
    loop {
        // bare `<string>` literal
        if let Ok(s) = input.try_parse(|i| i.expect_string_cloned()) {
            items.push(ContentComponent::Literal(s.as_ref().to_string()));
            continue;
        }
        // function
        let parsed = input.try_parse(|i| -> Result<ContentComponent, ParseError<'_, ()>> {
            let name = i.expect_function()?.clone();
            i.parse_nested_block(|inner| {
                parse_content_function(name.as_ref(), inner)
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
/// name が reserved の場合は `try_parse` の rewind で unconsumed に戻り loop を
/// 抜け、caller の `expect_exhausted` (rule.rs) が leftover token で declaration
/// を drop する。
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
        let name = match input.try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
            let ident = i.expect_ident()?.clone();
            if is_reserved_custom_ident(&ident) || ident.eq_ignore_ascii_case("none") {
                Err(i.new_custom_error(()))
            } else {
                Ok(SmolStr::new(ident.as_ref()))
            }
        }) {
            Ok(name) => name,
            Err(_) => break,
        };
        // <content-list> は 1+ items 必須。0 items → declaration drop。
        let items = parse_content_list_items(input);
        if items.is_empty() {
            return None;
        }
        entries.push((name, items));
        // 次 entry の separator: comma で継続、他 token で loop を抜ける
        // (caller `expect_exhausted` が leftover token を drop)。
        if input.try_parse(|i| i.expect_comma()).is_err() {
            break;
        }
    }

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Dispatch on function name (ASCII-case-insensitive、spec identifier 慣行)。
/// 未知の function name または引数 parse 失敗は `None` — caller の
/// `parse_nested_block` が custom error に変換する。
///
/// 各 `parse_*_fn` は自身では `expect_exhausted` を呼ばない —
/// [`parse_content`] 側の `parse_nested_block` が内部で
/// [`Parser::parse_entirely`] を経由し、closure 成功後の余剰 token を
/// exhaustion check で拒否する ([`parse_rgb_function`] と同じ規約)。
fn parse_content_function(name: &str, input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    match name.to_ascii_lowercase().as_str() {
        "string" => parse_string_fn(input),
        "counter" => parse_counter_fn(input),
        "counters" => parse_counters_fn(input),
        "attr" => parse_attr_fn(input),
        "target-counter" => parse_target_counter_fn(input),
        "target-counters" => parse_target_counters_fn(input),
        "target-text" => parse_target_text_fn(input),
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

    fn counter_pairs(pairs: &[(&str, i32)]) -> Vec<(SmolStr, i32)> {
        pairs
            .iter()
            .map(|(name, value)| (SmolStr::new(name), *value))
            .collect()
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
        assert_eq!(
            parse("none", "counter-reset"),
            Some(PropertyValue::CounterReset(Vec::new()))
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
        assert_eq!(
            parse("none", "counter-increment"),
            Some(PropertyValue::CounterIncrement(Vec::new()))
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
        assert_eq!(
            parse("none", "counter-set"),
            Some(PropertyValue::CounterSet(Vec::new()))
        );
    }

    #[test]
    fn counter_reset_is_case_insensitive_on_none() {
        // CSS spec: keyword `none` は ASCII case-insensitive
        assert_eq!(
            parse("NONE", "counter-reset"),
            Some(PropertyValue::CounterReset(Vec::new()))
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
            Some(PropertyValue::Content(v)) => v,
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
            vec![ContentComponent::Literal(String::from("hello"))]
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
            ContentComponent::Literal(String::from("Chapter "))
        );
        assert_eq!(
            items[1],
            ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
        assert_eq!(items[2], ContentComponent::Literal(String::from(": ")));
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
            Some(PropertyValue::Content(Vec::new()))
        );
    }

    #[test]
    fn content_none_returns_empty_list() {
        // spec §2.1: `none` — 本 crate では `normal` と同じく空 list に落とす。
        assert_eq!(
            parse("none", "content"),
            Some(PropertyValue::Content(Vec::new()))
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
        let cv = PropertyValue::Content(Vec::new());
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
            Some(PropertyValue::StringSet(v)) => v,
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
            vec![ContentComponent::Literal(String::from("hello"))]
        );
    }

    #[test]
    fn string_set_mixed_content_list_preserves_order() {
        // Verification 2 (corrected): string-set: chapter_title counter(chapter) ": " string(chapter_title)
        // 先頭 `chapter_title` は entry name (tuple 第 1 要素)。content-list は
        // 残りの `counter(chapter) ": " string(chapter_title)` = 3 items。
        let entries =
            string_set_entries(r#"chapter_title counter(chapter) ": " string(chapter_title)"#);
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
            ContentComponent::Literal(String::from(": "))
        );
        assert_eq!(
            entries[0].1[2],
            ContentComponent::String {
                name: SmolStr::new("chapter_title"),
                fetch: StringFetchMode::First,
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
            vec![ContentComponent::Literal(String::from("x"))]
        );
        assert_eq!(entries[1].0, SmolStr::new("b"));
        assert_eq!(
            entries[1].1,
            vec![ContentComponent::Literal(String::from("y"))]
        );
    }

    #[test]
    fn string_set_none_returns_empty_vec() {
        // spec §3.1: top-level `none` = empty list
        assert_eq!(
            parse("none", "string-set"),
            Some(PropertyValue::StringSet(Vec::new()))
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
            Some(PropertyValue::StringSet(Vec::new()))
        );
    }

    #[test]
    fn string_set_key_maps_to_string_set_property_key() {
        // PropertyValue::StringSet → PropertyKey::StringSet (cascade winner 選択の
        // discriminant integrity、既存 sibling counter-* / content と同じ pattern)。
        let v = PropertyValue::StringSet(Vec::new());
        assert_eq!(v.key(), PropertyKey::StringSet);
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
