use crate::layout::taffy_adapter::LayoutCache;
use crate::{layout::context::LayoutSession, node::arena::BoxKey};
use raikiri_layout_style::{PercentBasis, ViewContext};
use raikiri_style::property::WritingMode;
use rustc_hash::FxHashMap;
use std::cell::{RefCell, RefMut};

mod scopes;

/// A generation stays distinct when its 64-bit counter rolls over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct CacheEpoch {
    namespace: u64,
    value: u64,
}

impl Default for CacheEpoch {
    fn default() -> Self {
        Self {
            namespace: 0,
            value: 1,
        }
    }
}

impl CacheEpoch {
    #[cfg(test)]
    pub(crate) fn for_test(namespace: u64, value: u64) -> Self {
        Self { namespace, value }
    }

    /// Advance without reusing a counter in the same namespace.
    pub(crate) fn advance(&mut self) -> bool {
        if let Some(value) = self.value.checked_add(1) {
            self.value = value;
            true
        } else if let Some(namespace) = self.namespace.checked_add(1) {
            *self = Self {
                namespace,
                value: 1,
            };
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BasisKey {
    Taffy,
    Indefinite,
    Definite(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CacheFingerprint {
    axes: WritingMode,
    align_axis_is_inline: bool,
    basis: BasisKey,
    source: u64,
    metadata: u64,
    overrides: u64,
}

impl CacheFingerprint {
    pub(crate) fn new(view: ViewContext, source: u64, metadata: u64, overrides: u64) -> Self {
        let basis = match view.percent_basis {
            PercentBasis::Taffy => BasisKey::Taffy,
            PercentBasis::Indefinite => BasisKey::Indefinite,
            PercentBasis::Definite(value) => BasisKey::Definite(if value.is_nan() {
                f32::NAN.to_bits()
            } else if value == 0.0 {
                0
            } else {
                value.to_bits()
            }),
        };
        Self {
            axes: view.layout_wm,
            align_axis_is_inline: view.align_axis_is_inline,
            basis,
            source,
            metadata,
            overrides,
        }
    }
}

#[derive(Debug)]
pub(crate) struct CacheFrame {
    key: BoxKey,
    epoch: CacheEpoch,
    depth: usize,
    previous: Option<ActiveFrame>,
}

#[derive(Clone, Copy, Debug)]
struct ActiveFrame {
    key: BoxKey,
    epoch: CacheEpoch,
    depth: usize,
}

#[derive(Debug)]
struct CacheNodeState {
    context: Option<CacheFingerprint>,
    stamp: u64,
    overrides: u64,
    walk_session: u64,
    layout: super::context::NodeLayoutState,
}

impl CacheNodeState {
    fn new(key: BoxKey) -> Self {
        Self {
            context: None,
            stamp: 0,
            overrides: 0,
            walk_session: 0,
            layout: super::context::NodeLayoutState {
                box_key: key,
                taffy_child_walks: None,
            },
        }
    }
}

/// Small traversals stay inline; larger traversals share one pre-sized map.
#[derive(Debug, Default)]
struct NodeStates {
    first: [Option<(BoxKey, CacheNodeState)>; 4],
    // Keys come from validated arena identities, never external strings or arbitrary integers.
    rest: FxHashMap<BoxKey, CacheNodeState>,
    expected: usize,
}

impl NodeStates {
    fn get(&self, key: &BoxKey) -> Option<&CacheNodeState> {
        if self.rest.is_empty() {
            self.first
                .iter()
                .flatten()
                .find(|(first, _)| first == key)
                .map(|(_, state)| state)
        } else {
            self.rest.get(key)
        }
    }
    fn get_mut(&mut self, key: &BoxKey) -> Option<&mut CacheNodeState> {
        if self.rest.is_empty() {
            self.first
                .iter_mut()
                .flatten()
                .find(|(first, _)| first == key)
                .map(|(_, state)| state)
        } else {
            self.rest.get_mut(key)
        }
    }
    fn get_or_insert(&mut self, key: BoxKey) -> &mut CacheNodeState {
        if self.rest.is_empty() {
            if let Some(index) = self
                .first
                .iter()
                .position(|slot| slot.as_ref().is_some_and(|(first, _)| *first == key))
            {
                return &mut self.first[index].as_mut().expect("inline state exists").1;
            }
            if let Some(index) = self.first.iter().position(Option::is_none) {
                self.first[index] = Some((key, CacheNodeState::new(key)));
                return &mut self.first[index]
                    .as_mut()
                    .expect("inline state was inserted")
                    .1;
            }
            self.rest.reserve(self.expected.max(self.first.len() + 1));
            // Once the inline capacity is exceeded, every lookup uses the map directly.
            for slot in &mut self.first {
                let (key, state) = slot.take().expect("inline states are full");
                self.rest.insert(key, state);
            }
        }
        self.rest
            .entry(key)
            .or_insert_with(|| CacheNodeState::new(key))
    }
    fn values_mut(&mut self) -> impl Iterator<Item = &mut CacheNodeState> {
        self.first
            .iter_mut()
            .flatten()
            .map(|(_, state)| state)
            .chain(self.rest.values_mut())
    }
    fn clear(&mut self) {
        self.first = Default::default();
        self.rest.clear();
    }
    fn retain(&mut self, mut keep: impl FnMut(&BoxKey, &mut CacheNodeState) -> bool) {
        for slot in &mut self.first {
            if slot.as_mut().is_some_and(|(key, state)| !keep(key, state)) {
                *slot = None;
            }
        }
        self.rest.retain(keep);
    }
}

/// Cache generations belong to the document, while computation frames are scoped.
#[derive(Debug)]
pub(crate) struct LayoutCacheContext {
    epoch: CacheEpoch,
    publication_epoch: CacheEpoch,
    source: u64,
    metadata: u64,
    poisoned: bool,
    entries: RefCell<NodeStates>,
    walk_session: u64,
    active_frame: Option<ActiveFrame>,
}

impl Default for LayoutCacheContext {
    fn default() -> Self {
        let epoch = CacheEpoch::default();
        Self {
            epoch,
            publication_epoch: epoch,
            source: 0,
            metadata: 0,
            poisoned: false,
            entries: RefCell::new(NodeStates::default()),
            walk_session: 0,
            active_frame: None,
        }
    }
}

impl Clone for LayoutCacheContext {
    fn clone(&self) -> Self {
        // A document clone has no matching live session or validated cache stamps.
        Self {
            epoch: self.epoch,
            publication_epoch: self.publication_epoch,
            source: self.source,
            metadata: self.metadata,
            poisoned: self.poisoned,
            ..Self::default()
        }
    }
}

impl LayoutCacheContext {
    pub(crate) fn epoch(&self) -> CacheEpoch {
        self.epoch
    }
    pub(crate) fn publication_epoch(&self) -> CacheEpoch {
        self.publication_epoch
    }

    pub(crate) fn configure_nodes(&mut self, expected: usize) {
        self.entries.get_mut().expected = expected;
    }

    pub(crate) fn begin_session(&mut self) {
        self.walk_session = match self.walk_session.checked_add(1) {
            Some(value) => value,
            None => {
                for state in self.entries.get_mut().values_mut() {
                    state.walk_session = 0;
                    state.layout.taffy_child_walks = None;
                }
                1
            }
        };
    }

    pub(crate) fn end_session(&mut self) {
        // Release unfinished traversal buffers at the same lifetime boundary as before.
        for state in self.entries.get_mut().values_mut() {
            state.layout.taffy_child_walks = None;
        }
    }

    pub(crate) fn node_state(
        &self,
        key: BoxKey,
        dirty: bool,
    ) -> RefMut<'_, super::context::NodeLayoutState> {
        let session = self.walk_session;
        RefMut::map(self.entries.borrow_mut(), |entries| {
            let state = entries.get_or_insert(key);
            if state.walk_session != session || dirty {
                state.layout.taffy_child_walks = None;
                state.walk_session = session;
            }
            &mut state.layout
        })
    }

    pub(crate) fn invalidate(&mut self) {
        if self.poisoned {
            return;
        }
        let namespace = self.epoch.namespace;
        if !self.epoch.advance() {
            // No generation remains distinguishable, so keep every read/store disabled.
            self.poisoned = true;
        }
        if self.epoch.namespace != namespace || self.poisoned {
            for state in self.entries.get_mut().values_mut() {
                state.stamp = 0;
                state.context = None;
            }
        }
    }

    pub(crate) fn publish_source(&mut self) {
        self.invalidate();
        self.publication_epoch = self.epoch;
        self.source = self.source.wrapping_add(1);
        self.entries.get_mut().clear();
    }

    pub(crate) fn publish_metadata(&mut self) {
        self.invalidate();
        self.publication_epoch = self.epoch;
        self.metadata = self.metadata.wrapping_add(1);
        // Synthetic identities may be reused after reconstruction; retain only source entries.
        self.entries.get_mut().retain(|key, state| {
            state.context = None;
            state.overrides = 0;
            state.layout.taffy_child_walks = None;
            matches!(key, BoxKey::Source(_))
        });
    }

    pub(crate) fn override_changed(&mut self, key: BoxKey) {
        self.invalidate();
        let state = self.entries.get_mut().get_or_insert(key);
        state.overrides = state.overrides.wrapping_add(1);
        state.context = None;
    }

    #[cfg(test)]
    pub(crate) fn fingerprint(&self, key: BoxKey, view: ViewContext) -> CacheFingerprint {
        let overrides = self
            .entries
            .borrow()
            .get(&key)
            .map_or(0, |state| state.overrides);
        CacheFingerprint::new(view, self.source, self.metadata, overrides)
    }

    #[cfg(test)]
    pub(crate) fn enter(&mut self, key: BoxKey, fingerprint: CacheFingerprint) -> CacheFrame {
        self.enter_with(key, |_| fingerprint)
    }

    pub(crate) fn enter_view(&mut self, key: BoxKey, view: ViewContext) -> CacheFrame {
        let source = self.source;
        let metadata = self.metadata;
        self.enter_with(key, |overrides| {
            CacheFingerprint::new(view, source, metadata, overrides)
        })
    }

    fn enter_with(
        &mut self,
        key: BoxKey,
        fingerprint: impl FnOnce(u64) -> CacheFingerprint,
    ) -> CacheFrame {
        let state = self.entries.get_mut().get_or_insert(key);
        let fingerprint = fingerprint(state.overrides);
        let changed = state.context.is_some_and(|last| last != fingerprint);
        state.context = Some(fingerprint);
        if changed {
            self.invalidate();
            self.entries
                .get_mut()
                .get_mut(&key)
                .expect("entered state exists")
                .context = Some(fingerprint);
        }
        let previous = self.active_frame;
        let depth = previous.map_or(0, |frame| frame.depth + 1);
        self.active_frame = Some(ActiveFrame {
            key,
            epoch: self.epoch,
            depth,
        });
        CacheFrame {
            key,
            epoch: self.epoch,
            depth,
            previous,
        }
    }

    pub(crate) fn leave(&mut self, frame: CacheFrame) {
        let active = self.active_frame.expect("active cache frame");
        assert_eq!(
            active.depth, frame.depth,
            "cache frames leave in reverse order"
        );
        assert_eq!(active.key, frame.key);
        assert_eq!(active.epoch, frame.epoch);
        self.active_frame = frame.previous;
    }

    pub(crate) fn can_read(&self, key: BoxKey) -> bool {
        !self.poisoned
            && self
                .entries
                .borrow()
                .get(&key)
                .is_some_and(|state| state.stamp == self.epoch.value)
    }

    pub(crate) fn can_store(&self, key: BoxKey) -> bool {
        !self.poisoned
            && self
                .active_frame
                .is_some_and(|frame| frame.key == key && frame.epoch == self.epoch)
    }

    pub(crate) fn mark_stored(&mut self, key: BoxKey) {
        assert!(
            self.can_store(key),
            "only a current active computation may stamp its cache"
        );
        self.set_stamp(key, self.epoch.value);
    }

    fn set_stamp(&mut self, key: BoxKey, value: u64) {
        if let Some(state) = self.entries.get_mut().get_mut(&key) {
            state.stamp = value;
        }
    }
}

/// A cache snapshot carries its actual validated generation, never the capture time.
#[derive(Debug)]
pub(crate) struct SavedLayoutCache {
    key: BoxKey,
    cache: LayoutCache,
    epoch: Option<CacheEpoch>,
}

impl LayoutSession<'_> {
    pub(crate) fn take_layout_cache(&mut self, idx: usize) -> SavedLayoutCache {
        self.sync_layout_cache_mutations();
        let key = self.nodes.box_key(idx).expect("saved cache has a box key");
        let epoch = self
            .layout_cache_context
            .can_read(key)
            .then(|| self.layout_cache_context.epoch());
        self.document.layout_cache_context.set_stamp(key, 0);
        SavedLayoutCache {
            key,
            cache: std::mem::replace(&mut self.nodes[idx].cache, LayoutCache::new()),
            epoch,
        }
    }

    pub(crate) fn clone_layout_cache(&self, idx: usize) -> SavedLayoutCache {
        let key = self.nodes.box_key(idx).expect("saved cache has a box key");
        let epoch = (!self.layout_dirty && self.layout_cache_context.can_read(key))
            .then(|| self.layout_cache_context.epoch());
        SavedLayoutCache {
            key,
            cache: self.nodes[idx].cache.clone(),
            epoch,
        }
    }

    pub(crate) fn restore_layout_cache(&mut self, idx: usize, saved: SavedLayoutCache) {
        // A descendant unwind can dirty the document while outer scopes remain active.
        // Discard saved caches without publishing metadata inside those scopes.
        if !self.document.layout_dirty {
            self.sync_layout_cache_mutations();
        }
        let key = self
            .nodes
            .box_key(idx)
            .expect("restored cache has a box key");
        assert_eq!(key, saved.key, "saved cache belongs to the same box");
        if !self.document.layout_dirty
            && !self.layout_cache_context.poisoned
            && saved.epoch == Some(self.layout_cache_context.epoch())
        {
            self.nodes[idx].cache = saved.cache;
            let value = self.layout_cache_context.epoch.value;
            self.document.layout_cache_context.set_stamp(key, value);
        } else {
            self.nodes[idx].cache.clear();
            self.document.layout_cache_context.set_stamp(key, 0);
        }
    }
}

#[cfg(test)]
mod tests;
