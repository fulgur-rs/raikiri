use super::{BoxRect, Fragment, InlineBoxPiece, ParentLine, PieceContext, physical, placed};
use crate::layout::ifc::ruby::{ContainerOwner, RubyContainers};
use std::collections::HashMap;

pub(super) fn owner(
    annotation: shodo::RubyAnnotationView<'_>,
    fragments: &[Fragment<'_>],
    containers: &RubyContainers,
) -> Option<usize> {
    if containers.is_empty() {
        return None;
    }
    let sources = fragments
        .iter()
        .filter_map(|fragment| match fragment {
            Fragment::GlyphRun(run) => run.node().map(|node| node.0 as usize),
            Fragment::Atomic(atomic) => Some(atomic.node.0 as usize),
            // The automatic root may reuse a base's node ID. Its parent, not
            // its ID, distinguishes it from real content with that same ID.
            Fragment::InlineBox(piece) if piece.parent.is_some() => Some(piece.node.0 as usize),
            _ => None,
        })
        .chain(
            annotation
                .line()
                .forced_break()
                .map(|br| br.node.0 as usize),
        );
    match containers.resolve(sources, annotation.node().map(|node| node.0 as usize)) {
        ContainerOwner::Container(owner) => Some(owner),
        _ => None,
    }
}

pub(super) fn union(old: BoxRect, rect: BoxRect) -> BoxRect {
    let x = old.x.min(rect.x);
    let y = old.y.min(rect.y);
    BoxRect {
        x,
        y,
        width: (old.x + old.width).max(rect.x + rect.width) - x,
        height: (old.y + old.height).max(rect.y + rect.height) - y,
    }
}

pub(super) fn include(bounds: &mut Option<BoxRect>, rect: BoxRect) {
    *bounds = Some(bounds.map_or(rect, |old| union(old, rect)));
}

pub(super) type BaseBoxes = HashMap<(usize, Option<usize>), BoxRect>;

pub(super) fn base_boxes(
    fragments: &[Fragment<'_>],
    placement: super::Placement,
    offset: f32,
    context: PieceContext<'_>,
) -> BaseBoxes {
    let mut boxes = BaseBoxes::new();
    if context.containers.is_empty() {
        return boxes;
    }
    for fragment in fragments {
        #[cfg(test)]
        super::work::record(|work| work.inspected += 1);
        let Fragment::InlineBox(piece) = fragment else {
            continue;
        };
        let parent = piece.parent.and_then(|index| match fragments.get(index) {
            Some(Fragment::InlineBox(parent)) => Some(parent.node.0 as usize),
            _ => None,
        });
        let rect = physical(
            placed(piece.content_rect, offset, placement),
            0.0,
            context.axes,
            context.content_size,
        );
        boxes
            .entry((piece.node.0 as usize, parent))
            .and_modify(|old| *old = union(*old, rect))
            .or_insert(rect);
    }
    boxes
}

pub(super) fn base_box(
    annotation: shodo::RubyAnnotationView<'_>,
    owner: usize,
    parent: ParentLine<'_>,
    context: PieceContext<'_>,
) -> Option<BoxRect> {
    parent
        .bases
        .get(&(
            annotation.container().0 as usize,
            context.containers.parent(owner),
        ))
        .copied()
}

pub(super) fn coalesce(
    pieces: &mut Vec<InlineBoxPiece>,
    line_start: usize,
    containers: &RubyContainers,
) {
    if containers.is_empty() {
        return;
    }
    let line = pieces.split_off(line_start);
    let mut first: HashMap<usize, usize> = HashMap::new();
    for piece in line {
        if containers.is_container(piece.node) {
            if let Some(&index) = first.get(&piece.node) {
                let first = &mut pieces[index];
                first.border_box = union(first.border_box, piece.border_box);
                first.content_box = first.border_box;
                continue;
            }
            first.insert(piece.node, pieces.len());
        }
        pieces.push(piece);
    }
}
