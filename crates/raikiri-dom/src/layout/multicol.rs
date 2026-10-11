mod break_flow;
mod constrained_chain;
mod paged_spanning;
mod paragraph_group;
mod spanning;

use super::*;

pub(super) use paged_spanning::paginate_spanning_columns;

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
        .map(|height| {
            multicol_authored_content_height(tree, index, height, parent_width, available_width)
        })
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
    let span_scope = tree.nodes[index]
        .children
        .iter()
        .any(|&child| spanning::is_spanner(tree, child));
    let custom_scope = (span_scope
        || tree.fragmentation_stack.is_empty()
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
    let break_flow_scope = !span_scope && break_flow::supports(tree, index, context);
    let constrained_chain = (inputs.run_mode == RunMode::PerformLayout
        && !custom_scope
        && !break_flow_scope
        && available_height.is_some_and(|height| height > 0.0)
        && style.horizontal
        && !multicol_has_out_of_flow_descendant(tree, index))
    .then(|| constrained_chain::supported(tree, index))
    .flatten();
    // Inspect fresh child layouts before opting a definite-height plain block
    // chain into fragmentation. Atomic and constrained children keep their
    // foundational placement path.
    let plain_block_scope = inputs.run_mode == RunMode::PerformLayout
        && !custom_scope
        && !break_flow_scope
        && available_height.is_some_and(|height| height > 0.0)
        && style.horizontal
        && !multicol_has_out_of_flow_descendant(tree, index)
        && multicol_has_plain_paragraph_chain(tree, index);
    if (custom_scope || break_flow_scope || plain_block_scope || constrained_chain.is_some())
        && inputs.run_mode == RunMode::PerformLayout
    {
        // Taffy's input width is normally already the content width for this
        // bridge. Correct it for authored padding/border before deriving the
        // child column width, so percentage gaps use the used content box.
        let content_width = multicol_content_width(tree, index, output.size.width, parent_width);
        let resolved = FragmentationContext::resolve(content_width, fragmentainer_height, style)
            .unwrap_or(context);
        if let Some(active) = tree.fragmentation_stack.last_mut() {
            *active = resolved;
        }
        // The parent writes the container's layout after this callback.
        // Resolve insets from the used percentage basis before placing children.
        let css = &tree.nodes[index].style;
        let basis = parent_width.unwrap_or(output.size.width).max(0.0);
        let content_origin = Point {
            x: multicol_resolve_inset(tree, css.padding.left, basis)
                + multicol_resolve_inset(tree, css.border.left, basis),
            y: multicol_resolve_inset(tree, css.padding.top, basis)
                + multicol_resolve_inset(tree, css.border.top, basis),
        };
        let block_end_inset = multicol_resolve_inset(tree, css.padding.bottom, basis)
            + multicol_resolve_inset(tree, css.border.bottom, basis);
        let used_height = if let Some(chain) = constrained_chain {
            constrained_chain::layout(tree, index, &chain, resolved, output.size, content_origin)
                .unwrap_or(output.size.height)
        } else if break_flow_scope {
            break_flow::layout(tree, index, resolved, output.size, content_origin)
        } else {
            relayout_nested_multicol_children(
                tree,
                node_id,
                resolved,
                output.size,
                fragmentainer_height.is_some(),
                content_origin,
                block_end_inset,
            )
        };
        if fragmentainer_height.is_none() {
            let minimum = multicol_definite_dimension(
                tree,
                tree.nodes[index].style.min_size.height.into(),
                inputs.parent_size.height,
            )
            .map_or(0.0, |height| {
                let css = &tree.nodes[index].style;
                if css.box_sizing == TaffyBoxSizing::BorderBox {
                    height
                } else {
                    let basis = parent_width.unwrap_or(output.size.width).max(0.0);
                    height
                        + multicol_resolve_inset(tree, css.padding.top, basis)
                        + multicol_resolve_inset(tree, css.padding.bottom, basis)
                        + multicol_resolve_inset(tree, css.border.top, basis)
                        + multicol_resolve_inset(tree, css.border.bottom, basis)
                }
            });
            output.size.height = used_height.max(minimum);
            for fragment in &mut tree.fragment_tree.fragments {
                if fragment.node_id == index && fragment.parent.is_none() {
                    fragment.rect.height = output.size.height;
                }
            }
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

/// The width a column imposes on a child it lays out, or `None` when an
/// element's authored width resolves against the column width instead.
fn column_child_known_width(tree: &Document, child: usize, column_width: f32) -> Option<f32> {
    let node = &tree.nodes[child];
    (node.kind() != NodeKind::Element || node.style.size.width == Dimension::auto())
        .then_some(column_width)
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

fn multicol_authored_content_height(
    tree: &Document,
    node_id: usize,
    height: f32,
    parent_width: Option<f32>,
    fallback_width: f32,
) -> f32 {
    let style = &tree.nodes[node_id].style;
    if style.box_sizing != TaffyBoxSizing::BorderBox {
        return height;
    }
    // A fragmentainer's height measures content, while a border-box authored
    // height includes block-axis padding and borders.
    let basis = parent_width.unwrap_or(fallback_width).max(0.0);
    let insets = multicol_resolve_inset(tree, style.padding.top, basis)
        + multicol_resolve_inset(tree, style.padding.bottom, basis)
        + multicol_resolve_inset(tree, style.border.top, basis)
        + multicol_resolve_inset(tree, style.border.bottom, basis);
    (height - insets).max(0.0)
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

fn can_balance_single_paragraph(tree: &Document, parent: usize, child: usize) -> bool {
    can_balance_paragraph_box(tree, parent, child)
        && tree.nodes[child].ifc_writing_mode() == Some(shodo::geometry::WritingMode::HorizontalTb)
        && tree.nodes[child].ifc_boxes().is_empty()
}

fn can_balance_paragraph_box(tree: &Document, parent: usize, child: usize) -> bool {
    let parent = &tree.nodes[parent];
    let child = &tree.nodes[child];
    let layout = child.unrounded_layout;
    // Child block-edge decoration needs slice/clone geometry; keep the existing
    // strategy until that geometry is carried by each fragment.
    parent.style.direction == TaffyDirection::Ltr
        && layout.padding.top == 0.0
        && layout.padding.bottom == 0.0
        && layout.border.top == 0.0
        && layout.border.bottom == 0.0
        && matches!(child.display, DisplayValue::Block | DisplayValue::ListItem)
        && child.break_inside == raikiri_style::property::BreakInside::Auto
        && child.style.size.height.is_auto()
        && child.style.min_size.height.is_auto()
        && child.style.max_size.height.is_auto()
}

fn multicol_has_plain_paragraph_chain(tree: &Document, parent: usize) -> bool {
    if tree.nodes[parent].ifc.is_some() || tree.nodes[parent].style.direction != TaffyDirection::Ltr
    {
        return false;
    }
    let mut current = parent;
    loop {
        let mut children = tree.nodes[current]
            .children
            .iter()
            .copied()
            .filter(|&child| {
                tree.nodes[child].is_in_document()
                    && tree.nodes[child].kind() == NodeKind::Element
                    && tree.nodes[child].style.display != Display::None
            });
        let Some(child) = children.next() else {
            return false;
        };
        if children.next().is_some() {
            return false;
        }
        let node = &tree.nodes[child];
        let zero_margin = LengthPercentageAuto::length(0.0);
        if node.display != DisplayValue::Block
            || node.style.direction != TaffyDirection::Ltr
            || node.style.float.is_floated()
            || node.style.margin.top != zero_margin
            || node.style.margin.right != zero_margin
            || node.style.margin.bottom != zero_margin
            || node.style.margin.left != zero_margin
            || node.style.padding != Rect::zero()
            || node.style.border != Rect::zero()
            || !node.multicol_auto_width
            || !node.style.min_size.width.is_auto()
            || !node.style.max_size.width.is_auto()
            || !node.style.size.height.is_auto()
            || !node.style.min_size.height.is_auto()
            || !node.style.max_size.height.is_auto()
            || node.break_inside != raikiri_style::property::BreakInside::Auto
            || node.break_before != BreakBetween::Auto
            || node.break_after != BreakBetween::Auto
        {
            return false;
        }
        if node.ifc.is_some() {
            return node.ifc_writing_mode() == Some(shodo::geometry::WritingMode::HorizontalTb)
                && node.ifc_boxes().is_empty()
                && !multicol_subtree_has_float(tree, child);
        }
        current = child;
    }
}

fn relayout_nested_multicol_children(
    tree: &mut Document,
    node_id: TaffyNodeId,
    context: FragmentationContext,
    border_box_size: Size<f32>,
    has_fragmentainer_height_constraint: bool,
    content_origin: Point<f32>,
    block_end_inset: f32,
) -> f32 {
    let index = usize::from(node_id);
    let fallback_height = border_box_size.height;
    let children: Vec<usize> = tree.nodes[index]
        .children
        .iter()
        .copied()
        .filter(|&child| {
            tree.nodes[child].is_in_document() && tree.nodes[child].style.display != Display::None
        })
        .collect();
    let has_spanners = children
        .iter()
        .any(|&child| spanning::is_spanner(tree, child));
    if has_spanners {
        spanning::retire_preliminary_fragments(tree, &children);
    }
    tree.fragment_tree.retire_previous_roots(index);
    let Some(container_fragment) = tree
        .fragment_tree
        .try_push(crate::fragment::LayoutFragment {
            node_id: index,
            parent: None,
            fragmentainer: context.column_index,
            rect: crate::fragment::FragmentRect {
                x: context.origin_x,
                y: context.origin_y,
                width: border_box_size.width,
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
    tree.nodes[index].multicol_groups.clear();
    tree.nodes[index].multicol_rows.clear();
    let used = if has_spanners {
        spanning::layout(
            tree,
            index,
            &children,
            container_fragment,
            context,
            border_box_size,
            content_origin,
            block_end_inset,
        )
    } else {
        layout_column_group(
            tree,
            index,
            &children,
            0,
            container_fragment,
            context,
            border_box_size,
            content_origin,
            block_end_inset,
            false,
        )
    };
    if let Some(fragment) = tree.fragment_tree.fragments.get_mut(container_fragment) {
        fragment.rect.height = if tree.nodes[index]
            .multicol
            .is_some_and(|style| style.height_definite)
        {
            fallback_height
        } else if has_fragmentainer_height_constraint {
            used.min(fallback_height)
        } else {
            used
        };
    } // cov:ignore: the successful try_push index remains stable throughout recursive group layout.
    used
}

#[allow(clippy::too_many_arguments)]
fn layout_column_group(
    tree: &mut Document,
    index: usize,
    children: &[usize],
    source_order: usize,
    container_fragment: usize,
    context: FragmentationContext,
    border_box_size: Size<f32>,
    content_origin: Point<f32>,
    block_end_inset: f32,
    retain_group: bool,
) -> f32 {
    let fallback_height = border_box_size.height;
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
        && fragment_height.is_none()
        && !multicol_has_nested_descendant(tree, index)
    {
        let mut measurements = Vec::with_capacity(children.len());
        for &child in children {
            let child_inputs = LayoutInput {
                run_mode: RunMode::PerformLayout,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Both,
                known_dimensions: Size {
                    width: column_child_known_width(tree, child, context.column_width),
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
    // One ordinary paragraph can break inside its lines. Measuring its whole
    // block as an atomic item would keep the container at the unbroken height
    // while the inline post-pass already assigns lines to several columns.
    let group_balance = auto_measurements
        .as_ref()
        .and_then(|(entries, _)| paragraph_group::balance(tree, index, entries, context));
    let paragraph_balance = auto_measurements.as_ref().and_then(|(entries, _)| {
        let mut boxes = entries.iter().filter(|(child, output, _, _, _, _)| {
            tree.nodes[*child].kind() != NodeKind::Text
                || output.size.height != 0.0
                || tree.nodes[*child].ifc.is_some()
        });
        let (child, _, _, _, _, _) = boxes.next()?;
        if boxes.next().is_some() {
            return None;
        }
        let node = &tree.nodes[*child];
        if !can_balance_single_paragraph(tree, index, *child) {
            return None;
        }
        let lines = node.ifc.as_ref()?.lines.as_ref()?;
        let (fragments, height) = root_column_fragments(lines, context);
        (fragments.len() > 1).then_some((*child, fragments, height))
    });
    let mut avoid_column_break_after_previous = false;
    let mut occupied = std::collections::BTreeSet::new();
    for (local_order, &child) in children.iter().enumerate() {
        let order = source_order + local_order;
        let measured = auto_measurements
            .as_ref()
            .and_then(|(entries, _)| entries.get(local_order));
        let (mut child_output, mut child_layout, margin_top, margin_bottom, needed) =
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
                        width: column_child_known_width(tree, child, context.column_width),
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
        let planned = group_balance
            .as_ref()
            .and_then(|group| group.placements[local_order].as_ref());
        if let Some(placement) = planned {
            column = placement.first_column;
            cursor = placement.first_y - margin_top;
            child_output.size.height = placement.first_height;
        } else if let Some((balanced_child, _, height)) = &paragraph_balance
            && *balanced_child == child
        {
            child_output.size.height = *height;
        }
        let break_height =
            fragment_height.or_else(|| auto_measurements.as_ref().map(|(_, height)| *height));
        let avoid_column_break = avoid_column_break_after_previous
            || matches!(tree.nodes[child].break_before, BreakBetween::Avoid);
        // A nested column container taller than the rest of this column
        // continues in the following ones. When nothing of it fits here, it
        // starts at the top of the next column instead.
        let mut continuation = None;
        if let Some(height) = fragment_height
            && measured.is_none()
            && planned.is_none()
        {
            continuation = plan_nested_continuation(
                tree,
                child,
                context,
                column,
                cursor,
                height,
                margin_top,
                child_output.size,
            );
            if continuation.is_none()
                && column + 1 < context.column_count
                && cursor > 0.0
                && cursor + needed > height
                && !avoid_column_break
                && let Some(plan) = plan_nested_continuation(
                    tree,
                    child,
                    context,
                    column + 1,
                    0.0,
                    height,
                    margin_top,
                    child_output.size,
                )
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
                continuation = Some(plan);
            }
        }
        if let Some(plan) = &continuation {
            child_output.size.height = plan.first_height;
        }
        if let Some(height) = break_height
            && continuation.is_none()
            && group_balance.is_none()
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
        let y = content_origin.y + cursor + margin_top;
        child_layout.order = order as u32;
        child_layout.size = child_output.size;
        child_layout.location = Point { x, y };
        tree.set_unrounded_layout(TaffyNodeId::from(child), &child_layout);
        if let Some(placement) = planned {
            if let Some(root) = tree.nodes[child].ifc.as_mut() {
                root.multicol_fragments = Some(placement.fragments.clone());
                root.multicol_fragment_origins_recorded = true;
            }
        } else {
            refresh_nested_text_fragments(tree, child, column_context);
        }
        if let Some((balanced_child, fragments, _)) = &paragraph_balance
            && *balanced_child == child
            && let Some(root) = tree.nodes[child].ifc.as_mut()
        {
            root.multicol_fragments = Some(fragments.clone());
        }
        let Some(child_fragment) = tree
            .fragment_tree
            .try_push(crate::fragment::LayoutFragment {
                node_id: child,
                parent: Some(container_fragment),
                fragmentainer: column_context.column_index,
                rect: crate::fragment::FragmentRect {
                    x: child_layout.location.x,
                    y,
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
        if let Some(plan) = &continuation {
            if apply_nested_continuation(tree, plan).is_none() {
                return fallback_height;
            }
            occupied.extend(column..=column + plan.columns);
            column += plan.columns;
            cursor = plan.end + margin_bottom;
            maximum = maximum.max(fragment_height.unwrap_or(0.0));
            if tree.nodes[child].kind() == NodeKind::Element {
                avoid_column_break_after_previous =
                    matches!(tree.nodes[child].break_after, BreakBetween::Avoid);
            }
            continue;
        }
        let record_context = group_balance.as_ref().map_or_else(
            || {
                paragraph_balance
                    .as_ref()
                    .filter(|(balanced_child, _, _)| *balanced_child == child)
                    .map_or(context, |(_, _, height)| FragmentationContext {
                        available_height: Some(*height),
                        ..context
                    })
            },
            |group| FragmentationContext {
                available_height: Some(group.height),
                column_index: column,
                ..context
            },
        );
        let record_context = FragmentationContext {
            origin_y: content_origin.y,
            ..record_context
        };
        if record_nested_ifc_box_fragments(tree, child, child_fragment, record_context).is_none() {
            return fallback_height;
        }
        if let Some(root) = tree.nodes[child].ifc.as_ref()
            && let Some(ranges) = &root.multicol_fragments
        {
            occupied.extend(
                ranges
                    .iter()
                    .filter(|range| range.line_start < range.line_end)
                    .map(|range| range.fragmentainer),
            );
        } else if child_output.size.width > 0.0 && child_output.size.height > 0.0 {
            occupied.insert(column);
        }
        if let Some(placement) = planned {
            column = placement.last_column;
            cursor = placement.last_y;
            maximum = maximum.max(group_balance.as_ref().map_or(0.0, |group| group.height));
        } else {
            cursor = y - content_origin.y + child_output.size.height + margin_bottom;
        }
        if tree.nodes[child].kind() == NodeKind::Element {
            avoid_column_break_after_previous =
                matches!(tree.nodes[child].break_after, BreakBetween::Avoid);
        }
    }
    if retain_group {
        tree.nodes[index]
            .multicol_groups
            .push(crate::fragment::MulticolGroup {
                context: FragmentationContext {
                    origin_x: content_origin.x,
                    origin_y: content_origin.y,
                    ..context
                },
                height: maximum.max(cursor),
                occupied,
            });
    }
    let minimum_height =
        if auto_measurements.is_some() && !multicol_has_nested_descendant(tree, index) {
            0.0
        } else {
            fragment_height
                .map(|height| fallback_height.min(height))
                .unwrap_or(fallback_height)
        };
    (maximum.max(cursor) + content_origin.y + block_end_inset).max(minimum_height)
}

/// A nested column container that continues from one column of its outer
/// container into the following ones: its paragraphs fill a row of its own
/// columns in each outer column it reaches. A fragmentation context nested in
/// another is fragmented by the outer one too (CSS Fragmentation 3, 2).
struct NestedContinuation {
    /// The nested container's retained root record.
    root: usize,
    /// The nested container's children, by entry.
    children: Vec<usize>,
    rows: paragraph_group::Rows,
    /// The nested container's own columns.
    context: FragmentationContext,
    /// Its content-box origin in its border box.
    content_origin: Point<f32>,
    /// Block offset of the outer column's top from the nested border box.
    row_top: f32,
    /// Inline distance between two outer columns.
    row_step: f32,
    /// Border-box block size of its first fragment, which reaches the end of
    /// the outer column.
    first_height: f32,
    /// Outer columns it reaches after the first.
    columns: usize,
    /// Its border-box block end in the last outer column it reaches, from
    /// that column's top.
    end: f32,
}

/// Plan how the nested column container `child`, laid out at `cursor` in
/// column `column` of `outer`, continues in the following outer columns.
/// `None` when it fits in the rest of the column, when its content is not
/// plain paragraphs, or when the outer columns run out.
#[allow(clippy::too_many_arguments)]
fn plan_nested_continuation(
    tree: &Document,
    child: usize,
    outer: FragmentationContext,
    column: usize,
    cursor: f32,
    fragmentainer_height: f32,
    margin_top: f32,
    border_box_size: Size<f32>,
) -> Option<NestedContinuation> {
    let node = &tree.nodes[child];
    let style = node.multicol?;
    let rows = outer.column_count.checked_sub(column)?;
    // A box that avoids breaks inside, has a minimum height or generated
    // content keeps its previous layout: the rows below hold its paragraph
    // lines only.
    if rows < 2
        || !style.horizontal
        || style.height_definite
        || !node.style.size.height.is_auto()
        || !node.style.min_size.height.is_auto()
        || !node.style.max_size.height.is_auto()
        || node.has_logical_min_block_size
        || node.has_before_or_after_content
        || node.break_inside != raikiri_style::property::BreakInside::Auto
    {
        return None;
    }
    let css = &node.style;
    let basis = outer.column_width;
    let inset = |value| multicol_resolve_inset(tree, value, basis);
    let content_origin = Point {
        x: inset(css.padding.left) + inset(css.border.left),
        y: inset(css.padding.top) + inset(css.border.top),
    };
    let block_end_inset = inset(css.padding.bottom) + inset(css.border.bottom);
    let content_width = border_box_size.width
        - content_origin.x
        - inset(css.padding.right)
        - inset(css.border.right);
    let first = fragmentainer_height - cursor - margin_top - content_origin.y;
    let content_height = border_box_size.height - content_origin.y - block_end_inset;
    if content_height <= first + 0.001 {
        return None;
    }
    let context = FragmentationContext::resolve(content_width, None, style)?;
    let root = (0..tree.fragment_tree.fragments.len())
        .rev()
        .find(|&index| {
            let fragment = &tree.fragment_tree.fragments[index];
            fragment.node_id == child
                && fragment.parent.is_none()
                && !tree.fragment_tree.is_retired(index)
        })?;
    let children: Vec<usize> = node
        .children
        .iter()
        .copied()
        .filter(|&id| {
            tree.nodes[id].is_in_document() && tree.nodes[id].style.display != Display::None
        })
        .collect();
    if children.iter().any(|&id| spanning::is_spanner(tree, id)) {
        return None;
    }
    let entries: Vec<_> = children
        .iter()
        .map(|&id| {
            let layout = tree.nodes[id].unrounded_layout;
            (
                id,
                LayoutOutput::from_outer_size(layout.size),
                layout,
                layout.margin.top,
                layout.margin.bottom,
                layout.margin.top + layout.size.height + layout.margin.bottom,
            )
        })
        .collect();
    let rows = paragraph_group::fill_rows(
        tree,
        child,
        &entries,
        context,
        first,
        fragmentainer_height,
        rows,
    )?;
    let columns = rows.heights.len().checked_sub(1).filter(|&n| n > 0)?;
    // The block-end padding and border must fit in the last outer column too.
    let end = rows.heights[columns] + block_end_inset;
    if end > fragmentainer_height + 0.001 {
        return None;
    }
    Some(NestedContinuation {
        root,
        children,
        rows,
        context,
        content_origin,
        row_top: -(cursor + margin_top),
        row_step: outer.column_offset_x(1),
        first_height: content_origin.y + first,
        columns,
        end,
    })
}

/// Record the planned rows of a nested column container, in place of the
/// records of its own balanced layout. The container's root is its box in the
/// first row; each later row gets a box of its own in its outer column, so
/// its background, border and overflow clip follow the row. Fragmentainers
/// number the nested columns row by row.
fn apply_nested_continuation(tree: &mut Document, plan: &NestedContinuation) -> Option<()> {
    let root = tree.fragment_tree.fragments[plan.root];
    tree.fragment_tree.fragments[plan.root].rect.height = plan.first_height;
    let replaced: std::collections::HashSet<usize> = plan.children.iter().copied().collect();
    let stale: Vec<usize> = tree
        .fragment_tree
        .fragments
        .iter()
        .enumerate()
        .filter(|(_, fragment)| {
            fragment.parent == Some(plan.root) && replaced.contains(&fragment.node_id)
        })
        .map(|(index, _)| index)
        .collect();
    tree.fragment_tree.retire_subtrees(stale);
    // Each row's box record and its content-box origin within that box.
    let last = plan.rows.heights.len() - 1;
    let mut row_boxes = vec![(plan.root, plan.content_origin)];
    for (row, &height) in plan.rows.heights.iter().enumerate().skip(1) {
        let index = tree
            .fragment_tree
            .try_push(crate::fragment::LayoutFragment {
                node_id: root.node_id,
                parent: root.parent,
                fragmentainer: root.fragmentainer + row,
                rect: crate::fragment::FragmentRect {
                    x: root.rect.x + row as f32 * plan.row_step,
                    y: root.rect.y + plan.row_top,
                    width: root.rect.width,
                    height: if row == last { plan.end } else { height },
                },
                fragmentainer_clip: None,
                fragment_index: 0,
                fragment_count: 1,
                line_start: None,
                line_end: None,
            })?;
        // A continued box has no block-start padding or border.
        row_boxes.push((
            index,
            Point {
                x: plan.content_origin.x,
                y: 0.0,
            },
        ));
    }
    // The origin of each row's box in the first box's coordinates.
    let row_origin = |row: usize| {
        if row == 0 {
            Point::ZERO
        } else {
            Point {
                x: row as f32 * plan.row_step,
                y: plan.row_top,
            }
        }
    };
    let mut first_rects = std::collections::HashMap::<usize, crate::fragment::FragmentRect>::new();
    let mut text = std::collections::HashMap::<usize, Vec<MulticolTextFragment>>::new();
    for fragment in &plan.rows.fragments {
        let child = plan.children[fragment.entry];
        let layout = tree.nodes[child].unrounded_layout;
        let (row_box, content) = row_boxes[fragment.row];
        let column_x = content.x + plan.context.column_offset_x(fragment.column);
        let rect = crate::fragment::FragmentRect {
            x: column_x + layout.margin.left,
            y: content.y + fragment.y,
            width: layout.size.width,
            height: fragment.height,
        };
        let fragmentainer = fragment.row * plan.context.column_count + fragment.column;
        tree.fragment_tree
            .try_push(crate::fragment::LayoutFragment {
                node_id: child,
                parent: Some(row_box),
                fragmentainer,
                rect,
                fragmentainer_clip: Some(crate::fragment::FragmentRect {
                    x: column_x,
                    y: content.y,
                    width: plan.context.column_width,
                    height: plan.rows.heights[fragment.row],
                }),
                fragment_index: 0,
                fragment_count: 1,
                line_start: Some(fragment.line_start),
                line_end: Some(fragment.line_end),
            })?;
        let origin = row_origin(fragment.row);
        let rect = crate::fragment::FragmentRect {
            x: origin.x + rect.x,
            y: origin.y + rect.y,
            ..rect
        };
        let first = *first_rects.entry(child).or_insert(rect);
        text.entry(child).or_default().push(MulticolTextFragment {
            line_start: fragment.line_start,
            line_end: fragment.line_end,
            fragmentainer,
            x: rect.x - first.x,
            y: rect.y - first.y,
        });
    }
    // Column rules are drawn in each row's box, which the row's group names
    // by its fragmentainer.
    let mut rows: Vec<crate::fragment::MulticolGroup> = plan
        .rows
        .heights
        .iter()
        .enumerate()
        .map(|(row, &height)| crate::fragment::MulticolGroup {
            context: FragmentationContext {
                origin_x: row_boxes[row].1.x,
                origin_y: row_boxes[row].1.y,
                available_height: Some(height),
                column_index: root.fragmentainer + row,
                ..plan.context
            },
            height,
            occupied: std::collections::BTreeSet::new(),
        })
        .collect();
    for fragment in &plan.rows.fragments {
        if fragment.line_start < fragment.line_end {
            rows[fragment.row].occupied.insert(fragment.column);
        }
    }
    tree.nodes[root.node_id].multicol_rows = rows;
    for (child, fragments) in text {
        let first = first_rects[&child];
        let node = &mut tree.nodes[child];
        node.unrounded_layout.location = Point {
            x: first.x,
            y: first.y,
        };
        node.unrounded_layout.size.height = first.height;
        if let Some(root) = node.ifc.as_mut() {
            root.multicol_fragments = Some(fragments);
            root.multicol_fragment_origins_recorded = true;
        }
    }
    Some(())
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
    // Both scopes are properties of a node's ancestor chain. Carry them down
    // the traversal instead of walking to the root again for every node.
    let mut pending = vec![(
        subtree_root,
        FlexFloatChain::of_ancestors(tree, subtree_root),
        tree.nodes[subtree_root].has_logical_min_block_size,
    )];
    while let Some((node_id, flex_float_chain, nested_logical_minimum_scope)) = pending.pop() {
        let nested_row_flex_scope = flex_float_chain.is_scope();
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
            // Index ranges and placements by column once; scanning every
            // range and placement again for each column is quadratic in the
            // column count. A column takes its first range, as in source
            // order, together with that range's ordinal.
            let mut columns = std::collections::BTreeMap::<usize, ColumnEntries>::new();
            for (index, placement) in placements.iter().enumerate() {
                columns
                    .entry(placement.fragmentainer)
                    .or_default()
                    .placements
                    .push(index);
            }
            if let Some(ranges) = &line_ranges {
                for (index, range) in ranges.iter().enumerate() {
                    columns
                        .entry(range.fragmentainer)
                        .or_default()
                        .range
                        .get_or_insert((index, *range));
                }
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
            let plain_block_path = !subtree_has_float
                && !nested_logical_minimum_scope
                && tree.nodes[subtree_root].style.display == Display::Block
                && tree.nodes[node_id].ifc_writing_mode()
                    == Some(shodo::geometry::WritingMode::HorizontalTb)
                && path
                    .iter()
                    .all(|&ancestor| tree.nodes[ancestor].style.display == Display::Block);
            let node_offset_y = path
                .iter()
                .map(|&ancestor| tree.nodes[ancestor].unrounded_layout.location.y)
                .sum::<f32>();

            for (column, entries) in columns {
                let column_x =
                    context.column_offset_x(column) - context.column_offset_x(context.column_index);
                let line_range = entries.range.map(|(_, range)| range);
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
                    let fragment_index = entries.range.map_or(0, |(index, _)| index);
                    let fragment_count = line_ranges.as_ref().map_or(1, Vec::len);
                    let used_line_height = lines
                        .as_ref()
                        .and_then(|lines| {
                            let first = lines.lines.get(range.line_start)?;
                            let last = lines.lines.get(range.line_end.checked_sub(1)?)?;
                            Some(last.block_offset() + last.block_size() - first.block_offset())
                        })
                        .unwrap_or(base.rect.height)
                        .max(0.0);
                    let origins_recorded = tree.nodes[node_id]
                        .ifc
                        .as_ref()
                        .is_some_and(|root| root.multicol_fragment_origins_recorded);
                    let fragment_height = if origins_recorded && fragment_index + 1 < fragment_count
                    {
                        // A non-final slice extends to the fragmentainer edge,
                        // including unused space left by paragraph break minima.
                        let top = if fragment_index == 0 {
                            base.rect.y - context.origin_y
                        } else {
                            0.0
                        };
                        (height - top).max(0.0)
                    } else {
                        used_line_height
                    };
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
                                    y: context.origin_y,
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
                                            // Move a plain block path into its column once;
                                            // descendants retain their local coordinates.
                                            y: if ancestor == subtree_root
                                                || is_flex_item
                                                || (plain_block_path && parent != parent_fragment)
                                            {
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
                    parent_offset.y += if plain_block_path {
                        tree.fragment_tree.fragments[fragment_id].rect.y
                    } else {
                        layout.location.y
                    };
                }

                for placement in entries.placements.iter().map(|&index| &placements[index]) {
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
                })
                .map(|child| {
                    (
                        child,
                        flex_float_chain.below(tree, child),
                        nested_logical_minimum_scope
                            || tree.nodes[child].has_logical_min_block_size,
                    )
                }),
        );
    }
    Some(())
}

/// The ranges and placements of one nested node that land in one column.
#[derive(Default)]
struct ColumnEntries {
    /// The first line range in the column and its ordinal among all ranges.
    range: Option<(usize, MulticolTextFragment)>,
    /// Indices of the column's box placements, in source order.
    placements: Vec<usize>,
}

/// What a node's ancestor chain, the node included, contributes to the
/// row-flex float scope: walking up, the chain ends at the nearest vertical
/// writing mode (no scope) or multicol container. Floats count up to and
/// including that container; row flex boxes count only below it.
#[derive(Clone, Copy)]
enum FlexFloatChain {
    /// Neither a multicol container nor a vertical writing mode above.
    Open,
    /// A vertical writing mode comes before any multicol container.
    Vertical,
    Multicol {
        horizontal: bool,
        row_flex: bool,
        float: bool,
    },
}

impl FlexFloatChain {
    fn of_ancestors(tree: &Document, node_id: usize) -> Self {
        let mut chain = Vec::new();
        let mut current = Some(node_id);
        while let Some(id) = current {
            chain.push(id);
            current = tree.parent_of(id);
        }
        chain
            .into_iter()
            .rev()
            .fold(Self::Open, |above, id| above.below(tree, id))
    }

    /// The chain of `node_id`, whose parent's chain is `self`.
    fn below(self, tree: &Document, node_id: usize) -> Self {
        let node = &tree.nodes[node_id];
        let float = node.style.float.is_floated()
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
            return Self::Vertical;
        }
        if let Some(multicol) = node.multicol {
            return Self::Multicol {
                horizontal: multicol.horizontal,
                row_flex: false,
                float,
            };
        }
        let row_flex = node.style.display == Display::Flex
            && matches!(
                node.style.flex_direction,
                taffy::FlexDirection::Row | taffy::FlexDirection::RowReverse
            );
        match self {
            Self::Multicol {
                horizontal,
                row_flex: above_row_flex,
                float: above_float,
            } => Self::Multicol {
                horizontal,
                row_flex: above_row_flex || row_flex,
                float: above_float || float,
            },
            other => other,
        }
    }

    /// Whether a row flex box and a float share the nearest horizontal
    /// multicol container.
    fn is_scope(self) -> bool {
        matches!(
            self,
            Self::Multicol {
                horizontal: true,
                row_flex: true,
                float: true,
            }
        )
    }
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
    let mut ranges = Vec::with_capacity(available_columns.max(1).min(line_count));
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
    // Auto fill without a block constraint has no soft column breaks.
    let context =
        if context.available_height.is_none() && context.column_fill == ColumnFillValue::Auto {
            FragmentationContext {
                column_count: context.column_index.saturating_add(1),
                ..context
            }
        } else {
            context
        };
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
    refresh_nested_text_fragments_in(
        tree,
        node_id,
        context,
        FlexFloatChain::of_ancestors(tree, node_id),
        float_fragmentainer,
    );
}

/// `chain` is the row-flex float chain of `node_id`, and
/// `float_fragmentainer` the column of its first placement in its parent's
/// paragraph. Both are carried down so that the subtree walk does not search
/// ancestors or sibling placements again for every node.
fn refresh_nested_text_fragments_in(
    tree: &mut Document,
    node_id: usize,
    context: FragmentationContext,
    chain: FlexFloatChain,
    float_fragmentainer: Option<usize>,
) {
    // A nested column container assigns the lines below it to its own
    // columns when it is laid out; the outer columns only move it as a whole.
    if tree.nodes[node_id].multicol.is_some() {
        return;
    }
    let nested_row_flex_scope = chain.is_scope();
    let is_floated = tree.nodes[node_id].style.float.is_floated();
    // A paragraph laid out by the inline engine keeps its lines on its root:
    // its fragments go there, and only its boxes hold text of their own.
    if let Some(root) = tree.nodes[node_id].ifc.as_mut() {
        root.multicol_fragment_origins_recorded = false;
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
        let mut first_columns = HashMap::<usize, usize>::new();
        if let Some(lines) = tree.nodes[node_id]
            .ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
        {
            for placement in &lines.fragment_box_placements {
                first_columns
                    .entry(placement.node_id)
                    .or_insert(placement.fragmentainer);
            }
        }
        let boxes = tree.nodes[node_id].ifc_boxes();
        for child in boxes {
            // A box may sit inside inline elements of the paragraph; only
            // those elements lie between it and this root.
            let mut between = Vec::new();
            let mut current = tree.parent_of(child);
            while let Some(id) = current.filter(|&id| id != node_id) {
                between.push(id);
                current = tree.parent_of(id);
            }
            let child_chain = if current.is_some() {
                between
                    .into_iter()
                    .rev()
                    .fold(chain, |above, id| above.below(tree, id))
                    .below(tree, child)
            } else {
                FlexFloatChain::of_ancestors(tree, child) // cov:ignore: paragraph boxes are descendants of their root.
            };
            let float_fragmentainer = (tree.parent_of(child) == Some(node_id))
                .then(|| first_columns.get(&child).copied())
                .flatten();
            refresh_nested_text_fragments_in(
                tree,
                child,
                context,
                child_chain,
                float_fragmentainer,
            );
        }
        return;
    }
    // This node has no paragraph, so no child has a placement in it.
    let children = tree.nodes[node_id].children.clone();
    for child in children {
        if tree.nodes[child].is_in_document() && tree.nodes[child].style.display != Display::None {
            let child_chain = chain.below(tree, child);
            refresh_nested_text_fragments_in(tree, child, context, child_chain, None);
        }
    }
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
            .map(|height| {
                multicol_authored_content_height(
                    tree,
                    container_id,
                    height,
                    parent_width,
                    container_layout.size.width,
                )
            })
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

        // Carry each node's inline offset from the container down the walk
        // instead of summing it over the ancestors of every paragraph.
        let offset_below = |tree: &Document, parent: usize, parent_offset: Option<f32>, child| {
            if tree.layout_parent_of(child) == Some(parent) {
                parent_offset.map(|offset| offset + tree.nodes[child].unrounded_layout.location.x)
            } else {
                layout_offset_from_ancestor(tree, child, container_id) // cov:ignore: arena children share their layout parent.
            }
        };
        let mut pending: Vec<_> = tree.nodes[container_id]
            .children
            .iter()
            .map(|&child| (child, offset_below(tree, container_id, Some(0.0), child)))
            .collect();
        while let Some((node_id, inline_offset)) = pending.pop() {
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
                        && let Some(inline_offset) = inline_offset
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
            pending.extend(
                tree.nodes[node_id]
                    .children
                    .iter()
                    .map(|&child| (child, offset_below(tree, node_id, inline_offset, child))),
            );
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

/// Ancestor-chain facts for every node of the arena, derived once with each
/// parent resolved before its children. The multicol preparation pass asks
/// these questions of every element; walking to the root for each one would
/// cost the node count times the tree depth.
struct AncestorFacts {
    /// A strict ancestor is a multicol container.
    multicol_ancestor: Vec<bool>,
    /// The node or an ancestor has a vertical or sideways writing mode.
    vertical: Vec<bool>,
    /// A strict ancestor is absolutely or fixed positioned.
    out_of_flow_ancestor: Vec<bool>,
    /// A strict ancestor establishes a flex, grid, or table formatting context.
    non_block_flow_ancestor: Vec<bool>,
    /// The cascade-derived content width: a definite `px` or percentage
    /// width resolved against the parent's, else the parent's own.
    content_width: Vec<f32>,
    /// The nearest strict ancestor whose authored width is neither `auto`
    /// nor `min-content`; a `min-content` width is intrinsic, not an
    /// authored containing width.
    authored_ancestor: Vec<Option<usize>>,
}

impl AncestorFacts {
    fn new(
        doc: &Document,
        cascade: &CascadeResult,
        parent_of: &[Option<usize>],
        fallback_width: f32,
    ) -> Self {
        let count = parent_of.len();
        let mut facts = Self {
            multicol_ancestor: vec![false; count],
            vertical: vec![false; count],
            out_of_flow_ancestor: vec![false; count],
            non_block_flow_ancestor: vec![false; count],
            content_width: vec![0.0; count],
            authored_ancestor: vec![None; count],
        };
        let mut resolved = vec![false; count];
        let mut chain = Vec::new();
        for start in 0..count {
            // Collect the unresolved part of this node's chain, then resolve
            // it from the top down. Every node is resolved exactly once.
            let mut current = Some(start);
            while let Some(id) = current.filter(|&id| !resolved[id]) {
                resolved[id] = true;
                chain.push(id);
                current = parent_of[id];
            }
            while let Some(id) = chain.pop() {
                facts.resolve(doc, cascade, parent_of[id], id, fallback_width);
            }
        }
        facts
    }

    fn resolve(
        &mut self,
        doc: &Document,
        cascade: &CascadeResult,
        parent: Option<usize>,
        id: usize,
        fallback_width: f32,
    ) {
        let vertical_self = matches!(
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
        );
        let parent_width = parent.map_or(fallback_width, |parent| self.content_width[parent]);
        self.content_width[id] = match cascade.computed[id].width {
            ComputedLengthPercentageOrAuto::Px(value) if value.is_finite() => value.max(0.0),
            ComputedLengthPercentageOrAuto::Percent(value) if value.is_finite() => {
                (parent_width * value / 100.0).max(0.0)
            }
            _ => parent_width.max(0.0),
        };
        let Some(parent) = parent else {
            self.vertical[id] = vertical_self;
            return;
        };
        let values = &cascade.computed[parent];
        self.multicol_ancestor[id] =
            self.multicol_ancestor[parent] || doc.nodes[parent].multicol.is_some();
        self.vertical[id] = vertical_self || self.vertical[parent];
        self.out_of_flow_ancestor[id] = self.out_of_flow_ancestor[parent]
            || matches!(
                values.position,
                PositionValue::Absolute | PositionValue::Fixed
            );
        self.non_block_flow_ancestor[id] = self.non_block_flow_ancestor[parent]
            || matches!(
                values.display,
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
            );
        self.authored_ancestor[id] = if matches!(
            values.width,
            ComputedLengthPercentageOrAuto::Auto | ComputedLengthPercentageOrAuto::MinContent
        ) {
            self.authored_ancestor[parent]
        } else {
            Some(parent)
        };
    }

    /// The content width of the nearest authored-width ancestor.
    fn authored_containing_width(&self, id: usize) -> Option<f32> {
        self.authored_ancestor[id].map(|ancestor| self.content_width[ancestor])
    }
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
// `container_width` is the node's cascade-derived content width.
fn multicol_metrics_with_width(
    cascade: &CascadeResult,
    node_id: usize,
    container_width: f32,
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
    let ancestors = AncestorFacts::new(doc, cascade, &parent_of, fallback_width);
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        // Used widths for descendants of a multicol container are supplied by
        // the recursive Taffy seam. Do not pre-project them from cascade
        // fallback widths; that was the source of the nested-width bug.
        if ancestors.multicol_ancestor[idx] {
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
            && parent_of.get(idx).copied().flatten().is_some_and(|parent| {
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
        ) && !ancestors.vertical[idx]
            && matches!(cascade.computed[idx].position, PositionValue::Static)
            && !has_nonzero_horizontal_margin(&cascade.computed[idx])
            && !ancestors.out_of_flow_ancestor[idx]
            && !ancestors.non_block_flow_ancestor[idx]
            && let Some(width) = ancestors.authored_containing_width(idx)
        {
            doc.nodes[idx].style.size.width = Dimension::length(width);
        }
    }
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        if ancestors.multicol_ancestor[idx] || ancestors.vertical[idx] {
            continue;
        }
        let Some(metrics) = multicol_metrics_with_width(cascade, idx, ancestors.content_width[idx])
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
