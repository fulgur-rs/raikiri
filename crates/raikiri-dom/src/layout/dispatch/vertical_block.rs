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

use crate::layout::protocol::{
    AvailableSpace, Baselines, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode,
    SizingMode,
};
use crate::layout::taffy_adapter::{MaybeResolve, ResolveOrZero};
use raikiri_layout_geometry::{Edges, Line, Size};
use raikiri_layout_style::values::TaffyPosition;
use taffy::BoxSizing;

use super::block_flow::compute_block_flow;
use super::resolve_calc;
use super::transposed_axes::{
    physical_layout, transpose_child_output_to_vertical, transpose_inputs,
    transpose_output_to_physical,
};
use super::vertical_oof::used_writing_mode;
use crate::document::Document;
use crate::node::FormattingContext;
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
    container_view: Option<crate::layout::style_views::frozen::FrozenViewInput>,
    child_views: HashMap<usize, crate::layout::style_views::frozen::FrozenViewInput>,
    /// The children taffy stored a transposed layout for.
    placed: Vec<usize>,
    /// The writing mode whose axes the views of the frame are read on, when
    /// the container is a flex or grid container laid out on the axes of its
    /// own writing mode. A block container reads them with the physical axes
    /// swapped instead.
    own_axes: Option<WritingMode>,
}

impl TransposedBlock {
    /// A frame for `container` and its `children`, whose block axis runs
    /// from right to left when `right_to_left` is set. It holds no views
    /// until `prepare_frame_views` runs, which pushing the frame does.
    pub(super) fn new(
        tree: &mut LayoutSession<'_>,
        container: usize,
        children: Vec<usize>,
        right_to_left: bool,
    ) -> Self {
        Self {
            id: next_transposed_frame_id(tree),
            container,
            right_to_left,
            block_space: None,
            children,
            container_view: None,
            child_views: HashMap::default(),
            placed: Vec::new(),
            own_axes: None,
        }
    }

    /// A frame for the flex or grid container `container` and its `children`,
    /// whose views are read on the axes of the writing mode `writing_mode`
    /// of the container. The block axis runs from right to left in
    /// `vertical-rl` and `sideways-rl`.
    pub(super) fn on_own_axes(
        tree: &mut LayoutSession<'_>,
        container: usize,
        children: Vec<usize>,
        writing_mode: WritingMode,
    ) -> Self {
        let right_to_left = matches!(
            writing_mode,
            WritingMode::VerticalRl | WritingMode::SidewaysRl
        );
        Self {
            own_axes: Some(writing_mode),
            ..Self::new(tree, container, children, right_to_left)
        }
    }

    /// Whether the block axis of the container runs from right to left.
    pub(super) fn right_to_left(&self) -> bool {
        self.right_to_left
    }

    /// Whether taffy runs the inline axis of the container from its far end:
    /// the direction of the container on its layout axes is `rtl`, which a
    /// reversed inline direction in a vertical writing mode produces too.
    /// `false` until the frame holds its views.
    pub(super) fn inline_reversed(&self) -> bool {
        self.container_view
            .as_ref()
            .is_some_and(|input| input.view().direction().is_rtl())
    }

    /// The borders of the container on its layout axes; zero until the frame
    /// holds its views.
    pub(super) fn container_border(&self) -> Edges<f32> {
        self.container_view.as_ref().map_or(Edges::ZERO, |input| {
            input.view().border().resolve_or_zero(None, resolve_calc)
        })
    }

    /// Whether the container is laid out on the axes of its own writing mode:
    /// a flex or grid container, as opposed to a block container.
    pub(super) fn is_on_own_axes(&self) -> bool {
        self.own_axes.is_some()
    }

    /// The container the frame was built for.
    pub(super) fn container(&self) -> usize {
        self.container
    }

    /// The writing mode the container is laid out on the axes of, when it is
    /// a flex or grid container.
    pub(super) fn own_writing_mode(&self) -> Option<WritingMode> {
        self.own_axes
    }

    /// Resolve the percentages of the padding of the container against
    /// `basis`, the inline size of its containing block. The views of the
    /// frame must be prepared.
    pub(super) fn resolve_container_padding(&mut self, basis: Option<f32>) {
        if let Some(container) = &mut self.container_view {
            container.resolve_padding(basis);
        }
    }

    /// The children taffy stored a layout for on the axes of the frame, each
    /// once.
    pub(super) fn into_placed(mut self) -> Vec<usize> {
        // Taffy may store a child's layout more than once; the last one stands.
        self.placed.sort_unstable();
        self.placed.dedup();
        self.placed
    }

    fn has_child(&self, idx: usize) -> bool {
        self.child_views.contains_key(&idx)
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

pub(super) fn prepare_frame_views(tree: &mut LayoutSession<'_>, block: &mut TransposedBlock) {
    use crate::layout::style_views::frozen::FrozenViewInput;
    use raikiri_layout_style::{BridgeProjection, ClaimsPolicy, ViewContext};
    if block.container_view.is_some() {
        return;
    }
    let context = match block.own_axes {
        Some(layout_wm) => ViewContext {
            layout_wm,
            claims_policy: ClaimsPolicy::Positioning,
            ..ViewContext::default()
        },
        None => ViewContext {
            bridge_projection: BridgeProjection::BlockTranspose {
                right_to_left: block.right_to_left,
            },
            claims_policy: ClaimsPolicy::Positioning,
            ..ViewContext::default()
        },
    };
    block.container_view = Some(FrozenViewInput::capture(tree, block.container, context));
    for &child in &block.children {
        let input = FrozenViewInput::capture(tree, child, context);
        block.child_views.insert(child, input);
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
                    view_previous: None,
                });
            }
        }
        undo
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
    /// Run `f` with the frame on top of the stack set aside, when that frame
    /// holds the child `idx`; `None` otherwise.
    ///
    /// The container's view is out of the stack meanwhile, so that the child
    /// and its own descendants see their physical styles. The frame returns
    /// to the stack when `f` unwinds too.
    pub(super) fn with_frame_set_aside<R>(
        &mut self,
        idx: usize,
        f: impl FnOnce(&mut Self, &TransposedBlock) -> R,
    ) -> Option<R> {
        self.context
            .transposed_blocks
            .last()
            .filter(|block| block.has_child(idx))?;
        let block = self.context.transposed_blocks.pop()?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self, &block)));
        self.context.transposed_blocks.push(block);
        match result {
            Ok(result) => Some(result),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Lay out the child `node_id` of a vertical container, whose `inputs`
    /// are already on physical axes.
    ///
    /// Taffy resolves a box's percentage padding against the width of its
    /// parent, but the containing block of a child that lays out on
    /// horizontal axes is the vertical container, whose inline size is its
    /// physical height (CSS Writing Modes 4 §7.1). That inline size is the
    /// `padding_basis` such a child is given.
    pub(super) fn dispatch_physical_child(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        padding_basis: Option<f32>,
        block_ctx: Option<&mut super::block_flow::context::BlockContext<'_>>,
    ) -> LayoutOutput {
        if let Some(basis) = padding_basis {
            self.with_resolved_edges(usize::from(node_id), basis, false, |tree| {
                tree.compute_child_layout_with_block_ctx(node_id, inputs, block_ctx)
            })
        } else {
            self.compute_child_layout_with_block_ctx(node_id, inputs, block_ctx)
        }
    }

    /// Lay out `idx` on its physical axes when taffy asks for it as a child
    /// of the transposed container being laid out; `None` otherwise.
    pub(super) fn compute_transposed_child(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_ctx: Option<&mut super::block_flow::context::BlockContext<'_>>,
    ) -> Option<LayoutOutput> {
        let idx = usize::from(node_id);
        // An item of a flex or grid container shares no block formatting
        // context with it, and its sizes are not those of a block child.
        if self
            .context
            .transposed_blocks
            .last()
            .is_some_and(|block| block.has_child(idx) && block.is_on_own_axes())
        {
            let resolves_padding_on_physical_axes = resolves_padding_on_physical_axes(self, idx);
            return self.compute_adapted_child(node_id, inputs, resolves_padding_on_physical_axes);
        }
        self.with_frame_set_aside(idx, |tree, block| {
            let right_to_left = block_right_to_left(tree, block.container).unwrap_or(false);
            let share_context =
                block_ctx.is_some() && same_vertical_block_mode(tree, block.container, idx);
            let inputs = orthogonal_child_inputs(
                tree,
                idx,
                transpose_inputs(inputs, share_context),
                block.block_space,
            );
            // A vertical child transposes its own inputs and needs no help
            // with its percentage padding.
            let physical_padding_basis = resolves_padding_on_physical_axes(tree, idx)
                .then_some(inputs.parent_size.height)
                .flatten();
            let child_block_ctx = if share_context { block_ctx } else { None };
            let output = tree.dispatch_physical_child(
                node_id,
                inputs,
                physical_padding_basis,
                child_block_ctx,
            );
            let reverse_extent = output.size.width;
            let mut output = transpose_child_output_to_vertical(
                output,
                right_to_left,
                reverse_extent,
                share_context,
            );
            // A child's baselines are offsets from its own block-start edge,
            // which is this container's only when the block axes agree.
            if !same_vertical_block_mode(tree, block.container, idx) {
                output.baselines = Baselines::NONE;
            }
            output
        })
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
/// on horizontal axes: a horizontal box.
fn flows_horizontally(tree: &Document, idx: usize) -> bool {
    !is_vertical(tree, idx)
}

/// Whether taffy resolves the percentage padding of the child `idx` of a
/// vertical container against the physical width of its parent: a child that
/// flows horizontally, or a box without children that is measured as a leaf
/// whatever its writing mode.
fn resolves_padding_on_physical_axes(tree: &Document, idx: usize) -> bool {
    let node = &tree.nodes[idx];
    flows_horizontally(tree, idx)
        || (node.children.is_empty()
            && !matches!(
                node.formatting_context,
                FormattingContext::Flex
                    | FormattingContext::Grid
                    | FormattingContext::Table
                    | FormattingContext::None
            ))
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
            && style.float() == taffy::Float::None
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
fn stretched_width(
    style: &raikiri_layout_style::LayoutStyleView<'_>,
    space: f32,
    inputs: LayoutInput,
) -> f32 {
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
pub(super) fn resolve_child_padding(tree: &mut LayoutSession<'_>, width: f32) {
    let block = tree
        .context
        .transposed_blocks
        .last_mut()
        .expect("the container's block is pushed");
    let container = block
        .container_view
        .as_ref()
        .expect("a frame holds view input");
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

pub(super) fn with_transposed_frame<R>(
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

/// The inline size of the containing block of the vertical container `idx`,
/// which the percentages of its padding refer to (CSS Writing Modes 4 §7.1):
/// the physical height of a vertical parent, the physical width of a
/// horizontal one. `inputs` are on physical axes.
pub(super) fn containing_inline_size(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: &LayoutInput,
) -> Option<f32> {
    // A flex item is measured without a main-axis parent size; the flex
    // container's definite size is recorded for its percentage edges.
    let flex_basis = super::flex_item_percent_basis(tree, idx).unwrap_or(Size::NONE);
    if tree
        .taffy_parent_of(idx)
        .is_some_and(|parent| is_vertical(tree, parent))
    {
        inputs.parent_size.height.or(flex_basis.height)
    } else {
        inputs.parent_size.width.or(flex_basis.width)
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
    let containing_inline_size = containing_inline_size(tree, idx, &inputs);
    let children: Vec<_> = tree.child_ids(node_id).map(usize::from).collect();
    let shares_parent_context = block_ctx.is_some();
    let mut inputs = transpose_inputs(inputs, shares_parent_context);
    let mut block = TransposedBlock::new(tree, idx, children, right_to_left);
    prepare_frame_views(tree, &mut block);
    block.resolve_container_padding(containing_inline_size);
    block.block_space = inputs.known_dimensions.height.map(|height| {
        let (padding, border) = {
            let view = block
                .container_view
                .as_ref()
                .expect("a frame holds view input")
                .view();
            (view.padding(), view.border())
        };
        let padding = padding.resolve_or_zero(None, resolve_calc);
        let border = border.resolve_or_zero(None, resolve_calc);
        (height - padding.top - padding.bottom - border.top - border.bottom).max(0.0)
    });
    let (output, block) = with_transposed_frame(tree, block, |tree| {
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
    for child in block.into_placed() {
        let layout = &mut tree.nodes[child].layout;
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
mod tests;
