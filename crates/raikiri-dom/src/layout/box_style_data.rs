use super::cache_context::CacheEpoch;
use crate::node::arena::BoxKey;
use raikiri_layout_style::{AlignedCalc, OwnedPropertyOverride, PreparedData, StyleMetadata};
use raikiri_style::ComputedValues;
use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) enum LayoutBase {
    Source(usize),
    Synthetic(Arc<ComputedValues>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedStamp {
    pub(crate) source: CacheEpoch,
    pub(crate) recipe: CacheEpoch,
    pub(crate) metadata: CacheEpoch,
    pub(crate) topology: CacheEpoch,
}

#[derive(Debug)]
pub(crate) struct BoxStyleData {
    pub(crate) base: LayoutBase,
    pub(crate) metadata: StyleMetadata,
    pub(crate) pass: Vec<OwnedPropertyOverride>,
    pub(crate) prepared: Arc<PreparedData>,
    pub(crate) stamp: PreparedStamp,
}

/// Document-owned box facts and the backstop for escaped legacy calc handles.
#[derive(Debug, Default)]
pub(crate) struct BoxStyleStore {
    source: Vec<Option<BoxStyleData>>,
    generated: HashMap<BoxKey, BoxStyleData>,
    pub(crate) calc_values: Vec<Arc<AlignedCalc>>,
    stamp: PreparedStamp,
    ready: Option<PreparedStamp>,
    poisoned: bool,
}

impl Clone for BoxStyleStore {
    fn clone(&self) -> Self {
        let mut cloned = Self {
            calc_values: self.calc_values.clone(),
            stamp: self.stamp,
            poisoned: self.poisoned,
            ..Self::default()
        };
        // Keep escaped handles alive, but never certify records for another document.
        cloned.invalidate_source();
        cloned.poisoned |= !cloned.stamp.recipe.advance();
        cloned
    }
}

impl BoxStyleStore {
    pub(crate) fn stamp(&self) -> PreparedStamp {
        self.stamp
    }

    pub(crate) fn begin_publication(&mut self) {
        self.ready = None;
    }

    pub(crate) fn finish_publication(&mut self, stamp: PreparedStamp) -> bool {
        if self.poisoned
            || stamp != self.stamp
            || self
                .source
                .iter()
                .flatten()
                .chain(self.generated.values())
                .any(|record| record.stamp != stamp)
        {
            return false;
        }
        self.ready = Some(stamp);
        true
    }

    pub(crate) fn is_ready(&self) -> bool {
        !self.poisoned && self.ready == Some(self.stamp)
    }

    pub(crate) fn get(&self, key: BoxKey) -> Option<&BoxStyleData> {
        match key {
            BoxKey::Source(index) => self.source.get(index)?.as_ref(),
            _ => self.generated.get(&key),
        }
    }

    pub(crate) fn insert(&mut self, key: BoxKey, data: BoxStyleData) {
        self.begin_publication();
        match key {
            BoxKey::Source(index) => {
                if index >= self.source.len() {
                    self.source.resize_with(index + 1, || None);
                }
                self.source[index] = Some(data);
            }
            _ => {
                self.generated.insert(key, data);
            }
        }
    }

    pub(crate) fn invalidate_source(&mut self) {
        self.begin_publication();
        self.poisoned |= !self.stamp.source.advance();
        self.poisoned |= !self.stamp.topology.advance();
        self.poisoned |= !self.stamp.metadata.advance();
    }

    pub(crate) fn invalidate_key(&mut self, key: BoxKey) {
        self.begin_publication();
        match key {
            BoxKey::Source(index) => {
                if let Some(record) = self.source.get_mut(index) {
                    *record = None;
                }
                self.poisoned |= !self.stamp.source.advance();
            }
            _ => {
                self.generated.remove(&key);
            }
        }
        self.invalidate_recipe();
    }

    pub(crate) fn invalidate_anonymous_tables(&mut self) {
        self.begin_publication();
        self.generated
            .retain(|key, _| matches!(key, BoxKey::InitialContainingBlock));
        self.invalidate_recipe();
    }

    pub(crate) fn invalidate_anonymous_cells(&mut self) {
        self.begin_publication();
        self.generated
            .retain(|key, _| !matches!(key, BoxKey::AnonymousCell(_)));
        self.invalidate_recipe();
    }

    fn invalidate_recipe(&mut self) {
        self.begin_publication();
        self.poisoned |= !self.stamp.recipe.advance();
        self.poisoned |= !self.stamp.metadata.advance();
        self.poisoned |= !self.stamp.topology.advance();
    }
}

#[cfg(test)]
mod tests;
