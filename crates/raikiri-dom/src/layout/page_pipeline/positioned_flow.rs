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

pub(super) struct HeightFollowers(HashMap<usize, Vec<usize>>);

impl HeightFollowers {
    pub(super) fn new(document: &Document) -> Self {
        // Containing-block ownership need not match the DOM parent.
        let mut owners: HashMap<usize, Vec<usize>> = HashMap::new();
        for (&node, coordinate) in &document.oof_coordinates {
            if document.nodes[node].position() == &PositionValue::Absolute {
                owners.entry(coordinate.owner).or_default().push(node);
            }
        }
        Self(owners)
    }

    fn resize(
        &self,
        document: &mut super::super::context::LayoutSession<'_>,
        parent: usize,
        old_height: f32,
        growth: f32,
    ) {
        let Some(nodes) = self.0.get(&parent) else {
            return;
        };
        for &node in nodes {
            let style = document.physical_style(node);
            let inset = style.inset();
            if inset.top.is_auto() == inset.bottom.is_auto()
                || follows_height(style.size().height)
                || follows_height(style.min_size().height)
                || follows_height(style.max_size().height)
            {
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
                if node != owner
                    && (matches!(
                        document.nodes[node].position(),
                        PositionValue::Absolute | PositionValue::Fixed
                    ) || document.nodes[node].display().is_atomic_inline_level()
                        || is_floating_box_for_pagination(document, env.cascade, node))
                {
                    continue;
                }
                if document.nodes[node].is_display_none() {
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
                    // These contexts place their children in their own flows.
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
    let start = current_abs_y(document, root) + node.layout.padding.top + node.layout.border.top;
    let clip_end = visible_end(document, env.cascade, root);
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
        let prefix = if first_top > env.page_origins.origin(page) + 0.001
            && (fit.leaves_too_few_orphans()
                || avoids && whole_fits && fit.lines_before_break < metrics.len())
            && metrics[0].1 - metrics[0].0 <= env.page_origins.step_at(next) + 0.001
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
        if env.page_origins.origin(page) >= visible_end(document, env.cascade, root) - 0.001 {
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
        let moved = LineMove {
            block_id: root,
            flex_line: None,
            line,
            shift,
        };
        // Static anchors follow the line; all height-based insets are updated
        // once through the containing-block index, including deeper children.
        let on_lines: Vec<_> = out_of_flow_moves(document, env.cascade, &moved, growth)
            .into_iter()
            .filter(|&(node, _)| {
                out_of_flow_follow(document, env.cascade, root, node)
                    == Some(OutOfFlowFollow::OnLine)
            })
            .collect();
        shift_ifc_lines_from(document, root, line, shift, growth);
        for (node, delta) in on_lines {
            document.translate_pagination_y(node, delta);
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

fn visible_end(document: &Document, cascade: &CascadeResult, root: usize) -> f32 {
    document
        .inclusive_ancestors(root)
        .filter(|&id| element_clips_overflow(document, cascade, id))
        .map(|id| clip_block_end(document, cascade, id, current_abs_y(document, id)))
        .fold(f32::INFINITY, f32::min)
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
    let top = current_abs_y(document, node);
    let page = env.page_origins.page_index_for_y(top + 0.001);
    if top > env.page_origins.origin(page) + 0.001
        && (top + document.nodes[node].layout.size.height).min(visible_end(
            document,
            env.cascade,
            node,
        )) > env.page_origins.page_end(page) + 0.001
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
                    && out_of_flow_follow(document, cascade, parent, item.node)
                        == Some(OutOfFlowFollow::OnLine)
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
