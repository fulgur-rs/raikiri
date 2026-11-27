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

/// Whether `fragments` cut a paragraph that paints with an offset of its own,
/// a `position: relative` inset or a transform. A cut piece is clipped at its
/// column and the painter moves the offset inside that clip, so the page
/// would not report the runs of its lines; such a paragraph stays in the
/// columns that the pages cut.
fn cuts_shifted_paragraph(
    tree: &LayoutSession<'_>,
    fragments: &[paragraph_group::PagedFragment],
) -> bool {
    let cascade = tree.style_views.cascade();
    fragments.iter().any(|fragment| {
        fragment.clip.is_some()
            && (crate::layout::used_relative_offset_for_layout(
                tree.document,
                cascade,
                fragment.child,
            ) != (0.0, 0.0)
                || cascade
                    .computed
                    .get(fragment.child)
                    .is_some_and(|values| !values.transform.is_empty()))
    })
}

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
    // between its columns; the pages cut its columns as they cut those of a
    // container without spanners.
    if tree.context.pagination_on_transposed_axes || tree.nodes[owner].is_ifc_root() {
        return Ok(0.0);
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
    let whole = spanner_free && {
        let old = &old_groups[0];
        let top = owner_y + old.context.origin_y;
        let (_, origin, height) = page_for(top);
        top + old.height <= origin + height + EPSILON || inside_atomic_inline(tree, owner)
    };
    if whole || !style_of(tree, owner).size().height.is_auto() {
        if spanner_free {
            tree.nodes[owner].multicol_groups.clear();
        }
        return Ok(0.0);
    }
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
            if let Some(planned) = leaves
                .as_deref()
                .map(|leaves| lineless::paginate(tree, leaves, context, owner_y, page_for, work))
                .transpose()?
                .flatten()
            {
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
                previous_spanner.is_some() || has_content_before(tree, owner),
                page_for,
                work,
            )?
            .filter(|planned| !cuts_shifted_paragraph(tree, &planned.fragments))
            {
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
    tree.nodes[owner].multicol_groups = groups;
    tree.nodes[owner].layout.size.height += delta;
    edits.grow(owner, delta);
    Ok(delta)
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

#[cfg(test)]
mod tests;
