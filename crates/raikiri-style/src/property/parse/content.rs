//! Generated-content property parsers: `content`, counters, `quotes`,
//! `string-set`, `page` and GCPM functions.

use cssparser::{ParseError, Parser, ParserInput};
use smol_str::SmolStr;

use crate::Atom;
use crate::property::types::*;

use super::common::*;

/// CSS Paged Media 3 §8.1 の `page` を parse する
/// (<https://www.w3.org/TR/css-page-3/#using-named-pages>)。
///
/// Grammar: `auto | <custom-ident>`。`auto` 単独、それ以外は CSS-wide keyword
/// (`inherit`/`initial`/`unset`/`revert`/`revert-layer`) と `default`
/// を除く単一 ident ([`PageValue`] doc 参照)。
/// ASCII case-insensitive で比較し、保持する値は
/// authored のまま (Atom は case-sensitive)。
pub(super) fn parse_page_value(input: &mut Parser<'_, '_>) -> Option<PageValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(PageValue::Auto);
    }
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default" => None,
        _ => Some(PageValue::Named(Atom::from(ident.as_ref()))),
    }
}

/// `counter-reset` / `counter-increment` / `counter-set` の value を parse する。
///
/// Grammar (CSS Lists 3 §4):
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
pub(super) fn parse_counter_property(
    input: &mut Parser<'_, '_>,
    default_number: i32,
) -> Option<Vec<(SmolStr, i32)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut result = Vec::new();
    loop {
        // reserved keyword を counter-name として受理しない (spec §4、`<custom-ident>`
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

/// `quotes` の value を parse する。
///
/// Grammar (legacy CSS2 §12.3.1 subset, [`PropertyValue::Quotes`] doc 参照):
///   `quotes = none | [ <string> <string> ]+`
///
/// `none` を top-level alternative として先に処理。以降は `<string>` を LL(1)
/// で 2 個ずつ pair にして peel する。
///
/// `parse_counter_property` (`<counter-name> <integer>?` — integer 省略時は
/// property-specific default で補える) と異なり、`<string> <string>` の pair
/// は片方が欠けた時点でその entry 自体が spec-invalid になる (grammar に
/// optional 要素が無い) — このため、1 個目の `<string>` を消費した後 2 個目が
/// 取れない (trailing unpaired `<string>`、またはその位置に `<string>` でない
/// token が来た) 場合は、途中まで蓄積した pair を捨てて即座に declaration
/// 全体を drop する (`None` を返す)。counter-* 系のように「ここまでの pair は
/// 残し、以降を caller の `expect_exhausted` (rule.rs) に委ねる」設計には
/// しない — 奇数個の `<string>` は `[ <string> <string> ]+` に決して一致しない
/// ため、削るべき「途中まで正しい prefix」自体が存在しない。
pub(super) fn parse_quotes_property(input: &mut Parser<'_, '_>) -> Option<Vec<(SmolStr, SmolStr)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut result = Vec::new();
    loop {
        let open = match input.try_parse(|i| i.expect_string_cloned()) {
            Ok(s) => SmolStr::new(s.as_ref()),
            Err(_) => break,
        };
        let close = match input.try_parse(|i| i.expect_string_cloned()) {
            Ok(s) => SmolStr::new(s.as_ref()),
            // trailing unpaired `<string>` — spec-invalid, reject the whole
            // declaration (see function doc).
            Err(_) => return None,
        };
        result.push((open, close));
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// `<counter-name>` = `<custom-ident>` の除外リスト (CSS Lists 3 §4 + CSS Values 4
/// §4.2 <https://www.w3.org/TR/css-values-4/#custom-idents>)。
///
/// CSS-wide keyword + `default` (Counter Styles L3) + `none` (top-level alternative)
/// を弾く。case-insensitive 比較。
///
/// これは CSS Values 4 §4.2 の permanent な spec 除外規定であり、**CSS-wide
/// keyword の実装状況とは無関係** — [`PropertyValue`] doc の「CSS-wide keyword」節
/// が説明する「property value としては未実装」claim
/// とは別の話なので混同しないこと。
pub(crate) fn is_reserved_counter_name(ident: &str) -> bool {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default" | "none"
    )
}

/// Parse a registered consumer property's resolved-text grammar from a complete
/// CSS value string.  The returned style components are an internal bridge;
/// the public consumer event converts them to an owned neutral `String`.
pub(crate) fn parse_consumer_text_value(source: &str) -> Option<Vec<ContentComponent>> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|parser| {
            let items = parse_content(parser).ok_or_else(|| parser.new_custom_error(()))?;
            Ok::<_, ParseError<'_, ()>>(items)
        })
        .ok()
}

/// `content: normal | none | <content-list>` を parse する
/// (CSS Content 3 §1 <https://www.w3.org/TR/css-content-3/#content-property>)。
///
/// `normal` / `none` は spec で意味が異なる (pseudo-element の生成/非生成)。
/// `normal` は初期値として空 `Vec` に留め、明示的な `none` は内部 sentinel
/// [`ContentComponent::None`] にして downstream の pseudo-element consumer が
/// marker を抑制できるようにする。
///
/// items+ loop は `<string>` literal と function token (`counter(...)` 等) を
/// 順次 peel する。認識できない token に当たった時点で loop を break、caller
/// の `expect_exhausted` (rule.rs) が leftover を検知して declaration ごと drop。
///
/// `alt text` (spec `... [/ <string>...]?`) は現状 scope 外、`/` 以降は
/// unconsumed のまま caller に返す (現状 rule.rs の `expect_exhausted` により
/// declaration drop、alt text 対応時に本関数を extend)。
pub(crate) fn parse_content(input: &mut Parser<'_, '_>) -> Option<Vec<ContentComponent>> {
    // `normal` / `none` = 空 list (top-level alternative)。
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(Vec::new());
    }
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(vec![ContentComponent::None]);
    }

    let items = parse_content_list_items(input, ContentListMode::CssContent3);
    if items.is_empty() { None } else { Some(items) }
}

/// `<content-list>` の items+ loop 部分。
///
/// `<string>` bare literal、`<image>` の `<url>` alternative、bare keyword
/// (`contents` / `<quote>`)、function token (`counter(...)` / `string(...)` /
/// `target-*()` / `attr(...)` / `content(...)` / `leader(...)`) を順次 peel。
/// 認識できない token に当たった時点で break — 呼び出し側が leftover を検知
/// して drop する。
///
/// `content` property (`parse_content`) と `string-set` property
/// (`parse_string_set`) の両方から call されるが、GCPM 3 §1.1.1 は string-set
/// 向けに CSS Content 3 §2 の broad list を narrower に再定義しているため、
/// `mode` パラメータで受理 alternative 集合を分岐する:
/// - [`ContentListMode::CssContent3`] — content property (CSS Content 3 §2
///   <https://www.w3.org/TR/css-content-3/#content-values>)。10 alt full set
///   を受理 (`<image>` / `contents` / `<quote>` /
///   `leader()` は後に追加)。
/// - [`ContentListMode::GcpmStringSet`] — string-set property (CSS GCPM 3
///   §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#content-list>)。`string()` /
///   `target-counter()` / `target-counters()` / `target-text()` / `<image>` /
///   `contents` / `<quote>` / `leader()` は GCPM 3 §1.1.1 L82 narrow grammar
///   に含まれず reject (bare `<string>` literal は両 mode で受理)。
///
/// bare literal 分岐は spec 上両 mode で共通 (どちらの `<content-list>` grammar
/// も `<string>` を top-level alternative に含む) なので mode 判定なし。他の
/// 分岐は各 branch 内で mode guard を掛ける ([`parse_content_function`] の
/// match arm guard と同じ pattern)。`Parser::try_parse` は失敗時に読んだ token
/// を必ず rewind するため、branch の試行順序は正しさに影響しない
/// (どの順で並べても等価)。
///
/// (導入後、mode-parameterize を経て `<image>` / `contents` / `<quote>` /
/// `leader()` を追加)
pub(super) fn parse_content_list_items(
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
        // `<image>` の `<url>` alternative — CSS Images 3 <url> production
        // (`url(...)` / `url("...")`) のみ (`<gradient>` は未実装として
        // defer、`ContentComponent::Image` doc 参照)。`expect_url` は bare
        // quoted string を受理しない (`<url> = <url()> | <src()>`) ので上の
        // literal 分岐との誤 overlap は無い。CssContent3 mode 限定
        // (GCPM 3 §1.1.1 L82 narrow list に `<image>` は含まれない)。
        if mode == ContentListMode::CssContent3
            && let Ok(url) = input.try_parse(|i| i.expect_url())
        {
            items.push(ContentComponent::Image {
                url: url.as_ref().to_string(),
            });
            continue;
        }
        // bare keyword alternative — `contents` / `<quote>` (function でも
        // `<string>` でもない ident-only alternative)。CssContent3 mode 限定。
        if mode == ContentListMode::CssContent3
            && let Ok(c) = input.try_parse(|i| -> Result<ContentComponent, ParseError<'_, ()>> {
                let ident = i.expect_ident()?.clone();
                parse_content_bare_keyword(ident.as_ref()).ok_or_else(|| i.new_custom_error(()))
            })
        {
            items.push(c);
            continue;
        }
        // function — mode に応じて `string()` / `target-*()` / `leader()` を
        // reject する判定は `parse_content_function` の match arm side で実施。
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

/// `contents` keyword と `<quote>` (`open-quote` / `close-quote` /
/// `no-open-quote` / `no-close-quote`) の bare-ident alternative をまとめて
/// 判定する ([`parse_content_list_items`] 専用 helper)。CSS Content 3 §2.3
/// <https://www.w3.org/TR/css-content-3/#element-content> および §2.4.2
/// <https://www.w3.org/TR/css-content-3/#quote-values>。
fn parse_content_bare_keyword(ident: &str) -> Option<ContentComponent> {
    match ident.to_ascii_lowercase().as_str() {
        "contents" => Some(ContentComponent::Contents),
        "open-quote" => Some(ContentComponent::Quote(QuoteKeyword::OpenQuote)),
        "close-quote" => Some(ContentComponent::Quote(QuoteKeyword::CloseQuote)),
        "no-open-quote" => Some(ContentComponent::Quote(QuoteKeyword::NoOpenQuote)),
        "no-close-quote" => Some(ContentComponent::Quote(QuoteKeyword::NoCloseQuote)),
        _ => None,
    }
}

/// `string-set: none | [ <custom-ident> <content-list> ]#` を parse する
/// (CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>)。
///
/// `none` を top-level alternative として先に処理し、以降は
/// `(name, content-list)` entry を comma-separated で peel する。
///
/// `<custom-ident>` は CSS-wide keyword + `default` (css-values-4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents> が将来の CSS-wide
/// keyword 用に予約) + `none` (top-level alt、gcpm-3 §1.1.1) を弾く。
///
/// ## Entry separator の strict 化
///
/// `#` (comma-separated multiplier、CSS Values 4 §2.3
/// <https://www.w3.org/TR/css-values-4/#mult-comma>) は entry 間に comma を
/// 要求する一方、**trailing comma を許容しない**。従って comma を consume した
/// 直後の loop iteration では次 entry の name parse **必須** — 失敗すれば
/// `#` production 全体が spec-invalid、declaration drop = `None`。
///
/// 初回 iteration で name parse が失敗する case (`string-set: ,`,
/// `string-set: "x"` 等 name 不在) も含めて `.ok()?` で一律に `None` 上位伝播
/// する。この strict `?` propagation は sibling
/// [`parse_optional_counter_style`] と同 principle。
///
/// `<content-list>` は 1+ items 必須 (CSS Content 3 §2)。name の後に 1 item も
/// peel できなければ malformed → `None` (declaration drop)。
pub(super) fn parse_string_set(
    input: &mut Parser<'_, '_>,
) -> Option<Vec<(SmolStr, Vec<ContentComponent>)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut entries = Vec::new();
    loop {
        // <custom-ident> — CSS-wide keyword + `default` + `none` を弾く。
        // 既存 `is_reserved_custom_ident` (css-wide + default) と、property-specific
        // top-level alternative の `none` reject を組み合わせる (
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
        // を受理しない (詳細は `ContentListMode` doc)。
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
///   `target-text` / `leader` arm を match guard で外し fall-through で `None`
///   を返す (= declaration drop、caller の `parse_string_set` が
///   `<content-list>` 0 items → `None`)。`counter` / `counters` / `content` /
///   `attr` は両 mode で spec grammar に含まれるため gate なし。
///
/// `element()` / `<image>` (`url()`) / `contents` / `<quote>` は function 名 dispatch では
/// なく [`parse_content_list_items`] 側の bare-token branch で扱う (`<image>`
/// は url token、`contents`/`<quote>` は bare ident であり `expect_function`
/// にヒットしないため)。
///
/// 各 `parse_*_fn` は自身では `expect_exhausted` を呼ばない —
/// [`parse_content`] 側の `parse_nested_block` が内部で
/// [`Parser::parse_entirely`] を経由し、closure 成功後の余剰 token を
/// exhaustion check で拒否する ([`parse_rgb_function`](super::color::parse_rgb_function) と同じ規約)。
fn parse_content_function(
    name: &str,
    mode: ContentListMode,
    input: &mut Parser<'_, '_>,
) -> Option<ContentComponent> {
    match name.to_ascii_lowercase().as_str() {
        "string" if matches!(mode, ContentListMode::CssContent3) => parse_string_fn(input),
        "element" if matches!(mode, ContentListMode::CssContent3) => parse_element_fn(input),
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
        "leader" if matches!(mode, ContentListMode::CssContent3) => parse_leader_fn(input),
        _ => None,
    }
}

/// `string(<custom-ident> [, [ first | start | last | first-except ]? ])`。
/// CSS Content 3 §2.7.2 <https://www.w3.org/TR/css-content-3/#string-function>。
pub(super) fn parse_string_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_custom_ident(input)?;
    let fetch = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_string_fetch(input)?
    } else {
        StringFetchMode::default()
    };
    Some(ContentComponent::String { name, fetch })
}

fn parse_element_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    Some(ContentComponent::Element {
        name: parse_custom_ident(input)?,
    })
}

pub(super) fn parse_string_fetch(input: &mut Parser<'_, '_>) -> Option<StringFetchMode> {
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
/// spec verbatim: "A `<counter-name>` name cannot match the keyword `none`;
/// such an identifier is invalid as a `<counter-name>`"。
///
/// counter() / counters() (§4.7) の first argument、および
/// counter-reset / counter-increment / counter-set property
/// (§4.1 / §4.2) の name 引数で使う。後者は既に [`parse_counter_property`] が
/// [`is_reserved_counter_name`]
/// 経由で reject 済 — 本 helper は前者を同じ predicate に揃えるための wrapper。
pub(super) fn parse_counter_name(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
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
/// `?` propagation、silent Decimal fallback は撤去済み)。
fn parse_optional_counter_style(input: &mut Parser<'_, '_>) -> Option<CounterStyle> {
    if input.try_parse(|i| i.expect_comma()).is_ok() {
        // comma consumed — ident 必須。失敗は None として上位伝播。
        let ident = input.expect_ident().ok()?.clone();
        Some(counter_style_from_ident(ident.as_ref()))
    } else {
        Some(CounterStyle::default())
    }
}

pub(crate) fn counter_style_from_ident(ident: &str) -> CounterStyle {
    if ident.eq_ignore_ascii_case("decimal") {
        CounterStyle::Decimal
    } else {
        CounterStyle::Named(SmolStr::new(ident))
    }
}

/// `attr(<attribute-name> [, <fallback>])`。CSS Content 3 §2.1 and
/// CSS Values and Units 5 §7.7.1.
///
/// This phase intentionally supports only untyped fallbacks that are either a
/// quoted string or a single identifier. The latter is retained as an
/// invalid-fallback marker: it contributes no text when the attribute is
/// absent, matching the measured `invalid` case without pretending to support
/// typed `attr()` syntax.
fn parse_attr_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = input.expect_ident().ok()?.clone();
    let name = SmolStr::new(name.as_ref());
    if input.try_parse(|i| i.expect_comma()).is_err() {
        return Some(ContentComponent::Attr { name });
    }

    let fallback = if let Ok(value) =
        input.try_parse(|i| i.expect_string().map(|value| SmolStr::new(value.as_ref())))
    {
        Some(value)
    } else {
        // A single non-string fallback token is represented as invalid for
        // the narrow untyped subset. `parse_nested_block` still enforces that
        // no additional tokens remain in the function.
        input.expect_ident().ok()?;
        None
    };
    Some(ContentComponent::AttrFallback { name, fallback })
}

/// target-* の第 1 引数 `[ <string> | <url> ]` を raw String として抽出。
/// `url("...")` / `url(...)` / bare `"..."` を統一的に受ける
/// (cssparser の `expect_url_or_string` を使用)。`<url>` 側の 2 形式は
/// [`parse_url_value`](super::visual::parse_url_value) と共通だが、bare `<string>` alternative も grammar に
/// 含む点が一般 `<url>` value type と異なるため、専用 helper として分離する。
pub(super) fn parse_target_url(input: &mut Parser<'_, '_>) -> Option<String> {
    input
        .expect_url_or_string()
        .ok()
        .map(|s| s.as_ref().to_string())
}

/// `target-counter([<string>|<url>], <custom-ident>, <counter-style>?)`。
/// CSS Content 3 §2.6.1 <https://www.w3.org/TR/css-content-3/#target-counter>。
pub(super) fn parse_target_counter_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    input.expect_comma().ok()?;
    let name = parse_custom_ident(input)?;
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::TargetCounter { url, name, style })
}

/// `target-counters([<string>|<url>], <custom-ident>, <string>, <counter-style>?)`。
/// CSS Content 3 §2.6.2 <https://www.w3.org/TR/css-content-3/#target-counters>。
pub(super) fn parse_target_counters_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
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

pub(super) fn parse_content_part(input: &mut Parser<'_, '_>) -> Option<ContentPart> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "content" => Some(ContentPart::Content),
        "before" => Some(ContentPart::Before),
        "after" => Some(ContentPart::After),
        "first-letter" => Some(ContentPart::FirstLetter),
        _ => None,
    }
}

/// `content([ text | before | after | first-letter ]?)` (`?` は raikiri の
/// 受理済み記法であり、GCPM 3 の grammar 自体には無い formal optional
/// marker ではない)。CSS GCPM 3 §1.1.1.1 "The content() function"
/// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content> の keyword 集合
/// (`text | before | after | first-letter` の 4 種) をそのまま実装する。
///
/// **grammar 選択の根拠**: `content()` は GCPM 3 §1.1.1.1 と CSS Content 3
/// §2.7.3 <https://www.w3.org/TR/css-content-3/#funcdef-content> の 2 つの
/// spec に別々に定義されており、2 つの軸で食い違う。(1) keyword 集合 — GCPM 3
/// は 4 keyword のみ、CSS Content 3 はそこに `marker` を加えた 5 keyword。
/// (2) 引数の省略可否 — GCPM 3 の production 自体には `?` が無く引数は形式上
/// 必須だが、CSS Content 3 は `?` 付きで、省略時は `text` を暗黙採用すると
/// 明記する。この実装は (1) の keyword 集合では GCPM 3 §1.1.1.1 に従い、
/// `marker` を意図的に reject する。(2) の引数省略可否については逆に
/// CSS Content 3 §2.7.3 の `?` 付き grammar と同じ挙動 (省略時 `text`
/// フォールバック) を採用しており、GCPM 3 の厳密な grammar (引数必須) には
/// 従っていない — 「GCPM 3 に従う」と言えるのは keyword 集合の軸のみである。
/// これは spec 間の grammar 相反を軸ごとに解決した結果の選択であり、
/// 実装漏れではない。
///
/// **既知の feature gap**: `content` property 側の `<content-list>` は
/// CSS Content 3 §2 governance (broad grammar、[`ContentListMode::CssContent3`]
/// 参照) だが、この `content()` 内部の keyword 集合だけは両 property 呼び出し
/// 元で GCPM 3 §1.1.1.1 の 4-keyword 版のまま unconditional に適用される
/// (下記 mode dispatch の節参照)。引数省略時の `text` フォールバック挙動は
/// 既に CSS Content 3 §2.7.3 の記述と一致しているため、CSS Content 3 §2.7.3
/// の広い grammar を優先実装する必要が生じた場合、残る差分は `marker`
/// keyword の受理のみ。
///
/// bare `content()` (spec 例 `h2 { string-set: heading content() }`、
/// string-set/GCPM3 側の文脈) では [`ContentTextKeyword::Text`] を
/// フォールバック値として使う (根拠は GCPM 3 側の spec "default" 宣言では
/// ない — 詳細は [`ContentTextKeyword`] の doc comment 参照)。target-text()
/// の第 2 引数と
/// 違い、keyword は paren 直下に置かれる (comma を先行させない)。
///
/// GCPM 3 §1.1.1 の narrow `<content-list>` (string-set 側) と CSS Content 3
/// §2 の broad `<content-list>` (content property 側) の **両方** に対し
/// unconditional に受理される ([`ContentListMode`] mode gate なし、
/// mode dispatch 導入後もこの arm は両 mode で unconditional のまま、
/// [`parse_content_function`] の match arm 参照) — 上記の通り、この
/// unconditional な適用自体が「受理 keyword 集合は両 property とも
/// GCPM 3 §1.1.1.1 の 4 種」という選択の実装箇所である。
fn parse_content_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let keyword = if input.is_exhausted() {
        ContentTextKeyword::default()
    } else {
        parse_content_text_keyword(input)?
    };
    Some(ContentComponent::Content { keyword })
}

pub(super) fn parse_content_text_keyword(input: &mut Parser<'_, '_>) -> Option<ContentTextKeyword> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "text" => Some(ContentTextKeyword::Text),
        "before" => Some(ContentTextKeyword::Before),
        "after" => Some(ContentTextKeyword::After),
        "first-letter" => Some(ContentTextKeyword::FirstLetter),
        _ => None,
    }
}

/// `leader(<leader-type>)`。CSS Content 3 §2.5.1 "The leader() function"
/// <https://www.w3.org/TR/css-content-3/#leader-function>。spec production
/// `leader( <leader-type> )` に `?` が無いため引数は必須
/// (`parse_leader_type` 失敗 = declaration drop、`leader()` 単体は spec-invalid)。
fn parse_leader_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let leader_type = parse_leader_type(input)?;
    Some(ContentComponent::Leader(leader_type))
}

/// `<leader-type> = dotted | solid | space | <string>`。[`LeaderType`] の doc
/// も参照 (keyword を正規化せず個別 variant で保持する rationale)。
fn parse_leader_type(input: &mut Parser<'_, '_>) -> Option<LeaderType> {
    if let Ok(s) = input.try_parse(|i| i.expect_string_cloned()) {
        return Some(LeaderType::String(SmolStr::new(s.as_ref())));
    }
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "dotted" => Some(LeaderType::Dotted),
        "solid" => Some(LeaderType::Solid),
        "space" => Some(LeaderType::Space),
        _ => None,
    }
}
