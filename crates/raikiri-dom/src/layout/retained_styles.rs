use std::collections::HashMap;
use std::sync::Arc;

use crate::node::arena::BoxKey;

#[derive(Debug, Clone, Default)]
pub(crate) struct RetainedLayoutStyles {
    styles: HashMap<BoxKey, taffy::Style>,
    pub(crate) calc_values: Vec<Arc<crate::taffy_impl::ArenaCalc>>,
}

impl RetainedLayoutStyles {
    pub(crate) fn insert(&mut self, key: BoxKey, style: taffy::Style) {
        self.styles.insert(key, style);
    }

    pub(crate) fn get(&self, key: BoxKey) -> Option<&taffy::Style> {
        self.styles.get(&key)
    }

    pub(crate) fn get_mut(&mut self, key: BoxKey) -> Option<&mut taffy::Style> {
        self.styles.get_mut(&key)
    }

    pub(crate) fn retain_source_styles(&mut self) {
        self.styles
            .retain(|key, _| matches!(key, BoxKey::Source(_)));
    }

    pub(crate) fn retain_source_and_initial_containing_block(&mut self) {
        self.styles
            .retain(|key, _| matches!(key, BoxKey::Source(_) | BoxKey::InitialContainingBlock));
    }

    pub(crate) fn retain_without_cells(&mut self) {
        self.styles
            .retain(|key, _| !matches!(key, BoxKey::AnonymousCell(_)));
    }
}
