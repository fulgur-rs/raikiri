use std::cell::RefMut;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use crate::document::Document;
use crate::fragment::FragmentationContext;
use crate::layout::LayoutWarn;
use crate::node::arena::BoxKey;

// The next task routes layout entrypoints through these types.
#[derive(Debug, Default)]
pub(crate) struct LayoutContext {
    pub(crate) layout_warnings: Vec<LayoutWarn>,
    pub(crate) table_layout_error: Option<String>,
    pub(super) table_cache: super::table::cache::TableCache,
    pub(crate) ifc_width_probe: bool,
    pub(crate) fragmentation_stack: Vec<FragmentationContext>,
    pub(crate) vertical_oof_containing_blocks: Vec<(usize, f32)>,
    pub(crate) orthogonal_block_in_layout: Option<usize>,
    pub(crate) transposed_style_index: crate::taffy_impl::TransposedStyleIndex,
    pub(crate) transposed_frame_serial: u64,
    pub(crate) transposed_blocks: Vec<crate::taffy_impl::TransposedBlock>,
    pub(crate) block_item_percent_bases: Vec<(usize, Option<f32>)>,
    pub(crate) flex_item_percent_bases: Vec<(usize, taffy::Size<Option<f32>>)>,
    pub(crate) size_contained_measure: Vec<usize>,
    pub(crate) resolved_edge_bases: Vec<(usize, f32)>,
    /// The boxes being measured without their intrinsic `min-*` and `max-*`
    /// sizes (see `taffy_impl::content_limits`).
    pub(crate) content_limit_probes: Vec<usize>,
    /// The flex container whose intrinsic widths are being measured, whose
    /// items are bounded by their flex base sizes meanwhile (see
    /// `with_flex_intrinsic_probe` in [`crate::taffy_impl`]).
    pub(crate) flex_intrinsic_probe: Option<usize>,
    /// Definite block sizes retained as percentage bases for the direct
    /// children of boxes being measured for intrinsic height limits.
    pub(crate) content_limit_percentage_height_bases: Vec<(usize, f32)>,
    /// The width or height each box measured for its keyword sizes, while the
    /// outermost box laying out with them runs, and how many such layouts are
    /// running (see `taffy_impl::content_limits`).
    pub(crate) content_limit_measures: HashMap<crate::taffy_impl::content_limits::MeasureKey, f32>,
    pub(crate) content_limit_depth: usize,
    /// The final layout inputs observed for out-of-flow candidates during a
    /// preliminary positioning pass, before their intrinsic limits are
    /// installed.
    pub(crate) oof_content_limit_inputs: Vec<HashMap<usize, taffy::LayoutInput>>,
    /// Nested candidate wrappers skip their own prepasses while an outer
    /// candidate wrapper is gathering unconstrained positioning inputs.
    pub(crate) oof_content_limit_prepass_depth: usize,
    pub(crate) grid_layout_info: HashMap<usize, taffy::DetailedLayoutInfo<smol_str::SmolStr>>,
    pub(crate) taffy_hoisted_children: HashMap<usize, Vec<taffy::NodeId>>,
    pub(crate) pagination_on_transposed_axes: bool,
    pub(crate) text_group: TextGroupScope,
    /// The least space a line leaves unfilled so far in the group being
    /// measured.
    pub(crate) text_group_shortest: Option<f32>,
    /// The next child layout is in the group's block formatting context
    /// although it gets no block context (a vertical block child), so it
    /// keeps the group.
    pub(crate) keep_text_group: bool,
    /// The layout being entered is such a child: its size cache is bypassed,
    /// which would answer without laying its lines out in the group.
    pub(crate) group_child_uncached: bool,
}

/// The `text-group-align` group the lines being laid out belong to: the
/// lines of the block formatting context under the box that declares it
/// (CSS Text 4, group alignment).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum TextGroupScope {
    /// No group reaches the lines being laid out.
    #[default]
    None,
    /// Measuring: the lines laid out report the space they leave unfilled,
    /// and the shortest of it is the group's padding.
    Probing,
    /// Inside a group being measured or skipped, below a box that roots a
    /// group of its own: that group is neither measured nor applied, since
    /// its lines are not part of the outer group, and laying it out once
    /// keeps nested groups linear.
    Skipping,
    /// Laying out for real: every line takes the group's padding.
    Applying {
        align: shodo::style::TextGroupAlign,
        padding: f32,
    },
}

#[derive(Debug)]
pub(crate) struct NodeLayoutState {
    pub(crate) box_key: BoxKey,
    pub(crate) taffy_child_walks: Option<Vec<taffy::NodeId>>,
}

pub(crate) struct LayoutSession<'doc> {
    pub(crate) document: &'doc mut Document,
    pub(crate) context: LayoutContext,
    pub(crate) style_views: Option<super::style_views::StyleViews<'doc>>,
    pub(crate) scoped_overrides: super::style_views::ScopedOverrides,
}

impl<'doc> LayoutSession<'doc> {
    #[cfg(test)]
    pub(crate) fn new(document: &'doc mut Document) -> Self {
        document.layout_cache_context.begin_session();
        Self {
            document,
            context: LayoutContext::default(),
            style_views: None,
            scoped_overrides: Default::default(),
        }
    }

    pub(crate) fn with_cascade(
        document: &'doc mut Document,
        cascade: &'doc raikiri_style::CascadeResult,
    ) -> Self {
        document.layout_cache_context.begin_session();
        document.layout_cache_context.publish_source();
        document.box_style_data.begin_session();
        Self {
            document,
            context: LayoutContext::default(),
            style_views: Some(super::style_views::StyleViews::new(cascade)),
            scoped_overrides: Default::default(),
        }
    }

    /// Publish pending document mutations before any parent cache lookup.
    pub(crate) fn sync_layout_cache_mutations(&mut self) {
        self.assert_style_queries_allowed();
        self.publish_layout_cache_mutations();
        self.refresh_style_views();
    }

    pub(crate) fn publish_layout_cache_mutations(&mut self) {
        if self.document.layout_dirty {
            self.scoped_overrides.assert_inactive();
            self.document.layout_cache_context.publish_metadata();
            self.context.grid_layout_info.clear();
            self.context.taffy_hoisted_children.clear();
            self.document.layout_dirty = false;
        }
    }

    pub(crate) fn resolved_edge_basis(&self, idx: usize) -> Option<f32> {
        self.context
            .resolved_edge_bases
            .iter()
            .rev()
            .find(|(key, _)| *key == idx)
            .map(|(_, basis)| *basis)
    }

    pub(crate) fn cache_override_changed(&mut self, idx: usize) {
        self.assert_native_style_access_allowed();
        let key = self
            .nodes
            .box_key(idx)
            .expect("cache override has a box key");
        self.document.layout_cache_context.override_changed(key);
    }

    /// Enclose both Taffy's lookup and its eventual store in the same frame.
    pub(crate) fn with_cache_frame<R>(
        &mut self,
        idx: usize,
        mut view: raikiri_layout_style::ViewContext,
        compute: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.sync_layout_cache_mutations();
        let key = self
            .nodes
            .box_key(idx)
            .expect("cached layout has a box key");
        if let Some((_, basis)) = self
            .context
            .resolved_edge_bases
            .iter()
            .rev()
            .find(|(key, _)| *key == idx)
        {
            view.percent_basis = raikiri_layout_style::PercentBasis::Definite(*basis);
        }
        let frame = self.document.layout_cache_context.enter_view(key, view);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(self)));
        self.document.layout_cache_context.leave(frame);
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(crate) fn node_state(&self, key: usize) -> Option<RefMut<'_, NodeLayoutState>> {
        let box_key = self.document.nodes.box_key(key)?;
        let state = self
            .document
            .layout_cache_context
            .node_state(box_key, self.document.layout_dirty);
        debug_assert_eq!(state.box_key, box_key);
        Some(state)
    }
}

impl Drop for LayoutSession<'_> {
    fn drop(&mut self) {
        self.document.layout_cache_context.end_session();
    }
}

impl Deref for LayoutSession<'_> {
    type Target = Document;

    fn deref(&self) -> &Self::Target {
        self.document
    }
}

impl DerefMut for LayoutSession<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.document
    }
}

#[cfg(test)]
mod tests;
