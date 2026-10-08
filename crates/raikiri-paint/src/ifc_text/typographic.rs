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
    by_box: HashMap<usize, usize>,
    parents: HashMap<usize, Option<usize>>,
}

impl TypographicPaint {
    pub(super) fn new(root: Option<&Node>, pieces: &[InlineBoxPiece]) -> Self {
        let mut paint = Self {
            normal: Scene::new(),
            groups: Vec::new(),
            by_box: HashMap::new(),
            parents: pieces
                .iter()
                .map(|piece| (piece.node, piece.parent))
                .collect(),
        };
        if let Some(root) = root {
            for piece in pieces {
                if paint.by_box.contains_key(&piece.node) {
                    continue;
                }
                if let Some((cv, _)) = root.ifc_typographic_fragment(
                    piece.node,
                    piece.source_container,
                    piece.source_owner,
                ) && cv.opacity < 1.0
                {
                    paint.by_box.insert(piece.node, paint.groups.len());
                    paint.groups.push(Group {
                        scene: Scene::new(),
                        opacity: cv.opacity,
                        parent: None,
                        insertion: None,
                        children: Vec::new(),
                    });
                }
            }
            for (&id, &index) in &paint.by_box {
                paint.groups[index].parent = paint.nearest(root, root.ifc_typographic_parent(id));
            }
        }
        paint
    }

    pub(super) fn nearest(&self, root: &Node, mut id: Option<usize>) -> Option<usize> {
        while let Some(current) = id {
            if let Some(&index) = self.by_box.get(&current) {
                return Some(index);
            }
            id = root.ifc_typographic_parent(current);
        }
        None
    }

    pub(super) fn piece_group(&self, root: Option<&Node>, piece: &InlineBoxPiece) -> Option<usize> {
        let root = root?;
        if let Some(group) = self.nearest(root, Some(piece.node)) {
            return Some(group);
        }
        let mut parent = piece.parent;
        while let Some(id) = parent {
            if let Some(group) = self.nearest(root, Some(id)) {
                return Some(group);
            }
            parent = self.parents.get(&id).copied().flatten();
        }
        None
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
            if self.groups[index].insertion.is_none() {
                self.groups[index].insertion = Some(self.target(parent).commands.len());
            }
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
        let mut order: Vec<_> = (0..self.groups.len()).collect();
        order.sort_by_key(|&index| {
            let mut depth = 0;
            let mut parent = self.groups[index].parent;
            while let Some(index) = parent {
                depth += 1;
                parent = self.groups[index].parent;
            }
            std::cmp::Reverse(depth)
        });
        let mut roots = Vec::new();
        for index in order {
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
                Some(parent) => self.groups[parent].children.push(child),
                None => roots.push(child),
            }
        }
        scene.append_scene(merge_children(self.normal, roots), Affine::IDENTITY);
    }
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
