//! Running elements placed into page margin boxes (CSS GCPM 3 §1.2,
//! <https://www.w3.org/TR/css-gcpm-3/#running-elements>).
//!
//! The index of running elements and their standalone layout live in
//! raikiri-dom ([`RunningIndex`], [`raikiri_dom::layout_running_element`]).
//! This module keeps the layouts a document has made, and shows each one as
//! a [`Page`] read through the same accessors as a document page, so a
//! painter draws it with the code it already has.

use std::cell::{Cell, OnceCell};
use std::sync::Arc;

use raikiri_dom::{Document, RunningElementPage, RunningIndex};
use raikiri_style::CascadeResult;
use raikiri_traits::{LimitKind, NodeId, RenderError};

use super::Page;
use crate::svg_fonts::FaceCache;
use crate::svg_rendering::SvgSettings;

/// Running-element layouts already made, kept as long as the document layout
/// so that later requests borrow them instead of laying the element out
/// again.
///
/// Each element has an append-only digital search tree, keyed by the exact
/// normalized width bits. Set boxes never move, preserving borrowed layouts.
pub(crate) struct LayoutCache {
    /// A boxed slice keeps the document layout enum small.
    chains: Box<[(NodeId, OnceCell<Box<CachedLayout>>)]>,
    faces: Arc<FaceCache>,
    budget: Box<EntryBudget>,
}

struct EntryBudget {
    limit: usize,
    reserved: Cell<usize>,
}

struct Reservation<'a> {
    budget: &'a EntryBudget,
    committed: bool,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.budget.reserved.set(self.budget.reserved.get() - 1);
        }
    }
}

struct CachedLayout {
    width_bits: u32,
    layout: RunningElementLayout,
    children: [OnceCell<Box<CachedLayout>>; 2],
}

impl LayoutCache {
    pub(crate) fn new(index: &RunningIndex, faces: Arc<FaceCache>, limit: usize) -> Self {
        let mut nodes: Vec<NodeId> = index.nodes().collect();
        nodes.sort_unstable_by_key(|node| node.0);
        Self {
            chains: nodes
                .into_iter()
                .map(|node| (node, OnceCell::new()))
                .collect(),
            faces,
            budget: Box::new(EntryBudget {
                limit,
                reserved: Cell::new(0),
            }),
        }
    }

    /// The faces handed out to the SVG text of the document's layouts.
    pub(crate) fn faces(&self) -> &Arc<FaceCache> {
        &self.faces
    }

    /// Return an exact hit or its vacant slot, in at most 33 comparisons.
    fn slot<'a, const COUNT: bool>(
        mut slot: &'a OnceCell<Box<CachedLayout>>,
        bits: u32,
        meter: &mut crate::running_cache_diagnostics::Meter<COUNT>,
    ) -> &'a OnceCell<Box<CachedLayout>> {
        if COUNT {
            meter.work.lookups += 1;
        }
        let mut depth = 0;
        while let Some(link) = slot.get() {
            if COUNT {
                meter.work.width_comparisons += 1;
            }
            if link.width_bits == bits {
                return slot;
            }
            // At depth d the subtree shares d leading bits. At depth 32
            // equality is therefore certain, before this shift is reached.
            let branch = ((bits >> (31 - depth)) & 1) as usize;
            slot = &link.children[branch];
            depth += 1;
        }
        slot
    }

    /// Unknown nodes and failed/absent layouts consume no entry. A new
    /// node-width pair reserves capacity before invoking the builder.
    pub(crate) fn get_or_try_make(
        &self,
        node: NodeId,
        width: f32,
        make: impl FnOnce() -> Result<Option<RunningElementLayout>, RenderError>,
    ) -> Result<Option<&RunningElementLayout>, RenderError> {
        #[cfg(any(test, feature = "running-cache-diagnostics"))]
        if crate::running_cache_diagnostics::enabled() {
            return self.get_or_try_make_impl::<true>(node, width, make);
        }
        self.get_or_try_make_impl::<false>(node, width, make)
    }

    fn get_or_try_make_impl<const COUNT: bool>(
        &self,
        node: NodeId,
        width: f32,
        make: impl FnOnce() -> Result<Option<RunningElementLayout>, RenderError>,
    ) -> Result<Option<&RunningElementLayout>, RenderError> {
        let mut meter = crate::running_cache_diagnostics::Meter::<COUNT>::new();
        let Ok(position) = self
            .chains
            .binary_search_by_key(&node.0, |(node, _)| node.0)
        else {
            return Ok(None);
        };
        let root = &self.chains[position].1;
        let width_bits = width.to_bits();
        if let Some(link) = Self::slot(root, width_bits, &mut meter).get() {
            return Ok(Some(&link.layout));
        }
        let reserved = self.budget.reserved.get();
        if reserved >= self.budget.limit {
            if COUNT {
                meter.work.budget_denials += 1;
            }
            return Err(RenderError::LimitExceeded {
                kind: LimitKind::RunningLayoutEntries,
                limit: self.budget.limit as u64,
                actual: (reserved as u64).saturating_add(1),
            });
        }
        self.budget.reserved.set(reserved + 1);
        let mut reservation = Reservation {
            budget: &self.budget,
            committed: false,
        };
        if COUNT {
            meter.work.max_reserved_entries = reserved + 1;
            meter.work.make_calls += 1;
        }
        let Some(layout) = make()? else {
            return Ok(None);
        };
        if COUNT {
            meter.work.created_layouts += 1;
        }
        let completed = Box::new(CachedLayout {
            width_bits,
            layout,
            children: [OnceCell::new(), OnceCell::new()],
        });
        // The builder or allocator may have reentered. Re-search only after
        // allocating the complete box. Publication itself runs no user code
        // and allocates nothing; OnceCell makes this cache single-threaded.
        let slot = Self::slot(root, width_bits, &mut meter);
        if let Some(link) = slot.get() {
            return Ok(Some(&link.layout));
        }
        let link = slot.get_or_init(|| completed);
        reservation.committed = true;
        if COUNT {
            meter.work.admitted_entries += 1;
        }
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
    /// What the layout was asked for about SVG pictures.
    pub(crate) svg: SvgSettings,
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
            layout_running_element(
                self.document,
                self.cascade,
                node,
                width,
                self.layouts.faces().clone(),
                self.svg,
            )
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
    pub(super) svg_sources: super::inline_svg::InlineSvgCache,
    svg: SvgSettings,
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
            svg: self.svg,
            margin_values: &self.margin_values,
        }
    }
}

pub(crate) use raikiri_dom::used_running_element_width as used_width;

/// Lay running element `node` out at `width` CSS px, a width
/// [`used_width`] returned. See [`raikiri_dom::layout_running_element`].
/// The layout hands out SVG text faces from `faces`.
pub(crate) fn layout_running_element(
    source: &Document,
    cascade: &CascadeResult,
    node: NodeId,
    width: f32,
    faces: Arc<FaceCache>,
    svg: SvgSettings,
) -> Result<Option<RunningElementLayout>, RenderError> {
    let laid_out = raikiri_dom::layout_running_element(source, cascade, node, width)?;
    Ok(laid_out.map(|laid_out| RunningElementLayout {
        node,
        laid_out,
        svg_sources: super::inline_svg::InlineSvgCache::new(faces),
        svg,
        margin_values: raikiri_dom::MarginBoxNamedValues::new(),
    }))
}

#[cfg(test)]
mod tests;
