use crate::layout::context::LayoutSession;

impl LayoutSession<'_> {
    /// Resolve a table at its measured wrapper height, keeping that domain
    /// separate in the caches and restoring the owner when layout unwinds.
    pub(crate) fn with_measured_table_height<R>(
        &mut self,
        idx: usize,
        compute: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let mut saved = self.take_layout_cache(idx);
        let previous = self.context.table_wrapper_height_owner.replace(idx);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(self)));
        self.context.table_wrapper_height_owner = previous;
        // Size probes remain valid in their original domain. Its final
        // layout no longer describes geometry written in the scoped domain.
        saved.forget_final_layout();
        self.restore_layout_cache(idx, saved);
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(crate) fn with_resolved_edges<R>(
        &mut self,
        idx: usize,
        basis: f32,
        resolve_border: bool,
        compute: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.resolved_edge_scope(idx, basis, resolve_border, false, compute)
    }

    /// [`Self::with_resolved_edges`] for a probe that must not discard what has
    /// been cached so far, resolving the padding of `idx` only.
    ///
    /// The scope above moves the cache generation, on the way in and again on
    /// the way out, so a box that a parent lays out once for each size it tries
    /// measures everything inside it again for each of them, and a chain of such
    /// boxes takes time exponential in its length. This one sets only the box
    /// apart (see [`LayoutSession::push_local_overrides`]): what is cached for
    /// the boxes inside it stays right, since it is a function of their styles
    /// and of the inputs they are asked with, and the sizes of the box itself
    /// are not read again once the scope is left.
    pub(crate) fn with_local_resolved_edges<R>(
        &mut self,
        idx: usize,
        basis: f32,
        compute: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.resolved_edge_scope(idx, basis, false, true, compute)
    }

    /// Resolve box edges locally while the caller keeps this box's cache isolated.
    /// Table border cache invalidation remains identical to the global scope.
    pub(crate) fn with_local_resolved_box_edges<R>(
        &mut self,
        idx: usize,
        basis: f32,
        compute: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.resolved_edge_scope(idx, basis, true, true, compute)
    }

    fn resolved_edge_scope<R>(
        &mut self,
        idx: usize,
        basis: f32,
        resolve_border: bool,
        local: bool,
        compute: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.sync_layout_cache_mutations();
        self.cache_edges_changed(idx, local);
        if resolve_border && crate::layout::dispatch::is_table_box(self.nodes[idx].display()) {
            self.context.table_cache.clear();
        }
        let effective_basis = self
            .context
            .resolved_edge_bases
            .iter()
            .rev()
            .find(|(key, _)| *key == idx)
            .map_or(basis, |(_, basis)| *basis);
        self.context
            .resolved_edge_bases
            .push((idx, effective_basis));
        let previous_basis = self.begin_edge_recording(idx, basis);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(self)));
        self.cache_edges_changed(idx, local);
        if resolve_border && crate::layout::dispatch::is_table_box(self.nodes[idx].display()) {
            self.context.table_cache.clear();
        }
        self.context.resolved_edge_bases.pop();
        self.end_edge_recording(idx, previous_basis);
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn cache_edges_changed(&mut self, idx: usize, local: bool) {
        if local {
            self.cache_local_override_changed(idx);
        } else {
            self.cache_override_changed(idx);
        }
    }
}
