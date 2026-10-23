use crate::layout::context::LayoutSession;

impl LayoutSession<'_> {
    pub(crate) fn with_resolved_edges<R>(
        &mut self,
        idx: usize,
        basis: f32,
        resolve_border: bool,
        compute: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.sync_layout_cache_mutations();
        self.cache_override_changed(idx);
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
        self.cache_override_changed(idx);
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
}
