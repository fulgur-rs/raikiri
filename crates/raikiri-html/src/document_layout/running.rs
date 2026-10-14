//! Running elements placed into page margin boxes (CSS GCPM 3 §1.2,
//! <https://www.w3.org/TR/css-gcpm-3/#running-elements>).
//!
//! The index of running elements and their standalone layout live in
//! raikiri-dom ([`RunningIndex`], [`raikiri_dom::layout_running_element`]).
//! This module keeps the layouts a document has made, and shows each one as
//! a [`Page`] read through the same accessors as a document page, so a
//! painter draws it with the code it already has.

use std::cell::OnceCell;

use raikiri_dom::{Document, RunningElementPage, RunningIndex};
use raikiri_style::CascadeResult;
use raikiri_traits::{NodeId, RenderError};

use super::Page;

/// Running-element layouts already made, kept as long as the document layout
/// so that later requests borrow them instead of laying the element out
/// again.
///
/// Each running element has an append-only chain of layouts, one per width
/// it was laid out at. A link is never removed or replaced once set, so a
/// layout handed out stays valid while the cache lives.
#[derive(Default)]
pub(crate) struct LayoutCache {
    /// One chain per running element, sorted by node.
    chains: Vec<(NodeId, OnceCell<Box<CachedLayout>>)>,
}

struct CachedLayout {
    width_bits: u32,
    layout: RunningElementLayout,
    next: OnceCell<Box<CachedLayout>>,
}

impl LayoutCache {
    /// An empty cache for the running elements of `index`.
    pub(crate) fn new(index: &RunningIndex) -> Self {
        let mut nodes: Vec<NodeId> = index.nodes().collect();
        nodes.sort_unstable_by_key(|node| node.0);
        Self {
            chains: nodes
                .into_iter()
                .map(|node| (node, OnceCell::new()))
                .collect(),
        }
    }

    /// The layout of running element `node` at `width`, made with `make` the
    /// first time it is asked for. `None` when `node` is not a running
    /// element or `make` gives no layout; neither that nor an error is kept.
    pub(crate) fn get_or_try_make<E>(
        &self,
        node: NodeId,
        width: f32,
        make: impl FnOnce() -> Result<Option<RunningElementLayout>, E>,
    ) -> Result<Option<&RunningElementLayout>, E> {
        let Ok(position) = self
            .chains
            .binary_search_by_key(&node.0, |(node, _)| node.0)
        else {
            return Ok(None);
        };
        let width_bits = width.to_bits();
        let mut slot = &self.chains[position].1;
        while let Some(link) = slot.get() {
            if link.width_bits == width_bits {
                return Ok(Some(&link.layout));
            }
            slot = &link.next;
        }
        let Some(layout) = make()? else {
            return Ok(None); // cov:ignore: a document with a body always emits a page slice.
        };
        let link = slot.get_or_init(|| {
            Box::new(CachedLayout {
                width_bits,
                layout,
                next: OnceCell::new(),
            })
        });
        Ok(Some(&link.layout))
    }
}

/// What a page needs to show the document's running elements.
#[derive(Clone, Copy)]
pub(crate) struct RunningSource<'a> {
    pub(crate) index: &'a RunningIndex,
    pub(crate) layouts: &'a LayoutCache,
    /// The laid-out document and its cascade.
    pub(crate) document: &'a Document,
    pub(crate) cascade: &'a CascadeResult,
}

impl<'a> RunningSource<'a> {
    /// Running element `node` laid out at `width`, made once per element
    /// and width.
    pub(crate) fn layout(
        self,
        node: NodeId,
        width: f32,
    ) -> Result<Option<&'a RunningElementLayout>, RenderError> {
        let width = used_width(width);
        self.layouts.get_or_try_make(node, width, || {
            layout_running_element(self.document, self.cascade, node, width)
        })
    }
}

/// A running element laid out on its own at a margin box's width.
///
/// The element is laid out with its own computed values, which it inherits
/// from its place in the document (CSS GCPM 3 §1.2.1), inside a containing
/// block of the requested width. [`Self::page`] reads the result through
/// the accessors of a document page: the element's top-left content edge of
/// the containing block is the page origin `(0, 0)`, so a painter offsets
/// everything by the margin box's content-box origin.
pub struct RunningElementLayout {
    node: NodeId,
    laid_out: RunningElementPage,
    pub(super) svg_sources: super::page::InlineSvgCache,
    margin_values: raikiri_dom::MarginBoxNamedValues,
}

impl std::fmt::Debug for RunningElementLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningElementLayout")
            .field("node", &self.node)
            .field("width", &self.width())
            .field("height", &self.height())
            .finish_non_exhaustive()
    }
}

impl RunningElementLayout {
    /// The running element.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The node of the laid-out document that `node`, a node of
    /// [`Self::page`], was copied from. `None` for a node the page does not
    /// have.
    pub fn source_node(&self, node: NodeId) -> Option<NodeId> {
        let index = usize::try_from(node.0).ok()?;
        self.laid_out
            .source_nodes
            .get(index)
            .map(|&source| NodeId(source as u64))
    }

    /// Width of the containing block the element was laid out in.
    pub fn width(&self) -> f32 {
        self.laid_out.geometry.page_box.width
    }

    /// Height from the containing block's top edge to the bottom margin edge
    /// of the element, the space it takes in the margin box.
    pub fn height(&self) -> f32 {
        self.laid_out.height
    }

    /// The laid-out element as a page of [`Self::width`] by
    /// [`Self::height`] with no page margins.
    ///
    /// The page is read from a copy that holds only the element's subtree
    /// and its ancestors, numbered on its own: node ids the page reports
    /// are that copy's, and [`Self::source_node`] maps them back to the
    /// document's.
    ///
    /// Besides the element's own fragments, the page can have a box fragment
    /// for `<body>`. It carries no decoration and extends below
    /// [`Self::height`], down the page the element was measured on, so a
    /// painter that clips to the margin box draws nothing of it.
    pub fn page(&self) -> Page<'_> {
        Page {
            slice: &self.laid_out.slice,
            geometry: &self.laid_out.geometry,
            style: &self.laid_out.cascade.page,
            document: &self.laid_out.document,
            cascade: &self.laid_out.cascade,
            page_count: 1,
            page_count_deferred: false,
            paired_style: None,
            running: None,
            svg_sources: &self.svg_sources,
            margin_values: &self.margin_values,
        }
    }
}

pub(crate) use raikiri_dom::used_running_element_width as used_width;

/// Lay running element `node` out at `width` CSS px, a width
/// [`used_width`] returned. See [`raikiri_dom::layout_running_element`].
pub(crate) fn layout_running_element(
    source: &Document,
    cascade: &CascadeResult,
    node: NodeId,
    width: f32,
) -> Result<Option<RunningElementLayout>, RenderError> {
    let laid_out = raikiri_dom::layout_running_element(source, cascade, node, width)?;
    Ok(laid_out.map(|laid_out| RunningElementLayout {
        node,
        laid_out,
        svg_sources: super::page::InlineSvgCache::default(),
        margin_values: raikiri_dom::MarginBoxNamedValues::new(),
    }))
}

#[cfg(test)]
mod tests;
