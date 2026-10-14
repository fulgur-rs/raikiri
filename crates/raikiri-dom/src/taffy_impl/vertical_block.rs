//! Block containers in vertical writing modes.
//!
//! Taffy's block algorithm stacks children down the physical y axis, which
//! is the block axis of `horizontal-tb` only. In `vertical-rl` and
//! `vertical-lr` the block axis is the physical x axis and the inline axis
//! is the physical y axis (CSS Writing Modes 4 §2.1): block-level boxes
//! stack from right to left in `vertical-rl` and from left to right in
//! `vertical-lr`, and each one fills the container's physical height.
//!
//! This module runs taffy's block algorithm on a transposed view of such a
//! container. While the container is laid out, taffy reads the container's
//! style and its children's styles with the physical axes swapped: a
//! physical height is a transposed width, and the block-start edge (the
//! right edge in `vertical-rl`, the left edge in `vertical-lr`) is the
//! transposed top. Each child is laid out on its own physical axes, with
//! the sizes taffy asks for swapped back, and its output is swapped again
//! for taffy. When taffy has placed the children, their layouts are mapped
//! back to physical coordinates, measured from the container's block-start
//! edge.
//!
//! A vertical container establishes a block formatting context of its own
//! and its children are laid out each in a fresh one, so margins collapse
//! between adjacent siblings but not between a box and its children, and
//! floats stay inside the box that holds them.

use std::collections::HashMap;

use taffy::tree::{RequestedAxis, RunMode};
use taffy::util::{MaybeResolve, ResolveOrZero};
use taffy::{
    AbsoluteAxis, AvailableSpace, Baselines, BoxSizing, CollapsibleMarginSet, Display, Layout,
    LayoutInput, LayoutOutput, LengthPercentage, Line, NodeId, Point, Position as TaffyPosition,
    Rect, Size, SizingMode, Style, TraversePartialTree, compute_block_layout,
};

use super::resolve_calc;
use super::vertical_oof::used_writing_mode;
use crate::document::Document;
use raikiri_style::property::{DisplayValue, WritingMode};

/// A vertical block container being laid out on transposed axes.
#[derive(Clone, Debug)]
pub(crate) struct TransposedBlock {
    container: usize,
    /// The physical width of the container's content box, when definite.
    block_space: Option<f32>,
    container_style: Style,
    child_styles: HashMap<usize, Style>,
    /// The children taffy stored a transposed layout for.
    placed: Vec<usize>,
}

/// The block direction of `idx` when it lays out its block children on
/// vertical axes: `Some(true)` for `vertical-rl`, `Some(false)` for
/// `vertical-lr`.
///
/// A multicol container and its contents run vertical writing on
/// horizontal axes, as their sizing properties do, and so do the boxes the
/// table algorithm lays out; they are left to the horizontal block path.
pub(crate) fn block_right_to_left(tree: &Document, idx: usize) -> Option<bool> {
    let right_to_left = match used_writing_mode(tree, idx) {
        WritingMode::VerticalRl => true,
        WritingMode::VerticalLr => false,
        _ => return None,
    };
    if is_table_box(tree.nodes[idx].display) {
        return None;
    }
    let mut node = Some(idx);
    while let Some(id) = node {
        if tree.nodes[id].multicol.is_some() {
            return None;
        }
        node = tree.parent_of(id);
    }
    Some(right_to_left)
}

pub(crate) fn is_table_box(display: DisplayValue) -> bool {
    matches!(
        display,
        DisplayValue::Table
            | DisplayValue::InlineTable
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
            | DisplayValue::TableRow
            | DisplayValue::TableColumnGroup
            | DisplayValue::TableColumn
            | DisplayValue::TableCell
            | DisplayValue::TableCaption
    )
}

fn is_vertical(tree: &Document, idx: usize) -> bool {
    block_right_to_left(tree, idx).is_some()
}

impl Document {
    /// The transposed style of the container being laid out, when `idx` is
    /// that container.
    pub(super) fn transposed_container_style(&self, idx: usize) -> Option<&Style> {
        self.transposed_blocks
            .last()
            .filter(|block| block.container == idx)
            .map(|block| &block.container_style)
    }

    /// The transposed style of `idx`, when it is a child of the container
    /// being laid out.
    pub(super) fn transposed_child_style(&self, idx: usize) -> Option<&Style> {
        self.transposed_blocks
            .last()
            .and_then(|block| block.child_styles.get(&idx))
    }

    /// Record that taffy stored a transposed layout for `idx`.
    pub(super) fn note_transposed_layout(&mut self, idx: usize) {
        if let Some(block) = self.transposed_blocks.last_mut()
            && block.child_styles.contains_key(&idx)
        {
            block.placed.push(idx);
        }
    }

    /// Lay out `idx` on its physical axes when taffy asks for it as a child
    /// of the transposed container being laid out; `None` otherwise.
    ///
    /// The container's view is set aside meanwhile, so that the child and
    /// its own descendants see their physical styles.
    pub(super) fn compute_transposed_child(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
    ) -> Option<LayoutOutput> {
        let idx = usize::from(node_id);
        self.transposed_blocks
            .last()
            .filter(|block| block.child_styles.contains_key(&idx))?;
        let block = self.transposed_blocks.pop()?;
        let inputs =
            orthogonal_child_inputs(self, idx, transpose_inputs(inputs), block.block_space);
        // Taffy resolves a box's percentage padding against the width of
        // its parent, but the containing block of a horizontally flowing
        // child is the vertical container, whose inline size is its
        // physical height (CSS Writing Modes 4 §7.1). A vertical child
        // transposes its own inputs and needs no help.
        let physical_padding = flows_horizontally(self, idx)
            .then_some(inputs.parent_size.height)
            .flatten()
            .map(|inline_size| {
                let style = &mut self.nodes[idx].style;
                let resolved = style
                    .padding
                    .resolve_or_zero(Some(inline_size), resolve_calc)
                    .map(LengthPercentage::length);
                std::mem::replace(&mut style.padding, resolved)
            });
        let output = self.compute_child_layout_with_block_ctx(node_id, inputs, None);
        if let Some(padding) = physical_padding {
            self.nodes[idx].style.padding = padding;
        }
        self.transposed_blocks.push(block);
        Some(transpose_output(output))
    }
}

/// Whether the child `idx` of a vertical container lays its own content out
/// on horizontal axes: a horizontal box, or a flex or grid container, whose
/// algorithms place items on physical axes.
fn flows_horizontally(tree: &Document, idx: usize) -> bool {
    !is_vertical(tree, idx)
        || matches!(tree.nodes[idx].style.display, Display::Flex | Display::Grid)
}

/// `inputs` for the child `idx` of a vertical container, already on
/// physical axes. A child that flows horizontally is an orthogonal flow:
/// its block size is its physical height, which comes from its content, not
/// from the inline size of the container that taffy stretched it to (CSS
/// Writing Modes 4 §7.3), and its inline size is found in the physical
/// width of the container's content box, `block_space`, when that is
/// definite.
fn orthogonal_child_inputs(
    tree: &Document,
    idx: usize,
    inputs: LayoutInput,
    block_space: Option<f32>,
) -> LayoutInput {
    let style = &tree.nodes[idx].style;
    if !flows_horizontally(tree, idx)
        || inputs.sizing_mode != SizingMode::InherentSize
        || style.position == TaffyPosition::Absolute
    {
        return inputs;
    }
    let mut inputs = inputs;
    if style.size.height.is_auto() {
        inputs.known_dimensions.height = None;
    }
    if let Some(space) = block_space {
        inputs.available_space.width = AvailableSpace::Definite(space);
        // An in-flow block box with an auto width fills that space, as it
        // would under a horizontal block container; a table still shrinks
        // to fit.
        if inputs.known_dimensions.width.is_none()
            && style.size.width.is_auto()
            && style.float == taffy::Float::None
            && !is_table_box(tree.nodes[idx].display)
        {
            inputs.known_dimensions.width = Some(stretched_width(style, space, inputs));
        }
    }
    inputs
}

/// The border-box width of a horizontal block box with an auto width that
/// fills `space`, the physical width of its vertical container, within its
/// min and max widths. Its margins, which the container places like its
/// other children's, resolve against the container's inline size, its
/// physical height.
fn stretched_width(style: &Style, space: f32, inputs: LayoutInput) -> f32 {
    let inline_size = inputs.parent_size.height;
    let margin = style
        .margin
        .map(|margin| margin.resolve_or_zero(inline_size, resolve_calc));
    // Min and max widths are content-box sizes under `box-sizing:
    // content-box`.
    let edges = match style.box_sizing {
        BoxSizing::BorderBox => 0.0,
        BoxSizing::ContentBox => {
            // Against the container's inline size, as the child's own
            // layout resolves them.
            let basis = inputs.parent_size.height;
            let padding = style.padding.resolve_or_zero(basis, resolve_calc);
            let border = style.border.resolve_or_zero(basis, resolve_calc);
            padding.left + padding.right + border.left + border.right
        }
    };
    let basis = Some(space);
    let mut width = space - margin.left - margin.right;
    if let Some(max) = style.max_size.width.maybe_resolve(basis, resolve_calc) {
        width = width.min(max + edges);
    }
    if let Some(min) = style.min_size.width.maybe_resolve(basis, resolve_calc) {
        width = width.max(min + edges);
    }
    width.max(0.0)
}

/// Resolve the percentage paddings of the children of the transposed
/// container being laid out against the container's inline size, the inner
/// transposed width of its border box `width`. Taffy resolves an item's top
/// and bottom padding against the container's height, which is its
/// physical width here, while percentages of padding refer to the inline
/// size of the containing block on every side (CSS Writing Modes 4 §7.1).
fn resolve_child_padding(tree: &mut Document, width: f32, basis: Option<f32>) {
    let block = tree
        .transposed_blocks
        .last_mut()
        .expect("the container's block is pushed");
    let padding = block
        .container_style
        .padding
        .resolve_or_zero(basis, resolve_calc);
    let border = block
        .container_style
        .border
        .resolve_or_zero(basis, resolve_calc);
    let inline_size = (width - padding.left - padding.right - border.left - border.right).max(0.0);
    for style in block.child_styles.values_mut() {
        style.padding = style
            .padding
            .resolve_or_zero(Some(inline_size), resolve_calc)
            .map(LengthPercentage::length);
    }
}

/// Lay out the vertical block container `node_id`, whose block axis runs
/// from right to left when `right_to_left` is set, on transposed axes.
///
/// `inputs` are on physical axes, and so is the returned output.
pub(super) fn compute_vertical_block(
    tree: &mut Document,
    node_id: NodeId,
    inputs: LayoutInput,
    right_to_left: bool,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let container_style = transpose_style(&tree.nodes[idx].style, right_to_left);
    let child_styles = tree
        .child_ids(node_id)
        .map(usize::from)
        .map(|child| {
            (
                child,
                transpose_style(&tree.nodes[child].style, right_to_left),
            )
        })
        .collect();
    let mut inputs = transpose_inputs(inputs);
    let block_space = inputs.known_dimensions.height.map(|height| {
        let basis = inputs.parent_size.width;
        let padding = container_style.padding.resolve_or_zero(basis, resolve_calc);
        let border = container_style.border.resolve_or_zero(basis, resolve_calc);
        (height - padding.top - padding.bottom - border.top - border.bottom).max(0.0)
    });
    tree.transposed_blocks.push(TransposedBlock {
        container: idx,
        block_space,
        container_style,
        child_styles,
        placed: Vec::new(),
    });
    if inputs.known_dimensions.width.is_none() && shrinks_to_fit(tree, idx, &inputs) {
        inputs.known_dimensions.width = Some(fit_content_inline_size(tree, node_id, inputs));
    }
    if let Some(width) = inputs.known_dimensions.width {
        resolve_child_padding(tree, width, inputs.parent_size.width);
    }
    let output = compute_block_layout(tree, node_id, inputs, None);
    let mut block = tree.transposed_blocks.pop().expect("pushed above");
    let output = transpose_output(output);
    // Taffy may store a child's layout more than once; the last one stands.
    block.placed.sort_unstable();
    block.placed.dedup();
    for child in block.placed {
        let layout = &mut tree.nodes[child].unrounded_layout;
        *layout = physical_layout(layout, output.size.width, right_to_left);
    }
    output
}

/// Whether the vertical container `idx` takes the fit-content inline size
/// instead of filling its available space. Under a horizontal parent, or
/// as the root, it is an orthogonal flow, whose auto inline size is
/// fit-content (CSS Writing Modes 4 §7.3.1). Under a vertical block
/// container it fills the parent's inline size like any block-level box.
fn shrinks_to_fit(tree: &Document, idx: usize, inputs: &LayoutInput) -> bool {
    inputs.sizing_mode == SizingMode::InherentSize
        && tree.nodes[idx].style.size.height.is_auto()
        && !tree
            .taffy_parent_of(idx)
            .is_some_and(|parent| is_vertical(tree, parent))
}

/// The fit-content inline size of the transposed container `node_id`:
/// its max-content size, unless the available space is smaller, then that
/// space, but never below its min-content size (CSS Sizing 3 §5.1).
fn fit_content_inline_size(tree: &mut Document, node_id: NodeId, inputs: LayoutInput) -> f32 {
    let page_height = tree.fixed_page_area.1;
    let style = &tree.nodes[usize::from(node_id)].style;
    // The transposed horizontal margins are the physical top and bottom.
    let margin = style
        .margin
        .map(|margin| margin.resolve_or_zero(inputs.parent_size.height, resolve_calc));
    let mut measure = |available: AvailableSpace| {
        let measure_inputs = LayoutInput {
            run_mode: RunMode::ComputeSize,
            axis: RequestedAxis::Horizontal,
            available_space: Size {
                width: available,
                ..inputs.available_space
            },
            ..inputs
        };
        compute_block_layout(tree, node_id, measure_inputs, None)
            .size
            .width
    };
    let max_content = measure(AvailableSpace::MaxContent);
    // Indefinite available space falls back to the inline size of the
    // initial containing block, the page area height (CSS Writing Modes 4
    // §7.3.1).
    let available = inputs
        .available_space
        .width
        .into_option()
        .or((page_height > 0.0).then_some(page_height));
    let stretch = available.map_or(f32::INFINITY, |space| space - margin.top - margin.bottom);
    if max_content <= stretch.max(0.0) {
        return max_content;
    }
    measure(AvailableSpace::MinContent).max(stretch.max(0.0))
}

/// `rect` with its physical sides moved to the transposed sides they map
/// to: top and bottom become left and right, and the block-start side
/// becomes the top.
fn transpose_rect<T: Copy>(rect: Rect<T>, right_to_left: bool) -> Rect<T> {
    let (block_start, block_end) = if right_to_left {
        (rect.right, rect.left)
    } else {
        (rect.left, rect.right)
    };
    Rect {
        left: rect.top,
        right: rect.bottom,
        top: block_start,
        bottom: block_end,
    }
}

/// The inverse of [`transpose_rect`].
fn physical_rect<T: Copy>(rect: Rect<T>, right_to_left: bool) -> Rect<T> {
    let (left, right) = if right_to_left {
        (rect.bottom, rect.top)
    } else {
        (rect.top, rect.bottom)
    };
    Rect {
        left,
        right,
        top: rect.left,
        bottom: rect.right,
    }
}

fn swap<T>(size: Size<T>) -> Size<T> {
    Size {
        width: size.height,
        height: size.width,
    }
}

fn transpose_style(style: &Style, right_to_left: bool) -> Style {
    Style {
        size: swap(style.size),
        min_size: swap(style.min_size),
        max_size: swap(style.max_size),
        aspect_ratio: style.aspect_ratio.map(|ratio| 1.0 / ratio),
        margin: transpose_rect(style.margin, right_to_left),
        padding: transpose_rect(style.padding, right_to_left),
        border: transpose_rect(style.border, right_to_left),
        inset: transpose_rect(style.inset, right_to_left),
        overflow: Point {
            x: style.overflow.y,
            y: style.overflow.x,
        },
        ..style.clone()
    }
}

/// `inputs` with the axes swapped. Margins never collapse across the edges
/// of a vertical container, whose block axis is not the one taffy's margin
/// collapsing works along on the other side.
fn transpose_inputs(inputs: LayoutInput) -> LayoutInput {
    LayoutInput {
        axis: AbsoluteAxis::try_from(inputs.axis)
            .map_or(RequestedAxis::Both, |axis| axis.other_axis().into()),
        known_dimensions: swap(inputs.known_dimensions),
        known_dimensions_are_definite: swap(inputs.known_dimensions_are_definite),
        parent_size: swap(inputs.parent_size),
        available_space: swap(inputs.available_space),
        vertical_margins_are_collapsible: Line::FALSE,
        ..inputs
    }
}

/// `output` with the axes swapped. Its margins are left to the parent,
/// which reads them from the box's style: they do not collapse with the
/// box's children.
fn transpose_output(output: LayoutOutput) -> LayoutOutput {
    LayoutOutput {
        size: swap(output.size),
        scrollable_overflow_rect: swap_rect_axes(output.scrollable_overflow_rect),
        baselines: Baselines::NONE,
        top_margin: CollapsibleMarginSet::ZERO,
        bottom_margin: CollapsibleMarginSet::ZERO,
        margins_can_collapse_through: false,
    }
}

/// A scrollable overflow rectangle, measured along the inline and block
/// axes from the scroll origin, with its axes swapped.
fn swap_rect_axes(rect: Rect<f32>) -> Rect<f32> {
    Rect {
        left: rect.top,
        right: rect.bottom,
        top: rect.left,
        bottom: rect.right,
    }
}

/// The physical layout of a child placed by taffy at `layout` in a
/// transposed container whose physical width is `container_width`.
fn physical_layout(layout: &Layout, container_width: f32, right_to_left: bool) -> Layout {
    let block_start = layout.location.y;
    Layout {
        order: layout.order,
        location: Point {
            x: if right_to_left {
                container_width - block_start - layout.size.height
            } else {
                block_start
            },
            y: layout.location.x,
        },
        size: swap(layout.size),
        scrollable_overflow_rect: swap_rect_axes(layout.scrollable_overflow_rect),
        scrollbar_size: swap(layout.scrollbar_size),
        border: physical_rect(layout.border, right_to_left),
        padding: physical_rect(layout.padding, right_to_left),
        margin: physical_rect(layout.margin, right_to_left),
    }
}

#[cfg(test)]
mod tests;
