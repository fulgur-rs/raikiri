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
//! Same-writing-mode block descendants keep sharing their parent's block
//! formatting context on these transposed axes. Orthogonal flows and boxes
//! that establish an independent formatting context still start their own.

use crate::layout::context::LayoutSession;
use std::collections::HashMap;

#[cfg(test)]
use crate::LayoutStyle as Style;
use taffy::tree::{RequestedAxis, RunMode};
use taffy::util::{MaybeResolve, ResolveOrZero};
use taffy::{
    AvailableSpace, BlockItemStyle, BoxSizing, CoreStyle, LayoutInput, LayoutOutput, Line, NodeId,
    Position as TaffyPosition, Size, SizingMode, TraversePartialTree,
};
#[cfg(test)]
use taffy::{LengthPercentage, Point};

use super::block_flow::compute_block_flow;
use super::resolve_calc;
use super::transposed_axes::{
    physical_layout, transpose_child_output_to_vertical, transpose_inputs,
    transpose_output_to_physical,
};
#[cfg(test)]
use super::transposed_axes::{swap, transpose_rect};
use super::vertical_oof::used_writing_mode;
use crate::document::Document;
use raikiri_style::property::{DisplayValue, WritingMode};

/// A vertical block container being laid out on transposed axes.
#[derive(Clone, Debug)]
pub(crate) struct TransposedBlock {
    id: u64,
    container: usize,
    right_to_left: bool,
    /// The physical width of the container's content box, when definite.
    block_space: Option<f32>,
    children: Vec<usize>,
    #[cfg(test)]
    container_style: Style,
    #[cfg(test)]
    child_styles: HashMap<usize, Style>,
    container_view: Option<crate::layout::style_views::frozen::FrozenViewInput>,
    child_views: HashMap<usize, crate::layout::style_views::frozen::FrozenViewInput>,
    /// The children taffy stored a transposed layout for.
    placed: Vec<usize>,
}

impl TransposedBlock {
    fn has_child(&self, idx: usize) -> bool {
        #[cfg(test)]
        {
            self.child_styles.contains_key(&idx)
        }
        #[cfg(not(test))]
        {
            self.child_views.contains_key(&idx)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TransposedLocation {
    frame: usize,
    id: u64,
    child: Option<usize>,
}

#[derive(Debug, Default)]
pub(crate) struct TransposedStyleIndex(HashMap<usize, Vec<TransposedLocation>>);

impl TransposedStyleIndex {
    fn register(&mut self, block: &TransposedBlock, frame: usize) {
        self.0
            .entry(block.container)
            .or_default()
            .push(TransposedLocation {
                frame,
                id: block.id,
                child: None,
            });
        for &child in &block.children {
            self.0.entry(child).or_default().push(TransposedLocation {
                frame,
                id: block.id,
                child: Some(child),
            });
        }
    }

    fn remove(&mut self, idx: usize, location: TransposedLocation) {
        let locations = self.0.get_mut(&idx).expect("registered snapshot");
        assert_eq!(
            locations.pop(),
            Some(location),
            "snapshot registrations restore in reverse order"
        );
        if locations.is_empty() {
            self.0.remove(&idx);
        }
    }

    fn unregister(&mut self, block: &TransposedBlock, frame: usize) {
        for &child in &block.children {
            self.remove(
                child,
                TransposedLocation {
                    frame,
                    id: block.id,
                    child: Some(child),
                },
            );
        }
        self.remove(
            block.container,
            TransposedLocation {
                frame,
                id: block.id,
                child: None,
            },
        );
    }
}

fn prepare_frame_views(tree: &mut LayoutSession<'_>, block: &mut TransposedBlock) {
    use crate::layout::style_views::frozen::FrozenViewInput;
    use raikiri_layout_style::{BridgeProjection, ClaimsPolicy, ViewContext};
    if block.container_view.is_some() || tree.style_views.is_none() {
        return;
    }
    let context = ViewContext {
        bridge_projection: BridgeProjection::BlockTranspose {
            right_to_left: block.right_to_left,
        },
        claims_policy: ClaimsPolicy::Positioning,
        ..ViewContext::default()
    };
    block.container_view = FrozenViewInput::capture(tree, block.container, context);
    for &child in &block.children {
        if let Some(input) = FrozenViewInput::capture(tree, child, context) {
            block.child_views.insert(child, input);
        }
    }
}

fn push_transposed_frame(tree: &mut LayoutSession<'_>, mut block: TransposedBlock) {
    prepare_frame_views(tree, &mut block);
    tree.context
        .transposed_style_index
        .register(&block, tree.context.transposed_blocks.len());
    tree.context.transposed_blocks.push(block);
}

pub(crate) struct TransposedSizingUndo {
    frame: usize,
    id: u64,
    child: Option<usize>,
    #[cfg(test)]
    previous: Option<raikiri_layout_style::LegacySizingUndo>,
    view_previous: Option<raikiri_layout_style::OwnedOverrideUndo>,
}

impl TransposedSizingUndo {
    pub(crate) fn prepare(
        blocks: &[TransposedBlock],
        index: &TransposedStyleIndex,
        idx: usize,
    ) -> Vec<Self> {
        let mut undo = Vec::new();
        for location in index.0.get(&idx).into_iter().flatten() {
            if blocks
                .get(location.frame)
                .is_some_and(|block| block.id == location.id)
            {
                undo.push(Self {
                    frame: location.frame,
                    id: location.id,
                    child: location.child,
                    #[cfg(test)]
                    previous: None,
                    view_previous: None,
                });
            }
        }
        undo
    }

    #[cfg(test)]
    fn target<'a>(&self, blocks: &'a mut [TransposedBlock]) -> &'a mut Style {
        let block = &mut blocks[self.frame];
        assert_eq!(
            block.id, self.id,
            "scope restores the same transposed frame"
        );
        if let Some(child) = self.child {
            block
                .child_styles
                .get_mut(&child)
                .expect("scoped snapshot stays live")
        } else {
            &mut block.container_style
        }
    }

    fn view_target<'a>(
        &self,
        blocks: &'a mut [TransposedBlock],
    ) -> Option<&'a mut crate::layout::style_views::frozen::FrozenViewInput> {
        let block = &mut blocks[self.frame];
        assert_eq!(
            block.id, self.id,
            "scope restores the same transposed frame"
        );
        if let Some(child) = self.child {
            block.child_views.get_mut(&child)
        } else {
            block.container_view.as_mut()
        }
    }

    pub(crate) fn apply(
        &mut self,
        blocks: &mut [TransposedBlock],
        entry: &raikiri_layout_style::OwnedPropertyOverride,
    ) {
        let block = &blocks[self.frame];
        assert_eq!(
            block.id, self.id,
            "scope restores the same transposed frame"
        );
        #[cfg(test)]
        {
            let right_to_left = block.right_to_left;
            self.previous = Some(
                entry
                    .apply_transposed_legacy_sizing(self.target(blocks), right_to_left)
                    .expect("validated sizing entry"),
            );
        }
        self.view_previous = self
            .view_target(blocks)
            .map(|input| entry.replace_in(&mut input.late));
    }

    pub(crate) fn restore(mut self, blocks: &mut [TransposedBlock]) {
        if let Some(previous) = self.view_previous.take() {
            previous.restore(
                &mut self
                    .view_target(blocks)
                    .expect("frozen snapshot stays live")
                    .late,
            );
        }
        #[cfg(test)]
        if let Some(previous) = self.previous.take() {
            previous.restore(self.target(blocks));
        }
    }
}

fn next_transposed_frame_id(tree: &mut LayoutSession<'_>) -> u64 {
    let id = tree.context.transposed_frame_serial;
    tree.context.transposed_frame_serial = id
        .checked_add(1)
        .expect("transposed frame serial exhausted");
    id
}

/// The block direction of `idx` when it lays out its block children on
/// vertical axes: `Some(true)` for `vertical-rl` and `sideways-rl`,
/// `Some(false)` for `vertical-lr` and `sideways-lr`.
///
/// A multicol container and its contents run vertical writing on
/// horizontal axes, as their sizing properties do, and so do the boxes the
/// table algorithm lays out; they are left to the horizontal block path.
pub(crate) fn block_right_to_left(tree: &Document, idx: usize) -> Option<bool> {
    let right_to_left = match used_writing_mode(tree, idx) {
        WritingMode::VerticalRl | WritingMode::SidewaysRl => true,
        WritingMode::VerticalLr | WritingMode::SidewaysLr => false,
        _ => return None,
    };
    if is_table_box(tree.nodes[idx].display()) {
        return None;
    }
    if tree
        .inclusive_ancestors(idx)
        .any(|id| tree.nodes[id].multicol.is_some())
    {
        return None;
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

impl LayoutSession<'_> {
    /// The transposed style of the container being laid out, when `idx` is
    /// that container.
    #[cfg(test)]
    pub(crate) fn transposed_container_style(&self, idx: usize) -> Option<&Style> {
        self.context
            .transposed_blocks
            .last()
            .filter(|block| block.container == idx)
            .map(|block| &block.container_style)
    }

    /// The transposed style of `idx`, when it is a child of the container
    /// being laid out.
    #[cfg(test)]
    pub(crate) fn transposed_child_style(&self, idx: usize) -> Option<&Style> {
        self.context
            .transposed_blocks
            .last()
            .and_then(|block| block.child_styles.get(&idx))
    }

    pub(crate) fn transposed_container_view(
        &self,
        idx: usize,
    ) -> Option<raikiri_layout_style::LayoutStyleView<'_>> {
        self.context
            .transposed_blocks
            .last()
            .filter(|block| block.container == idx)
            .map(|block| {
                block
                    .container_view
                    .as_ref()
                    .expect("prepared frame has a container input")
                    .view()
            })
    }
    pub(crate) fn transposed_child_view(
        &self,
        idx: usize,
    ) -> Option<raikiri_layout_style::LayoutStyleView<'_>> {
        self.context
            .transposed_blocks
            .last()
            .filter(|block| block.has_child(idx))
            .map(|block| {
                block
                    .child_views
                    .get(&idx)
                    .expect("prepared frame has a child input")
                    .view()
            })
    }

    /// Record that taffy stored a transposed layout for `idx`.
    pub(super) fn note_transposed_layout(&mut self, idx: usize) {
        if let Some(block) = self.context.transposed_blocks.last_mut()
            && block.has_child(idx)
        {
            block.placed.push(idx);
        }
    }
}

impl LayoutSession<'_> {
    /// Lay out `idx` on its physical axes when taffy asks for it as a child
    /// of the transposed container being laid out; `None` otherwise.
    ///
    /// The container's view is set aside meanwhile, so that the child and
    /// its own descendants see their physical styles.
    pub(super) fn compute_transposed_child(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_ctx: Option<&mut super::block_flow::context::BlockContext<'_>>,
    ) -> Option<LayoutOutput> {
        let idx = usize::from(node_id);
        self.context
            .transposed_blocks
            .last()
            .filter(|block| block.has_child(idx))?;
        let block = self.context.transposed_blocks.pop()?;
        let right_to_left = block_right_to_left(self, block.container).unwrap_or(false);
        let share_context =
            block_ctx.is_some() && same_vertical_block_mode(self, block.container, idx);
        let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let inputs = orthogonal_child_inputs(
                self,
                idx,
                transpose_inputs(inputs, share_context),
                block.block_space,
            );
            // Taffy resolves a box's percentage padding against the width of
            // its parent, but the containing block of a horizontally flowing
            // child is the vertical container, whose inline size is its
            // physical height (CSS Writing Modes 4 §7.1). A vertical child
            // transposes its own inputs and needs no help.
            let physical_padding_basis = flows_horizontally(self, idx)
                .then_some(inputs.parent_size.height)
                .flatten();
            let child_block_ctx = if share_context { block_ctx } else { None };
            if let Some(basis) = physical_padding_basis {
                self.with_resolved_edges(idx, basis, false, |tree| {
                    tree.compute_child_layout_with_block_ctx(node_id, inputs, child_block_ctx)
                })
            } else {
                self.compute_child_layout_with_block_ctx(node_id, inputs, child_block_ctx)
            }
        }));
        self.context.transposed_blocks.push(block);
        let output = match output {
            Ok(output) => output,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        let reverse_extent = output.size.width;
        Some(transpose_child_output_to_vertical(
            output,
            right_to_left,
            reverse_extent,
            share_context,
        ))
    }
}

/// Whether two boxes use the same transposed vertical block coordinate space.
fn same_vertical_block_mode(tree: &Document, parent: usize, child: usize) -> bool {
    !flows_horizontally(tree, parent)
        && !flows_horizontally(tree, child)
        && block_right_to_left(tree, parent).is_some_and(|parent_direction| {
            block_right_to_left(tree, child) == Some(parent_direction)
        })
}

/// Whether `idx` has a parent whose block context uses the same transposed
/// vertical coordinate space as `idx`.
pub(crate) fn has_same_vertical_block_parent(tree: &LayoutSession<'_>, idx: usize) -> bool {
    tree.taffy_parent_of(idx)
        .is_some_and(|parent| same_vertical_block_mode(tree, parent, idx))
}

/// Whether the child `idx` of a vertical container lays its own content out
/// on horizontal axes: a horizontal box, or a flex or grid container, whose
/// algorithms place items on physical axes.
fn flows_horizontally(tree: &Document, idx: usize) -> bool {
    !is_vertical(tree, idx) || tree.nodes[idx].formatting_context.is_flex_or_grid()
}

/// `inputs` for the child `idx` of a vertical container, already on
/// physical axes. A child that flows horizontally is an orthogonal flow:
/// its block size is its physical height, which comes from its content, not
/// from the inline size of the container that taffy stretched it to (CSS
/// Writing Modes 4 §7.3), and its inline size is found in the physical
/// width of the container's content box, `block_space`, when that is
/// definite.
fn orthogonal_child_inputs(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    block_space: Option<f32>,
) -> LayoutInput {
    if !flows_horizontally(tree, idx)
        || inputs.sizing_mode != SizingMode::InherentSize
        || tree.nodes[idx].taffy_position() == TaffyPosition::Absolute
    {
        return inputs;
    }
    let style = tree.physical_style(idx);
    let mut inputs = inputs;
    if style.size().height.is_auto() {
        inputs.known_dimensions.height = None;
    }
    if let Some(space) = block_space {
        inputs.available_space.width = AvailableSpace::Definite(space);
        // An in-flow block box with an auto width fills that space, as it
        // would under a horizontal block container; a table still shrinks
        // to fit.
        if inputs.known_dimensions.width.is_none()
            && style.size().width.is_auto()
            && BlockItemStyle::float(&style) == taffy::Float::None
            && !is_table_box(tree.nodes[idx].display())
        {
            inputs.known_dimensions.width = Some(stretched_width(&style, space, inputs));
        }
    }
    inputs
}

/// The border-box width of a horizontal block box with an auto width that
/// fills `space`, the physical width of its vertical container, within its
/// min and max widths. Its margins, which the container places like its
/// other children's, resolve against the container's inline size, its
/// physical height.
fn stretched_width(style: &impl CoreStyle, space: f32, inputs: LayoutInput) -> f32 {
    let inline_size = inputs.parent_size.height;
    let margin = style
        .margin()
        .map(|margin| margin.resolve_or_zero(inline_size, resolve_calc));
    // Min and max widths are content-box sizes under `box-sizing:
    // content-box`.
    let edges = match style.box_sizing() {
        BoxSizing::BorderBox => 0.0,
        BoxSizing::ContentBox => {
            // Against the container's inline size, as the child's own
            // layout resolves them.
            let basis = inputs.parent_size.height;
            let padding = style.padding().resolve_or_zero(basis, resolve_calc);
            let border = style.border().resolve_or_zero(basis, resolve_calc);
            padding.left + padding.right + border.left + border.right
        }
    };
    let basis = Some(space);
    let mut width = space - margin.left - margin.right;
    if let Some(max) = style.max_size().width.maybe_resolve(basis, resolve_calc) {
        width = width.min(max + edges);
    }
    if let Some(min) = style.min_size().width.maybe_resolve(basis, resolve_calc) {
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
fn resolve_child_padding(tree: &mut LayoutSession<'_>, width: f32) {
    let block = tree
        .context
        .transposed_blocks
        .last_mut()
        .expect("the container's block is pushed");
    if let Some(container) = &block.container_view {
        let padding = container
            .view()
            .padding()
            .resolve_or_zero(None, resolve_calc);
        let border = container
            .view()
            .border()
            .resolve_or_zero(None, resolve_calc);
        let candidate_inline_size =
            (width - padding.left - padding.right - border.left - border.right).max(0.0);
        for input in block.child_views.values_mut() {
            input.resolve_padding(Some(candidate_inline_size));
        }
    }
    #[cfg(test)]
    {
        let padding = block
            .container_style
            .padding
            .resolve_or_zero(None, resolve_calc);
        let border = block
            .container_style
            .border
            .resolve_or_zero(None, resolve_calc);
        let inline_size =
            (width - padding.left - padding.right - border.left - border.right).max(0.0);
        for style in block.child_styles.values_mut() {
            style.padding = style
                .padding
                .resolve_or_zero(Some(inline_size), resolve_calc)
                .map(LengthPercentage::length);
        }
    }
}

fn with_transposed_frame<R>(
    tree: &mut LayoutSession<'_>,
    block: TransposedBlock,
    f: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> (R, TransposedBlock) {
    let id = block.id;
    push_transposed_frame(tree, block);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(tree)));
    let block = tree.context.transposed_blocks.pop().expect("pushed frame");
    tree.context
        .transposed_style_index
        .unregister(&block, tree.context.transposed_blocks.len());
    assert_eq!(block.id, id, "vertical frames restore in reverse order");
    match result {
        Ok(result) => (result, block),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Lay out the vertical block container `node_id`, whose block axis runs
/// from right to left when `right_to_left` is set, on transposed axes.
///
/// `inputs` are on physical axes, and so is the returned output.
pub(super) fn compute_vertical_block(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    right_to_left: bool,
    block_ctx: Option<&mut super::block_flow::context::BlockContext<'_>>,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    #[cfg(test)]
    let mut container_style = transpose_style(
        tree.layout_node_with_style(idx).unwrap().style,
        right_to_left,
    );
    // Percentages of the container's padding refer to the inline size of
    // its containing block (CSS Writing Modes 4 §7.1): the physical height
    // of a vertical parent, the physical width of a horizontal one.
    let parent = tree.taffy_parent_of(idx);
    // A flex item is measured without a main-axis parent size; the flex
    // container's definite size is recorded for its percentage edges.
    let flex_basis = super::flex_item_percent_basis(tree, idx).unwrap_or(Size::NONE);
    let containing_inline_size = if parent.is_some_and(|parent| is_vertical(tree, parent)) {
        inputs.parent_size.height.or(flex_basis.height)
    } else {
        inputs.parent_size.width.or(flex_basis.width)
    };
    #[cfg(test)]
    {
        container_style.padding = container_style
            .padding
            .resolve_or_zero(containing_inline_size, resolve_calc)
            .map(LengthPercentage::length);
    }
    let children: Vec<_> = tree.child_ids(node_id).map(usize::from).collect();
    #[cfg(test)]
    let child_styles = tree
        .child_ids(node_id)
        .map(usize::from)
        .map(|child| {
            (
                child,
                transpose_style(
                    tree.layout_node_with_style(child).unwrap().style,
                    right_to_left,
                ),
            )
        })
        .collect();
    let shares_parent_context = block_ctx.is_some();
    let mut inputs = transpose_inputs(inputs, shares_parent_context);
    let mut block = TransposedBlock {
        id: next_transposed_frame_id(tree),
        container: idx,
        right_to_left,
        block_space: None,
        children,
        #[cfg(test)]
        container_style,
        #[cfg(test)]
        child_styles,
        container_view: None,
        child_views: HashMap::default(),
        placed: Vec::new(),
    };
    prepare_frame_views(tree, &mut block);
    if let Some(container) = &mut block.container_view {
        container.resolve_padding(containing_inline_size);
    }
    block.block_space = inputs.known_dimensions.height.map(|height| {
        #[cfg(not(test))]
        let (padding, border) = {
            let view = block
                .container_view
                .as_ref()
                .expect("production frame input")
                .view();
            (view.padding(), view.border())
        };
        #[cfg(test)]
        let (padding, border) = (block.container_style.padding, block.container_style.border);
        let padding = padding.resolve_or_zero(None, resolve_calc);
        let border = border.resolve_or_zero(None, resolve_calc);
        (height - padding.top - padding.bottom - border.top - border.bottom).max(0.0)
    });
    let (output, mut block) = with_transposed_frame(tree, block, |tree| {
        if inputs.known_dimensions.width.is_none() && shrinks_to_fit(tree, idx, &inputs) {
            inputs.known_dimensions.width = Some(fit_content_inline_size(tree, node_id, inputs));
        }
        if let Some(width) = inputs.known_dimensions.width {
            resolve_child_padding(tree, width);
        }
        compute_block_flow(tree, node_id, inputs, block_ctx)
    });
    let reverse_extent = output.size.height;
    let output =
        transpose_output_to_physical(output, right_to_left, reverse_extent, shares_parent_context);
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
fn shrinks_to_fit(tree: &LayoutSession<'_>, idx: usize, inputs: &LayoutInput) -> bool {
    inputs.sizing_mode == SizingMode::InherentSize
        && tree.physical_style(idx).size().height.is_auto()
        && !tree
            .taffy_parent_of(idx)
            .is_some_and(|parent| is_vertical(tree, parent))
}

/// The fit-content inline size of the transposed container `node_id`:
/// its max-content size, unless the available space is smaller, then that
/// space, but never below its min-content size (CSS Sizing 3 §5.1).
fn fit_content_inline_size(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
) -> f32 {
    let page_height = tree.fixed_page_area.unwrap_or_default().1;
    // The transposed horizontal margins are the physical top and bottom.
    let margin = tree
        .physical_style(usize::from(node_id))
        .margin()
        .map(|margin| margin.resolve_or_zero(inputs.parent_size.height, resolve_calc));
    let mut measure = |available: AvailableSpace| {
        let measure_inputs = LayoutInput {
            run_mode: RunMode::ComputeSize,
            axis: RequestedAxis::Horizontal,
            available_space: Size {
                width: available,
                ..inputs.available_space
            },
            vertical_margins_are_collapsible: Line::FALSE,
            ..inputs
        };
        compute_block_flow(tree, node_id, measure_inputs, None)
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

#[cfg(test)]
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

#[cfg(test)]
mod tests;
