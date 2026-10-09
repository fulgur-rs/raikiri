use super::*;
use crate::fragment::{LayoutFragment, MulticolGroup};

// The page pipeline supplies the owner's final source position and page
// schedule. Shaped paragraph lines remain authoritative; only their column
// placement changes when a group resumes in a later page fragmentainer.
pub(in crate::layout) fn paginate_spanning_columns(
    tree: &mut Document,
    owner: usize,
    owner_y: f32,
    page_for: &impl Fn(f32) -> (u32, f32, f32),
    work: &mut ProjectionWork<'_, '_>,
) -> Result<f32, LayoutError> {
    let old_groups = tree.nodes[owner].multicol_groups.clone();
    if old_groups.is_empty() || !tree.nodes[owner].style.size.height.is_auto() {
        return Ok(0.0);
    }
    let Some(container) = tree
        .fragment_tree
        .fragments
        .iter()
        .position(|f| f.node_id == owner)
    else {
        return Ok(0.0); // cov:ignore: retained groups are created only after the owner source fragment exists.
    };
    let children = tree.nodes[owner].children.clone();
    work.charge(children.len())?;
    let mut groups = Vec::<MulticolGroup>::new();
    let mut replacements = HashMap::<usize, Vec<LayoutFragment>>::new();
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
                        shift_child(tree, id, container, delta);
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
                    replacements.insert(
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
                    );
                }
                delta = new_end - (old.context.origin_y + old.height);
            } else {
                groups.push(MulticolGroup {
                    context,
                    ..old.clone()
                });
                for &id in segment {
                    shift_child(tree, id, container, delta);
                }
            }
        }
        if let Some(id) = spanner {
            shift_child(tree, id, container, delta);
            let layout = tree.nodes[id].unrounded_layout;
            let y = owner_y + layout.location.y;
            let (_, origin, height) = page_for(y);
            if layout.size.height <= height
                && y > origin + 0.001
                && y + layout.size.height > origin + height + 0.001
            {
                let shift = origin + height - y;
                shift_child(tree, id, container, shift);
                delta += shift;
            }
        }
        start = end + 1;
    }
    replace_fragments(tree, replacements, work)?;
    tree.nodes[owner].multicol_groups = groups;
    tree.nodes[owner].unrounded_layout.size.height += delta;
    for fragment in &mut tree.fragment_tree.fragments {
        if fragment.node_id == owner {
            fragment.rect.height += delta;
        }
    }
    Ok(delta)
}

fn shift_child(tree: &mut Document, id: usize, container: usize, delta: f32) {
    tree.nodes[id].unrounded_layout.location.y += delta;
    for fragment in &mut tree.fragment_tree.fragments {
        if fragment.node_id == id && fragment.parent == Some(container) {
            fragment.rect.y += delta;
            if let Some(clip) = &mut fragment.fragmentainer_clip {
                clip.y += delta;
            }
        }
    }
}

fn replace_fragments(
    tree: &mut Document,
    mut replacements: HashMap<usize, Vec<LayoutFragment>>,
    work: &mut ProjectionWork<'_, '_>,
) -> Result<(), LayoutError> {
    if replacements.is_empty() {
        return Ok(());
    }
    let old = &tree.fragment_tree.fragments;
    work.charge(
        old.len()
            .saturating_add(replacements.values().map(Vec::len).sum::<usize>()),
    )?;
    let mut updated = Vec::new();
    let mut remap = vec![0; old.len()];
    let mut replaced = HashMap::new();
    for (index, fragment) in old.iter().enumerate() {
        if let Some(fragments) = replacements.remove(&fragment.node_id) {
            remap[index] = updated.len();
            replaced.insert(fragment.node_id, updated.len());
            updated.extend(fragments);
        } else if let Some(&first) = replaced.get(&fragment.node_id) {
            remap[index] = first;
        } else {
            remap[index] = updated.len();
            updated.push(*fragment);
        }
        if updated.len() > tree.fragment_tree.limit {
            // cov:ignore: the preceding scan reservation bounds old plus all replacement records by the same fragment limit.
            return Err(LayoutError::FragmentLimitExceeded {
                limit: tree.fragment_tree.limit,
            });
        }
    }
    for fragment in &mut updated {
        fragment.parent = fragment.parent.map(|parent| remap[parent]);
    }
    tree.fragment_tree.fragments = updated;
    Ok(())
}

#[cfg(test)]
mod tests;
