//! Absolutely positioned children of block containers in vertical writing
//! modes.
//!
//! Block layout runs on physical axes, so a vertical block container with
//! only out-of-flow children ends with a zero physical height. Taffy hands
//! that height to each absolutely positioned child as its available height
//! and places the child at the physical left edge. In vertical writing
//! modes the physical height is the inline axis: a shrink-to-fit inline size
//! then collapses to min-content, and the static position belongs at the
//! block-start edge (CSS Position 3 §4.3, CSS 2.1 §10.3.7, both read through
//! CSS Writing Modes 4 §7.1 abstract-to-physical mapping).
//!
//! This module records the inline extent such a container would fill, gives
//! it to the out-of-flow children as their available inline size, and moves
//! their static position to the block-start edge.

use taffy::util::{MaybeResolve, ResolveOrZero};
use taffy::{AvailableSpace, LayoutInput, Position as TaffyPosition, Size};

use super::resolve_calc;
use crate::document::Document;
use raikiri_style::property::WritingMode;

/// The writing mode in effect at `node`: the nearest authored value on the
/// node or an ancestor, or `horizontal-tb`.
pub(super) fn used_writing_mode(tree: &Document, node: usize) -> WritingMode {
    let mut current = Some(node);
    while let Some(id) = current {
        if let Some(mode) = tree.nodes[id].authored_writing_mode {
            return mode;
        }
        current = tree.parent_of(id);
    }
    WritingMode::HorizontalTb
}

fn is_vertical(mode: WritingMode) -> bool {
    matches!(mode, WritingMode::VerticalRl | WritingMode::VerticalLr)
}

fn is_absolute(tree: &Document, node: usize) -> bool {
    tree.nodes[node].style.position == TaffyPosition::Absolute
        && tree.nodes[node].style.display != taffy::Display::None
}

/// The physical height of the padding box `node` would fill as a vertical
/// block container with an auto inline size, when its out-of-flow children
/// need it as their containing block's inline size.
///
/// `None` when the node is horizontal, has no absolutely positioned child,
/// or already has a definite physical height that Taffy uses as the
/// containing block.
pub(super) fn containing_block_inline_size(
    tree: &Document,
    node: usize,
    inputs: &LayoutInput,
) -> Option<f32> {
    if !is_vertical(used_writing_mode(tree, node))
        || inputs.known_dimensions.height.is_some()
        || !tree.nodes[node]
            .children
            .iter()
            .any(|&child| is_absolute(tree, child))
    {
        return None;
    }
    let style = &tree.nodes[node].style;
    if style
        .size
        .height
        .maybe_resolve(inputs.parent_size.height, resolve_calc)
        .is_some()
    {
        return None;
    }
    // A block-level box fills the inline size of its containing block, which
    // is the parent's physical height in vertical writing modes.
    let available = match (inputs.parent_size.height, inputs.available_space.height) {
        (Some(height), _) | (None, AvailableSpace::Definite(height)) => height,
        (None, _) => return None,
    };
    let basis = Some(available);
    let margin = style.margin.map(|margin| {
        margin
            .resolve_to_option(basis.unwrap_or(0.0), resolve_calc)
            .unwrap_or(0.0)
    });
    let border = style.border.resolve_or_zero(basis, resolve_calc);
    let mut border_box = (available - margin.top - margin.bottom).max(0.0);
    if let Some(max) = style.max_size.height.maybe_resolve(basis, resolve_calc) {
        border_box = border_box.min(max);
    }
    if let Some(min) = style.min_size.height.maybe_resolve(basis, resolve_calc) {
        border_box = border_box.max(min);
    }
    Some((border_box - border.top - border.bottom).max(0.0))
}

/// `inputs` for the absolutely positioned `node`, with the inline extent of
/// a vertical containing block recorded by [`containing_block_inline_size`]
/// as its available physical height.
pub(super) fn absolute_child_inputs(
    tree: &Document,
    node: usize,
    inputs: LayoutInput,
) -> LayoutInput {
    let Some(&(container, inline_size)) = tree.vertical_oof_containing_blocks.last() else {
        return inputs;
    };
    if tree.parent_of(node) != Some(container)
        || !is_absolute(tree, node)
        || inputs.known_dimensions.height.is_some()
    {
        return inputs;
    }
    let inset = tree.nodes[node].style.inset;
    if !inset.top.is_auto() && !inset.bottom.is_auto() {
        return inputs;
    }
    LayoutInput {
        parent_size: Size {
            height: Some(inline_size),
            ..inputs.parent_size
        },
        available_space: Size {
            height: AvailableSpace::Definite(inline_size),
            ..inputs.available_space
        },
        ..inputs
    }
}

/// Move each absolutely positioned child of the vertical container `node`
/// whose `left` and `right` are both auto to its static position on the
/// block-start edge of the padding box: the right edge in `vertical-rl`, the
/// left edge in `vertical-lr`. A container laid out here has no in-flow
/// content before the child, so that edge is the static position.
pub(super) fn place_static_block_start(tree: &mut Document, node: usize, border_box: Size<f32>) {
    let mode = used_writing_mode(tree, node);
    if !is_vertical(mode) {
        return;
    }
    let style = &tree.nodes[node].style;
    let border = style
        .border
        .resolve_or_zero(Some(border_box.height), resolve_calc);
    let padding = style
        .padding
        .resolve_or_zero(Some(border_box.height), resolve_calc);
    let children: Vec<usize> = tree.nodes[node]
        .children
        .iter()
        .copied()
        .filter(|&child| is_absolute(tree, child))
        .collect();
    for child in children {
        let child_style = &tree.nodes[child].style;
        if !child_style.inset.left.is_auto() || !child_style.inset.right.is_auto() {
            continue;
        }
        let margin = child_style.margin.map(|margin| {
            margin
                .resolve_to_option(border_box.height, resolve_calc)
                .unwrap_or(0.0)
        });
        let layout = &mut tree.nodes[child].unrounded_layout;
        layout.location.x = match mode {
            WritingMode::VerticalRl => {
                border_box.width - border.right - padding.right - margin.right - layout.size.width
            }
            _ => border.left + padding.left + margin.left,
        };
    }
}

#[cfg(test)]
mod tests;
