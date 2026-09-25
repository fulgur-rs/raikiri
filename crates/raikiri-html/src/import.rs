//! Resolver-driven expansion of CSS `@import` statements.
//!
//! The style crate deliberately has no network dependency.  Imports are therefore
//! expanded in the HTML parse layer, where the existing [`NetworkProvider`] and
//! document base URL are already available.  A successfully fetched import is
//! replaced with its CSS at the import position; failed, unresolvable, cyclic,
//! or depth-limited imports are copied unchanged so the style parser can retain
//! them as opaque at-rules.

use raikiri_traits::{
    Body, Method, NetworkError, NetworkProvider, PolicyViolation, RenderWarning, Request,
    ResourceKind, ViolationType, WarningKind,
};
use url::Url;

/// Conservative fallback bound used when the provider does not expose a policy
/// object to this layer. Sandboxed providers still receive
/// [`ResourceKind::StylesheetImport`] on every request and can expose their
/// [`NetworkProvider::max_import_depth`] policy; this fallback prevents an
/// unwrapped provider from recursing forever.
const MAX_IMPORT_DEPTH: u32 = 4;
/// Bound the number of import fetches in one root stylesheet. Depth alone does
/// not prevent a wide fan-out from consuming unbounded resources.
const MAX_IMPORT_FETCHES: usize = 256;
/// Bound the bytes introduced by successful import expansion. The original
/// stylesheet is never truncated; imports beyond this budget stay opaque.
const MAX_IMPORT_EXPANSION_BYTES: usize = 16 * 1024 * 1024;
/// Bound cumulative response bytes before decoding or scanning them. This also
/// limits work spent on responses that ultimately cannot be expanded.
const MAX_IMPORT_RESPONSE_BYTES_TOTAL: usize = MAX_IMPORT_EXPANSION_BYTES;
/// Avoid decoding one provider-controlled response before the expansion budget
/// can reject it.
const MAX_IMPORT_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// Refuse unreasonably large provider-controlled URL tokens before parsing or
/// handing them to a network implementation.
const MAX_IMPORT_URL_LENGTH: usize = 8 * 1024;

#[derive(Debug)]
struct ImportStatement {
    start: usize,
    end: usize,
    url: String,
    media: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuleEnd {
    Statement(usize),
    Block(usize),
    EndOfInput(usize),
    Invalid,
}

#[derive(Debug, Default)]
pub(crate) struct ImportBudget {
    /// Total import fetches allowed for one parse operation, across all roots.
    fetches: usize,
    /// Total provider response bytes admitted for import processing.
    response_bytes: usize,
    /// Total bytes introduced by successful imports for one parse operation.
    expansion_bytes: usize,
}

struct ImportExpander<'a> {
    network: &'a dyn NetworkProvider,
    warnings: &'a mut Vec<RenderWarning>,
    max_depth: u32,
    chain: Vec<Url>,
    budget: &'a mut ImportBudget,
}

/// Expand imports in a stylesheet using the supplied base URL and provider.
///
/// This convenience entry point owns a fresh budget and is used by focused
/// callers that expand one independent stylesheet. The HTML parse pipeline
/// uses [`expand_stylesheet_imports_with_budget`] so all stylesheet roots in a
/// document share the same fetch and expansion limits.
///
/// `root_url` is the URL of a fetched stylesheet, when the source came from a
/// network response. It is seeded into the recursion chain so a response that
/// redirects back to an ancestor is treated as a cycle. Inline stylesheets do
/// not have a root URL and pass `None`.
#[allow(dead_code)] // retained as a one-root helper for focused parser tests
pub(crate) fn expand_stylesheet_imports(
    source: &str,
    base_url: Option<&Url>,
    root_url: Option<&Url>,
    network: Option<&dyn NetworkProvider>,
    warnings: &mut Vec<RenderWarning>,
) -> String {
    let mut budget = ImportBudget::default();
    expand_stylesheet_imports_with_budget(
        source,
        base_url,
        root_url,
        network,
        warnings,
        &mut budget,
    )
}

/// Expand imports while consuming the caller's document-wide budget.
pub(crate) fn expand_stylesheet_imports_with_budget(
    source: &str,
    base_url: Option<&Url>,
    root_url: Option<&Url>,
    network: Option<&dyn NetworkProvider>,
    warnings: &mut Vec<RenderWarning>,
    budget: &mut ImportBudget,
) -> String {
    let Some(network) = network else {
        return source.to_owned();
    };

    let normalized_base = normalize_base_url(base_url);
    let normalized_root = normalize_base_url(root_url);
    let max_depth = network.max_import_depth().unwrap_or(MAX_IMPORT_DEPTH);
    if root_url.is_some() && normalized_root.is_none() {
        return source.to_owned();
    }
    let mut expander = ImportExpander {
        network,
        warnings,
        max_depth,
        chain: normalized_root.into_iter().collect(),
        budget,
    };
    expander.expand(source, normalized_base.as_ref(), 0)
}

impl ImportExpander<'_> {
    fn expand(&mut self, source: &str, base_url: Option<&Url>, depth: u32) -> String {
        if depth >= self.max_depth {
            return source.to_owned();
        }

        let imports = scan_leading_imports(source);
        if imports.is_empty() {
            return source.to_owned();
        }

        let mut expanded = String::with_capacity(source.len());
        let mut cursor = 0;
        for import in imports {
            expanded.push_str(&source[cursor..import.start]);

            let replacement = self
                .resolve_import(&import, base_url, depth)
                .unwrap_or_else(|| source[import.start..import.end].to_owned());
            expanded.push_str(&replacement);
            cursor = import.end;
        }
        expanded.push_str(&source[cursor..]);
        expanded
    }

    fn record_import_fetch_error(&mut self, url: Url, error: NetworkError) {
        let safe_url = redacted_url(&url);
        let summary = network_error_summary(&error);
        let (kind, details) = match error {
            NetworkError::PolicyViolation(violation) => {
                let (kind, reason) = match &violation.violation_type {
                    ViolationType::FetchTooLarge { limit, actual }
                    | ViolationType::DecodedTooLarge { limit, actual } => (
                        WarningKind::ResourceLimitExceeded {
                            kind: violation.kind,
                            limit: *limit,
                            actual: *actual,
                        },
                        "resource response exceeded a byte limit",
                    ),
                    _ => (
                        WarningKind::PolicyWarning {
                            violation: sanitize_policy_violation(violation.clone()),
                        },
                        "fetch violated network policy",
                    ),
                };
                (kind, format!("stylesheet @import {reason} for {safe_url}"))
            }
            _ => (
                WarningKind::NetworkFallback {
                    url: safe_url.clone(),
                },
                format!("stylesheet @import fetch failed for {safe_url}: {summary}"),
            ),
        };
        self.warnings.push(RenderWarning {
            kind,
            node_id: None,
            details,
        });
    }

    fn record_import_fallback(&mut self, url: Url, details: String) {
        self.warnings.push(RenderWarning {
            kind: WarningKind::NetworkFallback {
                url: redacted_url(&url),
            },
            node_id: None,
            details,
        });
    }

    fn resolve_import(
        &mut self,
        import: &ImportStatement,
        base_url: Option<&Url>,
        depth: u32,
    ) -> Option<String> {
        if import.url.len() > MAX_IMPORT_URL_LENGTH {
            return None;
        }
        let requested_url = canonical_url(resolve_url(&import.url, base_url)?);
        if has_credentials(&requested_url)
            || requested_url.scheme().eq_ignore_ascii_case("javascript")
        {
            return None;
        }
        if requested_url.as_str().len() > MAX_IMPORT_URL_LENGTH {
            return None;
        }
        if self.chain.iter().any(|ancestor| ancestor == &requested_url)
            || self.budget.fetches >= MAX_IMPORT_FETCHES
            || self.budget.response_bytes >= MAX_IMPORT_RESPONSE_BYTES_TOTAL
            || self.budget.expansion_bytes >= MAX_IMPORT_EXPANSION_BYTES
        {
            return None;
        }

        self.budget.fetches += 1;
        let fetched = match self.network.fetch(Request {
            url: requested_url.clone(),
            method: Method::Get,
            content_type: None,
            headers: Vec::new(),
            body: Body::Empty,
            signal: None,
            kind: ResourceKind::StylesheetImport,
        }) {
            Ok(fetched) => fetched,
            Err(error) => {
                self.record_import_fetch_error(requested_url, error);
                return None;
            }
        };
        let Some(response_bytes) = self.budget.response_bytes.checked_add(fetched.bytes.len())
        else {
            self.budget.response_bytes = MAX_IMPORT_RESPONSE_BYTES_TOTAL;
            return None;
        };
        if response_bytes > MAX_IMPORT_RESPONSE_BYTES_TOTAL {
            self.budget.response_bytes = MAX_IMPORT_RESPONSE_BYTES_TOTAL;
            return None;
        }
        self.budget.response_bytes = response_bytes;
        if fetched.bytes.len() > MAX_IMPORT_RESPONSE_BYTES {
            self.record_import_fallback(
                requested_url,
                format!(
                    "stylesheet @import response exceeded the {} byte limit",
                    MAX_IMPORT_RESPONSE_BYTES
                ),
            );
            return None;
        }
        if fetched
            .content_type
            .as_deref()
            .is_some_and(|content_type| !is_css_content_type(content_type))
        {
            self.record_import_fallback(
                requested_url,
                "stylesheet @import response had a non-CSS Content-Type".to_owned(),
            );
            return None;
        }

        if fetched.final_url.as_str().len() > MAX_IMPORT_URL_LENGTH {
            self.record_import_fallback(
                requested_url,
                format!(
                    "stylesheet @import response URL exceeded the {} byte limit",
                    MAX_IMPORT_URL_LENGTH
                ),
            );
            return None;
        }
        let fetched_url = canonical_url(fetched.final_url);
        if has_credentials(&fetched_url) {
            self.record_import_fallback(
                fetched_url,
                "stylesheet @import response redirected to a credential-bearing URL".to_owned(),
            );
            return None;
        }
        if self.chain.iter().any(|ancestor| ancestor == &fetched_url) {
            return None;
        }

        let css = String::from_utf8_lossy(&fetched.bytes).into_owned();
        if !stylesheet_source_is_balanced(&css) {
            self.record_import_fallback(
                fetched_url,
                "stylesheet @import response was not a self-contained CSS source".to_owned(),
            );
            return None;
        }
        let chain_entries = if requested_url == fetched_url { 1 } else { 2 };
        self.chain.push(requested_url);
        if chain_entries == 2 {
            self.chain.push(fetched_url.clone());
        }
        let child = self.expand(&css, Some(&fetched_url), depth + 1);
        for _ in 0..chain_entries {
            let _ = self.chain.pop();
        }

        if child.is_empty() {
            return Some(String::new());
        }
        let replacement = if let Some(media) = &import.media {
            let mut wrapped = String::with_capacity(child.len() + media.len() + 12);
            wrapped.push_str("@media ");
            wrapped.push_str(media);
            wrapped.push('{');
            wrapped.push_str(&child);
            wrapped.push('}');
            wrapped
        } else {
            child
        };
        let total = self.budget.expansion_bytes.checked_add(replacement.len())?;
        if total > MAX_IMPORT_EXPANSION_BYTES {
            return None;
        }
        self.budget.expansion_bytes = total;
        Some(replacement)
    }
}

/// Find top-level imports before the first recognized rule that blocks them.
///
/// CSS import placement is checked against recognized rules. Unknown at-rules
/// are retained by the style parser but, like browser error recovery, do not
/// by themselves make a later import unusable. The scan is intentionally
/// source-preserving: it records byte spans and the decoded URL, while leaving
/// every non-replaced byte untouched.
fn is_import_barrier(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "color-profile"
            | "container"
            | "counter-style"
            | "custom-media"
            | "document"
            | "font-face"
            | "font-feature-values"
            | "font-palette-values"
            | "keyframes"
            | "namespace"
            | "page"
            | "property"
            | "scope"
            | "starting-style"
            | "supports"
            | "scroll-timeline"
            | "timeline-scope"
            | "viewport"
            | "view-timeline"
            | "view-transition"
            | "position-try"
            | "nest"
            | "when"
            | "else"
            | "media"
            | "-moz-document"
            | "-webkit-keyframes"
    )
}

fn scan_leading_imports(source: &str) -> Vec<ImportStatement> {
    let mut imports = Vec::new();
    let mut cursor = 0;
    let mut imports_allowed = true;

    while cursor < source.len() {
        let Some(next) = skip_ignored_prefix(source, cursor) else {
            break;
        };
        cursor = next;
        if cursor >= source.len() {
            break;
        }

        if source.as_bytes()[cursor] == b'@' {
            let start = cursor;
            let Some((after_name, name)) = parse_identifier(source, cursor + 1) else {
                break;
            };
            match scan_at_rule_end(source, after_name) {
                RuleEnd::Statement(delimiter) => {
                    let end = delimiter + 1;
                    if imports_allowed && name.eq_ignore_ascii_case("import") {
                        if let Some((url, media)) =
                            parse_import_prelude(&source[after_name..delimiter])
                        {
                            imports.push(ImportStatement {
                                start,
                                end,
                                url,
                                media,
                            });
                        }
                    } else if imports_allowed && name.eq_ignore_ascii_case("charset") {
                        // `@charset` is ignored for import placement. This
                        // matches CSS error recovery, including repeated
                        // declarations before the first import.
                    } else if imports_allowed
                        && name.eq_ignore_ascii_case("layer")
                        && !source[after_name..delimiter].trim().is_empty()
                    {
                        // A layer statement with a non-empty layer name is
                        // permitted before imports. The prelude is not
                        // interpreted here; malformed component values were
                        // already rejected by scan_at_rule_end.
                    } else if imports_allowed && is_import_barrier(&name) {
                        imports_allowed = false;
                    }
                    cursor = end;
                }
                RuleEnd::EndOfInput(end) => {
                    if imports_allowed
                        && name.eq_ignore_ascii_case("import")
                        && let Some((url, media)) = parse_import_prelude(&source[after_name..end])
                    {
                        imports.push(ImportStatement {
                            start,
                            end,
                            url,
                            media,
                        });
                    }
                    break;
                }
                RuleEnd::Block(open_brace) => {
                    // Imports nested in any block are not top-level imports.
                    // Unknown wrappers are otherwise ignored by browsers and
                    // do not prevent a later top-level import from being
                    // processed; recognized at-rules do.
                    let Some(end) = scan_block_end(source, open_brace) else {
                        break;
                    };
                    if is_import_barrier(&name) || name.eq_ignore_ascii_case("layer") {
                        imports_allowed = false;
                    }
                    cursor = end;
                }
                RuleEnd::Invalid => break,
            }
        } else {
            // A qualified rule before an import makes the import misplaced.
            imports_allowed = false;
            let Some(end) = scan_qualified_rule_end(source, cursor) else {
                break;
            };
            cursor = end;
        }
    }

    imports
}

fn skip_ignored_prefix(source: &str, mut cursor: usize) -> Option<usize> {
    loop {
        let bytes = source.as_bytes();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if source.get(cursor..)?.starts_with('\u{feff}') {
            cursor += '\u{feff}'.len_utf8();
            continue;
        }
        if source.get(cursor..)?.starts_with("/*") {
            let rest = source.get(cursor + 2..)?;
            let end = rest.find("*/")?;
            cursor += end + 4;
            continue;
        }
        // CDO/CDC tokens are ignored at the top level by cssparser.
        if source.get(cursor..)?.starts_with("<!--") {
            cursor += 4;
            continue;
        }
        if source.get(cursor..)?.starts_with("-->") {
            cursor += 3;
            continue;
        }
        return Some(cursor);
    }
}

fn parse_identifier(source: &str, mut cursor: usize) -> Option<(usize, String)> {
    let mut value = String::new();
    while cursor < source.len() {
        let byte = source.as_bytes()[cursor];
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') {
            value.push(byte as char);
            cursor += 1;
            continue;
        }
        if byte == b'\\' {
            let (next, decoded) = consume_escape(source, cursor)?;
            if let Some(ch) = decoded {
                value.push(ch);
            }
            cursor = next;
            continue;
        }
        if byte >= 0x80 {
            let ch = source[cursor..].chars().next()?;
            value.push(ch);
            cursor += ch.len_utf8();
            continue;
        }
        break;
    }
    (!value.is_empty()).then_some((cursor, value))
}

fn scan_at_rule_end(source: &str, mut cursor: usize) -> RuleEnd {
    let bytes = source.as_bytes();
    let mut parentheses = 0u32;
    let mut brackets = 0u32;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
                let Some(end) = source[cursor + 2..].find("*/") else {
                    return RuleEnd::Invalid;
                };
                cursor += end + 4;
            }
            b'\'' | b'"' => {
                let Some(next) = skip_string(source, cursor) else {
                    return RuleEnd::Invalid;
                };
                cursor = next;
            }
            b'\\' => {
                let Some((next, _)) = consume_escape(source, cursor) else {
                    return RuleEnd::Invalid;
                };
                cursor = next;
            }
            b'(' => {
                parentheses = parentheses.saturating_add(1);
                cursor += 1;
            }
            b')' => {
                if parentheses == 0 {
                    return RuleEnd::Invalid;
                }
                parentheses -= 1;
                cursor += 1;
            }
            b'[' => {
                brackets = brackets.saturating_add(1);
                cursor += 1;
            }
            b']' => {
                if brackets == 0 {
                    return RuleEnd::Invalid;
                }
                brackets -= 1;
                cursor += 1;
            }
            b'{' if parentheses == 0 && brackets == 0 => return RuleEnd::Block(cursor),
            b';' if parentheses == 0 && brackets == 0 => return RuleEnd::Statement(cursor),
            b'}' if parentheses == 0 && brackets == 0 => return RuleEnd::Invalid,
            _ => cursor += 1,
        }
    }
    if parentheses == 0 && brackets == 0 {
        RuleEnd::EndOfInput(cursor)
    } else {
        RuleEnd::Invalid
    }
}

fn scan_qualified_rule_end(source: &str, mut cursor: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut parentheses = 0u32;
    let mut brackets = 0u32;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
                let end = source[cursor + 2..].find("*/")?;
                cursor += end + 4;
            }
            b'\'' | b'"' => cursor = skip_string(source, cursor)?,
            b'\\' => cursor = consume_escape(source, cursor)?.0,
            b'(' => {
                parentheses = parentheses.saturating_add(1);
                cursor += 1;
            }
            b')' if parentheses > 0 => {
                parentheses -= 1;
                cursor += 1;
            }
            b'[' => {
                brackets = brackets.saturating_add(1);
                cursor += 1;
            }
            b']' if brackets > 0 => {
                brackets -= 1;
                cursor += 1;
            }
            b'{' if parentheses == 0 && brackets == 0 => return scan_block_end(source, cursor),
            b';' if parentheses == 0 && brackets == 0 => return Some(cursor + 1),
            _ => cursor += 1,
        }
    }
    Some(cursor)
}

fn scan_block_end(source: &str, open_brace: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut cursor = open_brace + 1;
    let mut braces = 1u32;
    let mut parentheses = 0u32;
    let mut brackets = 0u32;
    while cursor < bytes.len() {
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            let end = source[cursor + 2..].find("*/")?;
            cursor += end + 4;
            continue;
        }
        if matches!(bytes[cursor], b'\'' | b'"') {
            cursor = skip_string(source, cursor)?;
            continue;
        }
        if bytes[cursor] == b'\\' {
            cursor = consume_escape(source, cursor)?.0;
            continue;
        }
        if let Some((after_name, name)) = parse_identifier(source, cursor)
            && name.eq_ignore_ascii_case("url")
            && bytes.get(after_name) == Some(&b'(')
            && let Some((_, next)) = parse_url_function(source, after_name)
        {
            cursor = next;
            continue;
        }
        match bytes[cursor] {
            b'{' => braces = braces.checked_add(1)?,
            b'}' if parentheses == 0 && brackets == 0 => {
                braces -= 1;
                cursor += 1;
                if braces == 0 {
                    return Some(cursor);
                }
                continue;
            }
            b'(' => parentheses = parentheses.checked_add(1)?,
            b')' => {
                if parentheses == 0 {
                    return None;
                }
                parentheses -= 1;
            }
            b'[' => brackets = brackets.checked_add(1)?,
            b']' => {
                if brackets == 0 {
                    return None;
                }
                brackets -= 1;
            }
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn stylesheet_source_is_balanced(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut braces = 0u32;
    let mut parentheses = 0u32;
    let mut brackets = 0u32;
    while cursor < bytes.len() {
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            let Some(end) = source[cursor + 2..].find("*/") else {
                return false;
            };
            cursor += end + 4;
            continue;
        }
        if matches!(bytes[cursor], b'\'' | b'"') {
            let Some(next) = skip_string(source, cursor) else {
                return false;
            };
            cursor = next;
            continue;
        }
        if bytes[cursor] == b'\\' {
            let Some((next, _)) = consume_escape(source, cursor) else {
                return false;
            };
            cursor = next;
            continue;
        }
        if let Some((after_name, name)) = parse_identifier(source, cursor)
            && name.eq_ignore_ascii_case("url")
            && bytes.get(after_name) == Some(&b'(')
            && let Some((_, next)) = parse_url_function(source, after_name)
        {
            cursor = next;
            continue;
        }
        match bytes[cursor] {
            b'{' => braces = braces.saturating_add(1),
            b'}' => {
                if braces == 0 {
                    return false;
                }
                braces -= 1;
            }
            b'(' => parentheses = parentheses.saturating_add(1),
            b')' => {
                if parentheses == 0 {
                    return false;
                }
                parentheses -= 1;
            }
            b'[' => brackets = brackets.saturating_add(1),
            b']' => {
                if brackets == 0 {
                    return false;
                }
                brackets -= 1;
            }
            _ => {}
        }
        cursor += 1;
    }
    braces == 0 && parentheses == 0 && brackets == 0
}

fn skip_string(source: &str, quote_start: usize) -> Option<usize> {
    let quote = source.as_bytes().get(quote_start).copied()?;
    let mut cursor = quote_start + 1;
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            byte if byte == quote => return Some(cursor + 1),
            b'\\' => cursor = consume_escape(source, cursor)?.0,
            b'\n' | b'\r' | b'\x0c' => return None,
            _ => cursor += 1,
        }
    }
    None
}

fn consume_escape(source: &str, slash: usize) -> Option<(usize, Option<char>)> {
    let bytes = source.as_bytes();
    let mut cursor = slash + 1;
    let first = *bytes.get(cursor)?;
    if matches!(first, b'\n' | b'\r' | b'\x0c') {
        if first == b'\r' && bytes.get(cursor + 1) == Some(&b'\n') {
            cursor += 1;
        }
        return Some((cursor + 1, None));
    }
    if first.is_ascii_hexdigit() {
        let mut value = 0u32;
        let mut digits = 0;
        while digits < 6 {
            let Some(byte) = bytes.get(cursor) else { break };
            let Some(digit) = (*byte as char).to_digit(16) else {
                break;
            };
            value = value * 16 + digit;
            digits += 1;
            cursor += 1;
        }
        if bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            if bytes[cursor] == b'\r' && bytes.get(cursor + 1) == Some(&b'\n') {
                cursor += 2;
            } else {
                cursor += 1;
            }
        }
        let decoded = char::from_u32(value)
            .filter(|character| *character != '\0')
            .unwrap_or('\u{fffd}');
        return Some((cursor, Some(decoded)));
    }
    let ch = source[cursor..].chars().next()?;
    Some((cursor + ch.len_utf8(), Some(ch)))
}

fn parse_import_prelude(source: &str) -> Option<(String, Option<String>)> {
    let mut cursor = skip_whitespace_comments(source, 0)?;
    let (url, after_url) = if source.as_bytes().get(cursor) == Some(&b'\'')
        || source.as_bytes().get(cursor) == Some(&b'"')
    {
        parse_string_value(source, cursor)?
    } else {
        let (after_name, name) = parse_identifier(source, cursor)?;
        if !name.eq_ignore_ascii_case("url") || source.as_bytes().get(after_name) != Some(&b'(') {
            return None;
        }
        parse_url_function(source, after_name)?
    };
    cursor = skip_whitespace_comments(source, after_url)?;
    if cursor == source.len() {
        return Some((url, None));
    }
    let media = source[cursor..].trim();
    if media.is_empty() || starts_unsupported_import_clause(media) || !media_tail_is_valid(media) {
        return None;
    }
    Some((url, Some(media.to_owned())))
}

fn media_tail_is_valid(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut parentheses = 0u32;
    let mut brackets = 0u32;
    let mut component = false;
    while cursor < bytes.len() {
        if source[cursor..].starts_with("<!--") || source[cursor..].starts_with("-->") {
            return false;
        }
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            let Some(end) = source[cursor + 2..].find("*/") else {
                return false;
            };
            cursor += end + 4;
            continue;
        }
        if matches!(bytes[cursor], b'\'' | b'"') {
            let Some(next) = skip_string(source, cursor) else {
                return false;
            };
            component = true;
            cursor = next;
            continue;
        }
        if bytes[cursor] == b'\\' {
            let Some((next, _)) = consume_escape(source, cursor) else {
                return false;
            };
            component = true;
            cursor = next;
            continue;
        }
        match bytes[cursor] {
            b'(' => {
                parentheses = parentheses.saturating_add(1);
                component = true;
            }
            b')' => {
                if parentheses == 0 {
                    return false;
                }
                parentheses -= 1;
            }
            b'[' => {
                brackets = brackets.saturating_add(1);
                component = true;
            }
            b']' => {
                if brackets == 0 {
                    return false;
                }
                brackets -= 1;
            }
            b',' if parentheses == 0 && brackets == 0 => {
                if !component {
                    return false;
                }
                component = false;
            }
            b'{' | b'}' | b';' => return false,
            byte if !byte.is_ascii_whitespace() => component = true,
            _ => {}
        }
        cursor += 1;
    }
    parentheses == 0 && brackets == 0 && component
}

fn starts_unsupported_import_clause(source: &str) -> bool {
    let Some(cursor) = skip_ignored_prefix(source, 0) else {
        return true;
    };
    let Some((after_name, name)) = parse_identifier(source, cursor) else {
        return true;
    };
    if name.eq_ignore_ascii_case("layer") {
        return true;
    }
    if !name.eq_ignore_ascii_case("supports") {
        return false;
    }
    let Some(after_name) = skip_ignored_prefix(source, after_name) else {
        return true;
    };
    source.as_bytes().get(after_name) == Some(&b'(')
}

fn parse_string_value(source: &str, quote_start: usize) -> Option<(String, usize)> {
    let quote = *source.as_bytes().get(quote_start)?;
    let mut cursor = quote_start + 1;
    let mut value = String::new();
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            byte if byte == quote => return Some((value, cursor + 1)),
            b'\\' => {
                let (next, decoded) = consume_escape(source, cursor)?;
                if let Some(ch) = decoded {
                    value.push(ch);
                }
                cursor = next;
            }
            b'\n' | b'\r' | b'\x0c' => return None,
            _ => {
                let ch = source[cursor..].chars().next()?;
                value.push(ch);
                cursor += ch.len_utf8();
            }
        }
    }
    None
}

fn skip_ascii_whitespace(source: &str, mut cursor: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    Some(cursor)
}

fn skip_whitespace_comments(source: &str, mut cursor: usize) -> Option<usize> {
    loop {
        let bytes = source.as_bytes();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if source.get(cursor..)?.starts_with("/*") {
            let end = source[cursor + 2..].find("*/")?;
            cursor += end + 4;
            continue;
        }
        return Some(cursor);
    }
}

fn parse_url_function(source: &str, open_paren: usize) -> Option<(String, usize)> {
    let mut cursor = skip_ascii_whitespace(source, open_paren + 1)?;
    if source.as_bytes().get(cursor) == Some(&b'\'') || source.as_bytes().get(cursor) == Some(&b'"')
    {
        let (value, after_string) = parse_string_value(source, cursor)?;
        cursor = skip_whitespace_comments(source, after_string)?;
        return (source.as_bytes().get(cursor) == Some(&b')')).then_some((value, cursor + 1));
    }

    let mut value = String::new();
    let mut trailing_whitespace = false;
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            b')' => {
                let trimmed = value.trim_end_matches(|ch: char| ch.is_ascii_whitespace());
                if trimmed.is_empty() {
                    return None;
                }
                return Some((trimmed.to_owned(), cursor + 1));
            }
            byte if byte.is_ascii_whitespace() => {
                if !value.is_empty() {
                    trailing_whitespace = true;
                }
                cursor += 1;
            }
            b'\\' => {
                if trailing_whitespace {
                    return None;
                }
                let (next, decoded) = consume_escape(source, cursor)?;
                if let Some(ch) = decoded {
                    value.push(ch);
                }
                cursor = next;
            }
            b'\'' | b'"' | b'(' | b'\n' | b'\r' | b'\x0c' | 0 => return None,
            _ if trailing_whitespace => return None,
            _ => {
                let ch = source[cursor..].chars().next()?;
                value.push(ch);
                cursor += ch.len_utf8();
            }
        }
    }
    None
}

fn is_css_content_type(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|essence| essence.eq_ignore_ascii_case("text/css"))
}

fn has_credentials(url: &Url) -> bool {
    !url.username().is_empty() || url.password().is_some()
}

pub(crate) fn redacted_url(url: &Url) -> Url {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted
}

/// Keep provider-controlled diagnostics out of parse warnings. URLs are
/// already carried in structured warning fields, and free-form network errors
/// may contain credentials, headers, or other secrets supplied by a provider.
pub(crate) fn network_error_summary(error: &NetworkError) -> String {
    match error {
        NetworkError::Aborted => "request aborted".to_owned(),
        NetworkError::PolicyViolation(_) => "request denied by network policy".to_owned(),
        NetworkError::Io(_) => "network I/O error".to_owned(),
        NetworkError::Http(status) => format!("HTTP status error: {status}"),
        NetworkError::Other(_) => "provider returned a network error".to_owned(),
        _ => "provider returned an unknown network error".to_owned(),
    }
}

/// Policy details are provider-controlled free text. Retain the structured
/// violation type and redacted URL, but replace the free-form detail with a
/// fixed message so warning consumers cannot receive leaked credentials or
/// tokens from an untrusted provider.
pub(crate) fn sanitize_policy_violation(mut violation: PolicyViolation) -> PolicyViolation {
    violation.url = redacted_url(&violation.url);
    violation.details = "network policy denied the request".to_owned();
    violation
}

fn normalize_base_url(url: Option<&Url>) -> Option<Url> {
    let url = url?;
    if url.as_str().len() > MAX_IMPORT_URL_LENGTH || has_credentials(url) {
        return None;
    }
    Some(canonical_url(url.clone()))
}

fn canonical_url(mut url: Url) -> Url {
    url.set_fragment(None);
    url
}

fn resolve_url(href: &str, base: Option<&Url>) -> Option<Url> {
    match base {
        Some(base) => base.join(href).ok(),
        None => Url::parse(href).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use raikiri_traits::{
        FetchOutcome, FetchedResource, NetworkError, PolicyViolation, ResourceKind, ViolationType,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MapProvider {
        responses: Mutex<HashMap<String, String>>,
        requests: Mutex<Vec<String>>,
        max_depth: Option<u32>,
        content_type: Option<String>,
    }

    impl MapProvider {
        fn new(responses: &[(&str, &str)]) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .iter()
                        .map(|(url, css)| ((*url).to_owned(), (*css).to_owned()))
                        .collect(),
                ),
                requests: Mutex::new(Vec::new()),
                max_depth: None,
                content_type: Some("text/css".to_owned()),
            }
        }

        fn with_content_type(mut self, content_type: Option<&str>) -> Self {
            self.content_type = content_type.map(str::to_owned);
            self
        }

        fn with_max_depth(mut self, max_depth: u32) -> Self {
            self.max_depth = Some(max_depth);
            self
        }
    }

    impl NetworkProvider for MapProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            self.requests.lock().unwrap().push(request.url.to_string());
            let Some(css) = self
                .responses
                .lock()
                .unwrap()
                .get(request.url.as_str())
                .cloned()
            else {
                return Err(NetworkError::Other("not found".to_owned()));
            };
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: Bytes::from(css),
                content_type: self.content_type.clone(),
                final_url: request.url,
                encoding: None,
            }))
        }

        fn max_import_depth(&self) -> Option<u32> {
            self.max_depth
        }
    }

    #[test]
    fn scanner_preserves_only_leading_imports_and_decodes_urls() {
        let source = r#"/* c */ @import "a.css"; @import url('b.css') screen; p { color: red } @import "late.css";"#;
        let imports = scan_leading_imports(source);
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].url, "a.css");
        assert_eq!(imports[0].media, None);
        assert_eq!(imports[1].url, "b.css");
        assert_eq!(imports[1].media.as_deref(), Some("screen"));
    }

    #[test]
    fn a_stylesheet_bom_does_not_hide_a_leading_import() {
        let imports = scan_leading_imports("\u{feff}@import \"a.css\";");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].url, "a.css");
    }

    #[test]
    fn unknown_rules_and_empty_layer_statements_do_not_block_imports() {
        let source = r#"@unknown { ignored: true } @layer foo; @import "a.css";"#;
        let imports = scan_leading_imports(source);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].url, "a.css");

        let blocked = scan_leading_imports(r#"@media print {} @import "a.css";"#);
        assert!(blocked.is_empty());

        let with_url_brace = scan_leading_imports(
            r#"@unknown { value: url(foo}@import-nested.css;); } @import "top.css";"#,
        );
        assert_eq!(with_url_brace.len(), 1);
        assert_eq!(with_url_brace[0].url, "top.css");
    }

    #[test]
    fn layered_and_supports_imports_remain_opaque_for_now() {
        assert!(scan_leading_imports(r#"@import "a.css" layer(foo);"#).is_empty());
        assert!(scan_leading_imports(r#"@import "a.css" supports(display: grid);"#).is_empty());
    }

    #[test]
    fn scanner_handles_escaped_url_and_braces_inside_url() {
        let source = r#"@import url("dir/\7b theme\7d .css");"#;
        let imports = scan_leading_imports(source);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].url, "dir/{theme}.css");
    }

    #[test]
    fn malformed_import_and_unterminated_prefix_are_not_rewritten() {
        assert!(scan_leading_imports(r#"@import url("bad); @import "later.css";"#).is_empty());
        assert!(scan_leading_imports(r#"@import "a.css"; /* unterminated"#).len() == 1);
    }

    #[test]
    fn expansion_inlines_at_import_position_and_wraps_media() {
        let provider = MapProvider::new(&[
            ("https://example.test/a.css", "a { color: red }"),
            ("https://example.test/b.css", "b { color: blue }"),
        ]);
        let source = r#"@import "a.css"; @import url("b.css") screen; c { color: green }"#;
        let mut warnings = Vec::new();
        let expanded = expand_stylesheet_imports(
            source,
            Some(&Url::parse("https://example.test/main.css").unwrap()),
            None,
            Some(&provider),
            &mut warnings,
        );
        assert_eq!(
            expanded,
            "a { color: red } @media screen{b { color: blue }} c { color: green }"
        );
        assert_eq!(
            *provider.requests.lock().unwrap(),
            vec![
                "https://example.test/a.css".to_owned(),
                "https://example.test/b.css".to_owned()
            ]
        );
    }

    #[test]
    fn failed_import_remains_opaque_and_cycles_stop() {
        let provider = MapProvider::new(&[(
            "https://example.test/a.css",
            "@import \"main.css\"; a { color: red }",
        )]);
        let source = r#"@import "missing.css"; @import "a.css";"#;
        let mut warnings = Vec::new();
        let expanded = expand_stylesheet_imports(
            source,
            Some(&Url::parse("https://example.test/main.css").unwrap()),
            Some(&Url::parse("https://example.test/main.css").unwrap()),
            Some(&provider),
            &mut warnings,
        );
        assert!(expanded.contains(r#"@import "missing.css";"#));
        assert!(expanded.contains(r#"@import "main.css";"#));
        assert!(expanded.contains("a { color: red }"));
        assert_eq!(
            *provider.requests.lock().unwrap(),
            vec![
                "https://example.test/missing.css".to_owned(),
                "https://example.test/a.css".to_owned(),
            ]
        );
        assert!(
            warnings[0]
                .details
                .contains("provider returned a network error")
        );
        assert!(!warnings[0].details.contains("not found"));
    }

    #[test]
    fn non_css_import_response_remains_opaque() {
        let provider = MapProvider::new(&[("https://example.test/a.css", "a { color: red }")])
            .with_content_type(Some("text/html"));
        let source = r#"@import "a.css"; p { color: blue }"#;
        let mut warnings = Vec::new();
        let expanded = expand_stylesheet_imports(
            source,
            Some(&Url::parse("https://example.test/main.css").unwrap()),
            None,
            Some(&provider),
            &mut warnings,
        );
        assert_eq!(expanded, source);
        assert!(matches!(
            &warnings[0].kind,
            WarningKind::NetworkFallback { .. }
        ));
    }

    #[test]
    fn policy_failures_remain_opaque_and_are_reported_as_policy_warnings() {
        struct PolicyProvider;

        impl NetworkProvider for PolicyProvider {
            fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
                Err(NetworkError::PolicyViolation(PolicyViolation {
                    kind: ResourceKind::StylesheetImport,
                    url: request.url,
                    violation_type: ViolationType::HostNotAllowed,
                    details: "blocked https://user:secret@example.test/token".to_owned(),
                }))
            }
        }

        let provider = PolicyProvider;
        let source = r#"@import "a.css"; p { color: blue }"#;
        let mut warnings = Vec::new();
        let expanded = expand_stylesheet_imports(
            source,
            Some(&Url::parse("https://example.test/main.css").unwrap()),
            None,
            Some(&provider),
            &mut warnings,
        );
        assert_eq!(expanded, source);
        assert!(matches!(
            &warnings[0].kind,
            WarningKind::PolicyWarning { .. }
        ));
        assert_eq!(
            warnings[0].details,
            "stylesheet @import fetch violated network policy for https://example.test/a.css"
        );
        let WarningKind::PolicyWarning { violation } = &warnings[0].kind else {
            unreachable!("matched above");
        };
        assert_eq!(violation.details, "network policy denied the request");
    }

    #[test]
    fn fragments_do_not_bypass_active_import_cycle_detection() {
        let provider = MapProvider::new(&[(
            "https://example.test/a.css",
            r#"@import "main.css#fragment"; a { color: red }"#,
        )]);
        let mut warnings = Vec::new();
        let expanded = expand_stylesheet_imports(
            r#"@import "a.css#fragment";"#,
            Some(&Url::parse("https://example.test/main.css#root").unwrap()),
            Some(&Url::parse("https://example.test/main.css#root").unwrap()),
            Some(&provider),
            &mut warnings,
        );
        assert!(expanded.contains("a { color: red }"));
        assert!(expanded.contains(r#"@import "main.css#fragment";"#));
        assert_eq!(
            *provider.requests.lock().unwrap(),
            vec!["https://example.test/a.css".to_owned()]
        );
    }

    #[test]
    fn malformed_child_is_not_allowed_to_consume_parent_source() {
        let provider = MapProvider::new(&[("https://example.test/a.css", "a { color: red")]);
        let source = r#"@import "a.css"; p { color: blue }"#;
        let mut warnings = Vec::new();
        let expanded = expand_stylesheet_imports(
            source,
            Some(&Url::parse("https://example.test/main.css").unwrap()),
            None,
            Some(&provider),
            &mut warnings,
        );
        assert_eq!(expanded, source);
        assert!(matches!(
            &warnings[0].kind,
            WarningKind::NetworkFallback { .. }
        ));
    }

    #[test]
    fn malformed_url_tokens_are_not_fetched() {
        assert!(scan_leading_imports(r#"@import url(foo bar);"#).is_empty());
        assert!(scan_leading_imports(r#"@import url(foo(bar));"#).is_empty());
        assert!(scan_leading_imports(r#"@import url(foo) screen,,print;"#).is_empty());
        assert_eq!(
            scan_leading_imports(r#"@import url(foo\ bar);"#)[0].url,
            "foo bar"
        );
        assert_eq!(
            scan_leading_imports(r#"@import url(/**/foo.css);"#)[0].url,
            "/**/foo.css"
        );
        assert_eq!(
            scan_leading_imports(r#"@import url( /*x*/foo.css);"#)[0].url,
            "/*x*/foo.css"
        );
    }

    #[test]
    fn nested_imports_use_redirected_final_url_as_their_base() {
        struct RedirectProvider {
            requests: Mutex<Vec<String>>,
        }

        impl NetworkProvider for RedirectProvider {
            fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
                let requested = request.url.to_string();
                self.requests.lock().unwrap().push(requested.clone());
                match requested.as_str() {
                    "https://origin.test/styles/nested.css" => {
                        Ok(FetchOutcome::Body(FetchedResource {
                            bytes: Bytes::from_static(
                                b"@import \"grand.css\"; .nested { color: red }",
                            ),
                            content_type: Some("text/css".to_owned()),
                            final_url: Url::parse("https://cdn.test/assets/nested.css").unwrap(),
                            encoding: None,
                        }))
                    }
                    "https://cdn.test/assets/grand.css" => {
                        Ok(FetchOutcome::Body(FetchedResource {
                            bytes: Bytes::from_static(b".grand { color: blue }"),
                            content_type: Some("text/css".to_owned()),
                            final_url: Url::parse("https://cdn.test/assets/grand.css").unwrap(),
                            encoding: None,
                        }))
                    }
                    _ => Err(NetworkError::Other("not found".to_owned())),
                }
            }
        }

        let provider = RedirectProvider {
            requests: Mutex::new(Vec::new()),
        };
        let mut warnings = Vec::new();
        let expanded = expand_stylesheet_imports(
            r#"@import "nested.css";"#,
            Some(&Url::parse("https://origin.test/styles/main.css").unwrap()),
            Some(&Url::parse("https://origin.test/styles/main.css").unwrap()),
            Some(&provider),
            &mut warnings,
        );
        assert!(expanded.contains(".grand { color: blue }"));
        assert!(expanded.contains(".nested { color: red }"));
        assert_eq!(
            *provider.requests.lock().unwrap(),
            vec![
                "https://origin.test/styles/nested.css".to_owned(),
                "https://cdn.test/assets/grand.css".to_owned(),
            ]
        );
    }

    #[test]
    fn child_late_import_does_not_block_a_following_parent_import() {
        let provider = MapProvider::new(&[
            (
                "https://example.test/a.css",
                r#"a { color: red } @import "late.css";"#,
            ),
            ("https://example.test/b.css", "b { color: blue }"),
        ]);
        let mut warnings = Vec::new();
        let expanded = expand_stylesheet_imports(
            r#"@import "a.css"; @import "b.css";"#,
            Some(&Url::parse("https://example.test/main.css").unwrap()),
            None,
            Some(&provider),
            &mut warnings,
        );
        assert!(expanded.contains("a { color: red }"));
        assert!(expanded.contains("b { color: blue }"));
        assert!(expanded.contains(r#"@import "late.css";"#));
        assert_eq!(
            *provider.requests.lock().unwrap(),
            vec![
                "https://example.test/a.css".to_owned(),
                "https://example.test/b.css".to_owned(),
            ]
        );
    }

    #[test]
    fn provider_import_depth_limit_overrides_the_fallback_bound() {
        let provider = MapProvider::new(&[(
            "https://example.test/a.css",
            r#"@import "b.css"; a { color: red }"#,
        )])
        .with_max_depth(1);
        let mut warnings = Vec::new();
        let expanded = expand_stylesheet_imports(
            r#"@import "a.css";"#,
            Some(&Url::parse("https://example.test/main.css").unwrap()),
            Some(&Url::parse("https://example.test/main.css").unwrap()),
            Some(&provider),
            &mut warnings,
        );
        assert!(expanded.contains(r#"@import "b.css";"#));
        assert!(expanded.contains("a { color: red }"));
        assert_eq!(
            *provider.requests.lock().unwrap(),
            vec!["https://example.test/a.css".to_owned()]
        );
    }

    #[test]
    fn shared_budget_caps_imports_across_independent_roots() {
        let provider = MapProvider::new(&[("https://example.test/a.css", ".a { color: red }")]);
        let mut warnings = Vec::new();
        let mut budget = ImportBudget::default();
        let source = r#"@import "a.css";"#;
        let base = Url::parse("https://example.test/root.css").unwrap();

        for _ in 0..(MAX_IMPORT_FETCHES + 8) {
            let _ = expand_stylesheet_imports_with_budget(
                source,
                Some(&base),
                None,
                Some(&provider),
                &mut warnings,
                &mut budget,
            );
        }

        assert_eq!(
            provider.requests.lock().unwrap().len(),
            MAX_IMPORT_FETCHES,
            "independent stylesheet roots must share the document import budget"
        );
    }

    #[test]
    fn shared_budget_stops_fetching_after_cumulative_response_limit() {
        struct LargeProvider {
            response: Bytes,
            requests: Mutex<usize>,
        }

        impl NetworkProvider for LargeProvider {
            fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
                *self.requests.lock().unwrap() += 1;
                Ok(FetchOutcome::Body(FetchedResource {
                    bytes: self.response.clone(),
                    content_type: Some("text/css".to_owned()),
                    final_url: request.url,
                    encoding: None,
                }))
            }
        }

        let provider = LargeProvider {
            response: Bytes::from(vec![b' '; MAX_IMPORT_RESPONSE_BYTES]),
            requests: Mutex::new(0),
        };
        let mut warnings = Vec::new();
        let mut budget = ImportBudget::default();
        let source = r#"@import "a.css";"#;
        let base = Url::parse("https://example.test/root.css").unwrap();

        for _ in 0..3 {
            let _ = expand_stylesheet_imports_with_budget(
                source,
                Some(&base),
                None,
                Some(&provider),
                &mut warnings,
                &mut budget,
            );
        }

        assert_eq!(*provider.requests.lock().unwrap(), 2);
        assert_eq!(budget.response_bytes, MAX_IMPORT_RESPONSE_BYTES_TOTAL);
        assert_eq!(budget.expansion_bytes, MAX_IMPORT_EXPANSION_BYTES);
    }

    #[test]
    fn depth_limit_leaves_the_next_import_untouched() {
        let urls: Vec<String> = (0..=MAX_IMPORT_DEPTH + 1)
            .map(|depth| format!("https://example.test/{depth}.css"))
            .collect();
        let responses: Vec<(&str, &str)> = urls
            .windows(2)
            .map(|pair| {
                // Leak only test fixture strings; the process owns them for the
                // duration of this test.
                let css = format!("@import url(\"{}\");", pair[1]);
                let url: &'static str = Box::leak(pair[0].clone().into_boxed_str());
                let css: &'static str = Box::leak(css.into_boxed_str());
                (url, css)
            })
            .collect();
        let provider = MapProvider::new(&responses);
        let source = format!("@import url(\"{}\");", urls[0]);
        let mut warnings = Vec::new();
        let expanded = expand_stylesheet_imports(
            &source,
            Some(&Url::parse("https://example.test/root.css").unwrap()),
            Some(&Url::parse("https://example.test/root.css").unwrap()),
            Some(&provider),
            &mut warnings,
        );
        assert!(expanded.contains("@import"));
        assert_eq!(
            provider.requests.lock().unwrap().len(),
            MAX_IMPORT_DEPTH as usize
        );
    }
}
