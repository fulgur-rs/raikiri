use std::cell::RefMut;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use crate::document::Document;
use crate::fragment::FragmentationContext;
use crate::layout::LayoutWarn;
use crate::layout::protocol::{self, LayoutInput};
use crate::layout::taffy_adapter::GridInfo;
use crate::node::arena::BoxKey;
use raikiri_layout_geometry::{Point, Size};

// The next task routes layout entrypoints through these types.
#[derive(Debug, Default)]
pub(crate) struct LayoutContext {
    pub(crate) layout_warnings: Vec<LayoutWarn>,
    pub(crate) table_layout_error: Option<String>,
    pub(super) table_cache: super::table::cache::TableCache,
    pub(crate) ifc_width_probe: bool,
    pub(crate) fragmentation_stack: Vec<FragmentationContext>,
    pub(crate) vertical_oof_containing_blocks: Vec<(usize, f32)>,
    /// Whether the out-of-flow boxes being positioned have a containing block
    /// on vertical axes, whose alignment properties name the other physical
    /// axes than the library assumes (see `with_oof_axes` in [`crate::layout::dispatch`]).
    pub(crate) oof_axes_vertical: bool,
    pub(crate) orthogonal_block_in_layout: Option<usize>,
    pub(crate) transposed_style_index: crate::layout::dispatch::TransposedStyleIndex,
    pub(crate) transposed_frame_serial: u64,
    pub(crate) transposed_blocks: Vec<crate::layout::dispatch::TransposedBlock>,
    pub(crate) block_item_percent_bases: Vec<(usize, Option<f32>)>,
    pub(crate) flex_item_percent_bases: Vec<(usize, Size<Option<f32>>)>,
    pub(crate) size_contained_measure: Vec<usize>,
    pub(crate) resolved_edge_bases: Vec<(usize, f32)>,
    /// The boxes being measured without their intrinsic `min-*` and `max-*`
    /// sizes (see `dispatch::content_limits`).
    pub(crate) content_limit_probes: Vec<usize>,
    /// The flex container whose intrinsic widths are being measured, whose
    /// items are bounded by their flex base sizes meanwhile (see
    /// `with_flex_intrinsic_probe` in [`crate::layout::dispatch`]).
    pub(crate) flex_intrinsic_probe: Option<usize>,
    /// Definite block sizes retained as percentage bases for the direct
    /// children of boxes being measured for intrinsic height limits.
    pub(crate) content_limit_percentage_height_bases: Vec<(usize, f32)>,
    /// The width or height each box measured for its keyword sizes, while the
    /// outermost box laying out with them runs, and how many such layouts are
    /// running (see `dispatch::content_limits`).
    pub(crate) content_limit_measures:
        HashMap<crate::layout::dispatch::content_limits::MeasureKey, f32>,
    pub(crate) content_limit_depth: usize,
    /// The final layout inputs observed for out-of-flow candidates during a
    /// preliminary positioning pass, before their intrinsic limits are
    /// installed.
    pub(crate) oof_content_limit_inputs: Vec<HashMap<usize, LayoutInput>>,
    /// Nested candidate wrappers skip their own prepasses while an outer
    /// candidate wrapper is gathering unconstrained positioning inputs.
    pub(crate) oof_content_limit_prepass_depth: usize,
    /// What the grid algorithm reported for each grid container laid out so
    /// far. Only reports of grid layouts are stored.
    pub(crate) grid_layout_info: HashMap<usize, GridInfo>,
    pub(crate) taffy_hoisted_children: HashMap<usize, Vec<protocol::NodeId>>,
    /// The block extent each vertical paragraph root with an auto block size
    /// settled on, by root and inline size (the bits of the physical height).
    /// It is a first guess for the next layout of the same root, and a wrong
    /// one is corrected like any other.
    pub(crate) vertical_block_size_seeds: HashMap<(usize, u32), f32>,
    /// The static positions of the boxes hoisted to a containing block on
    /// vertical axes that align `safe` in an axis, which that block aligns
    /// again once the library has positioned them (see
    /// `dispatch::vertical_oof::alignment`).
    pub(crate) safe_static_positions: HashMap<usize, Point<protocol::AxisStaticPosition>>,
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
    /// Whether a multicol container of plain paragraphs and a height of
    /// `auto` is laid out without keeping its groups for pagination, which
    /// leaves its columns in one tall set that the page walk cuts (see
    /// its `paged_spanning` module in [`crate::layout::multicol`]).
    pub(crate) unpaged_plain_columns: bool,
    /// The multicol containers without a spanner whose columns pagination
    /// filled page by page.
    pub(crate) page_filled_columns: Vec<usize>,
    /// Whether the page walk moved any of those containers from where the
    /// pagination placed their columns, which are then in the wrong pages.
    pub(crate) page_filled_columns_moved: bool,
    /// How deeply the child layouts of this pass are nested, and whether that
    /// passed its limit.
    pub(crate) recursion: LayoutRecursion,
}

/// The nesting of the child layouts running at a moment (see
/// `compute_child_layout_with_block_ctx` in [`crate::layout::dispatch`]).
///
/// The stack grows on demand while a layout recurses, so a cycle that never
/// ends does not overflow it; it takes memory until the process is gone. Past
/// `limit` nested layouts the pass stops descending and `exceeded_at` names the
/// box it stopped at, which ends the pass with an error.
#[derive(Debug)]
pub(crate) struct LayoutRecursion {
    pub(crate) depth: usize,
    pub(crate) limit: usize,
    pub(crate) exceeded_at: Option<usize>,
    /// The deepest nesting so far, for the tests that size the limit.
    #[cfg(test)]
    pub(crate) peak: usize,
}

impl Default for LayoutRecursion {
    fn default() -> Self {
        Self {
            depth: 0,
            limit: super::MAX_LAYOUT_RECURSION,
            exceeded_at: None,
            #[cfg(test)]
            peak: 0,
        }
    }
}

impl LayoutRecursion {
    /// Count one more nested layout of the box `idx`, or refuse it: `false`
    /// when the limit is passed (now or earlier in this pass), and the layout
    /// must not run. The layout that is let in is left with [`Self::leave`].
    pub(crate) fn enter(&mut self, idx: usize) -> bool {
        if self.exceeded_at.is_some() {
            return false;
        }
        if self.depth >= self.limit {
            self.exceeded_at = Some(idx);
            return false;
        }
        self.depth += 1;
        #[cfg(test)]
        {
            self.peak = self.peak.max(self.depth);
        }
        true
    }

    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }
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
    pub(crate) taffy_child_walks: Option<Vec<protocol::NodeId>>,
}

pub(crate) struct LayoutSession<'doc> {
    pub(crate) document: &'doc mut Document,
    pub(crate) context: LayoutContext,
    pub(crate) style_views: super::style_views::StyleViews<'doc>,
    pub(crate) scoped_overrides: super::style_views::ScopedOverrides,
}

impl<'doc> LayoutSession<'doc> {
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
            style_views: super::style_views::StyleViews::new(cascade),
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
            self.context.safe_static_positions.clear();
            self.context.vertical_block_size_seeds.clear();
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
