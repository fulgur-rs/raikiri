//! raikiri-dom — DOM data model + layout engine (taffy + parley) + GCPM runtime side.
//!
//! Implements the node arena, taffy 6 trait implementations, and raikiri_traits::Dom co-design.
//! For details, see §4, raikiri-dom scope, in the original design.

// Public module rustdoc cross-links some crate-private helpers (e.g.
// `crate::layout::preshape_text`, `Document::flags_dirty`) which resolve fine
// under `--document-private-items` but trip the strict public build.  Preserve
// the cross-links; the linker's audience is intra-crate readers.
#![allow(rustdoc::private_intra_doc_links)]
//!
//! ## Module tour
//!
//! - [`document`] — `Document` arena + `append_element` / `append_text`
//! - `node`     — `Node` struct (crate-private, no rustdoc entry)
//! - [`taffy_impl`] — taffy 6 layout trait impls + `unsafe impl Send for Document`
//! - [`dom_impl`] — `raikiri_traits::{Dom, Node, Element}` +
//!   `raikiri_style::{StyleDom, StyleNode, StyleElement}` impls + `NodeRef` /
//!   `ElementRef` types (both trait families over one arena, see file header)
//! - [`fonts`] — constructs a cross-machine deterministic `FontContext` from the
//!   WPT bundled font directory (`build_wpt_font_ctx`)
//!
//! # Flat tree membership
//!
//! The [`NodeFlags::IS_IN_DOCUMENT`] bit on [`Node`] indicates reachability from
//! the Document root through flat-tree-parent edges. It is clear for these subtrees:
//!
//! - Descendants of a `<template>` element (the element itself has in_document=true)
//! - In the future: light-DOM descendants outside a shadow root, slotted-only
//!   descendants, and transient detached nodes during mutation
//!
//! Maintenance: the raikiri-html sink calls
//! [`Document::mark_in_document_flags`] in a single pass during `finish()`.
//! The current prototype is parse-only, so the flags stay fixed after finish.
//! When runtime mutation is introduced, add the equivalent of blitz
//! `process_added_subtree` / `process_removed_subtree`.
//!
//! To skip inert subtrees during traversal,
//! call [`Node::is_in_document`] on each iteration. Do not use string comparisons
//! (such as tag_name == "template") for separate checks: that makes the concept
//! implicit and risks omissions when shadow DOM is added.

mod diag;
mod fragment;
mod image_resolve;
mod node;
mod phase_b;
mod running;
mod target;

pub mod document;
pub mod dom_impl;
pub mod fonts;
pub mod layout;
pub mod taffy_impl;

pub use document::{Document, DomMutationError};
pub use dom_impl::{ChildIter, ElementRef, NodeRef, StyleChildIter};
pub use fonts::{
    FontError, FontFaceApplyReport, FontFaceLoader, FontWarn, apply_font_faces, build_wpt_font_ctx,
    build_wpt_font_ctx_with_observer, expand_font_face_aliases, register_font_face_sources,
};
pub use layout::{
    InitialPageContext, InitialPageContextError, InitialPageProbeResources, PageContentInsets,
    PageMargins, PageSlice, first_page_name, layout_page_fragments, layout_pages,
    layout_pages_with_page_geometry, layout_pages_with_page_geometry_and_resolver,
    layout_pages_with_page_geometry_and_resolver_and_base_url, layout_pages_with_page_steps,
    layout_pages_with_resolver, layout_pages_with_resolver_and_base_url, layout_single_page,
    layout_single_page_with_resolver, layout_single_page_with_resolver_and_base_url,
    page_content_insets, page_fragment_events_from_pages, page_fragment_geometry_table,
    page_fragments_from_slices, page_fragments_from_slices_with_page_geometry, page_margins,
    relayout_text_for_width, resolve_initial_page_context, resolve_page_fragment_geometry,
};
pub use node::{ElementData, Node, NodeData, NodeFlags, TextData};
pub use raikiri_traits::{
    NodeKind, PageFragment, PageFragmentEvent, PageFragmentGeometry, PageFragmentGeometryTable,
    PageFragmentInsets, PageFragmentItem, PageFragmentKind, PageFragmentLineRange,
    PageFragmentLink, PageFragmentLinkEvent, PageFragmentOrientation, PageFragmentPageGeometry,
    PageFragmentRect, QuirksMode,
};
pub use target::{CounterSnapshot, counter_snapshots};

#[cfg(feature = "logical-snapshot")]
pub mod snapshot;

#[cfg(test)]
mod tests;
