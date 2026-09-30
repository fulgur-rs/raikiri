//! Measure and break an ifc root's paragraph.

use super::root::{IfcLines, IfcRoot};
use raikiri_style::ComputedTextIndent;
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

#[cfg(test)]
mod tests;
