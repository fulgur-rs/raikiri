//! `Node`, `Element`, and `Document` members.

use boa_engine::{Context, JsResult, JsString, JsValue};
use raikiri_dom::NodeKind;

use super::host::HostError;
use super::interfaces::{HTML_NS, Members, wrap, wrap_optional};
use super::webidl::{
    dom_string, host_failure, host_failure_with_message, this_document, this_element, this_node,
    throw_dom_exception, unreachable_mutation_error, with_state,
};

/// Record that the DOM changed so the next layout-dependent read flushes.
pub(crate) fn mark_dirty(context: &mut Context) -> JsResult<()> {
    with_state(context, |s| s.dirty = true)
}

pub(crate) fn js_str(s: &str) -> JsValue {
    JsValue::from(JsString::from(s))
}

fn kind(context: &mut Context, index: usize) -> JsResult<Option<NodeKind>> {
    with_state(context, |s| {
        s.host.document().get_node(index).map(|n| n.kind())
    })
}

// ---- Node ------------------------------------------------------------

/// `Node.nodeType` (DOM §4.4).
fn node_type(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    Ok(JsValue::from(match kind(context, index)? {
        Some(NodeKind::Element) => 1,
        Some(NodeKind::Text) => 3,
        Some(NodeKind::ProcessingInstruction) => 7,
        Some(NodeKind::Comment) => 8,
        Some(NodeKind::Document) => 9,
        Some(NodeKind::DocumentFragment) => 11,
        _ => 0,
    }))
}

/// `Node.nodeName` (DOM §4.4). A `ProcessingInstruction`'s `nodeName` is its
/// target, not a fixed string.
fn node_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let name = with_state(context, |s| {
        let doc = s.host.document();
        let node = doc.get_node(index)?;
        Some(match node.kind() {
            NodeKind::Element => html_uppercased_name(doc, index, node.tag_name()?),
            NodeKind::Text => "#text".to_owned(),
            NodeKind::Comment => "#comment".to_owned(),
            NodeKind::ProcessingInstruction => doc.processing_instruction_target(index)?.to_owned(),
            NodeKind::Document => "#document".to_owned(),
            NodeKind::DocumentFragment => "#document-fragment".to_owned(),
            _ => String::new(),
        })
    })?;
    Ok(name.map_or_else(JsValue::null, |n| js_str(&n)))
}

/// DOM §4.9 "HTML-uppercased qualified name" for HTML-namespace elements.
fn html_uppercased_name(doc: &raikiri_dom::Document, index: usize, tag: &str) -> String {
    if doc.element_namespace_uri(index) == Some(super::interfaces::HTML_NS) {
        tag.to_ascii_uppercase()
    } else {
        tag.to_owned()
    }
}

fn parent_node(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let parent = with_state(context, |s| s.host.document().parent_of(index))?;
    wrap_optional(context, parent)
}

fn parent_element(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let parent = with_state(context, |s| {
        let doc = s.host.document();
        doc.parent_of(index).filter(|&p| {
            doc.get_node(p)
                .is_some_and(|n| n.kind() == NodeKind::Element)
        })
    })?;
    wrap_optional(context, parent)
}

/// DOM §4.4 "descendant text content" for a DocumentFragment: every Text
/// descendant's data, concatenated in tree order. [`raikiri_dom::Document::
/// element_text_content`] computes the same thing for an Element root but
/// requires an Element index, so a DocumentFragment root needs its own
/// walk; done with an explicit stack, not recursion, the same as every
/// other tree walk in this runtime (a script can build an arbitrarily deep
/// tree, and a native stack overflow there would abort the process rather
/// than raise a catchable error).
fn fragment_text_content(doc: &raikiri_dom::Document, root: usize) -> String {
    let mut out = String::new();
    let Some(root_node) = doc.get_node(root) else {
        return out; // cov:ignore: callers only ever pass an index already resolved to this DocumentFragment's own kind, so it is always present
    };
    let mut stack: Vec<usize> = root_node.children.iter().rev().copied().collect();
    while let Some(index) = stack.pop() {
        let Some(node) = doc.get_node(index) else {
            continue; // cov:ignore: every stack entry comes from `root` or a node's own children, always valid in the same arena
        };
        if let Some(text) = node.text_content() {
            out.push_str(text);
        }
        stack.extend(node.children.iter().rev().copied());
    }
    out
}

fn text_content(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let text = with_state(context, |s| {
        let doc = s.host.document();
        match doc.get_node(index).map(|n| n.kind()) {
            Some(NodeKind::Element) => doc.element_text_content(index),
            Some(NodeKind::DocumentFragment) => Some(fragment_text_content(doc, index)),
            Some(NodeKind::Text | NodeKind::Comment | NodeKind::ProcessingInstruction) => {
                doc.character_data(index).map(str::to_owned)
            }
            _ => None,
        }
    })?;
    Ok(text.map_or_else(JsValue::null, |t| js_str(&t)))
}

/// `textContent` is typed `DOMString?` (nullable): WebIDL's ES-value
/// conversion for a nullable `DOMString?` maps both `null` and `undefined`
/// to the IDL null value, so either one clears the node the same way,
/// unlike plain `ToString` (which would otherwise stringify `undefined` to
/// `"undefined"`).
fn set_text_content(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let value = match args.first() {
        Some(v) if v.is_null() || v.is_undefined() => String::new(),
        _ => dom_string(args, 0, context)?,
    };
    let target_kind = with_state(context, |s| {
        s.host.document().get_node(index).map(|n| n.kind())
    })?;
    match target_kind {
        Some(NodeKind::Element) => {
            // DOM §4.4 textContent setter, DocumentFragment/Element branch:
            // "replace all" within this with a single new Text node, or with
            // nothing for an empty string. Realized here through raikiri-dom's
            // own `set_element_text_content` (clear-then-push-if-nonempty in a
            // single call) rather than through `tree::replace_all` below --
            // both implement the same "replace all" semantics.
            let result = with_state(context, |s| {
                s.host
                    .document_mut()
                    .set_element_text_content(index, &value)
            })?;
            if result.is_err() {
                // cov:ignore: `set_element_text_content` only rejects an out-of-range or
                // non-Element index; `target_kind` above already resolved this index to Element.
                return Err(unreachable_mutation_error());
            }
            mark_dirty(context)?;
        }
        Some(NodeKind::DocumentFragment) => {
            // DOM §4.4 textContent setter, DocumentFragment/Element branch:
            // "replace all" within this with a single new Text node, or with
            // nothing for an empty string.
            let text_node = if value.is_empty() {
                None
            } else {
                Some(with_state(context, |s| {
                    s.host.document_mut().create_detached_text(&value)
                })?)
            };
            super::tree::replace_all(context, index, text_node)?;
            mark_dirty(context)?;
        }
        Some(NodeKind::Text | NodeKind::Comment | NodeKind::ProcessingInstruction) => {
            // DOM §4.4 textContent setter, CharacterData branch: replace data outright.
            let result = with_state(context, |s| {
                s.host.document_mut().set_character_data(index, &value)
            })?;
            if result.is_err() {
                // cov:ignore: `target_kind` above already resolved this index to a
                // Text, Comment, or ProcessingInstruction node.
                return Err(unreachable_mutation_error());
            }
            mark_dirty(context)?;
        }
        // Document and any other kind: DOM §4.4 "Otherwise: do nothing."
        _ => {}
    }
    Ok(JsValue::undefined())
}

// ---- Element -----------------------------------------------------------

fn tag_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let name = with_state(context, |s| {
        let doc = s.host.document();
        doc.get_node(index)
            .and_then(|n| n.tag_name())
            .map(|t| html_uppercased_name(doc, index, t))
    })?;
    Ok(name.map_or_else(JsValue::null, |n| js_str(&n)))
}

fn local_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let name = with_state(context, |s| {
        s.host
            .document()
            .get_node(index)
            .and_then(|n| n.tag_name())
            .map(str::to_owned)
    })?;
    Ok(name.map_or_else(JsValue::null, |n| js_str(&n)))
}

pub(crate) fn attribute(
    context: &mut Context,
    index: usize,
    name: &str,
) -> JsResult<Option<String>> {
    with_state(context, |s| {
        s.host
            .document()
            .element_attribute(index, name)
            .map(str::to_owned)
    })
}

fn get_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let name = dom_string(args, 0, context)?;
    Ok(attribute(context, index, &name)?.map_or_else(JsValue::null, |v| js_str(&v)))
}

fn has_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let name = dom_string(args, 0, context)?;
    Ok(JsValue::from(attribute(context, index, &name)?.is_some()))
}

/// Set an element attribute and mark the host dirty. Shared by
/// `setAttribute` and every `classList` mutator, which all serialize
/// through the `class` attribute. `Element.setAttribute` (DOM §4.9) throws
/// `InvalidCharacterError` for a syntactically invalid name; `class` is
/// always a valid name, so `classList`'s own callers never observe the
/// error branch.
pub(crate) fn write_attribute(
    context: &mut Context,
    index: usize,
    name: &str,
    value: &str,
) -> JsResult<()> {
    let result = with_state(context, |s| {
        s.host
            .document_mut()
            .set_element_attribute(index, name, value)
    })?;
    if let Err(message) = result {
        return Err(throw_dom_exception(
            context,
            "InvalidCharacterError",
            &message,
        ));
    }
    mark_dirty(context)
}

fn set_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let name = dom_string(args, 0, context)?;
    let value = dom_string(args, 1, context)?;
    write_attribute(context, index, &name, &value)?;
    Ok(JsValue::undefined())
}

/// `Element.removeAttribute` (DOM §4.9): unlike `setAttribute`, the
/// algorithm never validates `name` — it just looks up an attribute by
/// that name and removes it if found, so a syntactically invalid name (one
/// raikiri-dom's `remove_element_attribute` rejects before touching
/// anything) is simply never found, and this is a silent no-op.
fn remove_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let name = dom_string(args, 0, context)?;
    let removed = with_state(context, |s| {
        s.host.document_mut().remove_element_attribute(index, &name)
    })?;
    if matches!(removed, Ok(Some(_))) {
        mark_dirty(context)?;
    }
    Ok(JsValue::undefined())
}

/// `Element.classList`: a `DOMTokenList` backed by the `class` attribute
/// (see `super::token_list`), cached per element so repeated reads return
/// the same object.
fn class_list(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    if let Some(existing) = with_state(context, |s| s.class_lists.get(&index).cloned())? {
        return Ok(existing.into());
    }
    let object = super::token_list::token_list(context, index)?;
    with_state(context, |s| s.class_lists.insert(index, object.clone()))?;
    Ok(object.into())
}

/// `Element.namespaceURI` (DOM §4.9): `null` for an element created with a
/// genuinely null namespace (`createElementNS(null, ...)`, represented
/// internally as the empty string, never conflated with `raikiri_dom`'s own
/// `None` == "HTML default" convention).
fn namespace_uri(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let ns = with_state(context, |s| {
        s.host
            .document()
            .element_namespace_uri(index)
            .map(str::to_owned)
    })?;
    Ok(match ns.as_deref() {
        None | Some("") => JsValue::null(),
        Some(uri) => js_str(uri),
    })
}

/// `Element.innerHTML` getter (HTML Standard §8.5.4 "The innerHTML
/// property": getter steps run the fragment serializing algorithm): a live
/// serialization of the element's children, computed fresh on every read
/// from the current arena state rather than a retained source string. An
/// error means some element under `index` carries an attribute name
/// `Document::serialize_inner_html` rejects as not a valid XML `Name`.
/// The HTML fragment serialization algorithm itself never re-validates
/// attribute names that way (that check is specific to `Element.setAttribute`,
/// DOM §4.9); an HTML-parsed attribute name only needs to avoid a handful of
/// forbidden characters, a much larger set than valid XML `Name`s, so a
/// real HTML fragment parser can legitimately produce a name this fails on.
/// Treated as a host failure, the same as the setter's own fragment-parse
/// failure, until raikiri-dom's serializer stops applying that check here.
/// `Document::serialize_inner_html`'s own message names the offending arena
/// index; that index is an implementation detail and must never be
/// observable from script, so the exception thrown into script carries a
/// fixed message instead, while the harness-facing host failure keeps the
/// original, more specific one.
fn inner_html(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let result = with_state(context, |s| s.host.document().serialize_inner_html(index))?;
    match result {
        Ok(html) => Ok(js_str(&html)),
        Err(message) => Err(host_failure_with_message(
            context,
            HostError(message),
            "innerHTML serialization failed: invalid attribute name",
        )),
    }
}

/// `Element.innerHTML` setter (HTML Standard §8.5.4 "The innerHTML
/// property": setter steps run the fragment parsing algorithm with `this`
/// as the context element). `[LegacyNullToEmptyString]` (the attribute's
/// WebIDL type): `null` sets the empty string rather than converting to the
/// string `"null"`. Parses the given markup as an HTML fragment through the
/// host (context element's tag name and namespace), then replaces the
/// element's children with the parsed result. Targeting a `<template>`
/// replaces its template contents instead of its direct children
/// (`Document::replace_children_from`).
fn set_inner_html(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let markup = match args.first() {
        Some(v) if v.is_null() => String::new(),
        _ => dom_string(args, 0, context)?,
    };
    let parsed = with_state(context, |s| {
        let doc = s.host.document();
        let tag = doc
            .get_node(index)
            .and_then(|n| n.tag_name())
            .unwrap_or_default()
            .to_owned();
        let ns = doc
            .element_namespace_uri(index)
            .unwrap_or(super::interfaces::HTML_NS)
            .to_owned();
        s.host.parse_fragment(&tag, &ns, &markup)
    })?;
    let fragment = match parsed {
        Ok(fragment) => fragment,
        Err(error) => return Err(host_failure(context, error)),
    };
    with_state(context, |s| {
        let root = fragment.root_index();
        s.host
            .document_mut()
            .replace_children_from(index, &fragment, root);
    })?;
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

// ---- Document ------------------------------------------------------------

fn first_element_child(
    doc: &raikiri_dom::Document,
    parent: usize,
    tag: Option<&str>,
) -> Option<usize> {
    doc.get_node(parent)?.children.iter().copied().find(|&c| {
        doc.get_node(c)
            .and_then(|n| n.tag_name())
            .is_some_and(|t| tag.is_none_or(|want| t == want))
    })
}

fn document_element(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_document(this, context)?;
    let html = with_state(context, |s| {
        first_element_child(s.host.document(), index, None)
    })?;
    wrap_optional(context, html)
}

fn html_child(this: &JsValue, context: &mut Context, tag: &str) -> JsResult<JsValue> {
    let index = this_document(this, context)?;
    let found = with_state(context, |s| {
        let doc = s.host.document();
        let html = first_element_child(doc, index, Some("html"))?;
        first_element_child(doc, html, Some(tag))
    })?;
    wrap_optional(context, found)
}

fn head(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    html_child(this, context, "head")
}

fn body(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    html_child(this, context, "body")
}

/// `Document.createElement` (DOM §4.5). Every element it creates is in the
/// HTML namespace, so a `<template>` always needs the same
/// template-contents fragment root the HTML parser wires for a parsed
/// `<template>` ([`raikiri_dom::Document::allocate_template_fragment_root`]);
/// without it, a later `innerHTML` write would land directly on the
/// template element's own children instead of its (isolated, inert)
/// contents.
fn create_element(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    this_document(this, context)?;
    let name = dom_string(args, 0, context)?.to_ascii_lowercase();
    let result = with_state(context, |s| {
        s.host.document_mut().create_detached_element(&name)
    })?;
    match result {
        Ok(index) => {
            if name == "template" {
                with_state(context, |s| {
                    s.host.document_mut().allocate_template_fragment_root(index);
                })?;
            }
            mark_dirty(context)?;
            Ok(wrap(context, index)?.into())
        }
        Err(message) => Err(throw_dom_exception(
            context,
            "InvalidCharacterError",
            &message,
        )),
    }
}

// ---- createElementNS ("validate and extract", DOM §4.5) ------------------

const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

// The XML `Name` production. `createElementNS`'s "validate and extract"
// needs to validate a namespace prefix and a local name independently of
// each other (see [`validate_and_extract`]), which `raikiri_dom`'s
// equivalent check cannot do: it is private, and it always validates and
// allocates an element in the same call (`create_detached_element`), with
// no way to just check a candidate string's characters. So the same
// character classes are kept here too, ASCII- and Unicode-range-for-range
// identical to `raikiri_dom::document::is_valid_xml_name`'s own.
fn is_xml_name_start(ch: char) -> bool {
    matches!(
        ch,
        ':' | 'A'..='Z'
            | '_'
            | 'a'..='z'
            | '\u{C0}'..='\u{D6}'
            | '\u{D8}'..='\u{F6}'
            | '\u{F8}'..='\u{2FF}'
            | '\u{370}'..='\u{37D}'
            | '\u{37F}'..='\u{1FFF}'
            | '\u{200C}'..='\u{200D}'
            | '\u{2070}'..='\u{218F}'
            | '\u{2C00}'..='\u{2FEF}'
            | '\u{3001}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FDCF}'
            | '\u{FDF0}'..='\u{FFFD}'
            | '\u{10000}'..='\u{EFFFF}'
    )
}

fn is_xml_name_char(ch: char) -> bool {
    is_xml_name_start(ch)
        || matches!(
            ch,
            '0'..='9' | '-' | '.' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}'
        )
}

/// A non-empty XML `Name`: used on an already-split prefix or local name
/// half (DOM's "validate" step itself runs over the whole qualified name;
/// splitting first and checking each half catches an extra, empty, or
/// invalid-on-its-own segment a whole-string check would miss).
fn is_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_xml_name_start) && chars.all(is_xml_name_char)
}

/// DOM §4.5 "validate and extract": split `qualified_name` into an optional
/// prefix and a local name, then check the namespace/prefix combinations
/// the spec restricts. `namespace` is normalized from an empty string to
/// `None` first (the algorithm's own first step); both inputs are already
/// `DOMString`-converted.
fn validate_and_extract(
    context: &mut Context,
    namespace: Option<String>,
    qualified_name: &str,
) -> JsResult<(Option<String>, String)> {
    let namespace = namespace.filter(|n| !n.is_empty());
    let (prefix, local_name) = match qualified_name.split_once(':') {
        None => {
            if !is_xml_name(qualified_name) {
                return Err(throw_dom_exception(
                    context,
                    "InvalidCharacterError",
                    "qualifiedName is not a valid XML qualified name",
                ));
            }
            (None, qualified_name.to_owned())
        }
        Some((prefix, local)) => {
            if prefix.is_empty()
                || local.is_empty()
                || local.contains(':')
                || !is_xml_name(prefix)
                || !is_xml_name(local)
            {
                return Err(throw_dom_exception(
                    context,
                    "InvalidCharacterError",
                    "qualifiedName is not a valid XML qualified name",
                ));
            }
            (Some(prefix.to_owned()), local.to_owned())
        }
    };
    if prefix.is_some() && namespace.is_none() {
        return Err(throw_dom_exception(
            context,
            "NamespaceError",
            "a prefixed qualified name requires a namespace",
        ));
    }
    if prefix.as_deref() == Some("xml") && namespace.as_deref() != Some(XML_NAMESPACE) {
        return Err(throw_dom_exception(
            context,
            "NamespaceError",
            "the 'xml' prefix requires the XML namespace",
        ));
    }
    let is_xmlns_name = qualified_name == "xmlns" || prefix.as_deref() == Some("xmlns");
    if is_xmlns_name != (namespace.as_deref() == Some(XMLNS_NAMESPACE)) {
        return Err(throw_dom_exception(
            context,
            "NamespaceError",
            "'xmlns' and the XMLNS namespace must go together",
        ));
    }
    Ok((namespace, local_name))
}

/// `Document.createElementNS` (DOM §4.5): unlike `createElement`, this never
/// lowercases the given name (case matters for a foreign-namespace element,
/// e.g. SVG's `linearGradient`). `raikiri_dom`'s `ElementData` has no
/// separate prefix slot, so a prefixed qualified name's `tagName`/
/// `localName` both fall back to just its local-name half; nothing in this
/// runtime's own tests or WPT content passes a prefixed qualified name, so
/// this only under-reports `tagName` for one (e.g. `xlink:href`), never
/// over-reports it.
fn create_element_ns(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    this_document(this, context)?;
    let namespace = match args.first() {
        Some(v) if v.is_null() || v.is_undefined() => None,
        _ => Some(dom_string(args, 0, context)?),
    };
    let qualified_name = dom_string(args, 1, context)?;
    let (namespace, local_name) = validate_and_extract(context, namespace, &qualified_name)?;
    let result = with_state(context, |s| {
        s.host.document_mut().create_detached_element(&local_name)
    })?;
    let index = match result {
        Ok(index) => index,
        // cov:ignore: `validate_and_extract` already confirmed `local_name`
        // (the qualified name, or its local half) is a valid XML Name.
        Err(message) => {
            return Err(throw_dom_exception(
                context,
                "InvalidCharacterError",
                &message,
            ));
        }
    };
    let is_html = namespace.as_deref() == Some(HTML_NS);
    with_state(context, |s| {
        // `None` here is `raikiri_dom`'s own "HTML default" optimization,
        // not a marker for a genuinely null namespace -- an explicit empty
        // string keeps a null-namespace element from ever being read back
        // as HTML (see `element_namespace_uri`'s `unwrap_or` default).
        let dom_namespace = match namespace.as_deref() {
            Some(HTML_NS) => None,
            Some(other) => Some(other.into()),
            None => Some("".into()),
        };
        s.host
            .document_mut()
            .set_element_namespace(index, dom_namespace);
    })?;
    if is_html && local_name == "template" {
        with_state(context, |s| {
            s.host.document_mut().allocate_template_fragment_root(index);
        })?;
    }
    mark_dirty(context)?;
    Ok(wrap(context, index)?.into())
}

// ---- Document.title (HTML "document.title") -------------------------------

/// The first HTML-namespace `title` element in the document, in tree order
/// (an explicit stack, not recursion: a script can build an arbitrarily
/// deep tree). An SVG `title` (or one in any other namespace) never counts,
/// matching the HTML `title` element's own definition.
fn find_html_title(doc: &raikiri_dom::Document, document_index: usize) -> Option<usize> {
    let mut stack: Vec<usize> = doc
        .get_node(document_index)?
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(index) = stack.pop() {
        let Some(node) = doc.get_node(index) else {
            continue; // cov:ignore: every stack entry comes from a resolved node's own children, always valid in the same arena
        };
        if node.kind() == NodeKind::Element
            && node.tag_name() == Some("title")
            && doc.element_namespace_uri(index) == Some(HTML_NS)
        {
            return Some(index);
        }
        stack.extend(node.children.iter().rev().copied());
    }
    None
}

/// The `head` element of a document whose document element is `html`
/// (matching [`head`]'s own lookup).
fn head_index(doc: &raikiri_dom::Document, document_index: usize) -> Option<usize> {
    let html = first_element_child(doc, document_index, Some("html"))?;
    first_element_child(doc, html, Some("head"))
}

/// HTML "child text content": the concatenation of `node`'s direct `Text`
/// children's data, in order -- narrower than [`raikiri_dom::Document::
/// element_text_content`]'s deep descendant walk, which `document.title`
/// does not want (a `<title>` normally only ever has `Text` children
/// anyway, but the algorithm is specified this way, not as a descendant
/// walk).
fn child_text_content(doc: &raikiri_dom::Document, node: usize) -> String {
    let mut out = String::new();
    let Some(node) = doc.get_node(node) else {
        return out; // cov:ignore: every caller resolves `node` from the document immediately before this call
    };
    for &child in &node.children {
        if let Some(text) = doc
            .get_node(child)
            .filter(|c| c.kind() == NodeKind::Text)
            .and_then(|c| c.text_content())
        {
            out.push_str(text);
        }
    }
    out
}

/// HTML "document.title" getter's normalization: strip leading/trailing
/// ASCII whitespace and collapse internal runs to a single space.
fn normalize_title(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

fn title(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_document(this, context)?;
    let text = with_state(context, |s| {
        let doc = s.host.document();
        find_html_title(doc, index).map(|t| child_text_content(doc, t))
    })?;
    Ok(js_str(&normalize_title(&text.unwrap_or_default())))
}

/// HTML "document.title" setter: reuses an existing title element verbatim
/// (its text is replaced with `value` exactly as given, unnormalized);
/// otherwise creates one in `head` (a no-op when there is no
/// `documentElement`, the document element is not HTML, or there is no
/// `head` -- an SVG document element's own title handling is out of scope
/// here).
fn set_title(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_document(this, context)?;
    let value = dom_string(args, 0, context)?;
    // `Ok(title)`: an existing title element to overwrite. `Err(head)`: no
    // title exists yet, but `head` does, so one is created there. `None`:
    // no-op (see the doc comment above for each reason).
    let plan: Option<Result<usize, usize>> = with_state(context, |s| {
        let doc = s.host.document();
        if let Some(title) = find_html_title(doc, index) {
            return Some(Ok(title));
        }
        let doc_element = first_element_child(doc, index, None)?;
        if doc.element_namespace_uri(doc_element) != Some(HTML_NS) {
            return None;
        }
        Some(Err(head_index(doc, index)?))
    })?;
    let title = match plan {
        None => return Ok(JsValue::undefined()),
        Some(Ok(title)) => title,
        Some(Err(head)) => {
            let created = with_state(context, |s| {
                s.host.document_mut().create_detached_element("title")
            })?;
            let new_title = match created {
                Ok(t) => t,
                // cov:ignore: "title" is a fixed, always-valid XML element name.
                Err(_) => return Err(unreachable_mutation_error()),
            };
            let attached = with_state(context, |s| {
                s.host.document_mut().append_child(head, new_title)
            })?;
            if attached.is_err() {
                // cov:ignore: `head` was just resolved to an existing Element above.
                return Err(unreachable_mutation_error());
            }
            new_title
        }
    };
    let result = with_state(context, |s| {
        s.host
            .document_mut()
            .set_element_text_content(title, value.as_str())
    })?;
    if result.is_err() {
        // cov:ignore: `title` is either the found title element or the one
        // just created and attached above -- always an Element.
        return Err(unreachable_mutation_error());
    }
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

pub(crate) const NODE_MEMBERS: Members = Members {
    getters: &[
        ("nodeType", node_type),
        ("nodeName", node_name),
        ("parentNode", parent_node),
        ("parentElement", parent_element),
    ],
    accessors: &[("textContent", text_content, set_text_content)],
    methods: &[],
};

pub(crate) const ELEMENT_MEMBERS: Members = Members {
    getters: &[
        ("tagName", tag_name),
        ("localName", local_name),
        ("classList", class_list),
        ("namespaceURI", namespace_uri),
    ],
    accessors: &[("innerHTML", inner_html, set_inner_html)],
    methods: &[
        ("getAttribute", 1, get_attribute),
        ("hasAttribute", 1, has_attribute),
        ("setAttribute", 2, set_attribute),
        ("removeAttribute", 1, remove_attribute),
        (
            "getBoundingClientRect",
            0,
            super::style::get_bounding_client_rect,
        ),
    ],
};

pub(crate) const DOCUMENT_MEMBERS: Members = Members {
    getters: &[
        ("documentElement", document_element),
        ("head", head),
        ("body", body),
    ],
    accessors: &[("title", title, set_title)],
    methods: &[
        ("createElement", 1, create_element),
        ("createElementNS", 2, create_element_ns),
    ],
};

#[cfg(test)]
mod tests;
