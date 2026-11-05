//! Measure grid row areas using the native placement and track algorithms.
//!
//! The proxy discards root layout writes and captures its grid report locally.
//! Children are measured rather than placed; their measurement caches remain
//! active, but no final root geometry or pagination metadata is published.

use super::super::GridInfo;
use super::super::convert::{FromTaffy, IntoTaffy};
use crate::layout::context::LayoutSession;
use crate::layout::dispatch::{flex_cross_margins, grid_baseline, table_items};
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode,
};
use crate::layout::style_views::ReadRole;
use raikiri_layout_geometry::{BoxLayout, Point, Size};
use raikiri_layout_style::TaffyStyle;
use raikiri_layout_style::values::Position;
use std::collections::HashMap;
use taffy::{
    CoreStyle, LayoutFlexboxContainer, LayoutGridContainer, LayoutPartialTree, TraversePartialTree,
};

pub(crate) fn grid_item_row_rooms(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    mut inputs: LayoutInput,
) -> Vec<(NodeId, f32)> {
    inputs = super::with_stretch_width(tree, node, inputs);
    inputs.run_mode = RunMode::PerformLayout;
    let mut probe = RowMeasure::new(tree, node, false);
    let items = TraversePartialTree::child_ids(&probe, node.into_taffy())
        .filter(|&child| {
            let style = probe.get_grid_child_style(child);
            style.box_generation_mode() != taffy::BoxGenerationMode::None
                && !style.position().is_out_of_flow()
        })
        .map(NodeId::from_taffy)
        .collect::<Vec<_>>();
    taffy::compute_grid_layout(&mut probe, node.into_taffy(), inputs.into_taffy());
    let report = probe.report.expect("grid row measure reports its tracks");
    items
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| report.item_area_size(index).map(|area| (item, area.height)))
        .collect()
}

/// Run a size query with native placement writes isolated. A baseline query
/// additionally replays placement under the same active sizing overrides,
/// retaining only its baseline and preserving the ordinary measured size.
pub(super) fn compute_layout(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    grid: bool,
) -> LayoutOutput {
    let idx = usize::from(node);
    let baseline = tree.context.baseline_measure_owners.last() == Some(&idx)
        && !tree.nodes[idx].used_writing_mode.is_vertical();
    // Native size queries can request child placement to obtain baselines
    // for internal alignment, even when this owner exports none.
    let mut probe = RowMeasure::new(tree, node, true);
    let mut output = native_layout(&mut probe, node, inputs, grid);
    if baseline && output.baselines.first.is_none() {
        let placed = native_layout(
            &mut probe,
            node,
            LayoutInput {
                run_mode: RunMode::PerformLayout,
                ..inputs
            },
            grid,
        );
        let placed = if grid {
            let report = probe
                .report
                .as_ref()
                .expect("grid baseline measure reports tracks");
            grid_baseline::with_measured_first_baseline(probe.tree, idx, placed, report, |child| {
                probe
                    .layouts
                    .get(&child)
                    .copied()
                    .zip(probe.baselines.get(&child).copied())
            })
        } else {
            placed
        };
        output.baselines = placed.baselines;
    }
    output
}

fn native_layout(
    probe: &mut RowMeasure<'_, '_>,
    node: NodeId,
    inputs: LayoutInput,
    grid: bool,
) -> LayoutOutput {
    probe.flex_calls = (!grid).then(Vec::new);
    let mut output = LayoutOutput::from_taffy(if grid {
        taffy::compute_grid_layout(probe, node.into_taffy(), inputs.into_taffy())
    } else {
        taffy::compute_flexbox_layout(probe, node.into_taffy(), inputs.into_taffy())
    });
    if !grid
        && let Some(correction) = super::wrap_reverse::candidate(
            probe.tree,
            node,
            inputs,
            probe
                .flex_calls
                .as_deref()
                .expect("flex probe records callbacks"),
            &output,
            |child| probe.layouts.get(&child).copied(),
        )
    {
        probe.layouts.extend(correction.layouts);
        output = correction.output;
    }
    if !grid
        && output.baselines.first.is_some()
        && probe
            .tree
            .query_style(usize::from(node), ReadRole::FlexContainer)
            .flex_direction_is_row()
    {
        let mut shifts = Vec::new();
        for (&child, layout) in &mut probe.layouts {
            let style = probe.tree.query_style(child, ReadRole::FlexItem);
            if style.position() == Position::Absolute || probe.tree.nodes[child].is_display_none() {
                continue;
            }
            if let Some(corrected) =
                flex_cross_margins::corrected(*layout, style.margin(), true, false)
            {
                let delta = corrected.location.y - layout.location.y;
                *layout = corrected;
                if delta != 0.0 {
                    shifts.push((child, Point { x: 0.0, y: delta }));
                }
            }
        }
        if !shifts.is_empty() {
            table_items::shift_first_baseline(
                probe.tree,
                usize::from(node),
                inputs,
                probe
                    .flex_calls
                    .as_deref()
                    .expect("native flex probes record callbacks"),
                &shifts,
                &mut output,
            );
        }
    }
    output
}

struct RowMeasure<'a, 'doc> {
    tree: &'a mut LayoutSession<'doc>,
    owner: NodeId,
    child_baselines: bool,
    flex_calls: Option<Vec<super::FlexReplayCall>>,
    report: Option<GridInfo>,
    layouts: HashMap<usize, BoxLayout>,
    baselines: HashMap<usize, Option<f32>>,
}

impl<'a, 'doc> RowMeasure<'a, 'doc> {
    fn new(tree: &'a mut LayoutSession<'doc>, owner: NodeId, child_baselines: bool) -> Self {
        Self {
            tree,
            owner,
            child_baselines,
            flex_calls: None,
            report: None,
            layouts: HashMap::new(),
            baselines: HashMap::new(),
        }
    }

    fn baseline_request(&self, child: usize, inputs: &LayoutInput) -> bool {
        if inputs.run_mode != RunMode::PerformLayout {
            return false;
        }
        if self.child_baselines {
            return true;
        }
        // This is the native grid track algorithm's baseline-group query,
        // before columns or rows have supplied definite item dimensions.
        inputs.axis == RequestedAxis::Both
            && inputs.known_dimensions == Size::NONE
            && inputs.available_space
                == Size {
                    width: AvailableSpace::MinContent,
                    height: AvailableSpace::MinContent,
                }
            && grid_baseline::aligns_by_baseline(
                &self
                    .tree
                    .query_style(usize::from(self.owner), ReadRole::GridContainer),
                &self.tree.query_style(child, ReadRole::GridItem),
                false,
            )
    }
}

impl<'doc> TraversePartialTree for RowMeasure<'_, 'doc> {
    type ChildIter<'a>
        = <LayoutSession<'doc> as TraversePartialTree>::ChildIter<'a>
    where
        Self: 'a;
    fn child_ids(&self, node: taffy::NodeId) -> Self::ChildIter<'_> {
        TraversePartialTree::child_ids(self.tree, node)
    }
    fn child_count(&self, node: taffy::NodeId) -> usize {
        TraversePartialTree::child_count(self.tree, node)
    }
    fn get_child_id(&self, node: taffy::NodeId, index: usize) -> taffy::NodeId {
        TraversePartialTree::get_child_id(self.tree, node, index)
    }
}

impl LayoutPartialTree for RowMeasure<'_, '_> {
    type CustomIdent = smol_str::SmolStr;
    type CoreContainerStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;
    // cov:ignore: Required by LayoutPartialTree; native grid/flex algorithms read their specialized container styles instead.
    fn get_core_container_style(&self, node: taffy::NodeId) -> Self::CoreContainerStyle<'_> {
        self.tree.get_core_container_style(node)
    }
    fn resolve_calc_value(&self, value: *const (), basis: f32) -> f32 {
        self.tree.resolve_calc_value(value, basis)
    }
    fn set_unrounded_layout(&mut self, node: taffy::NodeId, layout: &taffy::Layout) {
        self.layouts.insert(
            usize::from(NodeId::from_taffy(node)),
            BoxLayout::from_taffy(*layout),
        );
    }
    fn compute_child_layout(
        &mut self,
        node: taffy::NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        let mut inputs = LayoutInput::from_taffy(inputs);
        let requested = inputs;
        let node = NodeId::from_taffy(node);
        let baseline = self.baseline_request(usize::from(node), &inputs);
        let placed = inputs.run_mode == RunMode::PerformLayout;
        inputs.run_mode = RunMode::ComputeSize;
        let output = if baseline {
            self.tree.with_baseline_measure(usize::from(node), |tree| {
                tree.compute_child_layout(node, inputs)
            })
        } else {
            self.tree.compute_child_layout(node, inputs)
        };
        if placed {
            self.baselines
                .insert(usize::from(node), output.baselines.first);
        }
        let reversed = self
            .tree
            .query_style(usize::from(self.owner), ReadRole::CoreContainer)
            .direction()
            .is_rtl();
        let output = super::native_child_output(self.tree, node, output, Some(reversed));
        if let Some(calls) = &mut self.flex_calls {
            calls.push(super::FlexReplayCall {
                node,
                inputs: requested,
                output: output.clone(),
            });
        }
        output.into_taffy()
    }
}

impl LayoutGridContainer for RowMeasure<'_, '_> {
    type GridContainerStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;
    fn get_grid_container_style(&self, node: taffy::NodeId) -> Self::GridContainerStyle<'_> {
        self.tree.get_grid_container_style(node)
    }
    fn get_grid_child_style(&self, node: taffy::NodeId) -> Self::GridItemStyle<'_> {
        self.tree.get_grid_child_style(node)
    }
    fn set_detailed_grid_info(
        &mut self,
        _: taffy::NodeId,
        report: taffy::DetailedGridInfo<Self::CustomIdent>,
    ) {
        self.report = Some(GridInfo::new(report));
    }
}

impl LayoutFlexboxContainer for RowMeasure<'_, '_> {
    type FlexboxContainerStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;
    fn get_flexbox_container_style(&self, node: taffy::NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.tree.get_flexbox_container_style(node)
    }
    fn get_flexbox_child_style(&self, node: taffy::NodeId) -> Self::FlexboxItemStyle<'_> {
        self.tree.get_flexbox_child_style(node)
    }
}
