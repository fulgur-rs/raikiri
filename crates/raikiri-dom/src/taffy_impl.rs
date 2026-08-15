//! Taffy layout trait implementations on Document.
//!
//! 初期スパイク実装 (`taffy-layout-modes`) の SpikeTree pattern を production
//! 化したもの。実装内容は spike と等価:
//! - `TraversePartialTree`: children iterator
//! - `CacheTree`: per-node cache getter / setter
//! - `LayoutPartialTree`: display に応じて block / flexbox / grid をdispatch
//! - `LayoutBlockContainer / LayoutFlexboxContainer / LayoutGridContainer`:
//!   style getter marker impls

use taffy::{
    CacheTree, Display, Layout, LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutInput, LayoutOutput, LayoutPartialTree, NodeId, Size, Style, TraversePartialTree,
    TraverseTree, compute_block_layout, compute_cached_layout, compute_flexbox_layout,
    compute_grid_layout, compute_leaf_layout,
};

use crate::document::Document;

/// Taffy child iterator。raw arena children から `is_in_document() == false`
/// (`<template>` descendants など) を filter する。
///
/// Taffy の layout tree = web spec の "flat tree" なので、layout traversal
/// では template contents を "存在しない" ものとして扱う必要がある (paint 側で
/// skip しても layout 側で size / position が計算されると sibling の位置に
/// 影響してしまう)。`raikiri_traits::Dom::child_ids` は raw children を返す
/// 契約なので、そちらは変更せず、taffy 経路でのみ filter する。
pub struct TaffyChildIter<'a> {
    doc: &'a Document,
    inner: core::slice::Iter<'a, usize>,
}

impl Iterator for TaffyChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        for &c in self.inner.by_ref() {
            if self.doc.nodes[c].is_in_document() {
                return Some(NodeId::from(c));
            }
        }
        None
    }
}

impl TraversePartialTree for Document {
    type ChildIter<'a> = TaffyChildIter<'a>;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        TaffyChildIter {
            doc: self,
            inner: self.nodes[usize::from(node_id)].children.iter(),
        }
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        // Filter に一致する必要あり (is_in_document children のみ数える)。
        self.nodes[usize::from(node_id)]
            .children
            .iter()
            .filter(|&&c| self.nodes[c].is_in_document())
            .count()
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        // Filtered index — child_ids iterator と同じ view で n 番目を返す。
        let idx = self.nodes[usize::from(node_id)]
            .children
            .iter()
            .copied()
            .filter(|&c| self.nodes[c].is_in_document())
            .nth(index)
            .expect("get_child_id: index out of range");
        NodeId::from(idx)
    }
}

impl TraverseTree for Document {}

impl CacheTree for Document {
    fn cache_get(&self, node_id: NodeId, inputs: &LayoutInput) -> Option<LayoutOutput> {
        self.nodes[usize::from(node_id)].cache.get(inputs)
    }

    fn cache_store(&mut self, node_id: NodeId, inputs: &LayoutInput, layout_output: LayoutOutput) {
        self.nodes[usize::from(node_id)]
            .cache
            .store(inputs, layout_output)
    }

    fn cache_clear(&mut self, node_id: NodeId) {
        self.nodes[usize::from(node_id)].cache.clear();
    }
}

impl LayoutPartialTree for Document {
    type CustomIdent = String;
    type CoreContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_core_container_style(&self, node_id: NodeId) -> Self::CoreContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    /// taffy が arena へ layout を書き戻す**唯一の**経路。
    ///
    /// ここで [`crate::layout::sanitize_taffy_layout`] を通すことで
    /// 「`Node.unrounded_layout` は決して非有限 f32 を含まない」を構造的に
    /// 保証する。bridge 側の入力 guard
    /// (`layout::sanitize_taffy`) だけでは nested percentage が used value 層で
    /// 複利して非有限に戻るため閉じない — 理由と実測は
    /// `layout::sanitize_taffy_layout` の doc を参照。
    ///
    /// clamp が実際に発火した field は `self.layout_warnings` (owned buffer)
    /// に積む。この trait method の signature は
    /// `taffy` crate が固定しているため観測用の引数を追加できない —
    /// `self` 経由で書ける owned buffer に積むことで signature を変えずに
    /// 診断を残す。`layout_single_page` がパス終了時にこの buffer を drain
    /// して observer-or-eprintln へ流す (詳細は
    /// [`Document::layout_warnings`](crate::document::Document) の doc)。
    ///
    /// RHS を先に `sanitized` へ束縛してから LHS へ代入する — `self.nodes[..]`
    /// (IndexMut 経由) と `self.layout_warnings` という `self` の 2 つの
    /// disjoint field を 1 文の代入式内で同時に borrow させないための意図的な
    /// 分割 (borrowck を通すためだけでなく、evaluation order を明示して
    /// 読み手に依存関係を隠さない狙いもある)。
    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        let sanitized = crate::layout::sanitize_taffy_layout(layout, &mut self.layout_warnings);
        self.nodes[usize::from(node_id)].unrounded_layout = sanitized;
    }

    fn resolve_calc_value(&self, _val: *const (), _basis: f32) -> f32 {
        // calc pointer は現状 populate されないため 0.0 を返す (spike と同じ)。
        // 将来 CSS calc() を実装する際に resolver をここに wire する予定。
        0.0
    }

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        // Lazy layout cache invalidation — mutations only set a flag; we
        // clear all node caches on the first compute after the flag flips
        // (amortized O(1) per mutation over N-node batches).
        if self.layout_dirty {
            for node in &mut self.nodes {
                node.cache.clear();
            }
            self.layout_dirty = false;
        }
        compute_cached_layout(self, node_id, inputs, |tree, node_id, inputs| {
            let idx = usize::from(node_id);
            let display = tree.nodes[idx].style.display;
            if display == Display::None {
                return LayoutOutput::HIDDEN;
            }
            let is_leaf = tree.nodes[idx].children.is_empty();
            if is_leaf {
                let style = tree.nodes[idx].style.clone();
                let text_intrinsic: Option<Size<f32>> =
                    tree.nodes[idx].text_layout().map(|l| Size {
                        width: l.width(),
                        height: l.height(),
                    });
                compute_leaf_layout(
                    inputs,
                    &style,
                    |_val, _basis| 0.0,
                    |known, _avail| {
                        // taffy が style から算出した known.width / .height が Some なら
                        // それを優先 (explicit size)、None なら parley intrinsic を使う、
                        // 両方無ければ 0。
                        Size {
                            width: known
                                .width
                                .or(text_intrinsic.map(|s| s.width))
                                .unwrap_or(0.0),
                            height: known
                                .height
                                .or(text_intrinsic.map(|s| s.height))
                                .unwrap_or(0.0),
                        }
                    },
                )
            } else {
                match display {
                    Display::Block => compute_block_layout(tree, node_id, inputs, None),
                    Display::Flex => compute_flexbox_layout(tree, node_id, inputs),
                    Display::Grid => compute_grid_layout(tree, node_id, inputs),
                    Display::None => unreachable!("Display::None handled above"),
                }
            }
        })
    }
}

impl LayoutBlockContainer for Document {
    type BlockContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: NodeId) -> Self::BlockContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    fn get_block_child_style(&self, child_node_id: NodeId) -> Self::BlockItemStyle<'_> {
        &self.nodes[usize::from(child_node_id)].style
    }
}

impl LayoutFlexboxContainer for Document {
    type FlexboxContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node_id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    fn get_flexbox_child_style(&self, child_node_id: NodeId) -> Self::FlexboxItemStyle<'_> {
        &self.nodes[usize::from(child_node_id)].style
    }
}

impl LayoutGridContainer for Document {
    type GridContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: NodeId) -> Self::GridContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    fn get_grid_child_style(&self, child_node_id: NodeId) -> Self::GridItemStyle<'_> {
        &self.nodes[usize::from(child_node_id)].style
    }
}

// SAFETY: taffy の `calc` feature が enable の場合、`Style::Dimension` は
// `CompactLength` 経由で `*const ()` (caller-owned calc expression arena
// pointer) を保持する。Raw pointer は !Send のため `Style: !Send`、そこから
// `Document: !Send` が導出される。
//
// **Invariant (self-contained arena approach)**:
// raikiri-dom 内で `Style` を保持する任意の型 (Document、将来追加予定の
// LayoutBuffer 等) に格納される全 `CompactLength::calc(ptr)` の `ptr` は、
// 同じ Document が own する calc arena (将来 raikiri-dom 内に追加予定) を
// 指す。この invariant が守られる限り、Document 全体を別 thread へ move
// しても pointer target が follow するため validity は保たれる。
//
// **Sync は付けない**: raikiri の (将来実装予定の) parallel layout 経路
// (16 margin box slot + column-count 並列化) は `Arc<GcpmSnapshot>`
// (owned deep copy、design doc
// §5.4.1) 又は `&Style` の read-only borrow 経由で動作。`&Document` を
// 複数 thread から同時 read する path は無いため Sync は不要。blitz-dom は
// stylo parallel style traversal のため `unsafe impl Sync for Node` を追加
// しているが、raikiri は stylo 非依存で該当 path なし。
//
// **Precedent**: blitz-dom `Node` にも同種の `unsafe impl Send` があり
// (`blitz-dom-0.3.0-beta.1/src/node/node.rs:136`、無注釈)、taffy + calc
// feature 上で確立された pattern。ただし blitz は stylo `Arc<ComputedValues>`
// chain が calc data を own する外部 arena モデル、raikiri は self-contained
// arena モデルで invariant の依存対象が異なる。
//
// **Current state**: calc pointer を populate する path は不在 (Node.style は
// `length(px)` / `percent` / `auto` のみ)。将来 sandboxed resolver で CSS
// calc() を実装する際、calc arena を raikiri-dom 側に配置し、
// `CompactLength::calc(...)` の唯一の callsite が arena allocation と同一
// site に閉じるよう API を絞る (structural enforcement)。calc() 実装前に
// 混入を防ぐ custom lint (`raikiri-lints::no_calc_construction`、design
// §5.4.1) を raikiri-dom crate に導入する予定。
#[allow(unsafe_code)]
unsafe impl Send for Document {}
