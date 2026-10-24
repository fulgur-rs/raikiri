//! Axis adapter for a flex or grid container laid out on the axes of its own
//! writing mode.
//!
//! Taffy's flex and grid algorithms know one pair of axes: x is the inline
//! axis and y the block axis of `horizontal-tb`. A container in a vertical
//! writing mode runs the same algorithm once its style is read through the
//! container's own writing mode: a physical height is its inline size and a
//! physical width is its block size (CSS Writing Modes 4 §7.1). Those are the
//! container's *layout axes*. In `vertical-rl` the block axis runs from the
//! physical right edge to the left, so mapping a block-axis coordinate back
//! also mirrors it within the container's physical width. The inline axis
//! maps to the physical y axis as it is: a reversed inline direction
//! (`direction: rtl`, `sideways-lr`) is part of the style the algorithm reads,
//! and it places the items from the far end of that axis itself.
//!
//! Everything that crosses between the algorithm and the rest of the layout
//! goes through [`AxisAdapter`]:
//!
//! - the inputs of the container, from physical axes to layout axes;
//! - the inputs of a child, from layout axes back to the child's physical
//!   axes, and its output the other way, with the container's view set aside
//!   meanwhile so that the child measures itself on the axes of its own
//!   writing mode;
//! - the output of the container and the layouts taffy stored for its
//!   children, from layout axes to physical axes.
//!
//! The conversions are those of [`super::transposed_axes`]. The adapter fixes
//! what a flex or grid container needs on top of them: its margins never
//! collapse with a child's across a conversion, as flex and grid items
//! establish formatting contexts of their own. Baselines need no conversion:
//! a box in a vertical writing mode reports them as offsets along its block
//! axis from its block-start border edge, which is the layout y axis. What
//! the adapter adds is the baseline of an item that has none in that form
//! (see [`item_baseline`]). A scrollable overflow rectangle is mapped by
//! [`OverflowFrame`] instead of being swapped: its offsets are measured from
//! the corner of the padding box where the box starts scrolling, and in
//! physical coordinates, from the top left corner, they are mirrored within
//! the padding box wherever the block axis runs from right to left or the
//! inline axis from the bottom. A child hands over its rectangle in physical
//! coordinates too, whatever its own direction is, so the one the container
//! gets is mirrored the same way. A child whose layout still returns the
//! rectangle measured from its own scroll origin, such as a block container
//! in `vertical-rl` or a horizontal box under `direction: rtl`, is not
//! corrected here.
//!
//! Orthogonal-flow sizing of an item (stretching it, resolving its automatic
//! sizes) is not done here: the inputs reach the child as they were.
//!
//! Out-of-flow children are not placed here either. The output of a
//! container carries the positioning area and the static positions for them,
//! and they are converted to physical axes with it, so that the placement
//! that follows, once the container's frame is left, works on physical
//! coordinates.

use crate::layout::protocol::{Baselines, LayoutInput, LayoutOutput, NodeId, RunMode};
use raikiri_layout_geometry::{BoxLayout, Edges, Size};
use raikiri_layout_style::values::ResolveOrZero;
use raikiri_style::property::WritingMode;

use super::transposed_axes::{
    OverflowFrame, padding_box, physical_layout, swap, transpose_child_output_to_vertical,
    transpose_inputs, transpose_output_to_physical,
};
use super::vertical_block::{
    TransposedBlock, block_right_to_left, containing_inline_size, prepare_frame_views,
    resolve_child_padding, with_transposed_frame,
};
use super::vertical_oof::used_writing_mode;
use crate::layout::context::LayoutSession;

/// The conversions for one container whose layout axes are the axes of a
/// vertical writing mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AxisAdapter {
    /// Whether the block axis runs from right to left: `vertical-rl` and
    /// `sideways-rl`. `vertical-lr` and `sideways-lr` run it from left to
    /// right.
    right_to_left: bool,
    /// Whether taffy runs the inline axis of the container from its far end,
    /// which is what it does under a reversed direction: the scrollable
    /// overflow of the container is then measured from the physical bottom.
    inline_reversed: bool,
    /// The borders of the container on its layout axes, which bound the
    /// padding box its scrollable overflow is measured in.
    container_border: Edges<f32>,
}

/// The inputs for a child, on the child's physical axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ChildInputs {
    pub(super) inputs: LayoutInput,
    /// The inline size of the container, when it is known: the basis for the
    /// percentage padding of a child that lays out on horizontal axes (CSS
    /// Writing Modes 4 §7.1). Taffy resolves that padding against the width
    /// of the parent, which here is the block size of the container.
    pub(super) padding_basis: Option<f32>,
}

impl AxisAdapter {
    /// The adapter for a container without borders whose inline axis runs
    /// from the physical top. A frame knows more about its container: see
    /// [`Self::for_frame`].
    #[cfg(test)]
    pub(super) fn new(right_to_left: bool) -> Self {
        Self {
            right_to_left,
            inline_reversed: false,
            container_border: Edges::ZERO,
        }
    }

    /// The adapter for the container `frame` was built for.
    pub(super) fn for_frame(frame: &TransposedBlock) -> Self {
        Self {
            right_to_left: frame.right_to_left(),
            inline_reversed: frame.inline_reversed(),
            container_border: frame.container_border(),
        }
    }

    /// How a scrollable overflow rectangle maps to physical coordinates and
    /// back, for a box with the given padding box on the layout axes, in
    /// the frame of this container.
    fn overflow_frame(self, padding_box: Size<f32>) -> OverflowFrame {
        OverflowFrame {
            block_reversed: self.right_to_left,
            inline_reversed: self.inline_reversed,
            padding_box,
        }
    }

    /// The inputs of the container on its layout axes, given those on
    /// physical axes. Every per-axis field is swapped together, so a known
    /// dimension keeps its definiteness and a parent size that is known on
    /// one side only stays known on that side. The container does not share
    /// its margins with the box that called it.
    pub(super) fn container_inputs(self, inputs: LayoutInput) -> LayoutInput {
        transpose_inputs(inputs, false)
    }

    /// The output of the container on physical axes, given the one taffy
    /// produced on layout axes.
    ///
    /// The block size of the container is the extent the block axis is
    /// mirrored within in `vertical-rl`. Only an output that comes with
    /// out-of-flow data has it in full: one that answers a request for a
    /// single axis carries no such data, and its other size may be zero. The
    /// scrollable overflow rectangle comes with the layout data too; it is
    /// mirrored within the padding box, the border box less the borders of
    /// the container, and a zero one stays zero. The margins of the
    /// container are left to its parent, which reads them from the box's
    /// style. The baselines of the container stay as they are: they are
    /// offsets along the block axis already.
    pub(super) fn container_output(self, output: LayoutOutput) -> LayoutOutput {
        let extent = output.size.height;
        let overflow = self
            .overflow_frame(padding_box(output.size, self.container_border))
            .physical(output.scrollable_overflow_rect);
        let mut output = transpose_output_to_physical(output, self.right_to_left, extent, false);
        output.scrollable_overflow_rect = overflow;
        output
    }

    /// The physical layout of a child taffy placed at `layout` in a
    /// container whose physical border box is `container_width` wide. The
    /// location is measured from the left edge of the container, so in
    /// `vertical-rl` it is `container_width - block offset - child width`.
    /// The scrollable overflow of the child is mirrored within its own
    /// padding box.
    pub(super) fn placed_layout(self, layout: &BoxLayout, container_width: f32) -> BoxLayout {
        let mut placed = physical_layout(layout, container_width, self.right_to_left);
        placed.scrollable_overflow_rect = self
            .overflow_frame(padding_box(layout.size, layout.border))
            .physical(layout.scrollable_overflow_rect);
        placed
    }

    /// The inputs for a child, given those taffy built on layout axes.
    pub(super) fn child_inputs(self, inputs: LayoutInput) -> ChildInputs {
        // The swap is its own inverse. A child never shares margins with the
        // container.
        let inputs = transpose_inputs(inputs, false);
        ChildInputs {
            padding_basis: inputs.parent_size.height,
            inputs,
        }
    }

    /// The output of a child on layout axes, given the physical one. The
    /// child's own physical width is the extent its block axis is mirrored
    /// within, and the scrollable overflow rectangle of the child, which is
    /// in physical coordinates, is mirrored within its padding box:
    /// `border` holds the borders of the child on its physical sides.
    /// `baseline` is the baseline the algorithm aligns the child by, from
    /// [`item_baseline`].
    pub(super) fn child_output(
        self,
        output: LayoutOutput,
        border: Edges<f32>,
        baseline: f32,
    ) -> LayoutOutput {
        let extent = output.size.width;
        // The sides of the border on the layout axes: the physical top and
        // bottom are the inline sides, the left and right the block sides.
        let layout_border = Edges {
            left: border.top,
            right: border.bottom,
            top: border.left,
            bottom: border.right,
        };
        let overflow = self
            .overflow_frame(padding_box(swap(output.size), layout_border))
            .on_layout_axes(output.scrollable_overflow_rect);
        let mut output =
            transpose_child_output_to_vertical(output, self.right_to_left, extent, false);
        output.scrollable_overflow_rect = overflow;
        output.baselines = Baselines {
            first: Some(baseline),
            last: None,
        };
        output
    }
}

/// The baseline of the item `idx` that the algorithm of the container `frame`
/// aligns it by, as an offset along the block axis from the block-start
/// border edge of the item; `output` is the physical output of the item.
///
/// The baseline that items share is the dominant baseline of the container
/// (CSS Box Alignment 3 §9.2): the central baseline in a vertical writing
/// mode whose text is not set sideways, the alphabetic one otherwise. An item
/// that is a vertical box with the container's block direction gives the
/// baseline of its first line, on the baseline that is dominant for the
/// item's own text. Every other item, one in an orthogonal flow, one
/// without lines, or one whose lines run against the block direction of the
/// container, has its baseline synthesized from its border box: the middle of
/// the box for the central baseline, and the line-under edge of the
/// container's writing mode for the alphabetic one (CSS Align 3 §9.1).
///
/// An item with the opposite block direction forms a baseline-sharing group
/// of its own, which the container aligns after its algorithm has run; the
/// synthesized baseline is only what the algorithm sees of such an item.
/// spec: <https://www.w3.org/TR/css-align-3/#baseline-terms>
fn item_baseline(
    tree: &LayoutSession<'_>,
    frame: &TransposedBlock,
    idx: usize,
    output: &LayoutOutput,
) -> f32 {
    let same_block_direction = block_right_to_left(tree, idx) == Some(frame.right_to_left());
    if same_block_direction && let Some(baseline) = output.baselines.first {
        return baseline;
    }
    let block_size = output.size.width;
    if tree.nodes[frame.container()].central_baseline {
        block_size / 2.0
    } else if frame.own_writing_mode() == Some(WritingMode::VerticalLr) {
        // Line-under is the left edge, where the block axis starts.
        0.0
    } else {
        block_size
    }
}

impl LayoutSession<'_> {
    /// Lay out the flex or grid container `node_id` on the axes of its own
    /// writing mode, which `algorithm` runs on.
    ///
    /// `inputs` and the returned output are on physical axes. `algorithm` is
    /// given the inputs on the layout axes, with the frame of the container
    /// on the stack: taffy reads the container's style and its children's
    /// through the container's writing mode, and every child is dispatched
    /// through [`Self::compute_adapted_child`]. Out-of-flow children and the
    /// intrinsic limits of the items are left to the caller, outside the
    /// frame.
    pub(super) fn compute_on_own_axes(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        algorithm: impl FnOnce(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
    ) -> LayoutOutput {
        let idx = usize::from(node_id);
        let containing_inline_size = containing_inline_size(self, idx, &inputs);
        let children: Vec<_> = self.child_ids(node_id).map(usize::from).collect();
        let mut frame =
            TransposedBlock::on_own_axes(self, idx, children, used_writing_mode(self, idx));
        // The adapter reads the direction and the borders of the container
        // from the views of the frame.
        prepare_frame_views(self, &mut frame);
        let adapter = AxisAdapter::for_frame(&frame);
        let inputs = adapter.container_inputs(inputs);
        frame.resolve_container_padding(containing_inline_size);
        let (output, frame) = with_transposed_frame(self, frame, |tree| {
            if let Some(width) = inputs.known_dimensions.width {
                resolve_child_padding(tree, width);
            }
            algorithm(tree, node_id, inputs)
        });
        let output = adapter.container_output(output);
        for child in frame.into_placed() {
            let layout = &mut self.nodes[child].layout;
            *layout = adapter.placed_layout(layout, output.size.width);
        }
        output
    }

    /// Lay out `node_id`, a child of the container whose frame is on top of
    /// the stack, with taffy's inputs and output on the container's layout
    /// axes; `None` when that frame does not hold the child.
    ///
    /// The child is dispatched on its physical axes and measures itself on
    /// those of its own writing mode, as the container's frame is set aside
    /// while it runs. The block direction is the frame's.
    /// `pads_on_physical_axes` tells whether taffy resolves the child's
    /// percentage padding against the physical width of its parent, as it
    /// does for a child that lays its content out on horizontal axes: such a
    /// child is given the inline size of the container as the basis, while one
    /// that lays out on vertical axes finds it in the parent size it is given.
    pub(super) fn compute_adapted_child(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        pads_on_physical_axes: bool,
    ) -> Option<LayoutOutput> {
        self.with_frame_set_aside(usize::from(node_id), |tree, frame| {
            let adapter = AxisAdapter::for_frame(frame);
            let child = adapter.child_inputs(inputs);
            // Taffy measures the flex base size of an item without a parent
            // size on the main axis; the container's definite inline size is
            // recorded for the item's percentage edges.
            let padding_basis = child
                .padding_basis
                .or_else(|| {
                    super::flex_item_percent_basis(tree, usize::from(node_id))
                        .and_then(|basis| basis.height)
                })
                .filter(|_| pads_on_physical_axes);
            // Borders are lengths, so they resolve without a basis.
            let border = tree
                .physical_style(usize::from(node_id))
                .border()
                .resolve_or_zero(None);
            let output = tree.dispatch_physical_child(node_id, child.inputs, padding_basis, None);
            // An item that runs against the block direction of the container
            // is aligned by the container afterwards, by the baseline of its
            // own first line.
            let item = usize::from(node_id);
            if tree.nodes[item].opposed_baseline_group
                && child.inputs.run_mode == RunMode::PerformLayout
            {
                tree.nodes[item].own_first_baseline = output.baselines.first;
            }
            let baseline = item_baseline(tree, frame, item, &output);
            adapter.child_output(output, border, baseline)
        })
    }
}

#[cfg(test)]
mod tests;
