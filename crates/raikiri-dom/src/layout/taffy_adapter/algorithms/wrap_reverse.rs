//! Preserve reversed cross-start when distributed flex alignment overflows.
//!
//! Replay recorded child results with positional alignment. No child is laid
//! out again, and changed sizing, margins, or callback schedules reject the
//! replay. A second positional replay distinguishes overflow from stretch
//! arithmetic without inferring private flex lines from item rectangles.

use std::collections::{HashMap, HashSet};

use super::super::convert::{FromTaffy, IntoTaffy};
use super::FlexReplayCall;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutInput, LayoutOutput, NodeId, RunMode};
use crate::layout::style_views::ReadRole;
use raikiri_layout_geometry::BoxLayout;
use raikiri_layout_style::TaffyStyle;
use raikiri_layout_style::values::{
    AlignContent, AlignContentKeyword, AlignSelf, AlignmentSafety, Direction, Position,
};
use taffy::{LayoutFlexboxContainer, LayoutPartialTree, TraversePartialTree};

pub(crate) struct Correction {
    pub(crate) output: LayoutOutput,
    pub(crate) layouts: Vec<(usize, BoxLayout)>,
}

pub(crate) fn required(tree: &LayoutSession<'_>, node: NodeId) -> bool {
    let style = tree.query_style(usize::from(node), ReadRole::FlexContainer);
    style.flex_lines_are_reversed()
        && matches!(
            style.align_content().keyword,
            AlignContentKeyword::Normal
                | AlignContentKeyword::Stretch
                | AlignContentKeyword::SpaceBetween
        )
}

pub(crate) fn candidate(
    tree: &LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    calls: &[FlexReplayCall],
    original: &LayoutOutput,
    lookup: impl Fn(usize) -> Option<BoxLayout>,
) -> Option<Correction> {
    if inputs.run_mode != RunMode::PerformLayout || !required(tree, node) {
        return None;
    }
    let (output, layouts) = replay(tree, node, inputs, calls, AlignContentKeyword::FlexStart)?;
    if output.size != original.size {
        return None;
    }
    let style = tree.query_style(usize::from(node), ReadRole::FlexContainer);
    let row = style.flex_direction_is_row();
    let outward = if !row && style.direction() == Direction::Rtl {
        1.0
    } else {
        -1.0
    };
    let mut updates = Vec::new();
    let mut common_shift = (f64::NEG_INFINITY, f64::INFINITY);
    for child in TraversePartialTree::child_ids(tree, node.into_taffy()) {
        let child = usize::from(NodeId::from_taffy(child));
        let item = tree.query_style(child, ReadRole::FlexItem);
        // Native flex emits absolute children as candidates, without writing
        // their layouts. Their static positions live in the replay output.
        if item.position() == Position::Absolute {
            continue;
        }
        let old = lookup(child)?;
        let new = *layouts.get(&child)?;
        let mut unchanged = new;
        unchanged.location = old.location;
        if unchanged != old {
            return None;
        }
        if tree.nodes[child].is_display_none() {
            if new != old {
                return None;
            }
        } else {
            let (a, b, main_a, main_b) = if row {
                (
                    old.location.y,
                    new.location.y,
                    old.location.x,
                    new.location.x,
                )
            } else {
                (
                    old.location.x,
                    new.location.x,
                    old.location.y,
                    new.location.y,
                )
            };
            let delta = f64::from(b) - f64::from(a);
            if !a.is_finite() || !b.is_finite() || main_a != main_b || delta * outward < 0.0 {
                return None;
            }
            // Intersect each child's rounding interval. A large relative
            // inset may quantize its shift more coarsely than its siblings;
            // it must not loosen comparisons between the smaller siblings.
            let tolerance = 8.0
                * f64::from(f32::EPSILON)
                * f64::from(a)
                    .abs()
                    .max(f64::from(b).abs())
                    .max(delta.abs())
                    .max(1.0);
            common_shift.0 = common_shift.0.max(delta - tolerance);
            common_shift.1 = common_shift.1.min(delta + tolerance);
        }
        updates.push((child, new));
    }
    if common_shift.0 > common_shift.1 {
        return None;
    }
    // Both positional alignments size the lines identically. Only negative
    // free space can put flex-start outward of logical start. Stretch's
    // different arithmetic alone can otherwise produce a one-ULP false hit.
    let (control_output, control) = replay(tree, node, inputs, calls, AlignContentKeyword::Start)?;
    // A row baseline excludes relative insets. It can retain a line shift
    // that rounded away when a large inset was added to item locations.
    let mut overflow = row
        && output
            .baselines
            .first
            .zip(control_output.baselines.first)
            .is_some_and(|(end, start)| {
                let delta = f64::from(end) - f64::from(start);
                delta.is_finite() && delta < 0.0
            });
    for &(child, new) in &updates {
        let start = *control.get(&child)?;
        if tree.nodes[child].is_display_none() {
            continue;
        }
        let delta = if row {
            f64::from(new.location.y) - f64::from(start.location.y)
        } else {
            f64::from(new.location.x) - f64::from(start.location.x)
        };
        if !delta.is_finite() || delta * outward < 0.0 {
            return None;
        }
        overflow |= delta * outward > 0.0;
    }
    overflow.then_some(Correction {
        output,
        layouts: updates,
    })
}

fn replay(
    tree: &LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    calls: &[FlexReplayCall],
    keyword: AlignContentKeyword,
) -> Option<(LayoutOutput, HashMap<usize, BoxLayout>)> {
    replay_with(
        tree,
        node,
        inputs,
        calls,
        Overrides {
            content: Some(AlignContent {
                keyword,
                safety: AlignmentSafety::Unsafe,
            }),
            items: None,
        },
    )
}

/// What a replay changes in the style the container is laid out with. Neither
/// alters the size a child is asked for, so the recorded results still apply.
#[derive(Clone, Copy)]
pub(crate) struct Overrides<'a> {
    /// The alignment of the flex lines, in place of the container's own.
    pub(crate) content: Option<AlignContent>,
    /// The items that are placed by another cross-axis alignment, and that
    /// alignment: without the offset of a relative box, which moves the item
    /// and not the line it is in.
    pub(crate) items: Option<(&'a HashSet<usize>, AlignSelf)>,
}

/// Lay the container `node` out again from the recorded results of its
/// children, with the `overrides` applied, and return what that places where.
pub(crate) fn replay_with(
    tree: &LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    calls: &[FlexReplayCall],
    overrides: Overrides<'_>,
) -> Option<(LayoutOutput, HashMap<usize, BoxLayout>)> {
    let mut replay = Replay {
        tree,
        calls,
        next: 0,
        mismatch: false,
        layouts: HashMap::new(),
        overrides,
    };
    let inputs = super::with_stretch_width(tree, node, inputs);
    let output = LayoutOutput::from_taffy(taffy::compute_flexbox_layout(
        &mut replay,
        node.into_taffy(),
        inputs.into_taffy(),
    ));
    (!replay.mismatch && replay.next == calls.len()).then_some((output, replay.layouts))
}

struct Replay<'a, 'doc> {
    tree: &'a LayoutSession<'doc>,
    calls: &'a [FlexReplayCall],
    next: usize,
    mismatch: bool,
    layouts: HashMap<usize, BoxLayout>,
    overrides: Overrides<'a>,
}

impl<'doc> TraversePartialTree for Replay<'_, 'doc> {
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
        let node = NodeId::from_taffy(node);
        let inputs = LayoutInput::from_taffy(inputs);
        let call = self.calls.get(self.next);
        self.next += 1;
        if let Some(call) = call.filter(|call| call.node == node && call.inputs == inputs) {
            call.output.clone().into_taffy()
        } else {
            self.mismatch = true;
            LayoutOutput::HIDDEN.into_taffy()
        }
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
        let style = self.tree.get_flexbox_container_style(node);
        match self.overrides.content {
            Some(alignment) => style.with_flex_align_content(alignment),
            None => style,
        }
    }
    fn get_flexbox_child_style(&self, node: taffy::NodeId) -> Self::FlexboxItemStyle<'_> {
        let style = self.tree.get_flexbox_child_style(node);
        match self.overrides.items {
            Some((items, alignment)) if items.contains(&usize::from(NodeId::from_taffy(node))) => {
                style.with_flex_line_probe(alignment)
            }
            _ => style,
        }
    }
}
