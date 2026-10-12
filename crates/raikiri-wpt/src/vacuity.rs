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
//!
//! Documents are parsed with the html5ever tokenizer and tree builder, and
//! style sheets and `style` attributes with cssparser, so markup that the
//! HTML parser drops or leaves inert and CSS that the CSS parser skips do not
//! count as declarations.

use crate::baseline_report::{Status, report_config, run_id_with_config};
use crate::reftest::{
    ReftestConfig, ReftestKind, ReftestPair, discover_pairs_for_file_with_wpt_root,
    run_pair_with_test_html,
};
use crate::runner::TestOutcome;
use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, DeclarationParser, ParseError, Parser,
    ParserInput, ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
    StyleSheetParser, Token,
};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token as HtmlToken, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use html5ever::tree_builder::{
    Attribute, ElementFlags, NodeOrText, QuirksMode, TreeBuilder, TreeBuilderOpts, TreeSink,
};
use html5ever::{Namespace, QualName, TokenizerResult, local_name, ns};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

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
    // A probe that errors for one reference leaves the verdict open; a later
    // reference can still keep the test.
    let mut inconclusive = None;
    'pairs: for pair in matches {
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
            None => {
                inconclusive.get_or_insert("blank");
                continue;
            }
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
                None => {
                    inconclusive.get_or_insert(probe);
                    continue 'pairs;
                }
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
    if let Some(probe) = inconclusive {
        return Ok(Verdict::Inconclusive { probe });
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
    for tag in scan(test_html).tags.iter().filter(|tag| !tag.inert) {
        if tag.is_html("link")
            && tag.has_rel("help")
            && let Some(href) = tag.applied_attribute("href")
        {
            let href = href.to_ascii_lowercase();
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
    let declared: BTreeSet<String> = test_declarations(test_html)
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
    for tag in scan(test_html).tags.iter().filter(|tag| !tag.inert) {
        let reference = tag.has_rel("match") || tag.has_rel("mismatch");
        if tag.is_html("meta") || (tag.is_html("link") && reference) {
            head.push_str(&test_html[tag.range.clone()]);
            head.push('\n');
        }
    }
    format!("<!DOCTYPE html>\n<html><head>\n{head}</head><body></body></html>\n")
}

/// One author declaration found in an HTML document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// Property (lowercased unless custom) and whitespace-normalized value,
    /// `name:value`.
    pub key: String,
    /// The `style` attribute or style sheet text that holds the declaration.
    site: Site,
    /// Byte range in the site's CSS text, including a trailing `;` when
    /// present.
    range: Range<usize>,
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

/// Where the CSS text of a declaration comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Site {
    /// The `style` attribute of the start tag with this index in
    /// [`Scan::tags`]. The CSS text is the decoded attribute value.
    Attribute(usize),
    /// The contents of a style element at this byte range of the document.
    /// `raw` means the CSS text is the source text itself; otherwise it is
    /// the element's decoded child text.
    Text { content: Range<usize>, raw: bool },
}

/// Collect declarations from style element contents and `style` attributes.
/// Comments, `<script>` contents and inert `<template>` contents are
/// skipped.
pub fn author_declarations(html: &str) -> Vec<Declaration> {
    declarations(html, &scan(html), false)
}

/// Declarations of a test document, including those inside inert templates.
/// Ablating them is harmless when they are inert, and a probe that shows they
/// have no effect flags a PASS that only holds because the template is inert.
fn test_declarations(html: &str) -> Vec<Declaration> {
    declarations(html, &scan(html), true)
}

fn declarations(html: &str, scan: &Scan, include_inert: bool) -> Vec<Declaration> {
    let mut out = Vec::new();
    for (index, tag) in scan.tags.iter().enumerate() {
        if tag.inert && !include_inert {
            continue;
        }
        if let Some(style) = tag.applied_attribute("style") {
            out.extend(
                css_declarations(style, true)
                    .into_iter()
                    .map(|decl| Declaration {
                        key: decl.key,
                        site: Site::Attribute(index),
                        range: decl.range,
                    }),
            );
        }
        if let Some((content, text, raw)) = tag.style_sheet(html) {
            out.extend(
                css_declarations(text, false)
                    .into_iter()
                    .map(|decl| Declaration {
                        key: decl.key,
                        site: Site::Text {
                            content: content.clone(),
                            raw,
                        },
                        range: decl.range,
                    }),
            );
        }
    }
    out
}

/// Remove every declaration whose key satisfies `remove`. Returns the new
/// document and the number of removed declarations.
///
/// A style sheet whose text is its source is edited in place. A `style`
/// attribute is re-serialized from the decoded attribute values of its start
/// tag, and the decoded text of an SVG style element whose source holds
/// character references, CDATA sections or comments replaces its contents,
/// so the edit never splits a character reference.
pub fn remove_declarations(html: &str, remove: impl Fn(&str) -> bool) -> (String, usize) {
    let scan = scan(html);
    let mut removed = 0;
    // Removed ranges in the CSS text of each site, in document order.
    let mut cuts: Vec<(Site, Vec<Range<usize>>)> = Vec::new();
    for decl in declarations(html, &scan, true) {
        if !remove(&decl.key) {
            continue;
        }
        removed += 1;
        match cuts.last_mut() {
            Some((site, ranges)) if *site == decl.site => ranges.push(decl.range),
            _ => cuts.push((decl.site, vec![decl.range])),
        }
    }
    let mut edits: Vec<(Range<usize>, String)> = cuts
        .into_iter()
        .map(|(site, ranges)| match site {
            Site::Attribute(index) => {
                let tag = &scan.tags[index];
                let style = tag.applied_attribute("style").unwrap_or_default();
                (tag.range.clone(), tag.serialize(&cut(style, &ranges)))
            }
            Site::Text { content, raw } => {
                let tag = scan
                    .tags
                    .iter()
                    .find(|tag| tag.content.as_ref() == Some(&content));
                let text = tag
                    .and_then(|tag| tag.style_sheet(html))
                    .map_or("", |(_, text, _)| text);
                let text = cut(text, &ranges);
                let text = if raw { text } else { escape(&text, false) };
                (content, text)
            }
        })
        .collect();
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(html.len());
    let mut last = 0;
    for (range, replacement) in edits {
        out.push_str(&html[last..range.start]);
        out.push_str(&replacement);
        last = range.end;
    }
    out.push_str(&html[last..]);
    (out, removed)
}

/// `text` without the sorted, disjoint `ranges`.
fn cut(text: &str, ranges: &[Range<usize>]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for range in ranges {
        out.push_str(&text[last..range.start]);
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

/// Escape text for an HTML attribute value (`attribute`) or for element
/// contents, as the HTML fragment serialization algorithm does.
fn escape(text: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attribute => out.push_str("&quot;"),
            '<' if !attribute => out.push_str("&lt;"),
            '>' if !attribute => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// The start tags of a document that the HTML tree builder turned into
/// elements, from [`scan`].
struct Scan {
    tags: Vec<Tag>,
}

/// One start tag whose element the tree builder created, or whose attributes
/// it added to an existing `html` or `body` element.
struct Tag {
    /// Namespace of the element.
    namespace: Namespace,
    /// Tag name as tokenized (ASCII-lowercased).
    name: String,
    /// Byte range of the whole start tag.
    range: Range<usize>,
    /// Decoded attributes in source order, without duplicates. The flag says
    /// whether the attribute applies: an attribute of a repeated `html` or
    /// `body` start tag applies only when the element did not have it yet.
    attributes: Vec<(String, String, bool)>,
    /// Whether the tag ends with `/>`.
    self_closing: bool,
    /// Whether the element is not connected to the document, which happens
    /// inside `<template>` contents.
    inert: bool,
    /// Byte range of the element's contents, from the end of its start tag
    /// to the start of the token that closed it. Only kept for style
    /// elements.
    content: Option<Range<usize>>,
    /// Text appended to the element by the tree builder. Only kept for style
    /// elements.
    text: String,
}

impl Tag {
    fn is_html(&self, name: &str) -> bool {
        self.namespace == ns!(html) && self.name == name
    }

    /// The value of the attribute `name` when it applies.
    fn applied_attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(attribute, _, applies)| *applies && attribute == name)
            .map(|(_, value, _)| value.as_str())
    }

    /// Whether the `rel` attribute has the token `token`.
    fn has_rel(&self, token: &str) -> bool {
        self.applied_attribute("rel").is_some_and(|rel| {
            rel.split_ascii_whitespace()
                .any(|t| t.eq_ignore_ascii_case(token))
        })
    }

    /// The contents of a style element that applies style sheets (HTML or
    /// SVG): the content range, the CSS text and whether that text is the
    /// source itself. HTML style contents are raw text; SVG style contents
    /// are parsed as markup, so their CSS text is the decoded child text.
    fn style_sheet<'a>(&'a self, html: &'a str) -> Option<(Range<usize>, &'a str, bool)> {
        let content = self.content.clone()?;
        let source = &html[content.clone()];
        if self.namespace == ns!(html) {
            Some((content, source, true))
        } else {
            let raw = source == self.text;
            Some((content, &self.text, raw))
        }
    }

    /// The start tag with the `style` attribute value replaced by `style`.
    fn serialize(&self, style: &str) -> String {
        let mut out = format!("<{}", self.name);
        for (name, value, applies) in &self.attributes {
            let value = if *applies && name == "style" {
                style
            } else {
                value
            };
            out.push_str(&format!(" {name}=\"{}\"", escape(value, true)));
        }
        if self.self_closing {
            out.push_str(" /");
        }
        out.push('>');
        out
    }
}

/// Tokenize and tree-build `html` with html5ever, recording the source range
/// of every start tag and the contents of style elements.
fn scan(html: &str) -> Scan {
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from_slice(html));
    let builder = TreeBuilder::new(
        Dom::default(),
        TreeBuilderOpts {
            // Scripts run, so `<noscript>` contents are raw text.
            scripting_enabled: true,
            ..TreeBuilderOpts::default()
        },
    );
    let sink = Recorder {
        html,
        queue: &queue,
        builder,
        previous_end: Cell::new(0),
    };
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    // The tokenizer pauses after a script end tag and after a `<meta>`
    // encoding declaration; the input is already decoded, so keep going.
    while !matches!(tokenizer.feed(&queue), TokenizerResult::Done) {}
    tokenizer.end();
    tokenizer.sink.builder.sink.finish_scan(html.len())
}

/// A token sink that records the source range of each token before passing
/// it on to the tree builder.
struct Recorder<'a> {
    html: &'a str,
    /// The input queue, whose remaining length gives the source offset of
    /// the tokenizer.
    queue: &'a BufferQueue,
    builder: TreeBuilder<Handle, Dom>,
    /// End of the previous token.
    previous_end: Cell<usize>,
}

impl Recorder<'_> {
    /// Bytes the tokenizer has consumed so far.
    fn consumed(&self) -> usize {
        let mut chunks = Vec::new();
        while let Some(chunk) = self.queue.pop_front() {
            chunks.push(chunk);
        }
        let remaining: usize = chunks.iter().map(|chunk| chunk.len()).sum();
        for chunk in chunks.into_iter().rev() {
            self.queue.push_front(chunk);
        }
        self.html.len() - remaining
    }

    /// Start of the tag token that ends at `end`. The previous token can end
    /// one character late when the tokenizer reconsumes it, so the search
    /// starts one character earlier, at the first `<` followed by the tag
    /// name.
    fn tag_start(&self, tag: &html5ever::tokenizer::Tag, end: usize) -> usize {
        let html = self.html;
        let mut lower = self.previous_end.get().min(end).saturating_sub(1);
        while !html.is_char_boundary(lower) {
            lower -= 1;
        }
        let open = if tag.kind == TagKind::EndTag {
            "</"
        } else {
            "<"
        };
        let name: &str = &tag.name;
        let window = &html[lower..end];
        window
            .match_indices(open)
            .map(|(at, _)| at)
            .find(|&at| {
                window
                    .get(at + open.len()..at + open.len() + name.len())
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .or_else(|| window.find('<'))
            .map_or(lower, |at| lower + at)
    }
}

impl TokenSink for Recorder<'_> {
    type Handle = Handle;

    fn process_token(&self, token: HtmlToken, line_number: u64) -> TokenSinkResult<Handle> {
        let dom = &self.builder.sink;
        match &token {
            HtmlToken::ParseError(_) => {}
            HtmlToken::EOFToken => {
                dom.token.set((self.html.len(), self.html.len()));
            }
            HtmlToken::TagToken(tag) => {
                let end = self.consumed();
                let start = self.tag_start(tag, end);
                dom.token.set((start, end));
                self.previous_end.set(end);
                if tag.kind == TagKind::StartTag {
                    *dom.start_tag.borrow_mut() = Some(PendingTag {
                        name: tag.name.to_string(),
                        attributes: tag
                            .attrs
                            .iter()
                            .map(|a| (a.name.local.to_string(), a.value.to_string()))
                            .collect(),
                        self_closing: tag.self_closing,
                        element: None,
                        namespace: ns!(html),
                        added: Vec::new(),
                    });
                }
            }
            _ => {
                let end = self.consumed();
                dom.token.set((self.previous_end.get().min(end), end));
                self.previous_end.set(end);
            }
        }
        let end_tag = matches!(&token, HtmlToken::TagToken(tag) if tag.kind == TagKind::EndTag);
        let eof = matches!(token, HtmlToken::EOFToken);
        let result = self.builder.process_token(token, line_number);
        let pending = dom.start_tag.borrow_mut().take();
        // The tree builder does not report every element it pops, so track
        // the open style element from the tokens: an end tag that does not
        // close an element inside it, a start tag outside it or the end of
        // the input closes it.
        if let Some((style, depth)) = dom.open_style.get() {
            let inside = pending
                .as_ref()
                .and_then(|p| p.element)
                .filter(|&element| element != style && dom.is_inside(element, style));
            let next = if eof {
                None
            } else if end_tag {
                depth.checked_sub(1).map(|depth| (style, depth))
            } else if let Some(pending) = &pending {
                match inside {
                    Some(_) if pending.self_closing => Some((style, depth)),
                    Some(_) => Some((style, depth + 1)),
                    None => None,
                }
            } else {
                Some((style, depth))
            };
            if next.is_none() {
                dom.close(style);
            }
            dom.open_style.set(next);
        }
        if let Some(pending) = pending {
            dom.record(pending);
        }
        result
    }

    fn end(&self) {
        self.builder.end();
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.builder
            .adjusted_current_node_present_but_not_in_html_namespace()
    }
}

/// A start tag token while the tree builder processes it.
struct PendingTag {
    name: String,
    attributes: Vec<(String, String)>,
    self_closing: bool,
    /// The element created for the token, and its namespace.
    element: Option<usize>,
    namespace: Namespace,
    /// Attributes added to an existing element.
    added: Vec<String>,
}

/// A node handle: the node's index and, for elements, its name.
#[derive(Clone, Debug)]
struct Handle(Rc<(usize, QualName)>);

impl Handle {
    fn id(&self) -> usize {
        self.0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeKind {
    Document,
    Element,
    TemplateContents,
    Other,
}

struct Node {
    kind: NodeKind,
    parent: Option<usize>,
    /// Attribute names of an element, to tell which merged attributes are
    /// new.
    attributes: Vec<String>,
    /// For a template element, its contents node.
    template_contents: Option<Handle>,
    integration_point: bool,
    /// Index in [`Dom::tags`] of the start tag that created the element.
    tag: Option<usize>,
    /// For a style element, the byte range of its contents. The end is
    /// `usize::MAX` until a token closes the element.
    content: Option<Range<usize>>,
    /// For a style element, the text appended to it.
    text: String,
}

/// A minimal tree sink that keeps what the scan needs: element parents,
/// template contents, and the text and extent of style elements.
#[derive(Default)]
struct Dom {
    nodes: RefCell<Vec<Node>>,
    tags: RefCell<Vec<Tag>>,
    /// Byte range of the token the tree builder is processing.
    token: Cell<(usize, usize)>,
    /// The start tag token the tree builder is processing.
    start_tag: RefCell<Option<PendingTag>>,
    /// The open style element and the number of open elements inside it.
    open_style: Cell<Option<(usize, usize)>>,
}

impl Dom {
    fn add(&self, kind: NodeKind, name: QualName) -> Handle {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(Node {
            kind,
            parent: None,
            attributes: Vec::new(),
            template_contents: None,
            integration_point: false,
            tag: None,
            content: None,
            text: String::new(),
        });
        Handle(Rc::new((nodes.len() - 1, name)))
    }

    fn other(&self) -> Handle {
        self.add(NodeKind::Other, QualName::new(None, ns!(), local_name!("")))
    }

    /// Keep the processed start tag when it created an element or added
    /// attributes to one.
    fn record(&self, pending: PendingTag) {
        let element = pending.element;
        if element.is_none() && pending.added.is_empty() {
            return;
        }
        let (start, end) = self.token.get();
        let mut tags = self.tags.borrow_mut();
        if let Some(id) = element {
            let mut nodes = self.nodes.borrow_mut();
            nodes[id].tag = Some(tags.len());
            if let Some(content) = &mut nodes[id].content {
                if pending.self_closing && pending.namespace != ns!(html) {
                    content.end = content.start;
                } else {
                    self.open_style.set(Some((id, 0)));
                }
            }
        }
        let attributes = pending
            .attributes
            .into_iter()
            .map(|(name, value)| {
                let applies = element.is_some() || pending.added.contains(&name);
                (name, value, applies)
            })
            .collect();
        tags.push(Tag {
            namespace: pending.namespace,
            name: pending.name,
            range: start..end,
            attributes,
            self_closing: pending.self_closing,
            inert: false,
            content: None,
            text: String::new(),
        });
    }

    /// Whether `node` is a descendant of `ancestor`.
    fn is_inside(&self, node: usize, ancestor: usize) -> bool {
        let nodes = self.nodes.borrow();
        let mut at = nodes[node].parent;
        while let Some(current) = at {
            if current == ancestor {
                return true;
            }
            at = nodes[current].parent;
        }
        false
    }

    /// End the contents of `style` at the start of the current token.
    fn close(&self, style: usize) {
        let start = self.token.get().0;
        if let Some(content) = &mut self.nodes.borrow_mut()[style].content {
            content.end = start.max(content.start);
        }
    }

    fn set_parent(&self, child: &NodeOrText<Handle>, parent: Option<usize>) {
        if let NodeOrText::AppendNode(node) = child {
            self.nodes.borrow_mut()[node.id()].parent = parent;
        }
    }

    fn finish_scan(&self, len: usize) -> Scan {
        let mut nodes = self.nodes.borrow_mut();
        let mut tags = self.tags.take();
        for id in 0..nodes.len() {
            let Some(index) = nodes[id].tag else { continue };
            // An element is connected when its ancestors reach the document.
            let mut at = Some(id);
            let mut connected = false;
            while let Some(current) = at {
                if nodes[current].kind == NodeKind::Document {
                    connected = true;
                    break;
                }
                at = nodes[current].parent;
            }
            let tag = &mut tags[index];
            tag.inert = !connected;
            tag.content = nodes[id].content.clone().map(|content| {
                // Never closed: the contents run to the end of the input.
                content.start..content.end.min(len)
            });
            tag.text = std::mem::take(&mut nodes[id].text);
        }
        Scan { tags }
    }
}

impl TreeSink for Dom {
    type Handle = Handle;
    type Output = ();
    type ElemName<'a> = &'a QualName;

    fn finish(self) {}

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> Handle {
        if self.nodes.borrow().is_empty() {
            return self.add(
                NodeKind::Document,
                QualName::new(None, ns!(), local_name!("")),
            );
        }
        Handle(Rc::new((0, QualName::new(None, ns!(), local_name!("")))))
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> &'a QualName {
        &target.0.1
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        let style = &*name.local == "style" && (name.ns == ns!(html) || name.ns == ns!(svg));
        let handle = self.add(NodeKind::Element, name.clone());
        {
            let mut nodes = self.nodes.borrow_mut();
            let node = &mut nodes[handle.id()];
            node.attributes = attrs.iter().map(|a| a.name.local.to_string()).collect();
            node.integration_point = flags.mathml_annotation_xml_integration_point;
            if style {
                node.content = Some(self.token.get().1..usize::MAX);
            }
        }
        if let Some(pending) = self.start_tag.borrow_mut().as_mut()
            && (*name.local).eq_ignore_ascii_case(pending.name.as_str())
        {
            // The token's own element is created last, after any implied or
            // reconstructed elements.
            pending.element = Some(handle.id());
            pending.namespace = name.ns.clone();
        }
        handle
    }

    fn create_comment(&self, _text: StrTendril) -> Handle {
        self.other()
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        self.other()
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        if let NodeOrText::AppendText(text) = &child {
            let mut nodes = self.nodes.borrow_mut();
            let node = &mut nodes[parent.id()];
            if node.content.is_some() {
                node.text.push_str(text);
            }
        }
        self.set_parent(&child, Some(parent.id()));
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        if self.nodes.borrow()[element.id()].parent.is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {}

    fn get_template_contents(&self, target: &Handle) -> Handle {
        if let Some(contents) = &self.nodes.borrow()[target.id()].template_contents {
            return contents.clone();
        }
        let contents = self.add(
            NodeKind::TemplateContents,
            QualName::new(None, ns!(), local_name!("")),
        );
        self.nodes.borrow_mut()[target.id()].template_contents = Some(contents.clone());
        contents
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        x.id() == y.id()
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &Handle, new_node: NodeOrText<Handle>) {
        let parent = self.nodes.borrow()[sibling.id()].parent;
        self.set_parent(&new_node, parent);
    }

    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let mut nodes = self.nodes.borrow_mut();
        let present = &mut nodes[target.id()].attributes;
        let mut added = Vec::new();
        for attr in attrs {
            let name = attr.name.local.to_string();
            if !present.contains(&name) {
                present.push(name.clone());
                added.push(name);
            }
        }
        if let Some(pending) = self.start_tag.borrow_mut().as_mut() {
            pending.added.extend(added);
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        self.nodes.borrow_mut()[target.id()].parent = None;
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        for child in self.nodes.borrow_mut().iter_mut() {
            if child.parent == Some(node.id()) {
                child.parent = Some(new_parent.id());
            }
        }
    }

    fn is_mathml_annotation_xml_integration_point(&self, handle: &Handle) -> bool {
        self.nodes.borrow()[handle.id()].integration_point
    }

    fn allow_declarative_shadow_roots(&self, _intended_parent: &Handle) -> bool {
        // The DOM does not attach declarative shadow roots, so their
        // templates stay inert.
        false
    }
}

/// One declaration found by [`css_declarations`].
#[derive(Debug)]
struct CssDeclaration {
    /// Byte range in the CSS text, from the property name through a trailing
    /// `;` when present.
    range: Range<usize>,
    key: String,
}

/// Declarations in CSS text, found with the CSS parser. `inline` parses the
/// text as a declaration list (a `style` attribute); otherwise it is a style
/// sheet, whose declarations sit in the blocks of qualified rules, nested
/// rules and at-rules.
fn css_declarations(css: &str, inline: bool) -> Vec<CssDeclaration> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut collector = Collector {
        css,
        inline,
        out: Vec::new(),
    };
    if inline {
        for _ in RuleBodyParser::new(&mut parser, &mut collector) {}
    } else {
        for _ in StyleSheetParser::new(&mut parser, &mut collector) {}
    }
    collector.out
}

/// Collects the declarations of every block in a style sheet or declaration
/// list.
struct Collector<'a> {
    css: &'a str,
    inline: bool,
    out: Vec<CssDeclaration>,
}

impl Collector<'_> {
    fn body<'i>(&mut self, input: &mut Parser<'i, '_>) -> Result<(), ParseError<'i, ()>> {
        for _ in RuleBodyParser::new(input, self) {}
        Ok(())
    }
}

fn consume<'i>(input: &mut Parser<'i, '_>) -> Result<(), ParseError<'i, ()>> {
    while input.next().is_ok() {}
    Ok(())
}

impl<'i> DeclarationParser<'i> for Collector<'_> {
    type Declaration = ();
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        declaration_start: &ParserState,
    ) -> Result<(), ParseError<'i, ()>> {
        let custom = name.starts_with("--");
        let mut value = String::new();
        // A `{}` block in a value is part of the declaration only for custom
        // properties; otherwise the text is reparsed as a nested rule.
        normalize_value(input, &mut value, &mut false, !custom)?;
        let end = input.position().byte_index();
        let end = end + usize::from(self.css[end..].starts_with(';'));
        if value.is_empty() {
            return Ok(());
        }
        self.out.push(CssDeclaration {
            range: declaration_start.position().byte_index()..end,
            // Property names are ASCII case-insensitive, custom property
            // names are not. Values keep their case: strings, URLs and custom
            // property values are case-sensitive, and a keyword that differs
            // only in case merely makes the declaration count as test-only.
            key: if custom {
                format!("{name}:{value}")
            } else {
                format!("{}:{value}", name.to_ascii_lowercase())
            },
        });
        Ok(())
    }
}

impl<'i> QualifiedRuleParser<'i> for Collector<'_> {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = ();

    fn parse_prelude<'t>(&mut self, input: &mut Parser<'i, 't>) -> Result<(), ParseError<'i, ()>> {
        consume(input)
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: (),
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<(), ParseError<'i, ()>> {
        self.body(input)
    }
}

impl<'i> AtRuleParser<'i> for Collector<'_> {
    type Prelude = ();
    type AtRule = ();
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<(), ParseError<'i, ()>> {
        if self.inline {
            return Err(input.new_error(BasicParseErrorKind::AtRuleInvalid(name)));
        }
        consume(input)
    }

    fn rule_without_block(&mut self, _prelude: (), _start: &ParserState) -> Result<(), ()> {
        Ok(())
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: (),
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<(), ParseError<'i, ()>> {
        // Treat every at-rule block as a mix of declarations and rules, so
        // `@page` and `@font-face` declarations count as well as the rules in
        // `@media` and `@supports`.
        self.body(input)
    }
}

impl<'i> RuleBodyItemParser<'i, (), ()> for Collector<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        !self.inline
    }
}

/// Append the tokens of a declaration value to `out` as source text, with
/// comments and runs of whitespace collapsed to one space and the ends
/// trimmed. Strings, URLs and other tokens keep their source spelling.
/// `reject_blocks` fails on a top-level `{}` block.
fn normalize_value<'i>(
    input: &mut Parser<'i, '_>,
    out: &mut String,
    pending_space: &mut bool,
    reject_blocks: bool,
) -> Result<(), ParseError<'i, ()>> {
    let push = |out: &mut String, text: &str, pending_space: &mut bool| {
        // A hex escape can end with the whitespace that terminates it, which
        // already separates the tokens.
        if *pending_space && !out.is_empty() && !ends_with_hex_escape(out) {
            out.push(' ');
        }
        *pending_space = false;
        out.push_str(text);
    };
    loop {
        let start = input.position();
        let Ok(token) = input.next_including_whitespace_and_comments() else {
            return Ok(());
        };
        let closer = match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {
                *pending_space = true;
                continue;
            }
            Token::CurlyBracketBlock if reject_blocks => {
                return Err(input.new_custom_error(()));
            }
            Token::Function(_) | Token::ParenthesisBlock => ")",
            Token::SquareBracketBlock => "]",
            Token::CurlyBracketBlock => "}",
            _ => {
                push(out, input.slice_from(start), pending_space);
                continue;
            }
        };
        push(out, input.slice_from(start), pending_space);
        input.parse_nested_block(|nested| normalize_value(nested, out, pending_space, false))?;
        // An unclosed block ends with the input.
        if input.slice_from(start).ends_with(closer) {
            push(out, closer, pending_space);
        }
    }
}

/// Whether `text` ends with a hex escape and the whitespace that terminates
/// it.
fn ends_with_hex_escape(text: &str) -> bool {
    let Some(rest) = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix(|c: char| c.is_ascii_whitespace()))
    else {
        return false;
    };
    let digits = rest.bytes().rev().take_while(u8::is_ascii_hexdigit).count();
    let rest = &rest[..rest.len() - digits];
    let backslashes = rest.bytes().rev().take_while(|&b| b == b'\\').count();
    (1..=6).contains(&digits) && backslashes % 2 == 1
}

#[cfg(test)]
mod tests;
