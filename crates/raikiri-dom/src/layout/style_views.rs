use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;

use crate::{Document, layout::context::LayoutSession, node::arena::BoxKey};
use raikiri_layout_style::{
    LayoutStyleView, OwnedPropertyOverride, PreparedStyle, StyleMetadata, ViewContext,
};
use raikiri_style::{CascadeResult, ComputedValues};
use raikiri_traits::NodeKind;

#[cfg(test)]
pub(crate) mod checked;
mod metadata;
pub(crate) use metadata::neutral_initial_values;
mod overrides;
pub(crate) use overrides::OverrideToken;

pub(crate) struct PreparedBox<'a> {
    prepared: PreparedStyle<'a>,
    pass: Vec<OwnedPropertyOverride>,
    scoped: Vec<OwnedPropertyOverride>,
    scopes: Vec<usize>,
    context: ViewContext,
    was_scoped: bool,
}

pub(crate) struct StyleViews<'a> {
    cascade: &'a CascadeResult,
    boxes: HashMap<BoxKey, PreparedBox<'a>>,
    pending: HashMap<BoxKey, Vec<OwnedPropertyOverride>>,
    legacy_calcs: HashMap<BoxKey, Vec<Arc<raikiri_layout_style::AlignedCalc>>>,
    size_scopes: Vec<overrides::SizeScope>,
    prepared_publication: Option<super::cache_context::CacheEpoch>,
    preparation_batch: bool,
    deferred_calc_prefix: Option<usize>,
}

impl<'a> StyleViews<'a> {
    pub(crate) fn new(cascade: &'a CascadeResult) -> Self {
        Self {
            cascade,
            boxes: HashMap::default(),
            pending: HashMap::default(),
            legacy_calcs: HashMap::default(),
            size_scopes: Vec::new(),
            prepared_publication: None,
            preparation_batch: false,
            deferred_calc_prefix: None,
        }
    }

    fn assert_outside_scopes(&self) {
        assert!(
            self.size_scopes.is_empty() && self.boxes.values().all(|item| item.scopes.is_empty()),
            "preparation runs outside style scopes"
        );
    }

    fn prepare(&mut self, doc: &mut Document) {
        use taffy::TraversePartialTree;
        let mut keys = Vec::new();
        for idx in doc.nodes.layout_keys() {
            if doc.nodes[idx].kind() == NodeKind::Element
                && matches!(
                    doc.nodes[idx].taffy_display(),
                    taffy::Display::Flex | taffy::Display::Grid
                )
            {
                if keys.capacity() == 0 {
                    keys.reserve(doc.nodes.len());
                }
                keys.push(idx);
                // Taffy queries child style before excluding hidden or out-of-flow items.
                keys.extend(doc.child_ids(taffy::NodeId::from(idx)).map(usize::from));
            }
        }
        keys.sort_unstable();
        keys.dedup();
        self.prepare_keys(doc, keys);
    }

    fn prepare_keys(&mut self, doc: &mut Document, keys: Vec<usize>) {
        self.assert_outside_scopes();
        if keys.len() == self.boxes.len()
            && keys.iter().all(|&idx| {
                matches!(doc.nodes.box_key(idx), Some(key @ BoxKey::Source(_)) if self.boxes.contains_key(&key))
            })
        {
            // Source computed values stay borrowed from the same cascade between bridges.
            // A metadata-only publication does not need another registry or calc binding table.
            for idx in keys {
                let metadata = metadata::prepare_metadata(doc, self.cascade, idx);
                let item = self.boxes.get_mut(&BoxKey::Source(idx)).unwrap();
                item.prepared.set_metadata(metadata);
                item.context = ViewContext::default();
                item.was_scoped = false;
            }
            self.prepared_publication = Some(doc.layout_cache_context.publication_epoch());
            return;
        }
        let mut previous = std::mem::replace(
            &mut self.boxes,
            HashMap::with_capacity_and_hasher(keys.len(), Default::default()),
        );
        for idx in keys {
            let key = doc.nodes.box_key(idx).expect("prepared layout key exists");
            let metadata = metadata::prepare_metadata(doc, self.cascade, idx);
            let (old, mut pass) = previous
                .remove(&key)
                .map(|item| (Some(item.prepared), item.pass))
                .unwrap_or_default();
            if let Some(pending) = self.pending.remove(&key) {
                pass.extend(pending);
            }
            let prepared = if let Some(mut old) = old.filter(|_| matches!(key, BoxKey::Source(_))) {
                old.set_metadata(metadata);
                old
            } else {
                self.prepare_one(doc, idx, metadata)
            };
            self.boxes.insert(
                key,
                PreparedBox {
                    prepared,
                    pass,
                    scoped: Vec::new(),
                    scopes: Vec::new(),
                    context: ViewContext::default(),
                    was_scoped: false,
                },
            );
        }
        // A source box may become a Flex/Grid item after a later topology change.
        for (key, old) in previous {
            if matches!(key, BoxKey::Source(_)) && !old.pass.is_empty() {
                // Overrides enter either a prepared box or pending, so these keys are disjoint.
                self.pending.insert(key, old.pass);
            }
        }
        self.prepared_publication = Some(doc.layout_cache_context.publication_epoch());
    }

    fn prepare_one(
        &mut self,
        doc: &mut Document,
        idx: usize,
        metadata: StyleMetadata,
    ) -> PreparedStyle<'a> {
        use raikiri_style::property::DisplayValue;
        let key = doc.nodes.box_key(idx).expect("prepared layout key exists");
        let node = &doc.nodes[idx];
        let neutral = !matches!(key, BoxKey::Source(_))
            || node.kind() != NodeKind::Element
            || node.display() == DisplayValue::Contents;
        let mut pool = self.legacy_calcs.remove(&key).unwrap_or_default();
        // Expressions come from CSS; keep randomized hashing for their exact bit keys.
        let mut owner_index = (!pool.is_empty()).then(|| {
            let mut index = std::collections::HashMap::with_capacity(pool.len());
            for (position, payload) in pool.iter().enumerate() {
                let value = payload.expression();
                index
                    .entry((
                        value.px.to_bits(),
                        value.percent.to_bits(),
                        payload.non_negative(),
                    ))
                    .or_insert(position);
            }
            index
        });
        let shared = if neutral {
            let owner = if key == BoxKey::InitialContainingBlock {
                doc.synthetic_style_values
                    .get(&key)
                    .expect("ICB recipe is published with the page dimensions")
                    .clone()
            } else if matches!(key, BoxKey::Source(_)) && node.kind() != NodeKind::Element {
                metadata::neutral_source_owner()
            } else {
                Arc::new(metadata::neutral_values(doc, self.cascade, idx))
            };
            doc.synthetic_style_values.insert(key, owner.clone());
            Some(owner)
        } else {
            None
        };
        let mut owner = |_, expression: raikiri_style::property::CalcLengthPercentage, range| {
            let key = (expression.px.to_bits(), expression.percent.to_bits(), range);
            if let Some(position) = owner_index.as_ref().and_then(|index| index.get(&key)) {
                return pool[*position].clone();
            }
            let payload = Arc::new(raikiri_layout_style::AlignedCalc::new(expression, range));
            // Retain owners before the prepared binding can publish a raw handle.
            doc.retained_layout_styles.calc_values.push(payload.clone());
            owner_index
                .get_or_insert_with(std::collections::HashMap::new)
                .insert(key, pool.len());
            pool.push(payload.clone());
            payload
        };
        let prepared = if let Some(values) = shared {
            PreparedStyle::prepare_shared_with(values, metadata, &mut owner)
        } else {
            PreparedStyle::prepare_with(&self.cascade.computed[idx], metadata, &mut owner)
        }
        .expect("adapter calc owners match the prepared computed values");
        if !pool.is_empty() {
            self.legacy_calcs.insert(key, pool);
        }
        prepared
    }
}

impl LayoutSession<'_> {
    #[inline]
    pub(crate) fn assert_style_queries_allowed(&self) {
        assert!(
            !self
                .style_views
                .as_ref()
                .is_some_and(|views| views.preparation_batch),
            "style queries run after the preparation batch"
        );
    }

    pub(crate) fn with_style_preparation_batch<R, E>(
        &mut self,
        run: impl FnOnce(&mut Self) -> Result<R, E>,
    ) -> Result<R, E> {
        self.assert_style_queries_allowed();
        self.assert_preparation_outside_legacy_scopes();
        if let Some(views) = &mut self.style_views {
            views.assert_outside_scopes();
            views.preparation_batch = true;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self)));
        let old_prefix = self.style_views.as_mut().and_then(|views| {
            views.preparation_batch = false;
            views.deferred_calc_prefix.take()
        });
        match result {
            Ok(Ok(value)) => {
                self.sync_layout_cache_mutations();
                if let Some(length) = old_prefix {
                    self.document
                        .retained_layout_styles
                        .calc_values
                        .drain(..length);
                }
                Ok(value)
            }
            Ok(Err(error)) => Err(error),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(crate) fn finish_style_bridge(&mut self, old_calc_len: usize) {
        if let Some(views) = &mut self.style_views
            && views.preparation_batch
        {
            views.deferred_calc_prefix = Some(old_calc_len);
            return;
        }
        self.prepare_style_views();
        self.document
            .retained_layout_styles
            .calc_values
            .drain(..old_calc_len);
    }

    #[cfg(test)]
    pub(crate) fn checked_style(
        &self,
        idx: usize,
        role: checked::ReadRole,
    ) -> checked::CheckedStyle<'_> {
        self.assert_style_queries_allowed();
        let key = self.nodes.box_key(idx).expect("style query has a box key");
        let mut context = checked::ReadContext::new(key, role);
        let view = self.style_views.as_ref().map(|views| {
            let prepared = views
                .boxes
                .get(&key)
                .unwrap_or_else(|| panic!("missing prepared style for {key:?}"));
            context.context = prepared.context;
            context.pass_entries = prepared.pass.len();
            context.scope_depth = prepared.scopes.len();
            context.was_scoped = prepared.was_scoped;
            self.style_view(idx)
        });
        checked::CheckedStyle::new(
            self.layout_style_for_key(idx)
                .expect("style query has a retained style"),
            view,
            context,
            &self.document.style_view_comparisons,
        )
    }

    pub(crate) fn propagate_viewport_overflow(&mut self, cascade: &CascadeResult, body: usize) {
        use raikiri_layout_style::Axis;
        use raikiri_style::property::OverflowValue;
        if super::body_overflow::propagate_body_overflow_to_viewport(self.document, cascade, body) {
            for (axis, original) in [
                (Axis::X, cascade.computed[body].overflow.x),
                (Axis::Y, cascade.computed[body].overflow.y),
            ] {
                if original != OverflowValue::Visible {
                    self.record_pass_override(
                        body,
                        OwnedPropertyOverride::overflow(axis, OverflowValue::Visible),
                    );
                }
            }
        }
    }

    pub(crate) fn retain_bridge_calcs(
        &mut self,
        key: BoxKey,
        values: Vec<Arc<raikiri_layout_style::AlignedCalc>>,
    ) {
        if let Some(views) = &mut self.style_views {
            self.document
                .retained_layout_styles
                .calc_values
                .extend(values.iter().cloned());
            if !values.is_empty() {
                views.legacy_calcs.insert(key, values);
            }
        } else {
            self.document
                .retained_layout_styles
                .calc_values
                .extend(values);
        }
    }

    pub(crate) fn begin_style_bridge(&mut self) {
        if let Some(views) = &mut self.style_views {
            views.assert_outside_scopes();
            views.boxes.clear();
            views.pending.clear();
            views.legacy_calcs.clear();
            views.prepared_publication = None;
        }
    }
    fn assert_preparation_outside_legacy_scopes(&self) {
        assert!(
            self.context.resolved_edge_bases.is_empty()
                && self.context.saved_size_scopes.is_empty(),
            "preparation runs outside legacy style scopes"
        );
    }

    pub(crate) fn prepare_style_views(&mut self) {
        self.assert_style_queries_allowed();
        self.assert_preparation_outside_legacy_scopes();
        if let Some(views) = &mut self.style_views {
            views.prepare(self.document);
        }
    }

    pub(crate) fn refresh_style_views(&mut self) {
        self.assert_style_queries_allowed();
        let publication = self.document.layout_cache_context.publication_epoch();
        if let Some(views) = &mut self.style_views
            && views.prepared_publication != Some(publication)
        {
            assert!(
                self.context.resolved_edge_bases.is_empty()
                    && self.context.saved_size_scopes.is_empty(),
                "preparation runs outside legacy style scopes"
            );
            views.prepare(self.document);
        }
    }

    pub(crate) fn has_style_view(&self, idx: usize) -> bool {
        self.nodes.box_key(idx).is_some_and(|key| {
            self.style_views
                .as_ref()
                .is_some_and(|views| views.boxes.contains_key(&key))
        })
    }

    #[inline]
    pub(crate) fn style_view(&self, idx: usize) -> LayoutStyleView<'_> {
        self.assert_style_queries_allowed();
        let key = self.nodes.box_key(idx).expect("style query has a box key");
        let views = self
            .style_views
            .as_ref()
            .expect("checked session borrows its cascade");
        let prepared = views
            .boxes
            .get(&key)
            .unwrap_or_else(|| panic!("missing prepared style for {key:?}"));
        LayoutStyleView::with_owned_overrides(
            &prepared.prepared,
            prepared.context,
            &prepared.pass,
            &prepared.scoped,
        )
    }

    pub(crate) fn record_pass_override(&mut self, idx: usize, value: OwnedPropertyOverride) {
        let key = self.nodes.box_key(idx).expect("override has a box key");
        if let Some(views) = &mut self.style_views {
            if let Some(payload) = value.calc_owner() {
                self.document
                    .retained_layout_styles
                    .calc_values
                    .push(payload.clone());
            }
            if let Some(prepared) = views.boxes.get_mut(&key) {
                prepared.pass.push(value);
            } else {
                views.pending.entry(key).or_default().push(value);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(all(test, feature = "snapshot-serde"))]
mod corpus_tests;
