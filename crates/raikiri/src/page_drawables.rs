//! Per-page per-attribute drawable maps (ECS 風 struct-of-arrays shape).
//!
//! [`PageDrawables`] は [`PageScene`](crate::PageScene) が保持する
//! per-attribute node map の集合。1 page 内の全 drawable state を
//! attribute 単位 (block / paragraph / image / svg / …) に分離して並べる
//! ことで、consumer の forward iterate と raikiri 内部の batch operation
//! の両方を cache-friendly / SIMD-friendly に保つ。
//!
//! # TrackedMap の役割
//!
//! [`TrackedMap`] は `BTreeMap<NodeId, V>` に **append-only insertion log** を
//! 添えた薄い wrapper。log を保持する主目的は 2 点:
//!
//! 1. Deterministic iteration semantics (BTreeMap ordering) を consumer に
//!    そのまま公開して byte-identical output を保証する
//! 2. 将来 raikiri 内部の convert pass が「特定 subtree の
//!    scope 内で新規挿入された NodeId のみ」を再訪する時、log の tail 参照
//!    で O(inserted-since) に置換可能な mechanism を先行 landing する
//!
//! Insertion log tail 読み出し API (`mark` / `since`) は現時点では
//! **pub にしない**。raikiri PageScene = per-page immutable snapshot
//! semantics では初期 unused、将来 raikiri 内部の convert /
//! reflow pass が必要とした時点で pub 化を再判断する。
//!
//! # 参考 shape
//!
//! Field 集合は fulgur drawables.rs (v0.12)
//! を reference shape として選定。fulgur 21 field のうち、[`PageDrawables`] は
//! 13 field を採用する。残り 8 field は 2 分類:
//!
//! - **3 field は [`PageScene`](crate::PageScene) に relocate**: body_offset_pt
//!   / root_id / body_id は per-page global state (fulgur は Drawables に flat
//!   保持、raikiri は snapshot scope で page-level state を PageScene に集約)
//! - **5 field は raikiri では不要**: paragraph_slices / root_dir_rtl /
//!   synthetic_id_counter / li_lbl_ids / li_lbody_ids は fulgur の reflow-
//!   machinery / PDF-tag-tree / RTL page priming 系、raikiri snapshot semantics
//!   には無関係

use crate::entries::{
    BlockEntry, BookmarkAnchorEntry, ImageEntry, LinkSpanEntry, ListItemEntry, MulticolRuleEntry,
    ParagraphEntry, SemanticEntry, SvgEntry, TableEntry, TransformEntry,
};
use raikiri_traits::NodeId;
use std::collections::{BTreeMap, BTreeSet};

/// `BTreeMap<NodeId, V>` + append-only insertion log の thin wrapper。
///
/// # Consumer contract
///
/// - [`Deref`](std::ops::Deref) 経由で `&BTreeMap<NodeId, V>` として読み出せる
///   (deterministic iteration order = key ascending)
/// - [`insert`](TrackedMap::insert) が唯一の追加 path、log を必ず更新する
/// - [`get_mut`](TrackedMap::get_mut) は既存 entry の value を書き換える
///   だけで log に append しない (key 集合を変えない mutation)
///
/// `DerefMut` は意図的に実装しない: `.entry()` / `.extend()` / `iter_mut()`
/// などの alternate mutation path が log を bypass するのを compile time で
/// fail させる。
///
/// # Fulgur shape reference
///
/// Field 構成は fulgur drawables.rs:65-117 の `TrackedMap<V>` の shape を
/// reference material として選定。fulgur が O(N²) → O(inserted-since) の
/// anti-quadratic 動機で書いた rationale は fulgur reflow-machinery
/// 依存の説明であり、raikiri PageScene = snapshot semantics には
/// 直接転写しない (raikiri は上記 consumer contract の 2 点のみを
/// promise する)。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct TrackedMap<V> {
    map: BTreeMap<NodeId, V>,
    /// Append-only insertion log。同じ key の re-insert は log 上重複を許す。
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
    /// `key` に `value` を関連付け、insertion log に `key` を append する。
    /// 既存 entry を上書きする場合も log は append される (log は insertion
    /// event の羅列であり map の key 集合 snapshot ではない)。
    pub fn insert(&mut self, key: NodeId, value: V) -> Option<V> {
        self.order.push(key);
        self.map.insert(key, value)
    }

    /// 既存 entry の value への `&mut` 参照。key を追加しないため log は
    /// 更新しない。`key` が map に存在しない場合は `None`。
    pub fn get_mut(&mut self, key: &NodeId) -> Option<&mut V> {
        self.map.get_mut(key)
    }
}

/// 1 page 分の per-attribute drawable map。[`PageScene`](crate::PageScene)
/// の drawables field として保持され、consumer が attribute 単位で
/// forward iterate して paint する。
///
/// # Field 選定基準
///
/// Fulgur drawables.rs の per-attribute map を
/// reference shape として、raikiri PageScene = immutable per-page snapshot
/// semantics で意味を持つ 13 field を選定。
/// fulgur-specific な reflow-machinery / PDF-tag-tree / RTL page priming 系
/// field (paragraph_slices / synthetic_id_counter / li_lbl_ids / li_lbody_ids
/// / root_dir_rtl 等) は raikiri では不要のため採用しない。
///
/// # landing scope
///
/// 最初は struct field surface のみ landing、その後
/// 各 Entry 型に fulgur reference と照合した minimal
/// field を追加し (`crate::entries` module doc参照)、
/// `build_page_scene` (crate::page_scene) が実際に
/// [`BlockEntry`] / [`ParagraphEntry`] を construct して `block_styles` /
/// `paragraphs` へ insert するようになった。他 9 field はまだ常に空
/// (対応する raikiri pipeline stage が無いため、`crate::entries` module doc
/// 参照)。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageDrawables {
    /// Block box (background / border / shadow / opacity 等) の per-node
    /// state。Fulgur drawables.rs:142-177 の `BlockEntry` shape が
    /// future populate 時の reference。
    pub block_styles: TrackedMap<BlockEntry>,
    /// Paragraph (shaped inline text lines) の per-node state。Fulgur
    /// drawables.rs:183-191 の `ParagraphEntry` shape が reference。
    pub paragraphs: TrackedMap<ParagraphEntry>,
    /// Raster image の per-node state。Fulgur drawables.rs:205-213 の
    /// `ImageEntry` shape が reference。
    pub images: TrackedMap<ImageEntry>,
    /// SVG image の per-node state。Fulgur drawables.rs:225-232 の
    /// `SvgEntry` shape が reference。
    pub svgs: TrackedMap<SvgEntry>,
    /// Table (outer frame の background / border / shadow) の per-node
    /// state。Fulgur drawables.rs:243-257 の `TableEntry` shape が reference。
    pub tables: TrackedMap<TableEntry>,
    /// List item marker (text / image / none) の per-node state。Fulgur
    /// drawables.rs:271-299 の `ListItemEntry` / `ListItemMarker` shape が
    /// reference。
    pub list_items: TrackedMap<ListItemEntry>,
    /// CSS transform (matrix + origin) の per-node state。TrackedMap 化
    /// せず単純な BTreeMap で保持 (fulgur drawables.rs:392-405 の
    /// `TransformEntry` に倣う、subtree scope 集計は insertion log 不要)。
    pub transforms: BTreeMap<NodeId, TransformEntry>,
    /// Multicol の column-rule 描画 geometry。Fulgur drawables.rs:316-340
    /// の `MulticolRuleEntry` + `ColumnRuleGeometry` shape が reference。
    pub multicol_rules: BTreeMap<NodeId, MulticolRuleEntry>,
    /// Bookmark anchor (PDF bookmark tree の source)。Fulgur
    /// drawables.rs:409-413 の `BookmarkAnchorEntry` shape が reference。
    pub bookmark_anchors: BTreeMap<NodeId, BookmarkAnchorEntry>,
    /// Link span (paragraph 内 glyph run 上に張る hyperlink target)。
    /// 1 node が複数 span を carry するため `Vec<(NodeId, LinkSpanEntry)>`
    /// で保持する (fulgur drawables.rs:418-419 の shape に倣う)。
    pub link_spans: Vec<(NodeId, LinkSpanEntry)>,
    /// Tagged-PDF semantics (tag / parent / alt_text 等) の per-node state。
    /// Fulgur tagging.rs:57 の `SemanticEntry` shape が reference。
    pub semantics: BTreeMap<NodeId, SemanticEntry>,
    /// Inline-box subtree の paint dispatch を skip する NodeId 集合。
    /// Inline block / inline-flex 等の paint 順序を parent 経由に統合する
    /// 時の scope marker。
    pub inline_box_subtree_skip: BTreeSet<NodeId>,
    /// Inline-box subtree に属する descendant NodeId の一覧。
    /// [`inline_box_subtree_skip`](PageDrawables::inline_box_subtree_skip) の
    /// scope 内 descendants を deterministic 順序で並べる。
    pub inline_box_subtree_descendants: BTreeMap<NodeId, Vec<NodeId>>,
}
