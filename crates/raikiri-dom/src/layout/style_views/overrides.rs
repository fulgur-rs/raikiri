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
    pub(crate) fn with_resolved_edges<R>(
        &mut self,
        idx: usize,
        basis: f32,
        resolve_border: bool,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        use raikiri_layout_style::PercentBasis;
        let key = self.nodes.box_key(idx).expect("edge scope has a box key");
        let previous_basis = self.style_views.as_mut().map(|views| {
            let prepared = views
                .boxes
                .get_mut(&key)
                .expect("edge scope has a prepared style");
            let previous = prepared.context.percent_basis;
            // A surrounding edge scope has already turned these edges into pixels.
            if previous == PercentBasis::Taffy {
                prepared.context.percent_basis = PercentBasis::Definite(basis);
            }
            prepared.was_scoped = true;
            previous
        });
        let style = self
            .layout_style_for_key_mut(idx)
            .expect("edge scope has a retained style");
        let previous_edges = (style.padding, style.border);
        let resolve = |edge| {
            taffy::LengthPercentage::length(
                crate::layout::used_style_length_percentage(edge, basis).unwrap_or(0.0),
            )
        };
        style.padding = style.padding.map(resolve);
        if resolve_border {
            style.border = style.border.map(resolve);
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        let style = self
            .layout_style_for_key_mut(idx)
            .expect("edge scope has a retained style");
        (style.padding, style.border) = previous_edges;
        if let Some(previous) = previous_basis {
            self.style_views
                .as_mut()
                .expect("edge scope has a session")
                .boxes
                .get_mut(&key)
                .expect("edge scope has a prepared style")
                .context
                .percent_basis = previous;
        }
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(crate) fn size_scope_checkpoint(&self) -> usize {
        self.style_views
            .as_ref()
            .map_or(0, |views| views.size_scopes.len())
    }

    pub(crate) fn save_size_scope(&mut self, idx: usize) -> Option<usize> {
        self.style_views.as_ref()?;
        let old = self
            .layout_style_for_key(idx)
            .expect("size scope has a retained style");
        let (size, min, max) = (old.size, old.min_size, old.max_size);
        let token = self.push_style_overrides(idx, Vec::new());
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
        self.restore_style_overrides(saved.token);
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
        let prepared = views
            .boxes
            .get_mut(&key)
            .expect("override has a prepared style");
        assert!(
            !prepared.scopes.is_empty(),
            "temporary values require an active scope"
        );
        prepared.scoped.push(
            OwnedPropertyOverride::length(property, value, resolved)
                .expect("typed physical property override"),
        );
    }

    pub(crate) fn push_style_overrides(
        &mut self,
        idx: usize,
        entries: Vec<OwnedPropertyOverride>,
    ) -> Option<OverrideToken> {
        let key = self.nodes.box_key(idx).expect("override has a box key");
        let views = self.style_views.as_mut()?;
        let prepared = views
            .boxes
            .get_mut(&key)
            .expect("override has a prepared style");
        let token = OverrideToken {
            key,
            length: prepared.scoped.len(),
            depth: prepared.scopes.len(),
        };
        prepared.scopes.push(token.length);
        prepared.was_scoped = true;
        prepared.scoped.extend(entries);
        Some(token)
    }

    pub(crate) fn restore_style_overrides(&mut self, token: Option<OverrideToken>) {
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

    pub(crate) fn style_override_depth(&self, idx: usize) -> usize {
        let key = self.nodes.box_key(idx).expect("scope has a box key");
        self.style_views
            .as_ref()
            .and_then(|views| views.boxes.get(&key))
            .map_or(0, |prepared| prepared.scopes.len())
    }

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
