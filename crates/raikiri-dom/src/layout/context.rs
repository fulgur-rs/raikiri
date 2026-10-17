use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use crate::document::Document;
use crate::fragment::FragmentationContext;
use crate::layout::LayoutWarn;
use crate::node::arena::BoxKey;

// The next task routes layout entrypoints through these types.
#[derive(Debug, Default)]
pub(crate) struct LayoutContext {
    pub(crate) node_states: RefCell<HashMap<BoxKey, NodeLayoutState>>,
    pub(crate) layout_warnings: Vec<LayoutWarn>,
    pub(crate) table_layout_error: Option<String>,
    pub(crate) ifc_width_probe: bool,
    pub(crate) fragmentation_stack: Vec<FragmentationContext>,
    pub(crate) vertical_oof_containing_blocks: Vec<(usize, f32)>,
    pub(crate) orthogonal_block_in_layout: Option<usize>,
    pub(crate) transposed_blocks: Vec<crate::taffy_impl::TransposedBlock>,
    pub(crate) block_item_percent_bases: Vec<(usize, Option<f32>)>,
    pub(crate) flex_item_percent_bases: Vec<(usize, taffy::Size<Option<f32>>)>,
    pub(crate) size_contained_measure: Vec<usize>,
    pub(crate) grid_layout_info: HashMap<usize, taffy::DetailedLayoutInfo<String>>,
    pub(crate) taffy_hoisted_children: HashMap<usize, Vec<taffy::NodeId>>,
    pub(crate) pagination_on_transposed_axes: bool,
    pub(crate) in_flow_heights: HashMap<usize, f32>,
}

#[derive(Debug)]
pub(crate) struct NodeLayoutState {
    pub(crate) box_key: BoxKey,
    pub(crate) taffy_child_walks: Option<Vec<taffy::NodeId>>,
}

pub(crate) struct LayoutSession<'doc> {
    pub(crate) document: &'doc mut Document,
    pub(crate) context: LayoutContext,
}

impl<'doc> LayoutSession<'doc> {
    pub(crate) fn new(document: &'doc mut Document) -> Self {
        Self {
            document,
            context: LayoutContext::default(),
        }
    }

    pub(crate) fn node_state(&self, key: usize) -> Option<RefMut<'_, NodeLayoutState>> {
        let box_key = self.document.nodes.box_key(key)?;
        let mut states = self.context.node_states.borrow_mut();
        states.entry(box_key).or_insert_with(|| NodeLayoutState {
            box_key,
            taffy_child_walks: None,
        });
        let state = RefMut::map(states, |states| {
            states.get_mut(&box_key).expect("state was inserted above")
        });
        debug_assert_eq!(state.box_key, box_key);
        Some(state)
    }
}

impl Deref for LayoutSession<'_> {
    type Target = Document;

    fn deref(&self) -> &Self::Target {
        self.document
    }
}

impl DerefMut for LayoutSession<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.document
    }
}

#[cfg(test)]
mod tests;
