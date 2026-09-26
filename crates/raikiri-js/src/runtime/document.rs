//! `Document.createElementNS` (DOM §4.5 "validate and extract") and
//! `Document.title` (HTML "document.title").

use boa_engine::{Context, JsResult, JsValue};
use raikiri_dom::NodeKind;

use super::interfaces::{HTML_NS, wrap};
use super::node::{first_element_child, js_str, mark_dirty};
use super::webidl::{
    dom_string, this_document, throw_dom_exception, unreachable_mutation_error, with_state,
};

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
pub(crate) fn create_element_ns(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
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
/// (matching `super::node::head`'s own lookup).
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

pub(crate) fn title(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
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
pub(crate) fn set_title(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_document(this, context)?;
    let value = dom_string(args, 0, context)?;
    // `Ok(title)`: an existing title element to overwrite. `Err(head)`: no
    // title exists yet, but `head` does, so one is created there. `None`:
    // no-op (see the doc comment above for each reason). The document
    // element's own namespace is checked first, before even looking for an
    // existing title: a non-HTML document element makes this a no-op
    // regardless of whether some stray HTML-namespace `title` element
    // happens to exist elsewhere in such a document.
    let plan: Option<Result<usize, usize>> = with_state(context, |s| {
        let doc = s.host.document();
        let doc_element = first_element_child(doc, index, None)?;
        if doc.element_namespace_uri(doc_element) != Some(HTML_NS) {
            return None;
        }
        if let Some(title) = find_html_title(doc, index) {
            return Some(Ok(title));
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

#[cfg(test)]
mod tests;
