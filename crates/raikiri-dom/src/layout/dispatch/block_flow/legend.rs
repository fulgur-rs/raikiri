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
use crate::layout::dispatch::content_limits;
use crate::layout::dispatch::used_writing_mode;
use crate::layout::protocol::{AvailableSpace, NodeId, OofCandidates, RunMode};
use raikiri_layout_geometry::{Edges, Point, Size};
use raikiri_layout_style::BoxStyle as Style;
use raikiri_layout_style::values::{
    BoxGenerationMode, Direction, ExpandedDimension, Float, Overflow, Position,
};
use raikiri_style::property::TextAlign;
use raikiri_traits::PaintRect;

/// The rendered legend among the layout children of `node`, the container
/// whose style is `container`, if `node` is an HTML `fieldset` that has one
/// and lays it out over its border.
///
/// A fieldset in a vertical writing mode, and one that clips its overflow,
/// keep the legend as an ordinary block for now: the first would need the
/// legend over its block-start border on the physical axes, and the second a
/// content box of its own that clips and scrolls without the legend.
pub(super) fn rendered_legend(
    tree: &LayoutSession<'_>,
    node: NodeId,
    container: &Style,
) -> Option<NodeId> {
    if !tree.nodes[usize::from(node)].is_html_fieldset()
        || used_writing_mode(tree, usize::from(node)).is_vertical()
        || container.overflow.x != Overflow::Visible
        || container.overflow.y != Overflow::Visible
    {
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

/// Where the `text-align` of the rendered `legend`, `width` wide, puts its
/// border box along the block-start border of a fieldset of `direction`:
/// `center` centres it in the content box of the fieldset, and the side
/// opposite the start (`right` in a left-to-right fieldset, `left` in a
/// right-to-left one) puts it at the end, inside its margins. The start and
/// the other values leave it at the start. An `auto` margin places the
/// legend instead. This follows Chromium, which reads `text-align` here, and
/// so maps the `align` attribute of the legend to it; the HTML Standard
/// names `justify-self` for that attribute, which Chromium ignores on a
/// legend. `inset` is the border plus padding of the fieldset on its left
/// and right, and `content_width` the width of its content box.
fn aligned_left(
    tree: &LayoutSession<'_>,
    legend: NodeId,
    style: &Style,
    direction: Direction,
    (inset, content_width): ([f32; 2], f32),
    width: f32,
    margin: Edges<f32>,
) -> Option<f32> {
    if style.margin.left.is_auto() || style.margin.right.is_auto() {
        return None;
    }
    let free = content_width - width;
    let rtl = direction == Direction::Rtl;
    match tree.style_views.cascade().computed[usize::from(legend)].text_align {
        TextAlign::Center if rtl => {
            Some(inset[0] + content_width - margin.right - width - free / 2.0)
        }
        TextAlign::Center => Some(inset[0] + margin.left + free / 2.0),
        TextAlign::Right if !rtl => Some(inset[0] + content_width - margin.right - width),
        TextAlign::Left if rtl => Some(inset[0] + margin.left),
        _ => None,
    }
}

/// A rendered legend laid out over the block-start border of its fieldset.
pub(crate) struct PlacedLegend {
    /// The top of the border box of the legend, from the top of the border
    /// box of the fieldset, before any relative offset.
    pub top: f32,
    /// How far the content of the fieldset moves down past its own
    /// block-start border and padding to clear the legend.
    pub content_offset: f32,
    /// The scrollable overflow rectangle the legend adds to the fieldset.
    pub overflow: Edges<f32>,
    /// The absolutely positioned descendants of the legend that the fieldset
    /// passes on, in the fieldset's coordinates.
    pub oof_candidates: OofCandidates,
}

/// Lay out the rendered `legend` of `fieldset`, whose border box is `width`
/// wide and whose content box is `parent.width` wide, and place it over the
/// fieldset's block-start border. `inset` is the border plus padding of the
/// fieldset on its left and right, and `border` its block-start border.
#[allow(clippy::too_many_arguments)]
pub(super) fn place(
    tree: &mut LayoutSession<'_>,
    fieldset: NodeId,
    legend: NodeId,
    order: usize,
    run_mode: RunMode,
    container: &Style,
    (inset, border): ([f32; 2], f32),
    width: f32,
    parent: Size<Option<f32>>,
) -> PlacedLegend {
    let content_width = parent.width.unwrap_or(0.0);
    let mut style = child_style(tree, legend);
    let mut margin = margins(&style, content_width);
    let room = (content_width - margin.horizontal_axis_sum()).max(0.0);
    let mut child_input = child_inputs(parent, None, run_mode);
    let mut metrics_input = child_input;
    metrics_input.available_space.width = AvailableSpace::Definite(room);
    // A keyword `min-width` or `max-width` limits the width, as it does for
    // the other children.
    content_limits::resolve_width_in_style(tree, legend, &mut style, &metrics_input);
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
                legend_metrics.clamp_width(match style.size.width.expand() {
                    ExpandedDimension::MinContent => minimum,
                    ExpandedDimension::MaxContent => maximum,
                    _ => maximum.min(room).max(minimum),
                })
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
        inset,
        (inset[0] + margin.left, room),
        output.size.width,
        None,
        &mut margin,
    );
    let left = aligned_left(
        tree,
        legend,
        &style,
        container.direction,
        (inset, content_width),
        output.size.width,
        margin,
    )
    .unwrap_or(left);
    let size = output.size.height;
    let top = ((border - size) / 2.0).max(0.0);
    let content_offset = (top + size + margin.bottom - border).max(0.0);
    let offset = relative(&style, parent, container.direction);
    let location = Point {
        x: left + offset.x,
        y: top + offset.y,
    };
    let mut oof_candidates = OofCandidates::new();
    if run_mode == RunMode::PerformLayout {
        if !output.oof_candidates.is_empty() {
            oof_candidates = output.oof_candidates.clone();
            oof_candidates.translate(location);
        }
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
        // relative offset moves it.
        tree.nodes[usize::from(fieldset)].legend_cutout = Some(PaintRect::new(
            left,
            top,
            output.size.width,
            output.size.height,
        ));
    }
    PlacedLegend {
        top,
        content_offset,
        overflow: overflow(&style, legend_metrics, &output, location),
        oof_candidates,
    }
}

/// Lay out the rendered `legend` of a fieldset whose content is a paragraph,
/// as the line loop of the paragraph meets it, its first box. The content box
/// of the fieldset is `parent`, its border plus padding on the left and right
/// `inset`, and its block-start border `border`.
pub(crate) fn place_in_paragraph(
    tree: &mut LayoutSession<'_>,
    legend: NodeId,
    run_mode: RunMode,
    parent: Size<Option<f32>>,
    inset: [f32; 2],
    border: f32,
) -> PlacedLegend {
    let fieldset = NodeId::from(
        tree.document
            .parent_of(usize::from(legend))
            .unwrap_or(usize::from(legend)),
    );
    let container = child_style(tree, fieldset);
    let order = tree
        .child_ids(fieldset)
        .position(|child| child == legend)
        .unwrap_or(0);
    let width = parent.width.unwrap_or(0.0) + inset[0] + inset[1];
    place(
        tree,
        fieldset,
        legend,
        order,
        run_mode,
        &container,
        (inset, border),
        width,
        parent,
    )
}
