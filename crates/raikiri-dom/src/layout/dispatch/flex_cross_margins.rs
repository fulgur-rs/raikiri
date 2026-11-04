//! Resolve overflowing cross-axis auto margins without changing line origins.

use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutInput, LayoutOutput, NodeId};
use crate::layout::style_views::ReadRole;
use crate::layout::taffy_adapter::algorithms::FlexReplayCall;
use raikiri_layout_geometry::{BoxLayout, Edges, Point};
use raikiri_layout_style::values::{Direction, LengthPercentageAuto, Position};

/// CSS Flexbox 1 section 9.6 keeps the logical start margin at zero when
/// auto margins cannot absorb positive room. The opposite margin preserves
/// the line's outer cross size, even when that margin was not auto.
/// Shared with read-only baseline probes so measurement matches placement.
pub(in crate::layout) fn corrected(
    mut layout: BoxLayout,
    authored: Edges<LengthPercentageAuto>,
    row: bool,
    rtl: bool,
) -> Option<BoxLayout> {
    let (start, end, start_auto, end_auto) = if row {
        (
            layout.margin.top,
            layout.margin.bottom,
            authored.top.is_auto(),
            authored.bottom.is_auto(),
        )
    } else {
        (
            layout.margin.left,
            layout.margin.right,
            authored.left.is_auto(),
            authored.right.is_auto(),
        )
    };
    if !(start_auto && start < 0.0 || end_auto && end < 0.0) {
        return None;
    }
    let sum = start + end;
    let logical_start_is_end = !row && rtl;
    let (new_start, new_end) = if logical_start_is_end {
        let end = if end_auto { 0.0 } else { end };
        (sum - end, end)
    } else {
        let start = if start_auto { 0.0 } else { start };
        (start, sum - start)
    };
    if row {
        layout.location.y += new_start - start;
        layout.margin.top = new_start;
        layout.margin.bottom = new_end;
    } else {
        layout.location.x += new_start - start;
        layout.margin.left = new_start;
        layout.margin.right = new_end;
    }
    Some(layout)
}

pub(super) fn normalize(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    calls: Option<&[FlexReplayCall]>,
    output: &mut LayoutOutput,
) {
    let parent = tree.query_style(idx, ReadRole::FlexContainer);
    let row = parent.flex_direction_is_row();
    let rtl = parent.direction() == Direction::Rtl;
    let items: Vec<_> = super::TaffyChildIter::new_for_session(tree, idx)
        .filter(|&child| {
            !tree.nodes[child].is_display_none()
                && tree.nodes[child].layout_position() != Position::Absolute
        })
        .collect();
    let mut layouts = Vec::new();
    let mut shifts = Vec::new();
    for &child in &items {
        let old = tree.nodes[child].layout;
        let style = tree.query_style(child, ReadRole::FlexItem);
        if let Some(layout) = corrected(old, style.margin(), row, rtl) {
            layouts.push((child, layout));
            let shift = Point {
                x: layout.location.x - old.location.x,
                y: layout.location.y - old.location.y,
            };
            if shift != Point::ZERO {
                shifts.push((child, shift));
            }
        }
    }
    if row && output.baselines.first.is_some() && !shifts.is_empty() {
        super::table_items::shift_first_baseline(
            tree,
            idx,
            inputs,
            calls.expect("rows with cross auto margins record their callbacks"),
            &shifts,
            output,
        );
    }
    for (child, layout) in layouts {
        tree.set_unrounded_layout(NodeId::from(child), &layout);
    }
    super::table_items::finish(tree, idx, inputs, &items, &shifts, output);
}

#[cfg(test)]
mod tests;
