//! RaikiriTreeSink — html5ever `TreeSink` implementation backed by
//! raikiri-dom::Document. Handle = arena index (usize).

use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};

use html5ever::interface::{Attribute, ElementFlags, NodeOrText, QualName, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::tree_builder::QuirksMode;
use markup5ever::ns;
use raikiri_dom::Document;
use raikiri_traits::{Dom, RenderWarning, WarningKind};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use taffy::Style;

use crate::types::UncascadedDocument;

/// The raikiri implementation of html5ever `TreeSink`. Handles are indices
/// (`usize`) into the raikiri-dom arena; output is [`UncascadedDocument`].
/// Store QualName / Attribute metadata in an FxHashMap inside a RefCell,
/// keeping html5ever-specific types out of raikiri-dom::Node.
pub struct RaikiriTreeSink {
    document: RefCell<Document>,
    /// Handle → full QualName (namespace + local name). This backs the
    /// `Ref<'_, QualName>` returned by `elem_name()`. On `finish()`, copy only
    /// non-HTML namespaces to raikiri-dom::Node.namespace.
    qual_names: RefCell<FxHashMap<usize, QualName>>,
    /// Handle → attributes. During parsing, `add_attrs_if_missing` merges them.
    /// On `finish()`, copy null-namespace attributes to raikiri-dom::Node.attributes
    /// and separate the `style` attribute into raikiri-dom::Node.inline_style.
    /// Support for namespaced attributes such as xlink:href is deferred.
    attributes: RefCell<FxHashMap<usize, Vec<Attribute>>>,
    /// Buffer of nonfatal parse errors reported by html5ever. Move these to
    /// UncascadedDocument.warnings on `finish()`.
    warnings: RefCell<Vec<RenderWarning>>,
    /// Document-wide quirks mode, intended for use by the cascade.
    quirks_mode: Cell<QuirksMode>,
    /// Maximum number of warnings recorded by `parse_error` (`None` = unlimited).
    /// Intended to receive [`raikiri_traits::RenderLimits::max_parse_warnings`];
    /// see that field's documentation for the rationale.
    max_parse_warnings: Option<usize>,
}

impl RaikiriTreeSink {
    /// Construct a new sink with an empty Document whose virtual root is at
    /// arena index 0.
    ///
    /// `max_parse_warnings` caps the warnings recorded by [`TreeSink::parse_error`]
    /// (`None` = unlimited). Callers normally pass the value of
    /// `raikiri_traits::RenderLimits::max_parse_warnings` directly.
    /// `raikiri_html::parse` uses the default; `raikiri::parse_html_with_limits`
    /// passes the value configured by the consumer.
    pub fn new(max_parse_warnings: Option<usize>) -> Self {
        Self {
            document: RefCell::new(Document::new()),
            qual_names: RefCell::new(FxHashMap::default()),
            attributes: RefCell::new(FxHashMap::default()),
            warnings: RefCell::new(Vec::new()),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
            max_parse_warnings,
        }
    }

    /// Construct a detached element node and register its QualName and attributes
    /// in the metadata tables.
    ///
    /// Populate `Node.inline_style`, `Node.namespace`, and `Node.attributes`
    /// from the metadata tables together in [`RaikiriTreeSink::finish`].
    /// Here, only register the tag and default Style in the Document and store
    /// full-fidelity metadata in the side tables.
    fn make_element(&self, name: QualName, attrs: Vec<Attribute>) -> usize {
        let tag: SmolStr = AsRef::<str>::as_ref(&name.local).into();
        let idx =
            self.document
                .borrow_mut()
                .append_element(None, tag, Style::default(), None::<&str>);
        self.qual_names.borrow_mut().insert(idx, name);
        self.attributes.borrow_mut().insert(idx, attrs);
        idx
    }

    /// Append Text to the parent's last child.
    ///
    /// The HTML tokenizer may split a single inline run across callbacks.
    /// Separate adjacent Text nodes would become separate block leaves and cause
    /// spurious line breaks in normal flow. Merge with the last Text node as required by TreeSink.
    fn append_text_smart(&self, parent: usize, text: StrTendril) {
        self.document
            .borrow_mut()
            .append_text_coalesced(parent, text.to_string());
    }
}

impl Default for RaikiriTreeSink {
    fn default() -> Self {
        Self::new(raikiri_traits::RenderLimits::default().max_parse_warnings)
    }
}

impl TreeSink for RaikiriTreeSink {
    type Handle = usize;
    type Output = UncascadedDocument;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> UncascadedDocument {
        let mut document = self.document.into_inner();
        let warnings = self.warnings.into_inner();
        let qual_names = self.qual_names.into_inner();
        let attributes = self.attributes.into_inner();

        // Copy metadata tables into raikiri-dom::Node.
        // qual_names → Node.namespace (non-HTML only).
        // attributes → Node.attributes (null namespace, excluding style) + Node.inline_style.
        wire_side_tables(&mut document, &qual_names, &attributes);

        // Store the value reported by html5ever's set_quirks_mode callback on the
        // Document itself. The cascade (raikiri-style id/class selector matching)
        // reads it through `impl StyleDom for Document`.
        let quirks_mode = convert_quirks(self.quirks_mode.get());
        document.set_quirks_mode(quirks_mode);

        // Remove the old strip_non_element_stubs behavior (which physically removed
        // pseudo-tag "#comment" and "#pi" Elements from the tree).
        // Persist Comment and ProcessingInstruction as the NodeData::Comment and
        // NodeData::ProcessingInstruction variants in the tree, matching WHATWG
        // DOM §4 NodeType. Both variants have their IS_IN_DOCUMENT bit cleared
        // by `mark_in_document_flags` step 2. Therefore:
        //
        // - The TaffyChildIter is_in_document filter excludes them from layout child
        //   counts (raikiri-dom/src/taffy_impl.rs:38,61,71).
        // - extract_inline_stylesheets / find_head_element / find_body skip them via
        //   the is_in_document() and Element gates.
        // - All cascade and paint traversals skip them through the same gates.
        //
        // Clear IS_IN_DOCUMENT on template subtrees, detached nodes (transient foster
        // parenting), and Comment/PI nodes. Do this before
        // extract_inline_stylesheets, which skips them through the
        // is_in_document() gate.
        document.mark_in_document_flags();

        let stylesheet_sources = extract_inline_stylesheets(&document);
        UncascadedDocument {
            dom: document,
            stylesheet_sources,
            warnings,
            quirks_mode,
        }
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        let mut warnings = self.warnings.borrow_mut();

        let Some(cap) = self.max_parse_warnings else {
            // Unlimited (only reached when a consumer explicitly sets
            // `max_parse_warnings = None`).
            warnings.push(RenderWarning {
                kind: WarningKind::HtmlParseError {
                    message: msg.into_owned(),
                },
                node_id: None,
                details: String::new(),
            });
            return;
        };
        if cap == 0 {
            // There is no slot for a real warning, but—as for cap > 0—add a synthetic
            // entry on the first call indicating that at least one parse error occurred
            // and was suppressed. An unconditional no-op would make this case
            // indistinguishable from zero parse errors and would silently disable the
            // trip-and-record behavior used for cap > 0.
            if warnings.is_empty() {
                warnings.push(RenderWarning {
                    kind: WarningKind::HtmlParseError {
                        message: "parse errors suppressed (max_parse_warnings = 0)".to_string(),
                    },
                    node_id: None,
                    details: String::new(),
                });
            }
            return;
        }

        // Reserve the last slot for a synthetic entry indicating that later errors
        // were suppressed. Without it, consumers could not distinguish exactly cap
        // errors from many more. Record at most cap - 1 real parse errors.
        let last_real_slot = cap - 1;
        match warnings.len().cmp(&last_real_slot) {
            std::cmp::Ordering::Less => {
                warnings.push(RenderWarning {
                    kind: WarningKind::HtmlParseError {
                        message: msg.into_owned(),
                    },
                    node_id: None,
                    details: String::new(),
                });
            }
            std::cmp::Ordering::Equal => {
                warnings.push(RenderWarning {
                    kind: WarningKind::HtmlParseError {
                        message: format!(
                            "further parse errors suppressed after reaching the {cap}-warning cap"
                        ),
                    },
                    node_id: None,
                    details: String::new(),
                });
            }
            std::cmp::Ordering::Greater => {}
        }
    }

    fn get_document(&self) -> usize {
        self.document.borrow().root_id().0 as usize
    }

    fn elem_name<'a>(&'a self, target: &'a usize) -> Ref<'a, QualName> {
        Ref::map(self.qual_names.borrow(), |m| {
            m.get(target)
                .expect("elem_name called on non-element handle")
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> usize {
        let idx = self.make_element(name, attrs);
        // When creating a `<template>` element, html5ever sets
        // `flags.template=true` (markup5ever `create_element_with_flags`).
        // Eagerly allocate a fragment root and store it in the template_contents
        // slot. Later `TreeSink::append(get_template_contents(t), ...)` calls
        // append children to the fragment root. The template element's own children
        // remain empty, as in blitz; see
        // `Document::allocate_template_fragment_root` documentation.
        if flags.template {
            self.document
                .borrow_mut()
                .allocate_template_fragment_root(idx);
        }
        idx
    }

    fn create_comment(&self, text: StrTendril) -> usize {
        // Persist as a NodeData::Comment variant
        // (previously a "#comment" pseudo-tag Element stripped in sink.finish).
        // Allocate detached (parent=None); html5ever later attaches it with
        // append(parent, ...).
        self.document
            .borrow_mut()
            .append_comment(None, text.to_string())
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> usize {
        // Persist as a NodeData::ProcessingInstruction variant
        // (previously a "#pi" pseudo-tag Element that was stripped).
        self.document.borrow_mut().append_processing_instruction(
            None,
            target.to_string(),
            data.to_string(),
        )
    }

    fn append(&self, parent: &usize, child: NodeOrText<usize>) {
        match child {
            NodeOrText::AppendNode(c) => {
                self.document.borrow_mut().attach_child(*parent, c);
            }
            NodeOrText::AppendText(text) => {
                self.append_text_smart(*parent, text);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &usize,
        prev_element: &usize,
        child: NodeOrText<usize>,
    ) {
        // Implement proper foster parenting later. It does not arise in the
        // current hello-world case; for now, insert_before if there is a parent,
        // otherwise append as a child of prev_element.
        let has_parent = self.document.borrow().parent_of(*element).is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    // Note: raikiri-dom now represents NodeKind::Comment and
    // NodeKind::ProcessingInstruction as variants, so create_comment and
    // create_pi directly allocate the corresponding NodeData variants above.
    // The old two-stage "#comment" / "#pi" pseudo-tag and
    // strip_non_element_stubs approach is gone.

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        // Do not create a doctype node yet; a separate callback reports quirks_mode.
    }

    fn get_template_contents(&self, target: &usize) -> usize {
        // When `create_element` sees `flags.template=true`, it stores
        // the fragment root's arena index in the `template_contents` slot.
        // The index returned here becomes the parent handle for subsequent
        // html5ever `TreeSink::append` calls: template contents become children
        // of the fragment root, while the template element itself retains
        // an empty children list.
        //
        // Defensive fallback: directly built elements (for example, in unit tests)
        // may lack a wired `template_contents` slot. Return `*target` as before.
        // The existing Node::is_in_document() gate still covers direct construction
        // because `mark_in_document_flags` skips templates in that path, so this
        // does not cause a silent bug.
        let doc = self.document.borrow();
        doc.get_node(*target)
            .and_then(|n| n.template_contents())
            .unwrap_or(*target)
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn append_before_sibling(&self, sibling: &usize, new_node: NodeOrText<usize>) {
        let parent = self
            .document
            .borrow()
            .parent_of(*sibling)
            .expect("append_before_sibling: sibling has no parent");
        match new_node {
            NodeOrText::AppendNode(c) => {
                // Detach if already attached (TreeSink contract: new_node can have an
                // old parent).
                self.document.borrow_mut().detach_from_parent(c);
                self.document
                    .borrow_mut()
                    .insert_child_before(parent, *sibling, c);
            }
            NodeOrText::AppendText(text) => {
                // Create a new Text node in the arena. It cannot start detached because
                // append_text requires a parent: append it to the parent's end, detach it,
                // then insert_before.
                let text_id = self
                    .document
                    .borrow_mut()
                    .append_text(parent, text.to_string());
                self.document.borrow_mut().detach_from_parent(text_id);
                self.document
                    .borrow_mut()
                    .insert_child_before(parent, *sibling, text_id);
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &usize, attrs: Vec<Attribute>) {
        let mut store = self.attributes.borrow_mut();
        let existing = store.entry(*target).or_default();
        for a in attrs {
            let name_exists = existing.iter().any(|e| e.name == a.name);
            if !name_exists {
                existing.push(a);
            }
        }
    }

    fn remove_from_parent(&self, target: &usize) {
        self.document.borrow_mut().detach_from_parent(*target);
    }

    fn reparent_children(&self, node: &usize, new_parent: &usize) {
        self.document
            .borrow_mut()
            .reparent_children(*node, *new_parent);
    }

    fn is_mathml_annotation_xml_integration_point(&self, handle: &usize) -> bool {
        // HTML5 §13.2.5 Tree construction: MathML `annotation-xml` element is
        // an HTML integration point iff its `encoding` attribute value is an
        // ASCII case-insensitive match for `text/html` or `application/xhtml+xml`.
        // Inspect metadata tables (qual_names + attributes) directly.
        let qual_names = self.qual_names.borrow();
        let Some(name) = qual_names.get(handle) else {
            return false;
        };
        if name.ns != ns!(mathml) || AsRef::<str>::as_ref(&name.local) != "annotation-xml" {
            return false;
        }
        let attributes = self.attributes.borrow();
        let Some(attrs) = attributes.get(handle) else {
            return false;
        };
        // HTML spec §13.2.5.32: duplicate attribute → ignore later occurrences
        // (first-wins). To honor the contract checked by `wire_side_tables` and
        // `sink_first_wins_on_duplicate_style_attribute`, use find() rather than
        // any(): take the first null-namespace encoding attribute and inspect only its value.
        let Some(encoding) = attrs
            .iter()
            .find(|a| a.name.ns == ns!() && AsRef::<str>::as_ref(&a.name.local) == "encoding")
        else {
            return false;
        };
        encoding.value.eq_ignore_ascii_case("text/html")
            || encoding.value.eq_ignore_ascii_case("application/xhtml+xml")
    }
}

/// A stylesheet-bearing element in `<head>`, retained in tree order so the
/// parse layer can interleave inline and fetched external sheets without
/// changing the public `stylesheet_sources: Vec<String>` projection.
///
/// `<template>` subtrees are inert and skipped. An explicit stack avoids call
/// stack growth for attacker-controlled deep documents.
#[derive(Debug)]
pub(crate) enum HeadStylesheetSource {
    Inline {
        node_id: raikiri_traits::NodeId,
    },
    External {
        node_id: raikiri_traits::NodeId,
        href: String,
    },
}

/// Collect inline and external stylesheet elements in document order.
///
/// The sink still exposes only inline text in `UncascadedDocument` at finish;
/// this internal projection lets the post-parse network pass rebuild the same
/// vector with external responses at their actual `<link>` positions.
pub(crate) fn collect_head_stylesheet_sources(doc: &Document) -> Vec<HeadStylesheetSource> {
    use raikiri_traits::{Dom, Element, Node};

    let Some(head_id) = find_head_element(doc) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut stack: Vec<raikiri_traits::NodeId> = vec![head_id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.node(id) else {
            // Every pushed id originates in this document's own tree. Keep the
            // defensive branch required by the `Dom::node` contract.
            continue;
        };
        if !node.is_in_document() {
            continue;
        }

        if let Some(el) = node.as_element()
            && is_stylesheet_style_element(&el)
        {
            if inline_stylesheet_has_content(doc, id) {
                out.push(HeadStylesheetSource::Inline { node_id: id });
            }
            // `<style>` contents are CSS text, not nested HTML elements.
            continue;
        }

        if let Some(el) = node.as_element()
            && el.tag_name() == "link"
            && is_stylesheet_link(el.attr("rel"), el.attr("type"), el.attr("title"))
            && let Some(href) = el.attr("href")
        {
            let trimmed_href = href.trim();
            if !trimmed_href.is_empty() {
                out.push(HeadStylesheetSource::External {
                    node_id: id,
                    href: trimmed_href.to_owned(),
                });
            }
        }

        // Push in reverse so LIFO pop yields document order.
        let children: Vec<_> = doc.child_ids(id).collect();
        for child_id in children.into_iter().rev() {
            stack.push(child_id);
        }
    }
    out
}

fn inline_stylesheet_text(doc: &Document, node_id: raikiri_traits::NodeId) -> String {
    use raikiri_traits::{Dom, Node};

    let mut text = String::new();
    for child_id in doc.child_ids(node_id) {
        if let Some(child) = doc.node(child_id)
            && let Some(value) = child.text_content()
        {
            text.push_str(value);
        }
    }
    strip_xhtml_cdata_wrapper(&text)
}

/// Remove the XML CDATA wrapper used by XHTML WPT inline stylesheets.
///
/// HTML's style-data state leaves the wrapper in the text node when an XHTML
/// test is parsed through the HTML-compatible sink. CSS sees `<![CDATA[` as
/// invalid leading tokens otherwise, so the whole stylesheet is silently
/// dropped even though the same source is valid in an XML-aware browser.
fn strip_xhtml_cdata_wrapper(text: &str) -> String {
    let trimmed = text.trim();
    trimmed
        .strip_prefix("<![CDATA[")
        .and_then(|inner| inner.strip_suffix("]]>"))
        .map_or_else(|| text.to_owned(), ToOwned::to_owned)
}

/// Return whether an element is a stylesheet-bearing `<style>` element.
///
/// Host document CSS accepts HTML/XHTML and SVG style elements. SVG style
/// text also remains in its subtree so the SVG parser can apply paint-only
/// properties to the serialized vector content.
fn is_stylesheet_style_element(element: &impl raikiri_traits::Element) -> bool {
    element.tag_name() == "style"
        && matches!(
            element.namespace_uri(),
            None | Some("http://www.w3.org/1999/xhtml") | Some("http://www.w3.org/2000/svg")
        )
}

fn inline_stylesheet_has_content(doc: &Document, node_id: raikiri_traits::NodeId) -> bool {
    use raikiri_traits::{Dom, Node};

    doc.child_ids(node_id).any(|child_id| {
        let Some(child) = doc.node(child_id) else {
            return false;
        };
        child.text_content().is_some_and(|text| !text.is_empty())
    })
}

fn extract_inline_stylesheets(doc: &Document) -> Vec<String> {
    let mut sources = collect_head_stylesheet_sources(doc)
        .into_iter()
        .filter_map(|source| match source {
            HeadStylesheetSource::Inline { node_id } => Some(inline_stylesheet_text(doc, node_id)),
            HeadStylesheetSource::External { .. } => None,
        })
        .collect::<Vec<_>>();

    // WPT and browser HTML commonly place metadata `<style>` after the implicit
    // head, including directly in the body. Keep the existing head projection
    // first, then append body/outside-head styles in document order.
    sources.extend(
        collect_body_inline_stylesheet_ids(doc)
            .into_iter()
            .map(|node_id| inline_stylesheet_text(doc, node_id)),
    );
    sources
}

fn collect_body_inline_stylesheet_ids(doc: &Document) -> Vec<raikiri_traits::NodeId> {
    use raikiri_traits::{Dom, Node};

    let head_id = find_head_element(doc);
    let mut out = Vec::new();
    let mut stack = vec![doc.root_id()];
    while let Some(id) = stack.pop() {
        if head_id == Some(id) {
            continue;
        }
        let Some(node) = doc.node(id) else {
            continue;
        };
        if !node.is_in_document() {
            continue;
        }
        if node
            .as_element()
            .is_some_and(|element| is_stylesheet_style_element(&element))
        {
            if inline_stylesheet_has_content(doc, id) {
                out.push(id);
            }
            continue;
        }
        let children: Vec<_> = doc.child_ids(id).collect();
        for child_id in children.into_iter().rev() {
            stack.push(child_id);
        }
    }
    out
}

/// Find the `<head>` element in the Document tree with iterative DFS.
/// It is usually the first child under `<html>`, but html5ever tree building
/// can place it elsewhere. Return None if a linear scan finds no head.
fn find_head_element(doc: &Document) -> Option<raikiri_traits::NodeId> {
    use raikiri_traits::{Dom, Element, Node};

    let mut stack: Vec<raikiri_traits::NodeId> = vec![doc.root_id()];
    while let Some(id) = stack.pop() {
        if let Some(node) = doc.node(id)
            && let Some(el) = node.as_element()
            && el.tag_name() == "head"
        {
            return Some(id);
        }
        let kids: Vec<_> = doc.child_ids(id).collect();
        for c in kids.into_iter().rev() {
            stack.push(c);
        }
    }
    None
}

/// Find the first `<base>` element with a nonempty `href` inside `<head>`
/// in document (tree) order. Return `None` if there is none.
///
/// The HTML Standard's "document base URL" algorithm
/// (§4.2.7 The base element / §urls-and-fetching "document base URL")
/// uses the frozen base URL of the first `<base>` element **with an href
/// attribute** in the document, even if that attribute has an empty value.
/// This implementation cannot strictly reproduce that behavior:
/// `raikiri_traits::Element::attr` normalizes empty attribute values to `None`
/// (see the `disabled` attribute discussion in
/// `collect_external_stylesheet_hrefs`). Thus the trait cannot distinguish
/// `<base href="">` from `<base>` without an href attribute: it lacks a
/// `has_attribute` method, and adding one would expand the public raikiri-traits API.
/// Instead of the strict behavior (for example, in `<base href=""><base
/// href="https://cdn.example/">`, the first, empty href freezes the document
/// base URL and the second element is ignored), choose the first `<base>` with
/// a nonempty href. This gives nearly the same result for ordinary documents.
/// With only an empty-href `<base>`, joining against the fallback base URL
/// returns that same URL as ignoring the element would.
///
/// Search only inside the head with DFS, as in
/// `collect_external_stylesheet_hrefs`. Defer `<base>` elements in `<body>`:
/// this scope does not process stylesheet links outside `<head>`, so there
/// would be no stylesheet to which a body `<base>` could apply.
///
/// Trim href values. `Element::attr` filters only truly empty strings (`""`),
/// so whitespace-only `href="   "` values reach this code. Without trimming,
/// `Url::join("   ")` in `resolve_url` would resolve to the base URL itself
/// (as described for link hrefs in `collect_external_stylesheet_hrefs`),
/// mistaking an effectively empty href for an override.
pub(crate) fn find_document_base_href(doc: &Document) -> Option<String> {
    use raikiri_traits::{Dom, Element, Node};

    let head_id = find_head_element(doc)?;

    let mut stack: Vec<raikiri_traits::NodeId> = vec![head_id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.node(id) else {
            // cov:ignore: unreachable in practice, mirrors the identical
            // defensive branch in `collect_external_stylesheet_hrefs` below.
            continue;
        };
        if !node.is_in_document() {
            continue;
        }
        if let Some(el) = node.as_element()
            && el.tag_name() == "base"
            && let Some(href) = el.attr("href")
        {
            let trimmed = href.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        let kids: Vec<_> = doc.child_ids(id).collect();
        for c in kids.into_iter().rev() {
            stack.push(c);
        }
    }
    None
}

/// Collect `<link rel="stylesheet" href="...">` elements inside `<head>`
/// in document order.
///
/// Keep collection of external links scoped to head-only DFS. Body `<link>`
/// support is deferred separately from body inline `<style>` support.
/// Do not fetch resources here (`NetworkProvider` I/O): `TreeSink::finish()`
/// in this module must remain free of I/O. Adding observable side effects
/// to the sink would cross the wall/sink boundary. Collect hrefs only; defer
/// fetching to post-processing in `parse.rs::parse_with_sink`, where
/// `ParseOptions::network` and `base_url` are available.
///
/// Do not inspect the `disabled` boolean attribute. The
/// `raikiri_traits::Element::attr` contract normalizes valueless and empty
/// values (`disabled` and `disabled=""`) to `None` (see
/// `raikiri-dom/src/dom_impl.rs::ElementRef::attr` and its
/// `.filter(|s| !s.is_empty())`). The trait cannot inspect the **presence**
/// of a boolean attribute rather than its **value**, because it lacks
/// `has_attribute`. Adding that method would expand the public raikiri-traits
/// API and cross wall/traits; it is tracked as a separate follow-up.
///
/// The hrefs collected here are raw attribute values, not yet adjusted
/// for `<base>`. [`find_document_base_href`] separately locates and resolves
/// `<base href>` inside `<head>`. The choice of base URL for resolving each
/// href belongs to `parse.rs::fetch_external_stylesheets`. This function
/// only collects hrefs; it does not resolve URLs.
///
/// Preferred/selected stylesheet-set semantics for `rel="alternate stylesheet"`
/// with a `title` are not implemented. See the `is_stylesheet_link` docs for
/// why alternate links with nonempty titles are always excluded. Another
/// deviation remains, in the other direction, and is not covered there:
/// titled non-alternate links (preferred stylesheets) are always applied
/// because preferred sets are not tracked. Per spec, a named set other than
/// the preferred set should be disabled when there are multiple sets;
/// that behavior is missing for both `<link>` and `<style>`.
#[cfg(test)]
pub(crate) fn collect_external_stylesheet_hrefs(
    doc: &Document,
) -> Vec<(raikiri_traits::NodeId, String)> {
    collect_head_stylesheet_sources(doc)
        .into_iter()
        .filter_map(|source| match source {
            HeadStylesheetSource::Inline { .. } => None,
            HeadStylesheetSource::External { node_id, href } => Some((node_id, href)),
        })
        .collect()
}

/// Return true only if the `rel` token list includes `stylesheet` (ASCII
/// case-insensitive) and `type` is absent, empty, or `text/css`
/// (case-insensitive, ignoring MIME parameters).
/// This implements only the relevant external-resource-link checks from
/// HTML Standard §4.2.4 (The link element). `media`, `crossorigin`,
/// `integrity`, and `disabled` remain out of scope (see `collect_external_stylesheet_hrefs`).
///
/// An empty `type` attribute (`type=""`) means no type was specified, just
/// as if the attribute were absent. The gate checks whether a MIME **value**
/// was specified, not merely whether the attribute exists, so an empty
/// value imposes no restriction (and is compatible with stylesheets).
/// `Element::attr` returns `Some("")` for `type=""`, intentionally distinguishing
/// it from absence (see `raikiri-dom::dom_impl::ElementRef::attr`). This
/// function therefore treats an empty value as unrestricted explicitly.
///
/// For nonempty `type`, ignore anything after `;` (a MIME parameter, such as
/// `text/css; charset=utf-8`). This matches browsers' actual "type attribute
/// gate": they ignore parameters and compare only the MIME essence.
///
/// If `rel` also contains `alternate` and `title` is nonempty, return
/// false unconditionally (do not apply the sheet). The CSSOM "add a CSS
/// style sheet" algorithm (<https://drafts.csswg.org/cssom/#add-a-css-style-sheet>)
/// only enables this type of sheet under these conditions:
///
/// - Step 4 changes the preferred stylesheet set name to this sheet's
///   title only if the alternate flag is unset and the preferred set name
///   is empty. Alternate sheets therefore cannot determine the preferred
///   set name.
/// - Step 5 unsets the disabled flag if the title is empty, matches the
///   preferred set name (when the last set name is null), or matches
///   the last set name selected by the user.
///
/// Therefore, whether an alternate sheet with a nonempty title is active
/// depends on titles of other `<link>` elements (which determine the
/// preferred set name) or the user's selected stylesheet set (last set name).
/// This crate tracks neither. This predicate sees only attributes of one
/// `<link>` and cannot make that decision, so it always excludes the sheet.
/// That does **not** mean the spec could never select such a sheet; rather,
/// this predicate lacks the information needed to tell if it is selected.
/// An alternate sheet with an empty title remains applicable: the first
/// branch of step 5 (empty title) unsets the disabled flag unconditionally,
/// as before.
fn is_stylesheet_link(rel: Option<&str>, type_attr: Option<&str>, title: Option<&str>) -> bool {
    let Some(rel) = rel else {
        return false;
    };
    let mut has_stylesheet_token = false;
    let mut has_alternate_token = false;
    for token in rel.split_ascii_whitespace() {
        if token.eq_ignore_ascii_case("stylesheet") {
            has_stylesheet_token = true;
        } else if token.eq_ignore_ascii_case("alternate") {
            has_alternate_token = true;
        }
    }
    if !has_stylesheet_token {
        return false;
    }
    // Titled alternate stylesheet: excluded unconditionally, see doc
    // comment above. Checks non-emptiness directly (rather than relying on
    // `Element::attr`'s empty-string-to-`None` normalization at the sole
    // call site) so the predicate is self-consistent for any caller,
    // including this module's own unit tests below.
    if has_alternate_token && title.is_some_and(|t| !t.is_empty()) {
        return false;
    }
    type_attr.is_none_or(|t| {
        t.is_empty()
            || t.split(';')
                .next()
                .unwrap_or(t)
                .trim()
                .eq_ignore_ascii_case("text/css")
    })
}

/// Copy the contents of the metadata tables (`qual_names` / `attributes`)
/// into raikiri-dom::Node.
///
/// - `qual_names`: if an element's namespace URI is not the HTML default
///   (`ns!(html)`), store it in `Node.namespace`; HTML uses optimized `None`.
/// - `attributes`: copy only null-namespace attributes to raikiri-dom.
///   Defer namespaced attributes such as SVG `xlink:href`. Separate `style`
///   into `Node.inline_style`; store the rest in ordered `Node.attributes`.
///
/// This single-pass conversion runs once, during `finish()`. During parsing,
/// only the metadata tables (RefCell) change; Node remains unchanged.
/// Bulk population on finish handles attributes supplied by html5ever in
/// either add_attrs_if_missing / create_element callback order.
fn wire_side_tables(
    doc: &mut Document,
    qual_names: &FxHashMap<usize, QualName>,
    attributes: &FxHashMap<usize, Vec<Attribute>>,
) {
    for (idx, name) in qual_names {
        // Keep Node.namespace = None for the default HTML namespace (optimized path).
        // Store other svg / mathml / xml / ... namespace URIs as SmolStr.
        let namespace =
            (name.ns != ns!(html)).then(|| SmolStr::new(AsRef::<str>::as_ref(&name.ns)));
        let prefix = name
            .prefix
            .as_ref()
            .map(|prefix| SmolStr::new(AsRef::<str>::as_ref(prefix)));
        if namespace.is_some() || prefix.is_some() {
            doc.set_element_namespace_info(*idx, namespace, prefix);
        }
    }
    for (idx, attrs) in attributes {
        let mut inline_style: Option<SmolStr> = None;
        let mut native: Vec<(SmolStr, SmolStr)> = Vec::with_capacity(attrs.len());
        let mut namespaced = Vec::new();
        for a in attrs {
            if a.name.ns != ns!() {
                namespaced.push((
                    SmolStr::new(AsRef::<str>::as_ref(&a.name.ns)),
                    a.name
                        .prefix
                        .as_ref()
                        .map(|prefix| SmolStr::new(AsRef::<str>::as_ref(prefix))),
                    SmolStr::new(AsRef::<str>::as_ref(&a.name.local)),
                    SmolStr::new(a.value.as_ref()),
                ));
                continue;
            }
            let local = AsRef::<str>::as_ref(&a.name.local);
            if local == "style" {
                // The Element trait maps empty `style=""` to None. Keep the raw value
                // on Node without normalizing it at this boundary; dom_impl filters it.
                // html5ever is expected to deduplicate attributes during parsing, but do
                // not depend on that: defensively use first-wins. This matches
                // Node.attributes.find() first-match behavior and HTML spec §13.2.5.32,
                // which says to ignore later occurrences of duplicate attributes.
                if inline_style.is_none() {
                    inline_style = Some(SmolStr::new(a.value.as_ref()));
                }
                continue;
            }
            native.push((SmolStr::new(local), SmolStr::new(a.value.as_ref())));
        }
        if let Some(source) = inline_style {
            doc.set_element_inline_style(*idx, Some(source));
        }
        if !native.is_empty() {
            doc.set_element_attributes(*idx, native);
        }
        for (namespace, prefix, local, value) in namespaced {
            let _ = doc.set_element_namespaced_attribute(*idx, namespace, prefix, local, value);
        }
    }
}

/// Convert html5ever `QuirksMode` to raikiri-native `QuirksMode`
/// without exposing an html5ever type in raikiri-traits.
fn convert_quirks(mode: QuirksMode) -> raikiri_traits::QuirksMode {
    match mode {
        QuirksMode::Quirks => raikiri_traits::QuirksMode::Quirks,
        QuirksMode::LimitedQuirks => raikiri_traits::QuirksMode::LimitedQuirks,
        QuirksMode::NoQuirks => raikiri_traits::QuirksMode::NoQuirks,
    }
}

#[cfg(test)]
mod stylesheet_link_tests {
    use super::is_stylesheet_link;

    #[test]
    fn no_rel_attribute_is_not_a_stylesheet_link() {
        assert!(!is_stylesheet_link(None, None, None));
    }

    #[test]
    fn rel_without_stylesheet_token_is_not_a_stylesheet_link() {
        assert!(!is_stylesheet_link(Some("icon"), None, None));
    }

    #[test]
    fn rel_stylesheet_token_is_case_insensitive() {
        assert!(is_stylesheet_link(Some("StyleSheet"), None, None));
        assert!(is_stylesheet_link(Some("STYLESHEET"), None, None));
    }

    #[test]
    fn rel_stylesheet_among_multiple_space_separated_tokens_matches() {
        assert!(is_stylesheet_link(Some("alternate stylesheet"), None, None));
        assert!(is_stylesheet_link(Some("stylesheet next"), None, None));
    }

    #[test]
    fn absent_type_attribute_is_treated_as_stylesheet() {
        assert!(is_stylesheet_link(Some("stylesheet"), None, None));
    }

    #[test]
    fn empty_type_attribute_is_treated_as_stylesheet_same_as_absent() {
        // `type=""` is "type unspecified", not "type is the empty MIME
        // essence" — it must gate identically to a wholly absent `type`
        // attribute (both `Some("")` and `None` reach this predicate now
        // that `Element::attr` distinguishes "present with empty value"
        // from "absent"; see the doc comment above `is_stylesheet_link`).
        assert!(is_stylesheet_link(Some("stylesheet"), Some(""), None));
    }

    #[test]
    fn type_text_css_case_insensitive_is_treated_as_stylesheet() {
        assert!(is_stylesheet_link(
            Some("stylesheet"),
            Some("text/css"),
            None
        ));
        assert!(is_stylesheet_link(
            Some("stylesheet"),
            Some("Text/CSS"),
            None
        ));
    }

    #[test]
    fn non_css_type_attribute_is_not_treated_as_stylesheet() {
        assert!(!is_stylesheet_link(
            Some("stylesheet"),
            Some("application/rss+xml"),
            None
        ));
    }

    #[test]
    fn type_with_charset_mime_parameter_is_still_treated_as_stylesheet() {
        // browsers ignore MIME parameters (charset, etc.) when gating on the
        // `type` attribute's essence — only `text/css` (before any `;`)
        // matters.
        assert!(is_stylesheet_link(
            Some("stylesheet"),
            Some("text/css; charset=utf-8"),
            None
        ));
        assert!(is_stylesheet_link(
            Some("stylesheet"),
            Some("TEXT/CSS;charset=UTF-8"),
            None
        ));
    }

    #[test]
    fn non_css_essence_with_mime_parameter_is_not_treated_as_stylesheet() {
        assert!(!is_stylesheet_link(
            Some("stylesheet"),
            Some("application/rss+xml; charset=utf-8"),
            None
        ));
    }

    #[test]
    fn untitled_alternate_stylesheet_is_treated_as_stylesheet() {
        // CSSOM "add a CSS style sheet" step 5: an empty title unsets the
        // disabled flag unconditionally, regardless of the alternate flag.
        assert!(is_stylesheet_link(Some("alternate stylesheet"), None, None));
    }

    #[test]
    fn empty_string_title_on_alternate_stylesheet_is_treated_as_stylesheet() {
        // Same as the `None` case above, but exercises `Some("")` directly
        // rather than relying on the call site's `Element::attr` contract
        // (which normalizes an empty attribute value to `None` before this
        // function ever sees it) to collapse the two.
        assert!(is_stylesheet_link(
            Some("alternate stylesheet"),
            None,
            Some("")
        ));
    }

    #[test]
    fn titled_alternate_stylesheet_is_excluded() {
        // CSSOM "add a CSS style sheet" step 4-6: a titled alternate
        // stylesheet only has its disabled flag unset if it matches the
        // page's preferred/selected stylesheet set. This crate tracks
        // neither, so it can never legitimately be "selected" — excluded
        // unconditionally rather than applied as if always preferred.
        assert!(!is_stylesheet_link(
            Some("alternate stylesheet"),
            None,
            Some("High Contrast")
        ));
    }

    #[test]
    fn titled_alternate_stylesheet_is_excluded_regardless_of_type_match() {
        assert!(!is_stylesheet_link(
            Some("stylesheet alternate"),
            Some("text/css"),
            Some("High Contrast")
        ));
    }

    #[test]
    fn titled_non_alternate_stylesheet_link_still_applies() {
        // No `alternate` token in `rel` — a titled *non-alternate* link is
        // the preferred stylesheet (its title becomes the page's preferred
        // stylesheet set name, per CSSOM "add a CSS style sheet" step 4),
        // so it's unaffected by the alternate-only exclusion.
        assert!(is_stylesheet_link(
            Some("stylesheet"),
            None,
            Some("Default")
        ));
    }
}

#[cfg(test)]
mod collect_external_stylesheet_hrefs_tests {
    use super::collect_external_stylesheet_hrefs;
    use raikiri_dom::Document;

    #[test]
    fn document_without_a_head_element_yields_no_hrefs() {
        // find_head_element's None branch: a Document that never got a
        // <head> attached at all (html5ever's tree construction always
        // synthesizes one, so this only happens for a hand-built Document
        // like this one — exercised directly since collect_external_stylesheet_hrefs
        // is pub(crate) and doesn't need the full parse pipeline).
        let doc = Document::new();
        assert!(collect_external_stylesheet_hrefs(&doc).is_empty());
    }
}

#[cfg(test)]
mod find_document_base_href_tests {
    use super::find_document_base_href;
    use raikiri_dom::Document;

    #[test]
    fn document_without_a_head_element_yields_no_base_href() {
        // Mirrors collect_external_stylesheet_hrefs_tests's identical case:
        // find_head_element's None branch, only reachable via a hand-built
        // Document (html5ever's tree construction always synthesizes a
        // <head>). Full document-order / trim / empty-href / <template>
        // behavior is exercised at the `parse()` level in lib.rs, where a
        // mock `NetworkProvider` can observe which URL was actually
        // resolved and requested.
        let doc = Document::new();
        assert!(find_document_base_href(&doc).is_none());
    }
}

#[cfg(test)]
mod inline_stylesheet_text_tests {
    use super::strip_xhtml_cdata_wrapper;

    #[test]
    fn strips_xhtml_cdata_wrapper_but_keeps_plain_css() {
        assert_eq!(
            strip_xhtml_cdata_wrapper("\n<![CDATA[\nbody { color: red }\n]]>\n"),
            "\nbody { color: red }\n"
        );
        assert_eq!(
            strip_xhtml_cdata_wrapper("body { color: red }"),
            "body { color: red }"
        );
    }
}
