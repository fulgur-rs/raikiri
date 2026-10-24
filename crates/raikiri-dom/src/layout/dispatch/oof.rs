//! Public Taffy positioning at the immediate-container geometry boundary.
//!
//! The document's CSS containing-block pass owns viewport and ancestor
//! positioning. Resolve immediate-container candidates here so all existing
//! consumers keep locations relative to their layout parent.

use super::grid_oof;
use crate::document::OofCoordinate;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, LayoutOutput, NodeId, OofCandidate, OofCandidates, OofLayoutResult,
    OofPosition, OofPositioningArea,
};
use crate::layout::style_views::ReadRole;
use crate::layout::taffy_adapter::algorithms::{
    claims_position, compute_oof_layout_for_area, compute_root_layout,
};
use crate::layout::taffy_adapter::{GridInfo, OofBatch, OofSpan};
use raikiri_layout_geometry::Size;

impl LayoutSession<'_> {
    /// Forget the hoisted children recorded for `node` by an earlier layout.
    pub(crate) fn clear_hoisted_children(&mut self, node: NodeId) {
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

    /// Record `children` as hoisted out of `node` to the box that is their
    /// containing block.
    pub(crate) fn add_hoisted_children<I>(&mut self, node: NodeId, children: I)
    where
        I: Iterator<Item = NodeId> + Clone,
    {
        if children.clone().next().is_some() {
            let owner = usize::from(node);
            self.context
                .taffy_hoisted_children
                .entry(owner)
                .or_default()
                .extend(children.clone());
            for child in children.clone().map(usize::from) {
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

    /// The tracks and areas a grid container reported, which the library
    /// positions its out-of-flow children in.
    pub(crate) fn oof_grid_info(&self, node: NodeId) -> &GridInfo {
        let node = usize::from(node);
        // The library resolves a grid area on the physical axes of
        // `horizontal-tb`. The tracks of a vertical grid are on its layout
        // axes, so `grid_oof` resolves its grid areas instead.
        if grid_oof::right_to_left(self, node).is_some() {
            return &GridInfo::NONE;
        }
        self.context
            .grid_layout_info
            .get(&node)
            .unwrap_or(&GridInfo::NONE)
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
    let result = layout_candidates_of(tree, node, candidates.iter(), area, grid);
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
    candidates: &OofCandidates,
    area: OofPositioningArea,
) -> OofLayoutResult {
    layout_candidates_of(tree, node, candidates.iter(), area, None)
}

/// [`layout_container_candidates`] for `node` that is a grid container laid out
/// on the axes `grid` describes, when it is.
fn layout_candidates_of(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    candidates: impl Iterator<Item = OofCandidate>,
    area: OofPositioningArea,
    grid: Option<grid_oof::GridAxes>,
) -> OofLayoutResult {
    let batch = OofBatch::collect(candidates.map(|mut candidate| {
        let node = usize::from(candidate.node);
        let css_is_fixed =
            tree.nodes[node].position() == &raikiri_style::property::PositionValue::Fixed;
        if tree.nodes[node].fixed_position || css_is_fixed {
            candidate.position = OofPosition::Fixed;
        }
        candidate
    }));
    let style = tree.query_style(usize::from(node), ReadRole::CoreContainer);
    let direction = style.direction();
    let claims = style.is_containing_block();
    layout_candidates(tree, node, &batch, area, direction, claims, grid)
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
        compute_root_layout(tree, root, available_space);
    });
}

pub(super) fn layout_candidates(
    tree: &mut LayoutSession<'_>,
    owner: NodeId,
    candidates: &OofBatch,
    area: OofPositioningArea,
    direction: taffy::Direction,
    claims: taffy::ContainingBlockClaims,
    grid: Option<grid_oof::GridAxes>,
) -> OofLayoutResult {
    let candidate_ids = candidates
        .iter()
        .filter(|candidate| claims_position(claims, candidate.position))
        .map(|candidate| candidate.node)
        .collect::<Vec<_>>();
    let result =
        super::content_limits::with_oof_content_limits(tree, Some(&candidate_ids), &[], |tree| {
            let span: OofSpan<'_> = candidates.all();
            match grid {
                Some(axes) => {
                    grid_oof::compute_oof_layout(tree, owner, span, area, direction, claims, axes)
                }
                None => compute_oof_layout_for_area(tree, owner, span, area, direction, claims),
            }
        });
    tree.clear_hoisted_children(owner);
    tree.add_hoisted_children(owner, result.hoisted.iter().copied());
    result
}
