//! `NodeList` and `HTMLCollection` (DOM §4.2.10).
//!
//! Both are [`indexed_object`]s. A live collection (`childNodes`,
//! `children`, `getElementsByTagName`, `getElementsByClassName`) stores
//! only what it enumerates -- a root node plus a filter -- and resolves it
//! against the current tree on access, caching the resolved list per
//! collection for the current mutation generation, so repeated `length`,
//! `item`, index, or iteration accesses within one generation do not
//! re-walk the tree. A static `NodeList` (`querySelectorAll`) stores the
//! node list it was created with.
//!
//! Spec refs:
//! - <https://dom.spec.whatwg.org/#interface-nodelist>
//! - <https://dom.spec.whatwg.org/#interface-htmlcollection>
//! - <https://webidl.spec.whatwg.org/#es-iterators> (NodeList's iteration
//!   members and both interfaces' `@@iterator` are the `%Array.prototype%`
//!   functions themselves)

use std::cell::{Cell, RefCell};

use boa_engine::object::JsObject;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{Context, JsNativeError, JsResult, JsSymbol, JsValue};
use raikiri_dom::Document;

use super::indexed::{IndexedSource, indexed_object, this_indexed};
use super::interfaces::{HTML_NS, Members, protos, wrap};
use super::query::{elements_by_tag_name, elements_with_class_tokens};
use super::tree::element_children_of;
use super::webidl::{arg_unsigned_long, dom_string, with_state};

/// What a collection enumerates. Every kind except `Static` is resolved
/// against the current tree on access; the walking kinds are cached per
/// collection per generation by the wrappers below, while `ChildNodes`
/// borrows the child list directly (no walk, so no cache).
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
    /// (`getElementsByClassName`). An empty token list (the argument was
    /// empty or only ASCII whitespace) matches nothing, so the collection
    /// is always empty.
    ClassNames(usize, Vec<String>),
    /// A fixed list of nodes, in the order given.
    Static(Vec<usize>),
    /// Connected HTML elements supplying a named Window property.
    WindowNamed(String),
}

/// Resolved nodes for one live collection at one generation. Each collection
/// object owns one; a mutation bumps the shared generation (see the state
/// field it is cached against), so the next access re-walks once and later
/// accesses in the same generation reuse it.
#[derive(Debug, Default)]
struct CollectionCache {
    generation: Cell<Option<u64>>,
    nodes: RefCell<Vec<usize>>,
}

/// Run `f` over the nodes `source` currently contains, in collection order.
/// `Static` lends its stored list and `ChildNodes` borrows the child list
/// directly, both without touching `cache`; every other (walking) kind
/// reuses `cache` when it was resolved for the current generation and
/// re-walks exactly once per generation otherwise.
///
/// `f` must not borrow the shared state itself: it runs while that state is
/// borrowed to lend the document, so reaching back into it (for example to
/// wrap a node) would fail the re-entrant borrow check. Callers copy out an
/// index or length inside `f` and wrap afterwards.
fn with_live_nodes<T>(
    source: &CollectionSource,
    cache: &CollectionCache,
    context: &mut Context,
    f: impl FnOnce(&Document, &[usize]) -> T,
) -> JsResult<T> {
    match source {
        CollectionSource::Static(nodes) => {
            return with_state(context, |s| {
                let doc = s.host.document();
                f(doc, nodes)
            });
        }
        CollectionSource::ChildNodes(node) => {
            let node = *node;
            return with_state(context, |s| {
                let doc = s.host.document();
                let children = doc.get_node(node).map_or(&[][..], |n| &n.children[..]);
                f(doc, children)
            });
        }
        _ => {}
    }
    let generation = with_state(context, |s| s.generation)?;
    if cache.generation.get() == Some(generation) {
        let cached = cache.nodes.borrow();
        return with_state(context, |s| {
            let doc = s.host.document();
            f(doc, &cached)
        });
    }
    let fresh = with_state(context, |s| {
        #[cfg(test)]
        {
            s.live_walks += 1;
        }
        let doc = s.host.document();
        match source {
            CollectionSource::Children(node) => element_children_of(doc, *node),
            CollectionSource::TagName(root, query) => elements_by_tag_name(doc, *root, query),
            CollectionSource::ClassNames(root, tokens) => {
                elements_with_class_tokens(doc, *root, tokens)
            }
            CollectionSource::WindowNamed(name) => super::window::named::nodes(doc, name),
            // cov:ignore: Static and ChildNodes return through the fast paths above.
            CollectionSource::Static(_) | CollectionSource::ChildNodes(_) => Vec::new(),
        }
    })?;
    *cache.nodes.borrow_mut() = fresh;
    cache.generation.set(Some(generation));
    let cached = cache.nodes.borrow();
    with_state(context, |s| {
        let doc = s.host.document();
        f(doc, &cached)
    })
}

fn cached_length(
    source: &CollectionSource,
    cache: &CollectionCache,
    context: &mut Context,
) -> JsResult<usize> {
    with_live_nodes(source, cache, context, |_, nodes| nodes.len())
}

fn cached_item(
    source: &CollectionSource,
    cache: &CollectionCache,
    index: usize,
    context: &mut Context,
) -> JsResult<Option<JsValue>> {
    match with_live_nodes(source, cache, context, |_, nodes| {
        nodes.get(index).copied()
    })? {
        Some(node) => Ok(Some(wrap(context, node)?.into())),
        None => Ok(None),
    }
}

/// The source of a `NodeList`.
pub(crate) struct NodeListSource {
    source: CollectionSource,
    cache: CollectionCache,
}

/// The source of an `HTMLCollection`.
pub(crate) struct HtmlCollectionSource {
    source: CollectionSource,
    cache: CollectionCache,
}

impl NodeListSource {
    fn length(&self, context: &mut Context) -> JsResult<usize> {
        cached_length(&self.source, &self.cache, context)
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        cached_item(&self.source, &self.cache, index, context)
    }
}

impl HtmlCollectionSource {
    fn with_nodes<T>(
        &self,
        context: &mut Context,
        f: impl FnOnce(&Document, &[usize]) -> T,
    ) -> JsResult<T> {
        with_live_nodes(&self.source, &self.cache, context, f)
    }

    fn length(&self, context: &mut Context) -> JsResult<usize> {
        cached_length(&self.source, &self.cache, context)
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        cached_item(&self.source, &self.cache, index, context)
    }
}

impl IndexedSource for NodeListSource {
    fn length(&self, context: &mut Context) -> JsResult<usize> {
        self.length(context)
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        self.item(index, context)
    }
}

impl IndexedSource for HtmlCollectionSource {
    fn length(&self, context: &mut Context) -> JsResult<usize> {
        self.length(context)
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        self.item(index, context)
    }
}

/// A new `NodeList` over `source`.
pub(crate) fn node_list(context: &mut Context, source: CollectionSource) -> JsResult<JsObject> {
    let prototype = protos(context).node_list.clone();
    indexed_object(
        context,
        prototype,
        NodeListSource {
            source,
            cache: CollectionCache::default(),
        },
    )
}

/// A new `HTMLCollection` over `source`.
pub(crate) fn html_collection(
    context: &mut Context,
    source: CollectionSource,
) -> JsResult<JsObject> {
    let prototype = protos(context).html_collection.clone();
    indexed_object(
        context,
        prototype,
        HtmlCollectionSource {
            source,
            cache: CollectionCache::default(),
        },
    )
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
    let found = source.with_nodes(context, |doc, nodes| {
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

/// Install `NodeList`'s `iterable<Node>` members (`forEach`/`entries`/
/// `keys`/`values`/`@@iterator`, via [`super::interfaces::
/// install_value_iterable`]), plus `HTMLCollection`'s own `@@iterator` --
/// DOM does not declare `HTMLCollection` `iterable<>`, so it gets only
/// that one member, not the other four operations.
pub(crate) fn install_iteration(
    context: &mut Context,
    node_list: &JsObject,
    html_collection: &JsObject,
) -> JsResult<()> {
    super::interfaces::install_value_iterable(context, node_list)?;
    let values = context.intrinsics().objects().array_prototype_values();
    let iterator = PropertyDescriptor::builder()
        .value(values)
        .writable(true)
        .enumerable(false)
        .configurable(true);
    html_collection.define_property_or_throw(JsSymbol::iterator(), iterator, context)?;
    Ok(())
}

#[cfg(test)]
mod tests;
