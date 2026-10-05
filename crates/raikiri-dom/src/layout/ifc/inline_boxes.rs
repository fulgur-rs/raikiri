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
                border_box: physical(piece.rect, line.block_offset(), axes, content_size),
                content_box: physical(piece.content_rect, line.block_offset(), axes, content_size),
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
