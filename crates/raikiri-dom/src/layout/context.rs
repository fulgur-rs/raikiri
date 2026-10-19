use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use crate::document::Document;
use crate::fragment::FragmentationContext;
use crate::layout::LayoutWarn;
use crate::node::arena::BoxKey;

// The next task routes layout entrypoints through these types.
#[derive(Debug, Default)]
pub(crate) struct LayoutContext {
    pub(crate) node_states: RefCell<HashMap<BoxKey, NodeLayoutState>>,
    pub(crate) layout_warnings: Vec<LayoutWarn>,
    pub(crate) table_layout_error: Option<String>,
    pub(crate) ifc_width_probe: bool,
    pub(crate) fragmentation_stack: Vec<FragmentationContext>,
    pub(crate) vertical_oof_containing_blocks: Vec<(usize, f32)>,
    pub(crate) orthogonal_block_in_layout: Option<usize>,
    pub(crate) transposed_blocks: Vec<crate::taffy_impl::TransposedBlock>,
    pub(crate) block_item_percent_bases: Vec<(usize, Option<f32>)>,
    pub(crate) flex_item_percent_bases: Vec<(usize, taffy::Size<Option<f32>>)>,
    pub(crate) size_contained_measure: Vec<usize>,
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
    pub(crate) grid_layout_info: HashMap<usize, taffy::DetailedLayoutInfo<String>>,
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
}

impl<'doc> LayoutSession<'doc> {
    pub(crate) fn new(document: &'doc mut Document) -> Self {
        Self {
            document,
            context: LayoutContext::default(),
        }
    }

    pub(crate) fn node_state(&self, key: usize) -> Option<RefMut<'_, NodeLayoutState>> {
        let box_key = self.document.nodes.box_key(key)?;
        let mut states = self.context.node_states.borrow_mut();
        states.entry(box_key).or_insert_with(|| NodeLayoutState {
            box_key,
            taffy_child_walks: None,
        });
        let state = RefMut::map(states, |states| {
            states.get_mut(&box_key).expect("state was inserted above")
        });
        debug_assert_eq!(state.box_key, box_key);
        Some(state)
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
