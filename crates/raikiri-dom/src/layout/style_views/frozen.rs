//! Own the inputs captured at a block frame's geometry boundary.

use super::*;
use crate::layout::taffy_adapter::ResolveOrZero;
use raikiri_layout_style::{LengthProperty, PercentBasis};
use raikiri_style::ComputedLengthPercentageOrAuto as L;

#[derive(Clone, Debug)]
pub(crate) struct FrozenViewInput {
    prepared: PreparedStyle<'static>,
    baseline: Vec<OwnedPropertyOverride>,
    pub(crate) late: Vec<OwnedPropertyOverride>,
    context: ViewContext,
}
impl FrozenViewInput {
    pub(crate) fn capture(tree: &mut LayoutSession<'_>, idx: usize, context: ViewContext) -> Self {
        tree.assert_style_queries_allowed();
        let views = &tree.style_views;
        let key = tree.nodes.box_key(idx).expect("snapshot key is live");
        let source = views
            .boxes
            .get(&key)
            .expect("snapshot has a prepared input");
        let record = tree
            .document
            .box_style_data
            .get(key)
            .expect("snapshot has a foundation");
        let values = match &record.base {
            LayoutBase::Source(index) => Arc::new(views.cascade.computed[*index].clone()),
            LayoutBase::Synthetic(values) => values.clone(),
        };
        let prepared =
            PreparedStyle::from_shared_data(values, record.metadata, record.prepared.clone())
                .expect("frozen calc bindings describe the captured values");
        let mut baseline = record.pass.clone();
        baseline.extend_from_slice(tree.scoped_overrides.entries(key));
        let owners: Vec<_> = record
            .prepared
            .owners()
            .cloned()
            .chain(
                baseline
                    .iter()
                    .filter_map(|entry| entry.calc_owner().cloned()),
            )
            .collect();
        let mut captured_context = source.context;
        captured_context.claims_policy = context.claims_policy;
        captured_context.block_item_basis = None;
        let mut input = Self {
            prepared,
            baseline,
            late: Vec::new(),
            context: captured_context,
        };
        // Capture already-resolved physical edges before changing the output geometry.
        if input.context.percent_basis != PercentBasis::Taffy {
            let padding = input
                .view()
                .padding()
                .resolve_or_zero(None, crate::layout::dispatch::resolve_calc);
            let border = input
                .view()
                .border()
                .resolve_or_zero(None, crate::layout::dispatch::resolve_calc);
            for (properties, edges) in [
                (
                    [
                        LengthProperty::PaddingTop,
                        LengthProperty::PaddingRight,
                        LengthProperty::PaddingBottom,
                        LengthProperty::PaddingLeft,
                    ],
                    padding,
                ),
                (
                    [
                        LengthProperty::BorderTop,
                        LengthProperty::BorderRight,
                        LengthProperty::BorderBottom,
                        LengthProperty::BorderLeft,
                    ],
                    border,
                ),
            ] {
                for (property, value) in
                    properties
                        .into_iter()
                        .zip([edges.top, edges.right, edges.bottom, edges.left])
                {
                    // Baked edges must outrank every captured pass and scoped entry.
                    input
                        .baseline
                        .push(OwnedPropertyOverride::length(property, L::Px(value), true).unwrap());
                }
            }
            input.context.percent_basis = PercentBasis::Taffy;
        }
        input.context.bridge_projection = context.bridge_projection;
        input.context.layout_wm = context.layout_wm;
        tree.retain_snapshot_calcs(owners);
        input
    }
    pub(crate) fn view(&self) -> LayoutStyleView<'_> {
        LayoutStyleView::with_owned_overrides(
            &self.prepared,
            self.context,
            &self.baseline,
            &self.late,
        )
    }
    pub(crate) fn resolve_padding(&mut self, basis: Option<f32>) {
        // The padding is stored as the style view reads it, in the library's
        // `Rect`, so it is resolved with the library's own trait.
        let padding = taffy::ResolveOrZero::resolve_or_zero(
            self.view().padding(),
            basis,
            crate::layout::dispatch::resolve_calc,
        );
        self.context.resolved_padding = Some(padding);
    }
}
