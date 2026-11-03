//! Removal of inherited float prefixes when their owning box leaves a page.

use super::*;

/// Consumes only a proven empty prefix below inherited floats. Owned boxes
/// keep their layout relative to the full-width lines below that prefix.
#[allow(clippy::too_many_arguments)]
pub(super) fn discard(
    document: &mut super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    root: usize,
    delta: f32,
    body_id: usize,
    shifts: &mut SubtreeShifts,
    flow_page: u32,
) -> (f32, f32) {
    let gap = document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .filter(|ifc| ifc.multicol_fragments.is_none())
        .and_then(|ifc| ifc.lines.as_ref())
        .map_or(0.0, |lines| lines.external_float_gap);
    let removed = gap.min(delta).max(0.0);
    if removed <= 0.001 {
        return (0.0, 0.0);
    }
    let boxes: Vec<usize> = document
        .ifc_layout_node(root)
        .and_then(|node| node.ifc.as_ref())
        .into_iter()
        .flat_map(|ifc| &ifc.boxes)
        .filter(|item| matches!(item.kind, IfcBoxKind::Atomic | IfcBoxKind::Float))
        .map(|item| item.node)
        .collect();
    let growth = shift_paragraph_lines(
        document,
        cascade,
        &LineMove {
            block_id: root,
            flex_line: None,
            line: 0,
            shift: -removed,
        },
    );
    for node in boxes {
        document.nodes[node].layout.location.y -= removed;
    }
    if let Some(lines) = document
        .table_layout_node_mut(root)
        .ifc
        .as_mut()
        .and_then(|ifc| ifc.lines.as_mut())
    {
        lines.external_float_gap -= removed;
        lines.discarded_float_gap += removed;
    }
    shrink_ancestors_after_content_change(
        document,
        cascade,
        root,
        growth,
        -removed,
        body_id,
        &IfcBoxShifts::default(),
        Some((shifts, flow_page)),
    );
    (removed, growth)
}

/// Source intervals and float owners prepared once for a pagination attempt.
#[derive(Default)]
pub(super) struct Index {
    prepared: bool,
    order: Vec<usize>,
    ends: Vec<usize>,
    roots: BTreeMap<usize, (usize, usize)>,
    floats: HashMap<usize, Vec<usize>>,
    #[cfg(test)]
    float_reads: usize,
}

impl Index {
    pub(super) fn prepare(
        &mut self,
        document: &mut super::super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
    ) {
        if self.prepared {
            return;
        }
        self.prepared = true;
        if document.context.pagination_on_transposed_axes {
            // The proof uses physical vertical coordinates; transposed
            // pagination must not inspect native style or consume it.
            for node in 0..document.nodes.len() {
                if let Some(lines) = document
                    .table_layout_node_mut(node)
                    .ifc
                    .as_mut()
                    .and_then(|ifc| ifc.lines.as_mut())
                {
                    lines.external_float_gap = 0.0;
                }
            }
            return;
        }
        self.order = vec![usize::MAX; document.nodes.len()];
        self.ends = vec![usize::MAX; document.nodes.len()];
        let mut pending: Vec<_> = document.nodes[0]
            .children
            .iter()
            .rev()
            .map(|&node| (node, false))
            .collect();
        let mut next = 0;
        while let Some((node, exiting)) = pending.pop() {
            if exiting {
                self.ends[node] = next;
                continue;
            }
            let item = &document.nodes[node];
            if item.display() == DisplayValue::None || !item.is_in_document() {
                continue;
            }
            self.order[node] = next;
            next += 1;
            if document
                .ifc_layout_node(node)
                .and_then(|node| node.ifc.as_ref())
                .and_then(|ifc| ifc.lines.as_ref())
                .is_some_and(|lines| lines.external_float_gap > 0.0)
            {
                self.roots.insert(
                    self.order[node],
                    (node, float_context_root(document, cascade, node, body_id)),
                );
            }
            if is_floating_box_for_pagination(document, cascade, node)
                && matches!(
                    cascade.computed[node].float,
                    FloatValue::Left | FloatValue::Right
                )
                && !matches!(
                    item.position(),
                    PositionValue::Absolute | PositionValue::Fixed
                )
            {
                self.floats
                    .entry(float_context_root(document, cascade, node, body_id))
                    .or_default()
                    .push(node);
            }
            pending.push((node, true));
            pending.extend(item.children.iter().rev().map(|&node| (node, false)));
        }
        if self.roots.is_empty() {
            return;
        }
        let mut owned: HashMap<usize, Vec<usize>> = HashMap::new();
        for (&node, coordinate) in &document.oof_coordinates {
            owned.entry(coordinate.owner).or_default().push(node);
        }
        let unsafe_ancestors: HashSet<_> = owned
            .iter()
            .filter_map(|(&owner, children)| {
                children
                    .iter()
                    .any(|&node| {
                        let style = document.physical_style(node);
                        !is_top_anchored_out_of_flow(document, cascade, node)
                            || follows_height(style.size().height)
                            || follows_height(style.min_size().height)
                            || follows_height(style.max_size().height)
                    })
                    .then_some(owner)
            })
            .collect();
        let rejected: Vec<_> = self
            .roots
            .iter()
            .filter_map(|(&order, &(root, _))| {
                let ancestor_has_oof = document
                    .ancestors(root)
                    .take_while(|&ancestor| ancestor != body_id)
                    .chain((root != body_id).then_some(body_id))
                    .any(|ancestor| unsafe_ancestors.contains(&ancestor));
                let unsafe_child = owned.get(&root).is_some_and(|children| {
                    let retained: HashSet<usize> = document
                        .ifc_layout_node(root)
                        .and_then(|node| node.ifc.as_ref())
                        .into_iter()
                        .flat_map(|ifc| &ifc.boxes)
                        .filter(|item| item.kind == IfcBoxKind::OutOfFlow)
                        .map(|item| item.node)
                        .collect();
                    children.iter().any(|&node| {
                        let style = document.physical_style(node);
                        let inset = style.inset();
                        !retained.contains(&node)
                            || follows_height(style.size().height)
                            || follows_height(style.min_size().height)
                            || follows_height(style.max_size().height)
                            || (!inset.top.is_auto() && !inset.bottom.is_auto())
                            || out_of_flow_follow(document, cascade, root, node).is_none()
                    })
                });
                (ancestor_has_oof || unsafe_child).then_some((order, root))
            })
            .collect();
        for (order, root) in rejected {
            self.roots.remove(&order);
            if let Some(lines) = document
                .table_layout_node_mut(root)
                .ifc
                .as_mut()
                .and_then(|ifc| ifc.lines.as_mut())
            {
                lines.external_float_gap = 0.0;
            }
        }
    }

    /// Removes only prefixes whose floats remain outside the moving holder.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn discard_in_holder(
        &mut self,
        document: &mut super::super::context::LayoutSession<'_>,
        cascade: &CascadeResult,
        holder: usize,
        delta: f32,
        body_id: usize,
        shifts: &mut SubtreeShifts,
        flow_page: u32,
    ) -> Vec<(usize, f32, f32)> {
        self.prepare(document, cascade, body_id);
        if self.roots.is_empty() {
            return Vec::new();
        }
        let start = self.order[holder];
        let end = self.ends[holder];
        let roots: Vec<_> = self
            .roots
            .range(start..end)
            .map(|(&order, &(root, context))| (order, root, context, current_abs_y(document, root)))
            .collect();
        // Earlier roots can move only later boxes in source order. The
        // retained floats already visited therefore keep their normal ends.
        let mut sweep: HashMap<usize, (usize, f32)> = HashMap::new();
        let mut consumed = Vec::new();
        for (order, root, context, original_top) in roots {
            let node = &document.nodes[root];
            let content_top = current_abs_y(document, root)
                - accumulated_relative_offset_y(document, cascade, body_id, root)
                + node.layout.border.top
                + node.layout.padding.top;
            let floats = self.floats.get(&context).map_or(&[][..], Vec::as_slice);
            let (cursor, maximum) = sweep.entry(context).or_insert_with(|| {
                (
                    floats.partition_point(|&float| self.order[float] < start),
                    f32::NEG_INFINITY,
                )
            });
            while let Some(&float) = floats.get(*cursor)
                && self.order[float] < order
            {
                #[cfg(test)]
                {
                    self.float_reads += 1;
                }
                let layout = &document.nodes[float].layout;
                let end = current_abs_y(document, float)
                    - accumulated_relative_offset_y(document, cascade, body_id, float)
                    + layout.size.height
                    + layout.margin.bottom;
                *maximum = maximum.max(end);
                *cursor += 1;
            }
            let retained = (*maximum - content_top).max(0.0);
            let gap = document
                .ifc_layout_node(root)
                .and_then(|node| node.ifc.as_ref())
                .and_then(|ifc| ifc.lines.as_ref())
                .map_or(0.0, |lines| lines.external_float_gap);
            let moved_by = delta + current_abs_y(document, root) - original_top;
            let allowed = moved_by.min(gap - retained).max(0.0);
            let (removed, growth) =
                discard(document, cascade, root, allowed, body_id, shifts, flow_page);
            if gap - removed <= 0.001 {
                self.roots.remove(&order);
            }
            if removed > 0.0 {
                consumed.push((root, removed, growth));
            }
        }
        consumed
    }
}

/// The visible line extent an avoiding box must take to its new page,
/// including content that exceeds a bounded border-box height.
pub(super) fn fit_height(document: &Document, candidate: &PageCandidate) -> f32 {
    document
        .ifc_layout_node(candidate.node_id)
        .and_then(|node| node.ifc.as_ref().map(|ifc| (node, ifc)))
        .and_then(|(node, ifc)| ifc.lines.as_ref().map(|lines| (node, lines)))
        .filter(|(_, lines)| lines.external_float_gap > 0.0)
        .map_or(candidate.height, |(node, lines)| {
            candidate
                .height
                .max(node.layout.border.top + node.layout.padding.top + lines.shifted_height())
        })
}

#[cfg(test)]
mod tests;
