//! The rendered legend of a `fieldset` (HTML Living Standard, "Rendering"
//! §15.3.13 "The fieldset and legend elements").
//!
//! The rendered legend is the first `legend` child of a `fieldset` that is
//! neither floated nor absolutely positioned. It sits over the block-start
//! border of the fieldset instead of in its content: its border box is
//! centred on that border when it is thinner, and otherwise starts at the
//! border-box edge with the border centred on it. Its block-start margin is
//! not used, as in Chromium. The content of the fieldset starts below the
//! farther of the border and the legend's margin box, then the padding.

use super::children::{margins, overflow, place_inline_axis, relative};
use super::{Metrics, child_inputs, child_style, commit};
use crate::layout::context::LayoutSession;
use crate::layout::dispatch::used_writing_mode;
use crate::layout::protocol::{AvailableSpace, LayoutInput, NodeId, RunMode};
use raikiri_layout_geometry::{Edges, Point, Size};
use raikiri_layout_style::BoxStyle as Style;
use raikiri_layout_style::values::{BoxGenerationMode, Float, Position};
use raikiri_traits::PaintRect;

/// The rendered legend among the layout children of `node`, if `node` is an
/// HTML `fieldset` that has one.
pub(super) fn rendered_legend(tree: &LayoutSession<'_>, node: NodeId) -> Option<NodeId> {
    if !tree.nodes[usize::from(node)].is_html_fieldset() {
        return None;
    }
    tree.child_ids(node).find(|&child| {
        let child_node = &tree.nodes[usize::from(child)];
        if child_node.tag_name() != Some("legend") || child_node.namespace_uri().is_some() {
            return false;
        }
        let style = child_style(tree, child);
        style.box_generation_mode() != BoxGenerationMode::None
            && style.float == Float::None
            && !matches!(style.position, Position::Absolute | Position::Fixed)
    })
}

/// A rendered legend laid out over the block-start border of its fieldset.
pub(super) struct PlacedLegend {
    /// How far the content of the fieldset moves down past its own
    /// block-start border and padding to clear the legend.
    pub content_offset: f32,
    /// The scrollable overflow rectangle the legend adds to the fieldset.
    pub overflow: Edges<f32>,
}

/// Lay out the rendered `legend` of `fieldset`, whose border box is
/// `width` wide and whose content box is `content_width` wide, and place it
/// over the fieldset's block-start border.
#[allow(clippy::too_many_arguments)]
pub(super) fn place(
    tree: &mut LayoutSession<'_>,
    fieldset: NodeId,
    legend: NodeId,
    order: usize,
    inputs: LayoutInput,
    container: &Style,
    metrics: Metrics,
    width: f32,
    parent: Size<Option<f32>>,
) -> PlacedLegend {
    let content_width = parent.width.unwrap_or(0.0);
    let style = child_style(tree, legend);
    let mut margin = margins(&style, content_width);
    let room = (content_width - margin.horizontal_axis_sum()).max(0.0);
    let mut child_input = child_inputs(parent, None, inputs.run_mode);
    let mut metrics_input = child_input;
    metrics_input.available_space.width = AvailableSpace::Definite(room);
    let legend_metrics = Metrics::resolve(
        &style,
        tree.nodes[usize::from(legend)].svg_width_follows_available,
        tree.nodes[usize::from(legend)].ratio_in_content_box,
        metrics_input,
    );
    // An `auto` inline size is the fit-content size in the room the
    // fieldset's content box leaves.
    let width_of = |tree: &mut LayoutSession<'_>, space: AvailableSpace| {
        let mut measure = child_input;
        measure.run_mode = RunMode::ComputeSize;
        measure.available_space.width = space;
        tree.compute_child_layout_with_block_ctx(legend, measure, None)
            .size
            .width
    };
    child_input.known_dimensions.width = Some(
        match legend_metrics
            .preferred
            .width
            .filter(|_| !legend_metrics.ratio_width)
        {
            Some(width) => width,
            None => {
                let maximum = width_of(tree, AvailableSpace::MaxContent);
                let minimum = width_of(tree, AvailableSpace::MinContent);
                legend_metrics.clamp_width(maximum.min(room).max(minimum))
            }
        },
    );
    child_input.known_dimensions.height = legend_metrics
        .preferred
        .height
        .filter(|_| !legend_metrics.ratio_height);
    child_input.available_space.width = AvailableSpace::Definite(room);
    let output = tree.compute_child_layout_with_block_ctx(legend, child_input, None);
    let left = place_inline_axis(
        &style,
        container.direction,
        width,
        [metrics.inset.left, metrics.inset.right],
        (metrics.inset.left + margin.left, room),
        output.size.width,
        &mut margin,
    );
    let border = metrics.border.top;
    let size = output.size.height;
    let top = ((border - size) / 2.0).max(0.0);
    let content_offset = (top + size + margin.bottom - border).max(0.0);
    let offset = relative(&style, parent, container.direction);
    let location = Point {
        x: left + offset.x,
        y: top + offset.y,
    };
    if inputs.run_mode == RunMode::PerformLayout {
        commit(
            tree,
            legend,
            order,
            legend_metrics,
            &output,
            location,
            margin,
        );
        // The legend hides the block-start border where it lies, before its
        // relative offset moves it. On transposed axes the rectangle would
        // not be physical, so a vertical fieldset keeps its whole border.
        tree.nodes[usize::from(fieldset)].legend_cutout =
            (!used_writing_mode(tree, usize::from(fieldset)).is_vertical())
                .then(|| PaintRect::new(left, top, output.size.width, output.size.height));
    }
    PlacedLegend {
        content_offset,
        overflow: overflow(&style, legend_metrics, &output, location),
    }
}
