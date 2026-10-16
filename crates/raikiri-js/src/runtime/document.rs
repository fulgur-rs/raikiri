//! `Document.createElementNS` (DOM §4.5 "validate and extract"),
//! `Document.title` (HTML "document.title"), `document.readyState`/
//! `currentScript` (HTML "current document readiness" / "current script"),
//! and `document.URL`/`documentURI`.

use boa_engine::{Context, JsResult, JsValue};
use raikiri_dom::NodeKind;

use super::interfaces::{HTML_NS, wrap, wrap_optional};
use super::node::{first_element_child, guard_fragment_budget, js_str, mark_dirty};
use super::webidl::{
    dom_string, guard_node_budget, host_failure, this_document, throw_dom_exception,
    unreachable_mutation_error, with_state,
};

// ---- createElementNS ("validate and extract", DOM §4.5) ------------------

const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

/// Which kind of local name [`validate_and_extract`] checks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameContext {
    Element,
    Attribute,
}

/// DOM §4.5 "validate and extract": split `qualified_name` at its first `:`
/// into an optional prefix and a local name, returned with the namespace.
/// The prefix must be a valid namespace prefix and the local name a valid
/// element or attribute local name for `name_context`; then the
/// namespace/prefix combinations the spec restricts are checked.
/// `namespace` is normalized from an empty string to `None` first (the
/// algorithm's own first step); both inputs are already `DOMString`-converted.
pub(crate) fn validate_and_extract(
    context: &mut Context,
    namespace: Option<String>,
    qualified_name: &str,
    name_context: NameContext,
) -> JsResult<(Option<String>, Option<String>, String)> {
    let namespace = namespace.filter(|n| !n.is_empty());
    let (prefix, local_name) = match qualified_name.split_once(':') {
        None => (None, qualified_name),
        Some((prefix, local)) => (Some(prefix), local),
    };
    let valid_local = match name_context {
        NameContext::Element => raikiri_dom::document::is_valid_element_local_name(local_name),
        NameContext::Attribute => raikiri_dom::document::is_valid_attribute_local_name(local_name),
    };
    if !prefix.is_none_or(raikiri_dom::document::is_valid_namespace_prefix) || !valid_local {
        return Err(throw_dom_exception(
            context,
            "InvalidCharacterError",
            "qualifiedName is not a valid qualified name",
        ));
    }
    let (prefix, local_name) = (prefix.map(str::to_owned), local_name.to_owned());
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
    Ok((namespace, prefix, local_name))
}

/// `Document.createElementNS` (DOM §4.5): unlike `createElement`, this never
/// lowercases the given name (case matters for a foreign-namespace element,
/// e.g. SVG's `linearGradient`). The prefix half of a prefixed qualified
/// name is stored alongside the namespace, so `tagName`/`nodeName` report
/// the full qualified name while `localName` reports only the local half.
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
    let (namespace, prefix, local_name) =
        validate_and_extract(context, namespace, &qualified_name, NameContext::Element)?;
    guard_node_budget(context)?;
    let result = with_state(context, |s| {
        s.host.document_mut().create_detached_element(&local_name)
    })?;
    let index = match result {
        Ok(index) => index,
        // cov:ignore: `validate_and_extract` already confirmed `local_name`
        // (the qualified name, or its local half) is a valid element local name.
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
        s.host.document_mut().set_element_namespace_info(
            index,
            dom_namespace,
            prefix.map(Into::into),
        );
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
/// walk). Also used by [`super::scripts`] for a classic script element's own
/// inline source text, the same "child text content" concept HTML's script
/// processing model itself specifies.
pub(crate) fn child_text_content(doc: &raikiri_dom::Document, node: usize) -> String {
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
            guard_node_budget(context)?;
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
    if !value.is_empty() {
        guard_node_budget(context)?;
    }
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

// ---- readyState / currentScript / URL / documentURI -----------------------

/// HTML "current document readiness". [`super::scripts::run`] is the only
/// driver of `Loading`/`Interactive` in this crate; everything else only
/// ever reads the default `Complete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ReadyState {
    /// `"loading"`.
    Loading,
    /// `"interactive"`.
    Interactive,
    /// `"complete"`.
    #[default]
    Complete,
}

impl ReadyState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Interactive => "interactive",
            Self::Complete => "complete",
        }
    }
}

// ---- write / writeln (HTML "dynamic markup insertion") ----------------

/// Where `document.write` inserts while a parser-run script executes: right
/// after the script, then after the last node the script already wrote.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WritePoint {
    script: usize,
    last_written: Option<usize>,
}

impl WritePoint {
    pub(crate) fn after(script: usize) -> Self {
        Self {
            script,
            last_written: None,
        }
    }
}

/// `document.write(...text)`.
pub(crate) fn write(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    write_markup(this, args, context, "")
}

/// `document.writeln(...text)`: `write` followed by a line feed.
pub(crate) fn writeln(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    write_markup(this, args, context, "\n")
}

/// HTML "document write steps" while the parser waits on a script: the text
/// is parsed as if it followed the script's end tag. Parsing has already
/// finished when scripts run here, so the text is parsed by itself as a
/// fragment in the context of the script's parent, and the nodes are
/// inserted after the script, following anything it wrote before. Markup
/// split across calls is parsed one call at a time. Written `<script>`
/// elements run once the writing script returns, before the parser's next
/// script. A script that has left the tree has nowhere to write.
///
/// With no running parser script there is no insertion point, and the steps
/// would open the document again, which this runtime does not support:
/// that throws a "NotSupportedError" `DOMException`.
fn write_markup(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
    suffix: &str,
) -> JsResult<JsValue> {
    this_document(this, context)?;
    let mut markup = String::new();
    for index in 0..args.len() {
        markup.push_str(&dom_string(args, index, context)?);
    }
    markup.push_str(suffix);
    let Some(point) = with_state(context, |s| s.write_point)? else {
        return Err(throw_dom_exception(
            context,
            "NotSupportedError",
            "document.write outside a script the parser runs would reopen the document",
        ));
    };
    if markup.is_empty() {
        return Ok(JsValue::undefined());
    }
    let place = with_state(context, |s| {
        let document = s.host.document();
        let parent = document.parent_of(point.script)?;
        let tag = document.get_node(parent)?.tag_name()?.to_owned();
        let namespace = document.element_namespace_uri(parent)?.to_owned();
        let after = point
            .last_written
            .filter(|&last| document.parent_of(last) == Some(parent))
            .unwrap_or(point.script);
        let siblings = &document.get_node(parent)?.children;
        let next = siblings
            .iter()
            .position(|&child| child == after)
            .and_then(|position| siblings.get(position + 1))
            .copied();
        Some((parent, next, tag, namespace))
    })?;
    let Some((parent, next, tag, namespace)) = place else {
        return Ok(JsValue::undefined());
    };
    let fragment = match with_state(context, |s| {
        s.host.parse_fragment(&tag, &namespace, &markup)
    })? {
        Ok(fragment) => fragment,
        Err(error) => return Err(host_failure(context, error)),
    };
    guard_node_budget(context)?;
    let holder = with_state(context, |s| {
        s.host.document_mut().create_detached_fragment()
    })?;
    guard_fragment_budget(context, &fragment)?;
    let written = with_state(context, |s| {
        let document = s.host.document_mut();
        document.replace_children_from(holder, &fragment, fragment.root_index());
        let scripts = super::scripts::scripts_below(document, holder).unwrap_or_default();
        s.written_scripts.extend(scripts);
        super::stylesheet::inserted_nodes(s.host.document(), holder)
    })?;
    super::dispatch::sync_event_handlers_in_subtree(context, holder)?;
    super::image::update_images_below(context, holder)?;
    let inserted = with_state(context, |s| {
        s.host.document_mut().pre_insert(parent, holder, next)?;
        super::stylesheet::note_child_list_change(s, parent, &written);
        if let Some(&last) = written.last() {
            s.write_point = Some(WritePoint {
                last_written: Some(last),
                ..point
            });
        }
        Ok(())
    })?;
    inserted.map_err(|error| super::tree::map_mutation_error(context, error))?;
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

pub(crate) fn ready_state(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    this_document(this, context)?;
    let state = with_state(context, |s| s.ready_state)?;
    Ok(js_str(state.as_str()))
}

pub(crate) fn current_script(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    this_document(this, context)?;
    let script = with_state(context, |s| s.current_script)?;
    wrap_optional(context, script)
}

/// `document.URL`: [`super::host::DocumentHost::document_url`], or
/// `"about:blank"` when the host has none, matching `window.location.href`'s
/// own fallback.
pub(crate) fn url(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    this_document(this, context)?;
    let url = with_state(context, |s| s.host.document_url())?;
    Ok(js_str(&url.unwrap_or_else(|| "about:blank".to_owned())))
}

/// `document.documentURI`: identical to `document.URL` (HTML defines both
/// getters as the same algorithm; the two names exist only for legacy
/// SVG/DOM-Core compatibility).
pub(crate) fn document_uri(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    url(this, args, context)
}

#[cfg(test)]
mod tests;
