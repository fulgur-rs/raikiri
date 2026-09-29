//! Per-page, per-attribute drawable maps (an ECS-style struct of arrays).
//!
//! [`PageDrawables`] groups the per-attribute node maps held by
//! [`PageScene`](crate::PageScene). It separates all drawable state for one
//! page by attribute (block, paragraph, image, SVG, etc.) so forward iteration
//! and batch processing in raikiri's dogfooding and validation paths remain
//! cache- and SIMD-friendly. The Fulgur-facing output contract is defined
//! separately, primarily in `raikiri-dom`.
//!
//! # Purpose of TrackedMap
//!
//! [`TrackedMap`] wraps `BTreeMap<NodeId, V>` with an **append-only insertion
//! log**. The log serves two purposes:
//!
//! 1. Expose deterministic BTreeMap iteration order to consumers to preserve
//!    byte-identical output.
//! 2. Prepare a future raikiri conversion pass to revisit only NodeIds newly
//!    inserted within a subtree scope, by reading the log tail in
//!    O(inserted-since) time.
//!
//! The log-tail API (`mark` / `since`) is **not public** yet. It is unused
//! while PageScene is an immutable per-page snapshot; publication should be
//! reconsidered if internal conversion or reflow passes need it. This is a
//! dogfooding design, not a promise about the Fulgur-facing API.
//!
//! # Reference shape
//!
//! Fulgur drawables.rs (v0.12) provides the field reference. Of its 21
//! fields, [`PageDrawables`] adopts 13. The other eight fall into two groups:
//!
//! - **Three moved to [`PageScene`](crate::PageScene)**: body_offset_pt,
//!   root_id, and body_id are page-global state. Fulgur keeps them flat in
//!   Drawables; raikiri collects them at the page snapshot level.
//! - **Five unnecessary in raikiri**: paragraph_slices, root_dir_rtl,
//!   synthetic_id_counter, li_lbl_ids, and li_lbody_ids serve Fulgur's
//!   reflow machinery, PDF tag tree, or RTL page priming, none of which
//!   applies to raikiri's snapshot semantics.

use crate::entries::{
    BlockEntry, BookmarkAnchorEntry, ImageEntry, LinkSpanEntry, ListItemEntry, MulticolRuleEntry,
    ParagraphEntry, SemanticEntry, SvgEntry, TableEntry, TransformEntry,
};
use raikiri_traits::NodeId;
use std::collections::{BTreeMap, BTreeSet};

/// A thin wrapper around `BTreeMap<NodeId, V>` with an append-only
/// insertion log.
///
/// # Internal dogfooding contract
///
/// - [`Deref`](std::ops::Deref) exposes `&BTreeMap<NodeId, V>` with
///   deterministic ascending-key iteration.
/// - [`insert`](TrackedMap::insert) is the only insertion path and always
///   updates the log.
/// - [`get_mut`](TrackedMap::get_mut) changes the value of an existing entry
///   but does not append to the log because the set of keys is unchanged.
///
/// `DerefMut` is intentionally absent. Alternative mutation paths such as
/// `.entry()`, `.extend()`, and `.iter_mut()` must fail to compile rather
/// than bypass the insertion log.
///
/// # Fulgur shape reference
///
/// The field arrangement follows `TrackedMap<V>` in Fulgur
/// drawables.rs:65-117. Fulgur's O(N²) → O(inserted-since) rationale
/// depends on its reflow machinery and does not directly carry over to
/// raikiri's snapshot semantics. Raikiri promises only the two consumer
/// properties above.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct TrackedMap<V> {
    map: BTreeMap<NodeId, V>,
    /// Append-only insertion log. Re-inserting a key records it again.
    order: Vec<NodeId>,
}

impl<V> Default for TrackedMap<V> {
    fn default() -> Self {
        Self {
            map: BTreeMap::new(),
            order: Vec::new(),
        }
    }
}

impl<V> std::ops::Deref for TrackedMap<V> {
    type Target = BTreeMap<NodeId, V>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<V> TrackedMap<V> {
    /// Associate `value` with `key` and append `key` to the insertion log.
    /// Replacing an existing entry also appends: this is an event log,
    /// not a snapshot of the map's distinct keys.
    pub fn insert(&mut self, key: NodeId, value: V) -> Option<V> {
        self.order.push(key);
        self.map.insert(key, value)
    }

    /// Mutable reference to the value of an existing entry. No key is
    /// inserted or logged. Returns `None` when `key` is absent.
    pub fn get_mut(&mut self, key: &NodeId) -> Option<&mut V> {
        self.map.get_mut(key)
    }
}

/// Per-attribute drawable maps for one page, held in the `drawables`
/// field of [`PageScene`](crate::PageScene). Raikiri's dogfooding and
/// validation paths iterate by attribute to paint. Fulgur-facing page
/// output is defined separately, primarily in `raikiri-dom`.
///
/// # Field selection
///
/// Fulgur drawables.rs provides the reference per-attribute maps. These
/// 13 fields make sense for an immutable raikiri page snapshot. Fields
/// specific to Fulgur's reflow machinery, PDF tagging, and RTL page
/// priming (paragraph_slices, synthetic_id_counter, li_lbl_ids,
/// li_lbody_ids, root_dir_rtl, etc.) are not needed here.
///
/// # Landing scope
///
/// Initially, only the struct fields existed. Later, minimal fields were
/// added to each Entry type using the Fulgur reference (see the
/// [`crate::entries`] module docs). `build_page_scene` (crate::page_scene)
/// now constructs [`BlockEntry`] and [`ParagraphEntry`] and inserts them
/// into `block_styles` and `paragraphs`. The other nine fields remain empty
/// because the matching raikiri pipeline stages do not exist; see the
/// [`crate::entries`] module docs.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageDrawables {
    /// Per-node block-box state (background, border, shadow, opacity, etc.).
    /// Fulgur drawables.rs:142-177 provides the `BlockEntry` reference for
    /// future population.
    pub block_styles: TrackedMap<BlockEntry>,
    /// Per-node paragraph state (shaped inline lines), based on Fulgur
    /// drawables.rs:183-191's `ParagraphEntry` shape.
    pub paragraphs: TrackedMap<ParagraphEntry>,
    /// Per-node raster-image state, based on the `ImageEntry` shape in
    /// Fulgur drawables.rs:205-213.
    pub images: TrackedMap<ImageEntry>,
    /// Per-node SVG-image state, based on the `SvgEntry` shape in
    /// Fulgur drawables.rs:225-232.
    pub svgs: TrackedMap<SvgEntry>,
    /// Per-node table outer-frame state (background, border, shadow), based
    /// on the `TableEntry` shape in Fulgur drawables.rs:243-257.
    pub tables: TrackedMap<TableEntry>,
    /// Per-node list-item marker state (text, image, or none), based on
    /// Fulgur drawables.rs:271-299's `ListItemEntry` / `ListItemMarker` shape.
    pub list_items: TrackedMap<ListItemEntry>,
    /// Per-node CSS transform state (matrix and origin). Use a plain
    /// BTreeMap rather than TrackedMap, following Fulgur's `TransformEntry`
    /// (drawables.rs:392-405); subtree aggregation needs no insertion log.
    pub transforms: BTreeMap<NodeId, TransformEntry>,
    /// Multicolumn column-rule geometry, following `MulticolRuleEntry` and
    /// `ColumnRuleGeometry` in Fulgur drawables.rs:316-340.
    pub multicol_rules: BTreeMap<NodeId, MulticolRuleEntry>,
    /// PDF bookmark-tree source anchor, following `BookmarkAnchorEntry`
    /// in Fulgur drawables.rs:409-413.
    pub bookmark_anchors: BTreeMap<NodeId, BookmarkAnchorEntry>,
    /// Hyperlink target span over a paragraph glyph run. A node may have
    /// multiple spans, so store `Vec<(NodeId, LinkSpanEntry)>`, following
    /// Fulgur drawables.rs:418-419.
    pub link_spans: Vec<(NodeId, LinkSpanEntry)>,
    /// Per-node tagged-PDF state (tag, parent, alt_text, etc.), based on
    /// `SemanticEntry` in Fulgur tagging.rs:57.
    pub semantics: BTreeMap<NodeId, SemanticEntry>,
    /// NodeIds whose inline-box subtrees skip paint dispatch. This marks
    /// the scope when inline-block/inline-flex paint order is routed through
    /// the parent.
    pub inline_box_subtree_skip: BTreeSet<NodeId>,
    /// Descendant NodeIds belonging to an inline-box subtree. Lists the
    /// descendants within [`inline_box_subtree_skip`](PageDrawables::inline_box_subtree_skip)
    /// in deterministic order.
    pub inline_box_subtree_descendants: BTreeMap<NodeId, Vec<NodeId>>,
}

#[cfg(test)]
mod tests;
