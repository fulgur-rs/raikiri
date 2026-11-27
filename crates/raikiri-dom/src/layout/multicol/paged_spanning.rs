use super::*;
use crate::document::ancestors::{AncestorCursor, ParentRelation};
use crate::fragment::{FragmentRect, LayoutFragment, MulticolGroup};
use lineless::EPSILON;
use std::collections::HashSet;

/// The origin of the container's space, with no size.
const ORIGIN: FragmentRect = FragmentRect {
    x: 0.0,
    y: 0.0,
    width: 0.0,
    height: 0.0,
};

/// Whether `owner`, or a box around it, is an atomic inline such as an
/// `inline-block`, whose contents the pages cut as one tall box.
fn inside_atomic_inline(tree: &LayoutSession<'_>, owner: usize) -> bool {
    let mut walk = AncestorCursor::new(Some(owner), ParentRelation::Layout);
    while let Some(id) = walk.current() {
        if tree.nodes[id].display().is_atomic_inline_level() {
            return true;
        }
        walk.advance(tree);
    }
    false
}

/// Drop the groups kept for containers without a spanner, when the columns of
/// the document are not broken along the page axis.
pub(in crate::layout) fn release_unspanned_groups(tree: &mut LayoutSession<'_>) {
    for owner in 0..tree.nodes.len() {
        if tree.nodes[owner].multicol_groups.is_empty() {
            continue;
        }
        let children = flow_children(tree, owner);
        if !children.iter().any(|&id| spanning::is_spanner(tree, id)) {
            tree.nodes[owner].multicol_groups.clear();
        }
    }
}

// The page pipeline supplies the owner's final source position and page
// schedule. Shaped paragraph lines remain authoritative; only their column
// placement changes when a group resumes in a later page fragmentainer.
//
// Fragment records are not edited in the tree: `edits` collects the changes
// of every owner, and the caller installs them once.
pub(in crate::layout) fn paginate_spanning_columns(
    tree: &mut LayoutSession<'_>,
    edits: &mut FragmentEdits,
    owner: usize,
    owner_y: f32,
    page_for: &impl Fn(f32) -> (u32, f32, f32),
    work: &mut ProjectionWork<'_, '_>,
) -> Result<f32, LayoutError> {
    let old_groups = tree.nodes[owner].multicol_groups.clone();
    // A multicol container is not broken when pagination runs on transposed
    // axes: its columns lay out on the horizontal axes of the page. The groups
    // of a container that is a paragraph of its own are kept for the rules
    // between its columns. Otherwise such a container fills its columns page
    // by page from its own lines.
    if tree.context.pagination_on_transposed_axes {
        return Ok(0.0);
    }
    if tree.nodes[owner].is_ifc_root() {
        return paginate_own_lines(tree, edits, owner, owner_y, page_for, work);
    }
    let children = flow_children(tree, owner);
    work.charge(children.len())?;
    let spanner_free = !children.iter().any(|&id| spanning::is_spanner(tree, id));
    // A container of spanners alone keeps no groups, yet breaks between them.
    if old_groups.is_empty()
        && (spanner_free
            || children.iter().any(|&id| {
                !spanning::is_spanner(tree, id) && spanning::holds_column_content(tree, id)
            }))
    {
        return Ok(0.0);
    }
    // The columns of a container without a spanner are filled page by page
    // only where it runs onto a later page. One inside a line stays whole, as
    // the cut of its page is not the cut of a box in the flow.
    // A container that clones its decorations repeats its block-axis border
    // and padding on each page it is cut on (CSS Fragmentation 3, 5.4), so
    // its content fits a page only above the bottom ones.
    let cloned = cloned_page_insets(tree, owner);
    // A container of a definite height without a spanner fills the columns
    // of the pages it runs onto up to its content box (CSS Multi-column 1,
    // 8.1). Its height stays as it is.
    let definite = tree.nodes[owner]
        .multicol
        .is_some_and(|columns| columns.height_definite);
    // Forced breaks that start more columns than a container of `auto`
    // height has go on in the columns of the next page, as Chromium prints
    // them, even when the content would fit this one.
    let whole = spanner_free
        && (inside_atomic_inline(tree, owner) || {
            let old = &old_groups[0];
            let top = owner_y + old.context.origin_y;
            let (_, origin, height) = page_for(top);
            let bottom = cloned.map_or(0.0, |(_, bottom)| bottom);
            top + old.height <= origin + height - bottom + EPSILON
                && (definite
                    || lineless::forced_column_starts(tree, owner) < old.context.column_count)
        });
    if whole || definite && (!spanner_free || cloned.is_some()) {
        if spanner_free {
            tree.nodes[owner].multicol_groups.clear();
        }
        return Ok(0.0);
    }
    // The columns of a later page start below the repeated top edges, and
    // those of every page end above the bottom ones. The first page has the
    // top ones above where the content starts already. Spanners keep the
    // pages as they are.
    let clone_page_for = cloned.map(|(top, bottom)| {
        move |y: f32| {
            let (mut page, mut origin, mut height) = page_for(y);
            // The columns of a page end above its bottom border and padding,
            // so those that go on from there start on the next page.
            if (y - (origin + height - bottom)).abs() <= EPSILON {
                (page, origin, height) = page_for(origin + height);
            }
            // Content that starts below where the columns of its page end,
            // in the bottom border and padding, keeps the rest of the page.
            // Edges that fill the page leave no room for columns; the page
            // keeps its whole height for them instead.
            let bottom = if y > origin + height - bottom + EPSILON {
                0.0
            } else {
                bottom
            };
            let room = height - top - bottom;
            if room > EPSILON {
                (page, origin + top, room)
            } else {
                (page, origin, height)
            }
        }
    });
    let columns_page_for = |y: f32| match &clone_page_for {
        Some(cloned) => cloned(y),
        None => page_for(y),
    };
    let columns_page_for = &columns_page_for;
    // The boxes of a container of boxes that hold no lines, which the columns
    // fill from their own layout instead of from lines. The boxes that
    // wrappers hold take their place, as in the layout that kept the groups.
    let leaves = spanner_free
        .then(|| lineless::supported_with_wrappers(tree, owner))
        .flatten();
    let Some(container) = edits.first(owner) else {
        return Ok(0.0); // cov:ignore: retained groups are created only after the owner source fragment exists.
    };
    let mut groups = Vec::<MulticolGroup>::new();
    let mut replacements = Vec::<(usize, Vec<LayoutFragment>)>::new();
    let mut wrapped_children = HashSet::new();
    // Fragment shifts are applied in one pass after the walk: scanning every
    // fragment for each shifted child is quadratic in the group's children.
    let mut shifts = HashMap::<usize, f32>::new();
    let mut start = 0;
    let mut group_index = 0;
    let mut delta = 0.0;
    let mut refilled_lineless = false;
    // The last spanner before the current segment, and whether column
    // content or a spanner came before it.
    let mut previous_spanner = None;
    let mut content_before = false;
    for end in 0..=children.len() {
        let spanner = children
            .get(end)
            .copied()
            .filter(|&id| spanning::is_spanner(tree, id));
        if end < children.len() && spanner.is_none() {
            continue;
        }
        let segment = &children[start..end];
        let segment_has_content = segment
            .iter()
            .any(|&id| spanning::holds_column_content(tree, id));
        if segment_has_content {
            // Where the content after a spanner starts before the pages place
            // it, to tell how far they pushed it down.
            let first_content = previous_spanner.and_then(|spanner| {
                let first = segment
                    .iter()
                    .copied()
                    .find(|&id| spanning::holds_column_content(tree, id))?;
                Some((spanner, first, tree.nodes[first].layout.location.y + delta))
            });
            let old = &old_groups[group_index];
            group_index += 1;
            let context = FragmentationContext {
                origin_y: old.context.origin_y + delta,
                ..old.context
            };
            let entries: Vec<_> = segment
                .iter()
                .map(|&id| {
                    let layout = tree.nodes[id].layout;
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
            // The columns of a container of a definite height end with its
            // content box.
            let content_end = definite.then(|| {
                let layout = tree.nodes[owner].layout;
                layout.size.height - layout.border.bottom - layout.padding.bottom
            });
            // The boxes after the last spanner of a container that fills its
            // columns in turn fill those of the pages they reach as the boxes
            // of a container without spanners do; the rows before a spanner,
            // and a last row that balances, keep their balanced columns. A
            // container inside a line stays whole, as one without spanners
            // does.
            let leaves = if spanner.is_none()
                && !spanner_free
                && context.column_fill == ColumnFillValue::Auto
                && !inside_atomic_inline(tree, owner)
            {
                lineless::supported_plain_segment(tree, owner, segment)
            } else {
                leaves.clone()
            };
            if let Some(planned) = leaves
                .as_deref()
                .map(|leaves| {
                    lineless::paginate(
                        tree,
                        leaves,
                        context,
                        owner_y,
                        content_end,
                        columns_page_for,
                        work,
                    )
                })
                .transpose()?
                .flatten()
            {
                if let Some((spanner, first, natural)) = first_content
                    && let Some(top) = planned
                        .pieces
                        .iter()
                        .filter(|piece| piece.child == first)
                        .map(|piece| piece.rect.y)
                        .reduce(f32::min)
                {
                    note_slack(tree, spanner, top - natural);
                }
                groups.extend(planned.groups.into_iter().map(|group| MulticolGroup {
                    run_start: old.run_start,
                    ..group
                }));
                let placed = (planned.pieces, planned.nested);
                refilled_lineless |= install_lineless_pieces(
                    tree,
                    edits,
                    segment,
                    container,
                    placed,
                    delta,
                    &mut shifts,
                );
                delta = planned.end - (old.context.origin_y + old.height);
            } else if let Some(planned) = paragraph_group::paginate(
                tree,
                &entries,
                context,
                owner_y,
                content_end,
                break_before_page(
                    tree,
                    owner,
                    owner_y,
                    &context,
                    previous_spanner.is_some(),
                    page_for,
                ),
                columns_page_for,
                work,
            )? {
                let new_end = planned.end;
                if let Some((spanner, first, natural)) = first_content
                    && let Some(top) = planned
                        .fragments
                        .iter()
                        .filter(|fragment| fragment.child == first && fragment.wrapper.is_none())
                        .map(|fragment| fragment.rect.y)
                        .reduce(f32::min)
                {
                    note_slack(tree, spanner, top - natural);
                }
                groups.extend(planned.groups.into_iter().map(|group| MulticolGroup {
                    run_start: old.run_start,
                    ..group
                }));
                // A wrapper whose boxes joined the flow gets one record from
                // its first piece to its lowest, and the records of what it
                // holds are placed in it. A wrapper that has no record of its
                // own leaves them to the nearest one that has.
                let mut wrappers = HashMap::<usize, (usize, FragmentRect)>::new();
                let rects: HashMap<usize, FragmentRect> = planned
                    .wrappers
                    .iter()
                    .map(|wrapper| (wrapper.node, wrapper.rect))
                    .collect();
                for wrapper in &planned.wrappers {
                    let (record, origin) = wrapper
                        .wrapper
                        .and_then(|outer| wrappers.get(&outer).copied())
                        .unwrap_or((container, ORIGIN));
                    let parent_origin = wrapper
                        .wrapper
                        .and_then(|outer| rects.get(&outer).copied())
                        .unwrap_or(ORIGIN);
                    let node = &mut tree.nodes[wrapper.node];
                    node.layout.location = Point {
                        x: wrapper.rect.x - parent_origin.x,
                        y: wrapper.rect.y - parent_origin.y,
                    };
                    node.layout.size.height = wrapper.rect.height;
                    let rect = FragmentRect {
                        x: wrapper.rect.x - origin.x,
                        y: wrapper.rect.y - origin.y,
                        ..wrapper.rect
                    };
                    let had_records = edits.first(wrapper.node).is_some();
                    edits.replace(
                        wrapper.node,
                        vec![LayoutFragment {
                            node_id: wrapper.node,
                            parent: Some(record),
                            fragmentainer: 0,
                            rect,
                            fragmentainer_clip: None,
                            fragment_index: 0,
                            fragment_count: 1,
                            line_start: None,
                            line_end: None,
                        }],
                    );
                    let entry = edits
                        .first(wrapper.node)
                        .filter(|_| had_records)
                        .map_or((record, origin), |own| (own, wrapper.rect));
                    wrappers.insert(wrapper.node, entry);
                }
                let mut per_child = HashMap::<usize, Vec<paragraph_group::PagedFragment>>::new();
                let mut wrapped = Vec::new();
                for fragment in planned.fragments {
                    if fragment.wrapper.is_some() && !per_child.contains_key(&fragment.child) {
                        wrapped.push(fragment.child);
                        wrapped_children.insert(fragment.child);
                    }
                    per_child.entry(fragment.child).or_default().push(fragment);
                }
                for &id in segment.iter().chain(&wrapped) {
                    if rects.contains_key(&id) {
                        continue;
                    }
                    let Some(mut fragments) = per_child.remove(&id) else {
                        shift_child(tree, &mut shifts, id, delta);
                        continue;
                    };
                    // The pieces inside a wrapper are placed in its record,
                    // and the box in the wrapper's box.
                    let wrapper = fragments[0].wrapper;
                    let (record, origin) = wrapper
                        .and_then(|wrapper| wrappers.get(&wrapper).copied())
                        .unwrap_or((container, ORIGIN));
                    let parent_origin = wrapper
                        .and_then(|wrapper| rects.get(&wrapper).copied())
                        .unwrap_or(ORIGIN);
                    let first = fragments[0].rect;
                    let node = &mut tree.nodes[id];
                    node.layout.location = Point {
                        x: first.x - parent_origin.x,
                        y: first.y - parent_origin.y,
                    };
                    node.layout.size = Size {
                        width: first.width,
                        height: first.height,
                    };
                    if let Some(root) = node.ifc.as_mut() {
                        root.multicol_fragments = Some(
                            fragments
                                .iter()
                                .map(|f| MulticolTextFragment {
                                    line_start: f.line_start,
                                    line_end: f.line_end,
                                    fragmentainer: f.fragmentainer,
                                    x: f.rect.x - first.x,
                                    y: f.rect.y - first.y,
                                })
                                .collect(),
                        );
                        root.multicol_fragment_origins_recorded = true;
                    } // cov:ignore: planned text ranges only originate from a paragraph with an IFC.
                    for fragment in &mut fragments {
                        fragment.rect.x -= origin.x;
                        fragment.rect.y -= origin.y;
                        if let Some(clip) = &mut fragment.clip {
                            clip.rect.x -= origin.x;
                            clip.rect.y -= origin.y;
                        }
                    }
                    replacements.push((
                        id,
                        fragments
                            .into_iter()
                            .map(|f| LayoutFragment {
                                node_id: id,
                                parent: Some(record),
                                fragmentainer: f.fragmentainer,
                                rect: f.rect,
                                fragmentainer_clip: f.clip,
                                fragment_index: 0,
                                fragment_count: 1,
                                line_start: Some(f.line_start),
                                line_end: Some(f.line_end),
                            })
                            .collect(),
                    ));
                }
                delta = new_end - (old.context.origin_y + old.height);
            } else {
                groups.push(MulticolGroup {
                    context,
                    ..old.clone()
                });
                for &id in segment {
                    shift_child(tree, &mut shifts, id, delta);
                }
            }
        }
        if let Some(id) = spanner {
            tree.context.planned_spanners.insert(id);
            shift_child(tree, &mut shifts, id, delta);
            let layout = tree.nodes[id].layout;
            let y = owner_y + layout.location.y;
            let (_, origin, height) = page_for(y);
            // A page value between column content, or another spanner, and
            // this spanner breaks the page there (Chromium): the spanner and
            // all after it start the next page. One at the start of the
            // container is the container's own break, which the flow around
            // it takes.
            let forced = (content_before || segment_has_content)
                && forced_page_break_before_spanner(
                    tree,
                    id,
                    previous_spanner.filter(|_| !segment_has_content),
                );
            content_before = true;
            previous_spanner = Some(id);
            let margin_top = y - layout.margin.top;
            let (_, top_origin, top_height) = page_for(margin_top);
            let top_end = top_origin + top_height;
            if forced && margin_top > top_origin + EPSILON {
                // A forced break keeps the margin above the spanner, unless
                // the margin runs past the end of the page the break opens:
                // it is cut there, and the spanner starts the page after it,
                // as a block does (Chromium).
                let (_, next_origin, next_height) = page_for(top_end + EPSILON);
                let next_end = next_origin + next_height;
                let shift = if y + top_end - margin_top >= next_end - EPSILON {
                    next_end - y
                } else {
                    top_end - margin_top
                };
                shift_child(tree, &mut shifts, id, shift);
                delta += shift;
            } else if layout.margin.top > 0.0
                && (margin_top > top_origin + EPSILON || forced)
                && y > top_end + EPSILON
            {
                // A margin that runs past the end of its page is truncated
                // there: the spanner starts the next page. So is one that a
                // forced break at the top of the page keeps.
                let shift = top_end - y;
                shift_child(tree, &mut shifts, id, shift);
                delta += shift;
            } else if layout.size.height <= height
                && y > origin + 0.001
                && y + layout.size.height > origin + height + 0.001
                && y + first_piece_end(tree, id, work)? > origin + height + EPSILON
            {
                // The page breaks inside the spanner as inside any block of
                // the flow (see the page walk), unless it ends inside the
                // first piece of the spanner, which leaves no place to break
                // before it but the spanner's own top.
                let shift = origin + height - y;
                shift_child(tree, &mut shifts, id, shift);
                delta += shift;
            }
            // The breaks inside the spanner grow it, as an earlier attempt
            // found: the columns after it are planned below what it grows to,
            // and it grows into that space without moving them.
            let growth = tree.context.spanner_growth.get(&id).copied().unwrap_or(0.0);
            if growth > 0.0 {
                delta += growth;
                note_slack(tree, id, growth);
            }
        }
        content_before |= segment_has_content;
        start = end + 1;
    }
    edits.shift_children(container, &shifts);
    let refilled = refilled_lineless || !replacements.is_empty();
    for (id, fragments) in replacements {
        // A box in a wrapper that had no records of its own was drawn
        // through the wrapper's; its pieces now say which column each of
        // its lines is in.
        if wrapped_children.contains(&id) {
            edits.replace_or_add(id, fragments);
        } else {
            edits.replace(id, fragments);
        }
    }
    if spanner_free {
        if refilled {
            tree.context.page_filled_columns.push(owner);
        } else {
            // The columns stay as the source layout made them.
            groups.clear();
        }
    }
    // A container with a minimum height keeps it, below the columns that end
    // higher once the pages fill them (CSS 2, 10.7).
    if definite {
        delta = 0.0;
    } else if spanner_free
        && let Some(old) = old_groups.first()
        && old.min_height > 0.0
    {
        let layout = tree.nodes[owner].layout;
        let old_end =
            old.context.origin_y + old.height + layout.border.bottom + layout.padding.bottom;
        delta = (old_end + delta).max(old.min_height) - old_end.max(old.min_height);
    }
    tree.nodes[owner].multicol_groups = groups;
    tree.nodes[owner].layout.size.height += delta;
    edits.grow(owner, delta);
    Ok(delta)
}

/// Fill the columns of `owner`, a balanced container that is a paragraph of
/// its own with no spanner, page by page where its lines run past the end of a
/// page: each page's columns are as tall as the page leaves, or the content
/// box of a container of a definite height, and only the last page balances
/// the lines left (CSS Multi-column 1, 8.1). The lines are placed again by the
/// column fragments of the paragraph. Returns how much taller the container
/// grows, as [`paginate_spanning_columns`] does.
fn paginate_own_lines(
    tree: &mut LayoutSession<'_>,
    edits: &mut FragmentEdits,
    owner: usize,
    owner_y: f32,
    page_for: &impl Fn(f32) -> (u32, f32, f32),
    work: &mut ProjectionWork<'_, '_>,
) -> Result<f32, LayoutError> {
    let node = &tree.nodes[owner];
    let layout = node.layout;
    let width = layout.size.width
        - layout.border.left
        - layout.border.right
        - layout.padding.left
        - layout.padding.right;
    let Some((style, context, lines, old, boxes)) = node.ifc.as_ref().and_then(|root| {
        let style = node.multicol.filter(|style| style.horizontal)?;
        let context = FragmentationContext::resolve(width, None, style)?;
        Some((
            style,
            context,
            root.lines.as_ref()?,
            root.multicol_fragments.as_ref()?,
            &root.boxes,
        ))
    }) else {
        return Ok(0.0);
    };
    let extents: Vec<(f32, f32)> = lines
        .lines
        .iter()
        .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
        .collect();
    // The atomic inlines of the paragraph stand where its first columns put
    // them, and the columns of a multicol container inside another one are
    // cut by the outer columns, not by the pages.
    if style.column_fill != ColumnFillValue::Balance
        || tree.context.unpaged_plain_columns
        || !lines.block_line_starts.is_empty()
        || !boxes.is_empty()
        || old.is_empty()
        || old.iter().any(|fragment| {
            fragment.line_start >= fragment.line_end || fragment.line_end > extents.len()
        })
        || !node.multicol_groups.is_empty()
        || cloned_page_insets(tree, owner).is_some()
        || inside_atomic_inline(tree, owner)
        || inside_other_multicol(tree, owner)
    {
        return Ok(0.0);
    }
    work.charge(extents.len())?;
    let top_inset = layout.border.top + layout.padding.top;
    let bottom_inset = layout.border.bottom + layout.padding.bottom;
    let count = context.column_count;
    let orphans = context.orphans.max(1);
    let widows = context.widows.max(1);
    // How far the columns reach below the top of the content box now.
    let old_end = old
        .iter()
        .map(|fragment| {
            fragment.y + extents[fragment.line_end - 1].1 - extents[fragment.line_start].0
        })
        .fold(0.0_f32, f32::max);
    let start = owner_y + top_inset;
    let (_, origin, height) = page_for(start);
    if start + old_end <= origin + height + EPSILON {
        return Ok(0.0);
    }
    // The columns of a container of a definite height end with its content
    // box; lines past there overflow them, which the pages do not fill again.
    let definite = style.height_definite;
    let limit = definite.then_some(owner_y + layout.size.height - bottom_inset);
    let mut fragments = Vec::new();
    let mut groups = Vec::new();
    let mut first = 0usize;
    let mut top = start;
    let mut end = 0.0_f32;
    while first < extents.len() {
        let (page, origin, height) = page_for(top);
        top = top.max(origin);
        let page_bottom = origin + height;
        let page_end = limit.map_or(page_bottom, |limit| page_bottom.min(limit));
        let capacity = page_end - top;
        if !work.page_available(page)? || capacity <= EPSILON {
            return Ok(0.0);
        }
        let rest = &extents[first..];
        // The lines left fit this page's columns: they balance in them.
        let fits = (0..count)
            .try_fold(0usize, |at, _| {
                (at < rest.len()).then(|| column_end(rest, at, capacity))
            })
            .is_none_or(|at| at >= rest.len());
        let column_height = if fits {
            balanced_column_height(
                rest,
                FragmentationContext {
                    available_height: Some(capacity),
                    column_index: 0,
                    ..context
                },
            )
            .map_or(capacity, |balanced| balanced.min(capacity))
        } else {
            capacity
        };
        let mut ranges = Vec::with_capacity(count);
        let mut at = first;
        while ranges.len() < count && at < extents.len() {
            let last = column_end(&extents, at, column_height);
            work.charge(1)?;
            ranges.push((at, last));
            at = last;
        }
        // The columns that balance the last lines leave `widows` after each
        // break between them, as the columns of a paragraph on screen do.
        if fits {
            let mut pieces: Vec<_> = ranges.iter().map(|&(start, end)| (start, end, 0)).collect();
            respect_orphans_and_widows(&mut pieces, context);
            ranges = pieces
                .into_iter()
                .map(|(start, end, _)| (start, end))
                .collect();
        }
        // A paragraph that starts below the top of a page goes on to the next
        // one when its first line does not fit, when its first column would
        // keep fewer than `orphans` lines before a break, or when a column
        // after a break on that page would hold fewer than `widows` (CSS
        // Fragmentation 3, 3.3). Below a definite height, the lines overflow.
        let (head_start, head_end) = ranges[0];
        if first == 0
            && top > origin + EPSILON
            && limit.is_none_or(|limit| limit >= page_bottom)
            && (extents[0].1 - extents[0].0 > capacity + EPSILON
                || head_end - head_start < orphans && head_end < extents.len()
                || !fits && ranges[1..].iter().any(|&(start, end)| end - start < widows))
        {
            top = page_bottom;
            continue;
        }
        // Lines move from the last column to the next page when that page
        // would start with fewer than `widows` of them, unless the column
        // cannot give them all and keep its `orphans`.
        if at < extents.len() {
            let deficit = widows.saturating_sub(extents.len() - at);
            let (column_start, column_last) = ranges[ranges.len() - 1];
            if deficit > 0 && column_last - column_start >= orphans + deficit {
                let last = ranges.len() - 1;
                ranges[last].1 -= deficit;
                at -= deficit;
            }
        }
        let y = top - start;
        // Each page's columns are a row of their own: the rules stand between
        // the columns that row fills, as tall as its columns, or as what is
        // left of a definite height.
        let row_height = if definite { capacity } else { column_height };
        groups.push(MulticolGroup {
            context: FragmentationContext {
                origin_x: layout.border.left + layout.padding.left,
                origin_y: top - owner_y,
                available_height: Some(row_height),
                ..context
            },
            height: row_height,
            occupied: (0..ranges.len()).collect(),
            slice: None,
            run_start: None,
            min_height: 0.0,
        });
        for (column, (line_start, line_end)) in ranges.into_iter().enumerate() {
            fragments.push(MulticolTextFragment {
                line_start,
                line_end,
                fragmentainer: column,
                x: context.column_offset_x(column),
                y,
            });
            end = end.max(y + extents[line_end - 1].1 - extents[line_start].0);
        }
        first = at;
        top = page_end;
    }
    if let Some(root) = tree.nodes[owner].ifc.as_mut() {
        root.multicol_fragments = Some(fragments);
    }
    tree.nodes[owner].multicol_groups = groups;
    tree.context.page_filled_columns.push(owner);
    if definite {
        return Ok(0.0);
    }
    // The container keeps its minimum and maximum heights (CSS 2, 10.7).
    let (minimum, maximum) = crate::layout::page_pipeline::border_box_height_bounds(tree, owner);
    let grown = (top_inset + end + bottom_inset).min(maximum).max(minimum);
    let delta = grown - layout.size.height;
    tree.nodes[owner].layout.size.height += delta;
    edits.grow(owner, delta);
    Ok(delta)
}

/// Whether a multicol container holds `owner` in its columns.
fn inside_other_multicol(tree: &LayoutSession<'_>, owner: usize) -> bool {
    let mut walk = AncestorCursor::new(Some(owner), ParentRelation::Layout);
    walk.advance(tree);
    while let Some(id) = walk.current() {
        if tree.nodes[id].multicol.is_some() {
            return true;
        }
        walk.advance(tree);
    }
    false
}

/// Notes that the pages pushed the column content after the spanner
/// `spanner` `pushed` below where it would start: the spanner may grow that
/// far into the space above it without moving it (see
/// `shift_columns_after_spanner` in the page walk).
fn note_slack(tree: &mut LayoutSession<'_>, spanner: usize, pushed: f32) {
    if pushed > EPSILON {
        *tree
            .context
            .spanner_column_slack
            .entry(spanner)
            .or_default() += pushed;
    }
}

/// Whether a page break is forced before the spanner `id`: by its own
/// `break-before`, or by the `break-after` of `previous`, a spanner with no
/// column content between the two.
fn forced_page_break_before_spanner(
    tree: &LayoutSession<'_>,
    id: usize,
    previous: Option<usize>,
) -> bool {
    let cascade = tree.style_views.cascade();
    let document: &Document = tree;
    crate::layout::page_pipeline::page_break_is_forced(
        document,
        cascade,
        id,
        cascade.computed[id].break_before,
    ) || previous.is_some_and(|previous| {
        crate::layout::page_pipeline::page_break_after_is_forced(
            document,
            cascade,
            previous,
            cascade.computed[previous].break_after,
        )
    })
}

/// How far below the border-box top of the spanner `id` its first piece
/// ends: the part at its start that a page does not end inside. That is its
/// top border and padding with the first piece of its first box, down to the
/// end of the first `orphans` lines of the first paragraph. A box that is not
/// a plain block or avoids a break inside it is one piece, and so is a box
/// with no content but its border and padding, which no page cuts; a box with
/// content and nothing in it breaks anywhere below its top edges. Each box
/// visited is charged to `work`.
fn first_piece_end(
    tree: &LayoutSession<'_>,
    id: usize,
    work: &mut ProjectionWork<'_, '_>,
) -> Result<f32, LayoutError> {
    let mut top = 0.0;
    let mut current = id;
    loop {
        work.charge(1)?;
        let node = layout_node(tree, current);
        let layout = node.layout;
        let bottom = top + layout.size.height;
        let content_top = top + layout.border.top + layout.padding.top;
        if node.break_inside != raikiri_style::property::BreakInside::Auto {
            return Ok(bottom);
        }
        // An inline box that holds blocks wraps them in the flow, which
        // starts with the first of them.
        let wraps_blocks = node.display() == DisplayValue::Inline && node.ifc.is_none();
        // The columns of a multicol container start with its first box, as
        // a block's flow does, when they flow on with the page rather than
        // fill a box of a size of its own. An orthogonal flow is monolithic
        // in the page's block axis.
        let style = style_of(tree, current);
        let columns = node.is_multicol_container()
            && node.ifc.is_none()
            && !node.used_writing_mode.is_vertical()
            && !node.size_containment.height
            && style.size().height.is_auto()
            && style.max_size().height.is_auto();
        if !wraps_blocks && !columns && !lineless::is_plain_block_holding_lines(tree, current) {
            // A flex container, a grid or a table breaks between its items
            // or rows as the page walk finds them, below its top edges.
            let breaks_inside = matches!(
                node.display(),
                DisplayValue::Flex | DisplayValue::Grid | DisplayValue::Table
            ) && lineless::is_plain_box_apart_from_display(tree, current, true);
            return Ok(if breaks_inside { content_top } else { bottom });
        }
        let content_bottom = bottom - layout.border.bottom - layout.padding.bottom;
        let lines = node.ifc.as_ref().and_then(|root| root.lines.as_ref());
        // The lines of a paragraph before the first block among them, and that
        // block when it comes first.
        let lines_before = lines.map_or(0, |lines| {
            lines
                .block_line_starts
                .iter()
                .map(|&(_, after)| after)
                .min()
                .unwrap_or(lines.lines.len())
                .min(lines.lines.len())
        });
        if let Some(lines) = lines.filter(|_| lines_before > 0) {
            // A block among the lines counts as one of them, so fewer lines
            // before it than the orphans need take it along whole.
            if lines_before < node.paragraph_orphans.max(1)
                && let Some(&(block, _)) = lines
                    .block_line_starts
                    .iter()
                    .min_by_key(|&&(_, after)| after)
            {
                let block = &tree.nodes[block].layout;
                return Ok(top + block.location.y + block.size.height);
            }
            let last = node.paragraph_orphans.clamp(1, lines_before) - 1;
            return Ok(content_top + lines.line_top(last) + lines.lines[last].block_size());
        }
        let first = match lines {
            Some(lines) => lines.block_line_starts.first().map(|&(child, _)| child),
            // A float or an out-of-flow box beside the flow does not start it.
            None => lineless::rendered_elements(tree, current).find(|&child| {
                !is_floated(tree, child)
                    && !matches!(
                        style_of(tree, child).position(),
                        Position::Absolute | Position::Fixed
                    )
            }),
        };
        match first {
            Some(child) => {
                top += tree.nodes[child].layout.location.y;
                current = child;
            }
            None if content_bottom > content_top + EPSILON => return Ok(content_top),
            None => return Ok(bottom),
        }
    }
}

/// Install the pieces the pages cut the boxes `segment` of a container into:
/// a box with pieces is replaced by them and stands where its first one is,
/// and a box without any moves by `delta` like the rest of the flow. The
/// pieces of what a wrapper holds follow the wrapper's and are placed in
/// them; `nested` says where each box or wrapper inside a wrapper stands.
/// Whether any box was replaced.
fn install_lineless_pieces(
    tree: &mut Document,
    edits: &mut FragmentEdits,
    segment: &[usize],
    container: usize,
    (pieces, nested): (Vec<lineless::PagedPiece>, Vec<(usize, Point<f32>)>),
    delta: f32,
    shifts: &mut HashMap<usize, f32>,
) -> bool {
    // The pieces of each box, in the order the boxes come, which puts a
    // wrapper before what it holds.
    let mut order = Vec::<usize>::new();
    let mut per_child = HashMap::<usize, Vec<lineless::PagedPiece>>::new();
    for piece in pieces {
        let own = per_child.entry(piece.child).or_default();
        if own.is_empty() {
            order.push(piece.child);
        }
        own.push(piece);
    }
    let placed: HashSet<usize> = order.iter().copied().collect();
    for id in order {
        let pieces = per_child.remove(&id).unwrap_or_default();
        let fragments: Vec<LayoutFragment> = pieces
            .iter()
            .map(|piece| LayoutFragment {
                node_id: piece.child,
                parent: Some(piece.parent.map_or(container, |(wrapper, index)| {
                    edits.record(wrapper, index).unwrap_or(container)
                })),
                fragmentainer: piece.fragmentainer,
                rect: piece.rect,
                fragmentainer_clip: piece.clip.map(Into::into),
                fragment_index: piece.index,
                fragment_count: piece.count,
                line_start: None,
                line_end: None,
            })
            .collect();
        tree.nodes[id].layout.location = Point {
            x: fragments[0].rect.x,
            y: fragments[0].rect.y,
        };
        edits.replace(id, fragments);
    }
    for (id, location) in nested {
        tree.nodes[id].layout.location = location;
    }
    for &id in segment.iter().filter(|id| !placed.contains(id)) {
        shift_child(tree, shifts, id, delta);
    }
    !placed.is_empty()
}

/// Move a child now and record the same move for its container fragments.
fn shift_child(tree: &mut Document, shifts: &mut HashMap<usize, f32>, id: usize, delta: f32) {
    tree.nodes[id].layout.location.y += delta;
    *shifts.entry(id).or_default() += delta;
}

/// Changes to the fragment tree made while the spanning column owners are
/// paginated one after another, installed with one rebuild at the end.
/// Rebuilding the tree, or scanning every record, for each owner would cost
/// the owner count times the fragment count.
pub(in crate::layout) struct FragmentEdits {
    /// Every record by id: the tree's records first, then the replacements
    /// in the order they were made.
    arena: Vec<LayoutFragment>,
    /// The number of the tree's own records.
    original: usize,
    /// The ids of the records that describe each source node now, in tree
    /// order.
    by_node: HashMap<usize, Vec<usize>>,
    /// Records whose source node was given new records since.
    dropped: Vec<bool>,
    /// Records of nodes that had none in the tree, each installed after its
    /// parent.
    added: Vec<usize>,
}

impl FragmentEdits {
    pub(in crate::layout) fn new(tree: &crate::fragment::FragmentTree) -> Self {
        let mut by_node = HashMap::<usize, Vec<usize>>::new();
        for (id, fragment) in tree.fragments.iter().enumerate() {
            by_node.entry(fragment.node_id).or_default().push(id);
        }
        Self {
            arena: tree.fragments.clone(),
            original: tree.fragments.len(),
            by_node,
            dropped: vec![false; tree.fragments.len()],
            added: Vec::new(),
        }
    }

    /// The first record of `node` in tree order.
    pub(in crate::layout) fn first(&self, node: usize) -> Option<usize> {
        self.by_node.get(&node)?.first().copied()
    }

    /// The record of `node` at `index` among its records.
    fn record(&self, node: usize, index: usize) -> Option<usize> {
        self.by_node.get(&node)?.get(index).copied()
    }

    fn records(&self, node: usize) -> Vec<usize> {
        self.by_node.get(&node).cloned().unwrap_or_default()
    }

    /// The record `id` stands for now: a dropped record is represented by
    /// the first record of its source node, as the rebuild remaps it.
    fn resolve(&self, id: usize) -> Option<usize> {
        if self.dropped[id] {
            self.first(self.arena[id].node_id)
        } else {
            Some(id)
        }
    }

    fn parent(&self, id: usize) -> Option<usize> {
        self.arena[id]
            .parent
            .and_then(|parent| self.resolve(parent))
    }

    /// Replace the records of `node` with `fragments`, whose parents are ids
    /// of this collection. A node without records keeps none.
    pub(in crate::layout) fn replace(&mut self, node: usize, fragments: Vec<LayoutFragment>) {
        let Some(old) = self.by_node.get(&node).filter(|ids| !ids.is_empty()) else {
            return;
        };
        for &id in old {
            self.dropped[id] = true;
        }
        let start = self.arena.len();
        self.arena.extend(fragments);
        self.dropped.resize(self.arena.len(), false);
        self.by_node
            .insert(node, (start..self.arena.len()).collect());
    }

    /// Replace the records of `node` with `fragments`, or give it them when
    /// it has none.
    pub(in crate::layout) fn replace_or_add(
        &mut self,
        node: usize,
        fragments: Vec<LayoutFragment>,
    ) {
        if self.by_node.get(&node).is_some_and(|ids| !ids.is_empty()) {
            self.replace(node, fragments);
            return;
        }
        let start = self.arena.len();
        self.arena.extend(fragments);
        self.dropped.resize(self.arena.len(), false);
        self.by_node
            .insert(node, (start..self.arena.len()).collect());
        self.added.extend(start..self.arena.len());
    }

    /// Move the records of the shifted children placed in `container`, with
    /// their column clips.
    pub(in crate::layout) fn shift_children(
        &mut self,
        container: usize,
        shifts: &HashMap<usize, f32>,
    ) {
        for (&node, &delta) in shifts {
            for id in self.records(node) {
                if self.parent(id) == Some(container) {
                    self.arena[id].translate(0.0, delta);
                }
            }
        }
    }

    /// Move the records of each node in `moved` that are placed in a record
    /// of `parent` by the node's distance, with their column clips.
    pub(in crate::layout) fn shift_following(
        &mut self,
        parent: usize,
        moved: &HashMap<usize, f32>,
    ) {
        if moved.is_empty() {
            return;
        }
        let parent_records: HashSet<usize> = self.records(parent).into_iter().collect();
        for (&node, &delta) in moved {
            for id in self.records(node) {
                if self
                    .parent(id)
                    .is_some_and(|parent| parent_records.contains(&parent))
                {
                    self.arena[id].translate(0.0, delta);
                }
            }
        }
    }

    /// Make every record of `node` `delta` taller.
    pub(in crate::layout) fn grow(&mut self, node: usize, delta: f32) {
        for id in self.records(node) {
            self.arena[id].rect.height += delta;
        }
    }

    /// Install the edited records in the tree. Replacements take the place
    /// of the first record they replace, and records placed in a replaced
    /// record are placed in the first replacement. On failure the tree is
    /// left as it was.
    pub(in crate::layout) fn finish(
        self,
        tree: &mut Document,
        work: &mut ProjectionWork<'_, '_>,
    ) -> Result<(), LayoutError> {
        if self.arena.len() == self.original {
            tree.fragment_tree.replace_finalized_fragments(self.arena);
            return Ok(());
        }
        work.charge(self.arena.len())?;
        let mut updated = Vec::new();
        // The arena id each installed record comes from.
        let mut sources = Vec::new();
        let mut remap = vec![None; self.arena.len()];
        let mut emitted = HashSet::new();
        let mut children = HashMap::<usize, Vec<usize>>::new();
        for &id in &self.added {
            if let Some(parent) = self.parent(id) {
                children.entry(parent).or_default().push(id);
            }
        }
        for id in 0..self.original {
            let ids = if !self.dropped[id] {
                vec![id]
            } else if emitted.insert(self.arena[id].node_id) {
                self.records(self.arena[id].node_id)
            } else {
                continue;
            };
            // Each record is followed by the added records placed in it, and
            // those by theirs.
            let mut pending: Vec<usize> = ids.into_iter().rev().collect();
            while let Some(id) = pending.pop() {
                remap[id] = Some(updated.len());
                updated.push(self.arena[id]);
                sources.push(id);
                pending.extend(children.remove(&id).unwrap_or_default().into_iter().rev());
            }
            if updated.len() > tree.fragment_tree.limit {
                // cov:ignore: the preceding work reservation bounds the original plus all replacement records by the same fragment limit.
                return Err(LayoutError::FragmentLimitExceeded {
                    limit: tree.fragment_tree.limit,
                });
            }
        }
        for (fragment, source) in updated.iter_mut().zip(sources) {
            fragment.parent = self.parent(source).and_then(|parent| remap[parent]);
        }
        tree.fragment_tree.replace_finalized_fragments(updated);
        Ok(())
    }
}

/// The page on which content before the columns leaves a break before them:
/// a previous spanner, which ends where the columns start, or in-flow content
/// before the container on the page where its margin box starts. Content that
/// ends on an earlier page leaves its break at that page's end, where the
/// container starts its page.
fn break_before_page(
    tree: &LayoutSession<'_>,
    owner: usize,
    owner_y: f32,
    context: &FragmentationContext,
    after_spanner: bool,
    page_for: &impl Fn(f32) -> (u32, f32, f32),
) -> Option<u32> {
    if after_spanner {
        return Some(page_for(owner_y + context.origin_y).0);
    }
    let top = owner_y - tree.nodes[owner].layout.margin.top.max(0.0);
    let (page, origin, _) = page_for(top);
    (top > origin + EPSILON && has_content_before(tree, owner)).then_some(page)
}

/// Whether in-flow content comes before `owner` in the flow that holds it,
/// or before one of its ancestors: a break before the columns is left there
/// (CSS Fragmentation 3, 4.4).
fn has_content_before(tree: &LayoutSession<'_>, owner: usize) -> bool {
    let mut node = owner;
    // The boxes of a `display: contents` ancestor stand in its parent's
    // flow, where the siblings before it are found.
    let layout_parent = |node: usize| {
        let mut parent = tree.parent_of(node);
        while let Some(contents) =
            parent.filter(|&parent| tree.nodes[parent].display() == DisplayValue::Contents)
        {
            parent = tree.parent_of(contents);
        }
        parent
    };
    while let Some(parent) = layout_parent(node) {
        let before = tree.children_through_contents(parent);
        let content = before
            .iter()
            .take_while(|&&child| child != node)
            .any(|&child| {
                let sibling = &tree.nodes[child];
                let in_flow_box = sibling.kind() == NodeKind::Element
                    && !crate::layout::page_pipeline::is_out_of_flow_positioned(tree, child)
                    && !style_of(tree, child).float().is_floated();
                // Text other than white space between blocks makes lines.
                let text = sibling.kind() == NodeKind::Text
                    && sibling
                        .text_content()
                        .is_some_and(|text| !text.trim_ascii().is_empty());
                sibling.is_in_document() && !sibling.is_display_none() && (in_flow_box || text)
            });
        if content {
            return true;
        }
        node = parent;
    }
    false
}

/// The block-axis border and padding, top and bottom, that the container
/// `owner` repeats on each page it is cut on, or `None` when it does not
/// clone its decorations or has none to repeat.
fn cloned_page_insets(tree: &LayoutSession<'_>, owner: usize) -> Option<(f32, f32)> {
    let node = &tree.nodes[owner];
    let top = node.layout.border.top + node.layout.padding.top;
    let bottom = node.layout.border.bottom + node.layout.padding.bottom;
    (node.clones_box_decorations && top + bottom > 0.0).then_some((top, bottom))
}

#[cfg(test)]
mod tests;
