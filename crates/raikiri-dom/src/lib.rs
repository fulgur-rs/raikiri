//! raikiri-dom — DOM data model + layout engine (taffy + parley) + GCPM runtime side.
//!
//! node arena + taffy 6 trait impl + raikiri_traits::Dom co-design を実装。
//! 詳細は design 由来の §4 raikiri-dom scope 参照。

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
//! - [`fonts`] — WPT bundled font dir から cross-machine 決定性 `FontContext`
//!   を構築する (`build_wpt_font_ctx`)
//!
//! # Flat tree membership
//!
//! [`Node`] は [`NodeFlags::IS_IN_DOCUMENT`] bit で「Document root から
//! flat-tree-parent 経由で到達可能」を表す。以下の subtree は clear される:
//!
//! - `<template>` element の子孫 (element 自身は in_document=true)
//! - 将来: shadow root 外の light-DOM 子孫、slotted-only 子孫、mutator の
//!   transient な detached node
//!
//! 維持: raikiri-html sink `finish()` が
//! [`Document::mark_in_document_flags`] を single pass で呼ぶ。現状の prototype
//! 実装は parse-only なので finish 後は固定。将来 runtime mutation を導入する
//! 時に blitz `process_added_subtree` / `process_removed_subtree` 相当を
//! 追加する予定。
//!
//! Traversal が inert subtree を skip したい場合、
//! [`Node::is_in_document`] を各 iteration で呼ぶ。string 比較 (tag_name ==
//! "template" 等) で個別判定するのは禁止 — 概念が implicit になり、shadow DOM
//! 追加時に漏れる。

mod diag;
mod fragment;
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
pub mod taffy_impl;

pub use document::Document;
pub use dom_impl::{ChildIter, ElementRef, NodeRef, StyleChildIter};
pub use fonts::{
    FontError, FontFaceApplyReport, FontFaceLoader, FontWarn, apply_font_faces, build_wpt_font_ctx,
    build_wpt_font_ctx_with_observer, expand_font_face_aliases, register_font_face_sources,
};
pub use layout::{
    InitialPageContext, InitialPageContextError, InitialPageProbeResources, PageContentInsets,
    PageMargins, PageSlice, first_page_name, layout_pages, layout_pages_with_page_geometry,
    layout_pages_with_page_geometry_and_resolver,
    layout_pages_with_page_geometry_and_resolver_and_base_url, layout_pages_with_page_steps,
    layout_pages_with_resolver, layout_pages_with_resolver_and_base_url, layout_single_page,
    layout_single_page_with_resolver, layout_single_page_with_resolver_and_base_url,
    page_content_insets, page_margins, relayout_text_for_width, resolve_initial_page_context,
};
pub use node::{ElementData, Node, NodeData, NodeFlags, TextData};
pub use page_projection::fragment::{Fragment, FragmentKind, RepeatKind};
pub use raikiri_traits::NodeKind;
pub use target::{CounterSnapshot, counter_snapshots};

#[cfg(test)]
mod tests;
