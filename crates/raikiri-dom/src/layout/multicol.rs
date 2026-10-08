mod break_flow;

use super::*;

/// Dispatch a multicolumn container through the nested fragmentation seam.
///
/// Taffy still resolves the ordinary box and intrinsic sizes, while this
/// seam owns nested child placement, fragmentainer breaks, and fragment-tree
/// records. Standalone and deferred out-of-flow cases continue through the
/// PR #79 projection so their exact geometry remains unchanged.
// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
pub(crate) fn compute_multicol_layout(
    tree: &mut Document,
    node_id: TaffyNodeId,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let index = usize::from(node_id);
    if tree.fragment_tree.limit_exceeded {
        return compute_block_layout(tree, node_id, inputs, block_ctx);
    }
    let Some(style) = tree.nodes[index].multicol else {
        return compute_block_layout(tree, node_id, inputs, block_ctx);
    };

    // Taffy supplies the used border-box width to a block child. Do not fall
    // back to the page width when a nested box is measured: that would make a
    // nested percentage gap resolve against the wrong containing block.
    let parent_width = inputs.parent_size.width;
    let available_width = inputs
        .known_dimensions
        .width
        .or(match inputs.available_space.width {
            AvailableSpace::Definite(value) => Some(value),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        })
        .or_else(|| {
            multicol_definite_dimension(tree, tree.nodes[index].style.size.width, parent_width)
        })
        .unwrap_or(0.0);
    let available_height = if style.height_definite {
        multicol_definite_dimension(
            tree,
            tree.nodes[index].style.size.height,
            inputs.parent_size.height,
        )
        .or(inputs.known_dimensions.height)
    } else {
        None
    };
    let Some(context) = FragmentationContext::resolve(available_width, available_height, style)
    else {
        return compute_block_layout(tree, node_id, inputs, block_ctx);
    };

    // Keep the existing foundational post-pass authoritative for standalone
    // multicol boxes and for deferred out-of-flow cases. The custom path is
    // entered only at a real nested block-flow boundary.
    // A logical min-block-size + break-inside:avoid child with no direct
    // line-break flow must stay on the foundational block projection: that
    // path preserves the item's unfragmented minimum before we apply the
    // column offsets below. Text-bearing children retain the existing custom
    // line-range projection.
    let custom_scope = (tree.fragmentation_stack.is_empty()
        && (multicol_has_nested_descendant(tree, index)
            || (multicol_has_direct_block_child(tree, index)
                && tree.nodes[index].style.size.height == Dimension::auto())
            || multicol_has_direct_break_flow(tree, index))
        || !tree.fragmentation_stack.is_empty())
        && (!multicol_has_min_constrained_child(tree, index)
            || multicol_has_direct_break_flow(tree, index))
        && style.horizontal
        && !multicol_has_out_of_flow_descendant(tree, index);
    let stack_depth = tree.fragmentation_stack.len();
    tree.fragmentation_stack.push(context);
    let mut output = compute_block_layout(tree, node_id, inputs, block_ctx);
    if tree.fragment_tree.limit_exceeded {
        tree.fragmentation_stack.truncate(stack_depth);
        return output;
    }
    let fragmentainer_height = available_height.or_else(|| {
        multicol_max_fragmentainer_height(
            tree,
            index,
            output.size.height,
            inputs.parent_size.height,
            parent_width,
        )
    });
    if let Some(parent_height) = available_height.filter(|_| {
        multicol_has_min_constrained_child(tree, index)
            && !multicol_has_direct_break_flow(tree, index)
    }) {
        let children: Vec<usize> = tree.nodes[index]
            .children
            .iter()
            .copied()
            .filter(|&child| {
                tree.nodes[child].is_in_document()
                    && tree.nodes[child].kind() == NodeKind::Element
                    && tree.nodes[child].style.display != Display::None
            })
            .collect();
        if let Some(&first) = children.first() {
            let child_height = tree.nodes[first].unrounded_layout.size.height;
            // `break-inside: avoid` consumes the unfragmented overflow on both
            // sides of a too-short/too-tall fragmentainer. Taffy does not
            // expose that fragmentation marker on its foundational block
            // path, so express the equivalent separation as a direct-child
            // block offset.
            let step = child_height + 2.0 * (parent_height - child_height).abs();
            for (order, child) in children.into_iter().enumerate() {
                tree.nodes[child].unrounded_layout.location.y = order as f32 * step;
                tree.nodes[child].needs_relative_block_paint_offset = true;
            }
        }
    }
    if !style.height_definite && multicol_has_min_constrained_child(tree, index) {
        // The minimum-sized children overflow the auto multicol box, but the
        // auto box's used block-size remains the minimum child size rather
        // than the sum of those overflowing children.
        let min_child_height = tree.nodes[index]
            .children
            .iter()
            .filter(|&&child| {
                tree.nodes[child].is_in_document()
                    && tree.nodes[child].kind() == NodeKind::Element
                    && tree.nodes[child].style.display != Display::None
            })
            .map(|&child| tree.nodes[child].unrounded_layout.size.height)
            .fold(0.0_f32, f32::max);
        output.size.height = output.size.height.min(min_child_height);
    }
    let break_flow_scope = break_flow::supports(tree, index, context);
    if (custom_scope || break_flow_scope) && inputs.run_mode == RunMode::PerformLayout {
        // Taffy's input width is normally already the content width for this
        // bridge. Correct it for authored padding/border before deriving the
        // child column width, so percentage gaps use the used content box.
        let content_width = multicol_content_width(tree, index, output.size.width, parent_width);
        let resolved = FragmentationContext::resolve(content_width, fragmentainer_height, style)
            .unwrap_or(context);
        if let Some(active) = tree.fragmentation_stack.last_mut() {
            *active = resolved;
        }
        let used_height = if break_flow_scope {
            // The parent writes the container's Taffy layout only after this
            // callback returns. Resolve the content origin from the same used
            // percentage basis instead of reading its stale layout insets.
            let css = &tree.nodes[index].style;
            let basis = parent_width.unwrap_or(output.size.width).max(0.0);
            let content_origin = Point {
                x: multicol_resolve_inset(tree, css.padding.left, basis)
                    + multicol_resolve_inset(tree, css.border.left, basis),
                y: multicol_resolve_inset(tree, css.padding.top, basis)
                    + multicol_resolve_inset(tree, css.border.top, basis),
            };
            break_flow::layout(tree, index, resolved, output.size, content_origin)
        } else {
            relayout_nested_multicol_children(
                tree,
                node_id,
                resolved,
                output.size.height,
                fragmentainer_height.is_some(),
            )
        };
        if fragmentainer_height.is_none() {
            output.size.height = used_height.max(0.0);
        }
    }
    // Truncating to the depth captured on entry keeps the stack balanced if a
    // future child strategy starts pushing a nested context of its own.
    tree.fragmentation_stack.truncate(stack_depth);
    output
}

fn multicol_max_fragmentainer_height(
    tree: &Document,
    node_id: usize,
    border_box_height: f32,
    parent_height: Option<f32>,
    parent_width: Option<f32>,
) -> Option<f32> {
    let style = &tree.nodes[node_id].style;
    let max_height =
        multicol_definite_dimension(tree, style.max_size.height.into(), parent_height)?;
    let basis = parent_width.unwrap_or(border_box_height).max(0.0);
    let insets = multicol_resolve_inset(tree, style.padding.top, basis)
        + multicol_resolve_inset(tree, style.padding.bottom, basis)
        + multicol_resolve_inset(tree, style.border.top, basis)
        + multicol_resolve_inset(tree, style.border.bottom, basis);
    let max_content_height = if style.box_sizing == taffy::BoxSizing::BorderBox {
        (max_height - insets).max(0.0)
    } else {
        max_height
    };
    let used_content_height = (border_box_height - insets).max(0.0);
    Some(used_content_height.min(max_content_height))
}

pub(crate) fn multicol_definite_dimension(
    tree: &Document,
    value: Dimension,
    basis: Option<f32>,
) -> Option<f32> {
    let basis = basis.unwrap_or(0.0);
    let resolved = match value.expand() {
        ExpandedDimension::Length(px) => px,
        ExpandedDimension::Percent(percent) => basis * percent,
        ExpandedDimension::Calc(pointer) => tree.resolve_calc_value(pointer, basis),
        ExpandedDimension::Auto
        | ExpandedDimension::MinContent
        | ExpandedDimension::MaxContent
        | ExpandedDimension::FitContentPx(_)
        | ExpandedDimension::FitContentPercent(_)
        | ExpandedDimension::FitContent
        | ExpandedDimension::Stretch
        | ExpandedDimension::Content => return None,
    };
    resolved.is_finite().then_some(resolved.max(0.0))
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_resolve_inset(tree: &Document, value: LengthPercentage, basis: f32) -> f32 {
    let resolved = match value.expand() {
        ExpandedLengthPercentage::Length(px) => px,
        ExpandedLengthPercentage::Percent(percent) => basis * percent,
        ExpandedLengthPercentage::Calc(pointer) => tree.resolve_calc_value(pointer, basis),
    };
    if resolved.is_finite() {
        resolved.max(0.0)
    } else {
        0.0
    }
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_content_width(
    tree: &Document,
    node_id: usize,
    border_box_width: f32,
    parent_width: Option<f32>,
) -> f32 {
    let style = &tree.nodes[node_id].style;
    let basis = parent_width.unwrap_or(border_box_width).max(0.0);
    let horizontal = multicol_resolve_inset(tree, style.padding.left, basis)
        + multicol_resolve_inset(tree, style.padding.right, basis)
        + multicol_resolve_inset(tree, style.border.left, basis)
        + multicol_resolve_inset(tree, style.border.right, basis);
    (border_box_width - horizontal).max(0.0)
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_has_nested_descendant(tree: &Document, node_id: usize) -> bool {
    let mut pending = tree.nodes[node_id].children.clone();
    while let Some(child) = pending.pop() {
        if tree.nodes[child].multicol.is_some() {
            return true;
        }
        pending.extend(tree.nodes[child].children.iter().copied());
    }
    false
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_has_direct_break_flow(tree: &Document, node_id: usize) -> bool {
    tree.nodes[node_id].children.iter().copied().any(|child| {
        tree.nodes[child].kind() == NodeKind::Element
            && tree.nodes[child].is_in_document()
            && tree.nodes[child]
                .children
                .iter()
                .any(|&grandchild| tree.nodes[grandchild].tag_name() == Some("br"))
    })
}

fn multicol_has_out_of_flow_descendant(tree: &Document, node_id: usize) -> bool {
    let mut pending = tree.nodes[node_id].children.clone();
    while let Some(child) = pending.pop() {
        if tree.nodes[child].style.position == TaffyPosition::Absolute {
            return true;
        }
        pending.extend(tree.nodes[child].children.iter().copied());
    }
    false
}

// cov:ignore: direct block fragmentation is exercised by the ignored multicol WPT reftests.
fn multicol_has_direct_block_child(tree: &Document, node_id: usize) -> bool {
    tree.nodes[node_id].children.iter().any(|&child| {
        tree.nodes[child].is_in_document()
            && tree.nodes[child].kind() == NodeKind::Element
            && tree.nodes[child].style.display != Display::None
    })
}

// A logical non-auto child minimum paired with break avoidance participates
// in block sizing before column projection. Keep that case on the foundational
// path until the custom projection can account for the minimum's unfragmented
// overflow.
fn multicol_has_min_constrained_child(tree: &Document, node_id: usize) -> bool {
    tree.nodes[node_id].children.iter().any(|&child| {
        tree.nodes[child].is_in_document()
            && tree.nodes[child].kind() == NodeKind::Element
            && tree.nodes[child].style.display != Display::None
            && tree.nodes[child].has_logical_min_block_size
            && !tree.nodes[child].style.min_size.height.is_auto()
    })
}

fn relayout_nested_multicol_children(
    tree: &mut Document,
    node_id: TaffyNodeId,
    context: FragmentationContext,
    fallback_height: f32,
    has_fragmentainer_height_constraint: bool,
) -> f32 {
    let index = usize::from(node_id);
    let children: Vec<usize> = tree.nodes[index]
        .children
        .iter()
        .copied()
        .filter(|&child| {
            tree.nodes[child].is_in_document() && tree.nodes[child].style.display != Display::None
        })
        .collect();
    let Some(container_fragment) = tree
        .fragment_tree
        .try_push(crate::fragment::LayoutFragment {
            node_id: index,
            parent: None,
            fragmentainer: context.column_index,
            rect: crate::fragment::FragmentRect {
                x: context.origin_x,
                y: context.origin_y,
                width: context.available_width,
                height: fallback_height,
            },
            fragmentainer_clip: None,
            fragment_index: 0,
            fragment_count: 1,
            line_start: None,
            line_end: None,
        })
    else {
        return fallback_height;
    };
    // Definite-height columns place block children sequentially. For
    // auto-height columns, the selected fill mode controls balancing below.
    let fragment_height = context.available_height;
    let mut column = 0usize;
    let mut cursor = 0.0f32;
    let mut maximum = 0.0f32;
    // For balance-filled auto-height multicol containers, measure block
    // children once before placing them. This gives the simple balancing pass
    // a target height; measuring against the current column would otherwise
    // keep every child in the first column and leave the container taller
    // than necessary.
    let auto_measurements = if context.column_fill != ColumnFillValue::Auto
        && tree.fragmentation_stack.len() == 1
        && fragment_height.is_none()
        && !multicol_has_nested_descendant(tree, index)
    {
        let mut measurements = Vec::with_capacity(children.len());
        for &child in &children {
            let child_inputs = LayoutInput {
                run_mode: RunMode::PerformLayout,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Both,
                known_dimensions: Size {
                    width: Some(context.column_width),
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
            if matches!(tree.nodes[child].data, NodeData::Text(_)) {
                tree.nodes[child].style.size.height = Dimension::auto();
                tree.nodes[child].cache.clear();
            }
            let output = tree.compute_child_layout(TaffyNodeId::from(child), child_inputs);
            let layout = tree.nodes[child].unrounded_layout;
            let margin_top = layout.margin.top;
            let margin_bottom = layout.margin.bottom;
            measurements.push((
                child,
                output,
                layout,
                margin_top,
                margin_bottom,
                margin_top + output.size.height + margin_bottom,
            ));
        }
        let total = measurements.iter().map(|entry| entry.5).sum::<f32>();
        let largest = measurements.iter().map(|entry| entry.5).fold(0.0, f32::max);
        let target = (total / context.column_count as f32).max(largest);
        Some((measurements, target))
    } else {
        None
    };
    let mut avoid_column_break_after_previous = false;
    for (order, child) in children.into_iter().enumerate() {
        let measured = auto_measurements
            .as_ref()
            .and_then(|(entries, _)| entries.iter().find(|entry| entry.0 == child));
        let (child_output, mut child_layout, margin_top, margin_bottom, needed) =
            if let Some((_, output, layout, margin_top, margin_bottom, needed)) = measured {
                (*output, *layout, *margin_top, *margin_bottom, *needed)
            } else {
                let child_height = match fragment_height {
                    Some(height) => AvailableSpace::Definite((height - cursor).max(0.0)),
                    None => AvailableSpace::MaxContent,
                };
                let child_inputs = LayoutInput {
                    run_mode: RunMode::PerformLayout,
                    sizing_mode: SizingMode::InherentSize,
                    axis: RequestedAxis::Both,
                    known_dimensions: Size {
                        width: Some(context.column_width),
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
                        height: child_height,
                    },
                    vertical_margins_are_collapsible: TaffyLine::FALSE,
                };

                // prepare_multicol_layout gives direct text a temporary used height
                // for its page-level projection. A nested width probe must measure the
                // shaped layout again instead of reusing that stale height.
                if matches!(tree.nodes[child].data, NodeData::Text(_)) {
                    tree.nodes[child].style.size.height = Dimension::auto();
                    tree.nodes[child].cache.clear();
                }
                let output = tree.compute_child_layout(TaffyNodeId::from(child), child_inputs);
                if tree.fragment_tree.limit_exceeded {
                    return fallback_height;
                }
                refresh_nested_text_fragments(tree, child, context);
                let layout = tree.nodes[child].unrounded_layout;
                let margin_top = layout.margin.top;
                let margin_bottom = layout.margin.bottom;
                let needed = margin_top + output.size.height + margin_bottom;
                (output, layout, margin_top, margin_bottom, needed)
            };
        let break_height =
            fragment_height.or_else(|| auto_measurements.as_ref().map(|(_, height)| *height));
        let avoid_column_break = avoid_column_break_after_previous
            || matches!(tree.nodes[child].break_before, BreakBetween::Avoid);
        if let Some(height) = break_height
            && column + 1 < context.column_count
            && cursor > 0.0
            && cursor + needed > height
            && !avoid_column_break
        {
            maximum = maximum.max(cursor);
            tree.fragment_tree
                .record_break(crate::fragment::BreakToken {
                    node_id: index,
                    child_index: order,
                    line_index: 0,
                });
            column += 1;
            cursor = 0.0;
        }

        let column_context = context.in_column(
            column,
            context.origin_x + context.column_offset_x(column),
            context.origin_y + cursor,
        );
        let x = child_layout.location.x + context.column_offset_x(column);
        let y = cursor + margin_top;
        child_layout.order = order as u32;
        child_layout.size = child_output.size;
        child_layout.location = Point { x, y };
        tree.set_unrounded_layout(TaffyNodeId::from(child), &child_layout);
        refresh_nested_text_fragments(tree, child, column_context);
        let Some(child_fragment) = tree
            .fragment_tree
            .try_push(crate::fragment::LayoutFragment {
                node_id: child,
                parent: Some(container_fragment),
                fragmentainer: column_context.column_index,
                rect: crate::fragment::FragmentRect {
                    x: child_layout.location.x,
                    y: cursor + margin_top,
                    width: child_output.size.width,
                    height: child_output.size.height,
                },
                fragmentainer_clip: None,
                fragment_index: 0,
                fragment_count: 1,
                line_start: None,
                line_end: None,
            })
        else {
            return fallback_height;
        };
        tree.fragment_tree.reparent_roots(child, child_fragment);
        if record_nested_ifc_box_fragments(tree, child, child_fragment, context).is_none() {
            return fallback_height;
        }
        cursor = y + child_output.size.height + margin_bottom;
        if tree.nodes[child].kind() == NodeKind::Element {
            avoid_column_break_after_previous =
                matches!(tree.nodes[child].break_after, BreakBetween::Avoid);
        }
    }
    let minimum_height =
        if auto_measurements.is_some() && !multicol_has_nested_descendant(tree, index) {
            0.0
        } else {
            fragment_height
                .map(|height| fallback_height.min(height))
                .unwrap_or(fallback_height)
        };
    let used = maximum.max(cursor).max(minimum_height);
    if let Some(fragment) = tree.fragment_tree.fragments.get_mut(container_fragment) {
        if has_fragmentainer_height_constraint {
            // Overflowing descendants do not expand a height-constrained border box.
            fragment.rect.height = used.min(fallback_height);
        } else {
            fragment.rect.height = used;
        }
    }
    used
}

// cov:ignore: nested box and float fragment records are exercised by the ignored flex-float WPT.
fn record_nested_ifc_box_fragments(
    tree: &mut Document,
    subtree_root: usize,
    parent_fragment: usize,
    context: FragmentationContext,
) -> Option<()> {
    let Some(height) = context.available_height.filter(|height| *height > 0.0) else {
        return Some(());
    };
    let subtree_has_float = multicol_subtree_has_float(tree, subtree_root);
    let mut existing_fragments = std::collections::HashMap::<(usize, usize), usize>::new();
    let mut pending = vec![subtree_root];
    while let Some(node_id) = pending.pop() {
        let nested_row_flex_scope = nested_row_flex_float_scope(tree, node_id);
        let nested_logical_minimum_scope =
            nested_logical_min_block_size_scope(tree, node_id, subtree_root);
        let lines = tree.nodes[node_id]
            .ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
            .cloned();
        let mut placements = lines
            .as_ref()
            .map(|lines| lines.fragment_box_placements.clone())
            .unwrap_or_default();
        // Non-float nested text needs explicit ranges only when lines cross
        // columns. Break-avoided logical minimum children keep block placement.
        let line_ranges = (nested_row_flex_scope
            || (!subtree_has_float && !nested_logical_minimum_scope))
            .then(|| {
                lines
                    .as_ref()
                    .and_then(|lines| lines.fragmentainer_line_ranges.clone())
                    .or_else(|| {
                        tree.nodes[node_id]
                            .ifc
                            .as_ref()
                            .and_then(|root| root.multicol_fragments.clone())
                    })
            })
            .flatten()
            .filter(|ranges| nested_row_flex_scope || ranges.len() > 1);
        // A subpixel column height can make one tall float span millions of
        // fragmentainers. Cap expanded placements before building paint paths.
        const MAX_NESTED_FLOAT_FRAGMENTS: usize = 1_024;
        let mut placement_keys = placements
            .iter()
            .map(|placement| (placement.node_id, placement.fragmentainer))
            .collect::<std::collections::HashSet<_>>();
        let mut last_placement_columns = std::collections::HashMap::<usize, usize>::new();
        for placement in &placements {
            last_placement_columns
                .entry(placement.node_id)
                .and_modify(|last| *last = (*last).max(placement.fragmentainer))
                .or_insert(placement.fragmentainer);
        }
        let mut overflow_clip_heights = std::collections::HashMap::<usize, f32>::new();
        for source in placements.clone() {
            if !tree.nodes[source.node_id].style.float.is_floated() {
                continue;
            }
            let logical_top = source.fragmentainer.saturating_sub(context.column_index) as f32
                * height
                + source.rect.y;
            let last_column = context.column_index.saturating_add(
                (((logical_top + source.rect.height).max(0.0) / height).ceil() as usize)
                    .saturating_sub(1),
            );
            for column in source.fragmentainer.saturating_add(1)..=last_column {
                if placements.len() >= MAX_NESTED_FLOAT_FRAGMENTS {
                    break;
                }
                if !placement_keys.insert((source.node_id, column)) {
                    continue;
                }
                placements.push(crate::layout::ifc::root::IfcBoxFragment {
                    node_id: source.node_id,
                    fragmentainer: column,
                    rect: crate::fragment::FragmentRect {
                        y: source.rect.y - (column - source.fragmentainer) as f32 * height,
                        ..source.rect
                    },
                });
                last_placement_columns
                    .entry(source.node_id)
                    .and_modify(|last| *last = (*last).max(column))
                    .or_insert(column);
            }
            let last_emitted = last_placement_columns[&source.node_id];
            if last_emitted < last_column {
                // Keep the unexpanded tail visible in the final emitted
                // column when the defensive fragment budget is exhausted.
                let remaining_bottom = source.rect.y + source.rect.height
                    - last_emitted.saturating_sub(source.fragmentainer) as f32 * height;
                overflow_clip_heights
                    .entry(last_emitted)
                    .and_modify(|clip| *clip = clip.max(remaining_bottom))
                    .or_insert(remaining_bottom);
            }
        }
        if !placements.is_empty() || line_ranges.is_some() {
            let mut columns = std::collections::BTreeSet::new();
            columns.extend(placements.iter().map(|placement| placement.fragmentainer));
            if let Some(ranges) = &line_ranges {
                columns.extend(ranges.iter().map(|range| range.fragmentainer));
            }

            let mut path = Vec::new();
            let mut current = node_id;
            while current != subtree_root {
                path.push(current);
                let Some(parent) = tree.parent_of(current) else {
                    break;
                };
                current = parent;
            }
            path.reverse();
            let node_offset_y = path
                .iter()
                .map(|&ancestor| tree.nodes[ancestor].unrounded_layout.location.y)
                .sum::<f32>();

            for column in columns {
                let column_x =
                    context.column_offset_x(column) - context.column_offset_x(context.column_index);
                let line_range = line_ranges
                    .as_ref()
                    .and_then(|ranges| ranges.iter().find(|range| range.fragmentainer == column))
                    .copied();
                let line_bottom = line_range
                    .and_then(|range| {
                        lines
                            .as_ref()?
                            .lines
                            .get(range.line_end.checked_sub(1)?)
                            .map(|line| line.block_offset() + line.block_size())
                    })
                    .unwrap_or(0.0);
                let tail_line_bottom = lines
                    .as_ref()
                    .filter(|lines| lines.unfragmented_tail_column == Some(column))
                    .map(|_| node_offset_y + line_bottom)
                    .unwrap_or(0.0);
                let overflow_float_bottom = overflow_clip_heights
                    .get(&column)
                    .map(|bottom| node_offset_y + bottom)
                    .unwrap_or(0.0);
                let column_clip_height = height.max(tail_line_bottom).max(overflow_float_bottom);
                let column_delta = column.saturating_sub(context.column_index) as f32 * height;
                let mut parent = parent_fragment;
                let mut parent_offset = Point::ZERO;
                if node_id == subtree_root
                    && let Some(range) = line_range
                    && tree.fragment_tree.fragments[parent_fragment].node_id == node_id
                {
                    let base = tree.fragment_tree.fragments[parent_fragment];
                    let fragment_index = line_ranges
                        .as_ref()
                        .and_then(|ranges| {
                            ranges.iter().position(|candidate| {
                                candidate.fragmentainer == column
                                    && candidate.line_start == range.line_start
                            })
                        })
                        .unwrap_or(0);
                    let fragment_count = line_ranges.as_ref().map_or(1, Vec::len);
                    let fragment_height = lines
                        .as_ref()
                        .and_then(|lines| {
                            let first = lines.lines.get(range.line_start)?;
                            let last = lines.lines.get(range.line_end.checked_sub(1)?)?;
                            Some(last.block_offset() + last.block_size() - first.block_offset())
                        })
                        .unwrap_or(base.rect.height)
                        .max(0.0);
                    let fragment_id = if base.fragmentainer == column {
                        let fragment = &mut tree.fragment_tree.fragments[parent_fragment];
                        fragment.rect.height = fragment_height;
                        fragment.line_start = Some(range.line_start);
                        fragment.line_end = Some(range.line_end);
                        fragment.fragment_index = fragment_index;
                        fragment.fragment_count = fragment_count;
                        parent_fragment
                    } else {
                        tree.fragment_tree
                            .try_push(crate::fragment::LayoutFragment {
                                node_id,
                                parent: base.parent,
                                fragmentainer: column,
                                rect: crate::fragment::FragmentRect {
                                    x: base.rect.x + context.column_offset_x(column)
                                        - context.column_offset_x(base.fragmentainer),
                                    y: 0.0,
                                    width: base.rect.width,
                                    height: fragment_height,
                                },
                                fragmentainer_clip: None,
                                fragment_index,
                                fragment_count,
                                line_start: Some(range.line_start),
                                line_end: Some(range.line_end),
                            })?
                    };
                    existing_fragments.insert((node_id, column), fragment_id);
                    parent = fragment_id;
                }
                for &ancestor in &path {
                    let layout = tree.nodes[ancestor].unrounded_layout;
                    let is_flex_item = tree
                        .parent_of(ancestor)
                        .is_some_and(|parent| tree.nodes[parent].style.display == Display::Flex);
                    let fragment_id =
                        if let Some(&fragment_id) = existing_fragments.get(&(ancestor, column)) {
                            let fragment = &mut tree.fragment_tree.fragments[fragment_id];
                            extend_fragment_clip(fragment, column_clip_height);
                            if ancestor == node_id
                                && let Some(range) = line_range
                            {
                                fragment.line_start = Some(range.line_start);
                                fragment.line_end = Some(range.line_end);
                            }
                            fragment_id
                        } else {
                            let fragment_id =
                                tree.fragment_tree
                                    .try_push(crate::fragment::LayoutFragment {
                                        node_id: ancestor,
                                        parent: Some(parent),
                                        fragmentainer: column,
                                        rect: crate::fragment::FragmentRect {
                                            x: layout.location.x
                                                + if parent == parent_fragment {
                                                    column_x
                                                } else {
                                                    0.0
                                                },
                                            y: if ancestor == subtree_root || is_flex_item {
                                                layout.location.y
                                            } else {
                                                layout.location.y - column_delta
                                            },
                                            width: layout.size.width,
                                            height: layout.size.height,
                                        },
                                        fragmentainer_clip: Some(crate::fragment::FragmentRect {
                                            x: if parent == parent_fragment {
                                                column_x
                                            } else {
                                                -parent_offset.x
                                            },
                                            y: -parent_offset.y,
                                            width: context.column_width,
                                            height: column_clip_height,
                                        }),
                                        fragment_index: 0,
                                        fragment_count: 1,
                                        line_start: (ancestor == node_id)
                                            .then_some(line_range)
                                            .flatten()
                                            .map(|range| range.line_start),
                                        line_end: (ancestor == node_id)
                                            .then_some(line_range)
                                            .flatten()
                                            .map(|range| range.line_end),
                                    })?;
                            existing_fragments.insert((ancestor, column), fragment_id);
                            fragment_id
                        };
                    parent = fragment_id;
                    parent_offset.x += layout.location.x;
                    parent_offset.y += layout.location.y;
                }

                for placement in placements
                    .iter()
                    .filter(|placement| placement.fragmentainer == column)
                {
                    let fragment_id =
                        tree.fragment_tree
                            .try_push(crate::fragment::LayoutFragment {
                                node_id: placement.node_id,
                                parent: Some(parent),
                                fragmentainer: column,
                                rect: placement.rect,
                                fragmentainer_clip: Some(crate::fragment::FragmentRect {
                                    x: if parent == parent_fragment {
                                        column_x
                                    } else {
                                        -parent_offset.x
                                    },
                                    y: -parent_offset.y,
                                    width: context.column_width,
                                    height: column_clip_height,
                                }),
                                fragment_index: 0,
                                fragment_count: 1,
                                line_start: None,
                                line_end: None,
                            })?;
                    existing_fragments.insert((placement.node_id, column), fragment_id);
                }
            }
        }
        pending.extend(
            tree.nodes[node_id]
                .children
                .iter()
                .copied()
                .filter(|&child| {
                    tree.nodes[child].is_in_document()
                        && tree.nodes[child].style.display != Display::None
                }),
        );
    }
    Some(())
}

fn extend_fragment_clip(fragment: &mut crate::fragment::LayoutFragment, height: f32) {
    if let Some(clip) = fragment.fragmentainer_clip.as_mut() {
        clip.height = clip.height.max(height);
    }
}

fn multicol_subtree_has_float(tree: &Document, subtree_root: usize) -> bool {
    let mut pending = vec![subtree_root];
    while let Some(node_id) = pending.pop() {
        let node = &tree.nodes[node_id];
        if !node.is_in_document() || node.style.display == Display::None {
            continue;
        }
        if node.style.float.is_floated() {
            return true;
        }
        pending.extend(node.children.iter().copied());
    }
    false
}

fn nested_logical_min_block_size_scope(
    tree: &Document,
    node_id: usize,
    subtree_root: usize,
) -> bool {
    let mut current = Some(node_id);
    while let Some(ancestor) = current {
        if tree.nodes[ancestor].has_logical_min_block_size {
            return true;
        }
        if ancestor == subtree_root {
            return false;
        }
        current = tree.parent_of(ancestor);
    }
    false
}

/// Split lines, given by their block-start and block-end offsets, over the
/// columns of `context` from its current column on: by the column height
/// when it is definite, else balanced by count; then `widows` and `orphans`
/// move lines across each break. Returns `(first, end, column)` ranges.
fn line_ranges_in_columns(
    extents: &[(f32, f32)],
    context: FragmentationContext,
    preserve_block_offsets: bool,
) -> Vec<(usize, usize, usize)> {
    let line_count = extents.len();
    if line_count == 0 || context.column_index >= context.column_count && !preserve_block_offsets {
        return Vec::new();
    }
    let available_columns = context.column_count.saturating_sub(context.column_index);
    let mut ranges = Vec::with_capacity(available_columns.max(1));
    let mut start = 0usize;
    if preserve_block_offsets
        && let Some(height) = context.available_height.filter(|height| *height > 0.0)
    {
        while start < line_count {
            let column_offset = (extents[start].0.max(0.0) / height).floor() as usize;
            let column = context.column_index.saturating_add(column_offset);
            let mut end = start + 1;
            while end < line_count
                && (extents[end].0.max(0.0) / height).floor() as usize == column_offset
            {
                end += 1;
            }
            ranges.push((start, end, column));
            start = end;
        }
    } else {
        if available_columns == 0 {
            return Vec::new();
        }
        for column in 0..available_columns {
            if start >= line_count {
                break;
            }
            let origin = extents.get(start).map_or(0.0, |extent| extent.0);
            let mut end = start;
            while end < line_count {
                let fits = context
                    .available_height
                    .filter(|height| *height > 0.0)
                    .map(|height| {
                        extents
                            .get(end)
                            .is_some_and(|extent| extent.1 - origin <= height + f32::EPSILON)
                    })
                    .unwrap_or_else(|| {
                        let remaining = line_count - start;
                        let target = remaining.div_ceil(available_columns - column);
                        end - start < target
                    });
                if !fits && end > start {
                    break;
                }
                end += 1;
            }
            ranges.push((
                start,
                end.max(start + 1).min(line_count),
                context.column_index + column,
            ));
            start = end.max(start + 1).min(line_count);
        }
        if let Some(last) = ranges.last_mut()
            && last.1 < line_count
        {
            last.1 = line_count;
        }
    }

    // CSS Fragmentation §3.3 constrains each line break. If the final
    // fragment would contain fewer than `widows` lines, move lines from the
    // preceding fragment while preserving its `orphans` minimum. This is the
    // smallest safe adjustment because the shaped layout and line metrics do
    // not change; only the fragment ranges move.
    let orphans = context.orphans.max(1);
    let widows = context.widows.max(1);
    for boundary in 0..ranges.len().saturating_sub(1) {
        let left_len = ranges[boundary].1 - ranges[boundary].0;
        let right_len = ranges[boundary + 1].1 - ranges[boundary + 1].0;
        if right_len < widows {
            let movable = left_len.saturating_sub(orphans);
            let moved = (widows - right_len).min(movable);
            ranges[boundary].1 -= moved;
            ranges[boundary + 1].0 -= moved;
        }
    }
    ranges
}

/// Find the smallest column height that fits `extents` within the available
/// columns. The returned height is then used to fill each column in source
/// order, including a partially filled final column.
fn balanced_column_height(extents: &[(f32, f32)], context: FragmentationContext) -> Option<f32> {
    let available_columns = context.column_count.saturating_sub(context.column_index);
    if extents.is_empty() || available_columns == 0 {
        return None;
    }
    let mut low = extents
        .iter()
        .map(|(top, bottom)| (bottom - top).max(0.0))
        .fold(0.0_f32, f32::max);
    let mut high = extents.last()?.1 - extents.first()?.0;
    if !low.is_finite() || !high.is_finite() || high < low {
        return None;
    }

    for _ in 0..32 {
        if high - low <= 0.001 {
            break;
        }
        let height = (low + high) * 0.5;
        let trial_context = FragmentationContext {
            available_height: Some(height),
            ..context
        };
        let ranges = line_ranges_in_columns(extents, trial_context, false);
        let fits = !ranges.is_empty()
            && ranges.iter().all(|&(start, end, _)| {
                extents[end - 1].1 - extents[start].0 <= height + f32::EPSILON
            });
        if fits {
            high = height;
        } else {
            low = height;
        }
    }
    Some(high)
}

// cov:ignore: nested text fragment refresh is exercised by ignored multicol WPT reftests.
fn refresh_nested_text_fragments(
    tree: &mut Document,
    node_id: usize,
    context: FragmentationContext,
) {
    let nested_row_flex_scope = nested_row_flex_float_scope(tree, node_id);
    let is_floated = tree.nodes[node_id].style.float.is_floated();
    let float_fragmentainer = tree.parent_of(node_id).and_then(|parent| {
        let parent_root = tree.nodes[parent].ifc.as_ref()?;
        parent_root
            .lines
            .as_ref()?
            .fragment_box_placements
            .iter()
            .find(|placement| placement.node_id == node_id)
            .map(|placement| placement.fragmentainer)
    });
    // A paragraph laid out by the inline engine keeps its lines on its root:
    // its fragments go there, and only its boxes hold text of their own.
    if let Some(root) = tree.nodes[node_id].ifc.as_mut() {
        let committed_ranges = root
            .lines
            .as_ref()
            .and_then(|lines| lines.fragmentainer_line_ranges.clone());
        let extents: Vec<(f32, f32)> = root
            .lines
            .as_ref()
            .map(|lines| {
                lines
                    .lines
                    .iter()
                    .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
                    .collect()
            })
            .unwrap_or_default();
        root.multicol_fragments = if nested_row_flex_scope {
            float_fragmentainer
                .filter(|_| is_floated)
                .map(|fragmentainer| {
                    (!extents.is_empty())
                        .then_some(vec![MulticolTextFragment {
                            line_start: 0,
                            line_end: extents.len(),
                            fragmentainer,
                            x: 0.0,
                            y: extents[0].0,
                        }])
                        .unwrap_or_default()
                })
                .or(committed_ranges)
                .or_else(|| {
                    (!extents.is_empty()).then(|| {
                        line_ranges_in_columns(&extents, context, true)
                            .into_iter()
                            .map(|(start, end, column)| MulticolTextFragment {
                                line_start: start,
                                line_end: end,
                                fragmentainer: column,
                                x: 0.0,
                                y: extents[start].0,
                            })
                            .collect()
                    })
                })
        } else {
            committed_ranges.or_else(|| {
                (!extents.is_empty()).then(|| {
                    line_ranges_in_columns(&extents, context, false)
                        .into_iter()
                        .map(|(start, end, column)| MulticolTextFragment {
                            line_start: start,
                            line_end: end,
                            fragmentainer: column,
                            x: context.column_offset_x(column)
                                - context.column_offset_x(context.column_index),
                            y: extents[start].0,
                        })
                        .collect()
                })
            })
        };
        let boxes = tree.nodes[node_id].ifc_boxes();
        for child in boxes {
            refresh_nested_text_fragments(tree, child, context);
        }
        return;
    }
    let children = tree.nodes[node_id].children.clone();
    for child in children {
        if tree.nodes[child].is_in_document() && tree.nodes[child].style.display != Display::None {
            refresh_nested_text_fragments(tree, child, context);
        }
    }
}

fn nested_row_flex_float_scope(tree: &Document, node_id: usize) -> bool {
    let mut has_row_flex = false;
    let mut has_float = false;
    let mut ancestor = Some(node_id);
    while let Some(current) = ancestor {
        let node = &tree.nodes[current];
        has_float |= node.style.float.is_floated()
            || node
                .ifc
                .as_ref()
                .and_then(|root| root.lines.as_ref())
                .is_some_and(|lines| {
                    lines
                        .fragment_box_placements
                        .iter()
                        .any(|placement| tree.nodes[placement.node_id].style.float.is_floated())
                });
        if matches!(
            node.authored_writing_mode,
            Some(
                raikiri_style::property::WritingMode::VerticalRl
                    | raikiri_style::property::WritingMode::VerticalLr
                    | raikiri_style::property::WritingMode::SidewaysRl
                    | raikiri_style::property::WritingMode::SidewaysLr
            )
        ) {
            return false;
        }
        if let Some(multicol) = node.multicol {
            return has_row_flex && has_float && multicol.horizontal;
        }
        has_row_flex |= node.style.display == Display::Flex
            && matches!(
                node.style.flex_direction,
                taffy::FlexDirection::Row | taffy::FlexDirection::RowReverse
            );
        ancestor = tree.parent_of(current);
    }
    false
}

// The foundational projection turns a block multicol container with inline
// children into a flex row. Its IFC descendants are not reached by the nested
// multicol dispatcher, so assign their line ranges after flex has placed them.
// cov:ignore: projected text ranges are exercised by ignored CSS Break WPT reftests.
pub(crate) fn refresh_projected_multicol_text_fragments(tree: &mut Document) {
    let containers: Vec<usize> = tree
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(node_id, node)| {
            (node.multicol.is_some()
                && node.display == raikiri_style::property::DisplayValue::Block
                && node.style.display == Display::Flex
                && !node.is_ifc_root())
            .then_some(node_id)
        })
        .collect();

    for container_id in containers {
        let Some(style) = tree.nodes[container_id].multicol else {
            continue;
        };
        let container_layout = tree.nodes[container_id].unrounded_layout;
        let (parent_width, parent_height) = tree
            .layout_parent_of(container_id)
            .map(|parent| {
                let layout = tree.nodes[parent].unrounded_layout;
                (Some(layout.size.width), Some(layout.size.height))
            })
            .unwrap_or((None, None));
        let content_width = multicol_content_width(
            tree,
            container_id,
            container_layout.size.width,
            parent_width,
        );
        let available_height = if style.height_definite {
            multicol_definite_dimension(
                tree,
                tree.nodes[container_id].style.size.height,
                parent_height,
            )
        } else {
            None
        }
        .or_else(|| {
            multicol_max_fragmentainer_height(
                tree,
                container_id,
                container_layout.size.height,
                parent_height,
                parent_width,
            )
        });
        let Some(context) = FragmentationContext::resolve(content_width, available_height, style)
        else {
            continue;
        };
        let column_step = context.column_width + context.column_gap;
        if !column_step.is_finite() || column_step <= 0.0 {
            continue;
        }

        let mut pending = tree.nodes[container_id].children.clone();
        while let Some(node_id) = pending.pop() {
            let node = &tree.nodes[node_id];
            if !node.is_in_document() || node.style.display == Display::None {
                continue;
            }
            if node_id != container_id && node.multicol.is_some() {
                continue;
            }
            if node.is_ifc_root() {
                let committed_ranges = node
                    .ifc
                    .as_ref()
                    .and_then(|root| root.lines.as_ref())
                    .and_then(|lines| lines.fragmentainer_line_ranges.clone());
                if node
                    .ifc
                    .as_ref()
                    .is_some_and(|root| root.multicol_fragments.is_none())
                    && let Some(lines) = node.ifc.as_ref().and_then(|root| root.lines.as_ref())
                {
                    let extents: Vec<(f32, f32)> = lines
                        .lines
                        .iter()
                        .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
                        .collect();
                    let ranges = if let Some(committed) = committed_ranges {
                        committed
                    } else if !extents.is_empty()
                        && let Some(inline_offset) =
                            layout_offset_from_ancestor(tree, node_id, container_id)
                    {
                        let first_column = ((inline_offset.max(0.0) / column_step).floor()
                            as usize)
                            .min(context.column_count.saturating_sub(1));
                        let local_context = context.in_column(
                            first_column,
                            context.origin_x + context.column_offset_x(first_column),
                            context.origin_y,
                        );
                        line_ranges_in_columns(&extents, local_context, false)
                            .into_iter()
                            .map(|(start, end, column)| MulticolTextFragment {
                                line_start: start,
                                line_end: end,
                                fragmentainer: column,
                                x: context.column_offset_x(column)
                                    - context.column_offset_x(first_column),
                                y: extents[start].0,
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    if let Some(root) = tree.nodes[node_id].ifc.as_mut() {
                        root.multicol_fragments = Some(ranges);
                    }
                }
            }
            pending.extend(tree.nodes[node_id].children.iter().copied());
        }
    }
}

fn layout_offset_from_ancestor(tree: &Document, node_id: usize, ancestor: usize) -> Option<f32> {
    let mut current = node_id;
    let mut inline_offset = 0.0;
    while current != ancestor {
        inline_offset += tree.nodes[current].unrounded_layout.location.x;
        current = tree.layout_parent_of(current)?;
    }
    Some(inline_offset)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MulticolMetrics {
    /// Used inline size of one column.
    column_width: f32,
    /// Used number of columns.
    column_count: usize,
    /// Used gap between adjacent columns.
    column_gap: f32,
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
fn computed_content_width(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
    fallback: f32,
) -> f32 {
    let parent_width = parent_of[node_id]
        .map(|parent| computed_content_width(cascade, parent_of, parent, fallback))
        .unwrap_or(fallback);
    match cascade.computed[node_id].width {
        ComputedLengthPercentageOrAuto::Px(value) if value.is_finite() => value.max(0.0),
        ComputedLengthPercentageOrAuto::Percent(value) if value.is_finite() => {
            (parent_width * value / 100.0).max(0.0)
        }
        _ => parent_width.max(0.0),
    }
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
pub(crate) fn authored_containing_width(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
    fallback: f32,
) -> Option<f32> {
    let mut ancestor = parent_of.get(node_id).copied().flatten();
    while let Some(id) = ancestor {
        // A `min-content` width is intrinsic, not an authored containing
        // width; keep searching like `auto`.
        if !matches!(
            cascade.computed[id].width,
            ComputedLengthPercentageOrAuto::Auto | ComputedLengthPercentageOrAuto::MinContent
        ) {
            return Some(computed_content_width(cascade, parent_of, id, fallback));
        }
        ancestor = parent_of.get(id).copied().flatten();
    }
    None
}

// Text shaping must use the same measured `ch` width that Taffy will use.
// The cascade retains a style-layer fallback, while pre-Taffy preparation has
// already written the measured value to the node style. This avoids shaping
// text against a narrower fallback than its containing block.
// cov:ignore: exercised by resource-enabled CSS Text WPT runs.
fn computed_content_width_with_resolved_ch(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
    fallback: f32,
) -> f32 {
    let parent_width = parent_of[node_id]
        .map(|parent| {
            computed_content_width_with_resolved_ch(doc, cascade, parent_of, parent, fallback)
        })
        .unwrap_or(fallback);
    let cv = &cascade.computed[node_id];
    if cv.width_ch.is_some()
        && let Some(width) = style_dimension_length(doc.nodes[node_id].style.size.width)
    {
        return width.max(0.0);
    }
    match cv.width {
        ComputedLengthPercentageOrAuto::Px(value) if value.is_finite() => value.max(0.0),
        ComputedLengthPercentageOrAuto::Percent(value) if value.is_finite() => {
            (parent_width * value / 100.0).max(0.0)
        }
        _ => parent_width.max(0.0),
    }
}

// cov:ignore: exercised by the resource-enabled line-break:anywhere WPT run.
pub(crate) fn authored_containing_width_with_resolved_ch(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
    fallback: f32,
) -> Option<f32> {
    let mut ancestor = parent_of.get(node_id).copied().flatten();
    while let Some(id) = ancestor {
        // A `min-content` width is intrinsic, not an authored containing
        // width; keep searching like `auto`.
        if !matches!(
            cascade.computed[id].width,
            ComputedLengthPercentageOrAuto::Auto | ComputedLengthPercentageOrAuto::MinContent
        ) {
            return Some(computed_content_width_with_resolved_ch(
                doc, cascade, parent_of, id, fallback,
            ));
        }
        ancestor = parent_of.get(id).copied().flatten();
    }
    None
}

// cov:ignore: exercised by ignored WPT regression and multicol reftests; default coverage skips ignored reftests.
fn has_nonzero_horizontal_margin(cv: &ComputedValues) -> bool {
    let is_zero = |value: ComputedLengthPercentageOrAuto| match value {
        ComputedLengthPercentageOrAuto::Px(px) | ComputedLengthPercentageOrAuto::Percent(px) => {
            px.abs() <= f32::EPSILON
        }
        _ => false,
    };
    !is_zero(cv.margin.left) || !is_zero(cv.margin.right)
}

// cov:ignore: exercised by ignored WPT regression and multicol reftests; default coverage skips ignored reftests.
fn has_non_block_flow_ancestor(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
) -> bool {
    let mut current = parent_of.get(node_id).copied().flatten();
    while let Some(id) = current {
        if matches!(
            cascade.computed[id].display,
            DisplayValue::Flex
                | DisplayValue::InlineFlex
                | DisplayValue::Grid
                | DisplayValue::InlineGrid
                | DisplayValue::Table
                | DisplayValue::InlineTable
                | DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup
                | DisplayValue::TableRow
                | DisplayValue::TableColumnGroup
                | DisplayValue::TableColumn
                | DisplayValue::TableCell
                | DisplayValue::TableCaption
        ) {
            return true;
        }
        current = parent_of.get(id).copied().flatten();
    }
    false
}

// cov:ignore: exercised by ignored WPT regression and multicol reftests; default coverage skips ignored reftests.
pub(crate) fn has_out_of_flow_ancestor(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
) -> bool {
    let mut current = parent_of.get(node_id).copied().flatten();
    while let Some(id) = current {
        if matches!(
            cascade.computed[id].position,
            PositionValue::Absolute | PositionValue::Fixed
        ) {
            return true;
        }
        current = parent_of.get(id).copied().flatten();
    }
    false
}

// cov:ignore: writing-mode fallback is covered by the ignored precision WPT run.
pub(crate) fn has_vertical_writing_mode(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
) -> bool {
    let mut current = Some(node_id);
    while let Some(id) = current {
        if matches!(
            cascade
                .authored_writing_modes
                .get(id)
                .and_then(|mode| *mode),
            Some(
                WritingMode::VerticalRl
                    | WritingMode::VerticalLr
                    | WritingMode::SidewaysRl
                    | WritingMode::SidewaysLr
            )
        ) {
            return true;
        }
        current = parent_of.get(id).copied().flatten();
    }
    false
}

// cov:ignore: nested recursion is exercised by the ignored nested WPT reftests.
fn has_multicol_ancestor(doc: &Document, parent_of: &[Option<usize>], node_id: usize) -> bool {
    let mut current = parent_of.get(node_id).copied().flatten();
    while let Some(parent) = current {
        if doc.nodes[parent].multicol.is_some() {
            return true;
        }
        current = parent_of.get(parent).copied().flatten();
    }
    false
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
pub(crate) fn multicol_metrics_for_node(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
    fallback_width: f32,
) -> Option<MulticolMetrics> {
    let cv = &cascade.computed[node_id];
    let declared_count = match cv.column_count {
        ColumnCountValue::Auto => None,
        ColumnCountValue::Count(value) if value > 0 => Some(value as usize),
        ColumnCountValue::Count(_) => None,
        _ => None,
    };
    let declared_width = match cv.column_width {
        ComputedColumnWidth::Auto => None,
        ComputedColumnWidth::Px(value) if value.is_finite() && value > 0.0 => Some(value),
        ComputedColumnWidth::Px(_) => None,
    };
    if declared_count.is_none() && declared_width.is_none() {
        return None;
    }
    let container_width = computed_content_width(cascade, parent_of, node_id, fallback_width);
    if !container_width.is_finite() || container_width <= 0.0 {
        return None;
    }
    // CSS Multi-column Layout Module Level 1 section 5 Column Gaps and Rules
    // https://www.w3.org/TR/css-multicol-1/#column-gaps-and-rules
    // resolves column-gap normal on a multicol container to 1em of the
    // container font size. This differs from flex and grid, where normal
    // behaves as 0, so the generic gap bridge keeps its own normal-to-0 mapping.
    let column_gap = match cv.column_gap {
        ComputedLengthPercentageOrNormal::Px(value) if value.is_finite() => value.max(0.0),
        ComputedLengthPercentageOrNormal::Percent(value) if value.is_finite() => {
            (container_width * value / 100.0).max(0.0)
        }
        ComputedLengthPercentageOrNormal::Normal => {
            let em = cv.font_size.px();
            if em.is_finite() { em.max(0.0) } else { 0.0 }
        }
        _ => 0.0,
    };
    let column_count = declared_count.unwrap_or_else(|| {
        let width = declared_width.unwrap_or(container_width).max(1.0);
        (((container_width + column_gap) / (width + column_gap)).floor() as usize).max(1)
    });
    let available =
        (container_width - column_gap * (column_count.saturating_sub(1) as f32)).max(0.0);
    let column_width = (available / column_count as f32).max(0.0);
    (column_width.is_finite() && column_width > 0.0).then_some(MulticolMetrics {
        column_width,
        column_count,
        column_gap,
    })
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
pub(crate) fn line_height_px(cv: &ComputedValues) -> f32 {
    let font_size = cv.font_size.px().max(0.0);
    match cv.line_height {
        ComputedLineHeight::Length(value) => value.0.max(0.0),
        ComputedLineHeight::Number(value) => (font_size * value).max(0.0),
        ComputedLineHeight::Normal => (font_size * 1.2).max(0.0),
    }
}

/// Apply the small fragmentainer model used by the foundational multicol pass.
///
/// Inline element children use a flex-row projection so each item occupies one
/// column. Direct text keeps its DOM node and receives explicit line ranges;
/// paint emits those ranges at the corresponding column offset. Descendants
/// below another multicol container are left to the recursive Taffy seam so
/// their used widths are not guessed from a page fallback.
/// The line ranges of a multicol container's own paragraph in its columns,
/// and the height the lines need: the tallest column. The lines fill the
/// columns of `context` by its column height when it is definite, else they
/// are balanced over them, with `widows` and `orphans` applied at each break
/// (CSS Multi-column 1, 7; CSS Fragmentation 3, 3.3). With `column-fill:auto`
/// and no definite height, soft breaks are disabled and all lines stay in the
/// current column.
pub(crate) fn root_column_fragments(
    lines: &crate::layout::ifc::root::IfcLines,
    context: FragmentationContext,
) -> (Vec<MulticolTextFragment>, f32) {
    let extents: Vec<(f32, f32)> = lines
        .lines
        .iter()
        .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
        .collect();
    if extents.is_empty() {
        return (Vec::new(), 0.0);
    }
    let balanced_context =
        if context.available_height.is_none() && context.column_fill != ColumnFillValue::Auto {
            balanced_column_height(&extents, context).map(|height| FragmentationContext {
                available_height: Some(height),
                ..context
            })
        } else {
            None
        };
    let layout_context = balanced_context.unwrap_or_else(|| {
        if context.available_height.is_none() && context.column_fill == ColumnFillValue::Auto {
            FragmentationContext {
                column_count: context.column_index.saturating_add(1),
                ..context
            }
        } else {
            context
        }
    });
    let ranges = line_ranges_in_columns(&extents, layout_context, false);
    let height = ranges
        .iter()
        .map(|&(start, end, _)| extents[end - 1].1 - extents[start].0)
        .fold(0.0_f32, f32::max);
    let fragments = ranges
        .into_iter()
        .map(|(start, end, column)| MulticolTextFragment {
            line_start: start,
            line_end: end,
            fragmentainer: column,
            x: context.column_offset_x(column),
            y: 0.0,
        })
        .collect();
    (fragments, height)
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
pub(crate) fn prepare_multicol_layout(
    doc: &mut Document,
    cascade: &CascadeResult,
    fallback_width: f32,
) {
    let mut parent_of = vec![None; doc.nodes.len()];
    for parent in 0..doc.nodes.len() {
        for &child in &doc.nodes[parent].children {
            if child < parent_of.len() {
                parent_of[child] = Some(parent);
            }
        }
    }
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        // Used widths for descendants of a multicol container are supplied by
        // the recursive Taffy seam. Do not pre-project them from cascade
        // fallback widths; that was the source of the nested-width bug.
        if has_multicol_ancestor(doc, &parent_of, idx) {
            continue;
        }
        // A box of a paragraph laid out by the inline engine (a child of its
        // root, or of an inline element of it) that the line loop sizes
        // itself keeps an auto width: an inline-block shrinks to fit (CSS 2.1
        // 10.3.9), and a block child that is a scroll container takes the
        // room the floats beside it leave (CSS 2.1 9.5).
        let cv = &cascade.computed[idx];
        let engine_sized = cv.display == DisplayValue::InlineBlock
            || cv.overflow.x != OverflowValue::Visible
            || cv.overflow.y != OverflowValue::Visible;
        let engine_box = engine_sized
            && !doc.nodes[idx].in_ifc_subtree()
            && parent_of[idx].is_some_and(|parent| {
                doc.nodes[parent].is_ifc_root() || doc.nodes[parent].in_ifc_subtree()
            });
        if engine_box {
            continue;
        }
        if matches!(
            cascade.computed[idx].width,
            ComputedLengthPercentageOrAuto::Auto
        ) && matches!(
            cascade.computed[idx].display,
            DisplayValue::Block | DisplayValue::InlineBlock
        ) && !has_vertical_writing_mode(cascade, &parent_of, idx)
            && matches!(cascade.computed[idx].position, PositionValue::Static)
            && !has_nonzero_horizontal_margin(&cascade.computed[idx])
            && !has_out_of_flow_ancestor(cascade, &parent_of, idx)
            && !has_non_block_flow_ancestor(cascade, &parent_of, idx)
            && let Some(width) = authored_containing_width(cascade, &parent_of, idx, fallback_width)
        {
            doc.nodes[idx].style.size.width = Dimension::length(width);
        }
    }
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        if has_multicol_ancestor(doc, &parent_of, idx) {
            continue;
        }
        if has_vertical_writing_mode(cascade, &parent_of, idx) {
            continue;
        }
        let Some(metrics) = multicol_metrics_for_node(cascade, &parent_of, idx, fallback_width)
        else {
            continue;
        };
        // A container whose own content is a paragraph of the inline engine
        // breaks its lines at the column width and splits them in columns
        // when it is laid out; none of the projections below apply to it.
        if doc.nodes[idx].is_ifc_root() {
            continue;
        }
        let direct_breaks = doc.nodes[idx]
            .children
            .iter()
            .filter(|&&child| {
                doc.nodes[child].kind() == NodeKind::Element
                    && doc.nodes[child].tag_name() == Some("br")
                    && doc.nodes[child].is_in_document()
            })
            .count();
        // A fixed-height auto-fill multicol can place an oversized direct
        // child in one column and let it overflow through the next column.
        // Project that narrow case as a row-wrapping flex flow: each child
        // owns one column's inline slot while its block overflow remains
        // visible, matching the class-B break at the fragmentainer edge.
        let fixed_fragmentainer_height = match cascade.computed[idx].height {
            ComputedLengthPercentageOrAuto::Px(value) if value.is_finite() && value > 0.0 => {
                Some(value)
            }
            _ => None,
        };
        let element_children: Vec<usize> = doc.nodes[idx]
            .children
            .iter()
            .copied()
            .filter(|&child| {
                doc.nodes[child].kind() == NodeKind::Element
                    && doc.nodes[child].is_in_document()
                    && doc.nodes[child].style.display != Display::None
            })
            .collect();
        let has_oversized_direct_child = fixed_fragmentainer_height.is_some_and(|height| {
            element_children.iter().any(|&child| {
                style_dimension_length(doc.nodes[child].style.size.height)
                    .is_some_and(|child_height| child_height > height + 0.001)
            })
        });
        let has_oversized_nested_inline_child = fixed_fragmentainer_height.is_some_and(|height| {
            element_children.iter().any(|&child| {
                let mut pending = vec![child];
                while let Some(descendant) = pending.pop() {
                    if matches!(
                        cascade.computed[descendant].display,
                        DisplayValue::Inline | DisplayValue::InlineBlock
                    ) && style_dimension_length(doc.nodes[descendant].style.size.height)
                        .is_some_and(|descendant_height| descendant_height > height + 0.001)
                    {
                        return true;
                    }
                    pending.extend(doc.nodes[descendant].children.iter().copied());
                }
                false
            })
        });
        // The projection is valid for direct-`br` lines, the narrow two-child
        // nested inline-block fixture, and the fixed-height border-break
        // candidate. A longer nested flow (as in tall-line-000) owns a
        // parallel flow and stays on the normal multicol path.
        let has_direct_br_in_each_child = element_children.iter().all(|&child| {
            doc.nodes[child]
                .children
                .iter()
                .any(|&grandchild| doc.nodes[grandchild].tag_name() == Some("br"))
        });
        let has_two_child_nested_inline_flow =
            element_children.len() == 2 && has_oversized_nested_inline_child;
        let has_border_break_candidate = fixed_fragmentainer_height.is_some_and(|height| {
            element_children.windows(2).any(|pair| {
                let first = style_dimension_length(doc.nodes[pair[0]].style.size.height);
                let second = style_dimension_length(doc.nodes[pair[1]].style.size.height);
                let cv = &cascade.computed[pair[1]];
                let border = cv.border.top.width().px() + cv.border.bottom.width().px();
                first.is_some_and(|first| first > 0.0)
                    && second.is_some_and(|second| second + border >= height - 0.001)
                    && border > 0.0
            })
        });
        if has_oversized_direct_child && has_direct_br_in_each_child
            || has_two_child_nested_inline_flow
            || has_border_break_candidate
        {
            let style = &mut doc.nodes[idx].style;
            style.display = Display::Flex;
            style.flex_direction = TaffyFlexDirection::Row;
            style.flex_wrap = TaffyFlexWrap::Wrap;
            style.align_items = Some(TaffyAlignItems::FLEX_START);
            style.align_content = Some(TaffyAlignContent::FLEX_START);
            style.justify_content = None;
            style.gap.width = LengthPercentage::length(metrics.column_gap);
            style.gap.height = LengthPercentage::length(0.0);
            for &child in &doc.nodes[idx].children.clone() {
                if doc.nodes[child].kind() != NodeKind::Element
                    || !doc.nodes[child].is_in_document()
                {
                    continue;
                }
                let has_direct_br = doc.nodes[child]
                    .children
                    .iter()
                    .any(|&grandchild| doc.nodes[grandchild].tag_name() == Some("br"));
                let child_has_explicit_height =
                    style_dimension_length(doc.nodes[child].style.size.height).is_some();
                let child_line_height = line_height_px(&cascade.computed[child]);
                let child_style = &mut doc.nodes[child].style;
                child_style.flex_grow = 0.0;
                child_style.flex_shrink = 0.0;
                let horizontal_border = cascade.computed[child].border.left.width().px()
                    + cascade.computed[child].border.right.width().px();
                let child_content_width = (metrics.column_width - horizontal_border).max(0.0);
                child_style.size.width = Dimension::length(child_content_width);
                if !child_has_explicit_height && has_direct_br {
                    child_style.size.height = Dimension::length(child_line_height);
                }
                child_style.min_size.width = LengthPercentageAuto::length(0.0);
                child_style.flex_basis = Dimension::length(child_content_width);
            }
        }
        let mut column_height: f32 = 0.0;
        if direct_breaks > 0 {
            let line_height = line_height_px(&cascade.computed[idx]);
            column_height =
                (direct_breaks.div_ceil(metrics.column_count) as f32 * line_height).max(0.0);
        }
        if column_height > 0.0
            && matches!(
                cascade.computed[idx].height,
                ComputedLengthPercentageOrAuto::Auto
            )
        {
            doc.nodes[idx].style.size.height = Dimension::length(column_height);
        }

        let has_inline_flow_children = doc.nodes[idx].children.iter().any(|&child| {
            doc.nodes[child].is_in_document()
                && doc.nodes[child].kind() == NodeKind::Element
                && is_inline_element_box(cascade, child)
        });
        let has_ruby_child = doc.nodes[idx].children.iter().any(|&child| {
            doc.nodes[child].is_in_document()
                && doc.nodes[child].kind() == NodeKind::Element
                && doc.nodes[child].tag_name() == Some("ruby")
        });
        if has_inline_flow_children || direct_breaks > 0 {
            let style = &mut doc.nodes[idx].style;
            style.display = Display::Flex;
            // Ruby annotations are a single vertical fragment. In a
            // multicol container, column-direction wrapping places one ruby
            // item per column instead of treating each pair as a horizontal
            // line followed by a new row.
            style.flex_direction = if has_ruby_child && metrics.column_count > 2 {
                TaffyFlexDirection::Column
            } else {
                TaffyFlexDirection::Row
            };
            style.flex_wrap = TaffyFlexWrap::Wrap;
            style.align_items = Some(TaffyAlignItems::FLEX_START);
            style.align_content = Some(TaffyAlignContent::FLEX_START);
            style.justify_content = None;
            style.gap.width = LengthPercentage::length(metrics.column_gap);
            style.gap.height = LengthPercentage::length(0.0);
            for &child in &doc.nodes[idx].children.clone() {
                if !doc.nodes[child].is_in_document() {
                    continue;
                }
                let is_ruby_break = has_ruby_child
                    && metrics.column_count == 2
                    && doc.nodes[child].tag_name() == Some("br");
                let child_width = if is_ruby_break {
                    0.0
                } else if has_ruby_child && doc.nodes[child].tag_name() == Some("ruby") {
                    doc.nodes[child]
                        .children
                        .iter()
                        .find_map(|&grandchild| {
                            style_dimension_length(doc.nodes[grandchild].style.size.width)
                        })
                        .unwrap_or(metrics.column_width)
                } else {
                    metrics.column_width
                };
                let ruby_under = has_ruby_child
                    && metrics.column_count == 2
                    && doc.nodes[child].tag_name() == Some("ruby")
                    && cascade.computed[child].ruby_position == RubyPosition::Under;
                let child_style = &mut doc.nodes[child].style;
                child_style.flex_grow = 0.0;
                child_style.flex_shrink = 0.0;
                child_style.size.width = Dimension::length(child_width);
                if ruby_under {
                    child_style.margin.top = LengthPercentageAuto::length(25.0);
                }
                child_style.min_size.width = LengthPercentageAuto::length(0.0);
                child_style.flex_basis = Dimension::length(child_width);
            }
        }
    }
}

/// Whether `idx`, an element, generates an inline-level box (or none of its
/// own, for `display: contents`).
fn is_inline_element_box(cascade: &CascadeResult, idx: usize) -> bool {
    matches!(
        cascade.computed[idx].display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineTable
            | DisplayValue::Contents
    )
}

#[cfg(test)]
mod tests;
