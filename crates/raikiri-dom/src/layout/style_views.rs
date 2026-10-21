use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;

use super::box_style_data::{BoxStyleData, LayoutBase, PreparedStamp, recipes};
use crate::{Document, layout::context::LayoutSession, node::arena::BoxKey};
use raikiri_layout_style::{
    LayoutStyleView, OwnedPropertyOverride, PreparedData, PreparedStyle, StyleMetadata, ViewContext,
};
use raikiri_style::{CascadeResult, ComputedValues};
use raikiri_traits::NodeKind;

#[cfg(test)]
pub(crate) mod checked;
mod metadata;
mod overrides;
pub(crate) use overrides::OverrideToken;

pub(crate) struct PreparedBox<'a> {
    prepared: PreparedStyle<'a>,
    scoped: Vec<OwnedPropertyOverride>,
    scopes: Vec<usize>,
    context: ViewContext,
    was_scoped: bool,
}

pub(crate) struct StyleViews<'a> {
    cascade: &'a CascadeResult,
    boxes: HashMap<BoxKey, PreparedBox<'a>>,
    legacy_calcs: HashMap<BoxKey, Vec<Arc<raikiri_layout_style::AlignedCalc>>>,
    size_scopes: Vec<overrides::SizeScope>,
    prepared_publication: Option<super::cache_context::CacheEpoch>,
    prepared_stamp: Option<PreparedStamp>,
    preparation_batch: bool,
    deferred_calc_prefix: Option<usize>,
}

impl<'a> StyleViews<'a> {
    pub(crate) fn new(cascade: &'a CascadeResult) -> Self {
        Self {
            cascade,
            boxes: HashMap::default(),
            legacy_calcs: HashMap::default(),
            size_scopes: Vec::new(),
            prepared_publication: None,
            prepared_stamp: None,
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
        doc.box_style_data.invalidate_metadata();
        self.prepared_stamp = None;
        self.prepare_foundations(doc);
        let mut boxes = HashMap::with_capacity_and_hasher(keys.len(), Default::default());
        for idx in keys {
            let key = doc.nodes.box_key(idx).expect("prepared layout key exists");
            let record = doc
                .box_style_data
                .get(key)
                .expect("box foundation is prepared");
            let prepared = match &record.base {
                LayoutBase::Source(source) => PreparedStyle::from_data(
                    &self.cascade.computed[*source],
                    record.metadata,
                    record.prepared.clone(),
                ),
                LayoutBase::Synthetic(values) => PreparedStyle::from_shared_data(
                    values.clone(),
                    record.metadata,
                    record.prepared.clone(),
                ),
            }
            .expect("foundation data is checked against the current computed values");
            boxes.insert(
                key,
                PreparedBox {
                    prepared,
                    scoped: Vec::new(),
                    scopes: Vec::new(),
                    context: ViewContext::default(),
                    was_scoped: false,
                },
            );
        }
        self.boxes = boxes;
        let stamp = doc.box_style_data.stamp();
        assert!(
            doc.box_style_data.finish_publication(stamp),
            "box preparation epochs are valid"
        );
        self.prepared_stamp = Some(stamp);
        self.prepared_publication = Some(doc.layout_cache_context.publication_epoch());
    }

    fn prepare_foundations(&mut self, doc: &mut Document) {
        // Every live box needs a foundation even when Taffy never queries its view.
        // The range iterator owns indices; source values stay externally borrowed.
        for idx in doc.nodes.layout_keys() {
            let key = doc.nodes.box_key(idx).expect("foundation key is live");
            if doc.box_style_data.get(key).is_none()
                && matches!(key, BoxKey::Source(_))
                && doc.nodes[idx].kind() != NodeKind::Element
            {
                doc.box_style_data.publish_recipe(
                    key,
                    recipes::neutral_source(recipes::BoxFacts {
                        direction: taffy::Direction::Ltr,
                        metadata: StyleMetadata {
                            used_writing_mode: doc.nodes[idx].used_writing_mode,
                            ..StyleMetadata::default()
                        },
                    }),
                );
            }
            let old = doc.box_style_data.take(key);
            let mut record = match old {
                Some(mut record) => {
                    assert_eq!(
                        matches!(record.base, LayoutBase::Synthetic(_)),
                        record.recipe.is_some(),
                        "synthetic bases carry an explicit recipe origin"
                    );
                    if let LayoutBase::Source(source) = record.base {
                        record.metadata = metadata::prepare_metadata(doc, self.cascade, idx);
                        if PreparedStyle::from_data(
                            &self.cascade.computed[source],
                            record.metadata,
                            record.prepared.clone(),
                        )
                        .is_err()
                        {
                            record.prepared =
                                self.prepare_data(doc, key, &self.cascade.computed[source]);
                        }
                    }
                    record
                }
                None => {
                    let BoxKey::Source(source) = key else {
                        panic!(
                            "generated box must publish its explicit recipe before binding: {key:?}"
                        );
                    };
                    assert_ne!(
                        doc.nodes[idx].display(),
                        raikiri_style::property::DisplayValue::Contents,
                        "neutralized source must publish its explicit recipe before binding"
                    );
                    BoxStyleData {
                        base: LayoutBase::Source(source),
                        recipe: None,
                        metadata: metadata::prepare_metadata(doc, self.cascade, idx),
                        pass: Vec::new(),
                        prepared: self.prepare_data(doc, key, &self.cascade.computed[source]),
                        stamp: doc.box_style_data.stamp(),
                    }
                }
            };
            if let Some(pending) = doc.box_style_data.pending.remove(&key) {
                record.pass.extend(pending);
            }
            record.stamp = doc.box_style_data.stamp();
            doc.box_style_data.insert(key, record);
        }
        // Recipe creation may advance epochs while filling neutral source holes.
        let stamp = doc.box_style_data.stamp();
        for idx in doc.nodes.layout_keys() {
            let key = doc.nodes.box_key(idx).unwrap();
            doc.box_style_data.get_mut(key).unwrap().stamp = stamp;
        }
    }

    fn prepare_data(
        &mut self,
        doc: &mut Document,
        key: BoxKey,
        values: &ComputedValues,
    ) -> Arc<PreparedData> {
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
        let mut has_calcs = false;
        let data = PreparedData::prepare_with(values, |_, expression, range| {
            has_calcs = true;
            let key = (expression.px.to_bits(), expression.percent.to_bits(), range);
            if let Some(position) = owner_index.as_ref().and_then(|index| index.get(&key)) {
                return pool[*position].clone();
            }
            let payload = Arc::new(raikiri_layout_style::AlignedCalc::new(expression, range));
            // Retain before a checked wrapper can publish the raw handle.
            doc.box_style_data.calc_values.push(payload.clone());
            owner_index
                .get_or_insert_with(std::collections::HashMap::new)
                .insert(key, pool.len());
            pool.push(payload.clone());
            payload
        })
        .expect("calc owners match source expressions and ranges");
        if !pool.is_empty() {
            self.legacy_calcs.insert(key, pool);
        }
        if has_calcs {
            Arc::new(data)
        } else {
            recipes::empty_prepared_data()
        }
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
            views.prepared_stamp = None;
            self.document.box_style_data.begin_publication();
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self)));
        let old_prefix = self.style_views.as_mut().and_then(|views| {
            views.preparation_batch = false;
            views.deferred_calc_prefix.take()
        });
        match result {
            Ok(Ok(value)) => {
                self.publish_layout_cache_mutations();
                self.prepare_style_views();
                if let Some(length) = old_prefix {
                    self.document.box_style_data.calc_values.drain(..length);
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
            .box_style_data
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
            context.pass_entries = self.document.box_style_data.get(key).unwrap().pass.len();
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
                .box_style_data
                .calc_values
                .extend(values.iter().cloned());
            if !values.is_empty() {
                views.legacy_calcs.insert(key, values);
            }
        } else {
            self.document.box_style_data.calc_values.extend(values);
        }
    }

    pub(crate) fn begin_style_bridge(&mut self) {
        self.assert_preparation_outside_legacy_scopes();
        if let Some(views) = &mut self.style_views {
            views.assert_outside_scopes();
            views.boxes.clear();
            self.document.box_style_data.begin_bridge();
            self.document
                .box_style_data
                .reserve_sources(self.document.nodes.len());
            views.legacy_calcs.clear();
            views.prepared_publication = None;
            views.prepared_stamp = None;
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
            && (views.prepared_publication != Some(publication)
                || !self.document.box_style_data.is_ready())
        {
            assert!(
                self.context.resolved_edge_bases.is_empty()
                    && self.context.saved_size_scopes.is_empty(),
                "preparation runs outside legacy style scopes"
            );
            assert!(
                views.prepared_publication.is_some(),
                "new session requires explicit style binding before cache queries"
            );
            if views.prepared_publication == Some(publication) {
                // Box identity can invalidate its foundation without a source bridge.
                // Publish before the parent's cache lookup can hide that change.
                self.document.layout_cache_context.publish_metadata();
            }
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
        assert!(
            views.prepared_stamp.is_some() && self.document.box_style_data.is_ready(),
            "style queries require completed document and session preparation"
        );
        let prepared = views
            .boxes
            .get(&key)
            .unwrap_or_else(|| panic!("missing prepared style for {key:?}"));
        LayoutStyleView::with_owned_overrides(
            &prepared.prepared,
            prepared.context,
            &self
                .document
                .box_style_data
                .get(key)
                .expect("prepared foundation exists")
                .pass,
            &prepared.scoped,
        )
    }

    pub(crate) fn record_pass_override(&mut self, idx: usize, value: OwnedPropertyOverride) {
        let key = self.nodes.box_key(idx).expect("override has a box key");
        if self.style_views.is_some() {
            if let Some(payload) = value.calc_owner() {
                self.document
                    .box_style_data
                    .calc_values
                    .push(payload.clone());
            }
            if let Some(record) = self.document.box_style_data.get_mut(key) {
                record.pass.push(value);
            } else {
                self.document
                    .box_style_data
                    .pending
                    .entry(key)
                    .or_default()
                    .push(value);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(all(test, feature = "snapshot-serde"))]
mod corpus_tests;
