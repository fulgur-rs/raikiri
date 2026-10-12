//! Vacuous-PASS audit for reftests.
//!
//! A reftest PASS shows that the test and its reference rasterize to the same
//! pixels; it does not show that the feature under test works. A PASS is
//! vacuous when the comparison cannot tell a working implementation from a
//! broken one, for example when both sides paint nothing or when the
//! declarations that only the test uses have no visible effect.
//!
//! [`audit_id`] re-runs a passing reftest with two kinds of probe per `rel=match`
//! reference:
//!
//! * **Blank probe.** The test is replaced by an empty document that keeps
//!   only the test's `<meta>` and reference `<link>` metadata. If the
//!   reference still matches, both sides render a blank page.
//! * **Ablation probes.** Author declarations (from `<style>` elements and
//!   `style` attributes) are removed from the test in three sets:
//!   the properties the test names as its subject in its file name or
//!   `rel=help` links, the properties the reference never uses, and every
//!   declaration the reference does not repeat. If the reference still
//!   matches after any of them, the removed declarations have no visible
//!   effect, so the PASS cannot tell whether they are implemented.
//!
//! The feature probe also flags negative tests, whose subject correctly has
//! no effect (a percentage height that behaves as `auto`, a float on the
//! root). Such a PASS guards against a wrong effect but does not show the
//! feature working, so the audit keeps it out of the promotion set.
//!
//! A test that only has `rel=mismatch` references is reported as vacuous
//! without probing: differing from one reference rarely establishes the
//! intended rendering. The probes are heuristics. Declarations in external
//! stylesheets or applied by script are not ablated, so a non-vacuous verdict
//! is weaker evidence when [`Verdict::Kept`] reports no test-only
//! declarations. Declarations are compared as source text without a cascade:
//! a reference declaration counts as used even inside an inactive `@media`
//! block, behind an unmatched selector or when overridden.

use crate::baseline_report::{Status, report_config, run_id_with_config};
use crate::reftest::{
    ReftestConfig, ReftestKind, ReftestPair, discover_pairs_for_file_with_wpt_root,
    run_pair_with_test_html,
};
use crate::runner::TestOutcome;
use std::collections::BTreeSet;
use std::ops::Range;
use std::path::Path;

/// Audit result for one test id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The test passes and at least one passing `rel=match` reference survives
    /// both probes. `ablated` counts the declarations removed by the ablation
    /// probe of that reference; zero means the test and reference share all
    /// inline declarations and only differ in markup or external resources.
    Kept {
        /// Declarations removed by the ablation probe.
        ablated: usize,
    },
    /// Every passing `rel=match` reference also matches an empty document.
    Blank,
    /// Every passing `rel=match` reference still matches after removing the
    /// test-only declarations.
    Unstyled {
        /// Which ablation still matched: `feature` (the properties the test
        /// names), `property` (properties the reference never uses), or
        /// `declaration` (declarations the reference does not repeat).
        probe: &'static str,
        /// Declarations removed by that probe.
        ablated: usize,
    },
    /// The test has no `rel=match` reference.
    MismatchOnly,
    /// A probe could not be rendered, so the audit cannot tell whether the
    /// PASS is vacuous.
    Inconclusive {
        /// The probe that failed to render.
        probe: &'static str,
    },
    /// The test does not pass, so it is not a promotion candidate.
    NotPass {
        /// Status reported by the baseline runner.
        status: Status,
        /// Runner detail for that status.
        detail: String,
    },
}

impl Verdict {
    /// Stable label written to the audit report.
    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Kept { .. } => "KEEP",
            Verdict::Blank => "VACUOUS-BLANK",
            Verdict::Unstyled { .. } => "VACUOUS-UNSTYLED",
            Verdict::MismatchOnly => "VACUOUS-MISMATCH-ONLY",
            Verdict::Inconclusive { .. } => "INCONCLUSIVE",
            Verdict::NotPass { .. } => "NOT-PASS",
        }
    }

    /// Human-readable detail written to the audit report.
    pub fn detail(&self) -> String {
        match self {
            Verdict::Kept { ablated: 0 } => "no test-only inline declarations".to_owned(),
            Verdict::Kept { ablated } => format!("{ablated} test-only declaration(s) ablated"),
            Verdict::Unstyled { probe, ablated } => {
                format!("still matches after the {probe} ablation of {ablated} declaration(s)")
            }
            Verdict::Blank => "reference matches an empty document".to_owned(),
            Verdict::MismatchOnly => "only rel=mismatch references".to_owned(),
            Verdict::Inconclusive { probe } => format!("the {probe} probe failed to render"),
            Verdict::NotPass { status, detail } => format!("{}: {detail}", status.as_str()),
        }
    }
}

/// Run `id` and, when it passes, probe each passing `rel=match` reference.
///
/// The id uses the baseline format: a root-relative WPT path with an optional
/// declared variant query.
pub fn audit_id(wpt_root: &Path, id: &str) -> Verdict {
    audit_id_with_config(wpt_root, id, report_config())
}

fn audit_id_with_config(wpt_root: &Path, id: &str, config: ReftestConfig) -> Verdict {
    let row = run_id_with_config(wpt_root, id, config);
    if row.status != Status::Pass {
        return Verdict::NotPass {
            status: row.status,
            detail: row.detail,
        };
    }
    probe_passing(wpt_root, id, config).unwrap_or_else(|detail| Verdict::NotPass {
        status: Status::Error,
        detail,
    })
}

/// Probe a test that the runner reported as passing. An `Err` carries the
/// detail of a file that could not be resolved, discovered or read.
fn probe_passing(wpt_root: &Path, id: &str, config: ReftestConfig) -> Result<Verdict, String> {
    let (path, query) = id.split_once('?').map_or((id, ""), |(p, q)| (p, q));
    let variant_query = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    // Dynamic reftests need absolute paths, as in the baseline runner.
    let resolve = |path: &Path| std::fs::canonicalize(path).map_err(|e| format!("resolve: {e}"));
    let wpt_root = resolve(wpt_root)?;
    let test_path = resolve(&wpt_root.join(path))?;
    let pairs = discover_pairs_for_file_with_wpt_root(&test_path, Some(&wpt_root))
        .map_err(|e| format!("discover: {e}"))?;
    let matches: Vec<&ReftestPair> = pairs
        .iter()
        .filter(|pair| pair.kind == ReftestKind::Match)
        .collect();
    if matches.is_empty() {
        return Ok(Verdict::MismatchOnly);
    }
    let read = |path: &Path| std::fs::read_to_string(path).map_err(|e| format!("read: {e}"));
    let test_html = read(&test_path)?;
    let blank = blank_document(&test_html);
    let features = feature_properties(&test_html, path);
    let by_feature = remove_declarations(&test_html, |key| features.contains(property_of(key)));
    let mut verdict: Option<Verdict> = None;
    for pair in matches {
        // Probe in the same resource mode in which the pair passes.
        let Some(resolve_images) = [true, false]
            .into_iter()
            .find(|&images| passes(pair, config, images, &variant_query, &test_html) == Some(true))
        else {
            continue;
        };
        let reference_html = read(&pair.reference)?;
        match passes(pair, config, resolve_images, &variant_query, &blank) {
            Some(true) => {
                verdict.get_or_insert(Verdict::Blank);
                continue;
            }
            Some(false) => {}
            None => return Ok(Verdict::Inconclusive { probe: "blank" }),
        }
        let reference = author_declarations(&reference_html);
        let reference_keys: BTreeSet<&str> = reference.iter().map(|d| d.key.as_str()).collect();
        let reference_properties: BTreeSet<&str> = reference.iter().map(|d| d.property()).collect();
        // Besides the feature properties, two broader ablations: the
        // declarations the reference does not repeat, and the properties the
        // reference never uses.
        let by_declaration = remove_declarations(&test_html, |key| !reference_keys.contains(key));
        let by_property = remove_declarations(&test_html, |key| {
            !reference_properties.contains(property_of(key))
        });
        let probes = [
            ("feature", &by_feature),
            ("property", &by_property),
            ("declaration", &by_declaration),
        ];
        let mut unstyled = None;
        for (probe, (html, removed)) in probes {
            if *removed == 0 {
                continue;
            }
            match passes(pair, config, resolve_images, &variant_query, html) {
                Some(true) => {
                    unstyled = Some((probe, *removed));
                    break;
                }
                Some(false) => {}
                None => return Ok(Verdict::Inconclusive { probe }),
            }
        }
        if let Some((probe, ablated)) = unstyled {
            if !matches!(verdict, Some(Verdict::Unstyled { .. })) {
                verdict = Some(Verdict::Unstyled { probe, ablated });
            }
            continue;
        }
        return Ok(Verdict::Kept {
            ablated: by_declaration.1,
        });
    }
    Ok(verdict.unwrap_or(Verdict::NotPass {
        status: Status::Fail,
        detail: "no rel=match reference passed when probed".to_owned(),
    }))
}

/// Properties declared in `test_html` that the test names as its subject:
/// the property spelled as whole hyphen-separated words in the file name or
/// in a `rel=help` link's last path segment or fragment. A name contained in
/// a longer matched name (`color` inside `background-color`) is dropped.
pub fn feature_properties(test_html: &str, test_id: &str) -> BTreeSet<String> {
    let mut subjects: Vec<String> = Vec::new();
    let file = test_id.rsplit('/').next().unwrap_or(test_id);
    subjects.push(file.split('.').next().unwrap_or(file).to_ascii_lowercase());
    for tag in tags(test_html) {
        let href = tag
            .attributes
            .iter()
            .find(|(name, _)| name == "href")
            .map(|(_, value)| test_html[value.clone()].to_ascii_lowercase());
        if tag.name == "link"
            && has_rel(test_html, &tag.attributes, "help")
            && let Some(href) = href
        {
            let (path, fragment) = href.split_once('#').unwrap_or((&href, ""));
            subjects.push(
                path.trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            );
            subjects.push(fragment.to_owned());
        }
    }
    let words: Vec<Vec<&str>> = subjects
        .iter()
        .map(|subject| {
            subject
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect()
        })
        .collect();
    let named = |property: &str| {
        // Vendor prefixes start with a hyphen; match their words only.
        let parts: Vec<&str> = property.split('-').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return false;
        }
        words.iter().any(|w| {
            w.windows(parts.len())
                .any(|window| window == parts.as_slice())
        })
    };
    let declared: BTreeSet<String> = author_declarations(test_html)
        .iter()
        .map(|decl| decl.property().to_owned())
        .filter(|property| !property.starts_with("--") && named(property))
        .collect();
    declared
        .iter()
        .filter(|property| {
            !declared.iter().any(|other| {
                other.len() > property.len()
                    && format!("-{other}-").contains(&format!("-{property}-"))
            })
        })
        .cloned()
        .collect()
}

/// Whether `pair` passes with `test_html` in place of the test file. `None`
/// means the run errored.
fn passes(
    pair: &ReftestPair,
    config: ReftestConfig,
    resolve_images: bool,
    variant_query: &str,
    test_html: &str,
) -> Option<bool> {
    let result =
        run_pair_with_test_html(pair, config, resolve_images, variant_query, test_html).ok()?;
    match result.outcome {
        TestOutcome::Pass => Some(true),
        TestOutcome::Fail(_) => Some(false),
        _ => None,
    }
}

/// An empty HTML document carrying the test's `<meta>` tags and reference
/// `<link>` tags, so that fuzzy and page-range metadata still apply.
pub fn blank_document(test_html: &str) -> String {
    let mut head = String::new();
    for tag in tags(test_html) {
        let reference = has_rel(test_html, &tag.attributes, "match")
            || has_rel(test_html, &tag.attributes, "mismatch");
        if tag.name == "meta" || (tag.name == "link" && reference) {
            head.push_str(&test_html[tag.range]);
            head.push('\n');
        }
    }
    format!("<!DOCTYPE html>\n<html><head>\n{head}</head><body></body></html>\n")
}

/// One author declaration found in an HTML document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// Byte range in the document, including a trailing `;` when present.
    pub range: Range<usize>,
    /// Property (lowercased unless custom) and whitespace-normalized value,
    /// `name:value`.
    pub key: String,
}

impl Declaration {
    /// The lowercased property name.
    pub fn property(&self) -> &str {
        property_of(&self.key)
    }
}

fn property_of(key: &str) -> &str {
    key.split_once(':').map_or(key, |(property, _)| property)
}

/// Collect declarations from `<style>` element contents and `style`
/// attributes. Comments and `<script>` contents are skipped.
pub fn author_declarations(html: &str) -> Vec<Declaration> {
    let mut out = Vec::new();
    for tag in tags(html) {
        for (attr, value) in &tag.attributes {
            if attr == "style" {
                // The attribute value is decoded before CSS parsing; scan the
                // decoded text and map ranges back to the source.
                let (css, map) = decode_character_references(&html[value.clone()]);
                out.extend(
                    css_declarations(&css, true, 0)
                        .into_iter()
                        .map(|decl| Declaration {
                            range: value.start + map[decl.range.start]
                                ..value.start + map[decl.range.end],
                            key: decl.key,
                        }),
                );
            }
        }
        if tag.name == "style"
            && let Some(content) = tag.raw_text
        {
            out.extend(css_declarations(
                &html[content.clone()],
                false,
                content.start,
            ));
        }
    }
    out
}

/// One start tag found by [`tags`].
struct Tag {
    /// Lowercased element name.
    name: String,
    /// Byte range of the whole start tag.
    range: Range<usize>,
    /// Attributes with the byte range of each value.
    attributes: Vec<(String, Range<usize>)>,
    /// Contents of a raw-text element (`style`, `script` and the like).
    raw_text: Option<Range<usize>>,
}

/// Start tags of `html` in document order. Comments are skipped, and the
/// contents of raw-text elements are not searched for tags, so markup inside
/// a comment or a script string does not count as an element.
fn tags(html: &str) -> Vec<Tag> {
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    // Open `<template>` elements whose contents are inert. Their styles and
    // metadata do not apply unless a script clones them. A declarative
    // shadow root template is not inert; its contents become the shadow tree.
    let mut inert_depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if lower[i..].starts_with("<!--") {
            i = comment_end(&lower, i + 4);
            continue;
        }
        if inert_depth > 0 && end_tag_at(&lower, i, "template") {
            inert_depth -= 1;
            i += "</template".len();
            continue;
        }
        let name_start = i + 1;
        let name_len = bytes[name_start..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || **b == b'-')
            .count();
        if name_len == 0 || !bytes[name_start].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let name = lower[name_start..name_start + name_len].to_owned();
        let Some(end) = tag_end(html, i) else {
            break;
        };
        let attributes = attributes(html, name_start + name_len, end);
        let raw_text = matches!(
            name.as_str(),
            "style"
                | "script"
                | "title"
                | "textarea"
                | "xmp"
                | "noscript"
                | "iframe"
                | "noembed"
                | "noframes"
                | "plaintext"
        )
        .then(|| {
            // Nothing ends `plaintext`; the rest of the document is text.
            let close = (name != "plaintext")
                .then(|| (end..bytes.len()).find(|&at| end_tag_at(&lower, at, &name)))
                .flatten();
            end..close.unwrap_or(bytes.len())
        });
        let next = raw_text.as_ref().map_or(end, |content| content.end);
        let inert = inert_depth > 0;
        if name == "template"
            && (inert
                || !attributes.iter().any(|(name, value)| {
                    name == "shadowrootmode"
                        && ["open", "closed"]
                            .iter()
                            .any(|mode| html[value.clone()].eq_ignore_ascii_case(mode))
                }))
        {
            inert_depth += 1;
        }
        if !inert {
            out.push(Tag {
                name,
                range: i..end,
                attributes,
                raw_text,
            });
        }
        i = next;
    }
    out
}

/// The offset just past a comment whose text starts at `from` (after
/// `<!--`). Like the HTML tokenizer, `<!-->` and `<!--->` end at once and
/// `--!>` also closes a comment.
fn comment_end(lower: &str, from: usize) -> usize {
    let rest = &lower[from..];
    if rest.starts_with('>') {
        return from + 1;
    }
    if rest.starts_with("->") {
        return from + 2;
    }
    [("-->", 3), ("--!>", 4)]
        .iter()
        .filter_map(|(close, len)| rest.find(close).map(|at| from + at + len))
        .min()
        .unwrap_or(lower.len())
}

/// Whether an end tag for `name` starts at `at`: `</name` followed by
/// whitespace, `/`, `>` or the end of input.
fn end_tag_at(lower: &str, at: usize, name: &str) -> bool {
    let rest = &lower.as_bytes()[at..];
    rest.starts_with(b"</")
        && rest[2..].starts_with(name.as_bytes())
        && rest
            .get(2 + name.len())
            .is_none_or(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>'))
}

/// Remove every declaration whose key satisfies `remove`. Returns the new
/// document and the number of removed declarations.
pub fn remove_declarations(html: &str, remove: impl Fn(&str) -> bool) -> (String, usize) {
    let mut out = String::with_capacity(html.len());
    let mut last = 0;
    let mut removed = 0;
    for decl in author_declarations(html) {
        if remove(&decl.key) {
            out.push_str(&html[last..decl.range.start]);
            last = decl.range.end;
            removed += 1;
        }
    }
    out.push_str(&html[last..]);
    (out, removed)
}

/// End offset (exclusive) of the tag that starts at `start`, honoring quoted
/// attribute values.
fn tag_end(html: &str, start: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut quote = None;
    for (offset, &byte) in bytes[start..].iter().enumerate() {
        match (quote, byte) {
            (Some(q), b) if b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(byte),
            (None, b'>') => return Some(start + offset + 1),
            (None, _) => {}
        }
    }
    None
}

/// Attributes of the tag spanning `attrs_start..tag_end`, as lowercased
/// names with the byte range of each value.
fn attributes(html: &str, attrs_start: usize, tag_end: usize) -> Vec<(String, Range<usize>)> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = attrs_start;
    while i < tag_end {
        while i < tag_end && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let name_start = i;
        while i < tag_end
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b'=' | b'>' | b'/')
        {
            i += 1;
        }
        let name = html[name_start..i].to_ascii_lowercase();
        while i < tag_end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= tag_end || bytes[i] != b'=' {
            if i == name_start {
                i += 1;
            } else if !out.iter().any(|(seen, _)| *seen == name) {
                out.push((name, i..i));
            }
            continue;
        }
        i += 1;
        while i < tag_end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = match bytes.get(i) {
            Some(&q @ (b'"' | b'\'')) => {
                let start = i + 1;
                let end = bytes[start..tag_end]
                    .iter()
                    .position(|&b| b == q)
                    .map_or(tag_end, |e| start + e);
                i = end + 1;
                start..end
            }
            _ => {
                let start = i;
                while i < tag_end && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                start..i
            }
        };
        // The HTML tokenizer keeps the first of duplicate attributes.
        if !out.iter().any(|(seen, _)| *seen == name) {
            out.push((name, value));
        }
    }
    out
}

/// Whether the `rel` attribute among `attrs` has the token `token`.
fn has_rel(html: &str, attrs: &[(String, Range<usize>)], token: &str) -> bool {
    attrs.iter().any(|(name, value)| {
        name == "rel"
            && html[value.clone()]
                .split_ascii_whitespace()
                .any(|t| t.eq_ignore_ascii_case(token))
    })
}

/// Declarations in CSS text. `inline` treats the top level as a declaration
/// list (a `style` attribute); otherwise only text inside blocks is
/// considered. Ranges are offset by `base`.
pub fn css_declarations(css: &str, inline: bool, base: usize) -> Vec<Declaration> {
    let bytes = css.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut nesting = 0usize;
    let mut segment_start = 0;
    let mut i = 0;
    let in_declarations = |depth: usize| inline || depth > 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |e| i + 2 + e + 2);
                continue;
            }
            b'\\' => {
                i += 2;
                continue;
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' | b'[' => nesting += 1,
            b')' | b']' => nesting = nesting.saturating_sub(1),
            // A `{}` block in a value (custom properties may hold one) is
            // part of the declaration, not a nested rule.
            b'{' if nesting > 0
                || (in_declarations(depth) && is_custom_property(&css[segment_start..i])) =>
            {
                nesting += 1;
            }
            b'}' if nesting > 0 => nesting -= 1,
            _ if nesting > 0 => {}
            b';' => {
                if in_declarations(depth) {
                    push_declaration(css, segment_start..i + 1, base, &mut out);
                }
                segment_start = i + 1;
            }
            b'{' => {
                depth += 1;
                segment_start = i + 1;
            }
            b'}' => {
                if in_declarations(depth) {
                    push_declaration(css, segment_start..i, base, &mut out);
                }
                depth = depth.saturating_sub(1);
                segment_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if in_declarations(depth) && segment_start < bytes.len() {
        push_declaration(css, segment_start..bytes.len(), base, &mut out);
    }
    out
}

/// Whether `segment`, the text of a declaration up to the current position,
/// already names a custom property and has reached its value.
fn is_custom_property(segment: &str) -> bool {
    segment
        .split_once(':')
        .is_some_and(|(name, _)| strip_comments(name).trim().starts_with("--"))
}

fn push_declaration(css: &str, range: Range<usize>, base: usize, out: &mut Vec<Declaration>) {
    let Some(text) = css.get(range.clone()) else {
        return;
    };
    let text = text.trim_end_matches(';');
    let Some(colon) = colon_outside_comments(text) else {
        return;
    };
    let (name, value) = (&text[..colon], &text[colon + 1..]);
    let name = strip_comments(name);
    let name = name.trim();
    // Identifier characters, including escapes and non-ASCII code points.
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'\\') || !b.is_ascii())
    {
        return;
    }
    let value = collapse_whitespace(value);
    if value.is_empty() {
        return;
    }
    out.push(Declaration {
        range: base + range.start..base + range.end,
        // Property names are ASCII case-insensitive, custom property names
        // are not. Values keep their case: strings, URLs and custom property
        // values are case-sensitive, and a keyword that differs only in case
        // merely makes the declaration count as test-only.
        key: if name.starts_with("--") {
            format!("{name}:{value}")
        } else {
            format!("{}:{value}", name.to_ascii_lowercase())
        },
    });
}

/// The offset of the first `:` in `text` outside CSS comments.
fn colon_outside_comments(text: &str) -> Option<usize> {
    let mut i = 0;
    while i < text.len() {
        if text.as_bytes()[i..].starts_with(b"/*") {
            i = text[i + 2..]
                .find("*/")
                .map_or(text.len(), |e| i + 2 + e + 2);
        } else if text.as_bytes()[i] == b':' {
            return Some(i);
        } else {
            i += 1;
        }
    }
    None
}

/// Decode numeric character references and the named references that can
/// change CSS tokenization; others are kept as written. The map gives the
/// source offset of every decoded byte, plus one entry for the end.
fn decode_character_references(text: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.len() + 1);
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let reference = rest.strip_prefix('&').and_then(|after| {
            let body_len = after
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'#')
                .count();
            let body = &after[..body_len];
            if !after[body_len..].starts_with(';') {
                return None;
            }
            let c = match body.strip_prefix('#') {
                Some(number) => match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse().ok(),
                }
                .and_then(char::from_u32),
                None => match body {
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "colon" => Some(':'),
                    "semi" => Some(';'),
                    "lpar" => Some('('),
                    "rpar" => Some(')'),
                    "lbrace" => Some('{'),
                    "rbrace" => Some('}'),
                    "sol" => Some('/'),
                    "ast" => Some('*'),
                    _ => None,
                },
            }?;
            Some((c, body_len + 2))
        });
        let (c, len) = reference.unwrap_or_else(|| {
            let c = rest.chars().next().unwrap_or_default();
            (c, c.len_utf8())
        });
        map.extend(std::iter::repeat_n(i, c.len_utf8()));
        out.push(c);
        i += len;
    }
    map.push(text.len());
    (out, map)
}

/// Normalize a declaration value: drop comments, trim, and collapse runs of
/// whitespace to one space. Quoted strings and unquoted `url()` contents are
/// data, not comments or insignificant whitespace, and are kept verbatim.
fn collapse_whitespace(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut pending_space = false;
    let mut i = 0;
    let push = |out: &mut String, text: &[char], pending_space: &mut bool| {
        if *pending_space && !out.is_empty() {
            out.push(' ');
        }
        *pending_space = false;
        out.extend(text);
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            pending_space = true;
            continue;
        }
        if c.is_whitespace() {
            pending_space = true;
            i += 1;
            continue;
        }
        let start = i;
        if c == '"' || c == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != c {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
        } else if c == '\\' {
            i = (i + 2).min(chars.len());
        } else if c == '(' && out.to_ascii_lowercase().ends_with("url") && !pending_space {
            // An unquoted URL runs to the closing parenthesis as one token.
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            i += 1;
            if !matches!(chars.get(j), Some('"' | '\'')) {
                while i < chars.len() && chars[i] != ')' {
                    i += if chars[i] == '\\' { 2 } else { 1 };
                }
                i = i.min(chars.len());
            }
        } else {
            i += 1;
        }
        push(&mut out, &chars[start..i], &mut pending_space);
    }
    out
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        out.push(' ');
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests;
