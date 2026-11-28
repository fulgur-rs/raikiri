//! The layout session as the layout library sees it.
//!
//! The library's algorithms call back into the tree they lay out through a set
//! of traits. The implementations here only translate: they convert the
//! library's types to the protocol's, call the session, and convert what comes
//! back. The work itself is done by the methods of the session that have the
//! same names and take the protocol's types.

use super::GridInfo;
use super::convert::{FromTaffy, IntoTaffy};
use crate::layout::context::LayoutSession;
use crate::layout::dispatch::TaffyChildIter;
use crate::layout::protocol::{AvailableSpace, LayoutInput, NodeId, RunMode};
use crate::layout::style_views::ReadRole;
use crate::layout::table;
use raikiri_layout_geometry::BoxLayout;
use raikiri_layout_style::TaffyStyle;
use taffy::util::ResolveOrZero;
use taffy::{
    CacheTree, CoreStyle, DetailedGridInfo, DetailedLayoutInfo, LayoutBlockContainer,
    LayoutContainingBlock, LayoutFlexboxContainer, LayoutGridContainer, LayoutPartialTree,
    TraversePartialTree, TraverseTree,
};

/// The children of a node, as the library's node ids.
pub struct ChildIds<'a>(TaffyChildIter<'a>);

impl Iterator for ChildIds<'_> {
    type Item = taffy::NodeId;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(taffy::NodeId::from)
    }
}

impl CacheTree for LayoutSession<'_> {
    fn cache_get(
        &mut self,
        node_id: taffy::NodeId,
        input: &taffy::LayoutInput,
    ) -> Option<taffy::LayoutOutput> {
        // Native root layout checks only whether this entry exists. Owned
        // geometry consumers use the protocol cache and retain its origin.
        LayoutSession::cache_get(
            self,
            NodeId::from_taffy(node_id),
            &LayoutInput::from_taffy(*input),
        )
        .map(IntoTaffy::into_taffy)
    }

    fn cache_store(
        &mut self,
        node_id: taffy::NodeId,
        input: &taffy::LayoutInput,
        layout_output: taffy::LayoutOutput,
    ) {
        LayoutSession::cache_store(
            self,
            NodeId::from_taffy(node_id),
            &LayoutInput::from_taffy(*input),
            FromTaffy::from_taffy(layout_output),
        );
    }

    fn cache_clear(&mut self, node_id: taffy::NodeId) {
        LayoutSession::cache_clear(self, NodeId::from_taffy(node_id));
    }
}

impl LayoutPartialTree for LayoutSession<'_> {
    type CustomIdent = smol_str::SmolStr;
    type CoreContainerStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;

    fn get_core_container_style(&self, node_id: taffy::NodeId) -> Self::CoreContainerStyle<'_> {
        TaffyStyle::new(self.query_style(usize::from(node_id), ReadRole::CoreContainer))
    }

    fn set_unrounded_layout(&mut self, node_id: taffy::NodeId, layout: &taffy::Layout) {
        let node = NodeId::from_taffy(node_id);
        let mut layout = BoxLayout::from_taffy(*layout);
        if let Some(rect) = self
            .context
            .native_overflow
            .as_ref()
            .and_then(|scope| scope.physical_children.get(&usize::from(node)))
        {
            layout.scrollable_overflow_rect = *rect;
        }
        LayoutSession::set_unrounded_layout(self, node, &layout);
        self.install_root_static_position_overrides(node);
    }

    fn resolve_calc_value(&self, val: *const (), basis: f32) -> f32 {
        crate::layout::dispatch::resolve_calc(val, basis)
    }

    fn compute_child_layout(
        &mut self,
        node_id: taffy::NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        let node = NodeId::from_taffy(node_id);
        let inputs = offered_without_shim(self, node, LayoutInput::from_taffy(inputs));
        let reversed = self
            .context
            .native_overflow
            .as_ref()
            .filter(|scope| scope.owner != node)
            .map(|scope| scope.reversed);
        let output = LayoutSession::compute_child_layout(self, node, inputs);
        if inputs.run_mode == RunMode::PerformLayout
            && let Some(capture) = self
                .context
                .root_oof_capture
                .as_mut()
                .filter(|capture| capture.root == node)
        {
            capture.candidates = output.oof_candidates.clone();
        }
        if inputs.run_mode == RunMode::PerformLayout
            && let Some(scope) = self.context.native_overflow.as_mut()
        {
            if output.overflow_origin == crate::layout::protocol::OverflowOrigin::Physical {
                scope
                    .physical_children
                    .insert(usize::from(node), output.scrollable_overflow_rect);
            } else {
                scope.physical_children.remove(&usize::from(node));
            }
        }
        super::algorithms::native_child_output(self, node, output, reversed).into_taffy()
    }
}

impl LayoutBlockContainer for LayoutSession<'_> {
    type BlockContainerStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: taffy::NodeId) -> Self::BlockContainerStyle<'_> {
        TaffyStyle::new(self.query_style(usize::from(node_id), ReadRole::BlockContainer))
    }

    fn get_block_child_style(&self, child_node_id: taffy::NodeId) -> Self::BlockItemStyle<'_> {
        TaffyStyle::new(self.query_style(usize::from(child_node_id), ReadRole::BlockItem))
    }
}

impl LayoutFlexboxContainer for LayoutSession<'_> {
    type FlexboxContainerStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;

    fn get_flexbox_container_style(
        &self,
        node_id: taffy::NodeId,
    ) -> Self::FlexboxContainerStyle<'_> {
        TaffyStyle::new(self.query_style(usize::from(node_id), ReadRole::FlexContainer))
    }

    fn get_flexbox_child_style(&self, child_node_id: taffy::NodeId) -> Self::FlexboxItemStyle<'_> {
        TaffyStyle::new(table::item_style(
            self,
            usize::from(child_node_id),
            ReadRole::FlexItem,
        ))
    }
}

impl LayoutGridContainer for LayoutSession<'_> {
    type GridContainerStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: taffy::NodeId) -> Self::GridContainerStyle<'_> {
        TaffyStyle::new(self.query_style(usize::from(node_id), ReadRole::GridContainer))
    }

    fn get_grid_child_style(&self, child_node_id: taffy::NodeId) -> Self::GridItemStyle<'_> {
        let item = usize::from(child_node_id);
        let view = table::item_style(self, item, ReadRole::GridItem);
        // The track sizing of the library carries an aspect ratio over to an
        // auto size before it looks for the automatic minimum size, so a
        // scroll container would contribute the carried size instead of the
        // zero of CSS Grid 1 §6.6. The item it sizes the tracks with is built
        // from the first read, so only that read hides the ratio; the item's
        // own layout and its alignment still use it.
        let view = if scrolls_at_a_carried_size(&view)
            && super::placement_read(&self.context, self.taffy_parent_of(item), item)
        {
            view.with_hidden_aspect_ratio()
        } else {
            view
        };
        // An item that shares a baseline along its column is sized and
        // placed with the room its alignment shifts it by.
        match self.context.grid_baseline_shims.get(&item) {
            Some(&shim) => TaffyStyle::new(view).with_extra_margin_left(shim),
            None => TaffyStyle::new(view),
        }
    }

    fn set_detailed_grid_info(
        &mut self,
        node_id: taffy::NodeId,
        detailed_grid_info: DetailedGridInfo<Self::CustomIdent>,
    ) {
        LayoutSession::set_detailed_grid_info(
            self,
            NodeId::from_taffy(node_id),
            GridInfo::new(detailed_grid_info),
        );
    }
}

impl LayoutContainingBlock for LayoutSession<'_> {
    type OofItemStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;

    fn get_oof_item_style(&self, node: taffy::NodeId) -> Self::OofItemStyle<'_> {
        let view = self.query_style(usize::from(node), ReadRole::Oof);
        let view = if self.oof_alignment_swapped() {
            view.with_swapped_oof_alignment()
        } else {
            view
        };
        TaffyStyle::new(if table::wrapper_owns_height(self, usize::from(node)) {
            view.with_hidden_table_height()
        } else {
            view
        })
    }

    fn clear_hoisted_children(&mut self, node: taffy::NodeId) {
        LayoutSession::clear_hoisted_children(self, NodeId::from_taffy(node));
    }

    fn add_hoisted_children(&mut self, node: taffy::NodeId, children: &[taffy::NodeId]) {
        let node = NodeId::from_taffy(node);
        self.remove_root_static_position_overrides(node);
        LayoutSession::add_hoisted_children(
            self,
            node,
            children.iter().copied().map(NodeId::from_taffy),
        );
    }

    fn get_detailed_layout_info(
        &self,
        node: taffy::NodeId,
    ) -> &DetailedLayoutInfo<smol_str::SmolStr> {
        LayoutSession::oof_grid_info(self, NodeId::from_taffy(node)).detailed()
    }
}

impl TraversePartialTree for LayoutSession<'_> {
    type ChildIter<'a>
        = ChildIds<'a>
    where
        Self: 'a;

    fn child_ids(&self, node_id: taffy::NodeId) -> Self::ChildIter<'_> {
        ChildIds(TaffyChildIter::new_for_session(self, usize::from(node_id)))
    }

    fn child_count(&self, node_id: taffy::NodeId) -> usize {
        LayoutSession::child_count(self, NodeId::from_taffy(node_id))
    }

    fn get_child_id(&self, node_id: taffy::NodeId, index: usize) -> taffy::NodeId {
        LayoutSession::get_child_id(self, NodeId::from_taffy(node_id), index).into_taffy()
    }
}

impl TraverseTree for LayoutSession<'_> {}

/// Whether a grid item has an aspect ratio that carries its definite size in
/// one axis over to an auto size in the other, where it is a scroll container.
fn scrolls_at_a_carried_size(view: &raikiri_layout_style::LayoutStyleView<'_>) -> bool {
    if view.aspect_ratio().is_none()
        || view.position() == raikiri_layout_style::values::Position::Absolute
    {
        return false;
    }
    let size = view.size();
    let overflow = view.overflow();
    (size.width.is_auto() && !size.height.is_auto() && overflow.x.is_scroll_container())
        || (size.height.is_auto() && !size.width.is_auto() && overflow.y.is_scroll_container())
}

/// The input a grid item is laid out under, where its left margin carries a
/// baseline shim: the item is offered the space of its grid area less its own
/// margins, as it is without the shim, for the shim only widens the columns
/// and shifts the item.
pub(crate) fn offered_without_shim(
    session: &LayoutSession<'_>,
    node: NodeId,
    mut inputs: LayoutInput,
) -> LayoutInput {
    let idx = usize::from(node);
    if let Some(&shim) = session.context.grid_baseline_shims.get(&idx)
        && let AvailableSpace::Definite(width) = inputs.available_space.width
    {
        // The algorithm clamps the space at zero, where the shim alone no
        // longer gives it back: take it from the grid area instead, which
        // is the item's percentage basis.
        let offered = match inputs.parent_size.width {
            Some(area) if width <= 0.0 => {
                let margin = TaffyStyle::new(session.physical_style(idx)).margin();
                let calc = crate::layout::dispatch::resolve_calc;
                let own = margin.left.resolve_or_zero(Some(area), calc)
                    + margin.right.resolve_or_zero(Some(area), calc);
                (area - own).max(0.0)
            }
            _ => width + shim,
        };
        inputs.available_space.width = AvailableSpace::Definite(offered);
    }
    inputs
}
