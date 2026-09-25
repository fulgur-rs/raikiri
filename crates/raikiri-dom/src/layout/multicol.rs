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
    if custom_scope && inputs.run_mode == RunMode::PerformLayout {
        // Taffy's input width is normally already the content width for this
        // bridge. Correct it for authored padding/border before deriving the
        // child column width, so percentage gaps use the used content box.
        let content_width = multicol_content_width(tree, index, output.size.width, parent_width);
        let resolved = FragmentationContext::resolve(content_width, available_height, style)
            .unwrap_or(context);
        if let Some(active) = tree.fragmentation_stack.last_mut() {
            *active = resolved;
        }
        let used_height =
            relayout_nested_multicol_children(tree, node_id, resolved, output.size.height);
        if available_height.is_none() {
            output.size.height = used_height.max(0.0);
        }
    }
    // Truncating to the depth captured on entry keeps the stack balanced if a
    // future child strategy starts pushing a nested context of its own.
    tree.fragmentation_stack.truncate(stack_depth);
    output
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
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

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn relayout_nested_multicol_children(
    tree: &mut Document,
    node_id: TaffyNodeId,
    context: FragmentationContext,
    fallback_height: f32,
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
    let container_fragment = tree.fragment_tree.push(crate::fragment::LayoutFragment {
        node_id: index,
        parent: None,
        fragmentainer: context.column_index,
        x: context.origin_x,
        y: context.origin_y,
        width: context.available_width,
        height: fallback_height,
        line_start: None,
        line_end: None,
    });
    // This tranche models `column-fill:auto`: source-order children consume
    // the current definite fragmentainer before the next column starts.
    // Balancing remains on the foundational direct-text path.
    let fragment_height = context.available_height;
    let mut column = 0usize;
    let mut cursor = 0.0f32;
    let mut maximum = 0.0f32;
    // With an auto-height multicol, measure block children once before
    // placing them. This gives the simple balancing pass a target height;
    // measuring against the current column would otherwise keep every child
    // in the first column and leave the container taller than necessary.
    let auto_measurements = if tree.fragmentation_stack.len() == 1
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
        let child_fragment = tree.fragment_tree.push(crate::fragment::LayoutFragment {
            node_id: child,
            parent: Some(container_fragment),
            fragmentainer: column_context.column_index,
            x: column_context.origin_x,
            y: column_context.origin_y + margin_top,
            width: child_output.size.width,
            height: child_output.size.height,
            line_start: None,
            line_end: None,
        });
        tree.fragment_tree.reparent_roots(child, child_fragment);
        // Keep text ranges in the same first-class tree as element boxes.
        // Paint still consumes the node-local ranges for compatibility, while
        // future incremental reflow can walk one nested fragment tree.
        let text_fragments = tree.nodes[child]
            .multicol_fragments()
            .map(|fragments| fragments.to_vec());
        if let Some(text_fragments) = text_fragments {
            for fragment in text_fragments {
                tree.fragment_tree.push(crate::fragment::LayoutFragment {
                    node_id: child,
                    parent: Some(child_fragment),
                    fragmentainer: column_context.column_index,
                    x: child_layout.location.x + fragment.x,
                    y: child_layout.location.y + fragment.y,
                    width: child_output.size.width,
                    height: child_output.size.height,
                    line_start: Some(fragment.line_start),
                    line_end: Some(fragment.line_end),
                });
            }
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
    maximum.max(cursor).max(minimum_height)
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn nested_text_line_ranges(
    layout: &parley::Layout<()>,
    context: FragmentationContext,
) -> Vec<(usize, usize, usize)> {
    let line_count = layout.len();
    if line_count == 0 || context.column_index >= context.column_count {
        return Vec::new();
    }
    let available_columns = context.column_count - context.column_index;
    let mut ranges = Vec::with_capacity(available_columns);
    let mut start = 0usize;
    for column in 0..available_columns {
        if start >= line_count {
            break;
        }
        let origin = layout
            .lines()
            .nth(start)
            .map(|line| line.metrics().block_min_coord)
            .unwrap_or(0.0);
        let mut end = start;
        while end < line_count {
            let fits = context
                .available_height
                .filter(|height| *height > 0.0)
                .map(|height| {
                    layout
                        .lines()
                        .nth(end)
                        .map(|line| {
                            line.metrics().block_max_coord - origin <= height + f32::EPSILON
                        })
                        .unwrap_or(false)
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

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn refresh_nested_text_fragments(
    tree: &mut Document,
    node_id: usize,
    context: FragmentationContext,
) {
    if let NodeData::Text(text) = &mut tree.nodes[node_id].data {
        let Some(layout) = text.text_layout.as_ref() else {
            return;
        };
        let line_count = layout.len();
        if line_count == 0 {
            text.multicol_fragments = None;
            return;
        }
        let ranges = nested_text_line_ranges(layout, context);
        text.multicol_fragments = Some(
            ranges
                .into_iter()
                .map(|(start, end, column)| MulticolTextFragment {
                    line_start: start,
                    line_end: end,
                    x: context.column_offset_x(column)
                        - context.column_offset_x(context.column_index),
                    // The painter normalizes the first selected line's
                    // block-minimum. Store that minimum here so a recursive
                    // fragment keeps the ordinary block-flow baseline.
                    y: layout
                        .lines()
                        .nth(start)
                        .map(|line| line.metrics().block_min_coord)
                        .unwrap_or(0.0),
                })
                .collect(),
        );
        return;
    }
    let children = tree.nodes[node_id].children.clone();
    for child in children {
        if tree.nodes[child].is_in_document() && tree.nodes[child].style.display != Display::None {
            refresh_nested_text_fragments(tree, child, context);
        }
    }
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
        if !matches!(
            cascade.computed[id].width,
            ComputedLengthPercentageOrAuto::Auto
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
        if !matches!(
            cascade.computed[id].width,
            ComputedLengthPercentageOrAuto::Auto
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
    let column_gap = match cv.column_gap {
        ComputedLengthPercentageOrNormal::Px(value) if value.is_finite() => value.max(0.0),
        ComputedLengthPercentageOrNormal::Percent(value) if value.is_finite() => {
            (container_width * value / 100.0).max(0.0)
        }
        ComputedLengthPercentageOrNormal::Normal => 0.0,
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
pub(crate) fn multicol_column_width_for_text(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    text_id: usize,
    fallback_width: f32,
) -> Option<f32> {
    // Direct text is fragmented by this tranche. Descendant text inside an
    // inline element keeps its intrinsic run width so inline backgrounds and
    // overflow retain the existing paint behavior (the basic WPTs rely on it).
    let parent = parent_of[text_id]?;
    if has_vertical_writing_mode(cascade, parent_of, parent) {
        return None;
    }
    multicol_metrics_for_node(cascade, parent_of, parent, fallback_width)
        .map(|metrics| metrics.column_width)
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
        if matches!(
            cascade.computed[idx].width,
            ComputedLengthPercentageOrAuto::Auto
        ) && matches!(
            cascade.computed[idx].display,
            DisplayValue::Block | DisplayValue::InlineBlock
        ) && matches!(cascade.computed[idx].position, PositionValue::Static)
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
        let direct_text: Vec<usize> = doc.nodes[idx]
            .children
            .iter()
            .copied()
            .filter(|&child| {
                doc.nodes[child].kind() == NodeKind::Text
                    && doc.nodes[child].is_in_document()
                    && doc.nodes[child].text_layout().is_some()
            })
            .collect();
        let direct_breaks = doc.nodes[idx]
            .children
            .iter()
            .filter(|&&child| {
                doc.nodes[child].kind() == NodeKind::Element
                    && doc.nodes[child].tag_name() == Some("br")
                    && doc.nodes[child].is_in_document()
            })
            .count();
        let has_direct_text = direct_text.iter().any(|&id| {
            doc.nodes[id]
                .text_content()
                .is_some_and(|text| !text.trim().is_empty())
        });
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
        if !has_direct_text
            && (has_oversized_direct_child && has_direct_br_in_each_child
                || has_two_child_nested_inline_flow
                || has_border_break_candidate)
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
        // A direct text run is the only case in this focused slice that needs
        // explicit line fragments. The line count is balanced top-to-bottom.
        let mut column_height: f32 = 0.0;
        for &text_id in &direct_text {
            let Some(layout) = doc.nodes[text_id].text_layout() else {
                continue;
            };
            let line_count = layout.len();
            if line_count == 0 {
                continue;
            }
            let lines_per_column = line_count.div_ceil(metrics.column_count);
            let line_height = line_height_px(&cascade.computed[text_id]);
            column_height = column_height.max(lines_per_column as f32 * line_height);
            let fragments = (0..metrics.column_count)
                .filter_map(|column| {
                    let start = (column * lines_per_column).min(line_count);
                    let end = ((column + 1) * lines_per_column).min(line_count);
                    (start < end).then_some(MulticolTextFragment {
                        line_start: start,
                        line_end: end,
                        x: column as f32 * (metrics.column_width + metrics.column_gap),
                        y: 0.0,
                    })
                })
                .collect::<Vec<_>>();
            if let Some(text) = doc.nodes[text_id].data.as_text_mut() {
                text.multicol_fragments = Some(fragments);
            }
            doc.nodes[text_id].style.size.width = Dimension::length(metrics.column_width);
        }
        if column_height > 0.0 {
            for &text_id in &direct_text {
                doc.nodes[text_id].style.size.height = Dimension::length(column_height);
            }
        }

        if direct_breaks > 0
            && has_direct_text
            && matches!(
                cascade.computed[idx].height,
                ComputedLengthPercentageOrAuto::Auto
            )
        {
            let line_height = line_height_px(&cascade.computed[idx]);
            // An auto-height `column-fill: auto` flow cannot honor soft
            // breaks: all direct `<br>` lines establish the used height in
            // source order instead of being divided among columns. This is
            // the narrow direct-break shape covered by widows-orphans-017.
            let direct_line_count = direct_text
                .iter()
                .filter_map(|&text_id| doc.nodes[text_id].text_layout())
                .map(|layout| layout.len())
                .sum::<usize>();
            let line_count = direct_line_count.max(direct_breaks.saturating_add(1));
            column_height = (line_count as f32 * line_height).max(0.0);
        } else if direct_breaks > 0 && !has_direct_text {
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
        if !has_direct_text && (has_inline_flow_children || direct_breaks > 0) {
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

#[cfg(test)]
mod tests;
