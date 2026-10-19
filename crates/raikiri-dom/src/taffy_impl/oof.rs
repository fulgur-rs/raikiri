//! Public Taffy positioning at the immediate-container geometry boundary.
//!
//! The document's CSS containing-block pass owns viewport and ancestor
//! positioning. Resolve immediate-container candidates here so all existing
//! consumers keep locations relative to their layout parent.

use super::BlockChildStyle;
use crate::document::OofCoordinate;
use crate::layout::context::LayoutSession;
use taffy::CoreStyle;
use taffy::{
    DetailedLayoutInfo, LayoutContainingBlock, LayoutOutput, LayoutPartialTree, NodeId,
    OofCandidate, OofLayoutResult, OofPositioningArea,
};

impl LayoutContainingBlock for LayoutSession<'_> {
    type OofItemStyle<'a>
        = BlockChildStyle<'a>
    where
        Self: 'a;

    fn get_oof_item_style(&self, node: NodeId) -> Self::OofItemStyle<'_> {
        let index = usize::from(node);
        let style = self
            .transposed_child_style(index)
            .unwrap_or(self.layout_node_with_style(index).unwrap().style);
        let mut query = BlockChildStyle::new(style, None);
        query.oof_table |= matches!(
            self.nodes[index].display(),
            raikiri_style::property::DisplayValue::Table
                | raikiri_style::property::DisplayValue::InlineTable
        );
        query
    }

    fn clear_hoisted_children(&mut self, node: NodeId) {
        let owner = usize::from(node);
        if let Some(previous) = self.context.taffy_hoisted_children.remove(&owner) {
            for child in previous.into_iter().map(usize::from) {
                if self
                    .oof_coordinates
                    .get(&child)
                    .is_some_and(|coordinate| coordinate.owner == owner)
                {
                    self.oof_coordinates.remove(&child);
                }
            }
        }
    }

    fn add_hoisted_children(&mut self, node: NodeId, children: &[NodeId]) {
        if !children.is_empty() {
            let owner = usize::from(node);
            self.context
                .taffy_hoisted_children
                .entry(owner)
                .or_default()
                .extend_from_slice(children);
            for child in children.iter().copied().map(usize::from) {
                let static_owner = self.taffy_parent_of(child).unwrap_or(owner);
                let (follow_static_x, follow_static_y) = {
                    let style = self.layout_node_with_style(child).unwrap().style;
                    (
                        style.inset.left.is_auto() && style.inset.right.is_auto(),
                        style.inset.top.is_auto() && style.inset.bottom.is_auto(),
                    )
                };
                self.oof_coordinates.insert(
                    child,
                    OofCoordinate {
                        owner,
                        static_owner,
                        follow_static_x,
                        follow_static_y,
                        initial_owner_origin: None,
                        initial_static_origin: None,
                    },
                );
            }
            super::align_hoisted_grid_abspos_children(self, node, children);
        }
    }

    fn get_detailed_layout_info(&self, node: NodeId) -> &DetailedLayoutInfo<String> {
        self.context
            .grid_layout_info
            .get(&usize::from(node))
            .unwrap_or(&DetailedLayoutInfo::None)
    }
}

pub(super) fn layout_local_oof(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    output: &mut LayoutOutput,
) {
    let Some(area) = output.oof_positioning_area else {
        return;
    };
    let candidates = output.oof_candidates.take();
    let candidates: Vec<_> = candidates
        .as_slice()
        .iter()
        .copied()
        .map(|mut candidate| {
            let node = usize::from(candidate.node);
            let style_is_fixed = tree
                .layout_node_with_style(node)
                .is_some_and(|node| node.style.position == taffy::Position::Fixed);
            let css_is_fixed =
                tree.nodes[node].position() == &raikiri_style::property::PositionValue::Fixed;
            if tree.nodes[node].fixed_position || css_is_fixed || style_is_fixed {
                candidate.position = taffy::Position::Fixed;
            }
            candidate
        })
        .collect();
    let style = tree.get_core_container_style(node);
    let direction = style.direction();
    let claims = style.is_containing_block();
    let result = layout_candidates(tree, node, &candidates, area, direction, claims);
    output.oof_candidates = result.unclaimed;
    output.scrollable_overflow_rect = output
        .scrollable_overflow_rect
        .union(result.scrollable_overflow_rect);
}

pub(super) fn layout_candidates(
    tree: &mut LayoutSession<'_>,
    owner: NodeId,
    candidates: &[OofCandidate],
    area: OofPositioningArea,
    direction: taffy::Direction,
    claims: taffy::ContainingBlockClaims,
) -> OofLayoutResult {
    let result = taffy::compute::compute_oof_layout_for_area(
        tree, owner, candidates, area, direction, claims,
    );
    tree.clear_hoisted_children(owner);
    tree.add_hoisted_children(owner, &result.hoisted);
    result
}
