use std::cell::{RefCell, RefMut};
use std::collections::HashMap;

use crate::document::Document;
use crate::node::arena::BoxKey;

// The next task routes layout entrypoints through these types.
#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct LayoutContext {
    pub(crate) node_states: RefCell<HashMap<BoxKey, NodeLayoutState>>,
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct NodeLayoutState {
    pub(crate) box_key: BoxKey,
    pub(crate) taffy_child_walks: Option<Vec<taffy::NodeId>>,
}

#[allow(dead_code)]
pub(crate) struct LayoutSession<'doc> {
    pub(crate) document: &'doc mut Document,
    pub(crate) context: LayoutContext,
}

#[allow(dead_code)]
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
        Some(RefMut::map(states, |states| {
            states.get_mut(&box_key).expect("state was inserted above")
        }))
    }
}

#[cfg(test)]
mod tests;
