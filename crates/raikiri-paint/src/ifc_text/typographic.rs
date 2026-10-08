//! Composite a typographic pseudo's complete paint as one opacity group.

use anyrender::{PaintScene, Scene};
use kurbo::{Affine, Rect};
use peniko::Mix;
use raikiri_dom::{InlineBoxPiece, Node};
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(crate) struct BackgroundSlice {
    pub(crate) outer: raikiri_dom::BoxRect,
    pub(crate) content: raikiri_dom::BoxRect,
}

pub(super) fn background_slices(pieces: &[InlineBoxPiece]) -> HashMap<usize, BackgroundSlice> {
    fn extend(rect: &mut raikiri_dom::BoxRect, next: raikiri_dom::BoxRect) {
        let end = (rect.x + rect.width).max(next.x + next.width);
        rect.x = rect.x.min(next.x);
        rect.width = end - rect.x;
    }
    let mut slices = HashMap::<usize, BackgroundSlice>::new();
    for piece in pieces {
        if raikiri_dom::generated_content::generated_origin(piece.node)
            .is_some_and(|(_, pseudo)| pseudo == raikiri_style::PseudoElem::FirstLetter)
        {
            slices
                .entry(piece.node)
                .and_modify(|slice| {
                    extend(&mut slice.outer, piece.border_box);
                    extend(&mut slice.content, piece.content_box);
                })
                .or_insert(BackgroundSlice {
                    outer: piece.border_box,
                    content: piece.content_box,
                });
        }
    }
    slices
}

struct Group {
    scene: Scene,
    opacity: f32,
    parent: Option<usize>,
    insertion: Option<usize>,
    children: Vec<(usize, usize, Scene)>,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct FragmentKey {
    node: usize,
    owner: Option<usize>,
}

impl FragmentKey {
    fn parent(self, node: usize) -> Self {
        Self { node, ..self }
    }
}

pub(super) struct TypographicPaint {
    normal: Scene,
    groups: Vec<Group>,
    nearest_groups: HashMap<FragmentKey, Option<usize>>,
    piece_groups: HashMap<FragmentKey, Option<usize>>,
}

impl TypographicPaint {
    pub(super) fn new(root: &Node, pieces: &[InlineBoxPiece]) -> Self {
        let mut paint = Self {
            normal: Scene::new(),
            groups: Vec::new(),
            nearest_groups: HashMap::new(),
            piece_groups: HashMap::new(),
        };
        let mut by_box = HashMap::new();
        let mut parents = HashMap::new();
        let mut opacities = HashMap::new();
        for piece in pieces {
            let key = FragmentKey {
                node: piece.node,
                owner: piece.source_owner,
            };
            parents.entry(key).or_insert_with(|| {
                root.ifc_typographic_parent(piece.node)
                    .map(|id| key.parent(id))
            });
            if let Some((cv, _)) = root.ifc_typographic_fragment(
                piece.node,
                piece.source_container,
                piece.source_owner,
            ) && cv.opacity < 1.0
            {
                opacities.insert(key, cv.opacity);
            }
        }
        // Resolve parents before children. Equal opacities share a group only
        // when their enclosing fragment groups also agree.
        for &key in parents.keys() {
            let mut path = Vec::new();
            let mut next = key;
            let mut group = loop {
                if let Some(&group) = paint.nearest_groups.get(&next) {
                    break group;
                }
                #[cfg(test)]
                PARENT_GROUP_VISITS.with(|visits| visits.set(visits.get() + 1));
                paint.nearest_groups.insert(next, None);
                path.push(next);
                match parents.get(&next).copied().flatten() {
                    Some(parent) => next = parent,
                    None => break None,
                }
            };
            for key in path.into_iter().rev() {
                if let Some(&opacity) = opacities.get(&key) {
                    let index = *by_box
                        .entry((key.node, opacity.to_bits(), group))
                        .or_insert_with(|| {
                            let index = paint.groups.len();
                            paint.groups.push(Group {
                                scene: Scene::new(),
                                opacity,
                                parent: group,
                                insertion: None,
                                children: Vec::new(),
                            });
                            index
                        });
                    group = Some(index);
                }
                paint.nearest_groups.insert(key, group);
            }
        }
        let piece_parents: HashMap<_, _> = pieces
            .iter()
            .map(|piece| {
                let key = FragmentKey {
                    node: piece.node,
                    owner: piece.source_owner,
                };
                (key, piece.parent.map(|id| key.parent(id)))
            })
            .collect();
        paint.piece_groups.extend(
            paint
                .nearest_groups
                .iter()
                .filter_map(|(&key, &group)| group.map(|group| (key, Some(group)))),
        );
        for &key in piece_parents.keys() {
            cached_group(key, &piece_parents, &mut paint.piece_groups);
        }
        paint
    }

    pub(super) fn nearest(&self, id: usize, owner: usize) -> Option<usize> {
        self.nearest_groups
            .get(&FragmentKey {
                node: id,
                owner: Some(owner),
            })
            .copied()
            .flatten()
    }

    pub(super) fn piece_group(&self, piece: &InlineBoxPiece) -> Option<usize> {
        self.piece_groups
            .get(&FragmentKey {
                node: piece.node,
                owner: piece.source_owner,
            })
            .copied()
            .flatten()
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
    mut id: FragmentKey,
    parents: &HashMap<FragmentKey, Option<FragmentKey>>,
    cache: &mut HashMap<FragmentKey, Option<usize>>,
) -> Option<usize> {
    let mut path = Vec::new();
    let group = loop {
        if let Some(&group) = cache.get(&id) {
            break group;
        }
        #[cfg(test)]
        PARENT_GROUP_VISITS.with(|visits| visits.set(visits.get() + 1));
        // Nested marker fragments can share an id and form an alias cycle.
        // Mark the path before following parents, then replace it with any
        // active group reached below; all active groups were seeded earlier.
        cache.insert(id, None);
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
