use rustc_hash::FxHashMap as HashMap;
use std::cell::OnceCell;
use std::sync::Arc;

use super::box_style_data::{BoxStyleData, LayoutBase, PreparedStamp, recipes};
use crate::{Document, layout::context::LayoutSession, node::arena::BoxKey};
use raikiri_layout_style::{
    LayoutStyleView, OwnedPropertyOverride, PercentBasis, PreparedData, PreparedStyle,
    StyleMetadata, ViewContext,
};
use raikiri_style::{CascadeResult, ComputedValues};
use raikiri_traits::NodeKind;

pub(crate) mod finished;
pub(crate) mod frozen;
pub(super) mod metadata;
mod overrides;
pub(crate) use overrides::ScopedOverrides;

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub(crate) enum ReadRole {
    FlexContainer,
    FlexItem,
    GridContainer,
    GridItem,
    CoreContainer,
    BlockContainer,
    BlockItem,
    Oof,
    /// The box's own style with no role context: no transposed block view,
    /// no containing-block claims and no block-item percentage basis.
    Physical,
    /// The paginator's reads of a laid-out box.
    Pagination,
}

/// A box's style along the axis pagination breaks the flow on, read one fact
/// at a time. See [`LayoutSession::block_axis`].
pub(crate) struct BlockAxis<'s> {
    session: &'s LayoutSession<'s>,
    idx: usize,
    snapshot: Option<&'s crate::layout::block_axis_style::BlockAxisStyle>,
    /// The view the facts are read from, built by the first one that needs it.
    view: std::cell::OnceCell<LayoutStyleView<'s>>,
}

impl BlockAxis<'_> {
    fn read<T>(
        &self,
        snapshot: impl FnOnce(&crate::layout::block_axis_style::BlockAxisStyle) -> T,
        view: impl FnOnce(&LayoutStyleView<'_>) -> T,
    ) -> T {
        match self.snapshot {
            Some(facts) => snapshot(facts),
            None => view(
                self.view
                    .get_or_init(|| self.session.pagination_style(self.idx)),
            ),
        }
    }

    pub(crate) fn height(&self) -> taffy::Dimension {
        self.read(|facts| facts.height, |style| style.size().height)
    }

    pub(crate) fn min_height(&self) -> taffy::LengthPercentageAuto {
        self.read(|facts| facts.min_height, |style| style.min_size().height)
    }

    pub(crate) fn max_height(&self) -> taffy::LengthPercentageAuto {
        self.read(|facts| facts.max_height, |style| style.max_size().height)
    }

    pub(crate) fn margin_top(&self) -> taffy::LengthPercentageAuto {
        self.read(|facts| facts.margin.top, |style| style.margin().top)
    }

    pub(crate) fn margin_bottom(&self) -> taffy::LengthPercentageAuto {
        self.read(|facts| facts.margin.bottom, |style| style.margin().bottom)
    }

    pub(crate) fn aspect_ratio(&self) -> Option<f32> {
        self.read(|facts| facts.aspect_ratio, |style| style.aspect_ratio())
    }
}

pub(crate) struct PreparedBox<'a> {
    prepared: PreparedStyle<'a>,
    context: ViewContext,
    /// The reduced physical style of the box, filled by the first read that no
    /// override and no percentage basis touches. It dies with the preparation.
    physical: OnceCell<Box<crate::LayoutStyle>>,
}

pub(crate) struct StyleViews<'a> {
    cascade: &'a CascadeResult,
    boxes: HashMap<BoxKey, PreparedBox<'a>>,
    // Primitives may retain snapshot handles after their frame has been popped.
    // Keep unique payloads across bridge replacement for the rest of the session.
    snapshot_calcs: HashMap<usize, Arc<raikiri_layout_style::AlignedCalc>>,
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
            snapshot_calcs: HashMap::default(),
            prepared_publication: None,
            prepared_stamp: None,
            preparation_batch: false,
            deferred_calc_prefix: None,
        }
    }

    fn prepare(&mut self, doc: &mut Document, scoped: &ScopedOverrides) {
        let keys = doc.nodes.layout_keys().collect();
        self.prepare_keys(doc, keys, scoped);
    }

    fn prepare_keys(&mut self, doc: &mut Document, keys: Vec<usize>, scoped: &ScopedOverrides) {
        scoped.assert_inactive();
        doc.layout_cache_context.publish_metadata();
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
                    context: ViewContext::default(),
                    physical: OnceCell::new(),
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
                        metadata: metadata::node_metadata(
                            &doc.nodes[idx],
                            raikiri_layout_style::StyleFlags::EMPTY,
                        ),
                    }),
                );
            }
            if let Some(record) = doc.box_style_data.get(key)
                && record.recipe.is_some()
                && let Some(recipe) = recipes::refresh(doc, self.cascade, key, idx, record)
            {
                doc.box_style_data.refresh_recipe(key, recipe);
            }
            match doc.box_style_data.get(key) {
                Some(record) => {
                    assert_eq!(
                        matches!(record.base, LayoutBase::Synthetic(_)),
                        record.recipe.is_some(),
                        "synthetic bases carry an explicit recipe origin"
                    );
                    if let LayoutBase::Source(source) = &record.base {
                        let source = *source;
                        let metadata = metadata::prepare_metadata(doc, self.cascade, idx);
                        let valid = PreparedStyle::from_data(
                            &self.cascade.computed[source],
                            metadata,
                            record.prepared.clone(),
                        )
                        .is_ok();
                        if valid {
                            doc.box_style_data.get_mut(key).unwrap().metadata = metadata;
                        } else {
                            let prepared = self.prepare_data(doc, &self.cascade.computed[source]);
                            let pass =
                                std::mem::take(&mut doc.box_style_data.get_mut(key).unwrap().pass);
                            // Replacing through the store retires the previous binding data.
                            doc.box_style_data.insert(
                                key,
                                BoxStyleData {
                                    base: LayoutBase::Source(source),
                                    recipe: None,
                                    metadata,
                                    pass,
                                    prepared,
                                    stamp: doc.box_style_data.stamp(),
                                },
                            );
                        }
                    }
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
                    let metadata = metadata::prepare_metadata(doc, self.cascade, idx);
                    let prepared = self.prepare_data(doc, &self.cascade.computed[source]);
                    doc.box_style_data.insert(
                        key,
                        BoxStyleData {
                            base: LayoutBase::Source(source),
                            recipe: None,
                            metadata,
                            pass: Vec::new(),
                            prepared,
                            stamp: doc.box_style_data.stamp(),
                        },
                    );
                }
            }
            if let Some(pending) = doc.box_style_data.pending.remove(&key) {
                doc.box_style_data
                    .get_mut(key)
                    .unwrap()
                    .pass
                    .extend(pending);
            }
        }
        // Recipe creation may advance epochs while filling neutral source holes.
        let stamp = doc.box_style_data.stamp();
        for idx in doc.nodes.layout_keys() {
            let key = doc.nodes.box_key(idx).unwrap();
            doc.box_style_data.get_mut(key).unwrap().stamp = stamp;
        }
    }

    fn prepare_data(&mut self, doc: &mut Document, values: &ComputedValues) -> Arc<PreparedData> {
        // Equal expressions of one box share an owner. They come from CSS, so
        // keep randomized hashing for their exact bit keys.
        let mut owners = std::collections::HashMap::new();
        let mut has_calcs = false;
        let data = PreparedData::prepare_with(values, |_, expression, range| {
            has_calcs = true;
            let key = (expression.px.to_bits(), expression.percent.to_bits(), range);
            owners
                .entry(key)
                .or_insert_with(|| {
                    let payload =
                        Arc::new(raikiri_layout_style::AlignedCalc::new(expression, range));
                    // Retain before a view can publish the raw handle.
                    doc.box_style_data.calc_values.push(payload.clone());
                    payload
                })
                .clone()
        })
        .expect("calc owners match source expressions and ranges");
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
        self.assert_native_style_access_allowed();
        self.assert_outside_preparation_batch();
    }

    #[inline]
    fn assert_outside_preparation_batch(&self) {
        assert!(
            !self.style_views.preparation_batch,
            "style queries run after the preparation batch"
        );
    }

    pub(crate) fn assert_native_style_access_allowed(&self) {
        assert!(
            !self.context.pagination_on_transposed_axes,
            "native style queries and updates are forbidden during transposed pagination"
        );
    }

    pub(crate) fn with_style_preparation_batch<R, E>(
        &mut self,
        run: impl FnOnce(&mut Self) -> Result<R, E>,
    ) -> Result<R, E> {
        self.assert_style_queries_allowed();
        self.assert_preparation_outside_legacy_scopes();
        self.style_views.preparation_batch = true;
        self.style_views.prepared_stamp = None;
        self.document.box_style_data.begin_publication();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self)));
        self.style_views.preparation_batch = false;
        let old_prefix = self.style_views.deferred_calc_prefix.take();
        match result {
            Ok(Ok(value)) => {
                self.publish_layout_cache_mutations();
                self.prepare_style_views();
                if let Some(length) = old_prefix {
                    self.retire_calc_prefix(length);
                }
                Ok(value)
            }
            Ok(Err(error)) => Err(error),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(crate) fn finish_style_bridge(&mut self, old_calc_len: usize) {
        if self.style_views.preparation_batch {
            self.style_views.deferred_calc_prefix = Some(old_calc_len);
            return;
        }
        self.prepare_style_views();
        self.retire_calc_prefix(old_calc_len);
    }

    fn retire_calc_prefix(&mut self, length: usize) {
        // Append after the retiring prefix to preserve escaped snapshot handles.
        self.document
            .box_style_data
            .calc_values
            .extend(self.style_views.snapshot_calcs.values().cloned());
        self.document.box_style_data.calc_values.drain(..length);
    }

    pub(crate) fn retain_snapshot_calcs(
        &mut self,
        owners: impl IntoIterator<Item = Arc<raikiri_layout_style::AlignedCalc>>,
    ) {
        let views = &mut self.style_views;
        for owner in owners {
            let identity = Arc::as_ptr(&owner) as usize;
            if let std::collections::hash_map::Entry::Vacant(slot) =
                views.snapshot_calcs.entry(identity)
            {
                slot.insert(owner.clone());
                self.document.box_style_data.calc_values.push(owner);
            }
        }
    }

    /// The box's own style with no role context.
    #[inline]
    pub(crate) fn physical_style(&self, idx: usize) -> LayoutStyleView<'_> {
        self.query_style(idx, ReadRole::Physical)
    }

    /// `reduce` applied to the box's physical style.
    ///
    /// Layout asks for the same reduction of a box on every pass over it. While
    /// nothing overrides the box and its percentage basis is Taffy's, the
    /// reduction depends only on the prepared style, so the first one is kept
    /// until the next preparation.
    pub(crate) fn reduced_physical_style(
        &self,
        idx: usize,
        reduce: impl FnOnce(LayoutStyleView<'_>) -> crate::LayoutStyle,
    ) -> crate::LayoutStyle {
        let Some(prepared) = self.unmodified_box(idx) else {
            return reduce(self.physical_style(idx));
        };
        self.assert_style_queries_allowed();
        (**prepared
            .physical
            .get_or_init(|| Box::new(reduce(self.physical_style(idx)))))
        .clone()
    }

    /// The prepared box when its physical style is the prepared style alone:
    /// no pass or scoped override and no replaced percentage basis.
    fn unmodified_box(&self, idx: usize) -> Option<&PreparedBox<'_>> {
        let key = self.nodes.box_key(idx)?;
        let prepared = self.style_views.boxes.get(&key)?;
        (prepared.context.percent_basis == PercentBasis::Taffy
            && self.document.box_style_data.get(key)?.pass.is_empty()
            && self.scoped_overrides.entries(key).is_empty())
        .then_some(prepared)
    }

    /// The paginator's view of the laid-out box `idx`.
    #[inline]
    pub(crate) fn pagination_style(&self, idx: usize) -> LayoutStyleView<'_> {
        self.query_style(idx, ReadRole::Pagination)
    }

    pub(crate) fn query_style(&self, idx: usize, role: ReadRole) -> LayoutStyleView<'_> {
        use raikiri_layout_style::ClaimsPolicy;
        if matches!(role, ReadRole::Pagination) {
            return self.pagination_view(idx);
        }
        self.assert_style_queries_allowed();
        let transposed = match role {
            ReadRole::BlockContainer | ReadRole::FlexContainer | ReadRole::GridContainer => {
                self.transposed_container_view(idx)
            }
            ReadRole::BlockItem | ReadRole::FlexItem | ReadRole::GridItem | ReadRole::Oof => {
                self.transposed_child_view(idx)
            }
            _ => None,
        };
        if let Some(view) = transposed {
            return view;
        }
        let view = self.prepared_style_view(idx);
        let mut context = view.context();
        context.claims_policy = if matches!(role, ReadRole::CoreContainer) {
            ClaimsPolicy::Prepared
        } else {
            ClaimsPolicy::Positioning
        };
        if matches!(role, ReadRole::BlockItem) {
            context.block_item_basis = self
                .context
                .block_item_percent_bases
                .last()
                .filter(|&&(container, _)| {
                    self.nodes[idx].taffy_position() != taffy::Position::Absolute
                        && self.taffy_parent_of(idx) == Some(container)
                })
                .and_then(|&(_, basis)| basis);
        }
        view.with_context(context)
    }

    /// The physical view of a laid-out box as the paginator reads it.
    ///
    /// Pagination of a vertical root runs on transposed axes, but boxes keep
    /// their styles on physical axes there. The facts that depend on the axis
    /// are read through [`Self::block_axis`]; the others (float, clear,
    /// box sizing, grid tracks, alignment) are the same on both. Only reads
    /// are allowed: updating styles or preparing boxes stays forbidden
    /// meanwhile.
    fn pagination_view(&self, idx: usize) -> LayoutStyleView<'_> {
        self.assert_outside_preparation_batch();
        let view = self.prepared_style_view(idx);
        let mut context = view.context();
        context.claims_policy = raikiri_layout_style::ClaimsPolicy::Positioning;
        view.with_context(context)
    }

    /// The style of box `idx` along the axis pagination breaks the flow on.
    ///
    /// While the document is paginated on transposed axes these are the
    /// facts of the snapshot taken when it was transposed; otherwise they are
    /// the physical ones of the pagination view. Each fact is read when it is
    /// asked for, because most callers need one or two of them.
    pub(crate) fn block_axis(&self, idx: usize) -> BlockAxis<'_> {
        BlockAxis {
            session: self,
            idx,
            snapshot: self.document.pagination_block_styles.get(idx),
            view: std::cell::OnceCell::new(),
        }
    }

    pub(crate) fn propagate_viewport_overflow(&mut self, cascade: &CascadeResult, body: usize) {
        use raikiri_layout_style::Axis;
        use raikiri_style::property::OverflowValue;
        if super::body_overflow::body_overflow_propagates_to_viewport(self.document, cascade, body)
        {
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

    pub(crate) fn begin_style_bridge(&mut self) {
        self.assert_native_style_access_allowed();
        self.assert_preparation_outside_legacy_scopes();
        let views = &mut self.style_views;
        views.boxes.clear();
        self.document.box_style_data.begin_bridge();
        self.document
            .box_style_data
            .reserve_sources(self.document.nodes.len());
        views.prepared_publication = None;
        views.prepared_stamp = None;
    }
    fn assert_preparation_outside_legacy_scopes(&self) {
        self.scoped_overrides.assert_inactive();
        assert!(
            self.context.resolved_edge_bases.is_empty(),
            "preparation runs outside legacy style scopes"
        );
    }

    pub(crate) fn prepare_style_views(&mut self) {
        self.assert_style_queries_allowed();
        self.assert_preparation_outside_legacy_scopes();
        self.scoped_overrides.clear_history();
        self.style_views
            .prepare(self.document, &self.scoped_overrides);
    }

    pub(crate) fn refresh_style_views(&mut self) {
        self.assert_style_queries_allowed();
        let publication = self.document.layout_cache_context.publication_epoch();
        let views = &mut self.style_views;
        if views.prepared_publication != Some(publication)
            || !self.document.box_style_data.is_ready()
        {
            self.scoped_overrides.assert_inactive();
            assert!(
                self.context.resolved_edge_bases.is_empty(),
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
            self.scoped_overrides.clear_history();
            views.prepare(self.document, &self.scoped_overrides);
        }
    }

    #[cfg(test)]
    pub(crate) fn has_style_view(&self, idx: usize) -> bool {
        self.nodes
            .box_key(idx)
            .is_some_and(|key| self.style_views.boxes.contains_key(&key))
    }

    #[inline]
    pub(crate) fn style_view(&self, idx: usize) -> LayoutStyleView<'_> {
        self.assert_style_queries_allowed();
        self.prepared_style_view(idx)
    }

    #[inline]
    fn prepared_style_view(&self, idx: usize) -> LayoutStyleView<'_> {
        let key = self.nodes.box_key(idx).expect("style query has a box key");
        let views = &self.style_views;
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
            self.scoped_overrides.entries(key),
        )
    }

    pub(crate) fn record_pass_override(&mut self, idx: usize, value: OwnedPropertyOverride) {
        // Publish before a parent cache lookup can hide the child's new input.
        self.cache_override_changed(idx);
        let key = self.nodes.box_key(idx).expect("override has a box key");
        if let Some(payload) = value.calc_owner() {
            self.document
                .box_style_data
                .calc_values
                .push(payload.clone());
        }
        if let Some(record) = self.document.box_style_data.get_mut(key) {
            value.install(&mut record.pass);
        } else {
            value.install(self.document.box_style_data.pending.entry(key).or_default());
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
