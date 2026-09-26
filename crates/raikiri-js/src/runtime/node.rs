//! `Node`, `Element`, and `Document` members.

use boa_engine::object::JsObject;
use boa_engine::{Context, JsResult, JsString, JsValue, NativeFunction};
use raikiri_dom::NodeKind;

use super::host::HostError;
use super::interfaces::{Members, closure_function, wrap, wrap_optional};
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
    if doc.element_namespace_uri(index) == Some("http://www.w3.org/1999/xhtml") {
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

fn class_tokens(context: &mut Context, index: usize) -> JsResult<Vec<String>> {
    Ok(attribute(context, index, "class")?
        .unwrap_or_default()
        .split_ascii_whitespace()
        .map(str::to_owned)
        .collect())
}

/// A `DOMTokenList` token (DOM §7.1): non-empty, no ASCII whitespace.
fn validate_token(context: &mut Context, token: &str) -> JsResult<()> {
    if token.is_empty() {
        return Err(throw_dom_exception(
            context,
            "SyntaxError",
            "token is empty",
        ));
    }
    if token
        .chars()
        .any(|c| matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r'))
    {
        return Err(throw_dom_exception(
            context,
            "InvalidCharacterError",
            "token contains whitespace",
        ));
    }
    Ok(())
}

/// `Element.classList`: a live-ish `DOMTokenList` backed by the `class`
/// attribute, cached per element so repeated reads return the same object.
/// `add`/`remove`/`contains`/`toggle` cover the surface the CSS Text i18n
/// corpus needs; the rest of `DOMTokenList` (`length`, indexing, `value`,
/// `replace`, iteration) is future work.
fn class_list(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    if let Some(existing) = with_state(context, |s| s.class_lists.get(&index).cloned())? {
        return Ok(existing.into());
    }
    let object = JsObject::with_object_proto(context.intrinsics());
    let add = NativeFunction::from_copy_closure(move |_, args, ctx| {
        let mut tokens = class_tokens(ctx, index)?;
        for i in 0..args.len() {
            let token = dom_string(args, i, ctx)?;
            validate_token(ctx, &token)?;
            if !tokens.contains(&token) {
                tokens.push(token);
            }
        }
        write_attribute(ctx, index, "class", &tokens.join(" "))?;
        Ok(JsValue::undefined())
    });
    let remove = NativeFunction::from_copy_closure(move |_, args, ctx| {
        let mut tokens = class_tokens(ctx, index)?;
        for i in 0..args.len() {
            let token = dom_string(args, i, ctx)?;
            validate_token(ctx, &token)?;
            tokens.retain(|t| t != &token);
        }
        write_attribute(ctx, index, "class", &tokens.join(" "))?;
        Ok(JsValue::undefined())
    });
    let contains = NativeFunction::from_copy_closure(move |_, args, ctx| {
        let token = dom_string(args, 0, ctx)?;
        Ok(JsValue::from(class_tokens(ctx, index)?.contains(&token)))
    });
    let toggle = NativeFunction::from_copy_closure(move |_, args, ctx| {
        let token = dom_string(args, 0, ctx)?;
        validate_token(ctx, &token)?;
        let mut tokens = class_tokens(ctx, index)?;
        let present = tokens.contains(&token);
        let force = args
            .get(1)
            .filter(|v| !v.is_undefined())
            .map(JsValue::to_boolean);
        let want = force.unwrap_or(!present);
        if want && !present {
            tokens.push(token);
        } else if !want && present {
            tokens.retain(|t| t != &token);
        } else {
            return Ok(JsValue::from(want));
        }
        write_attribute(ctx, index, "class", &tokens.join(" "))?;
        Ok(JsValue::from(want))
    });
    for (name, f, length) in [
        ("add", add, 0),
        ("remove", remove, 0),
        ("contains", contains, 1),
        ("toggle", toggle, 1),
    ] {
        let function = closure_function(context, name, length, f)?;
        object.set(JsString::from(name), function, true, context)?;
    }
    with_state(context, |s| s.class_lists.insert(index, object.clone()))?;
    Ok(object.into())
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
            .unwrap_or("http://www.w3.org/1999/xhtml")
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
/// HTML namespace (there is no `createElementNS` binding yet), so a
/// `<template>` always needs the same template-contents fragment root the
/// HTML parser wires for a parsed `<template>`
/// ([`raikiri_dom::Document::allocate_template_fragment_root`]); without it,
/// a later `innerHTML` write would land directly on the template element's
/// own children instead of its (isolated, inert) contents.
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
    accessors: &[],
    methods: &[("createElement", 1, create_element)],
};

#[cfg(test)]
mod tests;
