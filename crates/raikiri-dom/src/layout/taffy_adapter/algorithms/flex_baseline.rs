//! Identify a moved stretch item's contribution to a flex baseline.
//!
//! Flex line membership is private to the library. Replay its callback results
//! on a read-only tree instead of guessing the source from equal baselines.
//! Only non-baseline-aligned final-layout results are tagged: line sizing and
//! alignment are already settled then. No layout or cache write reaches the
//! session, and no child is laid out again.

use std::collections::HashMap;

use super::super::convert::{FromTaffy, IntoTaffy};
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutInput, LayoutOutput, NodeId, RunMode};
use crate::layout::style_views::ReadRole;
use raikiri_layout_style::TaffyStyle;
use raikiri_layout_style::values::{MaybeResolve, Position};
use taffy::{LayoutFlexboxContainer, LayoutPartialTree, TraversePartialTree};

#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub(crate) node: NodeId,
    pub(crate) inputs: LayoutInput,
    pub(crate) output: LayoutOutput,
}

// Contiguous mantissas keep one exponent and spacing throughout each chunk.
// The checked offset bound below makes addition round back to the exact tag.
const TAG_BASE: u32 = 0x7000_0000;
const CHUNK: usize = 1 << 20;

pub(crate) fn source(
    tree: &LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    calls: &[Call],
    moved: &[usize],
) -> Result<Option<usize>, &'static str> {
    let inputs = super::with_stretch_width(tree, node, inputs);
    for chunk in moved.chunks(CHUNK) {
        let tags: HashMap<_, _> = chunk
            .iter()
            .enumerate()
            .map(|(offset, &child)| {
                let offset = u32::try_from(offset).expect("replay chunk is bounded");
                (child, f32::from_bits(TAG_BASE + offset))
            })
            .collect();
        let mut replay = Replay {
            tree,
            calls,
            tags,
            next: 0,
            last: HashMap::new(),
            mismatch: false,
            offset_bound: 0.0,
        };
        let output =
            taffy::compute_flexbox_layout(&mut replay, node.into_taffy(), inputs.into_taffy());
        if replay.mismatch || replay.next != calls.len() {
            return Err("flex baseline replay changed its callback schedule");
        }
        let spacing = f64::from(f32::from_bits(TAG_BASE + 1)) - f64::from(f32::from_bits(TAG_BASE));
        if !replay.offset_bound.is_finite() || replay.offset_bound >= spacing / 4.0 {
            return Err("flex baseline replay exceeded its exact identity range");
        }
        if let Some(index) = output
            .baselines
            .first
            .and_then(|value| value.to_bits().checked_sub(TAG_BASE))
            .map(|value| value as usize)
            .filter(|&index| index < chunk.len())
        {
            return Ok(Some(chunk[index]));
        }
    }
    Ok(None)
}

struct Replay<'a, 'doc> {
    tree: &'a LayoutSession<'doc>,
    calls: &'a [Call],
    tags: HashMap<usize, f32>,
    next: usize,
    last: HashMap<usize, usize>,
    mismatch: bool,
    offset_bound: f64,
}

impl<'doc> TraversePartialTree for Replay<'_, 'doc> {
    type ChildIter<'a>
        = <LayoutSession<'doc> as TraversePartialTree>::ChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, parent: taffy::NodeId) -> Self::ChildIter<'_> {
        TraversePartialTree::child_ids(self.tree, parent)
    }
    fn child_count(&self, parent: taffy::NodeId) -> usize {
        TraversePartialTree::child_count(self.tree, parent)
    }
    fn get_child_id(&self, parent: taffy::NodeId, index: usize) -> taffy::NodeId {
        TraversePartialTree::get_child_id(self.tree, parent, index)
    }
}

impl LayoutPartialTree for Replay<'_, '_> {
    type CustomIdent = smol_str::SmolStr;
    type CoreContainerStyle<'a>
        = TaffyStyle<'a>
    where
        Self: 'a;

    // cov:ignore: Required by LayoutPartialTree; direct flex replay reads its specialized container style instead.
    fn get_core_container_style(&self, node: taffy::NodeId) -> Self::CoreContainerStyle<'_> {
        self.tree.get_core_container_style(node)
    }
    fn resolve_calc_value(&self, value: *const (), basis: f32) -> f32 {
        self.tree.resolve_calc_value(value, basis)
    }
    fn set_unrounded_layout(&mut self, node: taffy::NodeId, layout: &taffy::Layout) {
        let child = usize::from(NodeId::from_taffy(node));
        let Some(&index) = self.last.get(&child) else {
            return;
        };
        let call = &self.calls[index];
        let style = self.tree.query_style(child, ReadRole::FlexItem);
        let inset = style.inset();
        let relative = if style.position() == Position::Relative {
            inset
                .top
                .maybe_resolve(call.inputs.parent_size.height)
                .or_else(|| {
                    inset
                        .bottom
                        .maybe_resolve(call.inputs.parent_size.height)
                        .map(|bottom| -bottom)
                })
                .unwrap_or(0.0)
        } else {
            0.0
        };
        // Bound the unrounded offset and every real baseline, not the tagged
        // height. This also excludes a real result being mistaken for a tag.
        let bound = 2.0
            * (f64::from(layout.location.y).abs()
                + f64::from(relative).abs()
                + f64::from(call.output.size.height).abs()
                + f64::from(call.output.baselines.first.unwrap_or(0.0)).abs()
                + 1.0);
        if !bound.is_finite() {
            self.offset_bound = f64::INFINITY;
        } else {
            self.offset_bound = self.offset_bound.max(bound);
        }
        // All writes, including ordinary siblings' layouts, are discarded.
    }
    fn compute_child_layout(
        &mut self,
        node: taffy::NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        let node = NodeId::from_taffy(node);
        let inputs = LayoutInput::from_taffy(inputs);
        let index = self.next;
        self.next += 1;
        let Some(call) = self
            .calls
            .get(index)
            .filter(|call| call.node == node && call.inputs == inputs)
        else {
            self.mismatch = true;
            return LayoutOutput::HIDDEN.into_taffy();
        };
        let child = usize::from(node);
        self.last.insert(child, index);
        let mut output = call.output.clone();
        if inputs.run_mode == RunMode::PerformLayout
            && let Some(&tag) = self.tags.get(&child)
        {
            output.baselines.first = Some(tag);
            // The library clamps a scroll container's baseline to this size.
            // Cross size cannot affect row line allocation at this stage.
            output.size.height = tag;
        }
        output.into_taffy()
    }
}

impl LayoutFlexboxContainer for Replay<'_, '_> {
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
