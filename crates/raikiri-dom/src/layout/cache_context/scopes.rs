use crate::layout::context::LayoutSession;

impl LayoutSession<'_> {
    pub(crate) fn with_resolved_edges<R>(
        &mut self,
        idx: usize,
        basis: f32,
        resolve_border: bool,
        compute: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.cache_override_changed(idx);
        if resolve_border && crate::taffy_impl::is_table_box(self.nodes[idx].display()) {
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
        #[cfg(test)]
        let previous_basis = self.begin_edge_recording(idx, basis);
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
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(self)));
        self.cache_override_changed(idx);
        if resolve_border && crate::taffy_impl::is_table_box(self.nodes[idx].display()) {
            self.context.table_cache.clear();
        }
        let style = self
            .layout_style_for_key_mut(idx)
            .expect("edge scope has a retained style");
        (style.padding, style.border) = previous_edges;
        self.context.resolved_edge_bases.pop();
        #[cfg(test)]
        self.end_edge_recording(idx, previous_basis);
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}
