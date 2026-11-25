//! Reuse page-walk boundaries to reflow a float and its following block.

use super::{
    CascadeResult, ClearValue, DisplayValue, FloatValue, NodeKind, collapsed_margin_above,
    collapses_through, current_abs_y, margins_may_collapse_through,
};
use crate::layout::context::LayoutSession;
use raikiri_style::ComputedLengthPercentageOrAuto;
use raikiri_style::property::PositionValue;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
struct PrefixRun {
    cleared: Vec<usize>,
    containers: Vec<usize>,
    offset: f32,
}

#[derive(Clone, Debug, PartialEq)]
enum FlowStart {
    Clearance(PrefixRun),
    Margin,
}

/// The flow boundary and inherited visual offset selected by the page walk.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Boundary {
    pub(crate) page_origin: f32,
    pub(crate) relative_offset: f32,
}

/// How a float grew when the page walk moved its content down past page
/// edges, and how far the walk moved the flow beside it.
///
/// Chromium breaks the lines beside the float on each later page against the
/// part of the float that continues there. The float laid out unbroken ends
/// higher, by its growth less how much further the walk moved the flow
/// beside its end than the float's top: the lines that follow, up to the
/// float's new end, are broken beside it again (see [`Plan::line_extent`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineGrowth {
    /// How much the float grew.
    pub(crate) grown: f32,
    /// How far the walk moved the float's top.
    pub(crate) start_shift: f32,
    /// The root of the float's formatting context.
    pub(crate) context: usize,
    /// How far the walk moved the last of the in-flow content after the
    /// float in its formatting context that starts above the float's end.
    pub(crate) beside_shift: f32,
}

/// A bounded re-layout's local placements and consumed clearance boundaries.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Plan {
    groups: HashMap<(usize, usize), FlowStart>,
    floats: HashMap<usize, f32>,
    pushed: HashMap<usize, f32>,
    flows: HashMap<usize, f32>,
    /// Blocks the page walk moved whole to a later page, away from the
    /// floats their lines were broken beside, each with how far it moved
    /// them and where their top was in the page flow before: layout places
    /// them as far below their place in the flow, so that their lines are
    /// broken against the floats that are there, as Chromium lays them out.
    pushed_flows: HashMap<usize, (f32, f32)>,
    margin_flows: HashMap<usize, (usize, f32)>,
    completed: HashMap<usize, f32>,
    completed_containers: HashMap<usize, f32>,
    /// Floats after a forced break at the end of the content of a box whose
    /// block-end edges close on the page the break leaves, each with the
    /// lowest top it takes: where the box closes, as the flow after the box
    /// starts the next page beside the float.
    closed: HashMap<usize, f32>,
    /// Boxes after a forced break that start the page the break opens
    /// beside an earlier float that runs on onto it, each with its top
    /// there, local to its parent: the border-box top of a block, or the
    /// margin-box top of a float. Layout places them there in place of the
    /// clearance the break gives them.
    break_tops: HashMap<usize, f32>,
    /// Floats that the page walk grew past a page edge, each with how far
    /// below its end laid out unbroken the lines beside it are broken against
    /// it (see [`LineGrowth`]).
    line_extents: HashMap<usize, f32>,
}

impl Plan {
    pub(crate) fn is_empty(&self) -> bool {
        self.floats.is_empty()
            && self.pushed.is_empty()
            && self.pushed_flows.is_empty()
            && self.margin_flows.is_empty()
            && self.closed.is_empty()
            && self.break_tops.is_empty()
            && self.line_extents.is_empty()
    }

    /// How far below the end of the float `node`, laid out unbroken, the
    /// lines beside it are broken against it: the part of the float that the
    /// page walk moved past a page edge and that the lines beside it did not
    /// move past with it.
    pub(crate) fn line_extent(&self, node: usize) -> Option<f32> {
        self.line_extents.get(&node).copied()
    }

    /// Where a box after a forced break starts the page the break opens,
    /// beside an earlier float that runs on onto that page.
    pub(crate) fn break_top(&self, node: usize) -> Option<f32> {
        self.break_tops.get(&node).copied()
    }

    /// The boxes after a forced break that the plan starts beside an earlier
    /// float that runs on onto the page the break opens.
    pub(crate) fn break_top_nodes(&self) -> impl Iterator<Item = usize> + '_ {
        self.break_tops.keys().copied()
    }

    /// Whether `other` plans the same as this, but for the blocks moved away
    /// from the floats beside their lines and the boxes started beside an
    /// earlier float after a forced break.
    pub(crate) fn same_apart_from_flows_and_break_tops(&self, other: &Self) -> bool {
        let mut this = self.clone();
        this.pushed_flows.clone_from(&other.pushed_flows);
        this.break_tops.clone_from(&other.break_tops);
        this == *other
    }

    /// The lowest top of a float after a box that closed on the page the
    /// forced break before the float leaves.
    pub(crate) fn closed_float_top(&self, node: usize) -> Option<f32> {
        self.closed.get(&node).copied()
    }

    pub(crate) fn float_top(&self, node: usize) -> Option<f32> {
        self.floats.get(&node).copied()
    }

    /// Where a float that the page walk moved whole to a later page goes.
    pub(crate) fn pushed_float_top(&self, node: usize) -> Option<f32> {
        self.pushed.get(&node).copied()
    }

    /// The floats that the page walk moved whole to a later page.
    pub(crate) fn pushed_floats(&self) -> impl Iterator<Item = usize> + '_ {
        self.pushed.keys().copied()
    }

    pub(crate) fn has_pushed_floats(&self) -> bool {
        !self.pushed.is_empty()
    }

    /// Whether `other` plans the same as this, but for the blocks moved away
    /// from the floats beside their lines.
    pub(crate) fn same_apart_from_pushed_flows(&self, other: &Self) -> bool {
        let mut this = self.clone();
        this.pushed_flows.clone_from(&other.pushed_flows);
        this == *other
    }

    /// The blocks the page walk moved whole to a later page away from the
    /// floats beside their lines.
    pub(crate) fn pushed_flows(&self) -> impl Iterator<Item = usize> + '_ {
        self.pushed_flows.keys().copied()
    }

    /// How far below its place in the flow a block that the page walk moved
    /// whole to a later page, away from the floats beside its lines, goes.
    pub(crate) fn pushed_flow_delta(&self, node: usize) -> Option<f32> {
        self.pushed_flows.get(&node).map(|&(delta, _)| delta)
    }

    pub(crate) fn flow_top(&self, node: usize) -> Option<f32> {
        self.flows.get(&node).copied()
    }

    pub(crate) fn has_margin_flows(&self) -> bool {
        !self.margin_flows.is_empty()
    }

    /// The float that `flow` lines up below, and the margin the forced
    /// break keeps above `flow`: its own collapsed with the margins of its
    /// first children that adjoin it.
    pub(crate) fn margin_float(&self, flow: usize) -> Option<(usize, f32)> {
        self.margin_flows.get(&flow).copied()
    }

    pub(crate) fn completed_at(&self, node: usize, height: f32) -> Option<f32> {
        if height != 0.0 {
            return None;
        }
        self.completed
            .get(&node)
            .or_else(|| self.completed_containers.get(&node))
            .copied()
    }

    pub(crate) fn collect(document: &LayoutSession<'_>, cascade: &CascadeResult) -> Self {
        let mut plan = Self::default();
        let mut origins = HashMap::new();
        for &(float, flow) in &document.context.float_resumption_candidates {
            if !matches!(document.nodes[float].position(), PositionValue::Static) {
                continue;
            }
            let margin = plain_flow_margin(document, cascade, float, flow);
            let Some(start) = document
                .context
                .float_resumptions
                .groups
                .get(&(float, flow))
                .cloned()
                .or_else(|| leading_clear_run(document, cascade, flow).map(FlowStart::Clearance))
                .or_else(|| margin.map(|_| FlowStart::Margin))
            else {
                continue;
            };
            let Some(&boundary) = document.context.float_resumption_boundaries.get(&float) else {
                continue;
            };
            let border_top = current_abs_y(document, float);
            let boundary_top = boundary.page_origin + boundary.relative_offset;
            if !boundary_top.is_finite() || !border_top.is_finite() {
                continue;
            }
            let run = match &start {
                FlowStart::Clearance(run) => run,
                FlowStart::Margin => {
                    if margin.is_some() {
                        // Source geometry may shrink before a later forced break.
                        // Retain the relationship, not the old page's coordinates.
                        plan.margin_flows.insert(
                            flow,
                            (float, collapsed_margin_above(document, cascade, flow)),
                        );
                        plan.groups.insert((float, flow), start);
                    }
                    continue;
                }
            };
            let resume = boundary_top + run.offset;
            if !resume.is_finite() {
                continue;
            }
            // The margin box and the retained escaping strut are distinct
            // from the physical fragment boundary selected by pagination.
            plan.floats
                .insert(float, border_top - document.nodes[float].layout.margin.top);
            origins.insert(float, border_top);
            plan.flows.insert(flow, resume);
            origins.insert(flow, resume);
            let mut ancestors = HashSet::new();
            for &cleared in &run.cleared {
                plan.completed.insert(cleared, resume);
                origins.insert(cleared, resume);
                if cleared == flow {
                    continue;
                }
                let mut ancestor = document.layout_parent_of(cleared);
                while let Some(id) = ancestor {
                    if !ancestors.insert(id) || id == flow {
                        break;
                    }
                    ancestor = document.layout_parent_of(id);
                }
            }
            // Source preorder reversed is bottom-up. Every empty ancestor
            // and its sibling list are inspected once, even for a long run.
            for &id in run.containers.iter().rev() {
                if ancestors.contains(&id)
                    && !plan.completed.contains_key(&id)
                    && plan.can_complete_container(document, cascade, id, resume)
                {
                    plan.completed_containers.insert(id, resume);
                    origins.insert(id, resume);
                }
            }
            plan.groups.insert((float, flow), start);
        }
        // A float the walk moved whole to a later page, as Chromium places
        // it: at the top it took there, so the lines and boxes beside it on
        // the page it left take the whole width again. One placed so before,
        // that still starts the later page, keeps that place: the lines it
        // lets move down may carry it along.
        let mut kept = Vec::new();
        let mut seen = HashSet::new();
        let pushed = document.context.pushed_floats.iter().copied();
        for float in pushed.filter(|float| !plan.floats.contains_key(float) && seen.insert(*float))
        {
            if let Some(top) = document.context.float_resumptions.pushed_float_top(float) {
                kept.push((float, top));
            } else {
                // Layout adds the offsets of relatively positioned inline
                // ancestors after it places the float; keep them out of the
                // place it starts from, or the relayout adds them twice.
                let border_top =
                    current_abs_y(document, float) - relative_inline_offset_y(document, float);
                plan.pushed
                    .insert(float, border_top - document.nodes[float].layout.margin.top);
                origins.insert(float, border_top);
            }
        }
        for &(node, top) in &document.context.forced_break_tops {
            plan.break_tops.insert(node, top);
        }
        // Convert only after every planned parent has its final origin.
        // A nested completed chain then has local zeroes, independent of depth.
        for placements in [
            &mut plan.floats,
            &mut plan.pushed,
            &mut plan.flows,
            &mut plan.completed,
            &mut plan.completed_containers,
            &mut plan.break_tops,
        ] {
            for (&id, top) in placements {
                let parent_top = document.layout_parent_of(id).map_or(0.0, |parent| {
                    origins
                        .get(&parent)
                        .copied()
                        .unwrap_or_else(|| current_abs_y(document, parent))
                });
                *top -= parent_top;
            }
        }
        plan.pushed.extend(kept);
        // A block the walk moved whole to a later page, away from the floats
        // its lines were broken beside, goes as far down again, so that its
        // lines take the room the floats leave there (Chromium).
        let previous = &document.context.float_resumptions.pushed_flows;
        // One that a planned move already put lower and that the walk moves
        // again no longer sits where the move was planned for: the next walk
        // judges it afresh.
        for &(flow, delta, from) in &document.context.pushed_flows {
            if !previous.contains_key(&flow) {
                plan.pushed_flows.entry(flow).or_insert((delta, from));
            }
        }
        // One that the walk no longer moves keeps its move while the flow
        // above it still puts it where it started: then the move is still
        // what takes it to the later page. Otherwise, or when the walk moves
        // it on with its lines for another reason, the next walk judges it
        // afresh.
        let moved: HashSet<usize> = (document.context.pushed_flows.iter())
            .map(|&(moved, ..)| moved)
            .chain(document.context.flows_moved_otherwise.iter().copied())
            .collect();
        for (&flow, &(delta, from)) in previous {
            let moved = moved.contains(&flow);
            // Far down a long flow, f32 keeps fewer fractions of a pixel.
            let tolerance = 0.01_f32.max(from.abs() * 1e-6);
            if !moved && (current_abs_y(document, flow) - delta - from).abs() <= tolerance {
                plan.pushed_flows.insert(flow, (delta, from));
            }
        }
        plan.closed
            .extend(document.context.floats_after_closed_edges.iter().copied());
        // A float keeps the extent first planned for it: the lines broken
        // against it move as far again, and a new extent would break them
        // anew.
        let planned = &document.context.float_resumptions.line_extents;
        for (&float, growth) in &document.context.float_line_growth {
            let extent = growth.grown - (growth.beside_shift - growth.start_shift).max(0.0);
            if let Some(&extent) = planned.get(&float) {
                plan.line_extents.insert(float, extent);
            } else if extent > 0.001 {
                plan.line_extents.insert(float, extent);
            }
        }
        plan
    }

    fn can_complete_container(
        &self,
        document: &LayoutSession<'_>,
        cascade: &CascadeResult,
        id: usize,
        boundary: f32,
    ) -> bool {
        if !plain_container(document, cascade, id) {
            return false;
        }
        let mut children = document.nodes[id].children.clone();
        while let Some(child) = children.pop() {
            let node = &document.nodes[child];
            if hidden_or_positioned(document, child)
                || cascade.computed[child].float != FloatValue::None
            {
                continue;
            }
            if node.display() == DisplayValue::Contents {
                children.extend(node.children.iter().copied());
                continue;
            }
            if let crate::node::NodeData::Text(text) = &node.data {
                if collapsed_space(&text.text_content) {
                    continue;
                } // cov:ignore: nonempty text fixtures are rejected earlier by the measured IFC; retain this defensive fallback.
                return false; // cov:ignore: nonempty text fixtures are rejected earlier by the measured IFC; retain this defensive fallback.
            }
            if let Some(&end) = self
                .completed
                .get(&child)
                .or_else(|| self.completed_containers.get(&child))
            {
                if end == boundary {
                    continue;
                }
                return false;
            }
            if !collapses_through(document, cascade, child, node.layout.size.height) {
                return false;
            }
        }
        true
    }
}

// Reflow an in-flow block below the float's top plus its own margin, the
// margin the forced break keeps; the margins of its first children collapse
// into that one above its top. Layout places the whole box again there,
// so its lines, block children and generated blocks are broken against the
// float afresh, wherever the earlier layout put them. A relative offset only
// moves the box once it is placed. Resolve the used margin on every attempt.
fn plain_flow_margin(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    float: usize,
    flow: usize,
) -> Option<f32> {
    let node = &document.nodes[flow];
    let margin = node.layout.margin.top;
    if node.display() != DisplayValue::Block
        || !matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative
        )
        || node.used_writing_mode.is_vertical()
        || cascade.computed[flow].clear != ClearValue::None
        || !margins_may_collapse_through(document, cascade, flow)
        || document.layout_parent_of(float) != document.layout_parent_of(flow)
        || !margin.is_finite()
        || margin <= 0.0
    {
        return None;
    }
    Some(margin)
}

/// The vertical offsets of the relatively positioned inline elements
/// between `float` and its layout parent, the paragraph root, added up.
fn relative_inline_offset_y(document: &LayoutSession<'_>, float: usize) -> f32 {
    let parent = document.layout_parent_of(float);
    let offsets = parent.map_or(&[][..], |parent| {
        document.nodes[parent].ifc_relative_offsets()
    });
    document
        .ancestors(float)
        .take_while(|&id| Some(id) != parent)
        .filter_map(|id| offsets.iter().find(|&&(node, _)| node == id))
        .map(|&(_, (_, dy))| dy)
        .sum()
}

fn hidden_or_positioned(document: &LayoutSession<'_>, id: usize) -> bool {
    let node = &document.nodes[id];
    node.display() == DisplayValue::None
        || matches!(
            node.position(),
            PositionValue::Absolute | PositionValue::Fixed
        )
}

fn collapsed_space(text: &str) -> bool {
    text.chars()
        .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c'))
}

fn auto_or_zero(value: &ComputedLengthPercentageOrAuto) -> bool {
    match value {
        ComputedLengthPercentageOrAuto::Auto => true,
        ComputedLengthPercentageOrAuto::Px(value)
        | ComputedLengthPercentageOrAuto::Percent(value) => *value == 0.0,
        _ => false,
    }
}

fn plain_container(document: &LayoutSession<'_>, cascade: &CascadeResult, id: usize) -> bool {
    let node = &document.nodes[id];
    node.kind() == NodeKind::Element
        && node.display() == DisplayValue::Block
        && matches!(node.position(), PositionValue::Static)
        && !node.used_writing_mode.is_vertical()
        && margins_may_collapse_through(document, cascade, id)
        && node.layout.border.vertical_axis_sum() + node.layout.padding.vertical_axis_sum() == 0.0
        && auto_or_zero(&cascade.computed[id].height)
        && auto_or_zero(&cascade.computed[id].min_height)
        && !node.ifc.as_ref().is_some_and(|root| {
            !root.generated_blocks.is_empty()
                || root
                    .lines
                    .as_ref()
                    .is_some_and(|lines| !lines.lines.is_empty())
        })
}

fn leading_clear_run(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    flow: usize,
) -> Option<PrefixRun> {
    let mut pending = vec![flow];
    let mut run = PrefixRun {
        cleared: Vec::new(),
        containers: Vec::new(),
        offset: 0.0,
    };
    let mut active = false;
    while let Some(id) = pending.pop() {
        let node = &document.nodes[id];
        if hidden_or_positioned(document, id) {
            continue;
        }
        if let crate::node::NodeData::Text(text) = &node.data {
            if collapsed_space(&text.text_content) {
                continue;
            } // cov:ignore: nonempty text fixtures are rejected earlier by the measured IFC; retain this defensive fallback.
            break; // cov:ignore: nonempty text fixtures are rejected earlier by the measured IFC; retain this defensive fallback.
        }
        if node.display() == DisplayValue::Contents {
            pending.extend(node.children.iter().rev().copied());
            continue;
        }
        if !plain_container(document, cascade, id) {
            break;
        }
        run.containers.push(id);
        if cascade.computed[id].clear != ClearValue::None {
            let Some(&used) = document.context.block_clearance.get(&id) else {
                break;
            };
            if used && !active {
                let Some(&offset) = document.context.clearance_resumption_offsets.get(&id) else {
                    break;
                };
                run.offset = offset;
                active = true;
            }
            if active && collapses_through(document, cascade, id, node.layout.size.height) {
                run.cleared.push(id);
            } else if used {
                break;
            }
        }
        if node.children.is_empty() && node.layout.size.height > 0.0 {
            break;
        }
        pending.extend(node.children.iter().rev().copied());
    }
    active.then_some(run)
}

#[cfg(test)]
mod tests;
