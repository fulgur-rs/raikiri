//! Public Taffy positioning at the immediate-container geometry boundary.
//!
//! The document's CSS containing-block pass owns viewport and ancestor
//! positioning. Resolve immediate-container candidates here so all existing
//! consumers keep locations relative to their layout parent.

use super::grid_oof;
use crate::document::OofCoordinate;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, LayoutOutput, NodeId, OofCandidate, OofPositioningArea,
};
use crate::layout::style_views::ReadRole;
use raikiri_layout_geometry::Size;
use raikiri_layout_style::LayoutStyleView;
use taffy::CoreStyle;
use taffy::{DetailedLayoutInfo, LayoutContainingBlock, LayoutPartialTree, OofLayoutResult};

impl LayoutContainingBlock for LayoutSession<'_> {
    type OofItemStyle<'a>
        = LayoutStyleView<'a>
    where
        Self: 'a;

    fn get_oof_item_style(&self, node: NodeId) -> Self::OofItemStyle<'_> {
        self.query_style(usize::from(node), ReadRole::Oof)
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
                    let inset = self.physical_style(child).inset();
                    (
                        inset.left.is_auto() && inset.right.is_auto(),
                        inset.top.is_auto() && inset.bottom.is_auto(),
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

    fn get_detailed_layout_info(&self, node: NodeId) -> &DetailedLayoutInfo<smol_str::SmolStr> {
        let node = usize::from(node);
        // Taffy resolves a grid area on the physical axes of `horizontal-tb`.
        // The tracks of a vertical grid are on its layout axes, so `grid_oof`
        // resolves its grid areas instead.
        if grid_oof::right_to_left(self, node).is_some() {
            return &DetailedLayoutInfo::None;
        }
        self.context
            .grid_layout_info
            .get(&node)
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
    let grid = grid_oof::GridAxes::of(tree, usize::from(node), output.size.width);
    let result = layout_candidates_of(tree, node, candidates.as_slice(), area, grid);
    output.oof_candidates = result.unclaimed;
    output.scrollable_overflow_rect = output
        .scrollable_overflow_rect
        .union(result.scrollable_overflow_rect);
}

/// Position the `candidates` that `node` contains against `area`, its
/// positioning area, and return what it does not claim. The direction and
/// the claims are those of `node`'s own style.
pub(crate) fn layout_container_candidates(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    candidates: &[OofCandidate],
    area: OofPositioningArea,
) -> OofLayoutResult {
    layout_candidates_of(tree, node, candidates, area, None)
}

/// [`layout_container_candidates`] for `node` that is a grid container laid out
/// on the axes `grid` describes, when it is.
fn layout_candidates_of(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    candidates: &[OofCandidate],
    area: OofPositioningArea,
    grid: Option<grid_oof::GridAxes>,
) -> OofLayoutResult {
    let candidates: Vec<_> = candidates
        .iter()
        .copied()
        .map(|mut candidate| {
            let node = usize::from(candidate.node);
            let css_is_fixed =
                tree.nodes[node].position() == &raikiri_style::property::PositionValue::Fixed;
            if tree.nodes[node].fixed_position || css_is_fixed {
                candidate.position = taffy::Position::Fixed;
            }
            candidate
        })
        .collect();
    let style = tree.get_core_container_style(node);
    let direction = style.direction();
    let claims = style.is_containing_block();
    layout_candidates(tree, node, &candidates, area, direction, claims, grid)
}

/// Compute the root layout while resolving intrinsic constraints on the
/// fixed and otherwise unclaimed out-of-flow boxes against the initial
/// containing block.
pub(crate) fn compute_root_layout_with_content_limits(
    tree: &mut LayoutSession<'_>,
    root: NodeId,
    available_space: Size<AvailableSpace>,
) {
    super::content_limits::with_oof_content_limits(tree, None, &[root], |tree| {
        taffy::compute_root_layout(tree, root, available_space);
    });
}

pub(super) fn layout_candidates(
    tree: &mut LayoutSession<'_>,
    owner: NodeId,
    candidates: &[OofCandidate],
    area: OofPositioningArea,
    direction: taffy::Direction,
    claims: taffy::ContainingBlockClaims,
    grid: Option<grid_oof::GridAxes>,
) -> OofLayoutResult {
    let candidate_ids = candidates
        .iter()
        .filter(|candidate| claims.for_position(candidate.position))
        .map(|candidate| candidate.node)
        .collect::<Vec<_>>();
    let result =
        super::content_limits::with_oof_content_limits(tree, Some(&candidate_ids), &[], |tree| {
            match grid {
                Some(axes) => grid_oof::compute_oof_layout(
                    tree, owner, candidates, area, direction, claims, axes,
                ),
                None => taffy::compute::compute_oof_layout_for_area(
                    tree, owner, candidates, area, direction, claims,
                ),
            }
        });
    tree.clear_hoisted_children(owner);
    tree.add_hoisted_children(owner, &result.hoisted);
    result
}
