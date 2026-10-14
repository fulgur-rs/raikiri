//! Resolver-driven expansion of CSS `@import` statements.
//!
//! The style crate deliberately has no network dependency.  Imports are therefore
//! expanded in the HTML parse layer, where the existing [`NetworkProvider`] and
//! document base URL are already available.  A successfully fetched import is
//! represented by separate stylesheet parts at the import position; failed, unresolvable, cyclic,
//! or depth-limited imports are copied unchanged so the style parser can retain
//! them as opaque at-rules.

use cssparser::{
    AtRuleParser, CowRcStr, ParseError, Parser, ParserInput, ParserState, QualifiedRuleParser,
    StyleSheetParser, Token, match_ignore_ascii_case,
};
use raikiri_traits::{NetworkError, NetworkProvider, RenderWarning, ResourceKind, WarningKind};
use url::Url;

use crate::fetch::{
    get_request, mime_essence, network_error_summary, policy_violation_warning_kind, redacted_url,
    resolve_url, url_without_fragment,
};
use crate::{StylesheetPart, StylesheetSource};

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
/// Deepest block nesting accepted in an `@import` media list. Validation
/// recurses once per block, so the bound keeps stack use fixed.
const MAX_MEDIA_NESTING: usize = 32;

#[derive(Debug)]
struct ImportStatement {
    start: usize,
    end: usize,
    url: String,
    media: Option<String>,
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
#[cfg(test)]
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
    .into_iter()
    .map(|part| part.source)
    .collect()
}

/// Expand imports while consuming the caller's document-wide budget.
pub(crate) fn expand_stylesheet_imports_with_budget(
    source: &str,
    base_url: Option<&Url>,
    root_url: Option<&Url>,
    network: Option<&dyn NetworkProvider>,
    warnings: &mut Vec<RenderWarning>,
    budget: &mut ImportBudget,
) -> Vec<StylesheetPart> {
    let Some(network) = network else {
        return unexpanded_parts(source, base_url);
    };

    let normalized_base = normalize_base_url(base_url);
    let normalized_root = normalize_base_url(root_url);
    let max_depth = network.max_import_depth().unwrap_or(MAX_IMPORT_DEPTH);
    if root_url.is_some() && normalized_root.is_none() {
        return unexpanded_parts(source, None);
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

/// Expand `@import` in live `<style>` sources collected from a mutated DOM.
///
/// The live `parse_fragment` path already expands imports into its own
/// `stylesheet_sources`, but live-DOM consumers keep only the fragment DOM
/// and re-derive author sheets from connected `<style>` text at flush.
/// Expanding here keeps an `innerHTML`-inserted `@import` working the same
/// way as a parsed one.
///
/// Shares one document-wide budget across `sources`, like the document parse
/// does. Fetch diagnostics are discarded: the fragment path also drops them
/// with the fragment's own warnings, keeping the resolved parts.
pub fn expand_live_stylesheet_imports(
    sources: Vec<String>,
    base_url: Option<&Url>,
    network: Option<&dyn NetworkProvider>,
) -> Vec<StylesheetSource> {
    let Some(network) = network else {
        return sources
            .into_iter()
            .map(|source| StylesheetSource::new(source, None))
            .collect();
    };
    let mut warnings = Vec::new();
    let mut budget = ImportBudget::default();
    sources
        .iter()
        .map(|source| StylesheetSource {
            parts: expand_stylesheet_imports_with_budget(
                source,
                base_url,
                None,
                Some(network),
                &mut warnings,
                &mut budget,
            ),
            media: None,
        })
        .collect()
}

/// One part holding `source`, with relative `url()` values resolved against
/// the stylesheet's `base_url` when it is known.
fn unexpanded_parts(source: &str, base_url: Option<&Url>) -> Vec<StylesheetPart> {
    vec![StylesheetPart {
        source: with_absolute_urls(source, base_url),
        media: Vec::new(),
    }]
}

fn with_absolute_urls(source: &str, base_url: Option<&Url>) -> String {
    match base_url {
        Some(base) => crate::css_urls::absolutize_urls(source, base).into_owned(),
        None => source.to_owned(),
    }
}

impl ImportExpander<'_> {
    fn expand(&mut self, source: &str, base_url: Option<&Url>, depth: u32) -> Vec<StylesheetPart> {
        if depth >= self.max_depth {
            return unexpanded_parts(source, base_url);
        }

        let imports = scan_leading_imports(source);
        if imports.is_empty() {
            return unexpanded_parts(source, base_url);
        }

        let mut expanded = Vec::new();
        let mut pending = String::new();
        let mut cursor = 0;
        for import in imports {
            pending.push_str(&source[cursor..import.start]);
            if let Some(mut child) = self.resolve_import(&import, base_url, depth) {
                if !pending.is_empty() {
                    expanded.push(StylesheetPart {
                        source: with_absolute_urls(&std::mem::take(&mut pending), base_url),
                        media: Vec::new(),
                    });
                }
                expanded.append(&mut child);
            } else {
                pending.push_str(&source[import.start..import.end]);
            }
            cursor = import.end;
        }
        pending.push_str(&source[cursor..]);
        if !pending.is_empty() {
            expanded.push(StylesheetPart {
                source: with_absolute_urls(&pending, base_url),
                media: Vec::new(),
            });
        }
        expanded
    }

    fn record_import_fetch_error(&mut self, url: Url, error: NetworkError) {
        let safe_url = redacted_url(&url);
        let summary = network_error_summary(&error);
        let (kind, details) = match error {
            NetworkError::PolicyViolation(violation) => {
                let kind = policy_violation_warning_kind(&violation, violation.kind);
                let reason = if matches!(kind, WarningKind::ResourceLimitExceeded { .. }) {
                    "resource response exceeded a byte limit"
                } else {
                    "fetch violated network policy"
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
    ) -> Option<Vec<StylesheetPart>> {
        if import.url.len() > MAX_IMPORT_URL_LENGTH {
            return None;
        }
        let requested_url = url_without_fragment(&resolve_url(&import.url, base_url)?);
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
        let fetched = match self.network.fetch(get_request(
            requested_url.clone(),
            ResourceKind::StylesheetImport,
            None,
        )) {
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
        let fetched_url = url_without_fragment(&fetched.final_url);
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
        let chain_entries = if requested_url == fetched_url { 1 } else { 2 };
        self.chain.push(requested_url);
        if chain_entries == 2 {
            self.chain.push(fetched_url.clone());
        }
        let child = self.expand(&css, Some(&fetched_url), depth + 1);
        for _ in 0..chain_entries {
            let _ = self.chain.pop();
        }

        let replacement_bytes = child.iter().try_fold(0usize, |total, part| {
            let total = total.checked_add(part.source.len())?;
            part.media
                .iter()
                .try_fold(total, |total, media| total.checked_add(media.len()))
        })?;
        let parent_media_bytes = match &import.media {
            Some(media) => media.len().checked_mul(child.len())?,
            None => 0,
        };
        let replacement_bytes = replacement_bytes.checked_add(parent_media_bytes)?;
        let total = self.budget.expansion_bytes.checked_add(replacement_bytes)?;
        if total > MAX_IMPORT_EXPANSION_BYTES {
            return None;
        }

        // Check the full cost before copying the parent condition into parts.
        let mut child = child;
        if let Some(media) = &import.media {
            for part in &mut child {
                part.media.push(media.clone());
            }
        }
        self.budget.expansion_bytes = total;
        Some(child)
    }
}

/// At-rules that make a later `@import` misplaced.
///
/// CSS import placement is checked against recognized rules. Unknown at-rules
/// are retained by the style parser but, like browser error recovery, do not
/// by themselves make a later import unusable.
fn is_import_barrier(name: &str) -> bool {
    match_ignore_ascii_case! { name,
        "color-profile" | "container" | "counter-style" | "custom-media" | "document"
        | "font-face" | "font-feature-values" | "font-palette-values" | "keyframes"
        | "namespace" | "page" | "property" | "scope" | "starting-style" | "supports"
        | "scroll-timeline" | "timeline-scope" | "viewport" | "view-timeline"
        | "view-transition" | "position-try" | "nest" | "when" | "else" | "media"
        | "-moz-document" | "-webkit-keyframes" => true,
        _ => false,
    }
}

/// How one top-level rule affects the imports that follow it.
enum LeadingRule {
    /// A statement-form `@import`. `target` is `None` when the prelude is
    /// malformed or uses a clause this layer does not expand; the rule is then
    /// kept as written.
    Import {
        start: usize,
        target: Option<(String, Option<String>)>,
    },
    /// `@charset`, a `@layer` statement, or an unrecognized at-rule.
    Neutral,
    /// A recognized rule after which imports are ignored.
    Barrier,
}

enum LeadingPrelude {
    Import(Option<(String, Option<String>)>),
    Layer,
    Neutral,
    Barrier,
}

/// Classifies top-level rules for [`scan_leading_imports`].
struct LeadingRuleParser;

impl<'i> AtRuleParser<'i> for LeadingRuleParser {
    type Prelude = LeadingPrelude;
    type AtRule = LeadingRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<LeadingPrelude, ParseError<'i, ()>> {
        let prelude = match_ignore_ascii_case! { &name,
            "import" => LeadingPrelude::Import(parse_import_prelude(input)),
            "layer" => LeadingPrelude::Layer,
            "charset" => LeadingPrelude::Neutral,
            _ => if is_import_barrier(&name) {
                LeadingPrelude::Barrier
            } else {
                LeadingPrelude::Neutral
            },
        };
        skip_rest(input);
        Ok(prelude)
    }

    fn rule_without_block(
        &mut self,
        prelude: LeadingPrelude,
        start: &ParserState,
    ) -> Result<LeadingRule, ()> {
        Ok(match prelude {
            LeadingPrelude::Import(target) => LeadingRule::Import {
                start: start.position().byte_index(),
                target,
            },
            // A layer statement is permitted before imports.
            LeadingPrelude::Layer | LeadingPrelude::Neutral => LeadingRule::Neutral,
            LeadingPrelude::Barrier => LeadingRule::Barrier,
        })
    }

    fn parse_block<'t>(
        &mut self,
        prelude: LeadingPrelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<LeadingRule, ParseError<'i, ()>> {
        skip_rest(input);
        // Imports nested in any block are not top-level imports. Unknown
        // wrappers do not prevent a later top-level import from being
        // processed; recognized block at-rules, a `@layer` block included, do.
        Ok(match prelude {
            LeadingPrelude::Import(_) | LeadingPrelude::Neutral => LeadingRule::Neutral,
            LeadingPrelude::Layer | LeadingPrelude::Barrier => LeadingRule::Barrier,
        })
    }
}

impl<'i> QualifiedRuleParser<'i> for LeadingRuleParser {
    type Prelude = ();
    type QualifiedRule = LeadingRule;
    type Error = ();

    fn parse_prelude<'t>(&mut self, input: &mut Parser<'i, 't>) -> Result<(), ParseError<'i, ()>> {
        skip_rest(input);
        Ok(())
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: (),
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<LeadingRule, ParseError<'i, ()>> {
        skip_rest(input);
        Ok(LeadingRule::Barrier)
    }
}

/// Consume the remaining tokens so a nested or delimited parser ends
/// exhausted. Nested blocks are skipped by the next call to `next`.
fn skip_rest(input: &mut Parser<'_, '_>) {
    while input.next().is_ok() {}
}

/// Find top-level imports before the first recognized rule that blocks them.
///
/// The scan is source-preserving: it records byte spans and the decoded URL,
/// while leaving every non-replaced byte untouched.
fn scan_leading_imports(source: &str) -> Vec<ImportStatement> {
    // A byte order mark is not part of the stylesheet's token stream.
    let offset = if source.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let mut input = ParserInput::new(&source[offset..]);
    let mut parser = Parser::new(&mut input);
    let mut rules = LeadingRuleParser;
    let mut sheet = StyleSheetParser::new(&mut parser, &mut rules);
    let mut imports = Vec::new();
    while let Some(rule) = sheet.next() {
        match rule {
            Ok(LeadingRule::Import { start, target }) => {
                if let Some((url, media)) = target {
                    imports.push(ImportStatement {
                        start: offset + start,
                        end: offset + sheet.input.position().byte_index(),
                        url,
                        media,
                    });
                }
            }
            Ok(LeadingRule::Neutral) => {}
            // A qualified rule, valid or not, makes later imports misplaced.
            Ok(LeadingRule::Barrier) | Err(_) => break,
        }
    }
    imports
}

/// Parse the prelude of `@import` into its URL and optional media list.
///
/// Returns `None` for a malformed prelude and for `layer` / `supports()`
/// clauses, which are not expanded here.
fn parse_import_prelude(input: &mut Parser<'_, '_>) -> Option<(String, Option<String>)> {
    let url = parse_import_url(input)?;
    if input.is_exhausted() {
        return Some((url, None));
    }
    if starts_unsupported_import_clause(input) {
        return None;
    }
    let start = input.position();
    if !media_tail_is_valid(input) {
        return None;
    }
    let media = input.slice_from(start).trim();
    (!media.is_empty()).then(|| (url, Some(media.to_owned())))
}

/// `<url>` or `<string>`. Unlike [`Parser::expect_url_or_string`], a quoted
/// `url()` must hold exactly one string.
fn parse_import_url(input: &mut Parser<'_, '_>) -> Option<String> {
    let url = match input.next().ok()?.clone() {
        Token::QuotedString(url) | Token::UnquotedUrl(url) => url.as_ref().to_owned(),
        Token::Function(name) if name.eq_ignore_ascii_case("url") => input
            .parse_nested_block(|input| {
                let url = input.expect_string()?.as_ref().to_owned();
                input.expect_exhausted()?;
                Ok::<_, ParseError<'_, ()>>(url)
            })
            .ok()?,
        _ => return None,
    };
    // An empty URL names no external resource; resolving it would fetch the
    // base URL itself.
    (!url.is_empty()).then_some(url)
}

fn starts_unsupported_import_clause(input: &mut Parser<'_, '_>) -> bool {
    let state = input.state();
    let unsupported = match input.next() {
        Ok(Token::Ident(name)) if name.eq_ignore_ascii_case("layer") => true,
        Ok(Token::Function(name)) => {
            name.eq_ignore_ascii_case("layer") || name.eq_ignore_ascii_case("supports")
        }
        // `supports (...)` is not the `supports()` function, but it is kept
        // opaque rather than misread as a media query.
        Ok(Token::Ident(name)) if name.eq_ignore_ascii_case("supports") => {
            matches!(input.next(), Ok(Token::ParenthesisBlock))
        }
        _ => false,
    };
    input.reset(&state);
    unsupported
}

/// Whether the rest of the prelude is a comma-separated list of non-empty
/// component sequences without bad or unmatched tokens.
fn media_tail_is_valid(input: &mut Parser<'_, '_>) -> bool {
    let mut component = false;
    loop {
        let Ok(token) = input.next() else {
            return component;
        };
        match token {
            Token::Comma => {
                if !component {
                    return false;
                }
                component = false;
            }
            Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                component = true;
                if !nested_block_is_valid(input, 1) {
                    return false;
                }
            }
            token if is_invalid_media_token(token) => return false,
            _ => component = true,
        }
    }
}

/// Validate the block the parser has just opened. `depth` counts the blocks
/// already entered; nesting beyond [`MAX_MEDIA_NESTING`] is rejected so
/// provider-controlled CSS cannot exhaust the stack.
fn nested_block_is_valid(input: &mut Parser<'_, '_>, depth: usize) -> bool {
    if depth > MAX_MEDIA_NESTING {
        return false;
    }
    input
        .parse_nested_block(|input| {
            if block_tokens_are_valid(input, depth) {
                Ok(())
            } else {
                Err(input.new_custom_error::<_, ()>(()))
            }
        })
        .is_ok()
}

fn block_tokens_are_valid(input: &mut Parser<'_, '_>, depth: usize) -> bool {
    while let Ok(token) = input.next() {
        match token {
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                if !nested_block_is_valid(input, depth + 1) {
                    return false;
                }
            }
            token if is_invalid_media_token(token) => return false,
            _ => {}
        }
    }
    true
}

fn is_invalid_media_token(token: &Token<'_>) -> bool {
    matches!(
        token,
        Token::BadUrl(_)
            | Token::BadString(_)
            | Token::CloseParenthesis
            | Token::CloseSquareBracket
            | Token::CloseCurlyBracket
            | Token::CDO
            | Token::CDC
    )
}

fn is_css_content_type(content_type: &str) -> bool {
    mime_essence(content_type).eq_ignore_ascii_case("text/css")
}

fn has_credentials(url: &Url) -> bool {
    !url.username().is_empty() || url.password().is_some()
}

fn normalize_base_url(url: Option<&Url>) -> Option<Url> {
    let url = url?;
    if url.as_str().len() > MAX_IMPORT_URL_LENGTH || has_credentials(url) {
        return None;
    }
    Some(url_without_fragment(url))
}

#[cfg(test)]
mod tests;
