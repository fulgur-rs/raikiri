//! Lay out a paragraph together with the boxes that sit in its lines.

use super::clear_breaks::ClearBreakIndex;
use super::flow::{FlowGeometry, LineSpace, MAX_SPACE_RETRIES, line_space, resolve_indent};
use super::generated_atomics::logical_box;
use super::generated_blocks;
use super::geometry::IfcAxes;
use super::inline_boxes::{for_each_atomic, placed};
use super::root::{IfcBoxFragment, IfcLines, IfcRoot, with_state};
use crate::Document;
use crate::fragment::FragmentationContext;
use crate::layout::context::{LayoutSession, TextGroupScope};
use crate::layout::dispatch::block_flow::context::{
    BlockContext, BlockFormattingContext, BoxProbe,
};
use crate::layout::dispatch::{fit_content_room, measure_against_parent_height};
use crate::layout::protocol::{
    AvailableSpace, AxisStaticEdge, AxisStaticPosition, CollapsibleMarginSet, FloatDirection,
    LayoutInput, LayoutOutput, NodeId, OofCandidate, OofCandidates, OofPosition,
    OofPositioningArea, RequestedAxis, RunMode, SizingMode,
};
use crate::layout::style_views::ReadRole;
use raikiri_layout_geometry::{BoxLayout, Edges, Line, Point, Size};
use raikiri_layout_style::values::{
    self, AlignContentKeyword, BoxSizing, Clear, Display, ExpandedDimension, LengthPercentageAuto,
    MaybeResolve, Overflow, Position, ResolveOrZero,
};
use raikiri_style::property::{DisplayValue, PositionValue};
use shodo::geometry::{Direction, WritingMode};
use shodo::style::{LineOptions, TextGroupAlign};
use shodo::{
    AtomicIntrinsic, AtomicIntrinsics, AtomicSize, AtomicSizes, BreakToken, FloatClear,
    FloatCursor, FloatIntrinsic, FloatSide, LineConstraint, LineResult,
};
use std::collections::{HashMap, HashSet};

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
    /// An absolutely positioned (or fixed) box: it takes no room on the
    /// lines and is placed against the root once they are final.
    OutOfFlow,
}

/// A float met while laying out the current line. Defer placement in the
/// shared context until the line is final.
#[derive(Clone, Debug)]
pub(crate) struct TentativeFloat {
    pub(crate) node: usize,
    pub(crate) direction: FloatDirection,
    pub(crate) clear: Clear,
    pub(crate) margin_box: Size<f32>,
    pub(crate) margin: Edges<f32>,
    /// What `perform_child_layout` returned for the float.
    pub(crate) output: LayoutOutput,
}

struct RunBoxesOutcome {
    lines: IfcLines,
    continuation: Option<BreakToken>,
    atomic_outputs: Vec<(usize, LayoutOutput)>,
    /// The generated block still open where the segment stopped.
    open_block: Option<generated_blocks::OpenBlock>,
    /// Where the next segment's first line starts.
    next_start: LineStart,
}

/// What the engine knows of the line a break token starts, for the indent it
/// gives that line (CSS Text 3 §8.1).
#[derive(Clone, Copy, Debug)]
struct LineStart {
    /// The paragraph's first line.
    first: bool,
    /// A line after a forced break or a block.
    after_forced: bool,
}

impl LineStart {
    const PARAGRAPH: Self = Self {
        first: true,
        after_forced: false,
    };
}

struct FragmentedFloatOutput<'a> {
    perform: bool,
    adjoins_top_margin: bool,
    fragmentainer: Option<usize>,
    placements: &'a mut Vec<IfcBoxFragment>,
    oof_candidates: &'a mut OofCandidates,
}

/// Resolve what the root's percentages refer to before its lines are laid
/// out: the offsets of its relatively positioned inline elements, and the
/// percentage margins and padding of its inline boxes against the inline
/// size the lines are broken at. In columns that is the column width, as
/// the column box is the containing block of the content (CSS Multicol 1
/// §2); a vertical root's is its physical height (CSS Writing Modes 4 §7.2).
fn resolve_root_percentages(tree: &mut LayoutSession<'_>, idx: usize, geometry: &FlowGeometry) {
    let (nodes, ifc) = (&mut tree.document.nodes, &mut tree.document.ifc);
    let Some(root) = nodes[idx].ifc.as_mut() else {
        return;
    };
    root.resolve_percent_offsets(geometry.content_size.width);
    if let Some(state) = ifc.as_mut() {
        root.resolve_percent_edges(geometry.width, &mut state.layout_cx, &state.fonts);
    }
}

/// [`layout_with_boxes_in`] for a root whose margins stay inside it.
#[cfg(test)]
pub(crate) fn layout_with_boxes(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    geometry: FlowGeometry,
    block_ctx: Option<&mut BlockContext<'_>>,
    perform: bool,
) -> IfcLines {
    layout_with_boxes_in(tree, idx, geometry, block_ctx, perform, false, false)
}

/// Lay the paragraph out with its boxes against `block_ctx` (the parent's
/// context) or a context of its own. When `perform` is true, floats are
/// committed to the context and the boxes get their final layouts.
///
/// `top_margin_escapes` keeps adjoining leading margins out of the root's
/// internal coordinates and returns their full set in
/// [`IfcLines::leading_block_margin`].
/// `bottom_margin_escapes` is set for a root whose bottom margin may collapse
/// with the bottom margins of its last block child: the root is in its
/// parent's formatting context and has no bottom padding, border or height
/// in between. Those margins are then returned in
/// [`IfcLines::escaping_margin`] instead of being added to the height.
pub(crate) fn layout_with_boxes_in(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    geometry: FlowGeometry,
    block_ctx: Option<&mut BlockContext<'_>>,
    perform: bool,
    top_margin_escapes: bool,
    bottom_margin_escapes: bool,
) -> IfcLines {
    resolve_root_percentages(tree, idx, &geometry);
    let Some(mut root) = tree.nodes[idx]
        .ifc
        .as_ref()
        .map(|root| root.without_lines())
    else {
        return IfcLines {
            width: geometry.width,
            lines: std::sync::Arc::new(Vec::new()),
            oof_candidates: OofCandidates::new(),
            height: 0.0,
            beside_floats: false,
            escaping_margin: CollapsibleMarginSet::ZERO,
            leading_block_margin: None,
            block_line_starts: Vec::new(),
            generated_block_starts: Vec::new(),
            block_ends: Vec::new(),
            shifts: super::root::LineShifts::default(), // cov:ignore: empty IFC fallback has no runtime placement to measure.
            fragment_box_placements: Vec::new(), // cov:ignore: empty IFC fallback has no runtime placement to measure.
            fragmentainer_line_ranges: None, // cov:ignore: empty IFC fallback has no runtime placement to measure.
            unfragmented_tail_column: None,
            placed_blocks: Vec::new(),
            forced_breaks: Vec::new(),
        };
    };
    // One inner function takes the context from both arms: the caller's
    // context and a local one cannot share a lifetime, because the pointee of
    // `&mut BlockContext<'_>` is invariant.
    let outer_group = tree.context.text_group;
    let own_group = root.options.text_group_align != TextGroupAlign::None;
    match block_ctx {
        Some(ctx) => {
            join_group(tree, &mut root);
            if own_group {
                root.options.group_padding = own_group_padding(
                    tree,
                    &root,
                    geometry,
                    ctx,
                    (false, top_margin_escapes, bottom_margin_escapes),
                    outer_group,
                );
            }
            let lines = run_boxes(
                tree,
                idx,
                &root,
                geometry,
                ctx,
                perform,
                false,
                top_margin_escapes,
                bottom_margin_escapes,
            );
            leave_group(tree, own_group, &lines, outer_group);
            lines
        }
        None => {
            let mut bfc = BlockFormattingContext::new();
            let mut ctx = bfc.root_block_context();
            let inline_insets = geometry.inline_axis_insets();
            ctx.set_width(geometry.width + inline_insets.0 + inline_insets.1);
            join_group(tree, &mut root);
            if own_group {
                root.options.group_padding = own_group_padding(
                    tree,
                    &root,
                    geometry,
                    &mut ctx,
                    (true, false, false),
                    outer_group,
                );
            }
            let lines = run_boxes(
                tree, idx, &root, geometry, &mut ctx, perform, true, false, false,
            );
            leave_group(tree, own_group, &lines, outer_group);
            lines
        }
    }
}

/// Take the group the root's lines belong to, when it declares none of its
/// own: a group rooted above reaches its lines through the block context.
fn join_group(tree: &LayoutSession<'_>, root: &mut IfcRoot) {
    if let (TextGroupAlign::None, TextGroupScope::Applying { align, padding }) =
        (root.options.text_group_align, tree.context.text_group)
    {
        root.options.text_group_align = align;
        root.options.group_padding = padding;
    }
}

/// Report the root's lines to the group measuring them, and restore the
/// scope the root's own group replaced.
fn leave_group(
    tree: &mut LayoutSession<'_>,
    own_group: bool,
    lines: &IfcLines,
    outer: TextGroupScope,
) {
    if own_group {
        // A root with its own group measures and pads itself.
        tree.context.text_group = outer;
    } else if matches!(tree.context.text_group, TextGroupScope::Probing) {
        tree.context.text_group_shortest =
            shortest_remaining(lines, tree.context.text_group_shortest);
    }
}

/// The least space a non-empty line of `lines` leaves unfilled, or `other`
/// when that is less.
fn shortest_remaining(lines: &IfcLines, other: Option<f32>) -> Option<f32> {
    let remaining = lines.lines.iter().filter(|line| !line.is_empty());
    remaining
        .map(shodo::Line::remaining_inline_size)
        .chain(other)
        .reduce(f32::min)
}

/// [`group_padding`] for a root that declares a group, unless it sits in a
/// group being measured or skipped: its lines are not part of that group, so
/// it is laid out once, unpadded, and its descendants do not join anything.
fn own_group_padding(
    tree: &mut LayoutSession<'_>,
    root: &IfcRoot,
    geometry: FlowGeometry,
    ctx: &mut BlockContext<'_>,
    (local, top_margin_escapes, bottom_margin_escapes): (bool, bool, bool),
    outer: TextGroupScope,
) -> f32 {
    if matches!(outer, TextGroupScope::Probing | TextGroupScope::Skipping) {
        tree.context.text_group = TextGroupScope::Skipping;
        0.0
    } else {
        group_padding(
            tree,
            root,
            geometry,
            ctx,
            local,
            top_margin_escapes,
            bottom_margin_escapes,
        )
    }
}

/// The padding `text-group-align` takes from every line of the root: the
/// shortest space a line leaves unfilled (CSS Text 4, group alignment). The
/// lines are laid out once without it to find that space, against a copy of
/// the float context; group padding only takes space a line leaves, so the
/// lines break the same with it.
///
/// The lines of descendant blocks in the same block formatting context take
/// part too, through the group scope of the layout context; a root split
/// across fragmentainers is not group-aligned. The scope is left applying
/// the padding for the real layout of the root's block children.
fn group_padding(
    tree: &mut LayoutSession<'_>,
    root: &IfcRoot,
    geometry: FlowGeometry,
    ctx: &mut BlockContext<'_>,
    local: bool,
    top_margin_escapes: bool,
    bottom_margin_escapes: bool,
) -> f32 {
    if root.options.text_group_align == TextGroupAlign::None {
        return 0.0;
    }
    let align = root.options.text_group_align;
    tree.context.text_group = TextGroupScope::Probing;
    tree.context.text_group_shortest = None;
    let saved = ctx.snapshot();
    let probe = run_boxes_segment(
        tree,
        root,
        geometry,
        ctx,
        false,
        local,
        top_margin_escapes,
        bottom_margin_escapes,
        root.line_paragraph.start_token(),
        None,
        None,
        false,
        None,
        LineStart::PARAGRAPH,
    );
    ctx.restore(&saved);
    // The block children laid out by the probe reported their lines to the
    // scope; the root's own lines join them.
    let reported = tree.context.text_group_shortest.take();
    let padding = shortest_remaining(&probe.lines, reported).unwrap_or(0.0);
    tree.context.text_group = TextGroupScope::Applying { align, padding };
    padding
}

/// Lay an IFC root across horizontal fragmentainers with a fresh float context per column.
// cov:ignore: fragmentainer-local IFC layout is exercised by the ignored flex-float WPT.
pub(crate) fn layout_with_boxes_in_fragmentainers(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    geometry: FlowGeometry,
    context: FragmentationContext,
    perform: bool,
) -> IfcLines {
    resolve_root_percentages(tree, idx, &geometry);
    let Some(root) = tree.nodes[idx]
        .ifc
        .as_ref()
        .map(|root| root.without_lines())
    else {
        return IfcLines {
            width: geometry.width,
            lines: std::sync::Arc::new(Vec::new()),
            oof_candidates: OofCandidates::new(),
            height: 0.0,
            beside_floats: false,
            escaping_margin: CollapsibleMarginSet::ZERO,
            leading_block_margin: None,
            block_line_starts: Vec::new(),
            generated_block_starts: Vec::new(),
            block_ends: Vec::new(),
            shifts: super::root::LineShifts::default(),
            fragment_box_placements: Vec::new(),
            fragmentainer_line_ranges: Some(Vec::new()),
            unfragmented_tail_column: None,
            placed_blocks: Vec::new(),
            forced_breaks: Vec::new(),
        };
    };

    let mut token = root.line_paragraph.start_token();
    let mut next_start = LineStart::PARAGRAPH;
    let mut all_lines = Vec::new();
    let mut block_line_starts = Vec::new();
    let mut shifts = super::root::LineShifts::default();
    let mut fragment_box_placements = Vec::new();
    let mut fragmentainer_line_ranges = Vec::new();
    let mut unfragmented_tail_column = None;
    let mut atomic_outputs = Vec::new();
    let mut oof_candidates = OofCandidates::new();
    let mut max_height = 0.0_f32;
    let mut beside_floats = false;
    let mut escaping_margin = CollapsibleMarginSet::ZERO;

    let fragmentainer_height = context.available_height.filter(|height| *height > 0.0);
    // A paragraph can need more columns than the declared count. Bound the
    // continuation loop by its source units and float span, then cap subpixel
    // fragmentainers. The final segment overflows unfragmented at the cap.
    const MAX_IFC_FRAGMENTAINERS: usize = 1_024;
    let float_columns = fragmentainer_height
        .map(|height| {
            let float_extent = root
                .boxes
                .iter()
                .filter(|box_| box_.kind == IfcBoxKind::Float)
                .map(|box_| {
                    measure_float(tree, box_.node, geometry)
                        .margin_box
                        .height
                        .max(0.0)
                })
                .sum::<f32>();
            (float_extent / height).ceil() as usize
        })
        .unwrap_or(0);
    let column_budget = root
        .line_paragraph
        .text()
        .len()
        .saturating_add(root.boxes.len())
        .saturating_add(float_columns)
        .saturating_add(context.column_count)
        .clamp(1, MAX_IFC_FRAGMENTAINERS);
    let mut source_float_placements = Vec::new();
    // A generated block that a column ends inside goes on in the next.
    let mut carried_block = None;
    for offset in 0..column_budget {
        let column = context.column_index.saturating_add(offset);
        let has_next = offset + 1 < column_budget;
        let segment_height = has_next.then_some(fragmentainer_height).flatten();
        let column_geometry = FlowGeometry {
            top_edge: if column == context.column_index {
                geometry.top_edge
            } else {
                0.0
            },
            ..geometry
        };
        let mut bfc = BlockFormattingContext::new();
        let mut ctx = bfc.root_block_context();
        let inline_insets = geometry.inline_axis_insets();
        ctx.set_width(geometry.width + inline_insets.0 + inline_insets.1);
        let carried_placements = if perform {
            carried_float_placements(
                tree,
                &source_float_placements,
                context.column_index,
                column,
                fragmentainer_height,
            )
        } else {
            Vec::new()
        };
        if let Some(height) = fragmentainer_height {
            seed_carried_floats(
                tree,
                &mut ctx,
                column_geometry,
                &source_float_placements,
                context.column_index,
                column,
                height,
            );
        }
        let mut outcome = run_boxes_segment(
            tree,
            &root,
            column_geometry,
            &mut ctx,
            perform,
            true,
            false,
            false,
            token,
            segment_height,
            Some(column),
            has_next,
            carried_block,
            next_start,
        );
        next_start = outcome.next_start;
        carried_block = outcome
            .open_block
            .map(|open| open.continued(fragmentainer_height.unwrap_or_default()));

        let line_start = all_lines.len();
        let line_count = outcome.lines.lines.len();
        if !has_next
            && let Some(height) = fragmentainer_height
            && let Some(last) = outcome.lines.lines.last()
            && last.block_offset() + last.block_size() > height
        {
            unfragmented_tail_column = Some(column);
        }
        for (node, first) in outcome.lines.block_line_starts {
            block_line_starts.push((node, line_start + first));
        }
        for (first, shift) in outcome.lines.shifts {
            shifts.push(line_start + first, shift);
        }
        if line_count > 0 {
            let first = outcome.lines.lines[0].block_offset();
            fragmentainer_line_ranges.push(crate::node::MulticolTextFragment {
                line_start,
                line_end: line_start + line_count,
                fragmentainer: column,
                x: context.column_offset_x(column) - context.column_offset_x(context.column_index),
                y: first,
            });
        }
        max_height = max_height.max(outcome.lines.height);
        beside_floats |= outcome.lines.beside_floats;
        escaping_margin = outcome.lines.escaping_margin;
        for placement in &outcome.lines.fragment_box_placements {
            if !source_float_placements
                .iter()
                .any(|source| source.node_id == placement.node_id)
            {
                source_float_placements.push(*placement);
            }
        }
        fragment_box_placements.extend(carried_placements);
        fragment_box_placements.extend(outcome.lines.fragment_box_placements);
        oof_candidates.append(&mut outcome.lines.oof_candidates);
        atomic_outputs.extend(outcome.atomic_outputs);
        let column_lines = std::sync::Arc::try_unwrap(outcome.lines.lines)
            .unwrap_or_else(|_| unreachable!("column lines have a single owner"));
        all_lines.extend(column_lines);

        if let Some(next) = outcome.continuation {
            token = next;
        } else {
            break;
        }
    }

    let mut lines = std::sync::Arc::new(all_lines);
    if perform {
        truncate_for_text_overflow(tree, &root, &mut lines, geometry.width);
    }
    let mut result = IfcLines {
        width: geometry.width,
        lines: lines.clone(),
        oof_candidates,
        height: max_height,
        beside_floats,
        escaping_margin,
        leading_block_margin: None,
        block_line_starts,
        generated_block_starts: Vec::new(),
        block_ends: Vec::new(),
        shifts,
        fragment_box_placements,
        fragmentainer_line_ranges: Some(fragmentainer_line_ranges),
        unfragmented_tail_column,
        placed_blocks: Vec::new(),
        forced_breaks: Vec::new(),
    };
    if perform {
        let mut atomic_candidates =
            place_atomics(tree, &lines, &atomic_outputs, geometry, root.rtl);
        result.oof_candidates.append(&mut atomic_candidates);
        hide_truncated_atomics(tree, &lines, &atomic_outputs);
        offset_boxes_inside_relative_inlines(tree, idx, &root);
        super::records::record_inline_boxes(tree, idx, &root, &lines, &geometry, session_edges);
    }
    result
}

// cov:ignore: carried placements are exercised by the ignored flex-float WPT.
fn carried_float_placements(
    tree: &LayoutSession<'_>,
    sources: &[IfcBoxFragment],
    first_column: usize,
    column: usize,
    fragmentainer_height: Option<f32>,
) -> Vec<IfcBoxFragment> {
    let Some(height) = fragmentainer_height.filter(|height| *height > 0.0) else {
        return Vec::new();
    };
    let column_origin = column.saturating_sub(first_column) as f32 * height;
    sources
        .iter()
        .filter(|source| source.fragmentainer < column)
        .filter_map(|source| {
            let source_origin = source.fragmentainer.saturating_sub(first_column) as f32 * height;
            let source_y = source_origin + source.rect.y;
            (source_y + source.rect.height > column_origin).then_some(IfcBoxFragment {
                node_id: source.node_id,
                fragmentainer: column,
                rect: crate::fragment::FragmentRect {
                    y: source.rect.y - (column - source.fragmentainer) as f32 * height,
                    ..source.rect
                },
            })
        })
        .filter(|placement| tree.physical_style(placement.node_id).float().is_floated())
        .collect()
}

// cov:ignore: carried float seeding is exercised by the ignored flex-float WPT.
fn seed_carried_floats(
    tree: &LayoutSession<'_>,
    ctx: &mut BlockContext<'_>,
    geometry: FlowGeometry,
    sources: &[IfcBoxFragment],
    first_column: usize,
    column: usize,
    fragmentainer_height: f32,
) {
    if fragmentainer_height <= 0.0 {
        return;
    }
    let column_origin = column.saturating_sub(first_column) as f32 * fragmentainer_height;
    for source in sources
        .iter()
        .filter(|source| source.fragmentainer < column)
    {
        let layout = tree.nodes[source.node_id].layout;
        let margin = resolved_margins(tree, source.node_id, geometry.width);
        let source_column_origin =
            source.fragmentainer.saturating_sub(first_column) as f32 * fragmentainer_height;
        let float_top = source_column_origin + source.rect.y - margin.top;
        let float_bottom = float_top + source.rect.height + margin.top + margin.bottom;
        let visible_top = float_top.max(column_origin);
        let visible_bottom = float_bottom.min(column_origin + fragmentainer_height);
        if visible_bottom <= visible_top {
            continue;
        }
        let style = tree.physical_style(source.node_id);
        ctx.place_floated_box(
            Size {
                width: layout.size.width + margin.left + margin.right,
                height: visible_bottom - visible_top,
            },
            geometry.top_edge + visible_top - column_origin,
            FloatDirection::from_float(style.float()).unwrap_or(FloatDirection::Left),
            style.clear(),
            false,
        );
    }
}

/// The line loop. `local` is true when the context is this paragraph's own,
/// which makes the paragraph a block formatting context that contains its
/// floats and its children's margins. `bottom_margin_escapes` is described
/// at [`layout_with_boxes_in`].
#[allow(clippy::too_many_arguments)]
fn run_boxes(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    root: &IfcRoot,
    geometry: FlowGeometry,
    ctx: &mut BlockContext<'_>,
    perform: bool,
    local: bool,
    top_margin_escapes: bool,
    bottom_margin_escapes: bool,
) -> IfcLines {
    let mut outcome = run_boxes_segment(
        tree,
        root,
        geometry,
        ctx,
        perform,
        local,
        top_margin_escapes,
        bottom_margin_escapes,
        root.line_paragraph.start_token(),
        None,
        None,
        false,
        None,
        LineStart::PARAGRAPH,
    );
    if perform {
        truncate_for_text_overflow(tree, root, &mut outcome.lines.lines, geometry.width);
        let mut atomic_candidates = place_atomics(
            tree,
            &outcome.lines.lines,
            &outcome.atomic_outputs,
            geometry,
            root.rtl,
        );
        outcome.lines.oof_candidates.append(&mut atomic_candidates);
        hide_truncated_atomics(tree, &outcome.lines.lines, &outcome.atomic_outputs);
        offset_boxes_inside_relative_inlines(tree, idx, root);
        super::records::record_inline_boxes(
            tree,
            idx,
            root,
            &outcome.lines.lines,
            &geometry,
            session_edges,
        );
    }
    outcome.lines
}

#[allow(clippy::too_many_arguments)]
fn run_boxes_segment(
    tree: &mut LayoutSession<'_>,
    root: &IfcRoot,
    geometry: FlowGeometry,
    ctx: &mut BlockContext<'_>,
    perform: bool,
    local: bool,
    top_margin_escapes: bool,
    bottom_margin_escapes: bool,
    mut token: BreakToken,
    fragmentainer_height: Option<f32>,
    fragmentainer: Option<usize>,
    has_next_fragmentainer: bool,
    mut open_block: Option<generated_blocks::OpenBlock>,
    start: LineStart,
) -> RunBoxesOutcome {
    let FlowGeometry {
        width, top_edge, ..
    } = geometry;
    let mut options = root.options;
    options.text_indent.length = resolve_indent(root.indent, width);
    // Atomic inlines are sized once, before any line: their width does not
    // depend on the line they land in.
    let atomics = measure_atomics(tree, root, geometry);
    // Keeps the paragraph's own floats inside its content box.
    let inline_insets = geometry.inline_axis_insets();
    ctx.apply_content_box_inset([inline_insets.0, inline_insets.1]);

    if !top_margin_escapes {
        ctx.commit_strut();
    }
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
    // The collapsed top margins of a block child that starts the paragraph,
    // with no line, float or other margin above it.
    let mut leading_block_margin = None;
    let mut leading_margin_open = true;
    let mut block_line_starts = Vec::new();
    let mut generated_block_starts = Vec::new();
    let mut block_ends = Vec::new();
    let mut fragment_box_placements = Vec::new();
    let mut oof_candidates = OofCandidates::new();
    let mut continuation = None;
    // `open_block` is the block-level `::before` or `::after` whose lines
    // are being laid out: its start token was met and its end token was not.
    // The tokens come in document order; the cursor is the block of the next.
    let mut block_cursor = 0;
    // The boxes of the generated blocks closed so far, and the forced page
    // breaks they ask for.
    let mut placed_blocks = Vec::new();
    let mut forced_breaks = Vec::new();
    let horizontal = root.writing_mode == WritingMode::HorizontalTb;
    // The next line follows a forced break or a block: the engine treats it
    // as such for `text-indent: each-line`.
    let mut after_forced = start.after_forced;

    loop {
        calls += 1;
        if calls > max_calls {
            debug_assert!(false, "the line loop does not converge");
            break;
        }
        // Where the next line starts if it is a line: below the margins
        // still pending.
        let line_y = if top_margin_escapes && leading_margin_open {
            y
        } else {
            y + pending.resolve()
        };
        let base = line_space(
            ctx,
            inline_insets.0,
            top_edge,
            width,
            line_y,
            assumed_height,
        );
        let space = narrow_to_block(shorten(base, &tentative), open_block.as_ref(), width);
        // The lines of a generated block take its own alignment and indent.
        // Its content is part of the paragraph's text, so its lines keep the
        // paragraph's base direction.
        let (line_options, first_line) = open_block.as_ref().map_or_else(
            || (options, start.first && lines.is_empty()),
            |open| open.line_options(lines.len()),
        );
        let line_options = with_group_align(line_options, &options);
        let mut constraint = line_constraint(root, width, space, open_block.is_some());
        constraint.block_offset = line_y;
        constraint.floats_placed_through = cursor;
        // The state is taken only for the engine call: measuring a float
        // below lays out another node, which may be an ifc root itself.
        let result = with_state(tree, |state| {
            root.line_paragraph.next_line(
                &mut state.layout_cx,
                token,
                &line_options,
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
                let narrowed =
                    !tentative.is_empty() || space.inline_start != 0.0 || space.inline_size < width;
                let fits = float.margin_box.width <= space.inline_size || !narrowed;
                // A float that only fits below an earlier float may be
                // moved into the next fragmentainer. Do not commit its
                // intermediate position past this fragmentainer's bottom.
                if has_next_fragmentainer
                    && !fits
                    && let Some(height) = fragmentainer_height
                    && next_float_edge(ctx, top_edge, line_y).is_some_and(|edge| edge >= height)
                {
                    continuation = Some(token);
                    break;
                }
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
                if is_empty_closure(&line) {
                    // Empty inline closures around a block do not separate
                    // its adjoining margins or add a line box (CSS 2.2 9.4.2).
                    token = line.break_token();
                    after_forced = false;
                    continue;
                }
                // The line may span more of an outer float than the height it
                // was assumed to have; lay it out again if the space for its
                // real height differs.
                let real_base = line_space(ctx, inline_insets.0, top_edge, width, line_y, height);
                let real =
                    narrow_to_block(shorten(real_base, &tentative), open_block.as_ref(), width);
                if retries < MAX_SPACE_RETRIES && real != space {
                    assumed_height = height;
                    retries += 1;
                    continue;
                }
                if !lines.is_empty() // cov:ignore: continuation requires lines to overflow a fragmentainer and is exercised by the ignored flex-float WPT.
                    && fragmentainer_height.is_some_and(|limit| line_y + height > limit + 0.001)
                {
                    continuation = Some(token); // cov:ignore: this continuation branch is exercised by the ignored flex-float WPT.
                    break; // cov:ignore: this continuation branch is exercised by the ignored flex-float WPT.
                } // cov:ignore: continuation requires lines to overflow a fragmentainer and is exercised by the ignored flex-float WPT.
                // A line that does not fit beside the floats is moved down
                // until it fits or no float is left beside it (CSS 2.1 9.5).
                // Its own floats keep the place they have at this offset.
                // The engine indents the first line, and with `each-line` a
                // line after a forced break or a block; a hanging indent is on
                // every other line instead (CSS Text 3 §8.1).
                let text_indent = line_options.text_indent;
                let indented = first_line || (text_indent.each_line && after_forced);
                let indent = if indented != text_indent.hanging {
                    line_options.text_indent.length
                } else {
                    0.0
                };
                let narrowed = space.inline_start != 0.0 || space.inline_size < width;
                // A line that cannot wrap overflows where it is instead: moving
                // it below the floats would act as a soft wrap. A box that
                // keeps clear of the floats has none beside its lines.
                let overflows = narrowed
                    && !open_block
                        .as_ref()
                        .is_some_and(generated_blocks::OpenBlock::avoids_floats)
                    && !root.line_is_nowrap(&line)
                    && line.inline_size() + indent > space.inline_size + 0.01
                    && moves < MAX_LINE_MOVES;
                if overflows {
                    for float in tentative.drain(..) {
                        ceiling = ceiling.max(commit_fragmented_float(
                            tree,
                            ctx,
                            float,
                            line_y,
                            geometry,
                            FragmentedFloatOutput {
                                perform,
                                adjoins_top_margin: false,
                                fragmentainer,
                                placements: &mut fragment_box_placements,
                                oof_candidates: &mut oof_candidates,
                            },
                        ));
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
                    ceiling = ceiling.max(commit_fragmented_float(
                        tree,
                        ctx,
                        float,
                        line_y,
                        geometry,
                        FragmentedFloatOutput {
                            perform,
                            adjoins_top_margin: false,
                            fragmentainer,
                            placements: &mut fragment_box_placements,
                            oof_candidates: &mut oof_candidates,
                        },
                    ));
                }
                withdrawn.clear();
                assumed_height = 0.0;
                retries = 0;
                calls = 0;
                moves = 0;
                y = line_y + height;
                pending = CollapsibleMarginSet::ZERO;
                token = line.break_token();
                after_forced = line.break_reason() == shodo::BreakReason::Forced;
                // A cleared `<br>` ends this line: the next one starts below
                // the floats it clears (CSS 2.1 9.5.2).
                if line.break_reason() == shodo::BreakReason::Forced
                    && let Some(clear) = cleared_break(&line, &root.cleared_breaks)
                    && let Some(threshold) = ctx.cleared_threshold(clear)
                {
                    y = y.max(threshold - top_edge);
                }
                leading_margin_open = false;
                ctx.commit_strut();
                lines.push(line);
                // Floats that did not fit beside the line they are anchored
                // in go at the start of the next line, before it is measured.
                // Placing them any earlier would narrow the line that is
                // still being retried.
                for float in deferred.drain(..) {
                    ceiling = ceiling.max(commit_fragmented_float(
                        tree,
                        ctx,
                        float,
                        y,
                        geometry,
                        FragmentedFloatOutput {
                            perform,
                            adjoins_top_margin: false,
                            fragmentainer,
                            placements: &mut fragment_box_placements,
                            oof_candidates: &mut oof_candidates,
                        },
                    ));
                }
            }
            LineResult::BlockInInline { node, token_after } => {
                // Floats anchored before the block and not placed yet go
                // above it: the block starts below the last line.
                // cov:ignore: fragmented block-in-inline float placement is exercised by the ignored flex-float WPT.
                for float in tentative.drain(..).chain(deferred.drain(..)) {
                    ceiling = ceiling.max(commit_fragmented_float(
                        tree,
                        ctx,
                        float,
                        line_y,
                        geometry,
                        FragmentedFloatOutput {
                            perform,
                            adjoins_top_margin: top_margin_escapes && leading_margin_open,
                            fragmentainer,
                            placements: &mut fragment_box_placements,
                            oof_candidates: &mut oof_candidates,
                        },
                    ));
                }
                if let Some((index, edge)) =
                    generated_blocks::token(&root.generated_blocks, node.0, block_cursor)
                {
                    block_cursor = index;
                    if matches!(edge, generated_blocks::Token::Start(_)) {
                        generated_block_starts.push((block_line_starts.len(), lines.len()));
                    }
                    // A generated block is laid out along the block axis of
                    // a horizontal root, and of a vertical one when the box
                    // is on its axes; a vertical root breaks its lines at the
                    // tokens of the others.
                    let (generated_blocks::Token::Start(block)
                    | generated_blocks::Token::End(block)) = edge;
                    if horizontal || block.on_root_axes {
                        let at_start = leading_margin_open
                            && lines.is_empty()
                            && y == 0.0
                            && (top_margin_escapes || ceiling == f32::NEG_INFINITY);
                        let flow = generated_blocks::Flow {
                            y: &mut y,
                            pending: &mut pending,
                            leading_block_margin: &mut leading_block_margin,
                            lines: &lines,
                            ctx,
                            width,
                            content_left: inline_insets.0,
                            top_edge,
                            definite_height: if horizontal {
                                geometry.definite_height
                            } else {
                                geometry.vertical_width
                            },
                            at_start,
                            top_margin_escapes,
                            rtl: root.rtl && horizontal,
                        };
                        match edge {
                            generated_blocks::Token::Start(block) => {
                                let mut lines_height = |open: &generated_blocks::OpenBlock| {
                                    generated_block_lines_height(
                                        tree,
                                        root,
                                        &atomics,
                                        open,
                                        (token_after, block.end),
                                        width,
                                    )
                                };
                                open_block =
                                    Some(generated_blocks::open(block, flow, &mut lines_height));
                            }
                            generated_blocks::Token::End(block) => {
                                block_cursor += 1;
                                if let Some(open) = open_block.take() {
                                    let placed = generated_blocks::close(block, open, flow);
                                    // Pagination moves the lines of a paragraph
                                    // down at a forced break; a vertical
                                    // paragraph's lines are columns it does not
                                    // break between.
                                    if horizontal {
                                        forced_breaks.extend(block.forced_breaks(
                                            &open,
                                            placed.as_ref(),
                                            lines.len(),
                                        ));
                                    }
                                    if let Some(placed) = placed {
                                        leading_margin_open = false;
                                        ctx.commit_strut();
                                        placed_blocks.push(placed);
                                    }
                                }
                            }
                        }
                    }
                    token = token_after;
                    after_forced = true;
                    withdrawn.clear();
                    assumed_height = 0.0;
                    retries = 0;
                    moves = 0;
                    calls = 0;
                    continue;
                }
                let starts_paragraph = leading_margin_open
                    && lines.is_empty()
                    && y == 0.0
                    && (top_margin_escapes || ceiling == f32::NEG_INFINITY);
                block_line_starts.push((node.0 as usize, lines.len()));
                let mut block = if geometry.blocks_apart {
                    BlockChild::apart(y, pending)
                } else {
                    layout_block_child(
                        tree,
                        ctx,
                        node.0 as usize,
                        y,
                        pending,
                        geometry,
                        perform,
                        top_margin_escapes && starts_paragraph,
                    )
                };
                oof_candidates.append(&mut block.oof_candidates);
                if starts_paragraph {
                    if let Some(margin) = block.top_margin {
                        leading_block_margin = Some(margin);
                    }
                    leading_margin_open = block.collapses_through;
                }
                block_floats_bottom = block_floats_bottom.max(block.top + block.floats_bottom);
                if !block.collapses_through {
                    ctx.commit_strut();
                }
                (y, pending) = (block.next_y, block.pending);
                block_ends.push((node.0 as usize, lines.len(), y + pending.resolve()));
                // A forced page break after the block starts the lines after
                // it on a new page, which the floats before the break do not
                // reach: the lines get clearance past them, as a block after
                // the break does (CSS Fragmentation 3, 3.1).
                if tree
                    .forced_break_line_clearance
                    .binary_search(&(node.0 as usize))
                    .is_ok()
                    && let Some(threshold) = ctx.cleared_threshold(Clear::Both)
                    && threshold - top_edge > y + pending.resolve()
                {
                    y = threshold - top_edge;
                    pending = CollapsibleMarginSet::ZERO;
                }
                token = token_after;
                after_forced = true;
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
    if top_margin_escapes && leading_margin_open {
        pending = CollapsibleMarginSet::ZERO;
    }
    let escaping_margin = if continuation.is_some() {
        CollapsibleMarginSet::ZERO
    } else if bottom_margin_escapes {
        pending
    } else {
        y += pending.resolve();
        CollapsibleMarginSet::ZERO
    };
    // A float met after the last line still gets a place.
    // cov:ignore: trailing fragmented floats are exercised by the ignored flex-float WPT.
    if continuation.is_none() {
        for float in tentative.drain(..).chain(deferred.drain(..)) {
            commit_fragmented_float(
                tree,
                ctx,
                float,
                y,
                geometry,
                FragmentedFloatOutput {
                    perform,
                    adjoins_top_margin: false,
                    fragmentainer,
                    placements: &mut fragment_box_placements,
                    oof_candidates: &mut oof_candidates,
                },
            );
        }
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
    // A box still open goes on with the lines it has laid out here.
    let open_block = open_block.map(|open| open.after(lines.len()));
    let next_start = LineStart {
        first: start.first && lines.is_empty(),
        after_forced,
    };
    RunBoxesOutcome {
        lines: IfcLines {
            width,
            height,
            lines: std::sync::Arc::new(lines),
            oof_candidates,
            beside_floats: beside,
            escaping_margin,
            leading_block_margin,
            block_line_starts,
            generated_block_starts,
            block_ends,
            shifts: super::root::LineShifts::default(),
            fragment_box_placements,
            fragmentainer_line_ranges: None,
            unfragmented_tail_column: None,
            placed_blocks,
            forced_breaks,
        },
        continuation,
        atomic_outputs: atomics.outputs,
        open_block,
        next_start,
    }
}

fn commit_fragmented_float(
    tree: &mut LayoutSession<'_>,
    ctx: &mut BlockContext<'_>,
    float: TentativeFloat,
    y: f32,
    geometry: FlowGeometry,
    output: FragmentedFloatOutput<'_>,
) -> f32 {
    let FragmentedFloatOutput {
        perform,
        adjoins_top_margin,
        fragmentainer,
        placements,
        oof_candidates,
    } = output;
    let node = float.node;
    let (top, mut child_candidates) =
        commit_float(tree, ctx, float, y, geometry, perform, adjoins_top_margin);
    oof_candidates.append(&mut child_candidates);
    if perform && let Some(fragmentainer) = fragmentainer {
        let layout = tree.nodes[node].layout;
        placements.push(IfcBoxFragment {
            node_id: node,
            fragmentainer,
            rect: crate::fragment::FragmentRect {
                x: layout.location.x,
                y: layout.location.y,
                width: layout.size.width,
                height: layout.size.height,
            },
        });
    }
    top
}

/// The `clear` of the cleared `<br>` that ends `line`, if any.
fn cleared_break(line: &shodo::Line, cleared: &ClearBreakIndex) -> Option<Clear> {
    let node = line.forced_break()?.node.0 as usize;
    #[cfg(all(test, debug_assertions))]
    tests::clear_break_tests::note_probe();
    cleared.get(node)
}

/// Move every box that sits inside relatively positioned inline elements of
/// the paragraph by their offsets: an offset moves the element together with
/// everything inside it (CSS 2.1 9.4.3).
fn offset_boxes_inside_relative_inlines(tree: &mut LayoutSession<'_>, idx: usize, root: &IfcRoot) {
    if root.offsets.is_empty() {
        return;
    }
    // The content of an anonymous cell sits in the cell's owner.
    let owner = tree.ifc_source_owner(idx);
    for b in &root.boxes {
        let (dx, dy) = relative_offset_of(tree, idx, owner, &root.offsets, b.node);
        if dx != 0.0 || dy != 0.0 {
            let mut layout = tree.nodes[b.node].layout;
            layout.location.x += dx;
            layout.location.y += dy;
            tree.set_unrounded_layout(NodeId::from(b.node), &layout);
        }
    }
}

/// The offsets of the relatively positioned inline elements that contain
/// `node`, up to the paragraph root `idx` (or `owner`, the source owner of an
/// anonymous cell's content), added up. `offsets` holds the offset of each
/// element by node id.
fn relative_offset_of(
    tree: &Document,
    idx: usize,
    owner: usize,
    offsets: &[(usize, (f32, f32))],
    node: usize,
) -> (f32, f32) {
    tree.ancestors(node)
        .take_while(|&id| id != idx && id != owner)
        .filter_map(|id| offsets.iter().find(|(element, _)| *element == id))
        .fold((0.0, 0.0), |(dx, dy), (_, (x, y))| (dx + x, dy + y))
}

/// The nearest ancestor of `node` inside the paragraph rooted at `idx` (or at
/// `owner`, the source owner of an anonymous cell's content) that is the
/// containing block of `node`: for an absolutely positioned box one that is
/// positioned or establishes the containing block of fixed boxes, for a fixed
/// box the latter only. Only a non-atomic inline element can be one there, as
/// an atomic inline lays its own children out.
fn positioned_inline_ancestor(
    tree: &Document,
    idx: usize,
    owner: usize,
    node: usize,
) -> Option<usize> {
    let fixed = tree.nodes[node].position() == &PositionValue::Fixed;
    tree.ancestors(node)
        .take_while(|&id| id != idx && id != owner)
        .find(|&id| {
            let ancestor = &tree.nodes[id];
            ancestor.display() != DisplayValue::Contents
                && if fixed {
                    ancestor.claims_fixed_oof
                } else {
                    ancestor.claims_absolute_oof || ancestor.position() != &PositionValue::Static
                }
        })
}

/// The padding box of the inline element `inline`, in the coordinates of the
/// paragraph's content box: the bounding box of the padding boxes of its
/// `pieces` on the lines (CSS 2.1 10.1). `basis` resolves its borders, and
/// `line_shift` gives the move pagination made to the lines. An element with
/// no piece of its own, such as a ruby base container, which has no box and
/// holds its bases, spans the pieces of the elements in it. `None` when
/// there is none.
fn inline_padding_box(
    tree: &LayoutSession<'_>,
    pieces: &[super::inline_boxes::InlineBoxPiece],
    inline: usize,
    basis: f32,
    line_shift: impl Fn(usize) -> (f32, f32),
) -> Option<PaddingBox> {
    let mut border = tree
        .physical_style(inline)
        .border()
        .resolve_or_zero(Some(basis));
    let mut spanned: Vec<_> = pieces.iter().filter(|piece| piece.node == inline).collect();
    if spanned.is_empty() {
        border = Edges::ZERO;
        spanned = pieces
            .iter()
            .filter(|piece| tree.ancestors(piece.node).any(|id| id == inline))
            .collect();
    }
    // The edges of a piece: its border box less its borders. A side that is
    // not an edge of the element has no border, and the content box reaches
    // the border box there.
    let (left, top, right, bottom) = spanned
        .into_iter()
        .map(|piece| {
            let (outer, inner) = (piece.border_box, piece.content_box);
            let (dx, dy) = line_shift(piece.line);
            (
                outer.x + dx + border.left.min((inner.x - outer.x).max(0.0)),
                outer.y + dy + border.top.min((inner.y - outer.y).max(0.0)),
                outer.x + dx + outer.width
                    - border
                        .right
                        .min((outer.x + outer.width - inner.x - inner.width).max(0.0)),
                outer.y + dy + outer.height
                    - border
                        .bottom
                        .min((outer.y + outer.height - inner.y - inner.height).max(0.0)),
            )
        })
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))?;
    Some(PaddingBox {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
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

/// The space of a line of the generated block `open`, if one is open, set
/// in by its inline-side edges.
fn narrow_to_block(
    space: LineSpace,
    open: Option<&generated_blocks::OpenBlock>,
    width: f32,
) -> LineSpace {
    open.map_or(space, |open| open.narrow(space, width))
}

/// The most lines the lines of a generated block are measured over: more than
/// any paragraph has, so that only an engine that stops advancing reaches it.
const MAX_MEASURED_LINES: usize = 1 << 16;

/// The block size of the lines of the generated block `open`, laid out on
/// their own in a paragraph `width` wide: from the token `after` its start
/// token to its end token `end`, in the space its lines take in the line
/// loop. `None` when anything but its lines is met on the way, as the box
/// then depends on more than its own width.
fn generated_block_lines_height(
    tree: &mut LayoutSession<'_>,
    root: &IfcRoot,
    atomics: &AtomicMeasure,
    open: &generated_blocks::OpenBlock,
    (after, end): (BreakToken, u64),
    width: f32,
) -> Option<f32> {
    let space = open.narrow(
        LineSpace {
            inline_start: 0.0,
            inline_size: width,
        },
        width,
    );
    let constraint = line_constraint(root, width, space, true);
    let mut token = after;
    let mut count = 0;
    let mut height = 0.0;
    (0..MAX_MEASURED_LINES)
        .find_map(|_| {
            let options = with_group_align(open.line_options(count).0, &root.options);
            let result = with_state(tree, |state| {
                root.line_paragraph.next_line(
                    &mut state.layout_cx,
                    token,
                    &options,
                    &constraint,
                    &atomics.sizes,
                )
            });
            match result {
                Some(LineResult::Line(line)) => {
                    token = line.break_token();
                    if !is_empty_closure(&line) {
                        height += line.block_size();
                        count += 1;
                    }
                    None
                }
                other => Some(
                    matches!(other, Some(LineResult::BlockInInline { node, .. }) if node.0 == end)
                        .then_some(height),
                ),
            }
        })
        .flatten()
}

/// The constraint of a line in `space` of a paragraph `width` wide. The lines
/// of a generated block (`in_block`) wider than the paragraph run out past
/// its content box, which the physical insets cannot say.
fn line_constraint(
    root: &IfcRoot,
    width: f32,
    space: LineSpace,
    in_block: bool,
) -> LineConstraint<'_> {
    let right_inset = (width - space.inline_start - space.inline_size).max(0.0);
    let mut constraint = LineConstraint::from_physical_insets(
        width,
        space.inline_start,
        right_inset,
        root.writing_mode,
        if root.rtl {
            Direction::Rtl
        } else {
            Direction::Ltr
        },
    );
    if in_block {
        constraint.available_inline_size = space.inline_size;
    }
    constraint
}

/// `line_options` with the group alignment of the paragraph's `options`: it
/// pads every line of the root, generated blocks' lines included.
fn with_group_align(mut line_options: LineOptions, options: &LineOptions) -> LineOptions {
    if options.text_group_align != TextGroupAlign::None {
        line_options.text_group_align = options.text_group_align;
        line_options.group_padding = options.group_padding;
    }
    line_options
}

/// Whether `line` is the empty line around an inline closure that sits next
/// to a block: it has no content, no height, and does not end in a forced
/// break.
fn is_empty_closure(line: &shodo::Line) -> bool {
    line.is_empty() && line.block_size() == 0.0 && line.break_reason() != shodo::BreakReason::Forced
}

/// Take the tentative floats' margin boxes off the space of a line: a left
/// float moves the start right and takes its width off; a right float only
/// takes its width off.
fn shorten(base: LineSpace, tentative: &[TentativeFloat]) -> LineSpace {
    let mut space = base;
    for float in tentative {
        match float.direction {
            FloatDirection::Left => {
                space.inline_start += float.margin_box.width;
                space.inline_size -= float.margin_box.width;
            }
            FloatDirection::Right => space.inline_size -= float.margin_box.width,
        }
    }
    space.inline_size = space.inline_size.max(0.0);
    space
}

/// Margins of `node` resolved against `basis`, the root's content width; `auto`
/// is zero.
fn resolved_margins(tree: &LayoutSession<'_>, node: usize, basis: f32) -> Edges<f32> {
    tree.physical_style(node)
        .margin()
        .map(|margin| margin.resolve(Some(basis)).unwrap_or(0.0))
}

/// Lay a child of the root out the way taffy's block algorithm lays out a
/// float: no known size but `known_width`, the root's content width as the
/// percentage basis, and `available` as the room it may take. It is also how
/// an atomic inline is sized: shrink-to-fit within the line's content width.
/// Percentage heights resolve against `height_basis`, the root's definite
/// content height, and behave as `auto` without one. A vertical root passes
/// its inline extent as the basis, which its children's edges take their
/// percentages of, and the width its children's own width percentage
/// resolves to as `known_width`.
fn perform_box_layout(
    tree: &mut LayoutSession<'_>,
    node: usize,
    basis: f32,
    height_basis: Option<f32>,
    known_width: Option<f32>,
    available: Size<AvailableSpace>,
) -> LayoutOutput {
    // What taffy's `perform_child_layout` does; that helper is private to
    // taffy.
    tree.compute_child_layout(
        NodeId::from(node),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size {
                width: known_width,
                height: None,
            },
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: Size {
                width: Some(basis),
                height: height_basis,
            },
            available_space: available,
            // The box establishes a new block formatting context: its margins
            // do not collapse with its children's.
            vertical_margins_are_collapsible: Line::FALSE,
        },
    )
}

/// Lay a float out the way the block algorithm does: resolve its margins
/// against the container, then give it the room that is left.
fn measure_float(
    tree: &mut LayoutSession<'_>,
    node: usize,
    geometry: FlowGeometry,
) -> TentativeFloat {
    let margin = resolved_margins(tree, node, geometry.width);
    let vertical = geometry.axes.writing_mode() != WritingMode::HorizontalTb;
    let (inline_margin, block_margin) = if vertical {
        (margin.top + margin.bottom, margin.left + margin.right)
    } else {
        (margin.left + margin.right, margin.top + margin.bottom)
    };
    let available = (geometry.width - inline_margin).max(0.0);
    let known_width = if vertical {
        None
    } else {
        keyword_width(tree, node, &geometry, available)
    };
    let output = perform_box_layout(
        tree,
        node,
        geometry.width,
        geometry.definite_height,
        known_width,
        Size {
            width: AvailableSpace::Definite(available),
            height: AvailableSpace::MaxContent,
        },
    );
    let style = tree.physical_style(node);
    let output_size = shodo::geometry::PhysicalSize {
        width: output.size.width,
        height: output.size.height,
    };
    TentativeFloat {
        node,
        // The projection accepts only left and right floats.
        direction: FloatDirection::from_float(style.float()).unwrap_or(FloatDirection::Left),
        clear: style.clear(),
        margin_box: Size {
            width: geometry.axes.inline_extent(output_size) + inline_margin,
            height: geometry.axes.block_extent(output_size) + block_margin,
        },
        margin,
        output,
    }
}

/// Sizes the paragraph needs for its atomic inlines, and the layouts to store
/// once the lines are final.
pub(crate) struct AtomicMeasure {
    pub(crate) sizes: AtomicSizes,
    pub(crate) outputs: Vec<(usize, LayoutOutput)>,
}

/// Measure every atomic inline of the root at the paragraph's content width.
///
/// Each atomic is laid out as a node of its own, so this runs outside the
/// engine state scope: an atomic may hold another ifc root.
pub(crate) fn measure_atomics(
    tree: &mut LayoutSession<'_>,
    root: &IfcRoot,
    geometry: FlowGeometry,
) -> AtomicMeasure {
    let mut sizes = AtomicSizes::new();
    for &(id, size) in root.marker_atomic.iter().chain(&root.generated_atomics) {
        sizes.insert(id, size);
    }
    // The containing block's physical size: a vertical root's inline size is
    // its height, and its width is its block extent.
    let vertical = root.writing_mode != WritingMode::HorizontalTb;
    let containing = if vertical {
        (geometry.vertical_width, Some(geometry.width))
    } else {
        (Some(geometry.width), geometry.definite_height)
    };
    for generated in root.generated_boxes.iter() {
        sizes.insert(
            shodo::node::NodeId(generated.id),
            generated.size(
                Some(geometry.width),
                containing,
                root.writing_mode,
                root.rtl,
            ),
        );
    }
    let mut outputs = Vec::new();
    for b in root.boxes.iter().filter(|b| b.kind == IfcBoxKind::Atomic) {
        let node = b.node;
        // A child's margins and padding resolve their percentages against the
        // paragraph's inline extent (CSS 2.1 8.3).
        let margin = resolved_margins(tree, node, geometry.width);
        // Only an atomic that lays out on the axes of a vertical paragraph is
        // offered the paragraph's inline extent as its height; an orthogonal
        // one keeps taking it as its width.
        let along_height =
            vertical && crate::layout::dispatch::used_writing_mode(tree, node).is_vertical();
        let inline_margins = if along_height {
            margin.top + margin.bottom
        } else {
            margin.left + margin.right
        };
        let available = (geometry.width - inline_margins).max(0.0);
        // A vertical paragraph's inline extent is the box's height, so its
        // percentages of the width resolve against the containing block's
        // physical width, while the edges' percentages stay with the inline
        // extent (CSS Writing Modes 4 §7.2).
        let known_width = containing
            .0
            .filter(|_| vertical)
            .and_then(|physical_width| {
                used_block_width_with_edge_basis(tree, node, physical_width, geometry.width).map(
                    |width| {
                        clamp_block_width_with_edge_basis(
                            tree,
                            node,
                            physical_width,
                            geometry.width,
                            width,
                        )
                    },
                )
            })
            .or_else(|| {
                if vertical {
                    None
                } else {
                    keyword_width(tree, node, &geometry, available)
                }
            });
        // The room along the line is the room along the paragraph's inline
        // axis, the physical height of a vertical one.
        let room = AvailableSpace::Definite(available);
        let output = perform_box_layout(
            tree,
            node,
            geometry.width,
            geometry.definite_height,
            known_width,
            if along_height {
                Size {
                    width: AvailableSpace::MaxContent,
                    height: room,
                }
            } else {
                Size {
                    width: room,
                    height: AvailableSpace::MaxContent,
                }
            },
        );
        let id = shodo::node::NodeId(node as u64);
        // Only the alphabetic baseline is supplied. shodo measures it from
        // the margin box's line-over edge and falls back to the margin box's
        // line-under edge when there is none (CSS 2.1 10.8.1).
        let baseline = match root.line_paragraph.required_baseline(id) {
            Some(shodo::geometry::BaselineKind::Alphabetic) | None => {
                atomic_baseline(tree, node, &output).and_then(|from_start| {
                    baseline_from_line_over(
                        root.writing_mode,
                        crate::layout::dispatch::used_writing_mode(tree, node),
                        output.size.width,
                        margin,
                        from_start,
                    )
                })
            }
            Some(_) => None,
        };
        // shodo rounds every supplied length to 1/64 px on its own, so the
        // margins and the border box would round separately and the margin
        // box could grow past what the sizes add up to (two halves of a line
        // no longer fitting it). Round the margin box as a whole and give the
        // border box what is left after the rounded margins.
        let snap = |value: f32| (value * 64.0).round() / 64.0;
        // The lines of a vertical paragraph run along the box's height. A
        // horizontal paragraph takes the line-left margin as the inline-start
        // one whatever its direction.
        let (inline_size, block_size, margins) = logical_box(
            (output.size.width, output.size.height),
            (margin.top, margin.right, margin.bottom, margin.left),
            root.writing_mode,
            root.rtl && root.writing_mode != WritingMode::HorizontalTb,
        );
        let outer = snap(inline_size + margins.inline_start + margins.inline_end);
        let inline_size = (outer - snap(margins.inline_start) - snap(margins.inline_end)).max(0.0);
        sizes.insert(
            id,
            AtomicSize {
                inline_size,
                block_size,
                baseline,
                margins,
            },
        );
        outputs.push((node, output));
    }
    AtomicMeasure { sizes, outputs }
}

/// What laying out a block child gives the line loop.
struct BlockChild {
    /// Out-of-flow candidates from this child's subtree, translated into the root's coordinates.
    oof_candidates: OofCandidates,
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
    /// The block's top margin collapsed with the margins that leave it
    /// through its first child, which placed its box: `top` is `y` plus
    /// these collapsed with `pending`. For a collapsed-through box this is
    /// the full pending set, including its bottom margins. `None` when
    /// clearance or floats placed it instead.
    top_margin: Option<CollapsibleMarginSet>,
    /// No content or formatting boundary separates the margins through it.
    collapses_through: bool,
    /// The first baseline of the block, from the content-box top of the root,
    /// when it has one and the root lays out horizontal lines.
    first_baseline: Option<f32>,
}

impl BlockChild {
    /// A block that is not laid out with the lines: it takes no room in them.
    fn apart(y: f32, pending: CollapsibleMarginSet) -> Self {
        Self {
            oof_candidates: OofCandidates::new(),
            top: y,
            next_y: y,
            pending,
            floats_bottom: f32::NEG_INFINITY,
            top_margin: None,
            collapses_through: true,
            first_baseline: None,
        }
    }
}

fn translated_child_oof_candidates(output: &LayoutOutput, location: Point<f32>) -> OofCandidates {
    let mut candidates = output.oof_candidates.clone();
    candidates.translate(location);
    candidates
}

/// How low the floats reach before the float that starts a page, when `node`
/// is the box after that float, in the coordinates of `ctx`.
fn page_start_float_floor(
    tree: &LayoutSession<'_>,
    ctx: &BlockContext<'_>,
    node: usize,
) -> Option<f32> {
    tree.forced_break_float_floor.binary_search(&node).ok()?;
    ctx.page_start_float_floor()
}

/// Lay a block child out below the offset `y`, as wide as the content box
/// less its side margins, the way the parent's block algorithm lays out an
/// in-flow block (CSS 2.1 8.3.1): its top margin, with the margins of its
/// first child that leave it, collapses with the `pending` margins of the
/// block children above it. With `perform`, its final layout is stored.
#[allow(clippy::too_many_arguments)]
fn layout_block_child(
    tree: &mut LayoutSession<'_>,
    ctx: &mut BlockContext<'_>,
    node: usize,
    y: f32,
    pending: CollapsibleMarginSet,
    geometry: FlowGeometry,
    perform: bool,
    collapse_top: bool,
) -> BlockChild {
    if geometry.axes.writing_mode() != WritingMode::HorizontalTb {
        return layout_vertical_block_child(
            tree,
            ctx,
            node,
            y,
            pending,
            geometry,
            perform,
            collapse_top,
        );
    }
    if !in_the_roots_formatting_context(tree, node) {
        return layout_formatting_context_child(
            tree,
            ctx,
            node,
            y,
            pending,
            geometry,
            perform,
            collapse_top,
        );
    }
    let mut margin = resolved_margins(tree, node, geometry.width);
    let stretch = (geometry.width - margin.left - margin.right).max(0.0);
    // The child shares this root's context, which is prepared for the width
    // its keyword `min-width` and `max-width` leave it.
    let width = crate::layout::dispatch::content_limits::with_width_limits(
        tree,
        node,
        parent_size_of(&geometry),
        stretch,
        |tree| {
            let width =
                own_inline_width(tree, node, &geometry).map_or(stretch, |own| own.at(stretch));
            clamp_block_width(tree, node, geometry.width, width)
        },
    );
    // `auto` side margins share the room the block leaves (CSS 2.1 10.3.3),
    // as in taffy's block algorithm: both auto centre the block, one auto
    // takes all of it.
    let style_margin = tree.physical_style(node).margin();
    let free = (geometry.width - width - margin.left - margin.right).max(0.0);
    match (style_margin.left.is_auto(), style_margin.right.is_auto()) {
        (true, true) => margin.left = free / 2.0,
        (true, false) => margin.left = free,
        (false, false) if geometry.axes.direction() == Direction::Rtl => {
            // CSS 2.2 10.3.3 solves an over-constrained RTL block by changing
            // its used left margin. Apply it before creating the child BFC.
            margin.left = geometry.width - width - margin.right;
        }
        _ => {}
    }
    // The first pass discovers margins that escape the child. Retry against
    // its final normal-flow origin before committing shared float geometry.
    // Clearance (CSS 2.1 9.5.2): a cleared block whose top border edge would
    // sit above the floats it clears is placed below them, and its top
    // margin no longer collapses with the margins above it.
    let clear = tree.physical_style(node).clear();
    // The box after the float that starts a page is held below the floats
    // before that float as well.
    let floor = page_start_float_floor(tree, ctx, node);
    let clear_y = ctx
        .cleared_threshold(clear)
        .into_iter()
        .chain(floor)
        .map(|threshold| threshold - geometry.top_edge)
        .fold(f32::NEG_INFINITY, f32::max);
    let origin_resolved = ctx.origin_resolved();
    let mut guess = if origin_resolved && collapse_top {
        y.max(clear_y)
    } else {
        (y + pending.collapse_with_margin(margin.top).resolve()).max(clear_y)
    };
    // The context measures from the root's border box; the insets are added
    // to the root's own. The right inset follows the used width, as in
    // taffy's own in-flow loop.
    let right = geometry.width - margin.left - width + geometry.edges.1;
    let snapshot = origin_resolved.then(|| ctx.snapshot());
    let (output, floats_bottom, top, after, has_clearance, top_set) = loop {
        let mut child_ctx = ctx.sub_context_for_collapsing_child(
            geometry.top_edge + guess,
            [geometry.edges.0 + margin.left, right],
            geometry.top_edge + y,
            pending.collapse_with_margin(margin.top),
            collapse_top,
        );
        child_ctx.set_origin_resolved(origin_resolved);
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
                height: geometry.definite_height,
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
        let output = tree.compute_child_layout_with_block_ctx(
            NodeId::from(node),
            inputs,
            Some(&mut child_ctx),
        );
        let floats_bottom = child_ctx.floated_content_height_contribution();
        let adjoining = child_ctx.top_adjoining_floats();
        ctx.merge_adjoining_floats(adjoining);
        let top_set = output.top_margin.collapse_with_margin(margin.top);
        let bottom_set = output.bottom_margin.collapse_with_margin(margin.bottom);
        let hypothetical = if origin_resolved && collapse_top {
            ctx.hypothetical_top_position(pending.collapse_with_set(top_set))
                .unwrap_or(y)
        } else {
            y + pending.collapse_with_set(top_set).resolve()
        };
        let has_clearance = (clear != Clear::None || floor.is_some())
            && ((collapse_top && ctx.has_adjoining_float(clear)) || hypothetical < clear_y);
        let (top, after) = if has_clearance {
            {
                let top = clear_y
                    - if collapse_top && !origin_resolved {
                        pending.resolve()
                    } else {
                        0.0
                    };
                (top, (top + output.size.height, bottom_set))
            }
        } else if output.margins_can_collapse_through {
            // An empty block: its margins and the ones above it are one set, and
            // its box sits where that set would put the next box's top.
            let through = pending
                .collapse_with_set(top_set)
                .collapse_with_set(bottom_set);
            (
                if collapse_top {
                    y
                } else {
                    y + pending.collapse_with_set(top_set).resolve()
                },
                (y, through),
            )
        } else {
            let top = if collapse_top {
                y
            } else {
                y + pending.collapse_with_set(top_set).resolve()
            };
            (top, (top + output.size.height, bottom_set))
        };
        if let Some(snapshot) = &snapshot
            && ctx.has_floats()
            && (top - guess).abs() > 0.001
        {
            ctx.restore(snapshot);
            guess = top;
            continue;
        }
        break (output, floats_bottom, top, after, has_clearance, top_set);
    };
    ctx.add_child_floated_content_height_contribution(geometry.top_edge + guess + floats_bottom);
    let mut child_candidates = OofCandidates::new();
    if perform {
        // A relatively positioned block is moved from its place by its
        // offsets, which taffy carries in the layout location.
        let (dx, dy) = relative_inset(
            tree,
            node,
            geometry.content_size.width,
            relative_height(&geometry),
            geometry.axes,
        );
        let location = Point {
            x: geometry.edges.0 + margin.left + dx,
            y: geometry.top_edge + top + dy,
        };
        child_candidates = translated_child_oof_candidates(&output, location);
        commit_child_layout(tree, node, &output, location, geometry.width);
    }
    BlockChild {
        oof_candidates: child_candidates,
        top,
        next_y: after.0,
        pending: after.1,
        floats_bottom: floats_bottom + (guess - top),
        top_margin: (!has_clearance).then(|| {
            if output.margins_can_collapse_through {
                after.1
            } else {
                pending.collapse_with_set(top_set)
            }
        }),
        collapses_through: !has_clearance && output.margins_can_collapse_through,
        first_baseline: output.baselines.first.map(|baseline| top + baseline),
    }
}

/// Where [`place_block_across`] left the content that follows the block.
pub(crate) struct PlacedBlock {
    /// Where the content after the block starts, before `pending`.
    pub(crate) next_y: f32,
    /// Margins below the block that collapse with what follows.
    pub(crate) pending: CollapsibleMarginSet,
    /// The first baseline of the block, from the content-box top of the root.
    pub(crate) first_baseline: Option<f32>,
}

/// Lay out the block box `node` of the root on its own, as a box of a
/// formatting context of its own that spans `geometry.width`, with the content
/// above it ending at `y` and the margins of `pending` below that.
///
/// A column spanner is placed this way: the lines around it were broken at the
/// width of a column, and it spans all of them. It establishes a formatting
/// context, which keeps the margins and the floats of its children inside it
/// (CSS Multi-column Layout 1, 6.1).
pub(crate) fn place_block_across(
    tree: &mut LayoutSession<'_>,
    node: usize,
    y: f32,
    pending: CollapsibleMarginSet,
    geometry: FlowGeometry,
    perform: bool,
) -> PlacedBlock {
    let mut bfc = BlockFormattingContext::new();
    let mut ctx = bfc.root_block_context();
    let inline_insets = geometry.inline_axis_insets();
    ctx.set_width(geometry.width + inline_insets.0 + inline_insets.1);
    let block =
        layout_formatting_context_child(tree, &mut ctx, node, y, pending, geometry, perform, false);
    PlacedBlock {
        next_y: block.next_y,
        pending: block.pending,
        first_baseline: block.first_baseline,
    }
}

/// Lay out an ordinary block child along a vertical root's block axis.
#[allow(clippy::too_many_arguments)]
fn layout_vertical_block_child(
    tree: &mut LayoutSession<'_>,
    ctx: &mut BlockContext<'_>,
    node: usize,
    y: f32,
    pending: CollapsibleMarginSet,
    geometry: FlowGeometry,
    perform: bool,
    collapse_top: bool,
) -> BlockChild {
    let basis = geometry.axes.block_extent(geometry.content_size);
    let inline_basis = geometry.axes.inline_extent(geometry.content_size);
    let mut margin = resolved_margins(tree, node, inline_basis);
    let style_margin = tree.physical_style(node).margin();
    let vertical_rl = matches!(
        geometry.axes.writing_mode(),
        WritingMode::VerticalRl | WritingMode::SidewaysRl
    );
    let (block_start_margin, block_end_margin) = if vertical_rl {
        (margin.right, margin.left)
    } else {
        (margin.left, margin.right)
    };
    let width = used_block_width_with_edge_basis(tree, node, basis, inline_basis)
        .map(|width| clamp_block_width_with_edge_basis(tree, node, basis, inline_basis, width));
    let parent_size = Size {
        width: Some(geometry.content_size.width),
        height: Some(inline_basis),
    };
    let shares_context = shares_vertical_root_context(tree, node, geometry.axes.writing_mode());
    let own_inline_size = used_block_height(tree, node, inline_basis)
        .map(OwnWidth::Fixed)
        .or_else(|| content_keyword_width(tree, node, false, parent_size))
        .or_else(|| orthogonal_content_height(tree, node, width, parent_size, shares_context));
    let clear = tree.physical_style(node).clear();
    let clear_y = ctx
        .cleared_threshold(clear)
        .map_or(f32::NEG_INFINITY, |threshold| threshold - geometry.top_edge);
    let clears_adjoining_float =
        clear != Clear::None && collapse_top && ctx.has_top_adjoining_float(clear);
    let origin_resolved = ctx.origin_resolved();
    let top_margin = pending.collapse_with_margin(block_start_margin);
    let hypothetical = if collapse_top {
        ctx.hypothetical_top_position(top_margin)
            .unwrap_or(geometry.top_edge + y + top_margin.resolve())
    } else {
        geometry.top_edge + y + top_margin.resolve()
    };
    let adjoining_float = collapse_top && ctx.has_top_adjoining_float(Clear::Both);
    let forced_clearance = clears_adjoining_float;
    let min_y = if forced_clearance {
        ctx.cleared_threshold(clear).unwrap_or(hypothetical)
    } else if adjoining_float {
        geometry.top_edge + y
    } else {
        hypothetical
    };
    // An independent vertical context still avoids the parent's floats, while
    // its descendants lay out without receiving that context.
    let mut slot = None;
    let mut slot_separated = false;
    let mut inline_stretch = (inline_basis - margin.top - margin.bottom).max(0.0);
    let height = if !shares_context && (forced_clearance || ctx.has_active_floats(min_y)) {
        let node_id = NodeId::from(node);
        let placed = ctx.find_box_slot(
            min_y,
            [margin.top, margin.bottom],
            if geometry.axes.direction() == Direction::Ltr {
                values::Direction::Ltr
            } else {
                values::Direction::Rtl
            },
            clear,
            |probe| match probe {
                BoxProbe::Width { stretch } => {
                    let stretch = stretch.max(-(margin.top + margin.bottom));
                    clamp_block_height(
                        tree,
                        node,
                        inline_basis,
                        own_inline_size.map_or(stretch, |own| own.at(stretch)),
                    )
                }
                BoxProbe::Height { width: inline_size } => {
                    let inputs = LayoutInput {
                        run_mode: RunMode::ComputeSize,
                        sizing_mode: SizingMode::InherentSize,
                        axis: RequestedAxis::Both,
                        known_dimensions: Size {
                            width,
                            height: Some(inline_size),
                        },
                        known_dimensions_are_definite: Size {
                            width: width.is_some(),
                            height: true,
                        },
                        parent_size,
                        available_space: Size {
                            width: width
                                .map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
                            height: AvailableSpace::Definite(inline_size),
                        },
                        vertical_margins_are_collapsible: Line::FALSE,
                    };
                    tree.compute_child_layout_with_block_ctx(node_id, inputs, None)
                        .size
                        .width
                }
            },
        );
        let separation_origin = if adjoining_float {
            geometry.top_edge + y
        } else {
            hypothetical
        };
        slot_separated = forced_clearance || placed.slot.y > separation_origin + 0.001;
        inline_stretch = placed.slot.stretch_width;
        slot = Some(placed.slot);
        placed.width
    } else {
        own_inline_size.map_or(inline_stretch, |own| own.at(inline_stretch))
    };
    let height = clamp_block_height(tree, node, inline_basis, height);
    let inline_free = inline_stretch - height;
    let auto_margins =
        u8::from(style_margin.top.is_auto()) + u8::from(style_margin.bottom.is_auto());
    let auto_top = if style_margin.top.is_auto() {
        inline_free.max(0.0) / f32::from(auto_margins.max(1))
    } else {
        0.0
    };
    let auto_bottom = if style_margin.bottom.is_auto() {
        inline_free.max(0.0) / f32::from(auto_margins.max(1))
    } else {
        0.0
    };
    if slot.is_none() {
        if style_margin.top.is_auto() {
            margin.top = auto_top;
        }
        if style_margin.bottom.is_auto() {
            margin.bottom = auto_bottom;
        }
    }
    let inline_insets = geometry.inline_axis_insets();
    let inline_start = slot.map_or_else(
        || match geometry.axes.direction() {
            Direction::Ltr => margin.top,
            Direction::Rtl => margin.bottom,
        },
        |slot| {
            let slot_start = slot.x - inline_insets.0;
            match geometry.axes.direction() {
                Direction::Ltr => slot_start + auto_top,
                // RTL inline-start is the slot's far edge; its auto margin moves inward.
                Direction::Rtl => inline_basis - slot_start - slot.border_width + auto_bottom,
            }
        },
    );
    let mut guess = if origin_resolved && collapse_top {
        y.max(clear_y)
    } else {
        (y + pending.collapse_with_margin(block_start_margin).resolve()).max(clear_y)
    };
    let snapshot = (shares_context && origin_resolved).then(|| ctx.snapshot());
    let (
        output,
        floats_bottom,
        block_start,
        after,
        has_clearance,
        top_set,
        margins_can_collapse_through,
    ) = loop {
        let (output, floats_bottom, adjoining) = {
            let mut child_ctx = shares_context.then(|| {
                let mut child_ctx = ctx.sub_context_for_collapsing_child(
                    geometry.top_edge + guess,
                    [
                        inline_insets.0 + margin.top,
                        inline_basis - margin.top - height + inline_insets.1,
                    ],
                    geometry.top_edge + y,
                    pending.collapse_with_margin(block_start_margin),
                    collapse_top,
                );
                child_ctx.set_origin_resolved(origin_resolved);
                child_ctx
            });
            let inputs = LayoutInput {
                run_mode: if perform {
                    RunMode::PerformLayout
                } else {
                    RunMode::ComputeSize
                },
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Both,
                known_dimensions: Size {
                    width,
                    height: Some(height),
                },
                known_dimensions_are_definite: Size {
                    width: width.is_some(),
                    height: true,
                },
                parent_size,
                available_space: Size {
                    width: width.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
                    height: AvailableSpace::Definite(height),
                },
                vertical_margins_are_collapsible: if shares_context {
                    Line::TRUE
                } else {
                    Line::FALSE
                },
            };
            let node_id = NodeId::from(node);
            let output = if shares_context {
                tree.context.keep_text_group =
                    !matches!(tree.context.text_group, TextGroupScope::None);
                tree.compute_child_layout_with_block_ctx(node_id, inputs, child_ctx.as_mut())
            } else {
                tree.compute_child_layout(node_id, inputs)
            };
            let floats_bottom = child_ctx.as_ref().map_or(
                f32::NEG_INFINITY,
                BlockContext::floated_content_height_contribution,
            );
            let adjoining = child_ctx
                .as_ref()
                .map_or([false; 2], BlockContext::top_adjoining_floats);
            (output, floats_bottom, adjoining)
        };
        // Taffy's collapse-through flag follows physical height; a vertical
        // block's extent is its physical width, which must also be empty.
        let margins_can_collapse_through =
            output.margins_can_collapse_through && output.size.width == 0.0;
        ctx.merge_adjoining_floats(adjoining);
        let top_set = output.top_margin.collapse_with_margin(block_start_margin);
        let bottom_set = output.bottom_margin.collapse_with_margin(block_end_margin);
        let hypothetical = if origin_resolved && collapse_top {
            ctx.hypothetical_top_position(pending.collapse_with_set(top_set))
                .unwrap_or(y)
        } else {
            y + pending.collapse_with_set(top_set).resolve()
        };
        let has_clearance = slot_separated
            || clear != Clear::None && (clears_adjoining_float || hypothetical < clear_y);
        let (block_start, after) = if let Some(slot) = slot {
            let block_start = if collapse_top && !slot_separated {
                y
            } else {
                slot.y - geometry.top_edge
            };
            (block_start, (block_start + output.size.width, bottom_set))
        } else if has_clearance {
            let block_start = clear_y
                - if collapse_top && !origin_resolved {
                    pending.resolve()
                } else {
                    0.0
                };
            (block_start, (block_start + output.size.width, bottom_set))
        } else if margins_can_collapse_through {
            let through = pending
                .collapse_with_set(top_set)
                .collapse_with_set(bottom_set);
            (
                if collapse_top {
                    y
                } else {
                    y + pending.collapse_with_set(top_set).resolve()
                },
                (y, through),
            )
        } else {
            let block_start = if collapse_top {
                y
            } else {
                y + pending.collapse_with_set(top_set).resolve()
            };
            (block_start, (block_start + output.size.width, bottom_set))
        };
        if let Some(snapshot) = &snapshot
            && ctx.has_floats()
            && (block_start - guess).abs() > 0.001
        {
            ctx.restore(snapshot);
            guess = block_start;
            continue;
        }
        break (
            output,
            floats_bottom,
            block_start,
            after,
            has_clearance,
            top_set,
            margins_can_collapse_through,
        );
    };
    ctx.add_child_floated_content_height_contribution(geometry.top_edge + guess + floats_bottom);
    let physical = geometry.axes.rect(
        geometry.content_size,
        shodo::geometry::LogicalRect {
            inline_start,
            block_start,
            inline_size: output.size.height,
            block_size: output.size.width,
        },
    );
    let mut child_candidates = OofCandidates::new();
    if perform {
        let (dx, dy) = relative_inset(tree, node, basis, relative_height(&geometry), geometry.axes);
        let location = Point {
            x: geometry.insets.left + physical.x + dx,
            y: geometry.insets.top + physical.y + dy,
        };
        child_candidates = translated_child_oof_candidates(&output, location);
        commit_child_layout(tree, node, &output, location, inline_basis);
        // The baseline the block exported, where normal flow placed it: the
        // root's own baseline may come from it. A block whose block axis runs
        // the other way measures its baseline from the opposite edge.
        tree.nodes[node].vertical_first_baseline = output
            .baselines
            .first
            .filter(|_| {
                crate::layout::dispatch::block_right_to_left(tree, node) == Some(vertical_rl)
            })
            .map(|baseline| block_start + baseline);
    }
    let collapses_through = !has_clearance && margins_can_collapse_through;
    BlockChild {
        oof_candidates: child_candidates,
        top: block_start,
        next_y: after.0,
        pending: after.1,
        floats_bottom: floats_bottom + (guess - block_start),
        top_margin: (!has_clearance).then(|| {
            if margins_can_collapse_through {
                after.1
            } else {
                pending.collapse_with_set(top_set)
            }
        }),
        collapses_through,
        first_baseline: None,
    }
}

/// Whether the block child `node` takes part in the root's block formatting
/// context, as taffy's block algorithm decides for an in-flow item: a block
/// box that is not a scroll container. Flow roots, flex and grid containers
/// and scroll containers establish formatting contexts of their own.
fn in_the_roots_formatting_context(tree: &LayoutSession<'_>, node: usize) -> bool {
    let style = tree.query_style(node, ReadRole::BlockContainer);
    let overflow = style.overflow();
    tree.nodes[node].layout_display() == Display::Block
        && !crate::layout::dispatch::is_table_box(tree.nodes[node].display())
        && !overflow.x.is_scroll_container()
        && !overflow.y.is_scroll_container()
        && !style.contain().establishes_independent_formatting_context()
}

/// Whether the block child `node` of a vertical root laid out in
/// `root_writing_mode` takes part in the root's block formatting context: it
/// does when it has the root's writing mode and does not align its content. A
/// child with another writing mode, even one that flows in the same block
/// direction as `sideways-lr` does in a `vertical-lr` root (CSS Writing Modes
/// 4 §3.1), or that flows the other way, across or against the root, is laid
/// out as the root of a context of its own, whose margins do not collapse with
/// its content's and which does not overlap the root's floats.
fn shares_vertical_root_context(
    tree: &LayoutSession<'_>,
    node: usize,
    root_writing_mode: WritingMode,
) -> bool {
    in_the_roots_formatting_context(tree, node)
        && super::style::shodo_writing_mode(
            crate::layout::dispatch::used_writing_mode(tree, node),
            node,
        )
        .is_ok_and(|writing_mode| writing_mode == root_writing_mode)
        && tree.physical_style(node).align_content().keyword == AlignContentKeyword::Normal
        && crate::layout::dispatch::block_right_to_left(tree, node).is_some_and(|node_direction| {
            node_direction
                == matches!(
                    root_writing_mode,
                    WritingMode::VerticalRl | WritingMode::SidewaysRl
                )
        })
}

/// Lay out a block child that establishes a formatting context of its own
/// the way taffy's block algorithm does (CSS 2.1 9.4.1, 9.5): its top margin
/// collapses with the margins above it but not with its children's, and its
/// border box does not overlap the floats beside it: it is placed beside
/// them where it fits, narrowed to the room they leave, or below them.
#[allow(clippy::too_many_arguments)]
fn layout_formatting_context_child(
    tree: &mut LayoutSession<'_>,
    ctx: &mut BlockContext<'_>,
    node: usize,
    y: f32,
    pending: CollapsibleMarginSet,
    geometry: FlowGeometry,
    perform: bool,
    collapse_top: bool,
) -> BlockChild {
    let margin = resolved_margins(tree, node, geometry.width);
    let style_margin = tree.physical_style(node).margin();
    let clear = tree.physical_style(node).clear();
    let x_margins = margin.left + margin.right;
    let top_margin = pending.collapse_with_margin(margin.top);
    let hypothetical = if collapse_top {
        ctx.hypothetical_top_position(top_margin)
            .unwrap_or(geometry.top_edge + y + top_margin.resolve())
    } else {
        geometry.top_edge + y + top_margin.resolve()
    };
    let adjoining_float = collapse_top && ctx.has_adjoining_float(Clear::Both);
    // The box after the float that starts a page is held below the floats
    // before that float as well.
    let floor = page_start_float_floor(tree, ctx, node);
    let forced_clearance = collapse_top
        && (ctx.has_adjoining_float(clear)
            || floor.is_some_and(|floor| floor > hypothetical + 0.001));
    let min_y = if forced_clearance {
        ctx.cleared_threshold(clear)
            .into_iter()
            .chain(floor)
            .reduce(f32::max)
            .unwrap_or(hypothetical)
    } else if adjoining_float {
        // An adjoining float moves with the escaping margin strut.
        geometry.top_edge + y
    } else {
        hypothetical
    };
    let min_y = floor.map_or(min_y, |floor| min_y.max(floor));
    // `(stretch width, border-box position, whether floats narrowed it)`.

    let own = own_inline_width(tree, node, &geometry);
    let (stretch, position, beside_floats) = if ctx.has_active_floats(min_y) {
        let slot = ctx
            .find_box_slot(
                min_y,
                [margin.left, margin.right],
                values::Direction::Ltr,
                clear,
                |probe| match probe {
                    BoxProbe::Width { stretch } => {
                        let stretch = stretch.max(-x_margins);
                        clamp_block_width(
                            tree,
                            node,
                            geometry.width,
                            own.map_or(stretch, |own| own.at(stretch)),
                        )
                    }
                    BoxProbe::Height { width } => {
                        let inputs = formatting_context_inputs(
                            &geometry,
                            RunMode::ComputeSize,
                            width,
                            width,
                        );
                        tree.compute_child_layout(NodeId::from(node), inputs)
                            .size
                            .height
                    }
                },
            )
            .slot;
        (
            slot.stretch_width.max(-x_margins),
            Point {
                x: slot.x,
                y: slot.y,
            },
            true,
        )
    } else {
        (
            geometry.width - x_margins,
            Point {
                x: geometry.edges.0,
                y: min_y,
            },
            false,
        )
    };
    let width = clamp_block_width(
        tree,
        node,
        geometry.width,
        own.map_or(stretch, |own| own.at(stretch)),
    );
    let inputs = formatting_context_inputs(
        &geometry,
        if perform {
            RunMode::PerformLayout
        } else {
            RunMode::ComputeSize
        },
        width,
        stretch,
    );
    let output = tree.compute_child_layout(NodeId::from(node), inputs);
    // `auto` side margins share the room the box leaves; beside floats the
    // slot already holds the non-`auto` margins.
    let free = (stretch - output.size.width).max(0.0);
    let autos = u8::from(style_margin.left.is_auto()) + u8::from(style_margin.right.is_auto());
    let auto_left = if style_margin.left.is_auto() {
        free / f32::from(autos.max(1))
    } else {
        0.0
    };
    let extra_left = if beside_floats {
        auto_left
    } else {
        margin.left + auto_left
    };
    let separation_origin = if adjoining_float {
        geometry.top_edge + y
    } else {
        hypothetical
    };
    let separated = forced_clearance || position.y > separation_origin;
    let top = if collapse_top && !separated {
        y
    } else {
        position.y - geometry.top_edge
    };
    let mut child_candidates = OofCandidates::new();
    if perform {
        let (dx, dy) = relative_inset(
            tree,
            node,
            geometry.content_size.width,
            relative_height(&geometry),
            geometry.axes,
        );
        let location = Point {
            x: position.x + extra_left + dx,
            y: geometry.top_edge + top + dy,
        };
        child_candidates = translated_child_oof_candidates(&output, location);
        commit_child_layout(tree, node, &output, location, geometry.width);
    }
    BlockChild {
        oof_candidates: child_candidates,
        top,
        next_y: top + output.size.height,
        pending: CollapsibleMarginSet::from_margin(margin.bottom),
        // The box contains its own floats.
        floats_bottom: f32::NEG_INFINITY,
        // A box that establishes a formatting context keeps its children's
        // margins, but its own still collapses with the ones above it.
        // Specifying clear introduces no clearance without relevant floats.
        top_margin: (!separated).then_some(top_margin),
        collapses_through: false,
        first_baseline: output.baselines.first.map(|baseline| top + baseline),
    }
}

/// The input that lays out a block child that establishes a formatting
/// context `width` wide, in a slot that leaves `stretch` for an `auto` width.
fn formatting_context_inputs(
    geometry: &FlowGeometry,
    run_mode: RunMode,
    width: f32,
    stretch: f32,
) -> LayoutInput {
    LayoutInput {
        run_mode,
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
            height: geometry.definite_height,
        },
        available_space: Size {
            width: AvailableSpace::Definite(stretch),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

/// The size of the root's content box that a horizontal block child's
/// percentages resolve against.
fn parent_size_of(geometry: &FlowGeometry) -> Size<Option<f32>> {
    Size {
        width: Some(geometry.width),
        height: geometry.definite_height,
    }
}

/// The border-box height of the horizontal block child `node` of a vertical
/// root, when its height is not a length: the extent of its content at the
/// `width` it is laid out at. The child is an orthogonal flow, so its height
/// is its block size, not the inline size of the root that a child of the
/// root's own writing mode fills (CSS Writing Modes 4 §7.3). `None` for a
/// child that flows vertically.
fn orthogonal_content_height(
    tree: &mut LayoutSession<'_>,
    node: usize,
    width: Option<f32>,
    parent_size: Size<Option<f32>>,
    shares_context: bool,
) -> Option<OwnWidth> {
    if crate::layout::dispatch::used_writing_mode(tree, node).is_vertical() {
        return None;
    }
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width,
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: width.is_some(),
            height: false,
        },
        parent_size,
        available_space: Size {
            width: width.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line {
            start: shares_context,
            end: shares_context,
        },
    };
    let output = tree.compute_child_layout_with_block_ctx(NodeId::from(node), inputs, None);
    Some(OwnWidth::Fixed(output.size.height))
}

/// The border-box width of the float or atomic inline `node`, a paragraph root
/// or a flex or grid container in a horizontal paragraph, when its `width` is
/// `min-content` or `max-content`: the size the keyword measures, within the
/// `min-width` and `max-width` of the box. `None` for any other `width`, which
/// the box takes from the `available` room by shrinking to fit it (CSS 2.1
/// §10.3.5).
///
/// Such a box reads its `width` as `auto`, so the width is measured here and
/// handed to it as a known size.
fn keyword_width(
    tree: &mut LayoutSession<'_>,
    node: usize,
    geometry: &FlowGeometry,
    available: f32,
) -> Option<f32> {
    let root = &tree.nodes[node];
    if !(root.is_ifc_root() || root.display().is_flex_or_grid_container()) {
        return None;
    }
    let width = tree.physical_style(node).size().width.expand();
    if !matches!(
        width,
        ExpandedDimension::MinContent | ExpandedDimension::MaxContent
    ) {
        return None;
    }
    let own = content_keyword_width(tree, node, true, parent_size_of(geometry))?;
    Some(clamp_block_width(
        tree,
        node,
        geometry.width,
        own.at(available),
    ))
}

/// The border-box width a block child's own size gives along one axis, before
/// the room the root leaves it is known.
#[derive(Clone, Copy)]
enum OwnWidth {
    /// A length or percentage, or the `min-content` or `max-content` size.
    Fixed(f32),
    /// `fit-content`: the room the root leaves the child, but at least its
    /// `min-content` and at most its `max-content` size (CSS Sizing 3 §3.1.1).
    Fit { min: f32, max: f32 },
}

impl OwnWidth {
    /// The width in `stretch`, the room the root leaves the child.
    fn at(self, stretch: f32) -> f32 {
        match self {
            Self::Fixed(width) => width,
            Self::Fit { min, max } => max.min(min.max(stretch)),
        }
    }
}

/// The border-box width a horizontal block child's own `width` gives: a length
/// or percentage, else the size its sizing keyword measures (see
/// [`content_keyword_width`]); `None` for `auto`.
fn own_inline_width(
    tree: &mut LayoutSession<'_>,
    node: usize,
    geometry: &FlowGeometry,
) -> Option<OwnWidth> {
    used_block_width(tree, node, geometry.width)
        .map(OwnWidth::Fixed)
        .or_else(|| content_keyword_width(tree, node, true, parent_size_of(geometry)))
}

/// The border-box size of the block child `node` along the physical axis its
/// `min-content`, `max-content` or `fit-content` keyword sizes, measured the
/// way the parent's block algorithm measures such a child (CSS Sizing 3
/// §5.1); `None` when the size there is not such a keyword. `horizontal`
/// names the width; otherwise it is the height, the inline size of a vertical
/// root's children.
///
/// `fit-content` is built from the `min-content` and `max-content` measures,
/// which do not depend on the room the root leaves the child. A float beside
/// the child changes that room, so the search for a place beside floats
/// measures the child once.
fn content_keyword_width(
    tree: &mut LayoutSession<'_>,
    node: usize,
    horizontal: bool,
    parent_size: Size<Option<f32>>,
) -> Option<OwnWidth> {
    let size = tree.physical_style(node).size();
    let keyword = if horizontal { size.width } else { size.height };
    // A percentage against an indefinite size leaves no room, which sizes the
    // child like `auto`.
    let basis = if horizontal {
        parent_size.width
    } else {
        parent_size.height
    };
    let room = fit_content_room(tree, node, keyword.expand(), basis, horizontal);
    let mut measure = |along: AvailableSpace| {
        let (width, height) = if horizontal {
            (along, AvailableSpace::MaxContent)
        } else {
            (AvailableSpace::MaxContent, along)
        };
        let inputs = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size::NONE,
            known_dimensions_are_definite: Size {
                width: false,
                height: false,
            },
            parent_size,
            available_space: Size { width, height },
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let output = tree.compute_child_layout_with_block_ctx(NodeId::from(node), inputs, None);
        if horizontal {
            output.size.width
        } else {
            output.size.height
        }
    };
    match keyword.expand() {
        ExpandedDimension::MinContent => Some(OwnWidth::Fixed(measure(AvailableSpace::MinContent))),
        ExpandedDimension::MaxContent => Some(OwnWidth::Fixed(measure(AvailableSpace::MaxContent))),
        ExpandedDimension::FitContent => Some(OwnWidth::Fit {
            min: measure(AvailableSpace::MinContent),
            max: measure(AvailableSpace::MaxContent),
        }),
        // The limit stands in for the room the root leaves the child.
        ExpandedDimension::FitContentPx(_)
        | ExpandedDimension::FitContentPercent(_)
        | ExpandedDimension::FitContentCalc(_) => room.map(|room| {
            OwnWidth::Fixed(
                OwnWidth::Fit {
                    min: measure(AvailableSpace::MinContent),
                    max: measure(AvailableSpace::MaxContent),
                }
                .at(room),
            )
        }),
        _ => None,
    }
}

/// The border-box width a block child's own `width` gives, resolved against
/// the root's content width the way taffy's block algorithm resolves the size
/// of an in-flow item; `None` for `auto`.
fn used_block_width(tree: &LayoutSession<'_>, node: usize, basis: f32) -> Option<f32> {
    used_block_width_with_edge_basis(tree, node, basis, basis)
}

fn used_block_width_with_edge_basis(
    tree: &LayoutSession<'_>,
    node: usize,
    dimension_basis: f32,
    edge_basis: f32,
) -> Option<f32> {
    tree.physical_style(node)
        .size()
        .width
        .maybe_resolve(Some(dimension_basis))
        .map(|width| width + box_sizing_adjustment(tree, node, edge_basis))
}

/// `width` clamped by the block child's own `min-width` and `max-width`,
/// resolved like [`used_block_width`].
fn clamp_block_width(tree: &LayoutSession<'_>, node: usize, basis: f32, width: f32) -> f32 {
    clamp_block_width_with_edge_basis(tree, node, basis, basis, width)
}

fn clamp_block_width_with_edge_basis(
    tree: &LayoutSession<'_>,
    node: usize,
    dimension_basis: f32,
    edge_basis: f32,
    width: f32,
) -> f32 {
    let style = tree.physical_style(node);
    let adjustment = box_sizing_adjustment(tree, node, edge_basis);
    let min = style
        .min_size()
        .width
        .maybe_resolve(Some(dimension_basis))
        .map(|min| min + adjustment);
    let max = style
        .max_size()
        .width
        .maybe_resolve(Some(dimension_basis))
        .map(|max| max + adjustment);
    let width = max.map_or(width, |max| width.min(max));
    min.map_or(width, |min| width.max(min))
}

fn used_block_height(tree: &LayoutSession<'_>, node: usize, basis: f32) -> Option<f32> {
    tree.physical_style(node)
        .size()
        .height
        .maybe_resolve(Some(basis))
        .map(|height| height + box_sizing_height_adjustment(tree, node, basis))
}

fn clamp_block_height(tree: &LayoutSession<'_>, node: usize, basis: f32, height: f32) -> f32 {
    let style = tree.physical_style(node);
    let adjustment = box_sizing_height_adjustment(tree, node, basis);
    let min = style
        .min_size()
        .height
        .maybe_resolve(Some(basis))
        .map(|min| min + adjustment);
    let max = style
        .max_size()
        .height
        .maybe_resolve(Some(basis))
        .map(|max| max + adjustment);
    let height = max.map_or(height, |max| height.min(max));
    min.map_or(height, |min| height.max(min))
}

/// What a `content-box` size needs added to become a border-box size.
pub(crate) fn box_sizing_adjustment(tree: &LayoutSession<'_>, node: usize, basis: f32) -> f32 {
    let style = tree.physical_style(node);
    if style.box_sizing() != BoxSizing::ContentBox {
        return 0.0;
    }
    let padding = style.padding().resolve_or_zero(Some(basis));
    let border = style.border().resolve_or_zero(Some(basis));
    padding.left + padding.right + border.left + border.right
}

pub(crate) fn box_sizing_height_adjustment(
    tree: &LayoutSession<'_>,
    node: usize,
    basis: f32,
) -> f32 {
    let style = tree.physical_style(node);
    if style.box_sizing() != BoxSizing::ContentBox {
        return 0.0;
    }
    let padding = style.padding().resolve_or_zero(Some(basis));
    let border = style.border().resolve_or_zero(Some(basis));
    padding.top + padding.bottom + border.top + border.bottom
}

/// Store the final layout of every atomic inline at its fragment in the
/// accepted lines.
/// Truncate the performed lines for `text-overflow: ellipsis` before the
/// boxes on them are placed, so placement sees what remains on each line.
fn truncate_for_text_overflow(
    tree: &mut LayoutSession<'_>,
    root: &IfcRoot,
    lines: &mut std::sync::Arc<Vec<shodo::Line>>,
    width: f32,
) {
    if !root.ellipsis {
        return;
    }
    with_state(tree, |state| {
        super::flow::truncate_lines(root, lines, &mut state.layout_cx, width);
    });
}

/// Mark the atomic inlines an ellipsis removed from their lines, and clear
/// the mark on those still on a line, so paint skips only hidden ones.
fn hide_truncated_atomics(
    tree: &mut LayoutSession<'_>,
    lines: &[shodo::Line],
    outputs: &[(usize, LayoutOutput)],
) {
    if outputs.is_empty() {
        return;
    }
    let mut on_a_line = HashSet::with_capacity(outputs.len());
    for_each_atomic(lines, |atomic, _, _| {
        #[cfg(test)]
        crate::layout::test_support::IFC_ATOMIC_VISIBILITY_STEPS
            .with(|steps| steps.set(steps.get() + 1));
        on_a_line.insert(atomic.node.0 as usize);
    });
    for &(node, _) in outputs {
        #[cfg(test)]
        crate::layout::test_support::IFC_ATOMIC_VISIBILITY_STEPS
            .with(|steps| steps.set(steps.get() + 1));
        tree.nodes[node].flags.set(
            crate::node::NodeFlags::HIDDEN_BY_TEXT_OVERFLOW,
            !on_a_line.contains(&node),
        );
    }
}

fn place_atomics(
    tree: &mut LayoutSession<'_>,
    lines: &[shodo::Line],
    outputs: &[(usize, LayoutOutput)],
    geometry: FlowGeometry,
    rtl: bool,
) -> OofCandidates {
    let mut candidates = OofCandidates::new();
    if outputs.is_empty() {
        return candidates;
    }
    let mut outputs_by_node = HashMap::with_capacity(outputs.len());
    for (node, output) in outputs {
        #[cfg(test)]
        crate::layout::test_support::IFC_ATOMIC_OUTPUT_LOOKUP_STEPS
            .with(|steps| steps.set(steps.get() + 1));
        outputs_by_node.entry(*node).or_insert(output);
    }
    for_each_atomic(lines, |atomic, line_top, placement| {
        let node = atomic.node.0 as usize;
        #[cfg(test)]
        crate::layout::test_support::IFC_ATOMIC_OUTPUT_LOOKUP_STEPS
            .with(|steps| steps.set(steps.get() + 1));
        if let Some(output) = outputs_by_node.get(&node).copied() {
            // The fragment is relative to the line's top and the content-box
            // start; the layout is relative to the root's border box. A
            // relatively positioned atomic is drawn offset from that place,
            // which taffy's layout location carries for inline-level boxes.
            let (dx, dy) = relative_inset(
                tree,
                node,
                geometry.content_size.width,
                relative_height(&geometry),
                geometry.axes,
            );
            // An atomic in a ruby annotation is placed with its annotation.
            let rect = placed(atomic.border_rect, line_top, placement);
            // The rectangle is logical: its inline axis runs along the line
            // and its block axis across the lines, which a vertical paragraph
            // draws down and from the right (or left) edge. A right-to-left
            // line runs from the far edge, so its inline axis is mirrored
            // inside the content box.
            let mut size = geometry.content_size;
            if geometry.axes.writing_mode() == WritingMode::HorizontalTb {
                size.width = geometry.width;
            }
            let direction = if rtl { Direction::Rtl } else { Direction::Ltr };
            let physical = IfcAxes::new(geometry.axes.writing_mode(), direction).rect(size, rect);
            let location = Point {
                x: geometry.edges.0 + physical.x + dx,
                y: geometry.insets.top + physical.y + dy,
            };
            let mut child_candidates = translated_child_oof_candidates(output, location);
            candidates.append(&mut child_candidates);
            commit_child_layout(tree, node, output, location, geometry.width);
        }
    });
    candidates
}

/// Move the atomic inlines of lines that were split into columns to the
/// column of their line. [`place_atomics`] puts every atomic where the lines
/// stand in one tall column; each range of `fragments` starts its first line
/// at the top of its column, at the column's inline offset, the way the text
/// lines of the range are drawn. Only the `boxes` of the root are moved: the
/// other atomics of its lines are drawn with the lines.
pub(crate) fn move_atomics_to_columns(
    tree: &mut LayoutSession<'_>,
    boxes: &[IfcBox],
    lines: &[shodo::Line],
    fragments: &[crate::node::MulticolTextFragment],
) {
    let box_nodes: HashSet<usize> = boxes.iter().map(|box_| box_.node).collect();
    for fragment in fragments {
        let dx = fragment.x;
        let dy = fragment.y - lines[fragment.line_start].block_offset();
        // The atomics in a ruby annotation go with the line of their ruby.
        for_each_atomic(
            &lines[fragment.line_start..fragment.line_end],
            |atomic, _, _| {
                let node = atomic.node.0 as usize;
                if box_nodes.contains(&node) {
                    let mut layout = tree.nodes[node].layout;
                    layout.location.x += dx;
                    layout.location.y += dy;
                    tree.set_unrounded_layout(NodeId::from(node), &layout);
                }
            },
        );
    }
}

/// The atomic inlines of the root's `boxes` with the range of `fragments`
/// that holds the line each one sits on, in the order of the ranges. An atomic
/// in a ruby annotation sits on the line of its ruby.
pub(crate) fn atomic_fragments(
    boxes: &[IfcBox],
    lines: &[shodo::Line],
    fragments: &[crate::node::MulticolTextFragment],
) -> Vec<(usize, crate::node::MulticolTextFragment)> {
    let box_nodes: HashSet<usize> = boxes.iter().map(|box_| box_.node).collect();
    let mut placed = Vec::new();
    for fragment in fragments {
        let end = fragment.line_end.min(lines.len());
        let start = fragment.line_start.min(end);
        for_each_atomic(&lines[start..end], |atomic, _, _| {
            let node = atomic.node.0 as usize;
            if box_nodes.contains(&node) {
                placed.push((node, *fragment));
            }
        });
    }
    placed
}

/// The root's content height as the basis of a percentage `top` or
/// `bottom`, when it is definite.
fn relative_height(geometry: &FlowGeometry) -> Option<f32> {
    geometry.definite_height
}

/// The offset of a relatively positioned box (CSS 2.1 9.4.3): of opposing
/// insets the one on the start side of the root's `axes` wins, percentages
/// are of the root's content width and of its `height` when that is definite
/// (otherwise a vertical percentage computes to `auto`). `(0, 0)` for a box
/// in normal position.
fn relative_inset(
    tree: &LayoutSession<'_>,
    node: usize,
    basis: f32,
    height: Option<f32>,
    axes: IfcAxes,
) -> (f32, f32) {
    let inset = tree.physical_style(node).inset();
    if !matches!(
        tree.nodes[node].layout_position(),
        Position::Relative | Position::Sticky
    ) || inset.left.is_auto()
        && inset.right.is_auto()
        && inset.top.is_auto()
        && inset.bottom.is_auto()
    {
        return (0.0, 0.0);
    }
    let horizontal = |value: LengthPercentageAuto| value.maybe_resolve(Some(basis));
    let vertical = |value: LengthPercentageAuto| value.maybe_resolve(height);
    let (right_wins, bottom_wins) =
        super::style::inset_precedence(axes.writing_mode(), axes.direction() == Direction::Rtl);
    let left = horizontal(inset.left);
    let right = horizontal(inset.right).map(|right| -right);
    let dx = if right_wins {
        right.or(left)
    } else {
        left.or(right)
    }
    .unwrap_or(0.0);
    let top = vertical(inset.top);
    let bottom = vertical(inset.bottom).map(|bottom| -bottom);
    let dy = if bottom_wins {
        bottom.or(top)
    } else {
        top.or(bottom)
    }
    .unwrap_or(0.0);
    (dx, dy)
}

/// The baseline of an atomic inline from its border-box top, if it has one.
/// An inline-block that is a scroll container has none: it sits on its
/// bottom margin edge (CSS 2.1 10.8.1). `overflow: clip` does not make a
/// scroll container (CSS Overflow 3 §3.1).
fn atomic_baseline(tree: &LayoutSession<'_>, node: usize, output: &LayoutOutput) -> Option<f32> {
    let overflow = tree.physical_style(node).overflow();
    let scrolls = |overflow| !matches!(overflow, Overflow::Visible | Overflow::Clip);
    if scrolls(overflow.x) || scrolls(overflow.y) {
        return None;
    }
    output.baselines.first
}

/// The distance from the line-over edge of an atomic inline's margin box to
/// its first baseline, for a paragraph in `paragraph` mode. `from_start` is
/// the baseline's distance from the border box's block-start edge in the
/// atomic's own `item` mode, and `width` its physical border-box width.
///
/// Line-over is the physical right edge in vertical-rl and vertical-lr and
/// the left edge in sideways-lr, whichever way the block axis runs. An
/// atomic whose lines are not oriented and stacked like the paragraph's, such
/// as an orthogonal one or one whose block axis runs the other way, has no
/// baseline to share and sits on its margin box's line-under edge.
fn baseline_from_line_over(
    paragraph: WritingMode,
    item: raikiri_style::property::WritingMode,
    width: f32,
    margin: Edges<f32>,
    from_start: f32,
) -> Option<f32> {
    use raikiri_style::property::WritingMode as Item;
    match (paragraph, item) {
        (WritingMode::HorizontalTb, _) => Some(margin.top + from_start),
        (
            WritingMode::VerticalRl | WritingMode::SidewaysRl,
            Item::VerticalRl | Item::SidewaysRl,
        ) => Some(margin.right + from_start),
        (WritingMode::VerticalLr, Item::VerticalLr) => Some(margin.right + width - from_start),
        (WritingMode::SidewaysLr, Item::SidewaysLr) => Some(margin.left + from_start),
        _ => None,
    }
}

/// Place a float in the context at the line offset `y` and, when performing
/// layout, store its position. Returns the top of the placed margin box from
/// the content-box top.
pub(crate) fn commit_float(
    tree: &mut LayoutSession<'_>,
    ctx: &mut BlockContext<'_>,
    float: TentativeFloat,
    y: f32,
    geometry: FlowGeometry,
    perform: bool,
    adjoins_top_margin: bool,
) -> (f32, OofCandidates) {
    let mut oof_candidates = OofCandidates::new();
    if tree
        .forced_break_clearance
        .binary_search(&float.node)
        .is_ok()
    {
        ctx.record_page_start_float_floor(float.clear);
    }
    // The context measures block offsets from the border-box top.
    let position = ctx.place_floated_box(
        float.margin_box,
        geometry.top_edge + y,
        float.direction,
        float.clear,
        adjoins_top_margin,
    );
    if perform {
        let (x, y) = if geometry.axes.writing_mode() == WritingMode::HorizontalTb {
            (position.x, position.y)
        } else {
            // FloatContext reports physical coordinates from the top of the
            // inline axis; the converter expects an offset from inline-start.
            let inline_start = if geometry.axes.direction() == Direction::Rtl {
                geometry.axes.inline_extent(geometry.border_box_size())
                    - position.x
                    - float.margin_box.width
            } else {
                position.x
            };
            let physical = geometry.axes.rect(
                geometry.border_box_size(),
                shodo::geometry::LogicalRect {
                    inline_start,
                    block_start: position.y,
                    inline_size: float.margin_box.width,
                    block_size: float.margin_box.height,
                },
            );
            (physical.x, physical.y)
        };
        let location = Point {
            x: x + float.margin.left,
            y: y + float.margin.top,
        };
        let mut child_candidates = translated_child_oof_candidates(&float.output, location);
        oof_candidates.append(&mut child_candidates);
        commit_child_layout(tree, float.node, &float.output, location, geometry.width);
    }
    (position.y - geometry.top_edge, oof_candidates)
}

/// Store the final layout of a child that the ifc root laid out itself.
///
/// `location` is the border-box position relative to the ifc root's border
/// box. `output` is what the child's layout returned. `basis` is the root's
/// content inline size, used to resolve the child's percentage margins,
/// padding, and border.
pub(crate) fn commit_child_layout(
    tree: &mut LayoutSession<'_>,
    node: usize,
    output: &LayoutOutput,
    location: Point<f32>,
    basis: f32,
) {
    let edges = session_edges(tree, node, basis);
    commit_child_layout_with_edges(tree, node, output, location, edges);
}

/// The border, padding and margin of a box, resolved against one basis.
#[derive(Clone, Copy)]
pub(crate) struct BoxEdges {
    border: Edges<f32>,
    padding: Edges<f32>,
    margin: Edges<f32>,
}

impl BoxEdges {
    /// A box with no edges.
    pub(crate) const ZERO: Self = Self {
        border: Edges::ZERO,
        padding: Edges::ZERO,
        margin: Edges::ZERO,
    };

    /// The edges `style` states, with percentages of `basis` and `auto`
    /// margins as zero.
    pub(crate) fn of(style: &raikiri_layout_style::LayoutStyleView<'_>, basis: f32) -> Self {
        Self {
            border: style.border().resolve_or_zero(Some(basis)),
            padding: style.padding().resolve_or_zero(Some(basis)),
            margin: style
                .margin()
                .map(|margin| margin.resolve(Some(basis)).unwrap_or(0.0)),
        }
    }
}

/// The edges of `node` as the layout session reads its style.
pub(crate) fn session_edges(tree: &LayoutSession<'_>, node: usize, basis: f32) -> BoxEdges {
    BoxEdges::of(&tree.physical_style(node), basis)
}

/// [`commit_child_layout`] for a caller that has read the child's edges.
pub(crate) fn commit_child_layout_with_edges(
    tree: &mut LayoutSession<'_>,
    node: usize,
    output: &LayoutOutput,
    location: Point<f32>,
    edges: BoxEdges,
) {
    let layout = BoxLayout {
        order: tree.nodes[node].layout.order,
        location,
        size: output.size,
        scrollable_overflow_rect: output.scrollable_overflow_rect,
        scrollbar_size: Size::ZERO,
        border: edges.border,
        padding: edges.padding,
        margin: edges.margin,
    };
    tree.set_unrounded_layout(NodeId::from(node), &layout);
}

/// Whether `line` holds text or an atomic inline ahead of the anchor at
/// `position` along it. The fragments come in visual order and are compared by
/// their inline position, as Chromium decides it for text that bidi reorders:
/// a block-level box that comes after reversed text in the source but lies
/// before it on the line starts the line.
fn content_precedes_anchor(line: &shodo::Line, position: f32) -> bool {
    // Positions are snapped to 1/64 px by the line engine.
    const SLACK: f32 = 1.0 / 32.0;
    line.fragments().any(|fragment| match fragment {
        shodo::Fragment::GlyphRun(run) => {
            run.inline_size() > 0.0 && run.inline_start() + run.inline_size() <= position + SLACK
        }
        shodo::Fragment::Atomic(atomic) => {
            atomic.margin_rect.inline_start + atomic.margin_rect.inline_size <= position + SLACK
        }
        _ => false,
    })
}

/// Whether an anchor at `position` along `line` lies at an odd embedding level,
/// where text reads right to left.
///
/// The level of an anchor is not reported. A neutral takes the direction of
/// the text on both of its sides when they agree and that of the paragraph
/// otherwise (UAX 9, N1 and N2), so the runs next to the anchor tell it: when
/// the levels of the runs on both sides have one parity, the anchor has it
/// too. The paragraph direction decides when the anchor has a run on one side
/// only, or two runs of different parity at its sides.
fn anchor_has_odd_level(line: &shodo::Line, position: f32, paragraph_rtl: bool) -> bool {
    // Positions are snapped to 1/64 px by the line engine.
    const SLACK: f32 = 1.0 / 32.0;
    // The nearest run on each side, with its distance from the anchor.
    let mut before: Option<(f32, u8)> = None;
    let mut after: Option<(f32, u8)> = None;
    let runs = line.fragments().filter_map(|fragment| match fragment {
        shodo::Fragment::GlyphRun(run) if run.inline_size() > 0.0 => Some(run),
        _ => None,
    });
    for run in runs {
        let end = run.inline_start() + run.inline_size();
        let (side, gap) = if end <= position + SLACK {
            (&mut before, position - end)
        } else {
            (&mut after, run.inline_start() - position)
        };
        if side.is_none_or(|(nearest, _)| gap < nearest) {
            *side = Some((gap, run.bidi_level()));
        }
    }
    match (before, after) {
        (Some((_, before)), Some((_, after))) if before % 2 == after % 2 => before % 2 == 1,
        _ => paragraph_rtl,
    }
}

/// Where the static position of the out-of-flow box `node` lies along the
/// block axis of the paragraph, from the content box's block-start edge.
///
/// A box that was inline-level starts at the top of the line of its anchor. A
/// block-level one sits where an in-flow block would be there: below the line
/// when text or an atomic inline comes before it on the line, and at the top
/// of the line otherwise (CSS 2.1 10.6.4). One without an anchor has no line
/// of its own: it follows the lines that come after the block child before it
/// and before the next block child or block-level `::after` (or that block
/// itself when no line does), or starts the paragraph
/// when nothing comes before it, or else, for the lines and blocks that
/// `::before` generates, follows all the content. `boxes` are the paragraph's
/// boxes in document order.
fn static_block_offset(
    lines: &IfcLines,
    boxes: &[(usize, IfcBoxKind)],
    node: usize,
    anchor: Option<(usize, f32)>,
    inline_level: bool,
) -> f32 {
    if let Some((index, position)) = anchor {
        let top = lines.line_top(index);
        let line = &lines.lines[index];
        return if inline_level || !content_precedes_anchor(line, position) {
            top
        } else {
            top + line.block_size()
        };
    }
    let mut block_before = None;
    let mut block_after = None;
    let mut reached = false;
    for &(id, kind) in boxes {
        if id == node {
            reached = true;
        } else if kind == IfcBoxKind::Block {
            if reached {
                block_after = Some(id);
                break;
            }
            block_before = Some(id);
        }
    }
    match block_before {
        // A fragmenting layout records no block ends: take the end of the
        // content.
        Some(block) => lines
            .block_ends
            .iter()
            .find(|&&(id, ..)| id == block)
            .map_or(lines.height, |&(_, next_line, end)| {
                // A line holding nothing but the box is dropped, so the box
                // has no anchor and the lines after the block are all before
                // it: it follows the last of them, up to the next block or the
                // `::after` block, whose lines come after the box.
                let next_block = block_after
                    .and_then(|id| lines.block_line_starts.iter().find(|&&(b, _)| b == id))
                    .map(|&(_, first)| first);
                let blocks_before = lines
                    .block_line_starts
                    .iter()
                    .position(|&(id, _)| id == block)
                    .map_or(0, |index| index + 1);
                let generated_block = lines
                    .generated_block_starts
                    .iter()
                    .find(|&&(count, _)| count >= blocks_before)
                    .map(|&(_, first)| first);
                let lines_end = next_block
                    .into_iter()
                    .chain(generated_block)
                    .min()
                    .unwrap_or(lines.lines.len());
                match lines_end.checked_sub(1).filter(|&last| last >= next_line) {
                    Some(last) => lines.line_top(last) + lines.lines[last].block_size(),
                    None => end + lines.shift_of(next_line),
                }
            }),
        // Nothing comes before the box, but for lines and blocks generated by
        // `::before`, which are not among the boxes.
        None if lines.lines.is_empty() && lines.placed_blocks.is_empty() => 0.0,
        None => lines.height,
    }
}

/// The static position of an out-of-flow box in a paragraph of a vertical
/// writing mode, on physical axes. Its block-start edge lies `block_offset`
/// from the block-start edge of the content box, which is the right edge in
/// `vertical-rl` and the left edge in `vertical-lr`.
///
/// A box that was inline-level lies at its anchor, `anchor.0` from the
/// inline-start edge of the content box, and extends the way the text around
/// it reads: downwards for left-to-right text and upwards for right-to-left
/// text (`anchor.1`), but the other way round in `sideways-lr`, which lays
/// the lines out upwards. Any other box starts on the inline-start edge of the
/// content box. The inline direction is reversed, so that its start is the
/// bottom edge, by `direction: rtl` or by `sideways-lr`.
fn vertical_static_position(
    writing_mode: WritingMode,
    rtl: bool,
    anchor: Option<(f32, bool)>,
    block_offset: f32,
    (content_left, content_top): (f32, f32),
    (content_right, inline_extent): (f32, f32),
) -> Point<AxisStaticPosition> {
    let x = if matches!(
        writing_mode,
        WritingMode::VerticalRl | WritingMode::SidewaysRl
    ) {
        AxisStaticPosition::from_edge(content_right - block_offset, AxisStaticEdge::End)
    } else {
        AxisStaticPosition::from_edge(content_left + block_offset, AxisStaticEdge::Start)
    };
    let reversed = rtl != (writing_mode == WritingMode::SidewaysLr);
    let y = match anchor {
        Some((offset, odd_level)) => {
            let at = if reversed {
                content_top + inline_extent - offset
            } else {
                content_top + offset
            };
            let reads_upwards = odd_level != (writing_mode == WritingMode::SidewaysLr);
            AxisStaticPosition::from_edge(
                at,
                if reads_upwards {
                    AxisStaticEdge::End
                } else {
                    AxisStaticEdge::Start
                },
            )
        }
        None if reversed => {
            AxisStaticPosition::from_edge(content_top + inline_extent, AxisStaticEdge::End)
        }
        None => AxisStaticPosition::from_edge(content_top, AxisStaticEdge::Start),
    };
    Point { x, y }
}

/// The start of a box of `size` along an axis whose static position is `axis`,
/// with its margins inside the position.
fn start_of(axis: AxisStaticPosition, size: f32, margin_start: f32, margin_end: f32) -> f32 {
    match axis.align.keyword {
        AxisStaticEdge::End => axis.area.end - margin_end - size,
        _ => axis.area.start + margin_start,
    }
}

/// A padding box in the coordinates of the root's border box.
#[derive(Clone, Copy)]
struct PaddingBox {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// Place the absolutely positioned and fixed boxes of the paragraph rooted at
/// `idx`, once its lines are final and its border box is `border_box`, the
/// way taffy's block algorithm places the positioned children of a block
/// (CSS 2.1 10.3.7, 10.6.4): against the root's padding box, an `auto` inset
/// pair taking the static position. That position is where the box's anchor
/// sits in the lines: at the anchor for a box that was inline-level, and for
/// one that was block-level where a block would be there (see
/// [`static_block_offset`]). The axes of a vertical root are laid on the
/// physical ones by [`vertical_static_position`].
///
/// An absolutely positioned box inside a positioned inline element is placed
/// against that element instead: the bounding box of the padding boxes of its
/// pieces on the lines, moved by its offset (CSS 2.1 10.1). An inline element
/// that is a containing block through `filter` or `will-change` is one for
/// fixed boxes as well.
pub(crate) fn place_out_of_flow(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    border_box: Size<f32>,
    output: &mut LayoutOutput,
) {
    let Some(root) = tree.nodes[idx].ifc.as_ref() else {
        return;
    };
    let positioned: Vec<usize> = root
        .boxes
        .iter()
        .filter(|b| b.kind == IfcBoxKind::OutOfFlow)
        .map(|b| b.node)
        .collect();
    if positioned.is_empty() && output.oof_candidates.is_empty() {
        return;
    }
    let rtl = root.rtl;
    let writing_mode = root.writing_mode;
    let vertical_root = writing_mode != WritingMode::HorizontalTb;
    let offsets = root.offsets.clone();
    let Some(lines) = root.lines.clone() else {
        return;
    };
    let style = tree.physical_style(idx);
    let border = style.border().resolve_or_zero(Some(border_box.width));
    let padding = style.padding().resolve_or_zero(Some(border_box.width));
    // The containing block: the root's padding box.
    let root_cb = PaddingBox {
        x: border.left,
        y: border.top,
        width: (border_box.width - border.left - border.right).max(0.0),
        height: (border_box.height - border.top - border.bottom).max(0.0),
    };
    if positioned.is_empty() {
        output.oof_positioning_area = Some(OofPositioningArea {
            size: Size {
                width: root_cb.width,
                height: root_cb.height,
            },
            offset: Point {
                x: root_cb.x,
                y: root_cb.y,
            },
        });
        return;
    }
    let content_left = border.left + padding.left;
    let content_top = border.top + padding.top;
    let content_right = border_box.width - border.right - padding.right;
    // The kinds of the boxes in document order, for the boxes without an
    // anchor.
    let mut box_order: Option<Vec<(usize, IfcBoxKind)>> = None;
    let owner = tree.ifc_source_owner(idx);
    // The pieces of the inline elements on the lines, made when a box needs
    // them.
    let mut pieces = None;
    // The root's own style, for the boxes that take a static position.
    let mut container_style = None;
    let mut candidates = OofCandidates::new();
    for node in positioned {
        let inline_cb = positioned_inline_ancestor(tree, idx, owner, node)
            .and_then(|inline| {
                let pieces = pieces.get_or_insert_with(|| {
                    let content = shodo::geometry::PhysicalSize {
                        width: if vertical_root {
                            (border_box.width - content_left - border.right - padding.right)
                                .max(0.0)
                        } else {
                            lines.width
                        },
                        height: (border_box.height - content_top - border.bottom - padding.bottom)
                            .max(0.0),
                    };
                    super::inline_boxes::inline_box_pieces(
                        &lines.lines,
                        writing_mode,
                        content,
                        |_| None,
                    )
                });
                inline_padding_box(tree, pieces, inline, border_box.width, |line| {
                    super::positioned_runs::block_shift(writing_mode, lines.shift_of(line))
                })
            })
            .map(|inline| {
                let (dx, dy) = relative_offset_of(tree, idx, owner, &offsets, node);
                PaddingBox {
                    x: content_left + inline.x + dx,
                    y: content_top + inline.y + dy,
                    ..inline
                }
            });
        let anchor = lines.lines.iter().enumerate().find_map(|(index, line)| {
            line.fragments().find_map(|fragment| match fragment {
                shodo::Fragment::OutOfFlowAnchor(anchor) if anchor.node.0 as usize == node => {
                    Some((index, anchor.inline_position))
                }
                _ => None,
            })
        });
        let inline_level = tree.nodes[node].display().is_inline_level();
        // The box's static position along the root's block axis, from the
        // content box's block-start edge.
        if anchor.is_none() && box_order.is_none() {
            box_order = tree.nodes[idx]
                .ifc
                .as_ref()
                .map(|root| root.boxes.iter().map(|b| (b.node, b.kind)).collect());
        }
        let block_offset = static_block_offset(
            &lines,
            box_order.as_deref().unwrap_or_default(),
            node,
            anchor,
            inline_level,
        );
        // A box that was inline-level extends from its anchor the way the
        // text there reads.
        let inline_anchor = anchor.filter(|_| inline_level).map(|(index, position)| {
            (
                position,
                anchor_has_odd_level(&lines.lines[index], position, rtl),
            )
        });
        let static_spot = if vertical_root {
            vertical_static_position(
                writing_mode,
                rtl,
                inline_anchor,
                block_offset,
                (content_left, content_top),
                (content_right, lines.width),
            )
        } else {
            let x = match inline_anchor {
                Some((position, odd_level)) => AxisStaticPosition::from_edge(
                    content_left
                        + if rtl {
                            lines.width - position
                        } else {
                            position
                        },
                    if odd_level {
                        AxisStaticEdge::End
                    } else {
                        AxisStaticEdge::Start
                    },
                ),
                // Without an anchor the box starts at the inline-start edge
                // of the content box: the right one in a right-to-left root.
                None if rtl => {
                    AxisStaticPosition::from_edge(content_left + lines.width, AxisStaticEdge::End)
                }
                None => AxisStaticPosition::from_edge(content_left, AxisStaticEdge::Start),
            };
            Point {
                x,
                y: AxisStaticPosition::from_edge(content_top + block_offset, AxisStaticEdge::Start),
            }
        };
        let Some(cb) = inline_cb else {
            let position = if tree.nodes[node].position() == &PositionValue::Fixed {
                OofPosition::Fixed
            } else {
                OofPosition::Absolute
            };
            // A box that was block-level sits where a block would be there
            // and is aligned in the static-position rectangle of the content
            // box like the children of a block without lines. A box that was
            // inline-level keeps to the point of its anchor.
            let static_position = if inline_level {
                static_spot
            } else if vertical_root {
                let rl = matches!(
                    writing_mode,
                    WritingMode::VerticalRl | WritingMode::SidewaysRl
                );
                // The box is aligned by `justify-self` along the inline axis
                // and by `align-self` along the block axis, as in a
                // horizontal root, and the block axis is then laid on the
                // physical x axis.
                let aligned = crate::layout::dispatch::block_flow::aligned_static_position(
                    container_style.get_or_insert_with(|| {
                        crate::layout::dispatch::block_flow::style_input::physical(tree, idx)
                    }),
                    &crate::layout::dispatch::block_flow::style_input::physical(tree, node),
                    if rtl != (writing_mode == WritingMode::SidewaysLr) {
                        values::Direction::Rtl
                    } else {
                        values::Direction::Ltr
                    },
                    Line {
                        start: content_top,
                        end: content_top + lines.width,
                    },
                    if rl {
                        block_offset
                    } else {
                        content_left + block_offset
                    },
                );
                Point {
                    x: if rl {
                        crate::layout::dispatch::reverse_static_position(aligned.y, content_right)
                    } else {
                        aligned.y
                    },
                    y: aligned.x,
                }
            } else {
                crate::layout::dispatch::block_flow::aligned_static_position(
                    container_style.get_or_insert_with(|| {
                        crate::layout::dispatch::block_flow::style_input::physical(tree, idx)
                    }),
                    &crate::layout::dispatch::block_flow::style_input::physical(tree, node),
                    if rtl {
                        values::Direction::Rtl
                    } else {
                        values::Direction::Ltr
                    },
                    Line {
                        start: content_left,
                        end: content_left + lines.width,
                    },
                    content_top + block_offset,
                )
            };
            candidates.push(OofCandidate {
                node: NodeId::from(node),
                order: tree.nodes[node].layout.order,
                position,
                static_position,
            });
            continue;
        };
        let child = tree.physical_style(node);
        let (child_margin, child_inset) = (child.margin(), child.inset());
        // Two auto margins share the free space between the insets equally.
        // In the root's block axis that holds even when the space is
        // negative (CSS 2.1 §10.6.4); in its inline axis negative space
        // goes to the end margin instead (§10.3.7).
        let auto_vertical_margins = child_margin.top.is_auto() && child_margin.bottom.is_auto();
        let auto_horizontal_margins = child_margin.left.is_auto() && child_margin.right.is_auto();
        let margin = child_margin.map(|margin| margin.resolve(Some(cb.width)).unwrap_or(0.0));
        let horizontal = |value: LengthPercentageAuto| value.maybe_resolve(Some(cb.width));
        let vertical = |value: LengthPercentageAuto| value.maybe_resolve(Some(cb.height));
        let (left, right) = (horizontal(child_inset.left), horizontal(child_inset.right));
        let (top, bottom) = (vertical(child_inset.top), vertical(child_inset.bottom));
        let width = used_block_width(tree, node, cb.width).or(match (left, right) {
            (Some(left), Some(right)) => {
                Some((cb.width - left - right - margin.left - margin.right).max(0.0))
            }
            _ => None,
        });
        let height = child
            .size()
            .height
            .maybe_resolve(Some(cb.height))
            .or(match (top, bottom) {
                (Some(top), Some(bottom)) => {
                    Some((cb.height - top - bottom - margin.top - margin.bottom).max(0.0))
                }
                _ => None,
            });
        let available =
            (cb.width - margin.left - margin.right - left.unwrap_or(0.0) - right.unwrap_or(0.0))
                .max(0.0);
        let node_id = NodeId::from(node);
        let inputs = LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::ContentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size { width, height },
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: Size {
                width: Some(cb.width),
                height: Some(cb.height),
            },
            available_space: Size {
                width: AvailableSpace::Definite(available),
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let output = crate::layout::dispatch::content_limits::with_oof_child_content_limits(
            tree,
            node_id,
            inputs,
            |tree, node_id, inputs| tree.compute_child_layout(node_id, inputs),
        );
        let x = match (left, right) {
            (Some(left), Some(right)) if auto_horizontal_margins => {
                let free_space = cb.width - left - right - output.size.width;
                if vertical_root || free_space >= 0.0 {
                    cb.x + left + free_space / 2.0
                } else if rtl {
                    cb.x + cb.width - right - output.size.width
                } else {
                    cb.x + left
                }
            }
            (Some(left), _) => cb.x + left + margin.left,
            (None, Some(right)) => cb.x + cb.width - right - output.size.width - margin.right,
            (None, None) => start_of(static_spot.x, output.size.width, margin.left, margin.right),
        };
        let y = match (top, bottom) {
            (Some(top), Some(bottom)) if auto_vertical_margins => {
                let free_space = cb.height - top - bottom - output.size.height;
                if !vertical_root || free_space >= 0.0 {
                    cb.y + top + free_space / 2.0
                } else if rtl {
                    cb.y + cb.height - bottom - output.size.height
                } else {
                    cb.y + top
                }
            }
            (Some(top), _) => cb.y + top + margin.top,
            (None, Some(bottom)) => cb.y + cb.height - bottom - output.size.height - margin.bottom,
            (None, None) => start_of(static_spot.y, output.size.height, margin.top, margin.bottom),
        };
        commit_child_layout(tree, node, &output, Point { x, y }, cb.width);
    }
    if !candidates.is_empty() {
        output.oof_candidates.append(&mut candidates);
    }
    if !output.oof_candidates.is_empty() {
        output.oof_positioning_area = Some(OofPositioningArea {
            size: Size {
                width: root_cb.width,
                height: root_cb.height,
            },
            offset: Point {
                x: root_cb.x,
                y: root_cb.y,
            },
        });
    }
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

/// Min- and max-content inline extents of the root's boxes, for the
/// paragraph's intrinsic sizes. Each box is measured without changing layout.
///
/// `percent_height` is the root's definite content-box height, the basis a
/// box's percentage height resolves against while its width is measured: it
/// sizes a replaced box through its aspect ratio. A vertical paragraph
/// measures along the block axis of its boxes and passes none; it passes the
/// root's definite physical content width as `physical_width` instead, the
/// basis a box's percentage width resolves against while its height is
/// measured.
pub(crate) fn intrinsics_of_boxes(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    basis: f32,
    percent_height: Option<f32>,
    physical_width: Option<f32>,
) -> BoxIntrinsics {
    let writing_mode = tree.nodes[idx]
        .ifc
        .as_ref()
        .map_or(WritingMode::HorizontalTb, |root| root.writing_mode);
    let vertical = matches!(
        writing_mode,
        WritingMode::VerticalRl
            | WritingMode::VerticalLr
            | WritingMode::SidewaysRl
            | WritingMode::SidewaysLr
    );
    let boxes = tree.nodes[idx]
        .ifc
        .as_ref()
        .map(|root| root.boxes.clone())
        .unwrap_or_default();
    let mut intrinsics = BoxIntrinsics {
        engine: AtomicIntrinsics::new(),
        blocks: (0.0, 0.0),
    };
    if let Some(root) = tree.nodes[idx].ifc.as_ref() {
        // The generated blocks that a vertical paragraph does not set in
        // take nothing from its inline extent.
        for width in root
            .generated_blocks
            .iter()
            .filter(|block| !vertical || block.on_root_axes)
            .map(generated_blocks::GeneratedBlock::intrinsic_width)
        {
            intrinsics.blocks.0 = intrinsics.blocks.0.max(width);
            intrinsics.blocks.1 = intrinsics.blocks.1.max(width);
        }
        for &(id, size) in root.marker_atomic.iter().chain(&root.generated_atomics) {
            intrinsics.engine.insert_atomic(
                id,
                AtomicIntrinsic {
                    min_content: size.inline_size,
                    max_content: size.inline_size,
                },
            );
        }
        // Percentages count as zero, as the width they resolve against is
        // not known yet.
        for generated in root.generated_boxes.iter() {
            let size = generated.size(None, (None, None), root.writing_mode, root.rtl);
            let extent = size.inline_size + size.margins.inline_sum();
            intrinsics.engine.insert_atomic(
                shodo::node::NodeId(generated.id),
                AtomicIntrinsic {
                    min_content: extent,
                    max_content: extent,
                },
            );
        }
    }
    for b in boxes {
        match b.kind {
            IfcBoxKind::Float => {
                let margin = resolved_margins(tree, b.node, basis);
                let (min_content, max_content) = content_widths(
                    tree,
                    b.node,
                    basis,
                    percent_height,
                    margin.left + margin.right,
                    false,
                );
                let style = tree.physical_style(b.node);
                let side = match FloatDirection::from_float(style.float()) {
                    Some(FloatDirection::Right) => FloatSide::Right,
                    _ => FloatSide::Left,
                };
                let clear = match style.clear() {
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
            // Out of flow: no part of the paragraph's widths.
            IfcBoxKind::OutOfFlow => {}
            IfcBoxKind::Block => {
                let margin = resolved_margins(tree, b.node, basis);
                // Only a block in the root's formatting context has margins
                // that collapse with its content's; the others are laid out
                // as formatting-context roots.
                let margins_collapse = if vertical {
                    shares_vertical_root_context(tree, b.node, writing_mode)
                } else {
                    in_the_roots_formatting_context(tree, b.node)
                };
                let (min_content, max_content) = if vertical {
                    content_heights(
                        tree,
                        b.node,
                        basis,
                        physical_width,
                        margin.top + margin.bottom,
                        margins_collapse,
                    )
                } else {
                    content_widths(
                        tree,
                        b.node,
                        basis,
                        percent_height,
                        margin.left + margin.right,
                        margins_collapse,
                    )
                };
                intrinsics.blocks.0 = intrinsics.blocks.0.max(min_content);
                intrinsics.blocks.1 = intrinsics.blocks.1.max(max_content);
            }
            IfcBoxKind::Atomic => {
                let margin = resolved_margins(tree, b.node, basis);
                let (min_content, max_content) = if vertical {
                    content_heights(
                        tree,
                        b.node,
                        basis,
                        physical_width,
                        margin.top + margin.bottom,
                        false,
                    )
                } else {
                    content_widths(
                        tree,
                        b.node,
                        basis,
                        percent_height,
                        margin.left + margin.right,
                        false,
                    )
                };
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
/// its layout. `margins_collapse` is whether the box's own vertical margins
/// may collapse with its content's, as they do for a block in the root's
/// formatting context; floats, atomic inlines and the other blocks establish a
/// formatting context of their own and do not.
fn content_widths(
    tree: &mut LayoutSession<'_>,
    node: usize,
    basis: f32,
    percent_height: Option<f32>,
    margins_x: f32,
    margins_collapse: bool,
) -> (f32, f32) {
    let parent_size = Size {
        width: Some(basis),
        height: percent_height,
    };
    content_sizes_on_axis(
        tree,
        node,
        parent_size,
        margins_x,
        RequestedAxis::Horizontal,
        margins_collapse,
    )
}

fn content_heights(
    tree: &mut LayoutSession<'_>,
    node: usize,
    basis: f32,
    physical_width: Option<f32>,
    margins_y: f32,
    margins_collapse: bool,
) -> (f32, f32) {
    let parent_size = Size {
        width: physical_width,
        height: Some(basis),
    };
    content_sizes_on_axis(
        tree,
        node,
        parent_size,
        margins_y,
        RequestedAxis::Vertical,
        margins_collapse,
    )
}

/// Min- and max-content extents of `node` along `axis`, against the
/// `parent_size` its percentages resolve against.
///
/// The box is measured with the margin-collapsing mode its layout runs in:
/// Taffy keeps the entries of one mode per node and drops them all when a
/// result of another mode is stored, so a measurement in a mode the layout
/// does not use empties the cache of a box on every probe.
fn content_sizes_on_axis(
    tree: &mut LayoutSession<'_>,
    node: usize,
    parent_size: Size<Option<f32>>,
    margins: f32,
    axis: RequestedAxis,
    margins_collapse: bool,
) -> (f32, f32) {
    let vertical = axis == RequestedAxis::Vertical;
    let measure = |tree: &mut LayoutSession<'_>, available: AvailableSpace| {
        let output = tree.compute_child_layout(
            NodeId::from(node),
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                sizing_mode: SizingMode::InherentSize,
                axis,
                known_dimensions: Size::NONE,
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size,
                available_space: if vertical {
                    Size {
                        width: AvailableSpace::MaxContent,
                        height: available,
                    }
                } else {
                    Size {
                        width: available,
                        height: AvailableSpace::MaxContent,
                    }
                },
                vertical_margins_are_collapsible: Line {
                    start: margins_collapse,
                    end: margins_collapse,
                },
            },
        );
        if vertical {
            output.size.height + margins
        } else {
            output.size.width + margins
        }
    };
    let parent_height = parent_size.height.filter(|_| !vertical);
    measure_against_parent_height(tree, node, parent_height, |tree| {
        (
            measure(tree, AvailableSpace::MinContent),
            measure(tree, AvailableSpace::MaxContent),
        )
    })
}

#[cfg(test)]
mod tests;
