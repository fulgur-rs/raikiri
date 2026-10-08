//! Composite a typographic pseudo's complete paint as one opacity group.

use anyrender::{PaintScene, Scene};
use kurbo::{Affine, Rect};
use peniko::Mix;
use raikiri_dom::{InlineBoxPiece, Node};
use std::collections::HashMap;

struct Group {
    scene: Scene,
    opacity: f32,
    parent: Option<usize>,
    insertion: Option<usize>,
    children: Vec<(usize, usize, Scene)>,
}

pub(super) struct TypographicPaint {
    normal: Scene,
    groups: Vec<Group>,
    nearest_groups: HashMap<usize, Option<usize>>,
    piece_groups: HashMap<usize, Option<usize>>,
}

impl TypographicPaint {
    pub(super) fn new(root: Option<&Node>, pieces: &[InlineBoxPiece]) -> Self {
        let mut paint = Self {
            normal: Scene::new(),
            groups: Vec::new(),
            nearest_groups: HashMap::new(),
            piece_groups: HashMap::new(),
        };
        if let Some(root) = root {
            let mut by_box = HashMap::new();
            let mut parents = HashMap::new();
            for piece in pieces {
                parents.entry(piece.node)
                    .or_insert_with(|| root.ifc_typographic_parent(piece.node));
                if by_box.contains_key(&piece.node) {
                    continue;
                }
                if let Some((cv, _)) = root.ifc_typographic_fragment(
                    piece.node,
                    piece.source_container,
                    piece.source_owner,
                ) && cv.opacity < 1.0
                {
                    by_box.insert(piece.node, paint.groups.len());
                    paint.groups.push(Group {
                        scene: Scene::new(),
                        opacity: cv.opacity,
                        parent: None,
                        insertion: None,
                        children: Vec::new(),
                    });
                }
            }
            // Resolve retained parents once per box, including ancestors whose
            // pieces are absent on this line. Parent inspection scans styles.
            let mut pending: Vec<_> = parents.values().copied().flatten().collect();
            while let Some(id) = pending.pop() {
                if let std::collections::hash_map::Entry::Vacant(entry) = parents.entry(id) {
                    let parent = root.ifc_typographic_parent(id);
                    entry.insert(parent);
                    pending.extend(parent);
                }
            }
            paint
                .nearest_groups
                .extend(by_box.iter().map(|(&id, &group)| (id, Some(group))));
            for &id in parents.keys() {
                cached_group(id, &parents, &mut paint.nearest_groups);
            }
            for (&id, &index) in &by_box {
                paint.groups[index].parent = parents[&id]
                    .and_then(|parent| paint.nearest_groups.get(&parent).copied().flatten());
            }
            let piece_parents: HashMap<_, _> = pieces
                .iter()
                .map(|piece| (piece.node, piece.parent))
                .collect();
            paint.piece_groups.extend(
                paint
                    .nearest_groups
                    .iter()
                    .filter_map(|(&id, &group)| group.map(|group| (id, Some(group)))),
            );
            for &id in piece_parents.keys() {
                cached_group(id, &piece_parents, &mut paint.piece_groups);
            }
        }
        paint
    }

    pub(super) fn nearest(&self, id: usize) -> Option<usize> {
        self.nearest_groups.get(&id).copied().flatten()
    }

    pub(super) fn piece_group(&self, piece: &InlineBoxPiece) -> Option<usize> {
        self.piece_groups.get(&piece.node).copied().flatten()
    }

    pub(super) fn target(&mut self, group: Option<usize>) -> &mut Scene {
        match group {
            Some(index) => &mut self.groups[index].scene,
            None => &mut self.normal,
        }
    }

    pub(super) fn start_glyphs(&mut self, mut group: Option<usize>) {
        while let Some(index) = group {
            let parent = self.groups[index].parent;
            if self.groups[index].insertion.is_some() {
                break;
            }
            self.groups[index].insertion = Some(self.target(parent).commands.len());
            group = parent;
        }
    }

    pub(super) fn finish(
        mut self,
        scene: &mut impl PaintScene,
        clip: Rect,
        paint_transform: Affine,
    ) {
        // The enclosing TransformScene transforms recorded commands on replay.
        // Counter-transform only the opacity clip so it remains in page space.
        let inverse = paint_transform.inverse();
        let clip_transform = if inverse.as_coeffs().iter().all(|value| value.is_finite()) {
            inverse
        } else {
            // A singular ancestor transform has no drawable two-dimensional ink.
            Affine::IDENTITY
        };
        let mut remaining = vec![0; self.groups.len()];
        for group in &self.groups {
            if let Some(parent) = group.parent {
                remaining[parent] += 1;
            }
        }
        let mut order: Vec<_> = remaining
            .iter()
            .enumerate()
            .filter_map(|(index, &children)| (children == 0).then_some(index))
            .collect();
        let mut roots = Vec::new();
        while let Some(index) = order.pop() {
            let group = &mut self.groups[index];
            let content = merge_children(
                std::mem::take(&mut group.scene),
                std::mem::take(&mut group.children),
            );
            let mut composite = Scene::new();
            composite.push_layer(
                Mix::Normal,
                group.opacity,
                clip_transform,
                &clip,
                None,
                None,
            );
            composite.append_scene(content, Affine::IDENTITY);
            composite.pop_layer();
            let insertion = group.insertion;
            let parent = group.parent;
            let insertion = insertion.unwrap_or_else(|| self.target(parent).commands.len());
            let child = (insertion, index, composite);
            match parent {
                Some(parent) => {
                    self.groups[parent].children.push(child);
                    remaining[parent] -= 1;
                    if remaining[parent] == 0 {
                        order.push(parent);
                    }
                }
                None => roots.push(child),
            }
        }
        scene.append_scene(merge_children(self.normal, roots), Affine::IDENTITY);
    }
}

fn cached_group(
    mut id: usize,
    parents: &HashMap<usize, Option<usize>>,
    cache: &mut HashMap<usize, Option<usize>>,
) -> Option<usize> {
    let mut path = Vec::new();
    let group = loop {
        if let Some(&group) = cache.get(&id) {
            break group;
        }
        #[cfg(test)]
        PARENT_GROUP_VISITS.with(|visits| visits.set(visits.get() + 1));
        path.push(id);
        match parents.get(&id).copied().flatten() {
            Some(parent) => id = parent,
            None => break None,
        }
    };
    for id in path {
        cache.insert(id, group);
    }
    group
}

fn merge_children(scene: Scene, mut children: Vec<(usize, usize, Scene)>) -> Scene {
    if children.is_empty() {
        return scene;
    }
    children.sort_by_key(|(position, order, _)| (*position, *order));
    let mut children = children.into_iter().peekable();
    let mut result = Scene::new();
    for (index, command) in scene.commands.into_iter().enumerate() {
        while children
            .peek()
            .is_some_and(|(position, _, _)| *position == index)
        {
            if let Some((_, _, child)) = children.next() {
                result.commands.extend(child.commands);
            }
        }
        result.commands.push(command);
    }
    for (_, _, child) in children {
        result.commands.extend(child.commands);
    }
    result
}

#[cfg(test)]
thread_local! {
    static PARENT_GROUP_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests;
