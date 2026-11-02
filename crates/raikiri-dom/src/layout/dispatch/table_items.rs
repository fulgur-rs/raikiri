//! Retain the size that collapsed rows leave in a stretched table item.
//!
//! Authored heights are measured by the table itself. This pass only handles
//! external stretch assignments which still overwrite the returned grid size
//! or leave a reversed flex cross-start at the old position.

use std::collections::HashMap;

use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutInput, LayoutOutput, NodeId, RunMode};
use crate::layout::style_views::ReadRole;
use crate::layout::taffy_adapter::algorithms::{FlexReplayCall, flex_baseline_source};
use crate::node::FormattingContext;
use raikiri_layout_geometry::{Edges, Point, Size};
use raikiri_layout_style::values::{AlignItemsKeyword, Direction, Position, ResolveOrZero};

#[derive(Debug)]
pub(in crate::layout) struct Capture {
    container: usize,
    sizes: HashMap<usize, Measured>,
    calls: Option<Vec<FlexReplayCall>>,
}

#[derive(Debug, Clone, Copy)]
struct Measured {
    size: Size<f32>,
    offered: Size<Option<f32>>,
}

pub(super) fn record(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    output: &LayoutOutput,
) {
    let Some(parent) = tree
        .context
        .table_item_captures
        .last()
        .map(|capture| capture.container)
    else {
        return;
    };
    let idx = usize::from(node);
    if tree.taffy_parent_of(idx) != Some(parent) {
        return;
    }
    let table = tree.nodes[idx].formatting_context == FormattingContext::Table
        && tree.nodes[idx].layout_position() != Position::Absolute;
    let capture = tree
        .context
        .table_item_captures
        .last_mut()
        .expect("table capture remains active");
    if let Some(calls) = &mut capture.calls {
        calls.push(FlexReplayCall {
            node,
            inputs,
            output: output.clone(),
        });
    }
    if table && inputs.run_mode == RunMode::PerformLayout {
        capture.sizes.insert(
            idx,
            Measured {
                size: output.size,
                offered: inputs.known_dimensions,
            },
        );
    }
}

pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    algorithm: fn(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    if inputs.run_mode != RunMode::PerformLayout {
        return algorithm(tree, node, inputs);
    }
    let idx = usize::from(node);
    let flex = tree.nodes[idx].formatting_context == FormattingContext::Flex;
    let parent = tree.query_style(
        idx,
        if flex {
            ReadRole::FlexContainer
        } else {
            ReadRole::GridContainer
        },
    );
    let replay = flex
        && parent.flex_direction_is_row()
        && parent.flex_lines_are_reversed()
        && super::TaffyChildIter::new_for_session(tree, idx)
            .any(|child| crate::layout::table::item_owns_height(tree, child));
    tree.context.table_item_captures.push(Capture {
        container: idx,
        sizes: HashMap::new(),
        calls: replay.then(Vec::new),
    });
    let mut output = algorithm(tree, node, inputs);
    let capture = tree
        .context
        .table_item_captures
        .pop()
        .expect("table capture was pushed before layout");
    if !capture.sizes.is_empty() {
        if tree.nodes[idx].formatting_context == FormattingContext::Flex {
            normalize_flex(
                tree,
                idx,
                inputs,
                capture.sizes,
                capture.calls.as_deref(),
                &mut output,
            );
        } else {
            normalize_grid(tree, idx, inputs, capture.sizes, &mut output);
        }
    }
    output
}

fn is_stretch(alignment: raikiri_layout_style::values::AlignSelf) -> bool {
    matches!(
        alignment.keyword,
        AlignItemsKeyword::Normal | AlignItemsKeyword::Stretch
    )
}

fn normalize_grid(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    sizes: HashMap<usize, Measured>,
    output: &mut LayoutOutput,
) {
    let items: Vec<_> = super::TaffyChildIter::new_for_session(tree, idx)
        .filter(|&child| {
            !tree.nodes[child].is_display_none()
                && tree.nodes[child].layout_position() != Position::Absolute
        })
        .collect();
    let inline = super::grid_baseline::runs_vertically(tree, idx);
    let parent = tree.query_style(idx, ReadRole::GridContainer);
    let reversed = inline && parent.direction() == Direction::Rtl;
    let default_align = if inline {
        parent.justify_items()
    } else {
        parent.align_items()
    };
    let mut shifts = Vec::new();
    for &child in &items {
        let Some(measured) = sizes.get(&child) else {
            continue;
        };
        if !crate::layout::table::item_owns_height(tree, child) {
            continue;
        }
        let style = crate::layout::table::item_style(tree, child, ReadRole::GridItem);
        let alignment = if inline {
            style.bridge_justify_self()
        } else {
            style.bridge_align_self()
        }
        .unwrap_or(default_align);
        let margin = style.margin();
        let auto_margin = if inline {
            margin.left.is_auto() || margin.right.is_auto()
        } else {
            margin.top.is_auto() || margin.bottom.is_auto()
        };
        if !is_stretch(alignment) || auto_margin {
            continue;
        }
        let mut layout = tree.nodes[child].layout;
        let (old, actual) = if inline {
            (layout.size.width, measured.size.width)
        } else {
            (layout.size.height, measured.size.height)
        };
        if actual >= old {
            continue;
        }
        let shift = if reversed { old - actual } else { 0.0 };
        if inline {
            layout.size.width = actual;
            layout.location.x += shift;
        } else {
            layout.size.height = actual;
        }
        tree.set_unrounded_layout(NodeId::from(child), &layout);
        shifts.push((child, Point { x: shift, y: 0.0 }));
    }
    finish(tree, idx, inputs, &items, &shifts, output);
}

fn normalize_flex(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    sizes: HashMap<usize, Measured>,
    calls: Option<&[FlexReplayCall]>,
    output: &mut LayoutOutput,
) {
    let parent = tree.query_style(idx, ReadRole::FlexContainer);
    let row = parent.flex_direction_is_row();
    let reversed =
        parent.flex_lines_are_reversed() ^ (!row && parent.direction() == Direction::Rtl);
    if !reversed {
        return;
    }
    let default_align = parent.align_items();
    let items: Vec<_> = super::TaffyChildIter::new_for_session(tree, idx)
        .filter(|&child| tree.nodes[child].layout_position() != Position::Absolute)
        .collect();
    let mut shifts = Vec::new();
    let mut layouts = Vec::new();
    for &child in &items {
        let Some(measured) = sizes.get(&child) else {
            continue;
        };
        if !crate::layout::table::item_owns_height(tree, child) {
            continue;
        }
        let Some(offered) = (if row {
            measured.offered.height
        } else {
            measured.offered.width
        }) else {
            continue;
        };
        let actual = if row {
            measured.size.height
        } else {
            measured.size.width
        };
        if actual >= offered {
            continue;
        }
        let style = crate::layout::table::item_style(tree, child, ReadRole::FlexItem);
        let margin = style.margin();
        let auto_margin = if row {
            margin.top.is_auto() || margin.bottom.is_auto()
        } else {
            margin.left.is_auto() || margin.right.is_auto()
        };
        if !is_stretch(style.align_self().unwrap_or(default_align)) || auto_margin {
            continue;
        }
        let delta = offered - actual;
        let mut layout = tree.nodes[child].layout;
        if row {
            layout.location.y += delta;
        } else {
            layout.location.x += delta;
        }
        layouts.push((child, layout));
        shifts.push((
            child,
            Point {
                x: if row { 0.0 } else { delta },
                y: if row { delta } else { 0.0 },
            },
        ));
    }
    if row && output.baselines.first.is_some() && !shifts.is_empty() {
        let moved: Vec<_> = shifts.iter().map(|&(child, _)| child).collect();
        if let Some(calls) = calls {
            match flex_baseline_source(tree, NodeId::from(idx), inputs, calls, &moved) {
                Ok(Some(source)) => {
                    if let Some((_, shift)) = shifts.iter().find(|&&(child, _)| child == source) {
                        output.baselines.first =
                            output.baselines.first.map(|baseline| baseline + shift.y);
                    }
                }
                Ok(None) => {}
                Err(message) => tree.context.table_layout_error = Some(message.to_owned()),
            }
        }
    }
    for (child, layout) in layouts {
        tree.set_unrounded_layout(NodeId::from(child), &layout);
    }
    finish(tree, idx, inputs, &items, &shifts, output);
}
fn finish(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    items: &[usize],
    shifts: &[(usize, Point<f32>)],
    output: &mut LayoutOutput,
) {
    if shifts.is_empty() {
        return;
    }
    // Local out-of-flow layout runs after this pass. Only the candidates
    // already translated by the grid need to follow their moved item now.
    let horizontal: Vec<_> = shifts
        .iter()
        .map(|&(child, shift)| (child, shift.x))
        .collect();
    let vertical: Vec<_> = shifts
        .iter()
        .map(|&(child, shift)| (child, shift.y))
        .collect();
    output.oof_candidates.for_each_mut(|candidate| {
        let node = usize::from(candidate.node);
        if let Some(shift) = super::inline_baseline::shift_of(tree, idx, &horizontal, node) {
            candidate.static_position.x.area.start += shift;
            candidate.static_position.x.area.end += shift;
        }
        if let Some(shift) = super::inline_baseline::shift_of(tree, idx, &vertical, node) {
            candidate.static_position.y.area.start += shift;
            candidate.static_position.y.area.end += shift;
        }
    });
    output.scrollable_overflow_rect = overflow(tree, idx, inputs, items, output.size);
}

fn overflow(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    items: &[usize],
    size: Size<f32>,
) -> Edges<f32> {
    let flex = tree.nodes[idx].formatting_context == FormattingContext::Flex;
    let style = tree.query_style(
        idx,
        if flex {
            ReadRole::FlexContainer
        } else {
            ReadRole::GridContainer
        },
    );
    let direction = style.direction();
    let border = style.border().resolve_or_zero(inputs.parent_size.width);
    let padding = style.padding().resolve_or_zero(inputs.parent_size.width);
    let scrolls =
        style.overflow().x.is_scroll_container() || style.overflow().y.is_scroll_container();
    let mut rect = Edges::ZERO;
    for &child in items {
        let layout = tree.nodes[child].layout;
        let style = tree.query_style(
            child,
            if flex {
                ReadRole::FlexItem
            } else {
                ReadRole::GridItem
            },
        );
        let location = Point {
            x: if direction == Direction::Rtl {
                size.width - layout.location.x - layout.size.width - border.right
            } else {
                layout.location.x - border.left
            },
            y: layout.location.y - border.top,
        };
        rect = rect.union(super::vertical_oof::overflow_contribution(
            location,
            layout.size,
            layout.scrollable_overflow_rect,
            style.overflow(),
            style.contain(),
            scrolls,
        ));
    }
    if scrolls {
        rect.right += if direction == Direction::Rtl {
            padding.left
        } else {
            padding.right
        };
        rect.bottom += padding.bottom;
    }
    rect
}
