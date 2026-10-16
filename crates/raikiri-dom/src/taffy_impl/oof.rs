//! Public Taffy positioning at the immediate-container geometry boundary.
//!
//! The document's CSS containing-block pass owns viewport and ancestor
//! positioning. Resolve immediate-container candidates here so all existing
//! consumers keep locations relative to their layout parent.

use crate::Document;
use taffy::{
    ContainingBlockClaims, DetailedLayoutInfo, LayoutContainingBlock, LayoutOutput,
    LayoutPartialTree, NodeId, OofCandidate, OofPositioningArea, Style,
};

impl LayoutContainingBlock for Document {
    type OofItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_oof_item_style(&self, node: NodeId) -> Self::OofItemStyle<'_> {
        let index = usize::from(node);
        self.transposed_child_style(index)
            .unwrap_or(&self.nodes[index].style)
    }

    fn clear_hoisted_children(&mut self, node: NodeId) {
        self.taffy_hoisted_children.remove(&usize::from(node));
    }

    fn add_hoisted_children(&mut self, node: NodeId, children: &[NodeId]) {
        if !children.is_empty() {
            self.taffy_hoisted_children
                .entry(usize::from(node))
                .or_default()
                .extend_from_slice(children);
        }
    }

    fn get_detailed_layout_info(&self, node: NodeId) -> &DetailedLayoutInfo<String> {
        self.grid_layout_info
            .get(&usize::from(node))
            .unwrap_or(&DetailedLayoutInfo::None)
    }
}

pub(super) fn layout_local_oof(tree: &mut Document, node: NodeId, output: &mut LayoutOutput) {
    let Some(area) = output.oof_positioning_area else {
        return;
    };
    let candidates = output.oof_candidates.take();
    let direction = tree.get_core_container_style(node).direction;
    let overflow = layout_candidates(tree, node, candidates.as_slice(), area, direction);
    output.scrollable_overflow_rect = output.scrollable_overflow_rect.union(overflow);
}

pub(super) fn layout_candidates(
    tree: &mut Document,
    owner: NodeId,
    candidates: &[OofCandidate],
    area: OofPositioningArea,
    direction: taffy::Direction,
) -> taffy::Rect<f32> {
    let result = taffy::compute::compute_oof_layout_for_area(
        tree,
        owner,
        candidates,
        area,
        direction,
        ContainingBlockClaims::ALL,
    );
    tree.clear_hoisted_children(owner);
    tree.add_hoisted_children(owner, &result.hoisted);
    debug_assert!(result.unclaimed.is_empty());
    result.scrollable_overflow_rect
}
