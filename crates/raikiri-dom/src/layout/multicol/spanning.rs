//! Place full-width spanners between independently laid-out column groups.

use super::*;
use taffy::util::MaybeResolve;

pub(super) fn is_spanner(tree: &Document, child: usize) -> bool {
    let node = &tree.nodes[child];
    node.column_span_all
        && node.is_in_document()
        && matches!(node.display, DisplayValue::Block | DisplayValue::ListItem)
        && node.style.display != Display::None
        && node.style.position != TaffyPosition::Absolute
        && !node.style.float.is_floated()
}

pub(super) fn retire_preliminary_fragments(tree: &mut Document, children: &[usize]) {
    // The preliminary block measurement lays out nested column containers.
    // Spanners are subsequently laid out outside the owner's column stack;
    // retire obsolete records without invalidating a caller's container index.
    let mut pending: Vec<_> = children
        .iter()
        .copied()
        .filter(|&child| is_spanner(tree, child))
        .collect();
    let mut removed = std::collections::HashSet::new();
    while let Some(id) = pending.pop() {
        removed.insert(id);
        pending.extend(tree.nodes[id].children.iter().copied());
    }
    tree.fragment_tree.retire_nodes(&removed);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn layout(
    tree: &mut Document,
    owner: usize,
    children: &[usize],
    container_fragment: usize,
    context: FragmentationContext,
    border_box_size: Size<f32>,
    content_origin: Point<f32>,
    block_end_inset: f32,
) -> f32 {
    let mut start = 0;
    let mut block_offset = content_origin.y;
    let mut previous_margin: Option<f32> = None;
    for end in 0..=children.len() {
        let spanner = children
            .get(end)
            .copied()
            .filter(|&child| is_spanner(tree, child));
        if end < children.len() && spanner.is_none() {
            continue;
        }
        if children[start..end].iter().any(|&child| {
            let node = &tree.nodes[child];
            node.kind() != NodeKind::Text
                || node.unrounded_layout.size.height != 0.0
                || node.ifc.is_some()
        }) {
            previous_margin = None;
            // A group preceding a spanner is balanced even under auto fill.
            let group_context = FragmentationContext {
                column_fill: if spanner.is_some() {
                    ColumnFillValue::Balance
                } else {
                    context.column_fill
                },
                available_height: None,
                origin_y: block_offset,
                ..context
            };
            if let Some(active) = tree.fragmentation_stack.last_mut() {
                *active = group_context;
            }
            block_offset = layout_column_group(
                tree,
                owner,
                &children[start..end],
                start,
                container_fragment,
                group_context,
                Size {
                    height: 0.0,
                    ..border_box_size
                },
                Point {
                    y: block_offset,
                    ..content_origin
                },
                0.0,
                true,
            );
        }
        if let Some(child) = spanner {
            // A spanner establishes an independent formatting context. The
            // owner's column stack must not fragment its text or descendants.
            let stack = std::mem::take(&mut tree.fragmentation_stack);
            clear_column_state(tree, child);
            let fragment_start = tree.fragment_tree.fragments.len();
            let margin = &tree.nodes[child].style.margin;
            let inline_margin = [margin.left, margin.right]
                .into_iter()
                .filter_map(|value| {
                    value.maybe_resolve(
                        Some(context.available_width),
                        crate::taffy_impl::resolve_calc,
                    )
                })
                .sum::<f32>();
            let width = tree.nodes[child]
                .multicol_auto_width
                .then_some((context.available_width - inline_margin).max(0.0));
            let output = tree.compute_child_layout(
                TaffyNodeId::from(child),
                LayoutInput {
                    run_mode: RunMode::PerformLayout,
                    sizing_mode: SizingMode::InherentSize,
                    axis: RequestedAxis::Both,
                    known_dimensions: Size {
                        width,
                        height: None,
                    },
                    known_dimensions_are_definite: Size {
                        width: width.is_some(),
                        height: false,
                    },
                    parent_size: Size {
                        width: Some(context.available_width),
                        height: None,
                    },
                    available_space: Size {
                        width: AvailableSpace::Definite(context.available_width),
                        height: AvailableSpace::MaxContent,
                    },
                    vertical_margins_are_collapsible: TaffyLine::FALSE,
                },
            );
            tree.fragmentation_stack = stack;
            let mut child_layout = tree.nodes[child].unrounded_layout;
            child_layout.order = end as u32;
            child_layout.size = output.size;
            let margin_top = child_layout.margin.top;
            let adjoining = previous_margin.map_or(margin_top, |bottom| {
                bottom.max(margin_top).max(0.0) + bottom.min(margin_top).min(0.0) - bottom
            });
            child_layout.location = Point {
                x: content_origin.x + child_layout.margin.left,
                y: block_offset + adjoining,
            };
            tree.set_unrounded_layout(TaffyNodeId::from(child), &child_layout);
            let Some(fragment) = tree
                .fragment_tree
                .try_push(crate::fragment::LayoutFragment {
                    node_id: child,
                    parent: Some(container_fragment),
                    fragmentainer: 0,
                    rect: crate::fragment::FragmentRect {
                        x: child_layout.location.x,
                        y: child_layout.location.y,
                        width: output.size.width,
                        height: output.size.height,
                    },
                    fragmentainer_clip: None,
                    fragment_index: 0,
                    fragment_count: 1,
                    line_start: None,
                    line_end: None,
                })
            else {
                return border_box_size.height;
            };
            for index in fragment_start..fragment {
                if tree.fragment_tree.fragments[index].parent.is_some() {
                    continue;
                }
                // Root records were relative to their source node. Once
                // attached, retain intervening wrapper offsets relative to
                // the spanner fragment instead.
                let mut id = tree.fragment_tree.fragments[index].node_id;
                let mut offset = Point::ZERO;
                while id != child {
                    let location = tree.nodes[id].unrounded_layout.location;
                    offset.x += location.x;
                    offset.y += location.y;
                    id = tree
                        .layout_parent_of(id)
                        .expect("spanner roots remain in their source subtree");
                }
                let nested = &mut tree.fragment_tree.fragments[index];
                nested.rect.x += offset.x;
                nested.rect.y += offset.y;
                nested.parent = Some(fragment);
            }
            block_offset =
                child_layout.location.y + output.size.height + child_layout.margin.bottom;
            previous_margin = Some(child_layout.margin.bottom);
        }
        start = end + 1;
    }
    block_offset + block_end_inset
}

fn clear_column_state(tree: &mut Document, root: usize) {
    let mut pending = vec![root];
    while let Some(node_id) = pending.pop() {
        let node = &mut tree.nodes[node_id];
        node.cache.clear();
        if node_id != root && node.multicol.is_some() {
            continue;
        }
        if let Some(root) = node.ifc.as_mut() {
            root.multicol_fragments = None;
            root.multicol_fragment_origins_recorded = false;
        }
        pending.extend(node.children.iter().copied());
    }
}
