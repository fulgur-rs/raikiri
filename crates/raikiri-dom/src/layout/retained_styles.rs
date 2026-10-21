use rustc_hash::FxHashMap as HashMap;

use crate::node::arena::BoxKey;

#[derive(Debug, Clone, Default)]
pub(crate) struct RetainedLayoutStyles {
    // Source keys are arena indices; holes stay absent during out-of-order insertion.
    source_styles: Vec<Option<crate::LayoutStyle>>,
    styles: HashMap<BoxKey, crate::LayoutStyle>,
}

#[cfg(test)]
mod tests;

impl RetainedLayoutStyles {
    pub(crate) fn insert(&mut self, key: BoxKey, style: crate::LayoutStyle) {
        if let BoxKey::Source(index) = key {
            if index >= self.source_styles.len() {
                self.source_styles.resize_with(index + 1, || None);
            }
            self.source_styles[index] = Some(style);
        } else {
            self.styles.insert(key, style);
        }
    }

    pub(crate) fn get(&self, key: BoxKey) -> Option<&crate::LayoutStyle> {
        match key {
            BoxKey::Source(index) => self.source_styles.get(index)?.as_ref(),
            _ => self.styles.get(&key),
        }
    }

    pub(crate) fn get_mut(&mut self, key: BoxKey) -> Option<&mut crate::LayoutStyle> {
        match key {
            BoxKey::Source(index) => self.source_styles.get_mut(index)?.as_mut(),
            _ => self.styles.get_mut(&key),
        }
    }

    pub(crate) fn retain_source_styles(&mut self) {
        self.styles.clear();
    }

    pub(crate) fn retain_source_and_initial_containing_block(&mut self) {
        self.styles
            .retain(|key, _| matches!(key, BoxKey::InitialContainingBlock));
    }

    pub(crate) fn retain_without_cells(&mut self) {
        self.styles
            .retain(|key, _| !matches!(key, BoxKey::AnonymousCell(_)));
    }
}
