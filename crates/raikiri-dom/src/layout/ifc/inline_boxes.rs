//! Boxes of the inline elements of a paragraph, from the fragments shodo
//! reports for each line.

use shodo::geometry::LogicalRect;
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
}

/// A logical rectangle of a line as a physical one. `line_top` is the line's
/// block offset in the paragraph; a right to left line grows from the right
/// edge, so the inline axis is mirrored inside `content_width`.
fn physical(rect: LogicalRect, line_top: f32, content_width: f32, rtl: bool) -> BoxRect {
    let x = if rtl {
        content_width - (rect.inline_start + rect.inline_size)
    } else {
        rect.inline_start
    };
    BoxRect {
        x,
        y: line_top + rect.block_start,
        width: rect.inline_size,
        height: rect.block_size,
    }
}

/// Pieces of every inline element on every line, in the order shodo reports
/// them (an element before its descendants). `content_width` is the width the
/// lines were broken at; right to left lines are mirrored inside it. shodo's
/// inline-box fragments already exclude collapsible spaces hanging at a line
/// end, while preserved spaces remain in their boxes as ink overflow.
pub(crate) fn inline_box_pieces(
    lines: &[Line],
    content_width: f32,
    rtl: bool,
) -> Vec<InlineBoxPiece> {
    let mut pieces = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        let fragments: Vec<Fragment> = line.fragments().collect();
        for fragment in &fragments {
            let Fragment::InlineBox(piece) = fragment else {
                continue;
            };
            let parent = piece.parent.and_then(|index| match fragments.get(index) {
                Some(Fragment::InlineBox(parent)) => Some(parent.node.0 as usize),
                _ => None,
            });
            pieces.push(InlineBoxPiece {
                node: piece.node.0 as usize,
                line: line_index,
                border_box: physical(piece.rect, line.block_offset(), content_width, rtl),
                content_box: physical(piece.content_rect, line.block_offset(), content_width, rtl),
                has_start_edge: piece.has_start_edge,
                has_end_edge: piece.has_end_edge,
                parent,
            });
        }
    }
    pieces
}

#[cfg(test)]
mod tests;
