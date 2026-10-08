use super::*;
use crate::fragment::{FragmentRect, LayoutFragment};
use raikiri_style::property::BreakInside;
use std::collections::HashMap;

pub(super) fn supports(tree: &Document, root: usize, context: FragmentationContext) -> bool {
    if !context
        .available_height
        .is_some_and(|height| height.is_finite() && height > 0.0)
        || !tree.nodes[root]
            .multicol
            .is_some_and(|style| style.horizontal)
    {
        return false;
    }
    let mut pending: Vec<_> = tree.nodes[root]
        .children
        .iter()
        .copied()
        .map(|id| (id, 0usize, true))
        .collect();
    let mut visible = false;
    let mut needs_break_flow = false;
    while let Some((id, depth, projected)) = pending.pop() {
        if depth >= 128 {
            return false;
        }
        let node = &tree.nodes[id];
        if !node.is_in_document() || node.style.display == Display::None {
            continue;
        }
        // Rendered children are elements or text; flat-tree membership
        // already excludes comments, processing instructions and fragments.
        if let NodeData::Text(text) = &node.data {
            if !text.text_content.trim().is_empty() {
                return false;
            }
            continue;
        }
        visible = true;
        if node.style.position == TaffyPosition::Absolute {
            continue;
        }
        // This projection preserves already measured atomic subtrees, but
        // does not reconstruct block margins, clearance or constrained wrapper sizes.
        // Leave those boxes to the existing geometry-preserving strategies.
        let zero = LengthPercentageAuto::length(0.0);
        let defaults: taffy::Style = taffy::Style::default();
        if projected
            && (node.style.margin.top != zero
                || node.style.margin.right != zero
                || node.style.margin.bottom != zero
                || node.style.margin.left != zero
                || node.style.padding != Rect::zero()
                || node.style.border != Rect::zero()
                || node.style.inset != Rect::auto()
                || node.style.clear != defaults.clear
                || (!atomic(tree, id)
                    && (!node.multicol_auto_width
                        || node.style.min_size != defaults.min_size
                        || node.style.max_size != defaults.max_size
                        || node.style.overflow != defaults.overflow)))
        {
            return false;
        }
        // Keep existing strategies for contexts whose only constraints are
        // inside avoidance, line breaking or sizing. This seam handles the
        // between-box constraints that require rollback and propagation.
        needs_break_flow |= forced(node.break_before)
            || forced(node.break_after)
            || avoided(node.break_before)
            || avoided(node.break_after)
            || matches!(node.break_inside, BreakInside::AvoidColumn);
        if node.multicol.is_some()
            || !matches!(
                node.display,
                DisplayValue::Block | DisplayValue::InlineBlock
            )
            || node.has_logical_min_block_size
        {
            return false;
        }
        pending.extend(
            node.children
                .iter()
                .copied()
                .map(|child| (child, depth + 1, projected && !atomic(tree, id))),
        );
    }
    visible && needs_break_flow
}

#[derive(Clone)]
struct FlowBox {
    node: usize,
    ancestors: Vec<usize>,
    width: f32,
    height: f32,
    inline_offset: f32,
    before: BreakBetween,
    after: BreakBetween,
    floated: bool,
    splittable: bool,
}

fn forced(value: BreakBetween) -> bool {
    matches!(value, BreakBetween::Always | BreakBetween::Column)
}

fn avoided(value: BreakBetween) -> bool {
    matches!(value, BreakBetween::Avoid | BreakBetween::AvoidColumn)
}

fn atomic(tree: &Document, id: usize) -> bool {
    let node = &tree.nodes[id];
    node.style.float.is_floated()
        || node.display == DisplayValue::InlineBlock
        || node.style.size.height != Dimension::auto()
        || matches!(
            node.break_inside,
            BreakInside::Avoid | BreakInside::AvoidColumn
        )
        || !node.children.iter().any(|&child| {
            tree.nodes[child].is_in_document()
                && tree.nodes[child].style.display != Display::None
                && matches!(tree.nodes[child].data, NodeData::Element(_))
        })
}

fn propagated(outer: BreakBetween, inner: BreakBetween) -> BreakBetween {
    if forced(inner) || (!forced(outer) && avoided(inner)) {
        inner
    } else if forced(outer) || avoided(outer) {
        outer
    } else {
        inner
    }
}

fn descendant_edge(tree: &Document, id: usize, before: bool, depth: usize) -> BreakBetween {
    let own = if before {
        tree.nodes[id].break_before
    } else {
        tree.nodes[id].break_after
    };
    if depth >= 128 {
        return own;
    }
    let children = &tree.nodes[id].children;
    let in_flow = |&&child: &&usize| {
        tree.nodes[child].is_in_document()
            && tree.nodes[child].style.display != Display::None
            && !tree.nodes[child].style.float.is_floated()
            && tree.nodes[child].style.position != TaffyPosition::Absolute
            && matches!(tree.nodes[child].data, NodeData::Element(_))
    };
    let child = if before {
        children.iter().find(in_flow)
    } else {
        children.iter().rev().find(in_flow)
    };
    child.map_or(own, |&child| {
        propagated(own, descendant_edge(tree, child, before, depth + 1))
    })
}

fn collect(
    tree: &mut Document,
    id: usize,
    context: FragmentationContext,
    ancestors: &mut Vec<usize>,
    out: &mut Vec<FlowBox>,
) {
    if !tree.nodes[id].is_in_document()
        || tree.nodes[id].style.display == Display::None
        || !matches!(tree.nodes[id].data, NodeData::Element(_))
        || tree.nodes[id].style.position == TaffyPosition::Absolute
    {
        return;
    }
    let floated = tree.nodes[id].style.float.is_floated();
    let input = LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width: (tree.nodes[id].multicol_auto_width && !floated).then_some(context.column_width),
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        parent_size: Size {
            width: Some(context.column_width),
            height: context.available_height,
        },
        available_space: Size {
            width: AvailableSpace::Definite(context.column_width),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: TaffyLine::FALSE,
    };
    let output = tree.compute_child_layout(TaffyNodeId::from(id), input);
    let before = descendant_edge(tree, id, true, 0);
    let after = descendant_edge(tree, id, false, 0);
    let children = tree.nodes[id].children.clone();
    if atomic(tree, id) {
        out.push(FlowBox {
            node: id,
            ancestors: ancestors.clone(),
            width: output.size.width,
            height: output.size.height.max(0.0),
            inline_offset: tree.nodes[id].unrounded_layout.location.x,
            before,
            after,
            floated,
            splittable: !floated
                && tree.nodes[id].display != DisplayValue::InlineBlock
                && !matches!(
                    tree.nodes[id].break_inside,
                    BreakInside::Avoid | BreakInside::AvoidColumn
                ),
        });
        return;
    }
    let start = out.len();
    ancestors.push(id);
    for child in children {
        collect(tree, child, context, ancestors, out);
        if tree.fragment_tree.limit_exceeded {
            break;
        }
    }
    ancestors.pop();
    if out.len() > start {
        out[start].before = propagated(before, out[start].before);
        let last = out.len() - 1;
        out[last].after = propagated(after, out[last].after);
    }
}

fn fragment(
    node: usize,
    parent: Option<usize>,
    column: usize,
    rect: FragmentRect,
) -> LayoutFragment {
    LayoutFragment {
        node_id: node,
        parent,
        fragmentainer: column,
        rect,
        fragmentainer_clip: None,
        fragment_index: 0,
        fragment_count: 1,
        line_start: None,
        line_end: None,
    }
}

pub(super) fn layout(
    tree: &mut Document,
    root: usize,
    context: FragmentationContext,
    fallback: f32,
) -> f32 {
    let Some(height) = context.available_height else {
        return fallback;
    };
    let mut boxes = Vec::new();
    for child in tree.nodes[root].children.clone() {
        collect(tree, child, context, &mut Vec::new(), &mut boxes);
        if tree.fragment_tree.limit_exceeded {
            return fallback;
        }
    }
    let Some(container) = tree.fragment_tree.try_push(fragment(
        root,
        None,
        context.column_index,
        FragmentRect {
            x: context.origin_x,
            y: context.origin_y,
            width: context.available_width,
            height: fallback,
        },
    )) else {
        return fallback;
    };
    // A lookahead run includes normal-flow siblings across parallel floats.
    // Floats occupy space for painting, but consume no block-flow advance.
    let mut runs = HashMap::<usize, f32>::new();
    let mut previous: Option<usize> = None;
    let mut start = 0;
    let mut advance = 0.0;
    for (index, item) in boxes.iter().enumerate() {
        if item.floated {
            continue;
        }
        let connected = previous.is_some_and(|prior| {
            !forced(boxes[prior].after)
                && !forced(item.before)
                && (avoided(boxes[prior].after) || avoided(item.before))
        });
        if connected {
            advance += item.height;
            runs.insert(start, advance);
        } else {
            start = index;
            advance = item.height;
        }
        previous = Some(index);
    }
    let mut column = 0usize;
    let mut cursor = 0.0f32;
    let mut previous_after = BreakBetween::Auto;
    let mut wrappers = HashMap::<(usize, usize), (usize, f32)>::new();
    for (index, item) in boxes.iter().enumerate() {
        let run = runs.get(&index).copied().unwrap_or(item.height);
        if !item.floated
            && cursor > 0.0
            && (forced(previous_after)
                || forced(item.before)
                || (run <= height && cursor + run > height)
                || (cursor + item.height > height && item.height <= height))
        {
            column = column.saturating_add(1);
            cursor = 0.0;
        }
        let mut remaining = item.height;
        let mut first = true;
        loop {
            if cursor >= height && !item.floated {
                column = column.saturating_add(1);
                cursor = 0.0;
            }
            let used = if item.splittable {
                remaining.min((height - cursor).max(0.0))
            } else {
                remaining
            };
            let x = context.column_offset_x(column);
            let y = cursor;
            let mut parent_fragment = container;
            let mut parent_y = 0.0;
            for &ancestor in &item.ancestors {
                let (id, top) = if let Some(&(id, top)) = wrappers.get(&(column, ancestor)) {
                    tree.fragment_tree.fragments[id].rect.height =
                        (y + used - top).max(tree.fragment_tree.fragments[id].rect.height);
                    (id, top)
                } else {
                    let Some(id) = tree.fragment_tree.try_push(fragment(
                        ancestor,
                        Some(parent_fragment),
                        column,
                        FragmentRect {
                            x: if parent_fragment == container { x } else { 0.0 },
                            y: y - parent_y,
                            width: context.column_width,
                            height: used,
                        },
                    )) else {
                        return fallback;
                    };
                    wrappers.insert((column, ancestor), (id, y));
                    (id, y)
                };
                parent_fragment = id;
                parent_y = top;
            }
            let Some(id) = tree.fragment_tree.try_push(fragment(
                item.node,
                Some(parent_fragment),
                column,
                FragmentRect {
                    x: if parent_fragment == container {
                        x + item.inline_offset
                    } else {
                        item.inline_offset
                    },
                    y: y - parent_y,
                    width: item.width,
                    height: used,
                },
            )) else {
                return fallback;
            };
            if first {
                let mut layout = tree.nodes[item.node].unrounded_layout;
                layout.size = Size {
                    width: item.width,
                    height: item.height,
                };
                layout.location = Point {
                    x: x + item.inline_offset,
                    y,
                };
                tree.set_unrounded_layout(TaffyNodeId::from(item.node), &layout);
                tree.fragment_tree.reparent_roots(item.node, id);
                first = false;
            }
            if !item.floated {
                cursor += used;
            }
            remaining -= used;
            if !item.splittable || remaining <= 0.001 {
                break;
            }
            column = column.saturating_add(1);
            cursor = 0.0;
        }
        if !item.floated {
            previous_after = item.after;
        }
    }
    fallback
}

#[cfg(test)]
mod tests;
