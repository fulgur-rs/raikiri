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
//! establish formatting contexts of their own. Baselines are dropped by the
//! conversion, and a scrollable overflow rectangle is swapped without being
//! mirrored.
//!
//! Orthogonal-flow sizing of an item (stretching it, resolving its automatic
//! sizes) is not done here: the inputs reach the child as they were.
//!
//! Out-of-flow children are not placed here either. The output of a
//! container carries the positioning area and the static positions for them,
//! and they are converted to physical axes with it, so that the placement
//! that follows, once the container's frame is left, works on physical
//! coordinates.

use taffy::{Layout, LayoutInput, LayoutOutput, NodeId};

use super::transposed_axes::{
    physical_layout, transpose_child_output_to_vertical, transpose_inputs,
    transpose_output_to_physical,
};
use super::vertical_block::TransposedBlock;
use crate::layout::context::LayoutSession;

/// The conversions for one container whose layout axes are the axes of a
/// vertical writing mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AxisAdapter {
    /// Whether the block axis runs from right to left: `vertical-rl` and
    /// `sideways-rl`. `vertical-lr` and `sideways-lr` run it from left to
    /// right.
    right_to_left: bool,
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
    pub(super) fn new(right_to_left: bool) -> Self {
        Self { right_to_left }
    }

    /// The adapter for the container `frame` was built for.
    pub(super) fn for_frame(frame: &TransposedBlock) -> Self {
        Self::new(frame.right_to_left())
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
    /// margins of the container are left to its parent, which reads them from
    /// the box's style.
    pub(super) fn container_output(self, output: LayoutOutput) -> LayoutOutput {
        let extent = output.size.height;
        transpose_output_to_physical(output, self.right_to_left, extent, false)
    }

    /// The physical layout of a child taffy placed at `layout` in a
    /// container whose physical border box is `container_width` wide. The
    /// location is measured from the left edge of the container, so in
    /// `vertical-rl` it is `container_width - block offset - child width`.
    pub(super) fn placed_layout(self, layout: &Layout, container_width: f32) -> Layout {
        physical_layout(layout, container_width, self.right_to_left)
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
    /// within.
    pub(super) fn child_output(self, output: LayoutOutput) -> LayoutOutput {
        let extent = output.size.width;
        transpose_child_output_to_vertical(output, self.right_to_left, extent, false)
    }
}

impl LayoutSession<'_> {
    /// Lay out `node_id`, a child of the container whose frame is on top of
    /// the stack, with taffy's inputs and output on the container's layout
    /// axes; `None` when that frame does not hold the child.
    ///
    /// The child is dispatched on its physical axes and measures itself on
    /// those of its own writing mode, as the container's frame is set aside
    /// while it runs. The block direction is the frame's.
    /// `flows_horizontally` tells whether the child lays its content out on
    /// horizontal axes: such a child is given the inline size of the
    /// container as the basis for its percentage padding, while one that lays
    /// out on vertical axes finds it in the parent size it is given.
    pub(super) fn compute_adapted_child(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        flows_horizontally: bool,
    ) -> Option<LayoutOutput> {
        self.with_frame_set_aside(usize::from(node_id), |tree, frame| {
            let adapter = AxisAdapter::for_frame(frame);
            let child = adapter.child_inputs(inputs);
            let padding_basis = child.padding_basis.filter(|_| flows_horizontally);
            let output = tree.dispatch_physical_child(node_id, child.inputs, padding_basis, None);
            adapter.child_output(output)
        })
    }
}

#[cfg(test)]
mod tests;
