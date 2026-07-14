//! Taffy layout trait implementations on Document.
//!
//! spike (`crates/raikiri-feasibility/src/taffy_layout_modes.rs`) の SpikeTree
//! pattern を production 化したもの。実装内容は spike と等価:
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
pub struct ChildIter<'a>(core::slice::Iter<'a, usize>);

impl Iterator for ChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(NodeId::from)
    }
}

impl TraversePartialTree for Document {
    type ChildIter<'a> = ChildIter<'a>;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        ChildIter(self.nodes[usize::from(node_id)].children.iter())
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
        compute_cached_layout(self, node_id, inputs, |tree, node_id, inputs| {
            let idx = usize::from(node_id);
            let is_leaf = tree.nodes[idx].children.is_empty();
            let display = tree.nodes[idx].style.display;
            if is_leaf {
                let style = tree.nodes[idx].style.clone();
                compute_leaf_layout(
                    inputs,
                    &style,
                    |_val, _basis| 0.0,
                    |_known, _avail| Size::ZERO,
                )
            } else {
                match display {
                    Display::Block => compute_block_layout(tree, node_id, inputs, None),
                    Display::Flex => compute_flexbox_layout(tree, node_id, inputs),
                    Display::Grid => compute_grid_layout(tree, node_id, inputs),
                    Display::None => LayoutOutput::HIDDEN,
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
// M1.5 は feasibility spike (nzv.6) と同じ approach 1 を採用: Document 内に
// 格納される全 calc pointer は「同じ Document 内 (self-contained arena)」を
// 指す invariant を維持する限り、Document 全体を別 thread へ move しても
// pointer validity は破れない。
//
// M1.5 現段階では calc pointer を populate する経路が存在しない (Node.style
// は Consumer が taffy::Style を直接構築、M1.6 で ComputedValues 変換時も
// `length(px)` / `percent` / `auto` のみ使用予定)。calc pointer が入る余地が
// 生まれるのは M4 sandboxed resolver の CSS calc() 完全 support 段階。
//
// 最終 invariant の確定は m1.17 taffy-layoutbuffer-send-decision に委ねる:
//   - approach 1 (self-contained arena、この unsafe impl のまま)
//   - approach 2 (SendableStyle newtype で pointer を隠蔽)
//   - approach 3 ("no calc across thread boundary" construction guard)
// のいずれかに再整理される。
#[allow(unsafe_code)]
unsafe impl Send for Document {}
