use super::*;
use crate::document::ancestors::{AncestorCursor, ParentRelation};
use crate::fragment::{LayoutFragment, MulticolGroup};
use lineless::EPSILON;
use std::collections::HashSet;

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
        let children = tree.children_through_contents(owner);
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
    // axes: its columns lay out on the horizontal axes of the page.
    if old_groups.is_empty() || tree.context.pagination_on_transposed_axes {
        return Ok(0.0);
    }
    let children = tree.children_through_contents(owner);
    work.charge(children.len())?;
    let spanner_free = !children.iter().any(|&id| spanning::is_spanner(tree, id));
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
    let Some(container) = edits.first(owner) else {
        return Ok(0.0); // cov:ignore: retained groups are created only after the owner source fragment exists.
    };
    let mut groups = Vec::<MulticolGroup>::new();
    let mut replacements = Vec::<(usize, Vec<LayoutFragment>)>::new();
    // Fragment shifts are applied in one pass after the walk: scanning every
    // fragment for each shifted child is quadratic in the group's children.
    let mut shifts = HashMap::<usize, f32>::new();
    let mut start = 0;
    let mut group_index = 0;
    let mut delta = 0.0;
    for end in 0..=children.len() {
        let spanner = children
            .get(end)
            .copied()
            .filter(|&id| spanning::is_spanner(tree, id));
        if end < children.len() && spanner.is_none() {
            continue;
        }
        let segment = &children[start..end];
        if segment.iter().any(|&id| {
            let node = layout_node(tree, id);
            node.kind() != NodeKind::Text || node.layout.size.height != 0.0 || node.ifc.is_some()
        }) {
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
            if let Some(planned) =
                paragraph_group::paginate(tree, &entries, context, owner_y, page_for, work)?
                    .filter(|planned| !cuts_shifted_paragraph(tree, &planned.fragments))
            {
                let new_end = planned.end;
                groups.extend(planned.groups);
                let mut per_child = HashMap::<usize, Vec<paragraph_group::PagedFragment>>::new();
                for fragment in planned.fragments {
                    per_child.entry(fragment.child).or_default().push(fragment);
                }
                for &id in segment {
                    let Some(fragments) = per_child.remove(&id) else {
                        shift_child(tree, &mut shifts, id, delta);
                        continue;
                    };
                    let first = fragments[0].rect;
                    let node = &mut tree.nodes[id];
                    node.layout.location = Point {
                        x: first.x,
                        y: first.y,
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
                    replacements.push((
                        id,
                        fragments
                            .into_iter()
                            .map(|f| LayoutFragment {
                                node_id: id,
                                parent: Some(container),
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
            shift_child(tree, &mut shifts, id, delta);
            let layout = tree.nodes[id].layout;
            let y = owner_y + layout.location.y;
            let (_, origin, height) = page_for(y);
            // A spanner that can be cut where each page ends continues on the
            // next page, as a block of the flow does; the others move whole.
            if layout.size.height <= height
                && y > origin + 0.001
                && y + layout.size.height > origin + height + 0.001
                && !breaks_at_every_page_end(tree, id, y, origin + height, page_for, work)?
            {
                let shift = origin + height - y;
                shift_child(tree, &mut shifts, id, shift);
                delta += shift;
            }
        }
        start = end + 1;
    }
    edits.shift_children(container, &shifts);
    let refilled = !replacements.is_empty();
    for (id, fragments) in replacements {
        edits.replace(id, fragments);
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

/// Whether the spanner `id`, whose border box starts at `y` on a page that
/// ends at `page_end`, may be cut at each page end it crosses. Pages differ in
/// height when a page rule or a named page sizes them, so a spanner that fits
/// the page it starts on may cross more than one page end. Each one visited
/// is charged to `work`, which bounds their number and checks for an abort.
fn breaks_at_every_page_end(
    tree: &LayoutSession<'_>,
    id: usize,
    y: f32,
    mut page_end: f32,
    page_for: &impl Fn(f32) -> (u32, f32, f32),
    work: &mut ProjectionWork<'_, '_>,
) -> Result<bool, LayoutError> {
    let Some(limits) = CutLimits::of(tree, id, work)? else {
        return Ok(false);
    };
    let bottom = y + tree.nodes[id].layout.size.height;
    while page_end < bottom - EPSILON {
        if !limits.allows(page_end - y) {
            return Ok(false);
        }
        work.charge(1)?;
        let (_, origin, height) = page_for(page_end + EPSILON);
        page_end = origin + height;
    }
    Ok(true)
}

/// What keeps a page from ending inside a spanner, found once for all the page
/// ends the spanner crosses. A page may end inside a plain block of the flow
/// where only empty space is cut: the spanner and every box inside it are
/// plain blocks that allow a break inside them, no box holds anything but
/// lines of text, every line ends above the cut, and the cut lies in the
/// content of each box it crosses. The part of the spanner past the end of the
/// page continues on the next one then.
///
/// A box whose last line meets the cut would leave nothing but empty space
/// for the next page, so it does not break there.
struct CutLimits {
    /// The lowest cut, below the top of the spanner, that leaves every line
    /// above it.
    text_bottom: f32,
    /// The ranges, ascending and apart from each other, in which a cut would
    /// split a border or a padding. A cut at either end is outside the range.
    bands: Vec<(f32, f32)>,
}

impl CutLimits {
    /// The limits of the spanner `id`, or `None` when it holds something a page
    /// may not end inside. Each box visited is charged to `work`.
    fn of(
        tree: &LayoutSession<'_>,
        id: usize,
        work: &mut ProjectionWork<'_, '_>,
    ) -> Result<Option<Self>, LayoutError> {
        let mut text_bottom = f32::NEG_INFINITY;
        let mut bands = Vec::new();
        let mut pending = vec![(id, 0.0_f32)];
        while let Some((current, top)) = pending.pop() {
            work.charge(1)?;
            let node = layout_node(tree, current);
            if !lineless::is_plain_block_holding_lines(tree, current)
                || node.break_inside != raikiri_style::property::BreakInside::Auto
            {
                return Ok(None);
            }
            let layout = node.layout;
            let bottom = top + layout.size.height;
            let content_top = top + layout.border.top + layout.padding.top;
            let content_bottom = bottom - layout.border.bottom - layout.padding.bottom;
            // A border or padding is not split by the page, so no cut may fall
            // in one of the box.
            bands.extend(
                [
                    (top + EPSILON, (content_top - EPSILON).min(bottom - EPSILON)),
                    (
                        (content_bottom + EPSILON).max(top + EPSILON),
                        bottom - EPSILON,
                    ),
                ]
                .into_iter()
                .filter(|(low, high)| low < high),
            );
            // The lines hold all that a paragraph has, up to the boxes among
            // them.
            if let Some(root) = &node.ifc {
                if !root.boxes.is_empty() || root.multicol_fragments.is_some() {
                    return Ok(None);
                }
                if let Some(lines) = &root.lines
                    && let Some(last) = lines.lines.last()
                {
                    let last_end =
                        content_top + lines.line_top(lines.lines.len() - 1) + last.block_size();
                    text_bottom = text_bottom.max(last_end);
                }
                continue;
            }
            pending.extend(
                lineless::rendered_elements(tree, current)
                    .map(|child| (child, top + tree.nodes[child].layout.location.y)),
            );
        }
        bands.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut apart = Vec::<(f32, f32)>::with_capacity(bands.len());
        for (low, high) in bands {
            match apart.last_mut() {
                Some(last) if low < last.1 => last.1 = last.1.max(high),
                _ => apart.push((low, high)),
            }
        }
        Ok(Some(Self {
            text_bottom,
            bands: apart,
        }))
    }

    /// Whether a page may end `cut` below the top of the spanner.
    fn allows(&self, cut: f32) -> bool {
        // Only the last range that starts before the cut can hold it.
        let before = self.bands.partition_point(|&(low, _)| low < cut);
        self.text_bottom + EPSILON <= cut
            && before
                .checked_sub(1)
                .is_none_or(|last| cut >= self.bands[last].1)
    }
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
        }
    }

    /// The first record of `node` in tree order.
    pub(in crate::layout) fn first(&self, node: usize) -> Option<usize> {
        self.by_node.get(&node)?.first().copied()
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
                    let fragment = &mut self.arena[id];
                    fragment.rect.y += delta;
                    if let Some(clip) = &mut fragment.fragmentainer_clip {
                        clip.y += delta;
                    }
                }
            }
        }
    }

    /// Move the records of each node in `moved` that are placed in a record
    /// of `parent` by the node's distance.
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
                    self.arena[id].rect.y += delta;
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
            tree.fragment_tree.fragments = self.arena;
            return Ok(());
        }
        work.charge(self.arena.len())?;
        let mut updated = Vec::new();
        // The arena id each installed record comes from.
        let mut sources = Vec::new();
        let mut remap = vec![None; self.arena.len()];
        let mut emitted = HashSet::new();
        for id in 0..self.original {
            let ids = if !self.dropped[id] {
                vec![id]
            } else if emitted.insert(self.arena[id].node_id) {
                self.records(self.arena[id].node_id)
            } else {
                continue;
            };
            for id in ids {
                remap[id] = Some(updated.len());
                updated.push(self.arena[id]);
                sources.push(id);
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
        tree.fragment_tree.fragments = updated;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
