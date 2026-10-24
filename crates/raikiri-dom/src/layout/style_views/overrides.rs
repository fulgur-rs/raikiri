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

    pub(crate) fn override_checkpoint(&self) -> usize {
        self.scoped_overrides.journal.len()
    }

    pub(crate) fn push_overrides(&mut self, entries: Vec<(BoxKey, OwnedPropertyOverride)>) {
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
                entry.is_physical_sizing(),
                "scope entry is a physical sizing override"
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
            self.document.layout_cache_context.override_changed(key);
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
            self.scoped_overrides
                .journal
                .push(ScopeUndo { token, transposed });
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
            self.document
                .layout_cache_context
                .override_changed(saved.token.key);
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
