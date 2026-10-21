use super::*;

#[derive(Clone, Copy)]
pub(crate) struct OverrideToken {
    key: BoxKey,
    length: usize,
    depth: usize,
}

pub(super) struct SizeScope {
    idx: usize,
    token: Option<OverrideToken>,
    size: taffy::Size<taffy::Dimension>,
    min: taffy::Size<taffy::LengthPercentageAuto>,
    max: taffy::Size<taffy::LengthPercentageAuto>,
}

impl LayoutSession<'_> {
    pub(crate) fn begin_edge_recording(
        &mut self,
        idx: usize,
        basis: f32,
    ) -> Option<raikiri_layout_style::PercentBasis> {
        use raikiri_layout_style::PercentBasis;
        let key = self.nodes.box_key(idx).expect("edge scope has a box key");
        self.style_views.as_mut().and_then(|views| {
            let prepared = views.boxes.get_mut(&key)?;
            let previous = prepared.context.percent_basis;
            if previous == PercentBasis::Taffy {
                prepared.context.percent_basis = PercentBasis::Definite(basis);
            }
            prepared.was_scoped = true;
            Some(previous)
        })
    }

    pub(crate) fn end_edge_recording(
        &mut self,
        idx: usize,
        previous: Option<raikiri_layout_style::PercentBasis>,
    ) {
        if let Some(previous) = previous {
            let key = self.nodes.box_key(idx).expect("edge scope has a box key");
            self.style_views
                .as_mut()
                .expect("edge scope has a session")
                .boxes
                .get_mut(&key)
                .expect("edge scope has a prepared style")
                .context
                .percent_basis = previous;
        }
    }

    pub(crate) fn size_scope_checkpoint(&self) -> usize {
        self.style_views
            .as_ref()
            .map_or(0, |views| views.size_scopes.len())
    }

    pub(crate) fn save_size_scope(&mut self, idx: usize) -> Option<usize> {
        self.sync_layout_cache_mutations();
        if !self.has_style_view(idx) {
            return None;
        }
        let old = self
            .layout_style_for_key(idx)
            .expect("size scope has a retained style");
        let (size, min, max) = (old.size, old.min_size, old.max_size);
        let token = self.record_style_overrides(idx, Vec::new());
        let views = self.style_views.as_mut().expect("checked session");
        let marker = views.size_scopes.len();
        views.size_scopes.push(SizeScope {
            idx,
            token,
            size,
            min,
            max,
        });
        Some(marker)
    }

    pub(crate) fn restore_size_scope(&mut self, marker: Option<usize>) {
        let Some(marker) = marker else {
            return;
        };
        let views = self.style_views.as_mut().expect("size scope has a session");
        assert_eq!(
            views.size_scopes.len(),
            marker + 1,
            "size scopes restore in reverse order"
        );
        let saved = views.size_scopes.pop().expect("active size scope");
        self.restore_recorded_style_overrides(saved.token);
        let style = self
            .layout_style_for_key_mut(saved.idx)
            .expect("size scope has a retained style");
        (style.size, style.min_size, style.max_size) = (saved.size, saved.min, saved.max);
    }

    pub(crate) fn restore_size_scopes_to(&mut self, checkpoint: usize) {
        while self.size_scope_checkpoint() > checkpoint {
            self.restore_size_scope(Some(self.size_scope_checkpoint() - 1));
        }
    }

    pub(crate) fn record_scoped_length(
        &mut self,
        idx: usize,
        property: raikiri_layout_style::LengthProperty,
        value: raikiri_style::ComputedLengthPercentageOrAuto,
        resolved: bool,
    ) {
        let key = self.nodes.box_key(idx).expect("override has a box key");
        let Some(views) = &mut self.style_views else {
            return;
        };
        let Some(prepared) = views.boxes.get_mut(&key) else {
            return;
        };
        assert!(
            !prepared.scopes.is_empty(),
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
        prepared.scoped.push(entry);
    }

    pub(crate) fn record_style_overrides(
        &mut self,
        idx: usize,
        entries: Vec<OwnedPropertyOverride>,
    ) -> Option<OverrideToken> {
        self.sync_layout_cache_mutations();
        let key = self.nodes.box_key(idx).expect("override has a box key");
        let views = self.style_views.as_mut()?;
        let prepared = views.boxes.get_mut(&key)?;
        let token = OverrideToken {
            key,
            length: prepared.scoped.len(),
            depth: prepared.scopes.len(),
        };
        prepared.scopes.push(token.length);
        prepared.was_scoped = true;
        self.document.box_style_data.calc_values.extend(
            entries
                .iter()
                .filter_map(OwnedPropertyOverride::calc_owner)
                .cloned(),
        );
        prepared.scoped.extend(entries);
        Some(token)
    }

    pub(crate) fn restore_recorded_style_overrides(&mut self, token: Option<OverrideToken>) {
        let Some(token) = token else {
            return;
        };
        let prepared = self
            .style_views
            .as_mut()
            .expect("scope has a session")
            .boxes
            .get_mut(&token.key)
            .expect("scope has a prepared style");
        assert_eq!(
            prepared.scopes.len(),
            token.depth + 1,
            "style scopes restore in reverse order"
        );
        assert_eq!(prepared.scopes.pop(), Some(token.length));
        prepared.scoped.truncate(token.length);
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
        self.style_views
            .as_ref()
            .and_then(|views| views.boxes.get(&key))
            .map_or(0, |prepared| prepared.scopes.len())
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
