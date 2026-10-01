//! Measure and break an ifc root's paragraph.

use super::root::{IfcLines, IfcRoot};
use crate::Document;
use raikiri_style::{CascadeResult, ComputedTextIndent};
use raikiri_traits::NodeKind;
use shodo::geometry::BaselineKind;
use shodo::style::LineOptions;
use shodo::{AtomicIntrinsics, AtomicSizes, LayoutContext, LineConstraint, LineResult, Paragraph};
use taffy::{BlockContext, Clear};

/// Horizontal space of one line, in content-box coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LineSpace {
    /// Offset of the line's start from the content-box start.
    pub(crate) start: f32,
    pub(crate) width: f32,
}

/// What a layout needs to know about the root's box.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FlowGeometry {
    /// Content-box width the lines are broken at.
    pub(crate) width: f32,
    /// Border plus padding on the physical left and right side.
    pub(crate) edges: (f32, f32),
    /// Border plus padding above the content box. A `BlockContext` measures
    /// block offsets from the block's border-box top, so a block offset inside
    /// the content box needs this added.
    pub(crate) top_edge: f32,
}

/// How many times a line is laid out again because the space turned out to
/// differ for the height it took.
pub(crate) const MAX_SPACE_RETRIES: usize = 4;

pub(crate) fn resolve_indent(indent: ComputedTextIndent, width: f32) -> f32 {
    match indent {
        ComputedTextIndent::Px(px) => px,
        ComputedTextIndent::Percent(percent) => percent / 100.0 * width,
        ComputedTextIndent::Calc(calc) => calc.percent / 100.0 * width + calc.px,
    }
}

/// Min- and max-content widths of a paragraph without boxes.
#[cfg(test)]
pub(crate) fn intrinsic_widths(root: &IfcRoot, cx: &mut LayoutContext) -> (f32, f32) {
    intrinsic_widths_with(root, cx, &AtomicIntrinsics::EMPTY)
}

/// Min- and max-content widths of the paragraph with the intrinsic widths of
/// its boxes.
pub(crate) fn intrinsic_widths_with(
    root: &IfcRoot,
    cx: &mut LayoutContext,
    boxes: &AtomicIntrinsics,
) -> (f32, f32) {
    // The indent needs the width it is resolved against, which intrinsic
    // sizing does not have; it contributes only when it is a plain length.
    let mut options = root.options;
    if let ComputedTextIndent::Px(px) = root.indent {
        options.text_indent.length = px;
    }
    let sizes = root.paragraph.intrinsic_sizes(cx, &options, boxes);
    (sizes.min_content, sizes.max_content)
}

/// Space of a line at block offset `y` that is `height` tall, from the float
/// segments it spans.
///
/// `content_left` and `content_top` are the distances from the context's
/// border-box corner to the content-box corner (border plus padding). The
/// result is relative to the content-box start and never wider than
/// `content_width`.
pub(crate) fn line_space(
    ctx: &BlockContext<'_>,
    content_left: f32,
    content_top: f32,
    content_width: f32,
    y: f32,
    height: f32,
) -> LineSpace {
    let y = content_top + y;
    let mut start = 0.0_f32;
    let mut end = content_width;
    let mut slot = ctx.find_content_slot(y, Clear::None, None);
    loop {
        start = start.max(slot.x - content_left);
        end = end.min(slot.x - content_left + slot.width);
        // Without an active float the slot has no segment to continue from.
        let Some(segment) = slot.segment_id else {
            break;
        };
        // The next segment narrows the line only if it starts before the
        // line ends. A slot that does not advance ends the walk.
        let next = ctx.find_content_slot(y, Clear::None, Some(segment));
        if next
            .segment_id
            .is_none_or(|next_segment| next_segment <= segment)
            || next.y >= y + height
        {
            break;
        }
        slot = next;
    }
    LineSpace {
        start,
        width: (end - start).max(0.0),
    }
}

pub(crate) fn break_lines(root: &IfcRoot, cx: &mut LayoutContext, width: f32) -> IfcLines {
    let mut options = root.options;
    options.text_indent.length = resolve_indent(root.indent, width);
    let mut placed = place_lines(
        &root.paragraph,
        &options,
        cx,
        &AtomicSizes::EMPTY,
        |_, _| LineSpace { start: 0.0, width },
    );
    placed.width = width;
    placed
}

/// Lay the paragraph out line by line. `space(y, height)` gives the space
/// available to a line that starts at block offset `y` and is `height` tall.
pub(crate) fn place_lines(
    paragraph: &Paragraph,
    options: &LineOptions,
    cx: &mut LayoutContext,
    atomics: &AtomicSizes,
    mut space: impl FnMut(f32, f32) -> LineSpace,
) -> IfcLines {
    let mut lines: Vec<shodo::Line> = Vec::new();
    let mut token = paragraph.start_token();
    let mut y = 0.0_f32;
    let mut first_width = None;
    loop {
        let mut assumed_height = 0.0_f32;
        let mut retries = 0;
        let line = loop {
            let available = space(y, assumed_height);
            first_width.get_or_insert(available.width);
            let mut constraint = LineConstraint::new(available.width);
            constraint.inline_start_offset = available.start;
            constraint.block_offset = y;
            match paragraph.next_line(cx, token, options, &constraint, atomics) {
                LineResult::Line(line) => {
                    let height = line.block_size();
                    // The line may span more of a float than the height it
                    // was assumed to have; lay it out again if the space for
                    // its real height differs.
                    if retries < MAX_SPACE_RETRIES && space(y, height) != available {
                        assumed_height = height;
                        retries += 1;
                        continue;
                    }
                    break Some(line);
                }
                LineResult::Done => break None,
                other => {
                    // Floats and blocks inside the paragraph are not
                    // projected here, so no other result is expected.
                    debug_assert!(false, "unexpected line result: {other:?}");
                    break None;
                }
            }
        };
        let Some(line) = line else { break };
        y += line.block_size();
        token = line.break_token();
        lines.push(line);
    }
    IfcLines {
        width: first_width.unwrap_or(0.0),
        height: lines.iter().map(|line| line.block_size()).sum(),
        lines: std::sync::Arc::new(lines),
        beside_floats: false,
        escaping_margin: taffy::CollapsibleMarginSet::ZERO,
    }
}

pub(crate) fn first_baseline(lines: &IfcLines) -> Option<f32> {
    lines
        .lines
        .first()
        .map(|line| line.block_offset() + line.baseline(BaselineKind::Alphabetic))
}

pub(crate) fn last_baseline(lines: &IfcLines) -> Option<f32> {
    // `Line::baseline` is relative to the line; the block offset places it in
    // the paragraph.
    lines
        .lines
        .last()
        .map(|line| line.block_offset() + line.baseline(BaselineKind::Alphabetic))
}

/// Break every ifc root again at the width its text would be re-shaped at on
/// the parley path: the content width of the nearest authored-width ancestor
/// of its text, else `max_advance`. Box geometry is not touched.
pub(crate) fn rebreak_roots(doc: &mut Document, cascade: &CascadeResult, max_advance: f32) {
    let Some(mut state) = doc.ifc.take() else {
        return;
    };
    let parent_of: Vec<Option<usize>> = (0..doc.nodes.len()).map(|id| doc.parent_of(id)).collect();
    let roots: Vec<usize> = (0..doc.nodes.len())
        .filter(|&id| doc.nodes[id].ifc.is_some())
        .collect();
    for id in roots {
        // The parley rule starts at the parent of the text, so the root's
        // first text node is the starting point.
        let width = first_text_node(doc, id)
            .and_then(|text| {
                crate::layout::authored_containing_width_with_resolved_ch(
                    doc,
                    cascade,
                    &parent_of,
                    text,
                    max_advance,
                )
            })
            .unwrap_or(max_advance);
        let Some(root) = doc.nodes[id].ifc.as_mut() else {
            continue;
        };
        // Lines laid out beside floats depend on the float context of the
        // performed layout, which is gone here; breaking them again at the
        // full width would run them under the floats. The positions of a
        // root's own boxes are tied to the lines they were placed with, and
        // lines split in columns to the column width.
        if !root.boxes.is_empty()
            || root.multicol_fragments.is_some()
            || root.lines.as_ref().is_some_and(|lines| lines.beside_floats)
        {
            continue;
        }
        let lines = break_lines(root, &mut state.layout_cx, width);
        let ifc = root.without_lines();
        root.lines = Some(lines);
        // The inline elements follow the new lines; the root's box keeps the
        // border and padding it was laid out with.
        let layout = doc.nodes[id].unrounded_layout;
        let geometry = FlowGeometry {
            width,
            edges: (
                layout.padding.left + layout.border.left,
                layout.padding.right + layout.border.right,
            ),
            top_edge: layout.padding.top + layout.border.top,
        };
        let lines = doc.nodes[id]
            .ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
            .map(|lines| std::sync::Arc::clone(&lines.lines));
        if let Some(lines) = lines {
            super::records::record_inline_boxes(doc, id, &ifc, &lines, &geometry);
        }
    }
    doc.ifc = Some(state);
}

/// The first in-document text node under `root`, in document order.
fn first_text_node(doc: &Document, root: usize) -> Option<usize> {
    let mut stack: Vec<usize> = doc.nodes[root].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let node = &doc.nodes[id];
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Text => return Some(id),
            NodeKind::Element => stack.extend(node.children.iter().rev().copied()),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests;
