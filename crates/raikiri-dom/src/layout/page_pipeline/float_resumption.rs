//! Reuse page-walk boundaries when an empty clearing box completes at a break.

use super::{
    CascadeResult, ClearValue, DisplayValue, FloatValue, NodeKind, collapses_through,
    current_abs_y, margins_may_collapse_through,
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

/// The flow boundary and inherited visual offset selected by the page walk.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Boundary {
    pub(crate) page_origin: f32,
    pub(crate) relative_offset: f32,
}

/// A bounded re-layout's local placements and consumed clearance boundaries.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Plan {
    groups: HashMap<(usize, usize), PrefixRun>,
    floats: HashMap<usize, f32>,
    flows: HashMap<usize, f32>,
    completed: HashMap<usize, f32>,
    completed_containers: HashMap<usize, f32>,
}

impl Plan {
    pub(crate) fn is_empty(&self) -> bool {
        self.floats.is_empty()
    }

    pub(crate) fn float_top(&self, node: usize) -> Option<f32> {
        self.floats.get(&node).copied()
    }

    pub(crate) fn flow_top(&self, node: usize) -> Option<f32> {
        self.flows.get(&node).copied()
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
            let Some(run) = document
                .context
                .float_resumptions
                .groups
                .get(&(float, flow))
                .cloned()
                .or_else(|| leading_clear_run(document, cascade, flow))
            else {
                continue;
            };
            let Some(&boundary) = document.context.float_resumption_boundaries.get(&float) else {
                continue;
            };
            let border_top = current_abs_y(document, float);
            let resume = boundary.page_origin + boundary.relative_offset + run.offset;
            if !resume.is_finite() || !border_top.is_finite() {
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
            plan.groups.insert((float, flow), run);
        }
        // Convert only after every planned parent has its final origin.
        // A nested completed chain then has local zeroes, independent of depth.
        for placements in [
            &mut plan.floats,
            &mut plan.flows,
            &mut plan.completed,
            &mut plan.completed_containers,
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
                }
                return false;
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
            }
            break;
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
