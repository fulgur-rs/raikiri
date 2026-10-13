//! Composite the complete paint of a translucent inline element or
//! typographic pseudo as one opacity group.

use anyrender::{PaintScene, Scene};
use kurbo::{Affine, Rect};
use peniko::Mix;
use raikiri_dom::{Document, InlineBoxPiece, Node, NodeKind};
use raikiri_style::PseudoElem;
use raikiri_style::property::{DisplayValue, PositionValue, ZIndexValue};
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
    /// The element or typographic box whose paint the group composites.
    owner: usize,
    /// The owner's own boxes on every line, painted below the rest of the
    /// group and the groups with negative stack levels nested in it.
    boxes: Scene,
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

/// The paragraph whose groups are painted.
#[derive(Clone, Copy)]
pub(crate) struct Paragraph<'a> {
    pub(crate) document: &'a Document,
    pub(crate) cascade: &'a raikiri_style::CascadeResult,
    pub(crate) root_id: usize,
}

pub(super) struct TypographicPaint {
    normal: Scene,
    groups: Vec<Group>,
    nearest_groups: HashMap<FragmentKey, Option<usize>>,
    piece_groups: HashMap<FragmentKey, Option<usize>>,
    /// The innermost group of a translucent inline element around each
    /// element of the paragraph, the element included.
    element_groups: HashMap<usize, Option<usize>>,
}

impl TypographicPaint {
    /// A non-atomic inline element with an opacity below one composites its
    /// fragments on the paragraph's lines as one group, and so do the
    /// `::before`, `::after` and `::first-letter` boxes (CSS Color 4 §3.3).
    /// The groups nest as their boxes do.
    pub(super) fn new(paragraph: Paragraph<'_>, root: &Node, pieces: &[InlineBoxPiece]) -> Self {
        let cascade = paragraph.cascade;
        let mut paint = Self {
            normal: Scene::new(),
            groups: Vec::new(),
            nearest_groups: HashMap::new(),
            piece_groups: HashMap::new(),
            element_groups: HashMap::new(),
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
                    // The outermost typographic box is inside the inline
                    // elements around it.
                    None => {
                        break enclosing_element(paragraph.document, next)
                            .and_then(|element| paint.element_group(paragraph, element));
                    }
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
                                owner: key.node,
                                boxes: Scene::new(),
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

    /// The innermost group of a translucent inline element of `paragraph`
    /// around `id`, an element or generated box, `id` included. The walk up
    /// stops at the paragraph root and at any box that is not an inline
    /// element. Elements with `display: contents` form no group.
    pub(super) fn element_group(&mut self, paragraph: Paragraph<'_>, id: usize) -> Option<usize> {
        let start =
            raikiri_dom::generated_content::generated_origin(id).map_or(id, |(element, _)| element);
        let mut path = Vec::new();
        let mut group = None;
        for (id, opacity) in inline_ancestors(paragraph, start) {
            if let Some(&cached) = self.element_groups.get(&id) {
                group = cached;
                break;
            }
            path.push((id, opacity));
        }
        // Allocate the outer groups first, in the order their elements are
        // first painted.
        for (id, opacity) in path.into_iter().rev() {
            if let Some(opacity) = opacity {
                let index = self.groups.len();
                self.groups.push(Group {
                    scene: Scene::new(),
                    opacity,
                    level: element_stack_level(paragraph.cascade, id),
                    parent: group,
                    children: Vec::new(),
                    owner: id,
                    boxes: Scene::new(),
                });
                group = Some(index);
            }
            self.element_groups.insert(id, group);
        }
        group
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

    /// The scene the box of `piece` paints into: its own group's boxes when
    /// it is that group's owner, else the rest of `group`.
    pub(super) fn box_target(
        &mut self,
        group: Option<usize>,
        piece: &InlineBoxPiece,
    ) -> &mut Scene {
        match group {
            Some(index) if self.groups[index].owner == piece.node => &mut self.groups[index].boxes,
            _ => self.target(group),
        }
    }

    pub(super) fn target(&mut self, group: Option<usize>) -> &mut Scene {
        match group {
            Some(index) => &mut self.groups[index].scene,
            None => &mut self.normal,
        }
    }

    /// Composite the groups into `scene`. `first_line` is the paint of the
    /// paragraph's first line when a translucent `::first-line` wraps it,
    /// with that opacity: a group composited before the others of the
    /// paragraph, as it comes first in tree order.
    pub(super) fn finish(
        mut self,
        scene: &mut impl PaintScene,
        clip: Rect,
        paint_transform: Affine,
        first_line: Option<(Self, f32)>,
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
        let mut roots = Vec::new();
        if let Some((first_line, opacity)) = first_line {
            let mut content = Scene::new();
            first_line.finish(&mut content, clip, paint_transform, None);
            if !content.commands.is_empty() {
                let mut composite = Scene::new();
                composite.push_layer(Mix::Normal, opacity, clip_transform, &clip, None, None);
                composite.append_scene(content, Affine::IDENTITY);
                composite.pop_layer();
                roots.push((self.normal.commands.len(), 0, 0, composite));
            }
        }
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
        while let Some(index) = order.pop() {
            let group = &mut self.groups[index];
            let mut content = std::mem::take(&mut group.boxes);
            content.append_scene(
                merge_children(
                    std::mem::take(&mut group.scene),
                    std::mem::take(&mut group.children),
                ),
                Affine::IDENTITY,
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
            // paragraph (CSS Color 4 §3.3; CSS 2.1 Appendix E step 8). One
            // with a negative stack level goes below that paint, above only
            // the parent's own boxes (Appendix E step 3).
            let parent = group.parent;
            let level = group.level;
            let insertion = if level < 0 {
                0
            } else {
                self.target(parent).commands.len()
            };
            // Groups at the same place paint by stack level, then in tree
            // order, after a first line's group.
            let child = (insertion, level, index + 1, composite);
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

/// The stack level an inline element's group composites at among the other
/// groups of its paragraph: the `z-index` of a positioned element, and 0
/// otherwise (CSS 2.1 §9.9.1).
fn element_stack_level(cascade: &raikiri_style::CascadeResult, id: usize) -> i32 {
    cascade
        .computed
        .get(id)
        .map_or(0, |cv| match (&cv.position, cv.z_index) {
            (PositionValue::Static, _) => 0,
            (_, ZIndexValue::Integer(level)) => level,
            _ => 0,
        })
}

/// `start` and the inline elements of `paragraph` around it, innermost
/// first, with their group opacities. The walk stops at the paragraph root
/// and at any box that is not an inline element.
fn inline_ancestors(
    paragraph: Paragraph<'_>,
    start: usize,
) -> impl Iterator<Item = (usize, Option<f32>)> + '_ {
    let source_owner = paragraph.document.ifc_source_owner(paragraph.root_id);
    std::iter::successors(Some(start), move |&id| paragraph.document.parent_of(id))
        .take_while(move |&id| id != paragraph.root_id && id != source_owner)
        .map_while(move |id| {
            let node = paragraph.document.get_node(id)?;
            let cv = paragraph.cascade.computed.get(id)?;
            (node.kind() == NodeKind::Element
                && matches!(cv.display, DisplayValue::Inline | DisplayValue::Contents))
            .then(|| (id, raikiri_dom::paint_rules::opacity_layer(cv)))
        })
}

/// The opacities of the translucent inline elements of `paragraph` around
/// `id`, a box laid out beside its lines, outermost first.
pub(crate) fn enclosing_inline_opacities(paragraph: Paragraph<'_>, id: usize) -> Vec<(usize, f32)> {
    let mut opacities: Vec<_> = paragraph
        .document
        .parent_of(id)
        .into_iter()
        .flat_map(|parent| inline_ancestors(paragraph, parent))
        .filter_map(|(id, opacity)| opacity.map(|opacity| (id, opacity)))
        .collect();
    opacities.reverse();
    opacities
}

/// The element whose inline groups enclose the outermost typographic box
/// `key`: the element itself, the originating element of a `::before` or
/// `::after`, or the element whose text a `::first-letter` wraps.
fn enclosing_element(document: &Document, key: FragmentKey) -> Option<usize> {
    let Some((element, pseudo)) = raikiri_dom::generated_content::generated_origin(key.node) else {
        return Some(key.node);
    };
    // Letters taken from generated text are inside their pseudo-element's
    // box, which `key` then has as its parent.
    if pseudo == PseudoElem::FirstLetter
        && let Some(owner) = key.owner
        && document
            .get_node(owner)
            .is_some_and(|node| node.kind() == NodeKind::Text)
    {
        return document.parent_of(owner);
    }
    Some(element)
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
