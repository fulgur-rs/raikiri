//! Boxes of the inline elements of a paragraph, from the fragments shodo
//! reports for each line.

use super::geometry::IfcAxes;
use shodo::geometry::{LogicalRect, PhysicalSize, WritingMode};
use shodo::{Fragment, Line};

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
pub(crate) fn inline_box_pieces(
    lines: &[Line],
    writing_mode: WritingMode,
    content_size: PhysicalSize,
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
        for fragment in &fragments {
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
                    break;
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
    }
    pieces
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
