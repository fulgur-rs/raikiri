//! `NodeList` and `HTMLCollection` (DOM §4.2.10).
//!
//! Both are [`indexed_object`]s. A live collection (`childNodes`,
//! `children`, `getElementsByTagName`, `getElementsByClassName`) stores
//! only what it enumerates -- a root node plus a filter -- and re-walks the
//! tree on every `length`, `item`, index, or iteration access, so it always
//! reflects the current tree. A static `NodeList` (`querySelectorAll`)
//! stores the node list it was created with.
//!
//! Spec refs:
//! - <https://dom.spec.whatwg.org/#interface-nodelist>
//! - <https://dom.spec.whatwg.org/#interface-htmlcollection>
//! - <https://webidl.spec.whatwg.org/#es-iterators> (NodeList's iteration
//!   members and both interfaces' `@@iterator` are the `%Array.prototype%`
//!   functions themselves)

use boa_engine::object::JsObject;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{Context, JsNativeError, JsResult, JsString, JsSymbol, JsValue};
use raikiri_dom::Document;

use super::indexed::{IndexedSource, indexed_object, this_indexed};
use super::interfaces::{HTML_NS, Members, protos, wrap};
use super::query::{elements_by_tag_name, elements_with_class_tokens};
use super::tree::element_children_of;
use super::webidl::{dom_string, with_state};

/// What a collection enumerates. Every kind except [`Self::Static`] is
/// re-evaluated against the current tree on each access.
#[derive(Debug, Clone)]
pub(crate) enum CollectionSource {
    /// Every child of the node.
    ChildNodes(usize),
    /// The element children of the node.
    Children(usize),
    /// Descendant elements of the root matching a qualified name
    /// (`getElementsByTagName`).
    TagName(usize, String),
    /// Descendant elements of the root carrying every class token
    /// (`getElementsByClassName`); the token list is never empty.
    ClassNames(usize, Vec<String>),
    /// A fixed list of nodes, in the order given.
    Static(Vec<usize>),
}

impl CollectionSource {
    /// Run `f` over the nodes the collection currently contains, in
    /// collection order. A live kind walks the current tree; a static one
    /// lends its stored list without copying it.
    fn with_nodes<T>(
        &self,
        context: &mut Context,
        f: impl FnOnce(&Document, &[usize]) -> T,
    ) -> JsResult<T> {
        with_state(context, |s| {
            let doc = s.host.document();
            let live = match self {
                Self::Static(nodes) => return f(doc, nodes),
                Self::ChildNodes(node) => doc
                    .get_node(*node)
                    .map(|n| n.children.clone())
                    .unwrap_or_default(),
                Self::Children(node) => element_children_of(doc, *node),
                Self::TagName(root, query) => elements_by_tag_name(doc, *root, query),
                Self::ClassNames(root, tokens) => elements_with_class_tokens(doc, *root, tokens),
            };
            f(doc, &live)
        })
    }

    fn length(&self, context: &mut Context) -> JsResult<usize> {
        self.with_nodes(context, |_, nodes| nodes.len())
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        match self.with_nodes(context, |_, nodes| nodes.get(index).copied())? {
            Some(node) => Ok(Some(wrap(context, node)?.into())),
            None => Ok(None),
        }
    }
}

/// The source of a `NodeList`.
pub(crate) struct NodeListSource(CollectionSource);

/// The source of an `HTMLCollection`.
pub(crate) struct HtmlCollectionSource(CollectionSource);

impl IndexedSource for NodeListSource {
    fn length(&self, context: &mut Context) -> JsResult<usize> {
        self.0.length(context)
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        self.0.item(index, context)
    }
}

impl IndexedSource for HtmlCollectionSource {
    fn length(&self, context: &mut Context) -> JsResult<usize> {
        self.0.length(context)
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        self.0.item(index, context)
    }
}

/// A new `NodeList` over `source`.
pub(crate) fn node_list(context: &mut Context, source: CollectionSource) -> JsResult<JsObject> {
    let prototype = protos(context).node_list.clone();
    indexed_object(context, prototype, NodeListSource(source))
}

/// A new `HTMLCollection` over `source`.
pub(crate) fn html_collection(
    context: &mut Context,
    source: CollectionSource,
) -> JsResult<JsObject> {
    let prototype = protos(context).html_collection.clone();
    indexed_object(context, prototype, HtmlCollectionSource(source))
}

/// WebIDL `unsigned long` conversion of a required argument `i`.
fn arg_unsigned_long(args: &[JsValue], i: usize, context: &mut Context) -> JsResult<usize> {
    let value = args.get(i).ok_or_else(|| {
        JsNativeError::typ().with_message("1 argument required, but only 0 present")
    })?;
    Ok(value.to_u32(context)? as usize)
}

fn node_list_length(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let source = this_indexed::<NodeListSource>(this, context, "NodeList")?;
    Ok(source.length(context)?.into())
}

fn node_list_item(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let source = this_indexed::<NodeListSource>(this, context, "NodeList")?;
    let index = arg_unsigned_long(args, 0, context)?;
    Ok(source.item(index, context)?.unwrap_or_else(JsValue::null))
}

pub(crate) const NODE_LIST_MEMBERS: Members = Members {
    getters: &[("length", node_list_length)],
    accessors: &[],
    methods: &[("item", 1, node_list_item)],
};

fn html_collection_length(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_indexed::<HtmlCollectionSource>(this, context, "HTMLCollection")?;
    Ok(source.length(context)?.into())
}

fn html_collection_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_indexed::<HtmlCollectionSource>(this, context, "HTMLCollection")?;
    let index = arg_unsigned_long(args, 0, context)?;
    Ok(source.item(index, context)?.unwrap_or_else(JsValue::null))
}

/// Whether element `node` is named `name` in the `namedItem` sense: its
/// `id` is `name`, or it is an HTML element whose `name` attribute is.
fn has_name(doc: &Document, node: usize, name: &str) -> bool {
    doc.element_attribute(node, "id") == Some(name)
        || (doc.element_namespace_uri(node) == Some(HTML_NS)
            && doc.element_attribute(node, "name") == Some(name))
}

/// `namedItem(name)`: the first element, in collection order, named `name`
/// (DOM §4.2.10.2); the empty string never matches.
fn html_collection_named_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_indexed::<HtmlCollectionSource>(this, context, "HTMLCollection")?;
    if args.is_empty() {
        return Err(JsNativeError::typ()
            .with_message("1 argument required, but only 0 present")
            .into());
    }
    let name = dom_string(args, 0, context)?;
    if name.is_empty() {
        return Ok(JsValue::null());
    }
    let found = source.0.with_nodes(context, |doc, nodes| {
        nodes.iter().copied().find(|&n| has_name(doc, n, &name))
    })?;
    match found {
        Some(node) => Ok(wrap(context, node)?.into()),
        None => Ok(JsValue::null()),
    }
}

pub(crate) const HTML_COLLECTION_MEMBERS: Members = Members {
    getters: &[("length", html_collection_length)],
    accessors: &[],
    methods: &[
        ("item", 1, html_collection_item),
        ("namedItem", 1, html_collection_named_item),
    ],
};

/// Install the iteration members WebIDL defines as the `%Array.prototype%`
/// functions themselves: `NodeList`'s `forEach` / `entries` / `keys` /
/// `values` (operations: writable, enumerable, configurable) and both
/// interfaces' `@@iterator` (`%Array.prototype.values%`; writable,
/// non-enumerable, configurable).
pub(crate) fn install_iteration(
    context: &mut Context,
    node_list: &JsObject,
    html_collection: &JsObject,
) -> JsResult<()> {
    let array_prototype = context.intrinsics().constructors().array().prototype();
    for name in ["forEach", "entries", "keys", "values"] {
        let function = array_prototype.get(JsString::from(name), context)?;
        let operation = PropertyDescriptor::builder()
            .value(function)
            .writable(true)
            .enumerable(true)
            .configurable(true);
        node_list.define_property_or_throw(JsString::from(name), operation, context)?;
    }
    let values = context.intrinsics().objects().array_prototype_values();
    for prototype in [node_list, html_collection] {
        let iterator = PropertyDescriptor::builder()
            .value(values.clone())
            .writable(true)
            .enumerable(false)
            .configurable(true);
        prototype.define_property_or_throw(JsSymbol::iterator(), iterator, context)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
