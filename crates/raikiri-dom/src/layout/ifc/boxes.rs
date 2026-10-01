//! Lay out a paragraph together with the boxes that sit in its lines.

use super::flow::{FlowGeometry, LineSpace, MAX_SPACE_RETRIES, line_space, resolve_indent};
use super::root::{IfcLines, IfcRoot, with_state};
use crate::Document;
use crate::taffy_impl::resolve_calc;
use shodo::{
    AtomicIntrinsic, AtomicIntrinsics, AtomicSize, AtomicSizes, FloatClear, FloatCursor,
    FloatIntrinsic, FloatSide, LineConstraint, LineResult,
};
use taffy::util::ResolveOrZero;
use taffy::{
    AvailableSpace, BlockContext, BlockFormattingContext, Clear, CollapsibleMarginSet,
    FloatDirection, LayoutInput, LayoutPartialTree, Line, RequestedAxis, RunMode, Size, SizingMode,
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
    Atomic,
    /// A block child of the root: it ends the line before it, and the lines
    /// after it start below it.
    Block,
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
#[cfg(test)]
pub(crate) fn layout_with_boxes(
    tree: &mut Document,
    idx: usize,
    geometry: FlowGeometry,
    block_ctx: Option<&mut BlockContext<'_>>,
    perform: bool,
) -> IfcLines {
    layout_with_boxes_in(tree, idx, geometry, block_ctx, perform, false)
}

/// [`layout_with_boxes`] for a root whose bottom margin may collapse with the
/// bottom margins of its last block child (`bottom_margin_escapes`): the root
/// is in its parent's formatting context and has no bottom padding, border
/// or height in between. Those margins are then returned in
/// [`IfcLines::escaping_margin`] instead of being added to the height.
pub(crate) fn layout_with_boxes_in(
    tree: &mut Document,
    idx: usize,
    geometry: FlowGeometry,
    block_ctx: Option<&mut BlockContext<'_>>,
    perform: bool,
    bottom_margin_escapes: bool,
) -> IfcLines {
    let Some(root) = tree.nodes[idx]
        .ifc
        .as_ref()
        .map(|root| root.without_lines())
    else {
        return IfcLines {
            width: geometry.width,
            lines: std::sync::Arc::new(Vec::new()),
            height: 0.0,
            beside_floats: false,
            escaping_margin: CollapsibleMarginSet::ZERO,
        };
    };
    // One inner function takes the context from both arms: the caller's
    // context and a local one cannot share a lifetime, because the pointee of
    // `&mut BlockContext<'_>` is invariant.
    match block_ctx {
        Some(ctx) => run_boxes(
            tree,
            idx,
            &root,
            geometry,
            ctx,
            perform,
            false,
            bottom_margin_escapes,
        ),
        None => {
            let mut bfc = BlockFormattingContext::new();
            let mut ctx = bfc.root_block_context();
            ctx.set_width(geometry.width + geometry.edges.0 + geometry.edges.1);
            run_boxes(tree, idx, &root, geometry, &mut ctx, perform, true, false)
        }
    }
}

/// The line loop. `local` is true when the context is this paragraph's own,
/// which makes the paragraph a block formatting context that contains its
/// floats and its children's margins. `bottom_margin_escapes` is described
/// at [`layout_with_boxes_in`].
#[allow(clippy::too_many_arguments)]
fn run_boxes(
    tree: &mut Document,
    idx: usize,
    root: &IfcRoot,
    geometry: FlowGeometry,
    ctx: &mut BlockContext<'_>,
    perform: bool,
    local: bool,
    bottom_margin_escapes: bool,
) -> IfcLines {
    let FlowGeometry {
        width,
        edges,
        top_edge,
    } = geometry;
    let mut options = root.options;
    options.text_indent.length = resolve_indent(root.indent, width);
    // Atomic inlines are sized once, before any line: their width does not
    // depend on the line they land in.
    let atomics = measure_atomics(tree, root, geometry);
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
    // Top of the lowest float this paragraph has placed, from its content
    // box: a later float may not be placed above it (CSS 2.1 9.5.1 rule 5).
    let mut ceiling = f32::NEG_INFINITY;
    // Every float of a line is met, may be withdrawn and is met again, and
    // the line may be retried for its height; more calls than that for one
    // line mean the loop does not converge.
    let max_calls = 3 * root.boxes.len() + MAX_SPACE_RETRIES + 2;
    let mut moves = 0;
    let mut calls = 0;
    // Bottom of the floats inside block children, from the content-box top.
    let mut block_floats_bottom = f32::NEG_INFINITY;
    // Vertical margins after the last block child that are not placed yet:
    // they collapse with the next block child's top margins, and are added in
    // full before a line (margins never collapse with a line box; CSS 2.1
    // 8.3.1).
    let mut pending = CollapsibleMarginSet::ZERO;

    loop {
        calls += 1;
        if calls > max_calls {
            debug_assert!(false, "the line loop does not converge");
            break;
        }
        // Where the next line starts if it is a line: below the margins
        // still pending.
        let line_y = y + pending.resolve();
        let base = line_space(ctx, edges.0, top_edge, width, line_y, assumed_height);
        let space = shorten(base, &tentative);
        let mut constraint = LineConstraint::new(space.width);
        // `space` is measured from the content box's left edge; shodo measures
        // the offset from the inline-start edge, which is the right edge in a
        // right-to-left paragraph.
        constraint.inline_start_offset = if root.rtl {
            width - (space.start + space.width)
        } else {
            space.start
        };
        constraint.block_offset = line_y;
        constraint.floats_placed_through = cursor;
        // The state is taken only for the engine call: measuring a float
        // below lays out another node, which may be an ifc root itself.
        let result = with_state(tree, |state| {
            root.paragraph.next_line(
                &mut state.layout_cx,
                token,
                &options,
                &constraint,
                &atomics.sizes,
            )
        });
        let Some(result) = result else { break };
        match result {
            LineResult::FloatEncountered {
                node, float_cursor, ..
            } => {
                let node = node.0 as usize;
                cursor = Some(float_cursor);
                // A withdrawal rewinds the cursor, so a float that already
                // waits for the next line can be met again.
                if deferred.iter().any(|f| f.node == node) {
                    continue;
                }
                let float = measure_float(tree, node, geometry);
                // `space` already has the tentative floats taken off. A float
                // with no other float beside it is placed where it is even
                // when it is wider than the line.
                let narrowed = !tentative.is_empty() || space.start != 0.0 || space.width < width;
                let fits = float.margin_box.width <= space.width || !narrowed;
                // Clearance moves the float below the floats it clears,
                // whether they are placed already or only on this line.
                let clears_tentative = tentative.iter().any(|f| clears(float.clear, f.direction));
                let below = clears_tentative
                    || ceiling > line_y
                    || (float.clear != Clear::None
                        && ctx
                            .cleared_threshold(float.clear)
                            .is_some_and(|bottom| bottom > top_edge + line_y));
                // Floats are placed in source order (CSS 2.1 9.5.1 rule 5):
                // once one of this line waits for the next line, the later
                // ones wait too.
                if withdrawn.contains(&node) || !fits || below || !deferred.is_empty() {
                    deferred.push(float);
                } else {
                    tentative.push(float);
                }
            }
            LineResult::Line(line) => {
                // Placing a float moved its anchor to a later line: take the
                // last such float back and lay the line out again without it.
                // A displaced float that already waits for the next line
                // takes no room from this one and needs nothing.
                let displaced = line
                    .displaced_floats()
                    .iter()
                    .rev()
                    .find(|(node, _)| tentative.iter().any(|f| f.node == node.0 as usize));
                if let Some(&(node, earlier)) = displaced {
                    let node = node.0 as usize;
                    tentative.retain(|f| f.node != node);
                    withdrawn.push(node);
                    cursor = earlier.before();
                    continue;
                }
                let height = line.block_size();
                // The line may span more of an outer float than the height it
                // was assumed to have; lay it out again if the space for its
                // real height differs.
                let real = shorten(
                    line_space(ctx, edges.0, top_edge, width, line_y, height),
                    &tentative,
                );
                if retries < MAX_SPACE_RETRIES && real != space {
                    assumed_height = height;
                    retries += 1;
                    continue;
                }
                // A line that does not fit beside the floats is moved down
                // until it fits or no float is left beside it (CSS 2.1 9.5).
                // Its own floats keep the place they have at this offset.
                let indent = if lines.is_empty() {
                    options.text_indent.length
                } else {
                    0.0
                };
                let narrowed = space.start != 0.0 || space.width < width;
                let overflows = narrowed
                    && line.inline_size() + indent > space.width + 0.01
                    && moves < MAX_LINE_MOVES;
                if overflows {
                    for float in tentative.drain(..) {
                        ceiling =
                            ceiling.max(commit_float(tree, ctx, float, line_y, geometry, perform));
                    }
                }
                if overflows && let Some(next) = next_float_edge(ctx, top_edge, line_y) {
                    y = next;
                    pending = CollapsibleMarginSet::ZERO;
                    moves += 1;
                    assumed_height = 0.0;
                    retries = 0;
                    calls = 0;
                    continue;
                }
                if narrowed {
                    beside = true;
                }
                for float in tentative.drain(..) {
                    ceiling =
                        ceiling.max(commit_float(tree, ctx, float, line_y, geometry, perform));
                }
                withdrawn.clear();
                assumed_height = 0.0;
                retries = 0;
                calls = 0;
                moves = 0;
                y = line_y + height;
                pending = CollapsibleMarginSet::ZERO;
                token = line.break_token();
                lines.push(line);
                // Floats that did not fit beside the line they are anchored
                // in go at the start of the next line, before it is measured.
                // Placing them any earlier would narrow the line that is
                // still being retried.
                for float in deferred.drain(..) {
                    ceiling = ceiling.max(commit_float(tree, ctx, float, y, geometry, perform));
                }
            }
            LineResult::BlockInInline { node, token_after } => {
                // Floats anchored before the block and not placed yet go
                // above it: the block starts below the last line.
                for float in tentative.drain(..).chain(deferred.drain(..)) {
                    ceiling =
                        ceiling.max(commit_float(tree, ctx, float, line_y, geometry, perform));
                }
                let block =
                    layout_block_child(tree, ctx, node.0 as usize, y, pending, geometry, perform);
                block_floats_bottom = block_floats_bottom.max(block.top + block.floats_bottom);
                (y, pending) = (block.next_y, block.pending);
                token = token_after;
                // The call count is kept: every block is one of the root's
                // boxes, so blocks in a row stay within the bound, and a
                // result that does not move past its block ends the loop.
                withdrawn.clear();
                assumed_height = 0.0;
                retries = 0;
                moves = 0;
            }
            LineResult::Done => break,
            other => {
                // Absolutely positioned boxes are not projected, so no
                // other result is expected.
                debug_assert!(false, "unexpected line result: {other:?}");
                break;
            }
        }
    }
    // The margins below a last block child collapse through the root's
    // bottom edge when nothing separates them from the root's own; otherwise
    // the paragraph contains them.
    let escaping_margin = if bottom_margin_escapes {
        pending
    } else {
        y += pending.resolve();
        CollapsibleMarginSet::ZERO
    };
    // A float met after the last line still gets a place.
    for float in tentative.drain(..).chain(deferred.drain(..)) {
        commit_float(tree, ctx, float, y, geometry, perform);
    }
    if perform {
        place_atomics(tree, &lines, &atomics.outputs, geometry);
        super::records::record_inline_boxes(tree, idx, root, &lines, &geometry);
    }
    // A paragraph that is its own formatting context contains its floats; one
    // that is not leaves them hanging below, and the parent collects them.
    // The context measures from the border-box top, the height from the
    // content-box top, hence the subtraction.
    let height = if local {
        let floats_bottom = ctx.floated_content_height_contribution();
        let own = if floats_bottom.is_finite() {
            y.max(floats_bottom - top_edge)
        } else {
            y
        };
        // The context does not collect what its sub-contexts placed.
        own.max(block_floats_bottom)
    } else {
        y
    };
    IfcLines {
        width,
        height,
        lines: std::sync::Arc::new(lines),
        beside_floats: beside,
        escaping_margin,
    }
}

/// How many times one line is moved down past floats before it is accepted
/// where it is.
const MAX_LINE_MOVES: usize = 64;

/// Whether a float with `clear` is moved below a float on the `side`.
fn clears(clear: Clear, side: FloatDirection) -> bool {
    matches!(
        (clear, side),
        (Clear::Both, _)
            | (Clear::Left, FloatDirection::Left)
            | (Clear::Right, FloatDirection::Right)
    )
}

/// The next block offset below `y` (both from the content-box top) where the
/// floats beside a line change: the start of the next float segment, else the
/// bottom of the floats. `None` when no float is beside `y`.
fn next_float_edge(ctx: &BlockContext<'_>, top_edge: f32, y: f32) -> Option<f32> {
    let at = top_edge + y;
    let segment = ctx.find_content_slot(at, Clear::None, None).segment_id?;
    let next = ctx.find_content_slot(at, Clear::None, Some(segment));
    let edge = if next.segment_id.is_some() && next.y > at {
        next.y
    } else {
        ctx.cleared_threshold(Clear::Both)?
    };
    (edge > at).then_some(edge - top_edge)
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

/// Lay a child of the root out the way taffy's block algorithm lays out a
/// float: no known size, the root's content width as the percentage basis,
/// and `available` as the width it may take. It is also how an atomic inline
/// is sized: shrink-to-fit within the line's content width.
fn perform_box_layout(
    tree: &mut Document,
    node: usize,
    basis: f32,
    available: f32,
) -> taffy::LayoutOutput {
    // What taffy's `perform_child_layout` does; that helper is private to
    // taffy.
    tree.compute_child_layout(
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
                width: Some(basis),
                height: None,
            },
            available_space: Size {
                width: AvailableSpace::Definite(available),
                height: AvailableSpace::MaxContent,
            },
            // The box establishes a new block formatting context: its margins
            // do not collapse with its children's.
            vertical_margins_are_collapsible: Line::FALSE,
        },
    )
}

/// Lay a float out the way the block algorithm does: resolve its margins
/// against the container, then give it the room that is left.
fn measure_float(tree: &mut Document, node: usize, geometry: FlowGeometry) -> TentativeFloat {
    let margin = resolved_margins(tree, node, geometry.width);
    let available = (geometry.width - margin.left - margin.right).max(0.0);
    let output = perform_box_layout(tree, node, geometry.width, available);
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

/// Sizes the paragraph needs for its atomic inlines, and the layouts to store
/// once the lines are final.
pub(crate) struct AtomicMeasure {
    pub(crate) sizes: AtomicSizes,
    pub(crate) outputs: Vec<(usize, taffy::LayoutOutput)>,
}

/// Measure every atomic inline of the root at the paragraph's content width.
///
/// Each atomic is laid out as a node of its own, so this runs outside the
/// engine state scope: an atomic may hold another ifc root.
pub(crate) fn measure_atomics(
    tree: &mut Document,
    root: &IfcRoot,
    geometry: FlowGeometry,
) -> AtomicMeasure {
    let mut sizes = AtomicSizes::new();
    let mut outputs = Vec::new();
    for b in root.boxes.iter().filter(|b| b.kind == IfcBoxKind::Atomic) {
        let node = b.node;
        // A child's percentages resolve against the paragraph's content width.
        let margin = resolved_margins(tree, node, geometry.width);
        let available = (geometry.width - margin.left - margin.right).max(0.0);
        let output = perform_box_layout(tree, node, geometry.width, available);
        let id = shodo::node::NodeId(node as u64);
        // Only the alphabetic baseline is supplied (horizontal lines). shodo
        // measures it from the margin-box top and falls back to the margin
        // box's bottom edge when there is none (CSS 2.1 10.8.1).
        let baseline = match root.paragraph.required_baseline(id) {
            Some(shodo::geometry::BaselineKind::Alphabetic) | None => {
                atomic_baseline(tree, node, &output).map(|b| b + margin.top)
            }
            Some(_) => None,
        };
        sizes.insert(
            id,
            AtomicSize {
                inline_size: output.size.width,
                block_size: output.size.height,
                baseline,
                margins: shodo::node::Sides {
                    inline_start: margin.left,
                    inline_end: margin.right,
                    block_start: margin.top,
                    block_end: margin.bottom,
                },
            },
        );
        outputs.push((node, output));
    }
    AtomicMeasure { sizes, outputs }
}

/// What laying out a block child gives the line loop.
struct BlockChild {
    /// Top of the block's border box, from the content-box top.
    top: f32,
    /// Where the content after the block starts, before `pending`: the
    /// bottom of its border box, or `y` itself when it is collapsed through.
    next_y: f32,
    /// Margins below the block that collapse with what follows: its own
    /// bottom margin with the ones that leave it through its last child, or
    /// everything up to here when the block is collapsed through.
    pending: CollapsibleMarginSet,
    /// Bottom of the floats placed inside the block, from its top;
    /// `NEG_INFINITY` when it has none.
    floats_bottom: f32,
}

/// Lay a block child out below the offset `y`, as wide as the content box
/// less its side margins, the way the parent's block algorithm lays out an
/// in-flow block (CSS 2.1 8.3.1): its top margin, with the margins of its
/// first child that leave it, collapses with the `pending` margins of the
/// block children above it. With `perform`, its final layout is stored.
fn layout_block_child(
    tree: &mut Document,
    ctx: &mut BlockContext<'_>,
    node: usize,
    y: f32,
    pending: CollapsibleMarginSet,
    geometry: FlowGeometry,
    perform: bool,
) -> BlockChild {
    use taffy::LayoutBlockContainer;
    let mut margin = resolved_margins(tree, node, geometry.width);
    let stretch = (geometry.width - margin.left - margin.right).max(0.0);
    let width = used_block_width(tree, node, geometry.width).unwrap_or(stretch);
    let width = clamp_block_width(tree, node, geometry.width, width);
    // `auto` side margins share the room the block leaves (CSS 2.1 10.3.3),
    // as in taffy's block algorithm: both auto centre the block, one auto
    // takes all of it.
    let style_margin = tree.nodes[node].style.margin;
    let free = (geometry.width - width - margin.left - margin.right).max(0.0);
    match (style_margin.left.is_auto(), style_margin.right.is_auto()) {
        (true, true) => margin.left = free / 2.0,
        (true, false) => margin.left = free,
        _ => {}
    }
    // Placed where its own top margin alone puts it. Margins that leave the
    // block through its first child are known only after it is laid out; they
    // move its box below, not the floats it placed inside.
    let guess = y + pending.collapse_with_margin(margin.top).resolve();
    // The context measures from the root's border box; the insets are added
    // to the root's own. The right inset follows the used width, as in
    // taffy's own in-flow loop.
    let right = geometry.width - margin.left - width + geometry.edges.1;
    let mut child_ctx = ctx.sub_context(
        geometry.top_edge + guess,
        [geometry.edges.0 + margin.left, right],
    );
    let inputs = LayoutInput {
        run_mode: if perform {
            RunMode::PerformLayout
        } else {
            RunMode::ComputeSize
        },
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width: Some(width),
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        parent_size: Size {
            width: Some(geometry.width),
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::MaxContent,
        },
        // The block is in the paragraph's formatting context: the margins of
        // its first and last children collapse through it unless its padding,
        // border or height keep them in (taffy decides and returns them).
        vertical_margins_are_collapsible: Line::TRUE,
    };
    let output =
        tree.compute_block_child_layout(taffy::NodeId::from(node), inputs, Some(&mut child_ctx));
    let floats_bottom = child_ctx.floated_content_height_contribution();
    let top_set = output.top_margin.collapse_with_margin(margin.top);
    let bottom_set = output.bottom_margin.collapse_with_margin(margin.bottom);
    let (top, after) = if output.margins_can_collapse_through {
        // An empty block: its margins and the ones above it are one set, and
        // its box sits where that set would put the next box's top.
        let through = pending
            .collapse_with_set(top_set)
            .collapse_with_set(bottom_set);
        (
            y + pending.collapse_with_set(top_set).resolve(),
            (y, through),
        )
    } else {
        let top = y + pending.collapse_with_set(top_set).resolve();
        (top, (top + output.size.height, bottom_set))
    };
    if perform {
        let location = taffy::Point {
            x: geometry.edges.0 + margin.left,
            y: geometry.top_edge + top,
        };
        commit_child_layout(tree, node, &output, location, geometry.width);
    }
    BlockChild {
        top,
        next_y: after.0,
        pending: after.1,
        floats_bottom: floats_bottom + (guess - top),
    }
}

/// The border-box width a block child's own `width` gives, resolved against
/// the root's content width the way taffy's block algorithm resolves the size
/// of an in-flow item; `None` for `auto`.
fn used_block_width(tree: &Document, node: usize, basis: f32) -> Option<f32> {
    use taffy::util::MaybeResolve;
    let style = &tree.nodes[node].style;
    style
        .size
        .width
        .maybe_resolve(Some(basis), resolve_calc)
        .map(|width| width + box_sizing_adjustment(tree, node, basis))
}

/// `width` clamped by the block child's own `min-width` and `max-width`,
/// resolved like [`used_block_width`].
fn clamp_block_width(tree: &Document, node: usize, basis: f32, width: f32) -> f32 {
    use taffy::util::MaybeResolve;
    let style = &tree.nodes[node].style;
    let adjustment = box_sizing_adjustment(tree, node, basis);
    let min = style
        .min_size
        .width
        .maybe_resolve(Some(basis), resolve_calc)
        .map(|min| min + adjustment);
    let max = style
        .max_size
        .width
        .maybe_resolve(Some(basis), resolve_calc)
        .map(|max| max + adjustment);
    let width = max.map_or(width, |max| width.min(max));
    min.map_or(width, |min| width.max(min))
}

/// What a `content-box` size needs added to become a border-box size.
fn box_sizing_adjustment(tree: &Document, node: usize, basis: f32) -> f32 {
    let style = &tree.nodes[node].style;
    if style.box_sizing != taffy::BoxSizing::ContentBox {
        return 0.0;
    }
    let padding = style.padding.resolve_or_zero(Some(basis), resolve_calc);
    let border = style.border.resolve_or_zero(Some(basis), resolve_calc);
    padding.left + padding.right + border.left + border.right
}

/// Store the final layout of every atomic inline at its fragment in the
/// accepted lines.
fn place_atomics(
    tree: &mut Document,
    lines: &[shodo::Line],
    outputs: &[(usize, taffy::LayoutOutput)],
    geometry: FlowGeometry,
) {
    for line in lines {
        for fragment in line.fragments() {
            let shodo::Fragment::Atomic(atomic) = fragment else {
                continue;
            };
            let node = atomic.node.0 as usize;
            let Some((_, output)) = outputs.iter().find(|(n, _)| *n == node) else {
                continue;
            };
            // The fragment is relative to the line's top and the content-box
            // start; the layout is relative to the root's border box. A
            // relatively positioned atomic is drawn offset from that place,
            // which taffy's layout location carries for inline-level boxes.
            let (dx, dy) = relative_inset(tree, node, geometry.width);
            let location = taffy::Point {
                x: geometry.edges.0 + atomic.border_rect.inline_start + dx,
                y: geometry.top_edge + line.block_offset() + atomic.border_rect.block_start + dy,
            };
            commit_child_layout(tree, node, output, location, geometry.width);
        }
    }
}

/// The offset of a relatively positioned box (CSS 2.1 9.4.3): `left` over
/// `-right`, `top` over `-bottom`, percentages of the root's content width
/// (a vertical percentage has no definite basis here and is taken as zero).
/// `(0, 0)` for a box in normal position.
fn relative_inset(tree: &Document, node: usize, basis: f32) -> (f32, f32) {
    use taffy::util::MaybeResolve;
    let style = &tree.nodes[node].style;
    if style.position != taffy::Position::Relative
        || style.inset.left.is_auto()
            && style.inset.right.is_auto()
            && style.inset.top.is_auto()
            && style.inset.bottom.is_auto()
    {
        return (0.0, 0.0);
    }
    let horizontal =
        |value: taffy::LengthPercentageAuto| value.maybe_resolve(Some(basis), resolve_calc);
    let vertical = |value: taffy::LengthPercentageAuto| value.maybe_resolve(None, resolve_calc);
    let dx = horizontal(style.inset.left)
        .or(horizontal(style.inset.right).map(|right| -right))
        .unwrap_or(0.0);
    let dy = vertical(style.inset.top)
        .or(vertical(style.inset.bottom).map(|bottom| -bottom))
        .unwrap_or(0.0);
    (dx, dy)
}

/// The baseline of an atomic inline from its border-box top, if it has one.
/// An inline-block that is a scroll container has none: it sits on its
/// bottom margin edge (CSS 2.1 10.8.1).
fn atomic_baseline(tree: &Document, node: usize, output: &taffy::LayoutOutput) -> Option<f32> {
    let overflow = tree.nodes[node].style.overflow;
    if overflow.x != taffy::Overflow::Visible || overflow.y != taffy::Overflow::Visible {
        return None;
    }
    output.baselines.first
}

/// Place a float in the context at the line offset `y` and, when performing
/// layout, store its position. Returns the top of the placed margin box from
/// the content-box top.
pub(crate) fn commit_float(
    tree: &mut Document,
    ctx: &mut BlockContext<'_>,
    float: TentativeFloat,
    y: f32,
    geometry: FlowGeometry,
    perform: bool,
) -> f32 {
    // The context measures block offsets from the border-box top.
    let position = ctx.place_floated_box(
        float.margin_box,
        geometry.top_edge + y,
        float.direction,
        float.clear,
        false,
    );
    if perform {
        let location = taffy::Point {
            x: position.x + float.margin.left,
            y: position.y + float.margin.top,
        };
        commit_child_layout(tree, float.node, &float.output, location, geometry.width);
    }
    position.y - geometry.top_edge
}

/// Store the final layout of a child that the ifc root laid out itself.
///
/// `location` is the border-box position relative to the ifc root's border
/// box. `output` is what the child's layout returned. `basis` is what a
/// percentage in the child's margin, padding and border resolves against: the
/// root's content width.
pub(crate) fn commit_child_layout(
    tree: &mut Document,
    node: usize,
    output: &taffy::LayoutOutput,
    location: taffy::Point<f32>,
    basis: f32,
) {
    let style = &tree.nodes[node].style;
    let padding = style.padding.resolve_or_zero(Some(basis), resolve_calc);
    let border = style.border.resolve_or_zero(Some(basis), resolve_calc);
    let margin = style
        .margin
        .map(|margin| margin.resolve_to_option(basis, resolve_calc).unwrap_or(0.0));
    let layout = taffy::Layout {
        order: tree.nodes[node].unrounded_layout.order,
        location,
        size: output.size,
        scrollable_overflow_rect: output.scrollable_overflow_rect,
        scrollbar_size: Size::ZERO,
        border,
        padding,
        margin,
    };
    tree.set_unrounded_layout(taffy::NodeId::from(node), &layout);
}

/// Intrinsic widths of the root's boxes.
pub(crate) struct BoxIntrinsics {
    /// What the engine takes for atomic inlines and floats.
    pub(crate) engine: AtomicIntrinsics,
    /// The widest min- and max-content margin-box width of the block
    /// children. The engine breaks a line at a block child and does not size
    /// it, so the paragraph's widths are at least these.
    pub(crate) blocks: (f32, f32),
}

impl BoxIntrinsics {
    pub(crate) const EMPTY: Self = Self {
        engine: AtomicIntrinsics::EMPTY,
        blocks: (0.0, 0.0),
    };
}

/// Min- and max-content widths of the root's boxes, for the paragraph's
/// intrinsic sizes. Each box is measured without changing its layout.
pub(crate) fn intrinsics_of_boxes(tree: &mut Document, idx: usize, basis: f32) -> BoxIntrinsics {
    let boxes = tree.nodes[idx]
        .ifc
        .as_ref()
        .map(|root| root.boxes.clone())
        .unwrap_or_default();
    let mut intrinsics = BoxIntrinsics {
        engine: AtomicIntrinsics::new(),
        blocks: (0.0, 0.0),
    };
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
                intrinsics.engine.insert_float(
                    shodo::node::NodeId(b.node as u64),
                    FloatIntrinsic {
                        min_content,
                        max_content,
                        side,
                        clear,
                    },
                );
            }
            IfcBoxKind::Block => {
                let margin = resolved_margins(tree, b.node, basis);
                let (min_content, max_content) =
                    content_widths(tree, b.node, basis, margin.left + margin.right);
                intrinsics.blocks.0 = intrinsics.blocks.0.max(min_content);
                intrinsics.blocks.1 = intrinsics.blocks.1.max(max_content);
            }
            IfcBoxKind::Atomic => {
                let margin = resolved_margins(tree, b.node, basis);
                let (min_content, max_content) =
                    content_widths(tree, b.node, basis, margin.left + margin.right);
                intrinsics.engine.insert_atomic(
                    shodo::node::NodeId(b.node as u64),
                    AtomicIntrinsic {
                        min_content,
                        max_content,
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
