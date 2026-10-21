use super::cache_context::CacheEpoch;
use crate::node::arena::BoxKey;
use raikiri_layout_style::{AlignedCalc, OwnedPropertyOverride, PreparedData, StyleMetadata};
use raikiri_style::ComputedValues;
use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;

pub(crate) mod recipes;

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
    pub(crate) recipe: Option<recipes::RecipeKind>,
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
    icb: Option<BoxStyleData>,
    retired: Vec<BoxStyleData>,
    pub(crate) calc_values: Vec<Arc<AlignedCalc>>,
    pub(crate) pending: HashMap<BoxKey, Vec<OwnedPropertyOverride>>,
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
                .chain(self.icb.iter())
                .chain(self.generated.values())
                .any(|record| record.stamp != stamp)
        {
            return false;
        }
        self.retired.clear();
        self.ready = Some(stamp);
        true
    }

    pub(crate) fn is_ready(&self) -> bool {
        !self.poisoned && self.ready == Some(self.stamp)
    }

    pub(crate) fn get(&self, key: BoxKey) -> Option<&BoxStyleData> {
        match key {
            BoxKey::Source(index) => self.source.get(index)?.as_ref(),
            BoxKey::InitialContainingBlock => self.icb.as_ref(),
            _ => self.generated.get(&key),
        }
    }

    pub(crate) fn get_mut(&mut self, key: BoxKey) -> Option<&mut BoxStyleData> {
        match key {
            BoxKey::Source(index) => self.source.get_mut(index)?.as_mut(),
            BoxKey::InitialContainingBlock => self.icb.as_mut(),
            _ => self.generated.get_mut(&key),
        }
    }

    pub(crate) fn insert(&mut self, key: BoxKey, data: BoxStyleData) {
        self.begin_publication();
        let old = match key {
            BoxKey::Source(index) => {
                if index >= self.source.len() {
                    self.source.resize_with(index + 1, || None);
                }
                self.source[index].replace(data)
            }
            BoxKey::InitialContainingBlock => self.icb.replace(data),
            _ => self.generated.insert(key, data),
        };
        if let Some(old) = old {
            self.retired.push(old);
        }
    }

    pub(crate) fn take(&mut self, key: BoxKey) -> Option<BoxStyleData> {
        match key {
            BoxKey::Source(index) => self.source.get_mut(index)?.take(),
            BoxKey::InitialContainingBlock => self.icb.take(),
            _ => self.generated.remove(&key),
        }
    }

    pub(crate) fn begin_session(&mut self) {
        self.begin_publication();
        self.pending.clear();
        for record in self
            .source
            .iter_mut()
            .flatten()
            .chain(self.icb.iter_mut())
            .chain(self.generated.values_mut())
        {
            record.pass.clear();
        }
    }

    pub(crate) fn reserve_sources(&mut self, count: usize) {
        self.source
            .reserve_exact(count.saturating_sub(self.source.len()));
    }

    pub(crate) fn begin_bridge(&mut self) {
        self.begin_publication();
        self.retired.extend(self.source.drain(..).flatten());
        self.pending.clear();
        for record in self.icb.iter_mut().chain(self.generated.values_mut()) {
            record.pass.clear();
        }
    }

    pub(crate) fn invalidate_metadata(&mut self) {
        self.begin_publication();
        self.poisoned |= !self.stamp.metadata.advance();
    }

    pub(crate) fn publish_recipe(&mut self, key: BoxKey, recipe: recipes::Recipe) {
        self.invalidate_recipe();
        self.pending.remove(&key);
        self.insert(
            key,
            BoxStyleData {
                base: LayoutBase::Synthetic(recipe.values),
                recipe: Some(recipe.kind),
                metadata: recipe.metadata,
                pass: Vec::new(),
                prepared: recipes::empty_prepared_data(),
                stamp: self.stamp,
            },
        );
    }

    pub(crate) fn invalidate_source(&mut self) {
        self.begin_publication();
        self.poisoned |= !self.stamp.source.advance();
        self.poisoned |= !self.stamp.topology.advance();
        self.poisoned |= !self.stamp.metadata.advance();
    }

    pub(crate) fn refresh_recipe(&mut self, key: BoxKey, recipe: recipes::Recipe) {
        let pass = if self
            .get(key)
            .is_some_and(|record| record.recipe == Some(recipe.kind))
        {
            std::mem::take(&mut self.get_mut(key).unwrap().pass)
        } else {
            Vec::new()
        };
        self.publish_recipe(key, recipe);
        self.get_mut(key).unwrap().pass = pass;
    }

    pub(crate) fn invalidate_key(&mut self, key: BoxKey) {
        self.begin_publication();
        if let Some(old) = self.take(key) {
            self.retired.push(old);
        }
        if matches!(key, BoxKey::Source(_)) {
            self.poisoned |= !self.stamp.source.advance();
        }
        self.invalidate_recipe();
    }

    pub(crate) fn invalidate_anonymous_tables(&mut self) {
        self.begin_publication();
        self.retired
            .extend(self.generated.drain().map(|(_, record)| record));
        self.invalidate_recipe();
    }

    pub(crate) fn invalidate_anonymous_cells(&mut self) {
        self.begin_publication();
        self.retired.extend(
            self.generated
                .extract_if(|key, _| matches!(key, BoxKey::AnonymousCell(_)))
                .map(|(_, record)| record),
        );
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
