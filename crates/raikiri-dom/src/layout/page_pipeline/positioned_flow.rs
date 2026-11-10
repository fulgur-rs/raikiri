//! Fragment the block flow inside an absolute box after its outer flow settles.

use super::pagination_state::{FlowEnv, Halt, reach_page};
use super::*;
use std::ops::Range;

enum Event {
    Box(usize),
    Lines(usize, Range<usize>),
}

struct ContentChange {
    end_delta: f32,
    float_delta: f32,
    position_delta: f32,
}

fn padding_box_height(document: &Document, owner: usize) -> f32 {
    let layout = document.nodes[owner].layout;
    (layout.size.height - layout.border.top - layout.border.bottom).max(0.0)
}

fn depends_on_height(style: &raikiri_layout_style::LayoutStyleView<'_>) -> bool {
    let inset = style.inset();
    HeightFollowers::needs_reflow(style) || !inset.bottom.is_auto() || follows_height(inset.top)
}

fn supports_owner(document: &super::super::context::LayoutSession<'_>, owner: usize) -> bool {
    !document.context.pagination_on_transposed_axes
        && document.nodes.initial_containing_block_key() != Some(owner)
        && is_plain_block_box(document, owner)
}

pub(super) fn follows_ancestor_height(
    document: &super::super::context::LayoutSession<'_>,
    root: usize,
    node: usize,
) -> bool {
    document
        .oof_coordinates
        .get(&node)
        .is_some_and(|coordinate| {
            coordinate.owner != root
                && coordinate.placement.is_some()
                && supports_owner(document, coordinate.owner)
                && is_descendant_or_self(document, root, coordinate.owner)
                && depends_on_height(&document.physical_style(node))
        })
}

pub(super) fn owner_heights(
    document: &super::super::context::LayoutSession<'_>,
) -> HashMap<usize, f32> {
    if document.context.pagination_on_transposed_axes {
        return HashMap::new();
    }
    document
        .oof_coordinates
        .iter()
        .filter(|&(&node, coordinate)| {
            document.nodes[node].position() == &PositionValue::Absolute
                && coordinate.placement.is_some()
                && supports_owner(document, coordinate.owner)
                && depends_on_height(&document.physical_style(node))
        })
        .map(|(_, coordinate)| {
            (
                coordinate.owner,
                padding_box_height(document, coordinate.owner),
            )
        })
        .collect()
}

pub(super) fn changed_owners(
    document: &Document,
    heights: &HashMap<usize, f32>,
    edge_growth: &HashMap<usize, f32>,
) -> HashMap<usize, f32> {
    heights
        .iter()
        .filter_map(|(&owner, &height)| {
            let current = (padding_box_height(document, owner)
                - edge_growth.get(&owner).copied().unwrap_or(0.0))
            .max(0.0);
            ((current - height).abs() > 0.001).then_some((owner, current))
        })
        .collect()
}

pub(super) struct HeightFollowers {
    owners: HashMap<usize, Vec<usize>>,
    static_owners: HashMap<usize, Vec<usize>>,
    source_order: Vec<usize>,
    dirty: std::cell::RefCell<std::collections::BTreeMap<usize, std::collections::BTreeSet<usize>>>,
    reflowed: std::cell::RefCell<HashSet<usize>>,
}

impl HeightFollowers {
    pub(super) fn new(document: &Document) -> Self {
        // Containing-block ownership need not match the DOM parent.
        let mut owners: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut static_owners: HashMap<usize, Vec<usize>> = HashMap::new();
        for (&node, coordinate) in &document.oof_coordinates {
            if document.nodes[node].position() == &PositionValue::Absolute {
                owners.entry(coordinate.owner).or_default().push(node);
                if coordinate.follow_static_y {
                    static_owners
                        .entry(coordinate.static_owner)
                        .or_default()
                        .push(node);
                }
            }
        }
        let mut source_order = vec![0; document.nodes.len()];
        let mut pending = vec![0];
        let mut order = 0;
        while let Some(node) = pending.pop() {
            source_order[node] = order;
            order += 1;
            pending.extend(document.nodes[node].children.iter().rev().copied());
        }
        Self {
            owners,
            static_owners,
            source_order,
            dirty: Default::default(),
            reflowed: Default::default(),
        }
    }

    fn can_reflow(document: &Document, node: usize) -> bool {
        document
            .oof_coordinates
            .get(&node)
            .is_some_and(|coordinate| coordinate.placement.is_some())
    }

    fn needs_reflow(style: &raikiri_layout_style::LayoutStyleView<'_>) -> bool {
        let inset = style.inset();
        follows_height(style.size().height)
            || follows_height(style.min_size().height)
            || follows_height(style.max_size().height)
            || !inset.top.is_auto() && !inset.bottom.is_auto()
    }

    fn follows_static_line(document: &Document, node: usize) -> bool {
        is_unfixed_absolute_box(document, node)
            && document
                .oof_coordinates
                .get(&node)
                .is_some_and(|coordinate| coordinate.follow_static_y)
    }

    pub(super) fn was_reflowed(&self, document: &Document, node: usize) -> bool {
        let reflowed = self.reflowed.borrow();
        document
            .inclusive_ancestors(node)
            .any(|id| reflowed.contains(&id))
    }

    pub(super) fn flush(
        &self,
        document: &mut super::super::context::LayoutSession<'_>,
        control: &PageLayoutControl<'_>,
        outer_heights: Option<&HashMap<usize, f32>>,
    ) -> Result<(), Halt> {
        let mut dirty: Vec<_> = std::mem::take(&mut *self.dirty.borrow_mut())
            .into_iter()
            .collect();
        if dirty.is_empty() {
            return Ok(());
        }
        // Cached in-flow extents describe the pre-reflow subtrees. Materialize
        // deferred moves before the positioning pass replaces their layouts.
        document.finish_normal_block_shifts();
        dirty.sort_by_cached_key(|(parent, _)| {
            (
                document.inclusive_ancestors(*parent).count(),
                self.source_order[*parent],
            )
        });
        for (parent, nodes) in dirty {
            control.check_aborted().map_err(Halt::Failed)?;
            let mut groups = std::collections::BTreeMap::new();
            for node in nodes {
                let style = document.physical_style(node);
                let inset = style.inset();
                let placement = document
                    .oof_coordinates
                    .get(&node)
                    .and_then(|coordinate| coordinate.placement)
                    .expect("only captured positioned children are queued for reflow");
                let offset = document
                    .layout_offset_of(node)
                    .expect("a positioned child has a layout origin");
                let mut candidate = placement.candidate;
                if inset.left.is_auto() && inset.right.is_auto() {
                    let delta = offset.x - placement.offset.x;
                    candidate.static_position.x.area.start += delta;
                    candidate.static_position.x.area.end += delta;
                }
                if inset.top.is_auto() && inset.bottom.is_auto() {
                    let delta = offset.y - placement.offset.y;
                    candidate.static_position.y.area.start += delta;
                    candidate.static_position.y.area.end += delta;
                }
                let mut current_area = placement.area;
                current_area.size.height = outer_heights
                    .and_then(|heights| heights.get(&parent).copied())
                    .unwrap_or_else(|| padding_box_height(document, parent));
                let key = (
                    current_area.offset.x.to_bits(),
                    current_area.offset.y.to_bits(),
                    current_area.size.width.to_bits(),
                    current_area.size.height.to_bits(),
                );
                let (_, candidates) = groups.entry(key).or_insert_with(|| {
                    (current_area, super::super::protocol::OofCandidates::new())
                });
                candidates.push(candidate);
                self.reflowed.borrow_mut().insert(node);
            }
            for (_, (area, candidates)) in groups {
                super::super::dispatch::refresh_container_candidates(
                    document,
                    super::super::protocol::NodeId::from(parent),
                    &candidates,
                    area,
                );
                take_layout_failure(document).map_err(Halt::Failed)?;
            }
        }
        Ok(())
    }

    pub(super) fn queue_resize(
        &self,
        document: &super::super::context::LayoutSession<'_>,
        parent: usize,
    ) {
        if let Some(nodes) = self.owners.get(&parent) {
            for &node in nodes {
                if Self::can_reflow(document, node)
                    && Self::needs_reflow(&document.physical_style(node))
                {
                    self.dirty
                        .borrow_mut()
                        .entry(parent)
                        .or_default()
                        .insert(node);
                }
            }
        }
    }

    pub(super) fn queue_outer_resize(
        &self,
        document: &super::super::context::LayoutSession<'_>,
        parent: usize,
    ) {
        for &node in self.owners.get(&parent).into_iter().flatten() {
            if Self::can_reflow(document, node) && depends_on_height(&document.physical_style(node))
            {
                self.dirty
                    .borrow_mut()
                    .entry(parent)
                    .or_default()
                    .insert(node);
            }
        }
    }

    fn follow_static_after_block(
        &self,
        document: &mut Document,
        parent: usize,
        child: usize,
        delta: f32,
    ) {
        if document
            .ifc_layout_node(parent)
            .and_then(|node| node.ifc_lines())
            .is_some()
        {
            return;
        }
        if let Some(nodes) = self.static_owners.get(&parent) {
            for &node in nodes {
                if self.source_order[node] > self.source_order[child] {
                    document.translate_pagination_y(node, delta);
                }
            }
        }
    }

    fn resize(
        &self,
        document: &mut super::super::context::LayoutSession<'_>,
        parent: usize,
        old_height: f32,
        growth: f32,
    ) {
        if growth.abs() <= 0.001 {
            return;
        }
        let Some(nodes) = self.owners.get(&parent) else {
            return;
        };
        for &node in nodes {
            let style = document.physical_style(node);
            let inset = style.inset();
            if Self::needs_reflow(&style) {
                if Self::can_reflow(document, node) {
                    self.dirty
                        .borrow_mut()
                        .entry(parent)
                        .or_default()
                        .insert(node);
                }
                continue;
            }
            if inset.top.is_auto() && inset.bottom.is_auto() {
                continue;
            }
            let delta = with_height_delta(inset.top, inset.bottom, old_height, growth);
            if delta.abs() > 0.001 {
                document.translate_pagination_y(node, delta);
            }
        }
    }
}

pub(super) fn keep(
    document: &mut super::super::context::LayoutSession<'_>,
    env: &FlowEnv<'_>,
    owner: usize,
    reach: &mut PageReach,
    followers: &HeightFollowers,
) -> Result<(), Halt> {
    // A size-contained absolute box stays at its specified position and
    // fragments as one box, without repairing its internal flow.
    if document.nodes[owner].size_containment.height {
        return Ok(());
    }
    let mut pending = vec![Event::Box(owner)];
    while let Some(event) = pending.pop() {
        match event {
            Event::Box(node) => {
                if document.nodes[node].is_display_none() {
                    continue;
                }
                if document.nodes[node].generates_no_box() {
                    pending.extend(
                        document.nodes[node]
                            .children
                            .iter()
                            .rev()
                            .copied()
                            .map(Event::Box),
                    );
                    continue;
                }
                if node != owner
                    && (matches!(
                        document.nodes[node].position(),
                        PositionValue::Absolute | PositionValue::Fixed
                    ) || document.nodes[node].display().is_atomic_inline_level()
                        || is_floating_box_for_pagination(document, env.cascade, node))
                {
                    continue;
                }
                if node != owner && document.nodes[node].size_containment.height {
                    keep_whole_box(document, env, owner, node, reach, followers)?;
                    continue;
                }
                if document.nodes[node].used_writing_mode.is_vertical()
                    || document.nodes[node].multicol.is_some()
                    || matches!(
                        document.nodes[node].formatting_context,
                        FormattingContext::Flex
                            | FormattingContext::Grid
                            | FormattingContext::Table
                            | FormattingContext::Multicol
                    )
                {
                    // These contexts place their children in their own flows,
                    // except a flex or grid container whose only item grows it
                    // as a block grows with its child.
                    if let Some(item) = sole_item(document, node) {
                        pending.push(Event::Box(item));
                    }
                    continue;
                }
                let ifc = document
                    .ifc_layout_node(node)
                    .and_then(|node| node.ifc.as_ref());
                if let Some(lines) = ifc.and_then(|ifc| ifc.lines.as_ref()) {
                    // Revisit a suffix only after the block before it settles.
                    let mut events = Vec::new();
                    let mut start = 0;
                    for &(block, after) in &lines.block_line_starts {
                        if after > start {
                            events.push(Event::Lines(node, start..after));
                        }
                        events.push(Event::Box(block));
                        start = after;
                    }
                    if start < lines.lines.len() {
                        events.push(Event::Lines(node, start..lines.lines.len()));
                    }
                    pending.extend(events.into_iter().rev());
                } else {
                    pending.extend(
                        document.nodes[node]
                            .children
                            .iter()
                            .rev()
                            .copied()
                            .map(Event::Box),
                    );
                }
            }
            Event::Lines(root, run) => {
                keep_run(document, env, owner, root, run, reach, followers)?;
            }
        }
    }
    Ok(())
}

/// Whether nothing in the flow of the absolute box `owner` comes before its
/// descendant `root`: each box from `root` up to `owner` is the first in-flow
/// box of its parent that takes up room, has a margin or holds text, and no
/// flex or grid container on the way has padding or border above its items.
fn begins_owner_content(document: &Document, owner: usize, root: usize) -> bool {
    let mut child = root;
    document
        .inclusive_ancestors(root)
        .skip(1)
        .find_map(|parent| {
            let node = &document.nodes[parent];
            // A flex or grid container can break between its own padding and
            // border and its items, unlike a block, whose edges go with its
            // first child.
            let content_before = (node.display().is_flex_or_grid_container()
                && node.layout.border.top + node.layout.padding.top > 0.0)
                || node
                    .children
                    .iter()
                    .take_while(|&&sibling| sibling != child)
                    .any(|&sibling| {
                        sibling_renders(document, sibling)
                            && !matches!(
                                document.nodes[sibling].position(),
                                PositionValue::Absolute | PositionValue::Fixed
                            )
                            && (document.nodes[sibling].kind() != NodeKind::Element || {
                                let layout = document.nodes[sibling].layout;
                                layout.size.height > 0.0
                                    || layout.margin.top != 0.0
                                    || layout.margin.bottom != 0.0
                            })
                    });
            child = parent;
            (content_before || parent == owner).then_some(!content_before)
        })
        .unwrap_or(false)
}

/// The only in-flow item of the flex or grid container `node`, when nothing
/// sits beside or after it (see [`has_no_other_item`]).
fn sole_item(document: &super::super::context::LayoutSession<'_>, node: usize) -> Option<usize> {
    let display = document.nodes[node].display();
    let horizontal_container = display.is_flex_or_grid_container()
        && !document.nodes[node].used_writing_mode.is_vertical();
    document.nodes[node]
        .children
        .iter()
        .copied()
        .filter(|_| horizontal_container)
        .find(|&child| {
            // A `display: contents` child may hold several items.
            document.nodes[child].display() != DisplayValue::Contents
                && if display.is_flex_container() {
                    is_in_flow_flex_child_for_pagination(document, node, child)
                } else {
                    is_in_flow_grid_item_for_pagination(document, child)
                }
        })
        .filter(|&item| has_no_other_item(document, node, item))
}

fn keep_run(
    document: &mut super::super::context::LayoutSession<'_>,
    env: &FlowEnv<'_>,
    owner: usize,
    root: usize,
    run: Range<usize>,
    reach: &mut PageReach,
    followers: &HeightFollowers,
) -> Result<(), Halt> {
    let node = document
        .ifc_layout_node(root)
        .expect("a line event has an inline layout root");
    let ifc = node
        .ifc
        .as_ref()
        .expect("a line event retains its inline layout");
    if ifc.multicol_fragments.is_some()
        || ifc.writing_mode != shodo::geometry::WritingMode::HorizontalTb
        || !box_leaves_lines_to_pagination(document, env.cascade, root)
        || ifc.boxes.iter().any(|item| {
            item.kind == IfcBoxKind::OutOfFlow
                && out_of_flow_follow(document, env.cascade, root, item.node).is_none()
                && !HeightFollowers::can_reflow(document, item.node)
        })
    {
        return Ok(());
    }
    let owned = IfcTextLines {
        root,
        width: node.layout.size.width,
        lines: run
            .clone()
            .map(|line| IfcTextLine {
                line,
                top: 0.0,
                bottom: 0.0,
            })
            .collect(),
    };
    let mut metrics = positioned_owned_text_line_bounds(document, &owned)
        .expect("a line event retains its line records");
    let relative = relative_offset_below_owner(document, env, owner, root);
    let start =
        current_abs_y(document, root) - relative + node.layout.padding.top + node.layout.border.top;
    let clip_end = visible_end(document, env, root) - relative;
    metrics.truncate(metrics.partition_point(|&(top, _)| start + top < clip_end - 0.001));
    for (_, bottom) in &mut metrics {
        *bottom = bottom.min(clip_end - start);
    }
    if metrics.is_empty() {
        return Ok(());
    }
    let moves = if root == owner {
        positioned_line_moves(&metrics, start, env.page_origins)
    } else {
        let mut page = env
            .page_origins
            .page_index_for_y(start + metrics[0].0 + 0.001);
        let mut fit = ParagraphLineFit::measure(
            &metrics,
            start,
            env.page_origins.origin(page),
            env.page_origins.page_end(page),
            env.cascade.computed[root].orphans,
            env.cascade.computed[root].widows,
            0.0,
        );
        let mut moves = Vec::new();
        let first_top = start + metrics[0].0;
        let next = page.saturating_add(1);
        let avoids =
            box_avoids_page_break_inside(document, root, env.cascade.computed[root].break_inside);
        let whole_fits = metrics.last().expect("a visible run is nonempty").1 - metrics[0].0
            <= env.page_origins.step_at(next) + 0.001;
        // A break before a paragraph that begins the content of the box would
        // fall before the box itself, where its fragment starts: a first line
        // that fits stays, and the box breaks between its lines.
        let prefix = if first_top > env.page_origins.origin(page) + 0.001
            && (fit.leaves_too_few_orphans()
                || avoids && whole_fits && fit.lines_before_break < metrics.len())
            && metrics[0].1 - metrics[0].0 <= env.page_origins.step_at(next) + 0.001
            && (fit.lines_before_break == 0 || !begins_owner_content(document, owner, root))
        {
            // The ordinary paragraph splitter assumes its first line fits.
            page = next;
            let shift = env.page_origins.origin(page) - first_top;
            moves.push((0, shift, page));
            shift
        } else {
            0.0
        };
        fit = ParagraphLineFit::measure(
            &metrics,
            start + prefix,
            env.page_origins.origin(page),
            env.page_origins.page_end(page),
            env.cascade.computed[root].orphans,
            env.cascade.computed[root].widows,
            0.0,
        );
        moves.extend(paragraph_line_moves(
            &metrics,
            start + prefix,
            page,
            &fit,
            env.page_origins,
        ));
        moves
    };
    for (line, shift, page) in moves {
        // Earlier moves can grow an auto clip, while a fixed clip stays put.
        if env.page_origins.origin(page) >= visible_end(document, env, root) - relative - 0.001 {
            break;
        }
        reach_page(env.control, page)?;
        reach.include_checked(env.control, page, false)?;
        let line = run.start + line;
        let following: Vec<_> = document
            .ifc_layout_node(root)
            .and_then(|node| node.ifc.as_ref())
            .into_iter()
            .flat_map(|ifc| &ifc.boxes)
            .filter(|b| {
                matches!(b.kind, IfcBoxKind::Block | IfcBoxKind::Atomic)
                    && follows_line_of_paragraph(document, b.node, root, line)
            })
            .map(|b| b.node)
            .collect();
        let float_before = exported_float_end(document, env.cascade, owner, root);
        let layout = document.nodes[root].layout;
        let growth = lines_shift_growth(document, root, shift);
        // Static anchors follow the line; all height-based insets are updated
        // once through the containing-block index, including deeper children.
        let ifc = document
            .ifc_layout_node(root)
            .and_then(|node| node.ifc.as_ref());
        let anchors = std::cell::OnceCell::new();
        let on_lines: HashSet<_> = ifc
            .into_iter()
            .flat_map(|ifc| &ifc.boxes)
            .filter(|item| item.kind == IfcBoxKind::OutOfFlow)
            .filter(|item| HeightFollowers::follows_static_line(document, item.node))
            .filter(|item| {
                anchors
                    .get_or_init(|| {
                        ifc.and_then(|ifc| ifc.lines.as_ref())
                            .map(first_anchors)
                            .unwrap_or_default()
                    })
                    .get(&item.node)
                    .is_some_and(|&(anchor, _)| anchor >= line)
            })
            .map(|item| item.node)
            .collect();
        shift_ifc_lines_from(document, root, line, shift, growth);
        for node in on_lines {
            document.translate_pagination_y(node, shift);
        }
        followers.resize(
            document,
            root,
            layout.size.height - layout.border.top - layout.border.bottom,
            growth,
        );
        for node in following {
            document.translate_pagination_y(node, shift);
        }
        // A following block can carry floats that extend past the run's box.
        let float_after = exported_float_end(document, env.cascade, owner, root);
        let float_delta = if float_before.is_finite() && float_after.is_finite() {
            float_after - float_before
        } else {
            0.0
        };
        grow_inside(
            document,
            env.cascade,
            owner,
            root,
            ContentChange {
                end_delta: growth,
                float_delta,
                position_delta: 0.0,
            },
            followers,
        );
    }
    Ok(())
}

pub(super) fn visible_end(
    document: &super::super::context::LayoutSession<'_>,
    env: &FlowEnv<'_>,
    root: usize,
) -> f32 {
    clip_bounds(document, env, root).end
}

/// The clips around the lines of `root`, from the flow root down.
pub(super) fn clip_bounds(
    document: &super::super::context::LayoutSession<'_>,
    env: &FlowEnv<'_>,
    root: usize,
) -> candidates::ClipBounds {
    let mut ancestors = Vec::new();
    for id in document.inclusive_ancestors(root) {
        ancestors.push(id);
        if id == env.roots.element {
            break;
        }
    }
    let mut clip = candidates::ClipBounds::unclipped();
    for id in ancestors.into_iter().rev() {
        let node = &document.nodes[id];
        if node.kind() != NodeKind::Element || node.in_ifc_subtree() {
            continue;
        }
        clip = clip.descend(
            document,
            env.cascade,
            id,
            current_abs_y(document, id),
            id == env.roots.contents || id == env.roots.element,
        );
    }
    clip
}

// Relative offsets below the absolute owner paint after fragmentation. The
// owner's positioned anchor and its ancestors remain in the page coordinate.
fn relative_offset_below_owner(
    document: &Document,
    env: &FlowEnv<'_>,
    owner: usize,
    node: usize,
) -> f32 {
    document
        .inclusive_ancestors(node)
        .take_while(|&id| id != owner)
        .map(|id| layout_relative_offset_y(document, env.cascade, env.body_id, id))
        .sum()
}

fn exported_float_end(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    owner: usize,
    root: usize,
) -> f32 {
    let mut end = f32::NEG_INFINITY;
    visit_same_bfc_floats_in_subtree(
        document,
        cascade,
        owner,
        root,
        current_abs_y(document, root),
        |_, bottom| end = end.max(bottom),
    );
    end
}

fn keep_whole_box(
    document: &mut super::super::context::LayoutSession<'_>,
    env: &FlowEnv<'_>,
    owner: usize,
    node: usize,
    reach: &mut PageReach,
    followers: &HeightFollowers,
) -> Result<(), Halt> {
    let relative = relative_offset_below_owner(document, env, owner, node);
    let top = current_abs_y(document, node) - relative;
    let page = env.page_origins.page_index_for_y(top + 0.001);
    if top > env.page_origins.origin(page) + 0.001
        && (top + document.nodes[node].layout.size.height)
            .min(visible_end(document, env, node) - relative)
            > env.page_origins.page_end(page) + 0.001
    {
        let target = page.saturating_add(1);
        reach_page(env.control, target)?;
        reach.include_checked(env.control, target, false)?;
        let shift = env.page_origins.origin(target) - top;
        document.translate_pagination_y(node, shift);
        grow_inside(
            document,
            env.cascade,
            owner,
            node,
            ContentChange {
                end_delta: shift,
                float_delta: 0.0,
                position_delta: shift,
            },
            followers,
        );
    }
    Ok(())
}

fn grow_inside(
    document: &mut super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    owner: usize,
    mut child: usize,
    change: ContentChange,
    followers: &HeightFollowers,
) {
    let mut flow_delta = change.end_delta;
    let mut size_delta = change.end_delta;
    let mut float_delta = change.float_delta;
    let mut position_delta = change.position_delta;
    let mut walk = AncestorCursor::new(document.layout_parent_of(child), ParentRelation::Layout);
    while child != owner
        && let Some(parent) = walk.current()
    {
        walk.advance(document);
        if document.nodes[parent].size_containment.height {
            break; // cov:ignore: the event walker moves size-contained children whole without entering them and rejects a size-contained owner.
        }
        if document.nodes[parent].display() == DisplayValue::Contents {
            continue;
        }
        let height = document.nodes[parent].layout.size.height;
        let child_height = document.nodes[child].layout.size.height;
        let top =
            pagination_child_parent_y(document, parent, child, document.pagination_local_y(child));
        let allowed_oof = document
            .ifc_layout_node(parent)
            .and_then(|node| node.ifc.as_ref())
            .into_iter()
            .flat_map(|ifc| &ifc.boxes)
            .filter(|item| {
                item.kind == IfcBoxKind::OutOfFlow
                    && HeightFollowers::follows_static_line(document, item.node)
            })
            .map(|item| item.node)
            .collect();
        let followed = follow_ifc_block_with_oof(
            document,
            parent,
            child,
            top - position_delta,
            top + child_height - size_delta,
            flow_delta,
            Some(&allowed_oof),
        );
        shift_later_block_siblings(document, parent, child, flow_delta);
        followers.follow_static_after_block(document, parent, child, flow_delta);
        normal_block_shifts::refresh(document, cascade, owner, parent);
        let Some(grown) = grown_auto_height(
            document,
            cascade,
            parent,
            child,
            flow_delta,
            size_delta,
            float_delta,
            true,
            owner,
            &followed,
        ) else {
            break; // cov:ignore: this walker supplies an in-flow box; Contents and non-offset inline wrappers are unwrapped, so ancestor enumeration visits the moved child.
        };
        document.nodes[parent].layout.size.height = grown.height;
        document.mark_pagination_extent_dirty(parent);
        let border = document.nodes[parent].layout.border;
        followers.resize(
            document,
            parent,
            height - border.top - border.bottom,
            grown.height - height,
        );
        float_delta = if holds_floats(document, cascade, parent) {
            0.0
        } else {
            grown.float_extent_delta
        };
        size_delta = grown.height - height;
        flow_delta = grown.flow_heights.1 - grown.flow_heights.0;
        // Only the initial whole box moved; its ancestors grew in place.
        position_delta = 0.0;
        child = parent;
        if flow_delta.abs() <= 0.001 && size_delta.abs() <= 0.001 && float_delta.abs() <= 0.001 {
            break;
        }
    }
}
