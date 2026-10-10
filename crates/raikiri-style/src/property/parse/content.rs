//! Generated-content property parsers: `content`, counters, `quotes`,
//! `string-set`, `page` and GCPM functions.

use cssparser::{ParseError, Parser, ParserInput};
use smol_str::SmolStr;

use crate::Atom;
use crate::property::types::*;

use super::common::*;

/// Parse the CSS Paged Media 3 §8.1 `page` value
/// (<https://www.w3.org/TR/css-page-3/#using-named-pages>).
///
/// Grammar: `auto | <custom-ident>`. Accept `auto` on its own; otherwise accept
/// one identifier except CSS-wide keywords (`inherit`/`initial`/`unset`/`revert`/
/// `revert-layer`) and `default` (see the [`PageValue`] docs).
/// Compare ASCII-case-insensitively, but preserve the authored spelling
/// (Atom is case-sensitive).
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

/// Parse the value of `counter-reset`, `counter-increment`, or `counter-set`.
///
/// Grammar (CSS Lists 3 §4):
///   `<counter-name> = <custom-ident>` — any identifier except CSS-wide keywords
///   (inherit / initial / unset / revert / revert-layer), `default`, and `none`.
///   `[ <counter-name> <integer>? ]+ | none`.
///
/// `default_number`: the spec default for each property (reset=0, increment=1,
/// set=0).
///
/// Handle `none` as a top-level alternative first. Then peel off identifiers
/// with optional integers using LL(1). Return `None` if an identifier is
/// reserved or the first token is not an identifier (`counter-reset: 123 abc`,
/// for example); rule.rs silently drops the entire declaration.
///
/// If a later identifier is reserved (`chapter none`), `try_parse` rewinds it
/// and exits the loop without consuming it. The caller's `expect_exhausted`
/// (rule.rs) detects the leftover token and drops the declaration.
pub(super) fn parse_counter_property(
    input: &mut Parser<'_, '_>,
    default_number: i32,
) -> Option<Vec<(SmolStr, i32)>> {
    // `none` = empty list (top-level alternative).
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut result = Vec::new();
    loop {
        // Do not accept reserved keywords as counter names (spec §4 and the
        // `<custom-ident>` exclusion list). `try_parse` rewinds on a reserved name.
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
        // Optional trailing `<integer>` (missing → property-specific default).
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

/// Parse the `quotes` value.
///
/// Grammar (legacy CSS2 §12.3.1 subset; see [`PropertyValue::Quotes`] docs):
///   `quotes = none | [ <string> <string> ]+`
///
/// Handle `none` as a top-level alternative first. Then peel off `<string>`
/// tokens in pairs using LL(1).
///
/// Unlike `parse_counter_property` (`<counter-name> <integer>?`, where a missing
/// integer uses a property-specific default), a `<string> <string>` pair is
/// invalid if either part is missing: the grammar has no optional part.
/// Thus, if the second `<string>` is missing after consuming the first (an
/// unpaired trailing string or a non-string token in its place), discard any
/// accumulated pairs and immediately drop the entire declaration by returning
/// `None`. Do not keep the pairs parsed so far and leave exhaustion checking
/// to the caller's `expect_exhausted` (rule.rs), as counter-* parsing does:
/// an odd number of strings cannot match `[ <string> <string> ]+`, so there is
/// no valid prefix to retain.
pub(super) fn parse_quotes_property(input: &mut Parser<'_, '_>) -> Option<Vec<(SmolStr, SmolStr)>> {
    // `none` = empty list (top-level alternative).
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

/// Exclusion list for `<counter-name> = <custom-ident>` (CSS Lists 3 §4 and
/// CSS Values 4 §4.2 <https://www.w3.org/TR/css-values-4/#custom-idents>).
///
/// Reject CSS-wide keywords, `default` (Counter Styles L3), and `none`
/// (a top-level alternative), comparing case-insensitively.
///
/// This is a permanent spec exclusion in CSS Values 4 §4.2, **independent
/// of whether CSS-wide keywords are implemented**. Do not confuse it with
/// the separate claim that CSS-wide keywords are not yet implemented as
/// property values, explained in the "CSS-wide keywords" section of the
/// [`PropertyValue`] docs.
pub(crate) fn is_reserved_counter_name(ident: &str) -> bool {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default" | "none"
    )
}

/// Parse a registered consumer property's resolved-text grammar from a complete
/// CSS value string.  The producer resolves the returned components to the
/// owned neutral `String` that a consumer event carries.
pub fn parse_consumer_text_value(source: &str) -> Option<Vec<ContentComponent>> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|parser| {
            let items = parse_content(parser).ok_or_else(|| parser.new_custom_error(()))?;
            Ok::<_, ParseError<'_, ()>>(items)
        })
        .ok()
}

/// Parse `content: normal | none | <content-list>`
/// (CSS Content 3 §1 <https://www.w3.org/TR/css-content-3/#content-property>).
///
/// The spec distinguishes `normal` from `none` (generate vs. suppress a
/// pseudo-element). Keep `normal` as an empty `Vec` for the initial value,
/// but represent explicit `none` with the internal [`ContentComponent::None`]
/// sentinel so downstream pseudo-element consumers can suppress the marker.
///
/// The items+ loop peels off `<string>` literals and function tokens (such as
/// `counter(...)`) in sequence. On an unrecognized token it breaks; the
/// caller's `expect_exhausted` (rule.rs) sees the leftover token and drops the
/// entire declaration.
///
/// `alt text` (spec `... [/ <string>...]?`) is out of scope for now. Return
/// `/` and subsequent tokens unconsumed to the caller (currently rule.rs
/// drops the declaration via `expect_exhausted`; extend this function when
/// adding alt-text support).
pub(crate) fn parse_content(input: &mut Parser<'_, '_>) -> Option<Vec<ContentComponent>> {
    // `normal` / `none` = empty list (top-level alternative).
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

/// The items+ loop for `<content-list>`.
///
/// Peel off bare `<string>` literals, the `<url>` alternative of `<image>`,
/// bare keywords (`contents` / `<quote>`), and function tokens (`counter(...)` /
/// `string(...)` / `target-*()` / `attr(...)` / `content(...)` / `leader()`).
/// Break on an unrecognized token; the caller detects the leftover token and
/// drops the declaration.
///
/// Both the `content` property (`parse_content`) and the `string-set` property
/// (`parse_string_set`) call this function. GCPM 3 §1.1.1 narrows the broad
/// CSS Content 3 §2 list for string-set, so `mode` selects accepted alternatives:
/// - [`ContentListMode::CssContent3`] — content property (CSS Content 3 §2
///   <https://www.w3.org/TR/css-content-3/#content-values>). Accepts the full
///   set of 10 alternatives (`<image>` / `contents` / `<quote>` / `leader()`
///   were added later).
/// - [`ContentListMode::GcpmStringSet`] — string-set property (CSS GCPM 3
///   §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#content-list>). Rejects
///   `string()` / `target-counter()` / `target-counters()` / `target-text()` /
///   `<image>` / `contents` / `<quote>` / `leader()` because they are absent
///   from the narrow GCPM 3 §1.1.1 L82 grammar (both modes accept bare
///   `<string>` literals).
///
/// Both `<content-list>` grammars include `<string>` as a top-level
/// alternative, so the bare-literal branch needs no mode check. Guard the
/// other alternatives inside each branch (as in the match-arm guards in
/// [`parse_content_function`]). `Parser::try_parse` always rewinds tokens on
/// failure, so the order of branch attempts does not affect correctness.
///
/// (After introduction, this was parameterized by mode and `<image>` /
/// `contents` / `<quote>` / `leader()` were added.)
pub(super) fn parse_content_list_items(
    input: &mut Parser<'_, '_>,
    mode: ContentListMode,
) -> Vec<ContentComponent> {
    let mut items = Vec::new();
    loop {
        // Bare `<string>` literal — shared by both modes (no mode guard).
        if let Ok(s) = input.try_parse(|i| i.expect_string_cloned()) {
            items.push(ContentComponent::Literal(SmolStr::new(s.as_ref())));
            continue;
        }
        // `<url>` alternative of `<image>` — only the CSS Images 3 <url>
        // production (`url(...)` / `url("...")`); defer `<gradient>` as
        // unimplemented (see the `ContentComponent::Image` docs). `expect_url`
        // does not accept bare quoted strings (`<url> = <url()> | <src()>`),
        // so it cannot overlap the literal branch above. CssContent3 only:
        // the narrow GCPM 3 §1.1.1 L82 list excludes `<image>`.
        if mode == ContentListMode::CssContent3
            && let Ok(url) = input.try_parse(|i| i.expect_url())
        {
            items.push(ContentComponent::Image {
                url: url.as_ref().to_string(),
            });
            continue;
        }
        // Bare keyword alternatives: `contents` / `<quote>` (identifiers,
        // neither functions nor `<string>` literals). CssContent3 only.
        if mode == ContentListMode::CssContent3
            && let Ok(c) = input.try_parse(|i| -> Result<ContentComponent, ParseError<'_, ()>> {
                let ident = i.expect_ident()?.clone();
                parse_content_bare_keyword(ident.as_ref()).ok_or_else(|| i.new_custom_error(()))
            })
        {
            items.push(c);
            continue;
        }
        // Function: `parse_content_function` uses match-arm guards to reject
        // `string()` / `target-*()` / `leader()` according to the mode.
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

/// Check the bare-identifier alternatives for `contents` and `<quote>`
/// (`open-quote` / `close-quote` / `no-open-quote` / `no-close-quote`) together.
/// This helper is only for [`parse_content_list_items`]. CSS Content 3 §2.3
/// <https://www.w3.org/TR/css-content-3/#element-content> and §2.4.2
/// <https://www.w3.org/TR/css-content-3/#quote-values>.
fn parse_content_bare_keyword(ident: &str) -> Option<ContentComponent> {
    if ident.eq_ignore_ascii_case("contents") {
        return Some(ContentComponent::Contents);
    }
    QuoteKeyword::from_css_ident(ident).map(ContentComponent::Quote)
}

/// Parse `string-set: none | [ <custom-ident> <content-list> ]#`
/// (CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>).
///
/// Handle `none` as a top-level alternative first, then peel off
/// comma-separated `(name, content-list)` entries.
///
/// Reject CSS-wide keywords, `default` (reserved for future CSS-wide
/// keywords by CSS Values 4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents>), and `none` (the
/// top-level alternative in GCPM 3 §1.1.1) as `<custom-ident>` names.
///
/// ## Strict entry separators
///
/// The `#` comma-separated multiplier (CSS Values 4 §2.3
/// <https://www.w3.org/TR/css-values-4/#mult-comma>) requires commas between
/// entries but **does not allow a trailing comma**. Thus, after consuming a
/// comma, the next loop iteration **must** parse another entry name. If it
/// cannot, the entire `#` production is invalid: drop the declaration with
/// `None`.
///
/// `.ok()?` likewise propagates `None` when the first iteration cannot parse
/// a name (`string-set: ,`, `string-set: "x"`, etc.). This strict `?`
/// propagation follows the same principle as sibling
/// [`parse_optional_counter_style`].
///
/// `<content-list>` requires 1+ items (CSS Content 3 §2). If no item can be
/// peeled off after the name, return `None` and drop the malformed declaration.
pub(super) fn parse_string_set(
    input: &mut Parser<'_, '_>,
) -> Option<Vec<(SmolStr, Vec<ContentComponent>)>> {
    // `none` = empty list (top-level alternative).
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut entries = Vec::new();
    loop {
        // <custom-ident>: reject CSS-wide keywords, `default`, and `none`.
        // Combine the existing `is_reserved_custom_ident` predicate (CSS-wide
        // keywords + default) with the property-specific top-level `none`
        // alternative (see the `is_reserved_custom_ident` docs).
        //
        // Strictly propagate `.ok()?`: (a) a missing name on the first
        // iteration means zero entries for the `#` production; (b) a missing
        // name after `expect_comma` succeeded means a trailing comma. Both
        // cases return `None`.
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
        // <content-list> requires 1+ items; zero drops the declaration.
        // The narrow local GCPM 3 §1.1.1 <content-list> excludes `string()`
        // and `target-*()` (see the `ContentListMode` docs).
        let items = parse_content_list_items(input, ContentListMode::GcpmStringSet);
        if items.is_empty() {
            return None;
        }
        entries.push((name, items));
        // Continue on a comma separator; otherwise break and let the caller's
        // `expect_exhausted` reject leftover tokens. A preceding push ensures
        // entries is nonempty when breaking, so return `Some(entries)` below.
        if input.try_parse(|i| i.expect_comma()).is_err() {
            break;
        }
    }

    // Reaching break requires a prior push: aside from `.ok()?`, it is the
    // only loop exit, so `entries` has at least one item.
    Some(entries)
}

/// Dispatch by function name (ASCII-case-insensitive, as usual for spec
/// identifiers). Return `None` for unknown names or invalid arguments; the
/// caller's `parse_nested_block` converts it to a custom error.
///
/// `mode` selects each property's `<content-list>` vocabulary (see the
/// [`ContentListMode`] docs):
/// - [`ContentListMode::CssContent3`] (`content` property, CSS Content 3 §2)
///   permits every arm.
/// - [`ContentListMode::GcpmStringSet`] (`string-set` property, CSS GCPM 3
///   §1.1.1) guards out the `string` / `target-counter` / `target-counters` /
///   `target-text` / `leader` arms, falling through to `None` (declaration
///   dropped when the caller's `parse_string_set` sees zero `<content-list>`
///   items). `counter` / `counters` / `content` / `attr` belong to both modes'
///   spec grammars, so they have no guard.
///
/// `element()` / `<image>` (`url()`) / `contents` / `<quote>` are handled by
/// bare-token branches in [`parse_content_list_items`], not by function-name
/// dispatch: `<image>` is a URL token and `contents`/`<quote>` are bare
/// identifiers, so `expect_function` does not see them.
///
/// Individual `parse_*_fn` functions do not call `expect_exhausted`:
/// [`parse_content`]'s `parse_nested_block` internally calls
/// [`Parser::parse_entirely`], which rejects remaining tokens after its
/// closure succeeds (the same convention as
/// [`parse_rgb_function`](super::color::parse_rgb_function)).
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

/// `string(<custom-ident> [, [ first | start | last | first-except ]? ])`.
/// CSS Content 3 §2.7.2 <https://www.w3.org/TR/css-content-3/#string-function>.
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
    let name = parse_custom_ident(input)?;
    let fetch = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_string_fetch(input)?
    } else {
        StringFetchMode::default()
    };
    Some(ContentComponent::Element { name, fetch })
}

pub(super) fn parse_string_fetch(input: &mut Parser<'_, '_>) -> Option<StringFetchMode> {
    StringFetchMode::from_css_ident(input.expect_ident().ok()?)
}

/// `<counter-name>` (CSS Lists 3 §4
/// <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>):
/// a `<custom-ident>` production that additionally excludes `none`.
/// The spec says: "A `<counter-name>` name cannot match the keyword `none`;
/// such an identifier is invalid as a `<counter-name>`".
///
/// Used for the first argument to counter() / counters() (§4.7) and the name
/// arguments of counter-reset / counter-increment / counter-set (§4.1 / §4.2).
/// [`parse_counter_property`] already rejects the latter through
/// [`is_reserved_counter_name`]; this wrapper applies the same predicate to
/// the former.
pub(super) fn parse_counter_name(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_counter_name(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// `counter(<counter-name>, <counter-style>?)`.
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>.
/// The first argument follows the §4 `<counter-name>` grammar
/// (<https://www.w3.org/TR/css-lists-3/#typedef-counter-name>):
/// `<custom-ident>` with `none` additionally excluded.
fn parse_counter_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_counter_name(input)?;
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::Counter { name, style })
}

/// `counters(<counter-name>, <string>, <counter-style>?)`.
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>.
/// The first argument follows the §4 `<counter-name>` grammar
/// (<https://www.w3.org/TR/css-lists-3/#typedef-counter-name>):
/// `<custom-ident>` with `none` additionally excluded.
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

/// Optional trailing `, <counter-style>`. When absent, use the spec default
/// `decimal` (final argument to `counter()` / `counters()` in CSS Lists 3 §4.7
/// <https://www.w3.org/TR/css-lists-3/#counter-functions>, and to
/// `target-counter()` / `target-counters()` in CSS Content 3 §2.6.1-2
/// <https://www.w3.org/TR/css-content-3/#target-counter>).
///
/// The grammar is `<counter-style>?`; if a comma precedes it, an identifier
/// is mandatory. If no identifier follows a consumed comma (e.g.
/// `counter(chapter,)`), the trailing comma is invalid. Propagate `None` to
/// drop the entire declaration (the same strict `?` propagation as siblings
/// [`parse_string_fetch`] / [`parse_content_part`], rather than a silent
/// fallback to Decimal).
fn parse_optional_counter_style(input: &mut Parser<'_, '_>) -> Option<CounterStyle> {
    if input.try_parse(|i| i.expect_comma()).is_ok() {
        // Comma consumed: require an identifier and propagate failure as None.
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

/// `attr(<attribute-name> [, <fallback>])`. CSS Content 3 §2.1 and
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

/// Extract the first target-* argument `[ <string> | <url> ]` as a raw String.
/// Accept `url("...")`, `url(...)`, and bare `"..."` through cssparser's
/// `expect_url_or_string`. The two `<url>` forms are shared with
/// [`parse_url_value`](super::visual::parse_url_value), but this grammar also
/// accepts bare `<string>` values, unlike the general `<url>` value type;
/// hence this separate helper.
pub(super) fn parse_target_url(input: &mut Parser<'_, '_>) -> Option<String> {
    input
        .expect_url_or_string()
        .ok()
        .map(|s| s.as_ref().to_string())
}

/// `target-counter([<string>|<url>], <custom-ident>, <counter-style>?)`.
/// CSS Content 3 §2.6.1 <https://www.w3.org/TR/css-content-3/#target-counter>.
pub(super) fn parse_target_counter_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    input.expect_comma().ok()?;
    let name = parse_custom_ident(input)?;
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::TargetCounter { url, name, style })
}

/// `target-counters([<string>|<url>], <custom-ident>, <string>, <counter-style>?)`.
/// CSS Content 3 §2.6.2 <https://www.w3.org/TR/css-content-3/#target-counters>.
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

/// `target-text([<string>|<url>], [ content | before | after | first-letter ]?)`.
/// CSS Content 3 §2.6.3 <https://www.w3.org/TR/css-content-3/#target-text>.
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
    ContentPart::from_css_ident(input.expect_ident().ok()?)
}

/// `content([ text | before | after | first-letter ]?)`. The `?` describes
/// Raikiri's accepted syntax; it is not a formal optional marker in the
/// GCPM 3 grammar. CSS GCPM 3 §1.1.1.1 "The content() function"
/// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content> defines precisely
/// these four keywords (`text | before | after | first-letter`).
///
/// **Grammar choice**: GCPM 3 §1.1.1.1 and CSS Content 3 §2.7.3
/// <https://www.w3.org/TR/css-content-3/#funcdef-content> define `content()`
/// separately, differing on two axes. (1) Keyword set: GCPM 3 has four,
/// while CSS Content 3 adds `marker` as a fifth. (2) Optional argument:
/// the GCPM 3 production has no `?` and formally requires an argument,
/// while CSS Content 3 has `?` and explicitly defaults an omitted argument
/// to `text`. This implementation follows GCPM 3 §1.1.1.1 for the keyword
/// set, intentionally rejecting `marker`. For the optional argument it
/// instead follows CSS Content 3 §2.7.3, defaulting to `text`, rather than
/// the stricter GCPM 3 grammar. Thus it follows GCPM 3 only on the keyword
/// axis. This resolves the conflicting grammars by axis; it is not an
/// accidental omission.
///
/// **Known feature gap**: the `content` property's `<content-list>` uses the
/// broader CSS Content 3 §2 grammar (see [`ContentListMode::CssContent3`]),
/// but the keyword set inside `content()` is unconditionally the four-keyword
/// GCPM 3 §1.1.1.1 version for both calling properties (see mode dispatch
/// below). Omitting the argument already defaults to `text` as CSS Content 3
/// §2.7.3 specifies. If the broader CSS Content 3 §2.7.3 grammar takes
/// priority later, only accepting `marker` remains to be added.
///
/// Bare `content()` (e.g. `h2 { string-set: heading content() }` in the
/// string-set/GCPM3 context) falls back to [`ContentTextKeyword::Text`].
/// This is not based on a GCPM 3 spec declaration of a default; see the
/// [`ContentTextKeyword`] docs. Unlike the second argument to target-text(),
/// this keyword directly follows the opening parenthesis, with no comma.
///
/// This function is accepted unconditionally in both the narrow GCPM 3
/// §1.1.1 `<content-list>` (string-set) and the broad CSS Content 3 §2
/// `<content-list>` (content property). It has no [`ContentListMode`] guard;
/// even after mode dispatch was added, its arm in [`parse_content_function`]
/// remained unconditional. As noted above, this is where the choice of the
/// four GCPM 3 §1.1.1.1 keywords for both properties is implemented.
fn parse_content_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let keyword = if input.is_exhausted() {
        ContentTextKeyword::default()
    } else {
        parse_content_text_keyword(input)?
    };
    Some(ContentComponent::Content { keyword })
}

pub(super) fn parse_content_text_keyword(input: &mut Parser<'_, '_>) -> Option<ContentTextKeyword> {
    ContentTextKeyword::from_css_ident(input.expect_ident().ok()?)
}

/// `leader(<leader-type>)`. CSS Content 3 §2.5.1 "The leader() function"
/// <https://www.w3.org/TR/css-content-3/#leader-function>. The spec production
/// `leader( <leader-type> )` has no `?`, so an argument is mandatory:
/// failure in `parse_leader_type` drops the declaration, and bare `leader()`
/// is invalid.
fn parse_leader_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let leader_type = parse_leader_type(input)?;
    Some(ContentComponent::Leader(leader_type))
}

/// `<leader-type> = dotted | solid | space | <string>`. See the [`LeaderType`]
/// docs for why keyword variants are kept separate instead of normalized.
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
