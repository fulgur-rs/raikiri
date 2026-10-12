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
//! declarations.

use crate::baseline_report::{Status, run_id};
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
            Verdict::NotPass { status, detail } => format!("{}: {detail}", status.as_str()),
        }
    }
}

/// Run `id` and, when it passes, probe each passing `rel=match` reference.
///
/// The id uses the baseline format: a root-relative WPT path with an optional
/// declared variant query.
pub fn audit_id(wpt_root: &Path, id: &str) -> Verdict {
    let row = run_id(wpt_root, id);
    if row.status != Status::Pass {
        return Verdict::NotPass {
            status: row.status,
            detail: row.detail,
        };
    }
    let (path, query) = id.split_once('?').map_or((id, ""), |(p, q)| (p, q));
    let variant_query = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    // Dynamic reftests need absolute paths, as in the baseline runner.
    let canonical = std::fs::canonicalize(wpt_root).and_then(|root| {
        let test = std::fs::canonicalize(root.join(path))?;
        Ok((root, test))
    });
    let (wpt_root, test_path) = match canonical {
        Ok(paths) => paths,
        Err(error) => {
            return Verdict::NotPass {
                status: Status::Error,
                detail: format!("resolve: {error}"),
            };
        }
    };
    let wpt_root = wpt_root.as_path();
    let pairs = match discover_pairs_for_file_with_wpt_root(&test_path, Some(wpt_root)) {
        Ok(pairs) => pairs,
        Err(error) => {
            return Verdict::NotPass {
                status: Status::Error,
                detail: format!("discover: {error}"),
            };
        }
    };
    let matches: Vec<&ReftestPair> = pairs
        .iter()
        .filter(|pair| pair.kind == ReftestKind::Match)
        .collect();
    if matches.is_empty() {
        return Verdict::MismatchOnly;
    }
    let config = ReftestConfig {
        require_inline_fonts: true,
        ..ReftestConfig::default()
    };
    let read = |path: &Path| std::fs::read_to_string(path).map_err(|e| e.to_string());
    let test_html = match read(&test_path) {
        Ok(html) => html,
        Err(error) => {
            return Verdict::NotPass {
                status: Status::Error,
                detail: format!("read test: {error}"),
            };
        }
    };
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
        let reference_html = match read(&pair.reference) {
            Ok(html) => html,
            Err(_) => continue,
        };
        if passes(pair, config, resolve_images, &variant_query, &blank) == Some(true) {
            verdict.get_or_insert(Verdict::Blank);
            continue;
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
        let unstyled = probes.into_iter().find(|(_, (html, removed))| {
            *removed > 0 && passes(pair, config, resolve_images, &variant_query, html) == Some(true)
        });
        if let Some((probe, (_, ablated))) = unstyled {
            if !matches!(verdict, Some(Verdict::Unstyled { .. })) {
                verdict = Some(Verdict::Unstyled {
                    probe,
                    ablated: *ablated,
                });
            }
            continue;
        }
        let ablated = by_declaration.1;
        return Verdict::Kept { ablated };
    }
    verdict.unwrap_or(Verdict::NotPass {
        status: Status::Fail,
        detail: "no rel=match reference passed when probed".to_owned(),
    })
}

/// Properties declared in `test_html` that the test names as its subject:
/// the property spelled as whole hyphen-separated words in the file name or
/// in a `rel=help` link's last path segment or fragment. A name contained in
/// a longer matched name (`color` inside `background-color`) is dropped.
pub fn feature_properties(test_html: &str, test_id: &str) -> BTreeSet<String> {
    let mut subjects: Vec<String> = Vec::new();
    let file = test_id.rsplit('/').next().unwrap_or(test_id);
    subjects.push(file.split('.').next().unwrap_or(file).to_ascii_lowercase());
    let lower = test_html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(offset) = lower[from..].find("<link") {
        let start = from + offset;
        let Some(end) = tag_end(test_html, start) else {
            break;
        };
        let attrs = tag_attributes(test_html, start, end);
        let href = attrs
            .iter()
            .find(|(name, _)| name == "href")
            .map(|(_, value)| lower[value.clone()].to_owned());
        if has_rel(test_html, &attrs, "help")
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
        from = end;
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
        let parts: Vec<&str> = property.split('-').collect();
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
    let lower = test_html.to_ascii_lowercase();
    let mut head = String::new();
    for tag in ["<meta", "<link"] {
        let mut from = 0;
        while let Some(offset) = lower[from..].find(tag) {
            let start = from + offset;
            let Some(end) = tag_end(test_html, start) else {
                break;
            };
            let attrs = tag_attributes(test_html, start, end);
            let reference =
                has_rel(test_html, &attrs, "match") || has_rel(test_html, &attrs, "mismatch");
            if tag == "<meta" || reference {
                head.push_str(&test_html[start..end]);
                head.push('\n');
            }
            from = end;
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
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if lower[i..].starts_with("<!--") {
            i = lower[i + 4..]
                .find("-->")
                .map_or(bytes.len(), |e| i + 4 + e + 3);
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
        let name = &lower[name_start..name_start + name_len];
        let Some(end) = tag_end(html, i) else {
            break;
        };
        for (attr, value) in attributes(html, name_start + name_len, end) {
            if attr == "style" {
                out.extend(css_declarations(&html[value.clone()], true, value.start));
            }
        }
        i = end;
        if name == "style" || name == "script" {
            let close = format!("</{name}");
            let content_end = lower[i..].find(&close).map_or(bytes.len(), |e| i + e);
            if name == "style" {
                out.extend(css_declarations(&html[i..content_end], false, i));
            }
            i = content_end;
        }
    }
    out
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
        out.push((name, value));
    }
    out
}

/// Attributes of the start tag beginning at `start` (`<name ...>`).
fn tag_attributes(html: &str, start: usize, end: usize) -> Vec<(String, Range<usize>)> {
    let name_len = html.as_bytes()[start + 1..end]
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'-')
        .count();
    attributes(html, start + 1 + name_len, end)
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

fn push_declaration(css: &str, range: Range<usize>, base: usize, out: &mut Vec<Declaration>) {
    let Some(text) = css.get(range.clone()) else {
        return;
    };
    let text = text.trim_end_matches(';');
    let Some((name, value)) = text.split_once(':') else {
        return;
    };
    let name = strip_comments(name);
    let name = name.trim();
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return;
    }
    let value = strip_comments(value);
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
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
