//! Place full-width spanners between independently laid-out column groups.

use super::*;
use crate::layout::context::LayoutSession;
use raikiri_layout_style::values::MaybeResolve;

/// Whether `child` is a `column-span: all` block that is not a float, read
/// through the style views of a layout session.
pub(crate) fn is_spanner(tree: &LayoutSession<'_>, child: usize) -> bool {
    spans_all_columns(layout_node(tree, child)) && !is_floated(tree, child)
}

/// [`is_spanner`] for callers that read the computed values: the box
/// preparation, which runs before the style views are ready, and code that
/// runs after the layout session has ended. No layout pass overrides `float`,
/// so both answer alike.
pub(crate) fn is_spanner_in(doc: &Document, cascade: &CascadeResult, child: usize) -> bool {
    spans_all_columns(layout_node(doc, child))
        && !AuthoredBox::new(doc, cascade, child).is_floated()
}

/// `column-span: all` applies to the block-level boxes that are in the flow
/// (CSS Multi-column Layout 1, 6.1), whatever their inner display is.
fn spans_all_columns(node: &crate::node::Node) -> bool {
    node.column_span_all
        && node.is_in_document()
        && matches!(
            node.display(),
            DisplayValue::Block
                | DisplayValue::ListItem
                | DisplayValue::FlowRoot
                | DisplayValue::Flex
                | DisplayValue::Grid
                | DisplayValue::Table
        )
        && !node.is_display_none()
        && node.layout_position() != Position::Absolute
}

pub(super) fn retire_preliminary_fragments(tree: &mut LayoutSession<'_>, children: &[usize]) {
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
    tree: &mut LayoutSession<'_>,
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
            let node = layout_node(tree, child);
            node.kind() != NodeKind::Text || node.layout.size.height != 0.0 || node.ifc.is_some()
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
            if let Some(active) = tree.context.fragmentation_stack.last_mut() {
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
            let stack = std::mem::take(&mut tree.context.fragmentation_stack);
            clear_column_state(tree, child);
            let fragment_start = tree.fragment_tree.fragments.len();
            let margin = style_of(tree, child).margin();
            let inline_margin = [margin.left, margin.right]
                .into_iter()
                .filter_map(|value| value.maybe_resolve(Some(context.available_width)))
                .sum::<f32>();
            let width = tree.nodes[child]
                .multicol_auto_width
                .then_some((context.available_width - inline_margin).max(0.0));
            // A button fits its content within the room its margins leave, not
            // the width of the container (HTML Standard 15.5.3).
            let available_width = match width {
                Some(room) if layout_node(tree, child).is_html_button() => room,
                _ => context.available_width,
            };
            let output = tree.compute_child_layout(
                NodeId::from(child),
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
                        width: AvailableSpace::Definite(available_width),
                        height: AvailableSpace::MaxContent,
                    },
                    vertical_margins_are_collapsible: Line::FALSE,
                },
            );
            tree.context.fragmentation_stack = stack;
            let mut child_layout = tree.nodes[child].layout;
            child_layout.order = end as u32;
            child_layout.size = output.size;
            let margin_top = child_layout.margin.top;
            let adjoining = previous_margin.map_or(margin_top, |bottom| {
                bottom.max(margin_top).max(0.0) + bottom.min(margin_top).min(0.0) - bottom
            });
            // CSS 2.2 10.3.3: a box that overflows its container counts auto
            // margins as zero, and a box with no auto margin is
            // over-constrained. Either way the margin on the end side of the
            // container's direction gives, so in a right-to-left container the
            // box keeps its right margin and sits against the right edge.
            let room = context.available_width
                - output.size.width
                - child_layout.margin.left
                - child_layout.margin.right;
            let over_constrained =
                (!margin.left.is_auto() && !margin.right.is_auto()) || room < 0.0;
            let left = if context.rtl && over_constrained {
                context.available_width - output.size.width - child_layout.margin.right
            } else {
                child_layout.margin.left
            };
            child_layout.location = Point {
                x: content_origin.x + left,
                y: block_offset + adjoining,
            };
            tree.set_unrounded_layout(NodeId::from(child), &child_layout);
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
                    let location = tree.nodes[id].layout.location;
                    offset.x += location.x;
                    offset.y += location.y;
                    id = tree
                        .layout_parent_of(id)
                        .expect("spanner roots remain in their source subtree");
                }
                let nested = &mut tree.fragment_tree.fragments[index];
                nested.rect.x += offset.x;
                nested.rect.y += offset.y;
                tree.fragment_tree.attach_root(index, fragment);
            }
            block_offset =
                child_layout.location.y + output.size.height + child_layout.margin.bottom;
            previous_margin = Some(child_layout.margin.bottom);
        }
        start = end + 1;
    }
    block_offset + block_end_inset
}

pub(super) fn clear_column_state(tree: &mut LayoutSession<'_>, root: usize) {
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
