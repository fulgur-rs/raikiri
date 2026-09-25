//! `Node`, `Element`, and `Document` members.

use boa_engine::object::JsObject;
use boa_engine::{Context, JsNativeError, JsResult, JsString, JsValue, NativeFunction};
use raikiri_dom::NodeKind;
use raikiri_style::{SelectorQuery, StyleNodeId};

use super::interfaces::{Members, closure_function, wrap, wrap_optional};
use super::webidl::{
    arg_node, dom_string, this_document, this_element, this_node, throw_dom_exception, with_state,
};

/// Record that the DOM changed so the next layout-dependent read flushes.
pub(crate) fn mark_dirty(context: &mut Context) -> JsResult<()> {
    with_state(context, |s| s.dirty = true)
}

fn js_str(s: &str) -> JsValue {
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

/// `Node.nodeName` (DOM §4.4). Processing instructions report an empty
/// string rather than their target; a dedicated `ProcessingInstruction`
/// interface with its own `target` member would carry that value instead.
fn node_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let name = with_state(context, |s| {
        let doc = s.host.document();
        let node = doc.get_node(index)?;
        Some(match node.kind() {
            NodeKind::Element => html_uppercased_name(doc, index, node.tag_name()?),
            NodeKind::Text => "#text".to_owned(),
            NodeKind::Comment => "#comment".to_owned(),
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

fn text_content(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let text = with_state(context, |s| {
        let doc = s.host.document();
        match doc.get_node(index).map(|n| n.kind()) {
            Some(NodeKind::Element | NodeKind::DocumentFragment) => doc.element_text_content(index),
            Some(NodeKind::Text) => doc
                .get_node(index)
                .and_then(|n| n.text_content())
                .map(str::to_owned),
            _ => None,
        }
    })?;
    Ok(text.map_or_else(JsValue::null, |t| js_str(&t)))
}

fn set_text_content(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    // [LegacyNullToEmptyString]-like: null clears the node.
    let value = match args.first() {
        Some(v) if v.is_null() => String::new(),
        _ => dom_string(args, 0, context)?,
    };
    let result = with_state(context, |s| {
        match s.host.document().get_node(index).map(|n| n.kind()) {
            Some(NodeKind::Element) => s
                .host
                .document_mut()
                .set_element_text_content(index, &value)
                .map(|_| true),
            // Text/Comment data writes and fragment textContent arrive with the CharacterData work.
            _ => Ok(false),
        }
    })?;
    match result {
        Ok(true) => mark_dirty(context)?,
        Ok(false) => {}
        // `set_element_text_content` only rejects an out-of-range or non-Element index; the
        // match above already restricts this call to a resolved Element index.
        Err(message) => return Err(JsNativeError::error().with_message(message).into()), // cov:ignore: set_element_text_content never errors for an Element index resolved above
    }
    Ok(JsValue::undefined())
}

fn append_child(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_node(this, context)?;
    let child = arg_node(args, 0, context)?;
    let parent_ok = matches!(
        kind(context, parent)?,
        Some(NodeKind::Element | NodeKind::Document | NodeKind::DocumentFragment)
    );
    let child_is_document = kind(context, child)? == Some(NodeKind::Document);
    if !parent_ok || child_is_document {
        return Err(throw_dom_exception(
            context,
            "HierarchyRequestError",
            "node cannot be inserted here",
        ));
    }
    let result = with_state(context, |s| {
        s.host.document_mut().append_child(parent, child)
    })?;
    if let Err(message) = result {
        return Err(throw_dom_exception(
            context,
            "HierarchyRequestError",
            &message,
        ));
    }
    mark_dirty(context)?;
    Ok(args[0].clone())
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

fn attribute(context: &mut Context, index: usize, name: &str) -> JsResult<Option<String>> {
    with_state(context, |s| {
        s.host
            .document()
            .element_attribute(index, name)
            .map(str::to_owned)
    })
}

fn id(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    Ok(js_str(
        &attribute(context, index, "id")?.unwrap_or_default(),
    ))
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

/// Write (`Some`) or remove (`None`) an element attribute and mark the host
/// dirty. Shared by `setAttribute`/`removeAttribute` and every `classList`
/// mutator, which all serialize through the `class` attribute.
pub(crate) fn write_attribute(
    context: &mut Context,
    index: usize,
    name: &str,
    value: Option<&str>,
) -> JsResult<()> {
    let result = with_state(context, |s| {
        let doc = s.host.document_mut();
        match value {
            Some(value) => doc.set_element_attribute(index, name, value),
            None => doc.remove_element_attribute(index, name).map(|_| ()),
        }
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
    write_attribute(context, index, &name, Some(&value))?;
    Ok(JsValue::undefined())
}

fn remove_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let name = dom_string(args, 0, context)?;
    write_attribute(context, index, &name, None)?;
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
/// `add`/`remove`/`contains`/`toggle` cover the surface the legacy facade
/// exposed; the rest of `DOMTokenList` (`length`, indexing, `value`,
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
        write_attribute(ctx, index, "class", Some(&tokens.join(" ")))?;
        Ok(JsValue::undefined())
    });
    let remove = NativeFunction::from_copy_closure(move |_, args, ctx| {
        let mut tokens = class_tokens(ctx, index)?;
        for i in 0..args.len() {
            let token = dom_string(args, i, ctx)?;
            validate_token(ctx, &token)?;
            tokens.retain(|t| t != &token);
        }
        write_attribute(ctx, index, "class", Some(&tokens.join(" ")))?;
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
        write_attribute(ctx, index, "class", Some(&tokens.join(" ")))?;
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

/// Pre-order walk of `root`'s descendants, calling `visit` on every
/// **element**, with that element's ancestor **element** chain (root side
/// first, excluding `root` itself when it is not an element — in
/// particular the Document node is never pushed). Stops at the first
/// `Some`. This is the same ancestor shape
/// [`raikiri_style::SelectorQuery::matches`] expects, and what `:root`
/// (`ancestors.is_empty()`) relies on.
fn find_in_tree<T>(
    doc: &raikiri_dom::Document,
    root: usize,
    mut visit: impl FnMut(usize, &[StyleNodeId]) -> Option<T>,
) -> Option<T> {
    fn walk<T>(
        doc: &raikiri_dom::Document,
        node: usize,
        ancestors: &mut Vec<StyleNodeId>,
        visit: &mut dyn FnMut(usize, &[StyleNodeId]) -> Option<T>,
    ) -> Option<T> {
        let n = doc.get_node(node)?;
        let is_element = n.kind() == NodeKind::Element;
        if is_element && let Some(found) = visit(node, ancestors) {
            return Some(found);
        }
        if is_element {
            ancestors.push(StyleNodeId(node as u64));
        }
        for &child in &n.children {
            if let Some(found) = walk(doc, child, ancestors, visit) {
                return Some(found);
            }
        }
        if is_element {
            ancestors.pop();
        }
        None
    }
    walk(doc, root, &mut Vec::new(), &mut visit)
}

fn get_element_by_id(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_document(this, context)?;
    let id = dom_string(args, 0, context)?;
    if id.is_empty() {
        return Ok(JsValue::null());
    }
    let found = with_state(context, |s| {
        let doc = s.host.document();
        find_in_tree(doc, index, |e, _| {
            (doc.element_attribute(e, "id") == Some(id.as_str())).then_some(e)
        })
    })?;
    wrap_optional(context, found)
}

fn query_selector(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_document(this, context)?;
    let source = dom_string(args, 0, context)?;
    let query = match SelectorQuery::parse(&source) {
        Ok(query) => query,
        Err(message) => return Err(throw_dom_exception(context, "SyntaxError", &message)),
    };
    let found = with_state(context, |s| {
        // JS mutations leave `IS_IN_DOCUMENT` stale; the cascade matcher
        // consults it for sibling combinators and structural pseudo-classes,
        // so refresh it before walking (dirty-gated: a no-op when nothing
        // changed since the last refresh).
        s.host.document_mut().mark_in_document_flags();
        let doc = s.host.document();
        find_in_tree(doc, index, |e, ancestors| {
            query
                .matches(doc, StyleNodeId(e as u64), ancestors)
                .then_some(e)
        })
    })?;
    wrap_optional(context, found)
}

fn create_element(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    this_document(this, context)?;
    let name = dom_string(args, 0, context)?.to_ascii_lowercase();
    let result = with_state(context, |s| {
        s.host.document_mut().create_detached_element(&name)
    })?;
    match result {
        Ok(index) => {
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
    methods: &[("appendChild", 1, append_child)],
};

pub(crate) const ELEMENT_MEMBERS: Members = Members {
    getters: &[
        ("tagName", tag_name),
        ("localName", local_name),
        ("id", id),
        ("classList", class_list),
    ],
    accessors: &[],
    methods: &[
        ("getAttribute", 1, get_attribute),
        ("hasAttribute", 1, has_attribute),
        ("setAttribute", 2, set_attribute),
        ("removeAttribute", 1, remove_attribute),
    ],
};

pub(crate) const DOCUMENT_MEMBERS: Members = Members {
    getters: &[
        ("documentElement", document_element),
        ("head", head),
        ("body", body),
    ],
    accessors: &[],
    methods: &[
        ("getElementById", 1, get_element_by_id),
        ("querySelector", 1, query_selector),
        ("createElement", 1, create_element),
    ],
};

#[cfg(test)]
mod tests;
