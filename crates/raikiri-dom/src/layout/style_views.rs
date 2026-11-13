use rustc_hash::FxHashMap as HashMap;
use std::cell::OnceCell;
use std::sync::Arc;

use super::box_style_data::{BoxStyleData, LayoutBase, PreparedStamp, recipes};
use crate::{
    Document,
    layout::context::LayoutSession,
    node::{FormattingContext, arena::BoxKey},
};
use raikiri_layout_style::values::{
    Dimension, Direction as LayoutDirection, LengthPercentageAuto, Position,
};
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

    pub(crate) fn height(&self) -> Dimension {
        self.read(|facts| facts.height, |style| style.size().height)
    }

    pub(crate) fn min_height(&self) -> LengthPercentageAuto {
        self.read(|facts| facts.min_height, |style| style.min_size().height)
    }

    pub(crate) fn max_height(&self) -> LengthPercentageAuto {
        self.read(|facts| facts.max_height, |style| style.max_size().height)
    }

    pub(crate) fn margin_top(&self) -> LengthPercentageAuto {
        self.read(|facts| facts.margin.top, |style| style.margin().top)
    }

    pub(crate) fn margin_bottom(&self) -> LengthPercentageAuto {
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
    physical: OnceCell<Box<raikiri_layout_style::BoxStyle>>,
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
    /// The cascade the views read.
    pub(crate) fn cascade(&self) -> &'a CascadeResult {
        self.cascade
    }

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
        let inline_boxes = metadata::inline_boxes(doc, self.cascade);
        for idx in doc.nodes.layout_keys() {
            let key = doc.nodes.box_key(idx).expect("foundation key is live");
            if doc.box_style_data.get(key).is_none()
                && matches!(key, BoxKey::Source(_))
                && doc.nodes[idx].kind() != NodeKind::Element
            {
                doc.box_style_data.publish_recipe(
                    key,
                    recipes::neutral_source(recipes::BoxFacts {
                        direction: LayoutDirection::Ltr,
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
                        let metadata = metadata::prepare_metadata(
                            doc,
                            self.cascade,
                            idx,
                            inline_boxes.get(idx).copied().unwrap_or(false),
                        );
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
                    let metadata = metadata::prepare_metadata(
                        doc,
                        self.cascade,
                        idx,
                        inline_boxes.get(idx).copied().unwrap_or(false),
                    );
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
            let key = (expression.clone(), range);
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

    /// The inline size of a vertical paragraph, including a physical height
    /// keyword retained separately from the horizontal layout fallback.
    pub(crate) fn vertical_ifc_height(&self, idx: usize) -> Dimension {
        let height = self.physical_style(idx).size().height;
        if !height.is_auto()
            || !self.nodes[idx]
                .ifc
                .as_ref()
                .is_some_and(|root| root.writing_mode != shodo::geometry::WritingMode::HorizontalTb)
        {
            return height;
        }
        match self.retained_height_keyword(idx) {
            Some(raikiri_style::ContentSizeKeyword::MinContent) => Dimension::min_content(),
            Some(raikiri_style::ContentSizeKeyword::MaxContent) => Dimension::max_content(),
            Some(raikiri_style::ContentSizeKeyword::FitContent) => Dimension::fit_content(),
            _ => height,
        }
    }

    /// The authored height keyword retained by a source box, unless a pass
    /// or scoped override intentionally replaces its height. This reads
    /// metadata only; it does not resolve a preferred height.
    pub(crate) fn retained_height_keyword(
        &self,
        idx: usize,
    ) -> Option<raikiri_style::ContentSizeKeyword> {
        let key = self.nodes.box_key(idx).expect("live box has a key");
        let record = self.box_style_data.get(key).expect("box style");
        // An auto override also intentionally disables keyword restoration.
        if record
            .pass
            .iter()
            .chain(self.scoped_overrides.entries(key))
            .any(|entry| entry.overrides_length(raikiri_layout_style::LengthProperty::Height))
        {
            return None;
        }
        let LayoutBase::Source(source) = record.base else {
            return None;
        };
        self.style_views.cascade().computed[source].height_keyword
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
        reduce: impl FnOnce(LayoutStyleView<'_>) -> raikiri_layout_style::BoxStyle,
    ) -> raikiri_layout_style::BoxStyle {
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
        if matches!(role, ReadRole::FlexItem) {
            context.flex_item_ratio = self.flex_item_ratio(idx);
            context.held_cross_size = self.held_cross_size(idx, view.with_context(context));
            context.zero_min_main_axis = self
                .zero_min_main_axis(idx)
                .filter(|_| !self.container_is_vertical(idx));
        }
        if matches!(role, ReadRole::GridItem) {
            context.grid_item_ratio = self.grid_item_ratio(idx);
            context.carried_min_width = self.grid_item_carried_min_width(idx);
        }
        if matches!(role, ReadRole::BlockItem) {
            context.block_item_basis = self
                .context
                .block_item_percent_bases
                .last()
                .filter(|&&(container, _)| {
                    self.nodes[idx].layout_position() != Position::Absolute
                        && self.taffy_parent_of(idx) == Some(container)
                })
                .and_then(|&(_, basis)| basis);
        }
        view.with_context(context)
    }

    /// Whether the minimum height that the aspect ratio of the grid item
    /// `idx` carries over to its width needs the grid area to resolve: the
    /// item states no size and no `min-width`, a `min-height`, and a
    /// percentage in that `min-height`, in its padding or in its
    /// `max-width`, and its inline alignment is not `stretch`. A column whose
    /// minimum is `min-content` or `max-content` takes the content
    /// contribution of the item instead, so a grid with such a column leaves
    /// the item to the library.
    fn grid_item_percent_carry(&self, idx: usize, item: &LayoutStyleView<'_>) -> bool {
        use raikiri_layout_style::values::{
            AlignItemsKeyword, ExpandedLengthPercentage, ExpandedLengthPercentageAuto,
        };
        if !item.size().width.is_auto()
            || !item.size().height.is_auto()
            || !item.min_size().width.is_auto()
            || item.min_size().height.is_auto()
        {
            return false;
        }
        let auto_percent = |value: raikiri_layout_style::values::LengthPercentageAuto| match value
            .expand()
        {
            ExpandedLengthPercentageAuto::Auto | ExpandedLengthPercentageAuto::Length(_) => false,
            ExpandedLengthPercentageAuto::Calc(value) => value.has_percentage(),
            _ => true,
        };
        let percent = |edge: raikiri_layout_style::values::LengthPercentage| match edge.expand() {
            ExpandedLengthPercentage::Length(_) => false,
            ExpandedLengthPercentage::Calc(value) => value.has_percentage(),
            _ => true,
        };
        let padding = item.padding();
        let needs_area = auto_percent(item.min_size().height)
            || auto_percent(item.max_size().width)
            || [padding.left, padding.right, padding.top, padding.bottom]
                .into_iter()
                .any(percent);
        needs_area
            && self.grid_item_justify_self(idx).keyword != AlignItemsKeyword::Stretch
            && self.taffy_parent_of(idx).is_some_and(|container| {
                crate::layout::taffy_adapter::columns_have_no_content_minimum(
                    &self.physical_style(container),
                )
            })
    }

    /// The inline alignment of the grid item `idx`: its own `justify-self`,
    /// or the `justify-items` of its container for `auto`.
    fn grid_item_justify_self(&self, idx: usize) -> raikiri_layout_style::values::AlignSelf {
        use raikiri_layout_style::values::AlignSelf;
        self.physical_style(idx)
            .bridge_justify_self()
            .unwrap_or_else(|| {
                self.taffy_parent_of(idx)
                    .map_or(AlignSelf::NORMAL, |parent| {
                        self.physical_style(parent).justify_items()
                    })
            })
    }

    /// Preserve replaced sizes and defer content-based automatic ratio sizes
    /// to the item. The parent otherwise transfers bounds into stated replaced
    /// sizes or treats a non-replaced ratio height as final without its content floor.
    /// A percentage-sized item also resolves its ratio and bounds while its
    /// grid-area basis is known, rather than before the tracks are sized.
    fn grid_item_ratio(&self, idx: usize) -> raikiri_layout_style::GridItemRatio {
        use raikiri_layout_style::GridItemRatio;
        use raikiri_layout_style::values::{
            AlignItemsKeyword, AlignSelf, ExpandedDimension, ExpandedLengthPercentageAuto,
        };
        let item = self.physical_style(idx);
        let vertical_container = matches!(
            self.nodes[idx].formatting_context,
            FormattingContext::Flex | FormattingContext::Grid
        ) && crate::layout::dispatch::block_right_to_left(self, idx)
            .is_some()
            && states_a_size(item.size().width)
            && item.size().height.is_auto();
        if self.container_is_vertical(idx)
            || (self.nodes[idx].used_writing_mode.is_vertical() && !vertical_container)
            || self.nodes[idx].layout_position() == raikiri_layout_style::values::Position::Absolute
            || item.stated_aspect_ratio().is_none()
        {
            return GridItemRatio::Declared;
        }
        if crate::paint_rules::is_replaced_element(self.document, idx) {
            return if item.is_replaced()
                && (!item.size().width.is_auto()
                    || !item.size().height.is_auto()
                    || matches!(
                        item.min_size().width.expand(),
                        ExpandedLengthPercentageAuto::Length(_)
                            | ExpandedLengthPercentageAuto::Percent(_)
                            | ExpandedLengthPercentageAuto::Calc(_)
                    ))
            {
                GridItemRatio::Replaced
            } else {
                GridItemRatio::Declared
            };
        }
        let size = item.size();
        let percentage = |size: Dimension| match size.expand() {
            ExpandedDimension::Percent(_) => true,
            ExpandedDimension::Calc(value) => value.has_percentage(),
            _ => false,
        };
        let percentage_sized = percentage(size.width) || percentage(size.height);
        let auto_with_minimum = size.width.is_auto()
            && size.height.is_auto()
            && matches!(
                item.min_size().width.expand(),
                ExpandedLengthPercentageAuto::Length(_)
                    | ExpandedLengthPercentageAuto::Percent(_)
                    | ExpandedLengthPercentageAuto::Calc(_)
            );
        let measured = matches!(
            self.nodes[idx].formatting_context,
            FormattingContext::Block | FormattingContext::Inline | FormattingContext::Grid
        ) && (percentage_sized || auto_with_minimum);
        // Any box measures itself for a minimum that the area resolves.
        if !vertical_container && !measured && !self.grid_item_percent_carry(idx, &item) {
            return GridItemRatio::Declared;
        }
        let alignment = item.bridge_align_self().unwrap_or_else(|| {
            self.taffy_parent_of(idx)
                .map_or(AlignSelf::NORMAL, |parent| {
                    self.physical_style(parent).align_items()
                })
        });
        let alignment = if alignment.keyword == AlignItemsKeyword::Normal {
            AlignSelf::START
        } else {
            alignment
        };
        let justify = self.grid_item_justify_self(idx);
        let justify = if justify.keyword == AlignItemsKeyword::Normal {
            AlignSelf::START
        } else {
            justify
        };
        GridItemRatio::MeasuredByItem {
            align_self: alignment,
            justify_self: justify,
        }
    }

    /// The minimum width, in the box `box-sizing` names, that the aspect ratio
    /// of the grid item `idx` carries over from the length its `min-height`
    /// states, when the item states no size and no `min-width` and its inline
    /// alignment is not `stretch`.
    ///
    /// The library fills an `auto` minimum with the padding and border before
    /// it carries minimums across the ratio, so a `min-height` never becomes a
    /// minimum width, and it stretches a `normal` item to the width of its
    /// area, from which the ratio then gives the height. Chromium takes the
    /// width the ratio relates to the `min-height` past a narrower area:
    /// `aspect-ratio: 1; min-height: 100px` in a 50px column is 100px wide,
    /// and in a 200px column 200px by 200px. An item aligned to the start, center or end
    /// takes its width from the ratio too, and from a height that `min-height`
    /// holds above a shorter row it stretches to: `align-self: stretch` in a
    /// 60px row is 100px by 100px. An item that asks for `stretch` inline
    /// keeps the width of its area, and a `max-width` the item states caps the
    /// carried width.
    /// Percentages that only the grid area resolves are left to the item,
    /// which measures itself once the area is known (`grid_item_percent_carry`).
    fn grid_item_carried_min_width(&self, idx: usize) -> Option<f32> {
        use raikiri_layout_style::values::{
            AlignItemsKeyword, BoxSizing, ExpandedLengthPercentage, ExpandedLengthPercentageAuto,
        };
        let item = self.physical_style(idx);
        let ratio = item.stated_aspect_ratio()?;
        if self.container_is_vertical(idx)
            || self.nodes[idx].used_writing_mode.is_vertical()
            || self.nodes[idx].layout_position() == raikiri_layout_style::values::Position::Absolute
            || crate::paint_rules::is_replaced_element(self.document, idx)
            || !item.size().width.is_auto()
            || !item.size().height.is_auto()
            || !item.min_size().width.is_auto()
        {
            return None;
        }
        let ExpandedLengthPercentageAuto::Length(min_height) = item.min_size().height.expand()
        else {
            return None;
        };
        let justify = self.grid_item_justify_self(idx);
        if justify.keyword == AlignItemsKeyword::Stretch {
            return None;
        }
        // The padding and border, which only lengths give before the width of
        // the grid area is known: percentages leave the item to the library.
        let length = |edge: raikiri_layout_style::values::LengthPercentage| match edge.expand() {
            ExpandedLengthPercentage::Length(length) => Some(length),
            _ => None,
        };
        let (padding, border) = (item.padding(), item.border());
        let edges = |a, b, c, d| Some(length(a)? + length(b)? + length(c)? + length(d)?);
        let edges_x = edges(padding.left, padding.right, border.left, border.right)?;
        let edges_y = edges(padding.top, padding.bottom, border.top, border.bottom)?;
        let content_box = item.box_sizing() == BoxSizing::ContentBox;
        // The ratio relates the content box of a box sized by its content box
        // and of an `auto <ratio>`, else the border box.
        let width = if content_box || self.nodes[idx].ratio_in_content_box {
            let content_height = if content_box {
                min_height
            } else {
                (min_height - edges_y).max(0.0)
            };
            let content_width = content_height * ratio;
            if content_box {
                content_width
            } else {
                content_width + edges_x
            }
        } else {
            min_height.max(edges_y) * ratio
        };
        // A `max-width` caps the carried width, and one that only the width
        // of the grid area resolves leaves the item to the library.
        let width = match item.max_size().width.expand() {
            ExpandedLengthPercentageAuto::Length(max) => width.min(max),
            ExpandedLengthPercentageAuto::Auto => width,
            _ => return None,
        };
        Some(width.max(0.0))
    }

    /// The main axis of the flex item `idx` when its automatic minimum size
    /// is zero along it and the view reports an `auto` minimum there as zero,
    /// in the axes of the container: the width for a row, the height for a
    /// column, whichever physical axis the container's writing mode turns them
    /// into.
    ///
    /// The content size suggestion of an item with size containment along the
    /// main axis is not its contained size: Chromium shrinks it below
    /// `contain-intrinsic-size` like an item without content, down to its
    /// padding and border.
    ///
    /// A scroll container with an aspect ratio has no automatic minimum
    /// either. The library would carry the minimum of the
    /// other axis across the ratio onto its `auto` one and keep the item from
    /// shrinking below that, which Chromium does not.
    pub(crate) fn zero_min_main_axis(&self, idx: usize) -> Option<raikiri_layout_style::Axis> {
        use raikiri_layout_style::Axis;
        let contained = self.nodes[idx].size_containment;
        let item = self.physical_style(idx);
        let overflow = item.overflow();
        let scrolling_ratio = item.stated_aspect_ratio().is_some()
            && (overflow.x.is_scroll_container() || overflow.y.is_scroll_container());
        if (!contained.width && !contained.height && !scrolling_ratio)
            || self.nodes[idx].layout_position() == Position::Absolute
        {
            return None;
        }
        let container = self.taffy_parent_of(idx)?;
        if self.nodes[container].formatting_context != FormattingContext::Flex {
            return None;
        }
        let row = self.physical_style(container).flex_direction_is_row();
        let main_is_width = row != self.container_is_vertical(idx);
        let (contained, scrolls) = if main_is_width {
            (contained.width, overflow.x.is_scroll_container())
        } else {
            (contained.height, overflow.y.is_scroll_container())
        };
        let zero = contained || (scrolling_ratio && scrolls);
        zero.then_some(if row { Axis::X } else { Axis::Y })
    }

    /// The limits on the cross size of the flex item `idx` that its stated cross
    /// size holds, where `item` is the view of the item the flex algorithm reads.
    ///
    /// The flex algorithm carries a `min-*` or `max-*` size of the main axis
    /// across the aspect ratio into a limit on the cross size of an item that has
    /// none of its own, and clamps the cross size the item states with it. A
    /// cross size the item states is the one it has whatever the limits of the
    /// main axis are: a `max-width` of 30px on a box that states its height of
    /// 50px and has a ratio of 1 leaves the box 50px high. Only a cross size that
    /// is `auto` follows the ratio.
    ///
    /// A limit the item has on the cross axis itself is left as it is, so that it
    /// clamps the cross size it states. A minimum is held in place only when the
    /// item has no maximum on that axis, as a held minimum would win over it.
    fn held_cross_size(
        &self,
        idx: usize,
        item: raikiri_layout_style::LayoutStyleView<'_>,
    ) -> Option<raikiri_layout_style::HeldCrossSize> {
        use raikiri_layout_style::values::ExpandedLengthPercentageAuto;
        use raikiri_layout_style::{Axis, HeldCrossSize};
        let ratio = item
            .aspect_ratio()
            .filter(|_| !self.container_is_vertical(idx))?;
        let is_row = self
            .taffy_parent_of(idx)
            .is_none_or(|container| self.physical_style(container).flex_direction_is_row());
        let (size, min, max) = (item.size(), item.min_size(), item.max_size());
        let (cross, main_min, main_max, cross_min, cross_max, axis) = if is_row {
            (
                size.height,
                min.width,
                max.width,
                min.height,
                max.height,
                Axis::Y,
            )
        } else {
            (
                size.width,
                min.height,
                max.height,
                min.width,
                max.width,
                Axis::X,
            )
        };
        if !states_a_size(cross) {
            return None;
        }
        // Limits of its own on the cross axis clamp the cross size the item
        // states before the ratio gives it a main size, which is why the main
        // size and the width of a container that wraps the item follow the
        // clamped cross size.
        let size = stated_cross_within_limits(cross, cross_min, cross_max);
        // A held minimum would win over a maximum the item states on the cross
        // axis, unless it is the clamped cross size.
        let held_min = !main_min.is_auto()
            && is_no_limit(cross_min)
            && (size.is_some() || cross_max.is_auto());
        let held_max = !main_max.is_auto() && cross_max.is_auto();
        // The minimum of the main axis that the flex algorithm carries across
        // the ratio from the cross minimum would win over a maximum the item
        // states on the main axis, when it is larger: the maximum is the
        // minimum then. A smaller one is left alone, as it is not the minimum
        // that the item shrinks to.
        let carried_cross_min = if held_min {
            size
        } else {
            match cross_min.expand() {
                ExpandedLengthPercentageAuto::Length(min) if min > 0.0 => Some(min),
                _ => None,
            }
        };
        // The automatic minimum of a scroll container is zero, and the item
        // shrinks to it whatever the cross minimum is.
        let overflow = item.overflow();
        let scrolls_on_main = if is_row { overflow.x } else { overflow.y }.is_scroll_container();
        let main_floor = carried_cross_min
            .filter(|_| !scrolls_on_main)
            .and_then(|cross| {
                let carried = if is_row { cross * ratio } else { cross / ratio };
                match (main_min.is_auto(), main_max.expand()) {
                    (true, ExpandedLengthPercentageAuto::Length(max)) if carried > max => Some(max),
                    _ => None,
                }
            });
        let held = HeldCrossSize {
            axis,
            min: held_min,
            max: held_max,
            size,
            main_min: main_floor,
        };
        (held.min || held.max || held.size.is_some()).then_some(held)
    }

    /// The aspect ratio the flex algorithm is told about the flex item `idx`.
    ///
    /// An item that states its main size and leaves its cross size `auto` takes
    /// the cross size from the main size it ends up with after flexing. The
    /// library derives it from the main size the item states, which the item
    /// does not keep when it grows or shrinks, or when its flex basis is not
    /// `auto`, so the library is told no ratio and the item derives the cross
    /// size itself when it is laid out at the flexed main size. A paragraph item
    /// of a column that states no height, and no `max-height` to cap it, takes
    /// its main size from its lines and its ratio until the flex algorithm
    /// settles another one, and its width follows that one in the same way.
    ///
    /// An item with a flex basis of `content` that states its cross size gets
    /// its flex base size from that size through the ratio, even when it states
    /// its main size too and the ratio has nothing left to do for its own size.
    pub(crate) fn flex_item_ratio(&self, idx: usize) -> raikiri_layout_style::FlexItemRatio {
        use raikiri_layout_style::FlexItemRatio;
        use raikiri_layout_style::values::ExpandedDimension;
        let item = self.physical_style(idx);
        if self.container_is_vertical(idx) || item.stated_aspect_ratio().is_none() {
            return FlexItemRatio::Declared;
        }
        let size = item.size();
        let is_row = self
            .taffy_parent_of(idx)
            .is_none_or(|container| self.physical_style(container).flex_direction_is_row());
        let (main, cross) = if is_row {
            (size.width, size.height)
        } else {
            (size.height, size.width)
        };
        // The `100%` width of an `<svg>` that fills the room its parent offers
        // stands in for an `auto` one, so the svg states no width of its own.
        let width_stands_in = self.nodes[idx].svg_width_follows_available;
        // The main size of a paragraph item of a column is what its lines and
        // its ratio give, until the flex algorithm settles another one: it
        // flexes, or a `max-height` caps it. The cross size follows the main
        // size it settled for, except when the cap is what settled it: there
        // the library derives the cross size through the ratio, the same cap
        // carried over to the width included.
        let paragraph_main_flexes = self.flex_main_axis_is_width(idx) == Some(false)
            && self.nodes[idx].formatting_context == FormattingContext::Inline
            && crate::layout::dispatch::block_right_to_left(self, idx).is_none()
            && item.max_size().height.is_auto();
        let main_is_stated =
            (states_a_size(main) && (!is_row || !width_stands_in)) || paragraph_main_flexes;
        // Hiding the ratio leaves the cross size to the box itself, which
        // only blocks, flex and grid containers, paragraphs and replaced boxes
        // derive from the main size they are measured at. A flex container on
        // vertical writing axes derives its height from its width, so it does
        // that for the cross size of an item of a row only; as an item of a
        // column it keeps the ratio, which gives the width from the height.
        let derives_cross_from_ratio = match self.nodes[idx].formatting_context {
            FormattingContext::Block | FormattingContext::Grid | FormattingContext::Inline => true,
            FormattingContext::Flex => {
                is_row || crate::layout::dispatch::block_right_to_left(self, idx).is_none()
            }
            _ => false,
        };
        if item.aspect_ratio().is_some()
            && cross.is_auto()
            && main_is_stated
            && derives_cross_from_ratio
        {
            FlexItemRatio::CrossFollowsMain
        } else if matches!(item.flex_basis().expand(), ExpandedDimension::Content)
            && states_a_fixed_size(cross)
        {
            FlexItemRatio::BasisFromCross
        } else {
            FlexItemRatio::Declared
        }
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

/// Whether `size` is a length, a percentage or a `calc()`: not `auto` and no
/// sizing keyword.
fn states_a_size(size: raikiri_layout_style::values::Dimension) -> bool {
    use raikiri_layout_style::values::ExpandedDimension;
    matches!(
        size.expand(),
        ExpandedDimension::Length(_) | ExpandedDimension::Percent(_) | ExpandedDimension::Calc(_)
    )
}

/// Whether `limit` is `auto` or zero: a minimum or maximum that clamps nothing.
fn is_no_limit(limit: raikiri_layout_style::values::LengthPercentageAuto) -> bool {
    use raikiri_layout_style::values::ExpandedLengthPercentageAuto;
    match limit.expand() {
        ExpandedLengthPercentageAuto::Auto => true,
        ExpandedLengthPercentageAuto::Length(length) => length <= 0.0,
        _ => false,
    }
}

/// The cross size a flex item states, within the limits it states on the same
/// axis, when it states some and every one of them is a length: the cross size
/// the item has before its ratio gives it a main size. `None` where it states
/// no limit (the stated size is then the size it has) or a limit that waits
/// for a container to resolve.
fn stated_cross_within_limits(
    cross: raikiri_layout_style::values::Dimension,
    min: raikiri_layout_style::values::LengthPercentageAuto,
    max: raikiri_layout_style::values::LengthPercentageAuto,
) -> Option<f32> {
    use raikiri_layout_style::values::{ExpandedDimension, ExpandedLengthPercentageAuto};
    let ExpandedDimension::Length(size) = cross.expand() else {
        return None;
    };
    let min = match min.expand() {
        ExpandedLengthPercentageAuto::Auto => None,
        ExpandedLengthPercentageAuto::Length(min) => Some(min),
        _ => return None,
    };
    let max = match max.expand() {
        ExpandedLengthPercentageAuto::Auto => None,
        ExpandedLengthPercentageAuto::Length(max) => Some(max),
        _ => return None,
    };
    // A minimum of zero limits nothing: the bridge reports one for a flex item
    // whose automatic minimum is not content based.
    if max.is_none() && min.is_none_or(|min| min <= 0.0) {
        return None;
    }
    Some(
        size.min(max.unwrap_or(f32::INFINITY))
            .max(min.unwrap_or(0.0)),
    )
}

/// Whether `size` is a length or a `calc()` of lengths only: a size that
/// does not wait for the size of a container to resolve.
fn states_a_fixed_size(size: raikiri_layout_style::values::Dimension) -> bool {
    use raikiri_layout_style::values::ExpandedDimension;
    match size.expand() {
        ExpandedDimension::Length(_) => true,
        ExpandedDimension::Calc(calc) => !calc.has_percentage(),
        _ => false,
    }
}
