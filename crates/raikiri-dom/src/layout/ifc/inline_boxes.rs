//! Boxes of the inline elements of a paragraph, from the fragments shodo
//! reports for each line.

use super::geometry::IfcAxes;
use super::positioned_runs::Placement;
use raikiri_style::property::Sides;
use shodo::geometry::{LogicalRect, PhysicalSize, WritingMode};
use shodo::{AtomicFragment, Fragment, Line};
use std::collections::HashMap;

/// A physical rectangle in the paragraph's content box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxRect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// The part of an inline element on one line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineBoxPiece {
    /// DOM node id of the element.
    pub node: usize,
    /// Index of the line in the paragraph.
    pub line: usize,
    /// Border box.
    pub border_box: BoxRect,
    /// Content box (without the element's own padding and border).
    pub content_box: BoxRect,
    /// The element starts on this line: its start-side margin, border and
    /// padding are part of the piece.
    pub has_start_edge: bool,
    /// The element ends on this line: its end-side margin, border and padding
    /// are part of the piece.
    pub has_end_edge: bool,
    /// DOM node id of the parent inline element's fragment on this line, if
    /// the parent is itself an inline element of the paragraph.
    pub parent: Option<usize>,
    /// Nearest enclosing ordinary inline box, excluding typographic pseudo
    /// boxes. This identifies the inherited style of a split pseudo fragment.
    pub source_container: Option<usize>,
    /// Original text owner of the first glyph inside this piece, when present.
    pub source_owner: Option<usize>,
}

/// The part of a block-level `::before` or `::after` box between two cuts
/// that pagination makes in its paragraph's lines, in the paragraph's
/// content box with the cut's block offset applied.
///
/// The box is sliced along the block axis (CSS Fragmentation 3
/// `box-decoration-break: slice`): a piece without an edge has no border,
/// padding or corner radius on that side, and its background is laid out
/// over the joined box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneratedBlockPiece {
    /// The inline engine's id of the pseudo-element.
    pub node: usize,
    /// The paragraph line the piece goes with: its first line, or for a
    /// piece without one, the line nearest to it.
    pub line: usize,
    /// Border box of the piece.
    pub border_box: BoxRect,
    /// Content box of the piece.
    pub content_box: BoxRect,
    /// Border box of the whole box, moved as the piece is.
    pub joined_border_box: BoxRect,
    /// Content box of the whole box, moved as the piece is.
    pub joined_content_box: BoxRect,
    /// How far pagination moved the piece along the block axis.
    pub shift: f32,
    /// The piece holds the block-start edge of the box.
    pub has_block_start_edge: bool,
    /// The piece holds the block-end edge of the box.
    pub has_block_end_edge: bool,
}

/// Map a line-local logical rectangle into the root's physical content box.
fn physical(
    rect: LogicalRect,
    line_top: f32,
    axes: IfcAxes,
    content_size: PhysicalSize,
) -> BoxRect {
    let physical = axes.rect(
        content_size,
        LogicalRect {
            block_start: rect.block_start + line_top,
            ..rect
        },
    );
    BoxRect {
        x: physical.x,
        y: physical.y,
        width: physical.width,
        height: physical.height,
    }
}

/// Pieces of every inline element on every line, in the order shodo reports
/// them (an element before its descendants). Each line's used direction
/// controls inline progression inside `content_size`. shodo's inline-box
/// fragments already exclude collapsible spaces hanging at a line end, while
/// preserved spaces remain in their boxes as ink overflow.
///
/// An atomic `::before` or `::after` without content has a piece of its
/// own, a whole box, when `generated_edges` gives the padding and border on
/// its physical sides.
pub(crate) fn inline_box_pieces(
    lines: &[Line],
    writing_mode: WritingMode,
    content_size: PhysicalSize,
    generated_edges: impl Fn(u64) -> Option<Sides<f32>>,
) -> Vec<InlineBoxPiece> {
    let mut pieces = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        let axes = IfcAxes::new(writing_mode, line.used_direction());
        let fragments: Vec<Fragment> = line.fragments().collect();
        let mut source_spans = Vec::new();
        let has_typographic_box = fragments.iter().any(|fragment| {
            let Fragment::InlineBox(piece) = fragment else {
                return false;
            };
            crate::generated_content::generated_origin(piece.node.0 as usize)
                .is_some_and(|(_, pseudo)| pseudo == raikiri_style::PseudoElem::FirstLetter)
        });
        if has_typographic_box {
            source_spans.extend(fragments.iter().filter_map(|fragment| {
                let Fragment::GlyphRun(run) = fragment else {
                    return None;
                };
                let owner = run.node()?.0 as usize;
                Some((
                    run.inline_start(),
                    run.inline_start() + run.inline_size(),
                    owner,
                ))
            }));
            source_spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        let line_start = pieces.len();
        let mut annotations = Vec::new();
        for fragment in &fragments {
            if let Fragment::RubyAnnotation(annotation) = fragment {
                let mut own = Vec::new();
                annotation_pieces(
                    *annotation,
                    Placement::IDENTITY.annotation(annotation.transform(), line.block_offset()),
                    line_index,
                    axes,
                    content_size,
                    &mut own,
                );
                annotations.push((annotation.container().0 as usize, own));
                continue;
            }
            if let Fragment::Atomic(atomic) = fragment {
                if let Some(edges) = generated_edges(atomic.node.0) {
                    pieces.push(generated_atomic_piece(
                        atomic,
                        edges,
                        &fragments,
                        line_index,
                        line.block_offset(),
                        axes,
                        content_size,
                    ));
                }
                continue;
            }
            let Fragment::InlineBox(piece) = fragment else {
                continue;
            };
            let parent = piece.parent.and_then(|index| match fragments.get(index) {
                Some(Fragment::InlineBox(parent)) => Some(parent.node.0 as usize),
                _ => None,
            });
            let mut ancestor = piece.parent;
            let mut source_container = None;
            while let Some(index) = ancestor {
                let Some(Fragment::InlineBox(parent)) = fragments.get(index) else {
                    break; // cov:ignore: shodo's parent indices refer to an earlier InlineBox fragment of the same line.
                };
                let id = parent.node.0 as usize;
                if !crate::generated_content::generated_origin(id)
                    .is_some_and(|(_, pseudo)| pseudo == raikiri_style::PseudoElem::FirstLetter)
                {
                    source_container = Some(id);
                    break;
                }
                ancestor = parent.parent;
            }
            let start = piece.content_rect.inline_start;
            let end = start + piece.content_rect.inline_size;
            let index = source_spans.partition_point(|span| span.0 < start);
            let span = index
                .checked_sub(1)
                .and_then(|index| source_spans.get(index))
                .filter(|span| span.1 > start)
                .or_else(|| source_spans.get(index).filter(|span| span.0 < end));
            let source_owner = span.map(|span| span.2);
            pieces.push(InlineBoxPiece {
                node: piece.node.0 as usize,
                line: line_index,
                border_box: physical(piece.rect, line.block_offset(), axes, content_size),
                content_box: physical(piece.content_rect, line.block_offset(), axes, content_size),
                has_start_edge: piece.has_start_edge,
                has_end_edge: piece.has_end_edge,
                parent,
                source_container,
                source_owner,
            });
        }
        insert_annotation_pieces(&mut pieces, line_start, annotations);
    }
    pieces
}

/// The piece of the atomic `::before` or `::after` placed as `atomic` on
/// the line at `line_top`, whose padding and border on each physical side
/// are `edges`. Its parent is its element's box on the line, if any.
fn generated_atomic_piece(
    atomic: &AtomicFragment,
    edges: Sides<f32>,
    fragments: &[Fragment],
    line: usize,
    line_top: f32,
    axes: IfcAxes,
    content_size: PhysicalSize,
) -> InlineBoxPiece {
    let border_box = physical(atomic.border_rect, line_top, axes, content_size);
    let content_box = BoxRect {
        x: border_box.x + edges.left,
        y: border_box.y + edges.top,
        width: (border_box.width - edges.left - edges.right).max(0.0),
        height: (border_box.height - edges.top - edges.bottom).max(0.0),
    };
    let parent = crate::generated_content::generated_origin(atomic.node.0 as usize)
        .map(|(element, _)| element)
        .filter(|&element| {
            fragments.iter().any(|fragment| {
                matches!(fragment, Fragment::InlineBox(piece) if piece.node.0 as usize == element)
            })
        });
    InlineBoxPiece {
        node: atomic.node.0 as usize,
        line,
        border_box,
        content_box,
        has_start_edge: true,
        has_end_edge: true,
        parent,
        source_container: parent,
        source_owner: None,
    }
}

/// Insert the pieces of each annotation after the pieces of its ruby
/// segment's box and of that box's descendants on the line, which start at
/// `line_start`, so that boxes paint in tree order. shodo reports
/// annotations after every other fragment of the line.
fn insert_annotation_pieces(
    pieces: &mut Vec<InlineBoxPiece>,
    line_start: usize,
    annotations: Vec<(usize, Vec<InlineBoxPiece>)>,
) {
    if annotations.is_empty() {
        return;
    }
    #[cfg(test)]
    let allow_batch = !work::legacy_requested();
    #[cfg(not(test))]
    let allow_batch = true;
    if annotations.len() > 1
        && allow_batch
        && let Some(batch) = annotation_batch::plan(&pieces[line_start..], &annotations)
    {
        batch.insert(pieces, line_start, annotations);
        return;
    }
    #[cfg(test)]
    if !annotations.is_empty() {
        work::record(|work| work.fallback_lines += 1);
    }
    // Where the next annotation of each segment goes.
    let mut ends: HashMap<usize, usize> = HashMap::new();
    for (container, own) in annotations {
        let at = match ends.get(&container) {
            Some(&at) => at,
            None => {
                let line = &pieces[line_start..];
                #[cfg(test)]
                work::record(|work| work.parent_entries += line.len());
                let parents: HashMap<usize, Option<usize>> = line
                    .iter()
                    .map(|piece| (piece.node, piece.parent))
                    .collect();
                // A node can have pieces with different parents (an empty
                // annotation box nests in a box of the same node), so the
                // walk is bounded by the number of nodes on the line.
                let inside = |mut node: Option<usize>| {
                    for _ in 0..=parents.len() {
                        let Some(id) = node else {
                            return false;
                        };
                        if id == container {
                            return true;
                        }
                        node = parents.get(&id).copied().flatten();
                    }
                    false
                };
                match line.iter().position(|piece| piece.node == container) {
                    Some(start) => {
                        let mut end = line_start + start + 1;
                        while end < pieces.len() && inside(pieces[end].parent) {
                            end += 1;
                        }
                        end
                    }
                    None => pieces.len(), // cov:ignore: every ruby segment opens a box keyed by the annotations' container.
                }
            }
        };
        let count = own.len();
        #[cfg(test)]
        work::record(|work| {
            work.moved_pieces += pieces.len() - at;
            work.end_updates += ends.len();
        });
        pieces.splice(at..at, own);
        for end in ends.values_mut() {
            if *end >= at {
                *end += count;
            }
        }
        ends.insert(container, at + count);
    }
}

/// The logical rectangle `rect` of a line that starts `line_top` below its
/// origin, placed in the paragraph by `placement`.
pub(super) fn placed(rect: LogicalRect, line_top: f32, placement: Placement) -> LogicalRect {
    let start = placement.apply(rect.inline_start, rect.block_start + line_top);
    let end = placement.apply(
        rect.inline_start + rect.inline_size,
        rect.block_start + line_top + rect.block_size,
    );
    LogicalRect {
        inline_start: start.0.min(end.0),
        block_start: start.1.min(end.1),
        inline_size: (end.0 - start.0).abs(),
        block_size: (end.1 - start.1).abs(),
    }
}

/// Call `visit` for every atomic inline of `lines`, including those in ruby
/// annotations and the annotations nested in them. It gets the fragment, the
/// block offset of the line the fragment is on, and the placement that maps
/// that line to the paragraph's logical coordinates: the identity for an atomic
/// in the paragraph's own lines.
pub(super) fn for_each_atomic(
    lines: &[Line],
    mut visit: impl FnMut(&AtomicFragment, f32, Placement),
) {
    for line in lines {
        for fragment in line.fragments() {
            match fragment {
                Fragment::Atomic(atomic) => {
                    visit(&atomic, line.block_offset(), Placement::IDENTITY)
                }
                Fragment::RubyAnnotation(annotation) => {
                    let placement =
                        Placement::IDENTITY.annotation(annotation.transform(), line.block_offset());
                    annotation_atomics(annotation, placement, &mut visit);
                }
                _ => {}
            }
        }
    }
}

/// Call `visit` for the atomic inlines of the ruby `annotation`, whose line
/// `placement` maps to the paragraph, and of the annotations nested in it.
fn annotation_atomics(
    annotation: shodo::RubyAnnotationView<'_>,
    placement: Placement,
    visit: &mut impl FnMut(&AtomicFragment, f32, Placement),
) {
    let line = annotation.line();
    for fragment in line.fragments() {
        match fragment {
            Fragment::Atomic(atomic) => visit(&atomic, line.block_offset(), placement),
            Fragment::RubyAnnotation(nested) => annotation_atomics(
                nested,
                placement.annotation(nested.transform(), line.block_offset()),
                visit,
            ),
            _ => {}
        }
    }
}

/// Add the pieces of the inline elements in the ruby `annotation`, whose
/// line `placement` maps to the paragraph, and in the annotations nested in
/// it. They belong to the paragraph's line `line_index`.
fn annotation_pieces(
    annotation: shodo::RubyAnnotationView<'_>,
    placement: Placement,
    line_index: usize,
    axes: IfcAxes,
    content_size: PhysicalSize,
    pieces: &mut Vec<InlineBoxPiece>,
) {
    let line = annotation.line();
    let fragments: Vec<Fragment> = line.fragments().collect();
    let line_start = pieces.len();
    let mut nested_annotations = Vec::new();
    for fragment in &fragments {
        match fragment {
            Fragment::InlineBox(piece) => {
                let parent = piece.parent.and_then(|index| match fragments.get(index) {
                    Some(Fragment::InlineBox(parent)) => Some(parent.node.0 as usize),
                    _ => None, // cov:ignore: shodo's parent indices refer to an earlier InlineBox fragment of the same line.
                });
                let rect = |rect| {
                    physical(
                        placed(rect, line.block_offset(), placement),
                        0.0,
                        axes,
                        content_size,
                    )
                };
                pieces.push(InlineBoxPiece {
                    node: piece.node.0 as usize,
                    line: line_index,
                    border_box: rect(piece.rect),
                    content_box: rect(piece.content_rect),
                    has_start_edge: piece.has_start_edge,
                    has_end_edge: piece.has_end_edge,
                    parent,
                    source_container: parent,
                    source_owner: None,
                });
            }
            Fragment::RubyAnnotation(nested) => {
                let mut own = Vec::new();
                annotation_pieces(
                    *nested,
                    placement.annotation(nested.transform(), line.block_offset()),
                    line_index,
                    axes,
                    content_size,
                    &mut own,
                );
                nested_annotations.push((nested.container().0 as usize, own));
            }
            _ => {}
        }
    }
    insert_annotation_pieces(pieces, line_start, nested_annotations);
}

/// The node of each line's ending forced break, with a box that spans the
/// line's block extent at its inline start and has no inline size. A `<br>`
/// takes part in the paragraph as a break, not as a box of its own; this is
/// where it is placed.
pub(crate) fn forced_break_boxes(
    lines: &[Line],
    writing_mode: WritingMode,
    content_size: PhysicalSize,
) -> Vec<(usize, BoxRect)> {
    lines
        .iter()
        .filter_map(|line| {
            let node = line.forced_break()?.node.0 as usize;
            let axes = IfcAxes::new(writing_mode, line.used_direction());
            let rect = LogicalRect {
                inline_start: 0.0,
                block_start: 0.0,
                inline_size: 0.0,
                block_size: line.block_size(),
            };
            Some((
                node,
                physical(rect, line.block_offset(), axes, content_size),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod work;

mod annotation_batch;
