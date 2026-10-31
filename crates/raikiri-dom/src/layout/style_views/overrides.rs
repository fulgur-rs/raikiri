use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OverrideToken {
    key: BoxKey,
    length: usize,
    depth: usize,
    checkpoint: usize,
}

#[derive(Default)]
struct ScopedBox {
    entries: Vec<OwnedPropertyOverride>,
    depth: usize,
}

#[derive(Default)]
pub(crate) struct ScopedOverrides {
    boxes: HashMap<BoxKey, ScopedBox>,
    journal: Vec<ScopeUndo>,
}

struct ScopeUndo {
    token: OverrideToken,
    transposed: Vec<crate::layout::dispatch::TransposedSizingUndo>,
    /// Whether the scope left the cache generation alone; see
    /// [`LayoutSession::push_local_overrides`].
    local: bool,
}

impl ScopedOverrides {
    pub(in crate::layout) fn assert_inactive(&self) {
        assert!(
            self.journal.is_empty(),
            "preparation runs outside style scopes"
        );
    }

    pub(super) fn entries(&self, key: BoxKey) -> &[OwnedPropertyOverride] {
        self.boxes.get(&key).map_or(&[], |state| &state.entries)
    }

    #[cfg(test)]
    pub(super) fn depth(&self, key: BoxKey) -> usize {
        self.boxes.get(&key).map_or(0, |state| state.depth)
    }

    pub(super) fn clear_history(&mut self) {
        self.assert_inactive();
        self.boxes.clear();
    }
}

impl LayoutSession<'_> {
    pub(crate) fn with_overrides<R>(
        &mut self,
        entries: Vec<(BoxKey, OwnedPropertyOverride)>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let checkpoint = self.override_checkpoint();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.push_overrides(entries);
            f(self)
        }));
        self.restore_overrides_to(checkpoint);
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// [`LayoutSession::with_overrides`] for a scope that
    /// [`LayoutSession::push_local_overrides`] describes.
    pub(crate) fn with_local_overrides<R>(
        &mut self,
        entries: Vec<(BoxKey, OwnedPropertyOverride)>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let checkpoint = self.override_checkpoint();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.push_local_overrides(entries);
            f(self)
        }));
        self.restore_overrides_to(checkpoint);
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(crate) fn override_checkpoint(&self) -> usize {
        self.scoped_overrides.journal.len()
    }

    pub(crate) fn push_overrides(&mut self, entries: Vec<(BoxKey, OwnedPropertyOverride)>) {
        self.push_scoped_overrides(entries, false);
    }

    /// Push overrides for a probe that must not discard what has been cached
    /// so far.
    ///
    /// [`LayoutSession::push_overrides`] moves the cache generation, on the way
    /// in and again on the way out. Every cached size becomes unreadable and
    /// every box being laid out becomes unable to store its own, so a probe
    /// that overrides a box once for each width its parent tries measures
    /// everything inside the box again for each of them, and a chain of such
    /// boxes takes time exponential in its length.
    ///
    /// A scope pushed here leaves the generation alone. That is sound when the
    /// override changes only the inputs the boxes inside the overridden box are
    /// asked with: a cached size is a function of the box's style and of those
    /// inputs, so it stays right for the inputs it was cached for. What does
    /// depend on the override is the sizes of the overridden box itself, which
    /// the caller keeps apart for as long as the scope lasts (see
    /// `with_saved_probe_caches`).
    pub(crate) fn push_local_overrides(&mut self, entries: Vec<(BoxKey, OwnedPropertyOverride)>) {
        self.push_scoped_overrides(entries, true);
    }

    fn note_override_changed(&mut self, key: BoxKey, local: bool) {
        if local {
            self.document
                .layout_cache_context
                .local_override_changed(key);
        } else {
            self.document.layout_cache_context.override_changed(key);
        }
    }

    fn push_scoped_overrides(
        &mut self,
        entries: Vec<(BoxKey, OwnedPropertyOverride)>,
        local: bool,
    ) {
        self.sync_layout_cache_mutations();
        if entries.is_empty() {
            return;
        }
        let mut indexed = Vec::with_capacity(entries.len());
        let mut counts = HashMap::<BoxKey, usize>::default();
        // Validate every entry before changing any state.
        for (key, entry) in entries {
            let index = self.nodes.layout_index(key).expect("override key is live");
            assert!(
                entry.is_physical_sizing()
                    || entry.is_physical_inset()
                    || entry.is_physical_margin()
                    || entry.is_self_alignment(),
                "scope entry is a physical sizing, inset, margin or self-alignment override"
            );
            *counts.entry(key).or_default() += 1;
            indexed.push((key, index, entry));
        }
        self.scoped_overrides.boxes.reserve(counts.len());
        self.scoped_overrides.journal.reserve(indexed.len());
        for (key, count) in counts {
            self.scoped_overrides
                .boxes
                .entry(key)
                .or_default()
                .entries
                .reserve(count);
        }
        self.document.box_style_data.calc_values.extend(
            indexed
                .iter()
                .filter_map(|(_, _, entry)| entry.calc_owner())
                .cloned(),
        );
        #[cfg(test)]
        let mut applied = 0;
        for (key, index, entry) in indexed {
            let transposed = crate::layout::dispatch::TransposedSizingUndo::prepare(
                &self.context.transposed_blocks,
                &self.context.transposed_style_index,
                index,
            );
            if !transposed.is_empty() {
                // A primitive can outlive both the late override and its frame.
                self.retain_snapshot_calcs(entry.calc_owner().cloned());
            }
            self.note_override_changed(key, local);
            let state = self
                .scoped_overrides
                .boxes
                .get_mut(&key)
                .expect("reserved scope key");
            let token = OverrideToken {
                key,
                length: state.entries.len(),
                depth: state.depth,
                checkpoint: self.scoped_overrides.journal.len(),
            };
            state.depth += 1;
            state.entries.push(entry);
            self.scoped_overrides.journal.push(ScopeUndo {
                token,
                transposed,
                local,
            });
            let undo = self
                .scoped_overrides
                .journal
                .last_mut()
                .expect("registered undo slot");
            for snapshot in &mut undo.transposed {
                snapshot.apply(
                    &mut self.context.transposed_blocks,
                    state.entries.last().expect("registered entry"),
                );
            }
            #[cfg(test)]
            {
                applied += 1;
                super::tests::scoped_override_tests::after_setup_entry(self, index, applied);
            }
        }
    }

    pub(crate) fn restore_overrides_to(&mut self, checkpoint: usize) {
        assert!(
            checkpoint <= self.override_checkpoint(),
            "scope checkpoint is active"
        );
        while self.override_checkpoint() > checkpoint {
            let saved = self
                .scoped_overrides
                .journal
                .pop()
                .expect("active override");
            self.note_override_changed(saved.token.key, saved.local);
            for snapshot in saved.transposed.into_iter().rev() {
                snapshot.restore(&mut self.context.transposed_blocks);
            }
            let state = self
                .scoped_overrides
                .boxes
                .get_mut(&saved.token.key)
                .expect("active scope key");
            state.entries.truncate(saved.token.length);
            state.depth = saved.token.depth;
        }
    }

    pub(crate) fn begin_edge_recording(
        &mut self,
        idx: usize,
        basis: f32,
    ) -> Option<raikiri_layout_style::PercentBasis> {
        self.assert_style_queries_allowed();
        use raikiri_layout_style::PercentBasis;
        let key = self.nodes.box_key(idx).expect("edge scope has a box key");
        let prepared = self.style_views.boxes.get_mut(&key)?;
        let previous = prepared.context.percent_basis;
        if previous == PercentBasis::Taffy {
            prepared.context.percent_basis = PercentBasis::Definite(basis);
        }
        Some(previous)
    }

    pub(crate) fn end_edge_recording(
        &mut self,
        idx: usize,
        previous: Option<raikiri_layout_style::PercentBasis>,
    ) {
        if let Some(previous) = previous {
            let key = self.nodes.box_key(idx).expect("edge scope has a box key");
            self.style_views
                .boxes
                .get_mut(&key)
                .expect("edge scope has a prepared style")
                .context
                .percent_basis = previous;
        }
    }

    #[cfg(test)]
    pub(crate) fn record_scoped_length(
        &mut self,
        idx: usize,
        property: raikiri_layout_style::LengthProperty,
        value: raikiri_style::ComputedLengthPercentageOrAuto,
        resolved: bool,
    ) {
        let key = self.nodes.box_key(idx).expect("override has a box key");
        assert!(
            self.scoped_overrides.depth(key) > 0,
            "temporary values require an active scope"
        );
        let entry = OwnedPropertyOverride::length(property, value, resolved)
            .expect("typed physical property override");
        if let Some(payload) = entry.calc_owner() {
            self.document
                .box_style_data
                .calc_values
                .push(payload.clone());
        }
        self.scoped_overrides
            .boxes
            .get_mut(&key)
            .expect("active scope key")
            .entries
            .push(entry);
    }

    #[cfg(test)]
    pub(crate) fn record_style_overrides(
        &mut self,
        idx: usize,
        entries: Vec<OwnedPropertyOverride>,
    ) -> Option<OverrideToken> {
        self.sync_layout_cache_mutations();
        let key = self.nodes.box_key(idx).expect("override has a box key");
        self.scoped_overrides.journal.reserve(1);
        let state = self.scoped_overrides.boxes.entry(key).or_default();
        state.entries.reserve(entries.len());
        self.document.box_style_data.calc_values.extend(
            entries
                .iter()
                .filter_map(OwnedPropertyOverride::calc_owner)
                .cloned(),
        );
        self.document.layout_cache_context.override_changed(key);
        let token = OverrideToken {
            key,
            length: state.entries.len(),
            depth: state.depth,
            checkpoint: self.scoped_overrides.journal.len(),
        };
        self.scoped_overrides.journal.push(ScopeUndo {
            token,
            transposed: Vec::new(),
            local: false,
        });
        state.depth += 1;
        state.entries.extend(entries);
        Some(token)
    }

    #[cfg(test)]
    pub(crate) fn restore_recorded_style_overrides(&mut self, token: Option<OverrideToken>) {
        let Some(token) = token else { return };
        assert_eq!(
            self.scoped_overrides.journal.last().map(|undo| undo.token),
            Some(token),
            "style scopes restore in reverse order"
        );
        self.restore_overrides_to(token.checkpoint);
    }

    #[cfg(test)]
    pub(crate) fn push_style_overrides(
        &mut self,
        idx: usize,
        entries: Vec<OwnedPropertyOverride>,
    ) -> Option<OverrideToken> {
        self.cache_override_changed(idx);
        self.record_style_overrides(idx, entries)
    }

    #[cfg(test)]
    pub(crate) fn restore_style_overrides(&mut self, token: Option<OverrideToken>) {
        if let Some(token) = token {
            self.document
                .layout_cache_context
                .override_changed(token.key);
        }
        self.restore_recorded_style_overrides(token);
    }

    #[cfg(test)]
    pub(crate) fn style_override_depth(&self, idx: usize) -> usize {
        let key = self.nodes.box_key(idx).expect("scope has a box key");
        self.scoped_overrides.depth(key)
    }

    #[cfg(test)]
    pub(crate) fn with_style_overrides<R>(
        &mut self,
        idx: usize,
        entries: Vec<OwnedPropertyOverride>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let token = self.push_style_overrides(idx, entries);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        self.restore_style_overrides(token);
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}
