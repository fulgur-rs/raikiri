//! raikiri-dom — DOM data model + layout engine (taffy + the shodo inline engine) + GCPM runtime side.
//!
//! Implements the node arena, the layout dispatch over taffy, and raikiri_traits::Dom co-design.
//! For details, see §4, raikiri-dom scope, in the original design.

#![forbid(unsafe_code)]
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
//! - [`dom_impl`] — `raikiri_traits::{Dom, Node, Element}` +
//!   `raikiri_style::{StyleDom, StyleNode, StyleElement}` impls + `NodeRef` /
//!   `ElementRef` types (both trait families over one arena, see file header)
//! - `fonts` — font collections for the inline engine: a cross-machine
//!   deterministic one from the WPT bundled font directory
//!   (`build_wpt_font_collection`), bundled bytes, the installed fonts, and
//!   the `@font-face` document layer
//!
//! # Flat tree membership
//!
//! The `NodeFlags::IS_IN_DOCUMENT` bit on [`Node`] indicates reachability from
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
#[doc(hidden)]
pub mod generated_content;
pub mod image_geometry;
mod image_resolve;
mod image_url;
#[doc(hidden)]
pub mod inline_svg_cache;
#[doc(hidden)]
pub mod inline_svg_style;
mod margin_boxes;
mod node;
mod normal_line_height;
mod page_projection;
mod running;
// cov:ignore: public re-exports have no runtime behavior to measure.
pub use running::{
    ResolvedPageGeometry, RunningElementPage, RunningIndex, element_string_value,
    layout_running_element, resolve_page_geometry, used_running_element_width,
};
mod target;

mod column_rules;
pub mod document;
pub mod dom_impl;
#[doc(hidden)]
pub mod fonts;
pub mod layout;
#[doc(hidden)]
pub mod paint_rules;
#[doc(hidden)]
pub mod text_decoration;
pub use text_decoration::{DecorationKind, DecorationLine, DecorationStyle};

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
    AnnotationRun, BoxRect, GeneratedBlockPiece, IfcBuildMode, IfcTextLine, IfcTextLines,
    InlineBoxPiece, LineGlyph, PositionedContentImage, PositionedLine, PositionedLines,
    PositionedMarker, PositionedRun, cumulative_offset, generates_inline_box, relative_offset,
    used_relative_offset,
};
// cov:ignore: public re-exports have no runtime behavior to measure.
pub use column_rules::ColumnRule;
// cov:ignore: public re-exports have no runtime behavior to measure.
pub use layout::{
    InitialPageContext, InitialPageContextError, InitialPageProbeResources, MAX_LAYOUT_DEPTH,
    PageContentInsets, PageImageOptions, PageLayoutControl, PageLayoutOptions, PageMargins,
    PageSlice, first_page_name, layout_pages, layout_pages_with_options, layout_single_page,
    layout_single_page_with_resolver, layout_single_page_with_resolver_and_base_url,
    page_content_insets, page_content_insets_for_page, page_margins, page_margins_for_page,
    relayout_text_for_width, resolve_initial_page_context, validate_layout_depth,
};
// cov:ignore: public re-exports have no runtime behavior to measure.
#[doc(hidden)]
pub use layout::pages_follow_x_axis;
#[doc(hidden)]
pub use layout::placed_without_relative_offset;
#[doc(hidden)]
pub use layout::{CanvasBoxUse, canvas_background_owner, canvas_box_use};
pub use layout::{RootBox, root_box};
#[doc(hidden)]
pub use layout::{StandaloneAlign, StandaloneImage, StandaloneStyle, StandaloneText};
#[doc(hidden)]
pub use layout::{establishes_fixed_containing_block, has_applied_transform, is_transformable};
pub use margin_boxes::MarginBoxContentImage;
pub use margin_boxes::{
    DeferredGlyph, DeferredSlot, DeferredValue, MarginBox, MarginBoxBackgroundImage,
    MarginBoxBorder, MarginBoxNamedValues, MarginBoxPageContext, MarginBoxRunning, MarginBoxText,
    page_margin_boxes, page_margin_boxes_with_values,
};
pub use margin_boxes::{margin_box_content_image_urls, page_margin_box_content_image_urls};
pub use node::{CanvasBitmap, CanvasBitmapError, ElementData, Node, NodeData, NodeFlags, TextData};
pub use page_projection::continuation::{CrossingBox, PageStart, PageStartToken};
pub use page_projection::fragment::{Fragment, FragmentKind, OverflowClip, RepeatKind};
pub use page_projection::generated_boxes::{GeneratedBox, LineBox};
pub use page_projection::generated_images::GeneratedImageId;
pub use page_projection::paint_order::{ClipKind, PaintEvent};
// cov:ignore: public type re-exports have no executable mapping; API integration tests verify them.
pub use page_projection::text_runs::{
    FontBlob, FontId, FontRef, FontVariation, GeneratedKind, Glyph, PositionedGlyphRun, RunSource,
    Synthesis, Tag, TextLineId, TextShadow,
};
pub use raikiri_traits::{NodeKind, QuirksMode};
// cov:ignore: public re-exports have no executable coverage mapping.
pub use target::{
    CompactCounterSnapshots, CounterSnapshot, CounterSnapshotBudget, CounterSnapshotLimitExceeded,
    CounterSnapshots, MAX_COUNTER_SNAPSHOT_ESTIMATED_BYTES, compact_counter_snapshots,
    compact_counter_snapshots_with_budget, counter_snapshots, counter_snapshots_with_budget,
};

#[cfg(feature = "logical-snapshot")]
pub mod snapshot;

#[cfg(test)]
mod tests;

mod scene_geometry;
#[doc(hidden)]
pub use scene_geometry::{SceneGeometry, SceneGeometryNode};

mod continuation_diagnostics;
#[cfg(any(test, feature = "table-reservation-diagnostics"))]
#[doc(hidden)]
pub mod table_reservation_diagnostics; // cov:ignore: module declaration has no executable coverage mapping.
#[cfg(feature = "continuation-diagnostics")]
#[doc(hidden)]
// cov:ignore: public re-exports have no executable coverage mapping.
pub use continuation_diagnostics::{
    ContinuationWork, begin_continuation_diagnostics, finish_continuation_diagnostics,
};

mod running_index_diagnostics;
#[cfg(feature = "running-index-diagnostics")]
#[doc(hidden)]
// cov:ignore: public re-exports have no executable coverage mapping.
pub use running_index_diagnostics::{
    RunningIndexWork, begin_running_index_diagnostics, finish_running_index_diagnostics,
};
