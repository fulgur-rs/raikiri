//! raikiri-dom — DOM data model + layout engine (taffy + the shodo inline engine) + GCPM runtime side.
//!
//! Implements the node arena, taffy 6 trait implementations, and raikiri_traits::Dom co-design.
//! For details, see §4, raikiri-dom scope, in the original design.

// Public module rustdoc cross-links some crate-private helpers (e.g.
// `Document::flags_dirty`) which resolve fine
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
//! - [`fonts`] — font collections for the inline engine: a cross-machine
//!   deterministic one from the WPT bundled font directory
//!   (`build_wpt_font_collection`), bundled bytes, the installed fonts, and
//!   the `@font-face` document layer
//!
//! # Flat tree membership
//!
//! The [`NodeFlags::IS_IN_DOCUMENT`] bit on [`Node`] indicates reachability from
//! the Document root through flat-tree-parent edges. It is clear for these subtrees:
//!
//! - Detached subtrees unreachable from the Document root, including a
//!   `<template>` element's contents fragment (the element itself has
//!   in_document=true, as do its ordinary light-DOM children — only the
//!   associated contents fragment is inert)
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
pub mod generated_content;
pub mod image_geometry;
mod image_resolve;
mod node;
mod page_projection;
mod phase_b;
mod running;
mod target;

pub mod document;
pub mod dom_impl;
pub mod fonts;
pub mod layout;
#[doc(hidden)]
pub mod paint_rules;
#[doc(hidden)]
pub mod text_decoration;
pub use text_decoration::{DecorationKind, DecorationLine, DecorationStyle};
pub mod taffy_impl;

pub use document::{Document, DomMutationError};
pub use dom_impl::{ChildIter, ElementRef, NodeRef, StyleChildIter};
pub use fonts::{
    BundledFace, FontError, FontFaceApplyReport, FontFaceLoader, FontWarn,
    build_bundled_font_collection, build_inline_document_fonts, build_wpt_font_collection,
    system_font_collection,
};
#[doc(hidden)]
pub use fragment::{FragmentRect, LayoutFragment};
#[doc(hidden)]
// cov:ignore: this attribute controls rustdoc metadata and has no runtime coverage.
pub use layout::{
    BoxRect, IfcBuildMode, IfcTextLine, IfcTextLines, InlineBoxPiece, LineGlyph, PositionedLine,
    PositionedLines, PositionedMarker, PositionedRun, cumulative_offset, relative_offset,
};
// cov:ignore: public re-exports have no runtime behavior to measure.
pub use layout::{
    InitialPageContext, InitialPageContextError, InitialPageProbeResources, MAX_LAYOUT_DEPTH,
    PageContentInsets, PageLayoutControl, PageMargins, PageSlice, first_page_name, layout_pages,
    layout_pages_with_page_geometry, layout_pages_with_page_geometry_and_control,
    layout_pages_with_page_geometry_and_resolver,
    layout_pages_with_page_geometry_and_resolver_and_base_url,
    layout_pages_with_page_geometry_and_resolver_and_base_url_and_control,
    layout_pages_with_page_steps, layout_pages_with_resolver,
    layout_pages_with_resolver_and_base_url, layout_pages_with_resolver_and_base_url_and_control,
    layout_single_page, layout_single_page_with_resolver,
    layout_single_page_with_resolver_and_base_url, page_content_insets,
    page_content_insets_for_page, page_margins, page_margins_for_page, relayout_text_for_width,
    resolve_initial_page_context, validate_layout_depth,
};
#[doc(hidden)]
pub use layout::{StandaloneAlign, StandaloneStyle, StandaloneText};
pub use node::{CanvasBitmap, CanvasBitmapError, ElementData, Node, NodeData, NodeFlags, TextData};
pub use page_projection::fragment::{Fragment, FragmentKind, OverflowClip, RepeatKind};
pub use page_projection::paint_order::{ClipKind, PaintEvent};
// cov:ignore: public type re-exports have no executable mapping; API integration tests verify them.
pub use page_projection::text_runs::{
    FontBlob, FontId, FontRef, FontVariation, GeneratedKind, Glyph, PositionedGlyphRun, RunSource,
    Synthesis, Tag, TextLineId,
};
pub use raikiri_traits::{NodeKind, QuirksMode};
// cov:ignore: public re-exports have no executable coverage mapping.
pub use target::{
    CounterSnapshot, CounterSnapshotBudget, CounterSnapshotLimitExceeded,
    MAX_COUNTER_SNAPSHOT_ESTIMATED_BYTES, counter_snapshots, counter_snapshots_with_budget,
};

#[cfg(feature = "logical-snapshot")]
pub mod snapshot;

#[cfg(test)]
mod tests;
