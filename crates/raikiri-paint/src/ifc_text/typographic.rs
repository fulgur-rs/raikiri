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

/// `rtl` tells whether an element's inline base direction is right-to-left.
pub(super) fn background_slices(
    pieces: &[InlineBoxPiece],
    rtl: impl Fn(usize) -> bool,
) -> Vec<Option<BackgroundSlice>> {
    let mut groups = HashMap::<usize, Vec<usize>>::new();
    for (index, piece) in pieces.iter().enumerate() {
        groups.entry(piece.node).or_default().push(index);
    }
    let mut slices = vec![None; pieces.len()];
    for (&node, indices) in groups.iter_mut() {
        if indices.len() < 2 {
            continue;
        }
        // Join the boxes line after line, each line's in visual order,
        // excluding their owners' margins (CSS Fragmentation 3 §5.4
        // `box-decoration-break: slice`). The first line's fragment sits at
        // the inline start of the joined box, so right-to-left boxes join
        // later lines to its left. Keep each fragment's own height and move
        // the virtual composite origin so its slice aligns with the
        // fragment's physical position.
        let rtl = rtl(node);
        indices.sort_by(|&a, &b| {
            let lines = pieces[a].line.cmp(&pieces[b].line);
            if rtl { lines.reverse() } else { lines }
                .then(pieces[a].border_box.x.total_cmp(&pieces[b].border_box.x))
        });
        let first = &pieces[indices[0]];
        let last = &pieces[*indices.last().expect("nonempty pseudo fragment group")];
        let left = first.content_box.x - first.border_box.x;
        let right =
            last.border_box.x + last.border_box.width - last.content_box.x - last.content_box.width;
        let width = indices
            .iter()
            .map(|&index| pieces[index].border_box.width)
            .sum::<f32>();
        let mut offset = 0.0;
        for &index in indices.iter() {
            let piece = &pieces[index];
            let mut outer = piece.border_box;
            outer.x -= offset;
            outer.width = width;
            let mut content = piece.content_box;
            content.x = outer.x + left;
            content.width = width - left - right;
            slices[index] = Some(BackgroundSlice { outer, content });
            offset += piece.border_box.width;
        }
    }
    slices
}

struct Group {
    scene: Scene,
    opacity: f32,
    level: i32,
    parent: Option<usize>,
    children: Vec<Child>,
}

/// A composited group placed in its parent's paint: the parent command it
/// precedes, its stack level, its allocation order, and its commands.
type Child = (usize, i32, usize, Scene);

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
    /// `cascade` supplies the opacity of the `::before` and `::after` boxes,
    /// which, like `::first-letter` ones, composite their complete paint as
    /// one group (CSS Color 4 §3.3).
    pub(super) fn new(
        root: &Node,
        pieces: &[InlineBoxPiece],
        cascade: &raikiri_style::CascadeResult,
    ) -> Self {
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
                    // The outermost first letter taken from `::before` or
                    // `::after` text is inside that pseudo-element's box.
                    .or_else(|| {
                        piece
                            .source_owner
                            .filter(|&owner| {
                                owner != piece.node
                                    && raikiri_dom::generated_content::generated_origin(owner)
                                        .is_some()
                            })
                            .map(|owner| key.parent(owner))
                    })
            });
            if let Some((cv, _)) = root.ifc_typographic_fragment(
                piece.node,
                piece.source_container,
                piece.source_owner,
            ) && cv.opacity < 1.0
            {
                opacities.insert(key, cv.opacity);
            } else if let Some(opacity) =
                raikiri_dom::paint_rules::pseudo_opacity_layer(cascade, piece.node)
            {
                opacities.insert(key, opacity);
            }
        }
        // Resolve parents before children. Equal opacities share a group only
        // when their enclosing fragment groups also agree.
        // Allocate groups in fragment paint order so overlapping owner groups
        // with equal insertion positions do not depend on hash iteration.
        for piece in pieces {
            let key = FragmentKey {
                node: piece.node,
                owner: piece.source_owner,
            };
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
                                level: raikiri_dom::paint_rules::pseudo_stack_level(
                                    cascade, key.node,
                                ),
                                parent: group,
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
            // The text of a `::before` or `::after` box is its own.
            .or_else(|| {
                (id == owner)
                    .then(|| {
                        self.nearest_groups.get(&FragmentKey {
                            node: id,
                            owner: None,
                        })
                    })
                    .flatten()
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
            // A group whose pieces are on lines painted elsewhere draws
            // nothing here.
            let mut composite = Scene::new();
            if !content.commands.is_empty() {
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
            }
            // A group composites after the rest of its parent's paint, like
            // the other stacking contexts with `z-index: auto` of the
            // paragraph (CSS Color 4 §3.3; CSS 2.1 Appendix E step 8).
            let parent = group.parent;
            let level = group.level;
            let insertion = self.target(parent).commands.len();
            // Groups at the same place paint by stack level, then in tree
            // order.
            let child = (insertion, level, index, composite);
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

fn merge_children(scene: Scene, mut children: Vec<Child>) -> Scene {
    if children.is_empty() {
        return scene;
    }
    children.sort_by_key(|(position, level, order, _)| (*position, *level, *order));
    let mut children = children.into_iter().peekable();
    let mut result = Scene::new();
    for (index, command) in scene.commands.into_iter().enumerate() {
        while children
            .peek()
            .is_some_and(|(position, _, _, _)| *position == index)
        {
            if let Some((_, _, _, child)) = children.next() {
                result.commands.extend(child.commands);
            }
        }
        result.commands.push(command);
    }
    for (_, _, _, child) in children {
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
