use super::*;
use crate::fragment::{LayoutFragment, MulticolGroup};
use std::collections::HashSet;

// The page pipeline supplies the owner's final source position and page
// schedule. Shaped paragraph lines remain authoritative; only their column
// placement changes when a group resumes in a later page fragmentainer.
//
// Fragment records are not edited in the tree: `edits` collects the changes
// of every owner, and the caller installs them once.
pub(in crate::layout) fn paginate_spanning_columns(
    tree: &mut Document,
    edits: &mut FragmentEdits,
    owner: usize,
    owner_y: f32,
    page_for: &impl Fn(f32) -> (u32, f32, f32),
    work: &mut ProjectionWork<'_, '_>,
) -> Result<f32, LayoutError> {
    let old_groups = tree.nodes[owner].multicol_groups.clone();
    if old_groups.is_empty() || !tree.nodes[owner].style.size.height.is_auto() {
        return Ok(0.0);
    }
    let Some(container) = edits.first(owner) else {
        return Ok(0.0); // cov:ignore: retained groups are created only after the owner source fragment exists.
    };
    let children = tree.nodes[owner].children.clone();
    work.charge(children.len())?;
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
            let node = &tree.nodes[id];
            node.kind() != NodeKind::Text
                || node.unrounded_layout.size.height != 0.0
                || node.ifc.is_some()
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
            if let Some(planned) =
                paragraph_group::paginate(tree, owner, &entries, context, owner_y, page_for, work)?
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
                    node.unrounded_layout.location = Point {
                        x: first.x,
                        y: first.y,
                    };
                    node.unrounded_layout.size = Size {
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
                                fragmentainer_clip: Some(f.clip),
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
            let layout = tree.nodes[id].unrounded_layout;
            let y = owner_y + layout.location.y;
            let (_, origin, height) = page_for(y);
            if layout.size.height <= height
                && y > origin + 0.001
                && y + layout.size.height > origin + height + 0.001
            {
                let shift = origin + height - y;
                shift_child(tree, &mut shifts, id, shift);
                delta += shift;
            }
        }
        start = end + 1;
    }
    edits.shift_children(container, &shifts);
    for (id, fragments) in replacements {
        edits.replace(id, fragments);
    }
    tree.nodes[owner].multicol_groups = groups;
    tree.nodes[owner].unrounded_layout.size.height += delta;
    edits.grow(owner, delta);
    Ok(delta)
}

/// Move a child now and record the same move for its container fragments.
fn shift_child(tree: &mut Document, shifts: &mut HashMap<usize, f32>, id: usize, delta: f32) {
    tree.nodes[id].unrounded_layout.location.y += delta;
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
