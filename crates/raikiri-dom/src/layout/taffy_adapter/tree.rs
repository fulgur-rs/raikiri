//! The layout session as the layout library sees it.
//!
//! The library's algorithms call back into the tree they lay out through a set
//! of traits. The implementations here only translate: they convert the
//! library's types to the protocol's, call the session, and convert what comes
//! back. The work itself is done by the methods of the session that have the
//! same names and take the protocol's types.

use super::GridInfo;
use super::convert::{FromTaffy, IntoTaffy};
use crate::document::Document;
use crate::layout::context::LayoutSession;
use crate::layout::dispatch::TaffyChildIter;
use crate::layout::protocol::{LayoutInput, NodeId};
use crate::layout::style_views::ReadRole;
use raikiri_layout_geometry::BoxLayout;
use raikiri_layout_style::LayoutStyleView;
use taffy::{
    CacheTree, DetailedGridInfo, DetailedLayoutInfo, LayoutBlockContainer, LayoutContainingBlock,
    LayoutFlexboxContainer, LayoutGridContainer, LayoutPartialTree, TraversePartialTree,
    TraverseTree,
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
        = LayoutStyleView<'a>
    where
        Self: 'a;

    fn get_core_container_style(&self, node_id: taffy::NodeId) -> Self::CoreContainerStyle<'_> {
        self.query_style(usize::from(node_id), ReadRole::CoreContainer)
    }

    fn set_unrounded_layout(&mut self, node_id: taffy::NodeId, layout: &taffy::Layout) {
        LayoutSession::set_unrounded_layout(
            self,
            NodeId::from_taffy(node_id),
            &BoxLayout::from_taffy(*layout),
        );
    }

    fn resolve_calc_value(&self, val: *const (), basis: f32) -> f32 {
        crate::layout::dispatch::resolve_calc(val, basis)
    }

    fn compute_child_layout(
        &mut self,
        node_id: taffy::NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        LayoutSession::compute_child_layout(
            self,
            NodeId::from_taffy(node_id),
            LayoutInput::from_taffy(inputs),
        )
        .into_taffy()
    }
}

impl LayoutBlockContainer for LayoutSession<'_> {
    type BlockContainerStyle<'a>
        = LayoutStyleView<'a>
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = LayoutStyleView<'a>
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: taffy::NodeId) -> Self::BlockContainerStyle<'_> {
        self.query_style(usize::from(node_id), ReadRole::BlockContainer)
    }

    fn get_block_child_style(&self, child_node_id: taffy::NodeId) -> Self::BlockItemStyle<'_> {
        self.query_style(usize::from(child_node_id), ReadRole::BlockItem)
    }
}

impl LayoutFlexboxContainer for LayoutSession<'_> {
    type FlexboxContainerStyle<'a>
        = LayoutStyleView<'a>
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = LayoutStyleView<'a>
    where
        Self: 'a;

    fn get_flexbox_container_style(
        &self,
        node_id: taffy::NodeId,
    ) -> Self::FlexboxContainerStyle<'_> {
        self.query_style(usize::from(node_id), ReadRole::FlexContainer)
    }

    fn get_flexbox_child_style(&self, child_node_id: taffy::NodeId) -> Self::FlexboxItemStyle<'_> {
        self.query_style(usize::from(child_node_id), ReadRole::FlexItem)
    }
}

impl LayoutGridContainer for LayoutSession<'_> {
    type GridContainerStyle<'a>
        = LayoutStyleView<'a>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = LayoutStyleView<'a>
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: taffy::NodeId) -> Self::GridContainerStyle<'_> {
        self.query_style(usize::from(node_id), ReadRole::GridContainer)
    }

    fn get_grid_child_style(&self, child_node_id: taffy::NodeId) -> Self::GridItemStyle<'_> {
        self.query_style(usize::from(child_node_id), ReadRole::GridItem)
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
        = LayoutStyleView<'a>
    where
        Self: 'a;

    fn get_oof_item_style(&self, node: taffy::NodeId) -> Self::OofItemStyle<'_> {
        self.query_style(usize::from(node), ReadRole::Oof)
    }

    fn clear_hoisted_children(&mut self, node: taffy::NodeId) {
        LayoutSession::clear_hoisted_children(self, NodeId::from_taffy(node));
    }

    fn add_hoisted_children(&mut self, node: taffy::NodeId, children: &[taffy::NodeId]) {
        LayoutSession::add_hoisted_children(
            self,
            NodeId::from_taffy(node),
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

impl TraversePartialTree for Document {
    type ChildIter<'a>
        = ChildIds<'a>
    where
        Self: 'a;

    fn child_ids(&self, node_id: taffy::NodeId) -> Self::ChildIter<'_> {
        ChildIds(TaffyChildIter::new(self, usize::from(node_id), false))
    }

    fn child_count(&self, node_id: taffy::NodeId) -> usize {
        TaffyChildIter::new(self, usize::from(node_id), false).count()
    }

    fn get_child_id(&self, node_id: taffy::NodeId, index: usize) -> taffy::NodeId {
        taffy::NodeId::from(
            TaffyChildIter::new(self, usize::from(node_id), false)
                .nth(index)
                .expect("get_child_id: index out of range"),
        )
    }
}

impl TraverseTree for Document {}
