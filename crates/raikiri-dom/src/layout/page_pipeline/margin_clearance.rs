//! Conservative clearance facts from the layout before pagination moves boxes.

use super::{
    BoxParentScope, BoxTree, CascadeResult, ClearValue, DisplayValue, FloatValue, PageCandidate,
    PositionValue, clears_float_side, float_outer_extent, float_side_index, holds_floats,
    is_block_level_flow_box, is_floating_box_for_pagination,
};
use crate::layout::context::LayoutSession;
use raikiri_layout_style::values::AlignContentKeyword;
use std::collections::{HashMap, HashSet};

/// Clearing boxes whose margins can be treated like an ordinary block's.
#[derive(Default)]
pub(super) struct ClearanceFree(HashSet<usize>);

#[derive(Clone, Copy)]
struct FloatLimit {
    /// A shared direct layout parent, or unknown after a mixed/positioned float.
    parent: Option<usize>,
    bottom: f32,
}

impl ClearanceFree {
    /// Classify clear before any pagination shift, visiting source order once.
    pub(super) fn collect(
        document: &LayoutSession<'_>,
        cascade: &CascadeResult,
        candidates: &[PageCandidate],
    ) -> Self {
        if !candidates
            .iter()
            .any(|candidate| cascade.computed[candidate.node_id].clear != ClearValue::None)
        {
            return Self::default();
        }
        let tree = BoxTree::for_layout(document);
        let mut result = Self::default();
        let mut floats: HashMap<usize, [Option<FloatLimit>; 2]> = HashMap::new();
        let mut committed = HashSet::new();
        let mut pending = vec![(document.root, document.root)];
        while let Some((id, context)) = pending.pop() {
            let node = &document.nodes[id];
            let contents = node.display() == DisplayValue::Contents;
            if node.display() == DisplayValue::None
                || !contents
                    && matches!(
                        node.position(),
                        PositionValue::Absolute | PositionValue::Fixed
                    )
            {
                continue;
            }
            let parent = tree.parent(id, BoxParentScope::Document);
            let floating = is_floating_box_for_pagination(document, cascade, id);
            let independent = !contents && holds_floats(document, cascade, id);
            let sides = floats.entry(context).or_default();
            let clear = cascade.computed[id].clear;
            if clear != ClearValue::None && !floating {
                let style = document.pagination_style(id);
                // A committed sibling ends the parent's escaping top-margin
                // strut. Used clearance would then pin this static border
                // edge to a matching float bottom, so only a strict gap proves
                // the float is expired. Unknown parents remain conservative.
                let ordinary = matches!(node.position(), PositionValue::Static)
                    && !node.used_writing_mode.is_vertical()
                    && !independent
                    && is_block_level_flow_box(node.display())
                    && style.is_block()
                    && !style.is_replaced()
                    && !style.avoids_floats()
                    && !style.is_table()
                    && style.bridge_align_content().keyword == AlignContentKeyword::Normal
                    && document.layout_parent_of(id) == parent
                    && parent.is_some_and(|parent| {
                        committed.contains(&parent)
                            && document.nodes[parent].used_writing_mode == node.used_writing_mode
                    });
                let safe = sides.iter().enumerate().all(|(side, limit)| {
                    if !clears_float_side(clear, [FloatValue::Left, FloatValue::Right][side]) {
                        return true;
                    }
                    limit.is_none_or(|limit| {
                        ordinary
                            && limit.parent.is_some()
                            && limit.parent == parent
                            && node.layout.location.y > limit.bottom + 0.001
                    })
                });
                // A performed block layout can prove the hypothetical top
                // already cleared the float, even in an escaping margin strut.
                let measured_free = matches!(node.position(), PositionValue::Static)
                    && !node.used_writing_mode.is_vertical()
                    && !independent
                    && document.context.block_clearance.get(&id) == Some(&false);
                if safe || measured_free {
                    result.0.insert(id);
                }
            }
            if floating {
                if let Some(side) = float_side_index(cascade.computed[id].float) {
                    let parent = if matches!(node.position(), PositionValue::Static)
                        && document.layout_parent_of(id) == parent
                    {
                        parent
                    } else {
                        None
                    };
                    let bottom = float_outer_extent(document, id).1;
                    sides[side] =
                        Some(
                            sides[side].map_or(FloatLimit { parent, bottom }, |old| FloatLimit {
                                parent: old.parent.filter(|&old_parent| Some(old_parent) == parent),
                                bottom: old.bottom.max(bottom),
                            }),
                        );
                }
            } else if !contents
                && node.layout.size.height > 0.0
                && is_block_level_flow_box(node.display())
                && let Some(parent) = parent
            {
                committed.insert(parent);
            }
            // A float contributes to its parent's context, but contains its
            // own floats. Contents boxes remain transparent to both walks.
            let child_context = if independent || floating { id } else { context };
            pending.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|&child| (child, child_context)),
            );
        }
        result
    }

    pub(super) fn contains(&self, id: usize) -> bool {
        self.0.contains(&id)
    }
}
