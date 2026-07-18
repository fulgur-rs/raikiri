//! Taffy layout trait implementations on Document.
//!
//! M0 spike (nzv.6 `taffy-layout-modes`) の SpikeTree pattern を production 化
//! したもの。実装内容は spike と等価:
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

/// Child iterator for taffy traits.
pub struct TaffyChildIter<'a>(core::slice::Iter<'a, usize>);

impl Iterator for TaffyChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(NodeId::from)
    }
}

impl TraversePartialTree for Document {
    type ChildIter<'a> = TaffyChildIter<'a>;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        TaffyChildIter(self.nodes[usize::from(node_id)].children.iter())
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        self.nodes[usize::from(node_id)].children.len()
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        NodeId::from(self.nodes[usize::from(node_id)].children[index])
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

    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        self.nodes[usize::from(node_id)].unrounded_layout = *layout;
    }

    fn resolve_calc_value(&self, _val: *const (), _basis: f32) -> f32 {
        // M1.5: calc pointer は populate されないため 0.0 を返す (spike と同じ)。
        // M4 で CSS calc() を実装する際に resolver をここに wire する予定。
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
// **Invariant (m1.17 決定、approach A: self-contained arena)**:
// raikiri-dom 内で `Style` を保持する任意の型 (Document、M2 以降 LayoutBuffer
// 等) に格納される全 `CompactLength::calc(ptr)` の `ptr` は、同じ Document
// が own する calc arena (M4 で raikiri-dom 内に追加予定) を指す。この
// invariant が守られる限り、Document 全体を別 thread へ move しても pointer
// target が follow するため validity は保たれる。
//
// **Sync は付けない**: raikiri の parallel layout 経路 (M4 の 16 margin box
// slot + column-count) は `Arc<GcpmSnapshot>` (owned deep copy、design doc
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
// **State (M1.5)**: calc pointer を populate する path は不在 (Node.style は
// `length(px)` / `percent` / `auto` のみ)。M4 sandboxed resolver で CSS calc()
// を実装する際、calc arena を raikiri-dom 側に配置し、`CompactLength::calc(...)`
// の唯一の callsite が arena allocation と同一 site に閉じるよう API を絞る
// (structural enforcement)。M4 前に混入を防ぐ custom lint
// (`raikiri-lints::no_calc_construction`、design §5.4.1) を M2〜M3 で raikiri-dom
// crate に導入する。
#[allow(unsafe_code)]
unsafe impl Send for Document {}
