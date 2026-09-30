//! Measure and break an ifc root's paragraph.

use super::root::{IfcLines, IfcRoot};
use crate::Document;
use raikiri_style::{CascadeResult, ComputedTextIndent};
use raikiri_traits::NodeKind;
use shodo::geometry::BaselineKind;
use shodo::{AtomicIntrinsics, AtomicSizes, LayoutContext};

pub(crate) fn resolve_indent(indent: ComputedTextIndent, width: f32) -> f32 {
    match indent {
        ComputedTextIndent::Px(px) => px,
        ComputedTextIndent::Percent(percent) => percent / 100.0 * width,
        ComputedTextIndent::Calc(calc) => calc.percent / 100.0 * width + calc.px,
    }
}

pub(crate) fn intrinsic_widths(root: &IfcRoot, cx: &mut LayoutContext) -> (f32, f32) {
    // The indent needs the width it is resolved against, which intrinsic
    // sizing does not have; it contributes only when it is a plain length.
    let mut options = root.options;
    if let ComputedTextIndent::Px(px) = root.indent {
        options.text_indent.length = px;
    }
    let sizes = root
        .paragraph
        .intrinsic_sizes(cx, &options, &AtomicIntrinsics::EMPTY);
    (sizes.min_content, sizes.max_content)
}

pub(crate) fn break_lines(root: &IfcRoot, cx: &mut LayoutContext, width: f32) -> IfcLines {
    let mut options = root.options;
    options.text_indent.length = resolve_indent(root.indent, width);
    let lines = root
        .paragraph
        .break_all(cx, &options, width, &AtomicSizes::EMPTY);
    let height = lines.iter().map(|line| line.block_size()).sum();
    IfcLines {
        width,
        lines,
        height,
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
        root.lines = Some(break_lines(root, &mut state.layout_cx, width));
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
