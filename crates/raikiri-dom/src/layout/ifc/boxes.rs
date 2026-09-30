//! Lay out a paragraph together with the boxes that sit in its lines.

use super::flow::{FlowGeometry, LineSpace, MAX_SPACE_RETRIES, line_space, resolve_indent};
use super::root::{IfcLines, IfcRoot, with_state};
use crate::Document;
use crate::taffy_impl::resolve_calc;
use shodo::{
    AtomicIntrinsics, AtomicSizes, FloatClear, FloatCursor, FloatIntrinsic, FloatSide,
    LineConstraint, LineResult,
};
use taffy::{
    AvailableSpace, BlockContext, BlockFormattingContext, Clear, FloatDirection, LayoutInput,
    LayoutPartialTree, Line, RequestedAxis, RunMode, Size, SizingMode,
};

/// A child of an ifc root that is not part of the paragraph's text: it is
/// laid out as a box of its own and painted by the walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IfcBox {
    pub(crate) node: usize,
    pub(crate) kind: IfcBoxKind,
}

/// How a box sits in the paragraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IfcBoxKind {
    Float,
}

/// A float met while laying out the current line. It is not in the context
/// yet: `place_floated_box` cannot be undone, so it is only recorded here
/// until the line is final.
#[derive(Clone, Debug)]
pub(crate) struct TentativeFloat {
    pub(crate) node: usize,
    pub(crate) direction: FloatDirection,
    pub(crate) clear: Clear,
    pub(crate) margin_box: Size<f32>,
    pub(crate) margin: taffy::Rect<f32>,
    /// What `perform_child_layout` returned for the float.
    pub(crate) output: taffy::LayoutOutput,
}

/// Lay the paragraph out with its boxes against `block_ctx` (the parent's
/// context) or a context of its own. When `perform` is true, floats are
/// committed to the context and the boxes get their final layouts.
pub(crate) fn layout_with_boxes(
    tree: &mut Document,
    idx: usize,
    geometry: FlowGeometry,
    block_ctx: Option<&mut BlockContext<'_>>,
    perform: bool,
) -> IfcLines {
    let Some(root) = tree.nodes[idx]
        .ifc
        .as_ref()
        .map(|root| root.without_lines())
    else {
        return IfcLines {
            width: geometry.width,
            lines: Vec::new(),
            height: 0.0,
            beside_floats: false,
        };
    };
    // One inner function takes the context from both arms, as in
    // `flow::layout_flow`.
    match block_ctx {
        Some(ctx) => run_boxes(tree, &root, geometry, ctx, perform, false),
        None => {
            let mut bfc = BlockFormattingContext::new();
            let mut ctx = bfc.root_block_context();
            ctx.set_width(geometry.width + geometry.edges.0 + geometry.edges.1);
            run_boxes(tree, &root, geometry, &mut ctx, perform, true)
        }
    }
}

/// The line loop. `local` is true when the context is this paragraph's own,
/// which makes the paragraph a block formatting context that contains its
/// floats.
fn run_boxes(
    tree: &mut Document,
    root: &IfcRoot,
    geometry: FlowGeometry,
    ctx: &mut BlockContext<'_>,
    perform: bool,
    local: bool,
) -> IfcLines {
    let FlowGeometry {
        width,
        edges,
        top_edge,
    } = geometry;
    let mut options = root.options;
    options.text_indent.length = resolve_indent(root.indent, width);
    // Keeps the paragraph's own floats inside its content box.
    ctx.apply_content_box_inset([edges.0, edges.1]);

    let mut token = root.paragraph.start_token();
    let mut cursor: Option<FloatCursor> = None;
    let mut y = 0.0_f32;
    let mut lines: Vec<shodo::Line> = Vec::new();
    let mut tentative: Vec<TentativeFloat> = Vec::new();
    let mut deferred: Vec<TentativeFloat> = Vec::new();
    let mut withdrawn: Vec<usize> = Vec::new();
    let mut assumed_height = 0.0_f32;
    let mut retries = 0;
    let mut beside = false;
    // Every float of a line is met, may be withdrawn and is met again, and
    // the line may be retried for its height; more calls than that for one
    // line mean the loop does not converge.
    let max_calls = 3 * root.boxes.len() + MAX_SPACE_RETRIES + 2;
    let mut calls = 0;

    loop {
        calls += 1;
        if calls > max_calls {
            debug_assert!(false, "the line loop does not converge");
            break;
        }
        let base = line_space(ctx, edges.0, top_edge, width, y, assumed_height);
        let space = shorten(base, &tentative);
        let mut constraint = LineConstraint::new(space.width);
        constraint.inline_start_offset = space.start;
        constraint.block_offset = y;
        constraint.floats_placed_through = cursor;
        // The state is taken only for the engine call: measuring a float
        // below lays out another node, which may be an ifc root itself.
        let result = with_state(tree, |state| {
            root.paragraph.next_line(
                &mut state.layout_cx,
                token,
                &options,
                &constraint,
                &AtomicSizes::EMPTY,
            )
        });
        let Some(result) = result else { break };
        match result {
            LineResult::FloatEncountered {
                node, float_cursor, ..
            } => {
                let node = node.0 as usize;
                let float = measure_float(tree, node, geometry);
                // `space` already has the tentative floats taken off.
                let fits = float.margin_box.width <= space.width;
                let below = float.clear != Clear::None
                    && ctx
                        .cleared_threshold(float.clear)
                        .is_some_and(|bottom| bottom > top_edge + y);
                if withdrawn.contains(&node) || !fits || below {
                    deferred.push(float);
                } else {
                    tentative.push(float);
                }
                cursor = Some(float_cursor);
            }
            LineResult::Line(line) => {
                if let Some(&(node, earlier)) = line.displaced_floats().last() {
                    // Placing the float moved its anchor to a later line.
                    // Take it back and lay the line out again without it.
                    let node = node.0 as usize;
                    tentative.retain(|f| f.node != node);
                    deferred.retain(|f| f.node != node);
                    withdrawn.push(node);
                    cursor = earlier.before();
                    continue;
                }
                let height = line.block_size();
                // The line may span more of an outer float than the height it
                // was assumed to have; lay it out again if the space for its
                // real height differs.
                let real = shorten(
                    line_space(ctx, edges.0, top_edge, width, y, height),
                    &tentative,
                );
                if retries < MAX_SPACE_RETRIES && real != space {
                    assumed_height = height;
                    retries += 1;
                    continue;
                }
                if space.start != 0.0 || space.width < width {
                    beside = true;
                }
                for float in tentative.drain(..) {
                    commit_float(tree, ctx, float, y, geometry, perform);
                }
                withdrawn.clear();
                assumed_height = 0.0;
                retries = 0;
                calls = 0;
                y += height;
                token = line.break_token();
                lines.push(line);
                // Floats that did not fit beside the line they are anchored
                // in go at the start of the next line, before it is measured.
                // Placing them any earlier would narrow the line that is
                // still being retried.
                for float in deferred.drain(..) {
                    commit_float(tree, ctx, float, y, geometry, perform);
                }
            }
            LineResult::Done => break,
            other => {
                // Atomic inlines and blocks are not projected into
                // paragraphs laid out here, so no other result is expected.
                debug_assert!(false, "unexpected line result: {other:?}");
                break;
            }
        }
    }
    // A float met after the last line still gets a place.
    for float in tentative.drain(..).chain(deferred.drain(..)) {
        commit_float(tree, ctx, float, y, geometry, perform);
    }
    // A paragraph that is its own formatting context contains its floats; one
    // that is not leaves them hanging below, and the parent collects them.
    // The context measures from the border-box top, the height from the
    // content-box top, hence the subtraction.
    let height = if local {
        let floats_bottom = ctx.floated_content_height_contribution();
        if floats_bottom.is_finite() {
            y.max(floats_bottom - top_edge)
        } else {
            y
        }
    } else {
        y
    };
    IfcLines {
        width,
        height,
        lines,
        beside_floats: beside,
    }
}

/// Take the tentative floats' margin boxes off the space of a line: a left
/// float moves the start right and takes its width off; a right float only
/// takes its width off.
fn shorten(base: LineSpace, tentative: &[TentativeFloat]) -> LineSpace {
    let mut space = base;
    for float in tentative {
        match float.direction {
            FloatDirection::Left => {
                space.start += float.margin_box.width;
                space.width -= float.margin_box.width;
            }
            FloatDirection::Right => space.width -= float.margin_box.width,
        }
    }
    space.width = space.width.max(0.0);
    space
}

/// Margins of `node` resolved against `basis`, the root's content width; `auto`
/// is zero.
fn resolved_margins(tree: &Document, node: usize, basis: f32) -> taffy::Rect<f32> {
    tree.nodes[node]
        .style
        .margin
        .map(|margin| margin.resolve_to_option(basis, resolve_calc).unwrap_or(0.0))
}

/// Lay a float out the way the block algorithm does: resolve its margins
/// against the container, then give it the room that is left.
fn measure_float(tree: &mut Document, node: usize, geometry: FlowGeometry) -> TentativeFloat {
    let margin = resolved_margins(tree, node, geometry.width);
    let available = (geometry.width - margin.left - margin.right).max(0.0);
    // What taffy's `perform_child_layout` does; that helper is private to
    // taffy.
    let output = tree.compute_child_layout(
        taffy::NodeId::from(node),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size::NONE,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: Size {
                width: Some(geometry.width),
                height: None,
            },
            available_space: Size {
                width: AvailableSpace::Definite(available),
                height: AvailableSpace::MaxContent,
            },
            // A float establishes a new block formatting context: its margins
            // do not collapse with its children's.
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );
    let style = &tree.nodes[node].style;
    TentativeFloat {
        node,
        // The projection accepts only left and right floats.
        direction: style
            .float
            .float_direction()
            .unwrap_or(FloatDirection::Left),
        clear: style.clear,
        margin_box: Size {
            width: output.size.width + margin.left + margin.right,
            height: output.size.height + margin.top + margin.bottom,
        },
        margin,
        output,
    }
}

/// Place a float in the context at the line offset `y` and, when performing
/// layout, store its position.
pub(crate) fn commit_float(
    tree: &mut Document,
    ctx: &mut BlockContext<'_>,
    float: TentativeFloat,
    y: f32,
    geometry: FlowGeometry,
    perform: bool,
) {
    // The context measures block offsets from the border-box top.
    let position = ctx.place_floated_box(
        float.margin_box,
        geometry.top_edge + y,
        float.direction,
        float.clear,
        false,
    );
    if perform {
        let layout = &mut tree.nodes[float.node].unrounded_layout;
        layout.location = taffy::Point {
            x: position.x + float.margin.left,
            y: position.y + float.margin.top,
        };
        layout.size = float.output.size;
    }
}

/// Min- and max-content widths of the root's boxes, for the paragraph's
/// intrinsic sizes. Each box is measured without changing its layout.
pub(crate) fn intrinsics_of_boxes(tree: &mut Document, idx: usize, basis: f32) -> AtomicIntrinsics {
    let boxes = tree.nodes[idx]
        .ifc
        .as_ref()
        .map(|root| root.boxes.clone())
        .unwrap_or_default();
    let mut intrinsics = AtomicIntrinsics::new();
    for b in boxes {
        match b.kind {
            IfcBoxKind::Float => {
                let margin = resolved_margins(tree, b.node, basis);
                let (min_content, max_content) =
                    content_widths(tree, b.node, basis, margin.left + margin.right);
                let style = &tree.nodes[b.node].style;
                let side = match style.float.float_direction() {
                    Some(FloatDirection::Right) => FloatSide::Right,
                    _ => FloatSide::Left,
                };
                let clear = match style.clear {
                    Clear::None => FloatClear::None,
                    Clear::Left => FloatClear::Left,
                    Clear::Right => FloatClear::Right,
                    Clear::Both => FloatClear::Both,
                };
                intrinsics.insert_float(
                    shodo::node::NodeId(b.node as u64),
                    FloatIntrinsic {
                        min_content,
                        max_content,
                        side,
                        clear,
                    },
                );
            }
        }
    }
    intrinsics
}

/// Min- and max-content margin-box width of `node`, measured without changing
/// its layout. taffy measures a child's contribution to a container's
/// intrinsic width with `InherentSize` (compute/block.rs
/// `determine_content_based_container_width`), so a child with a `width` of
/// its own contributes that width.
fn content_widths(tree: &mut Document, node: usize, basis: f32, margins_x: f32) -> (f32, f32) {
    let measure = |tree: &mut Document, available: AvailableSpace| {
        let output = tree.compute_child_layout(
            taffy::NodeId::from(node),
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Horizontal,
                known_dimensions: Size::NONE,
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size: Size {
                    width: Some(basis),
                    height: None,
                },
                available_space: Size {
                    width: available,
                    height: AvailableSpace::MaxContent,
                },
                vertical_margins_are_collapsible: Line::TRUE,
            },
        );
        output.size.width + margins_x
    };
    (
        measure(tree, AvailableSpace::MinContent),
        measure(tree, AvailableSpace::MaxContent),
    )
}

#[cfg(test)]
mod tests;
