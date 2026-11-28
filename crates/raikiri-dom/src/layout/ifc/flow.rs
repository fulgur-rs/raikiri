//! Measure and break an ifc root's paragraph.

use super::boxes::{BoxEdges, IfcBox, IfcBoxKind};
use super::geometry::IfcAxes;
use super::root::{IfcLines, IfcRoot};
use crate::Document;
use crate::layout::context::LayoutSession;
use crate::layout::dispatch::block_flow::context::{BlockContext, FloatView};
use crate::layout::protocol::{CollapsibleMarginSet, OofCandidates};
use raikiri_layout_geometry::{Edges, Line};
use raikiri_layout_style::values::Clear;
use raikiri_style::{CascadeResult, ComputedTextIndent};
use raikiri_traits::NodeKind;
use shodo::geometry::BaselineKind;
use shodo::geometry::{Direction, PhysicalSize, WritingMode};
use shodo::style::LineOptions;
use shodo::{AtomicIntrinsics, AtomicSizes, LayoutContext, LineConstraint, LineResult, Paragraph};
use std::collections::HashSet;

/// Logical inline space of one line, in content-box coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LineSpace {
    /// Offset of logical inline-start from the content-box start.
    pub(crate) inline_start: f32,
    /// Available logical inline extent.
    pub(crate) inline_size: f32,
}

/// Physical content-box geometry and the axes used by the IFC root.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FlowGeometry {
    /// Logical inline extent the lines are broken at.
    pub(crate) width: f32,
    pub(crate) axes: IfcAxes,
    /// Physical content-box size.
    pub(crate) content_size: PhysicalSize,
    /// Physical border plus padding on every side.
    pub(crate) insets: Edges<f32>,
    /// Border plus padding on the physical left and right side.
    pub(crate) edges: (f32, f32),
    /// Border plus padding at the block-start edge of the content box.
    pub(crate) top_edge: f32,
    /// The root's definite content-box height, the basis for the percentage
    /// heights of its floats and atomic inlines (CSS 2.1 §10.5). `None` when
    /// the root's height depends on its content.
    pub(crate) definite_height: Option<f32>,
    /// The most that a horizontal root's `max-height` allows its content box,
    /// the limit of the room a vertical float or atomic inline fits its lines
    /// in while the height of the root is open. `None` for a root without a
    /// length `max-height`.
    pub(crate) height_cap: Option<f32>,
    /// The most that a vertical root's `max-width` allows its content box,
    /// the limit of the room a horizontal child fits its width in while the
    /// width of the root is open. `None` for a root without a length
    /// `max-width`.
    pub(crate) width_cap: Option<f32>,
    /// A vertical root's definite physical content width, its block extent,
    /// the basis for the percentage widths of its empty atomic `::before`
    /// and `::after` boxes. `None` when the width depends on the content.
    pub(crate) vertical_width: Option<f32>,
    /// Whether the margins of a vertical root's block children at its
    /// block-start and block-end edges stay inside it, which is all that a
    /// `stretch` width leaves out of its room. A margin escapes through an edge
    /// that has no border or padding when the root shares its parent's
    /// formatting context.
    pub(crate) held_margins: Line<bool>,
    /// The column spanners of the paragraph are not laid out with its lines:
    /// the lines stand as if the spanners took no room, and the caller places
    /// the spanners once it knows where the lines end up. Its other blocks
    /// are laid out among the lines.
    pub(crate) blocks_apart: bool,
}

impl FlowGeometry {
    pub(crate) fn new(axes: IfcAxes, content_size: PhysicalSize, insets: Edges<f32>) -> Self {
        let top_edge = match axes.writing_mode() {
            WritingMode::HorizontalTb => insets.top,
            WritingMode::VerticalRl | WritingMode::SidewaysRl => insets.right,
            WritingMode::VerticalLr | WritingMode::SidewaysLr => insets.left,
        };
        Self {
            width: axes.inline_extent(content_size),
            axes,
            content_size,
            insets,
            edges: (insets.left, insets.right),
            top_edge,
            definite_height: None,
            height_cap: None,
            width_cap: None,
            vertical_width: None,
            held_margins: Line::TRUE,
            blocks_apart: false,
        }
    }

    pub(crate) fn horizontal(width: f32, edges: (f32, f32), top_edge: f32) -> Self {
        let insets = Edges {
            top: top_edge,
            right: edges.1,
            bottom: 0.0,
            left: edges.0,
        };
        Self::new(
            IfcAxes::new(WritingMode::HorizontalTb, Direction::Ltr),
            PhysicalSize { width, height: 0.0 },
            insets,
        )
    }

    /// Physical insets along the IFC's inline axis, in increasing coordinate
    /// order for the block formatting context.
    pub(crate) fn inline_axis_insets(self) -> (f32, f32) {
        if self.axes.writing_mode() == WritingMode::HorizontalTb {
            (self.insets.left, self.insets.right)
        } else {
            (self.insets.top, self.insets.bottom)
        }
    }

    /// Physical size of the root border box represented by this geometry.
    pub(crate) fn border_box_size(self) -> PhysicalSize {
        PhysicalSize {
            width: self.content_size.width + self.insets.left + self.insets.right,
            height: self.content_size.height + self.insets.top + self.insets.bottom,
        }
    }
}

/// How many times a line is laid out again because the space turned out to
/// differ for the height it took.
pub(crate) const MAX_SPACE_RETRIES: usize = 4;

pub(crate) fn resolve_indent(indent: ComputedTextIndent, width: f32) -> f32 {
    match indent {
        ComputedTextIndent::Px(px) => px,
        ComputedTextIndent::Percent(percent) => percent / 100.0 * width,
        ComputedTextIndent::Calc(calc) => calc.percent / 100.0 * width + calc.px,
    }
}

/// Min- and max-content widths of a paragraph without boxes.
#[cfg(test)]
pub(crate) fn intrinsic_widths(root: &IfcRoot, cx: &mut LayoutContext) -> (f32, f32) {
    intrinsic_widths_with(root, cx, &AtomicIntrinsics::EMPTY)
}

/// Min- and max-content widths of the paragraph with the intrinsic widths of
/// its boxes.
pub(crate) fn intrinsic_widths_with(
    root: &IfcRoot,
    cx: &mut LayoutContext,
    boxes: &AtomicIntrinsics,
) -> (f32, f32) {
    // The indent needs the width it is resolved against, which intrinsic
    // sizing does not have; its percentage counts as zero.
    let mut options = root.options;
    options.text_indent.length = resolve_indent(root.indent, 0.0);
    let Some(apart) = &root.block_intrinsics else {
        let sizes = root.paragraph.intrinsic_sizes(cx, &options, boxes);
        return (sizes.min_content, sizes.max_content);
    };
    // The content of the block-level pseudo-elements adds what their boxes
    // make of it, as a block child does.
    let sizes = apart.paragraph.intrinsic_sizes(cx, &options, boxes);
    let (mut min, mut max) = (sizes.min_content, sizes.max_content);
    let boxes_and_contents = apart.contents.iter().filter_map(|content| {
        root.generated_blocks
            .iter()
            .find(|block| block.start == content.start)
            .map(|block| (block, content))
    });
    for (block, content) in boxes_and_contents {
        let sizes = content
            .paragraph
            .intrinsic_sizes(cx, &block.intrinsic_line_options(), boxes);
        let (block_min, block_max) = block.intrinsic_widths((sizes.min_content, sizes.max_content));
        min = min.max(block_min);
        max = max.max(block_max);
    }
    (min, max.max(min))
}

/// Space of a line at block offset `y` that is `height` tall, from the float
/// segments it spans.
///
/// `content_inline_start` and `content_block_start` are the context's insets
/// from its border-box origin to the content-box origin on each IFC axis.
/// The result is relative to the content-box inline start and never wider
/// than `content_width`.
pub(crate) fn line_space(
    ctx: &BlockContext<'_>,
    content_inline_start: f32,
    content_block_start: f32,
    content_width: f32,
    y: f32,
    height: f32,
) -> LineSpace {
    let y = content_block_start + y;
    let mut start = 0.0_f32;
    let mut end = content_width;
    let mut slot = ctx.find_content_slot(y, Clear::None, None);
    loop {
        start = start.max(slot.x - content_inline_start);
        end = end.min(slot.x - content_inline_start + slot.width);
        // Without an active float the slot has no segment to continue from.
        let Some(segment) = slot.segment_id else {
            break;
        };
        // The next segment narrows the line only if it starts before the
        // line ends. A slot that does not advance ends the walk.
        let next = ctx.find_content_slot(y, Clear::None, Some(segment));
        if next
            .segment_id
            .is_none_or(|next_segment| next_segment <= segment)
            || next.y >= y + height
        {
            break;
        }
        slot = next;
    }
    // A float that the page walk grew past a page edge narrows the lines
    // below its end laid out unbroken, down to where it ends on the page
    // after.
    let [left, right] = ctx.line_extension_edges(y, height);
    if let Some(edge) = left {
        start = start.max(edge - content_inline_start);
    }
    if let Some(edge) = right {
        end = end.min(edge - content_inline_start);
    }
    LineSpace {
        inline_start: start,
        inline_size: (end - start).max(0.0),
    }
}

/// The floats that the boxes after an inline-level out-of-flow box of a
/// paragraph found placed, recorded while its lines are laid out. A line that
/// holds nothing but such a box comes before the floats that follow the box
/// in the paragraph, including those inside a later block child, so the
/// floats are kept as they stood at the first float or block child after
/// each such box (its boundary). Only those boundaries are kept: a view keeps
/// the float storage shared, so the next float placed copies it.
pub(crate) struct FloatSnapshots {
    boundaries: HashSet<usize>,
    views: Vec<(usize, Option<FloatView>)>,
}

impl FloatSnapshots {
    /// The boundaries of the inline-level out-of-flow boxes of a horizontal
    /// paragraph whose `boxes` are listed, or `None` when it has no such box.
    /// A box after the last float or block has no boundary and reads the
    /// floats as they stand once the lines are laid out, so the set may be
    /// empty.
    pub(crate) fn new(boxes: &[IfcBox], inline_level: impl Fn(usize) -> bool) -> Option<Self> {
        let mut boundaries = HashSet::new();
        let mut next = None;
        let mut any = false;
        for b in boxes.iter().rev() {
            match b.kind {
                IfcBoxKind::Float | IfcBoxKind::Block => next = Some(b.node),
                IfcBoxKind::OutOfFlow if inline_level(b.node) => {
                    // A box after the last float or block has no boundary
                    // but still needs the final floats.
                    any = true;
                    boundaries.extend(next);
                }
                _ => {}
            }
        }
        any.then(|| Self {
            boundaries,
            views: Vec::new(),
        })
    }

    /// Keep the floats `ctx` holds when `node`, about to be placed, is a
    /// boundary.
    pub(crate) fn record(&mut self, node: usize, ctx: &BlockContext<'_>) {
        if self.boundaries.remove(&node) {
            self.views.push((node, ctx.float_view()));
        }
    }
}

/// The floats beside a paragraph, kept once its lines are laid out to ask
/// for the room of a line that holds nothing but out-of-flow boxes (see
/// [`line_space`] and [`FloatSnapshots`]).
pub(crate) struct LineFloats {
    /// The floats as they stood at each boundary (`None` when there were
    /// none).
    before: Vec<(usize, Option<FloatView>)>,
    /// All the floats once the lines are laid out.
    after: Option<FloatView>,
    /// The insets of the content box from the context's border-box origin,
    /// along the inline and the block axis.
    content_insets: (f32, f32),
    content_width: f32,
}

impl LineFloats {
    /// The floats `ctx` holds and the floats kept at the boundaries,
    /// beside a content box `content_width` wide.
    pub(crate) fn new(
        ctx: &BlockContext<'_>,
        snapshots: FloatSnapshots,
        content_insets: (f32, f32),
        content_width: f32,
    ) -> Self {
        Self {
            before: snapshots.views,
            after: ctx.float_view(),
            content_insets,
            content_width,
        }
    }

    /// The space of an empty line at block offset `y` from the content box,
    /// beside the floats placed before the boundary `next`, or beside all of
    /// them when `next` is `None` or was never reached.
    pub(crate) fn space_at(&self, y: f32, next: Option<usize>) -> LineSpace {
        let view = next
            .and_then(|next| self.before.iter().find(|&&(node, _)| node == next))
            .map_or(self.after.as_ref(), |(_, view)| view.as_ref());
        view.map_or(
            LineSpace {
                inline_start: 0.0,
                inline_size: self.content_width,
            },
            |view| {
                view.query(|ctx| {
                    line_space(
                        ctx,
                        self.content_insets.0,
                        self.content_insets.1,
                        self.content_width,
                        y,
                        0.0,
                    )
                })
            },
        )
    }
}

pub(crate) fn break_lines(root: &IfcRoot, cx: &mut LayoutContext, width: f32) -> IfcLines {
    let mut options = root.options;
    options.text_indent.length = resolve_indent(root.indent, width);
    let mut atomics = AtomicSizes::new();
    for &(id, size) in root.marker_atomic.iter().chain(&root.generated_atomics) {
        atomics.insert(id, size);
    }
    let containing = if root.writing_mode == WritingMode::HorizontalTb {
        (Some(width), root.definite_height)
    } else {
        (root.vertical_width, Some(width))
    };
    for generated in root.generated_boxes.iter() {
        atomics.insert(
            shodo::node::NodeId(generated.id),
            generated.size(Some(width), containing, root.writing_mode, root.rtl),
        );
    }
    let mut placed = place_lines(
        &root.line_paragraph,
        &options,
        cx,
        &atomics,
        width,
        IfcAxes::new(
            root.writing_mode,
            if root.rtl {
                Direction::Rtl
            } else {
                Direction::Ltr
            },
        ),
        root.outside_marker_height,
        |_, _| LineSpace {
            inline_start: 0.0,
            inline_size: width,
        },
    );
    placed.width = width;
    placed
}

/// Lay the paragraph out line by line. `space(y, height)` gives the space
/// available to a line that starts at block offset `y` and is `height` tall.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_lines(
    paragraph: &Paragraph,
    options: &LineOptions,
    cx: &mut LayoutContext,
    atomics: &AtomicSizes,
    content_width: f32,
    axes: IfcAxes,
    outside_marker_height: Option<f32>,
    mut space: impl FnMut(f32, f32) -> LineSpace,
) -> IfcLines {
    let mut lines: Vec<shodo::Line> = Vec::new();
    let mut token = paragraph.start_token();
    let mut y = 0.0_f32;
    let mut first_width = None;
    loop {
        let mut assumed_height = 0.0_f32;
        let mut retries = 0;
        let mut leading = 0.0;
        let line = loop {
            let available = space(y, assumed_height);
            first_width.get_or_insert(available.inline_size);
            let end_inset =
                (content_width - available.inline_start - available.inline_size).max(0.0);
            let mut constraint = LineConstraint::from_physical_insets(
                content_width,
                available.inline_start,
                end_inset,
                axes.writing_mode(),
                axes.direction(),
            );
            constraint.block_offset = y;
            constraint.block_start_leading = leading;
            match paragraph.next_line(cx, token, options, &constraint, atomics) {
                LineResult::Line(line) => {
                    let wanted = if lines.is_empty() {
                        super::outside_marker::leading(
                            outside_marker_height,
                            line.baseline(BaselineKind::Alphabetic) - line.block_start_leading(),
                        )
                    } else {
                        0.0
                    };
                    if wanted != leading {
                        leading = wanted;
                        continue;
                    }
                    let height = line.block_size();
                    // The line may span more of a float than the height it
                    // was assumed to have; lay it out again if the space for
                    // its real height differs.
                    if retries < MAX_SPACE_RETRIES && space(y, height) != available {
                        assumed_height = height;
                        retries += 1;
                        continue;
                    }
                    break Some(line);
                }
                LineResult::Done => break None,
                other => {
                    // Floats and blocks inside the paragraph are not
                    // projected here, so no other result is expected.
                    debug_assert!(false, "unexpected line result: {other:?}");
                    break None;
                }
            }
        };
        let Some(line) = line else { break };
        y = line.block_offset() + line.block_size();
        token = line.break_token();
        lines.push(line);
    }
    // Most paragraphs keep fewer lines than a vector grows room for.
    lines.shrink_to_fit();
    IfcLines {
        width: first_width.unwrap_or(0.0),
        height: y,
        lines: std::sync::Arc::new(lines),
        object_text_prefixes: Default::default(),
        oof_candidates: OofCandidates::new(),
        beside_floats: false,
        below_floats: false,
        external_float_gap: 0.0,
        discarded_float_gap: 0.0,
        escaping_margin: CollapsibleMarginSet::ZERO,
        leading_block_margin: None,
        leading_block_baseline: None,
        block_line_starts: Vec::new(),
        generated_block_starts: Vec::new(),
        generated_block_lines: Vec::new(),
        first_block_start: None,
        block_starts: Vec::new(),
        flow_end: 0.0,
        block_ends: Vec::new(),
        block_boxes: Vec::new(),
        shifts: super::root::LineShifts::default(),
        fragment_box_placements: Vec::new(),
        fragmentainer_line_ranges: None,
        unfragmented_tail_column: None,
        placed_blocks: Vec::new(),
        forced_breaks: Vec::new(),
        line_floats: None,
    }
}

/// End each horizontal line that overflows `available` in an ellipsis when
/// `root` has `text-overflow: ellipsis` (CSS Overflow 3 §5.1). The engine
/// drops the content past the ellipsis and leaves the line's metrics alone;
/// the root's overflow clip trims inline boxes the ellipsis cuts.
pub(crate) fn apply_text_overflow(
    root: &IfcRoot,
    lines: &mut IfcLines,
    cx: &mut LayoutContext,
    available: f32,
) {
    truncate_lines(root, &mut lines.lines, cx, available);
}

/// [`apply_text_overflow`] on a shared line list. A line already truncated
/// is left as it is.
pub(crate) fn truncate_lines(
    root: &IfcRoot,
    lines: &mut std::sync::Arc<Vec<shodo::Line>>,
    cx: &mut LayoutContext,
    available: f32,
) {
    if !root.ellipsis || root.writing_mode != WritingMode::HorizontalTb {
        return;
    }
    for line in std::sync::Arc::make_mut(lines) {
        line.truncate_with_ellipsis(cx, available);
    }
}

pub(crate) fn first_baseline(lines: &IfcLines) -> Option<f32> {
    first_baseline_of(lines, BaselineKind::Alphabetic)
}

pub(crate) fn last_baseline(lines: &IfcLines) -> Option<f32> {
    last_baseline_of(lines, BaselineKind::Alphabetic)
}

/// The `kind` baseline of the first line, measured along the block axis from
/// the block-start edge of the content box, where the content alignment of
/// the paragraph box put it.
pub(crate) fn first_baseline_of(lines: &IfcLines, kind: BaselineKind) -> Option<f32> {
    lines
        .lines
        .first()
        .map(|line| line.block_offset() + line.baseline(kind) + lines.shifts.aligned())
}

/// The `kind` baseline of the last line, measured like [`first_baseline_of`].
pub(crate) fn last_baseline_of(lines: &IfcLines, kind: BaselineKind) -> Option<f32> {
    // `Line::baseline` is relative to the line; the block offset places it in
    // the paragraph.
    lines
        .lines
        .last()
        .map(|line| line.block_offset() + line.baseline(kind) + lines.shifts.aligned())
}

/// Break every ifc root again at a page-specific width: the content width of
/// the nearest authored-width ancestor of its text, with eligible auto blocks
/// projected from that width, else `max_advance`. Box geometry is not touched.
pub(crate) fn rebreak_roots(
    doc: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
    max_advance: f32,
) {
    let Some(mut state) = doc.ifc.take() else {
        return;
    };
    let roots: Vec<usize> = (0..doc.nodes.len())
        .filter(|&id| doc.nodes[id].ifc.is_some())
        .collect();
    let widths = crate::layout::multicol::AncestorFacts::for_rebreak(doc, cascade, max_advance);
    for id in roots {
        // A root inside a `min-content`, `max-content` or `fit-content` box
        // keeps the lines its intrinsic width was measured with; no page
        // width applies to it.
        if widths.in_intrinsic_width(id) {
            continue;
        }
        // The search starts at the parent of the text, so the root's first
        // text node is the starting point.
        let generated_text = [
            raikiri_style::PseudoElem::Before,
            raikiri_style::PseudoElem::After,
            raikiri_style::PseudoElem::Marker,
        ]
        .into_iter()
        .any(|pseudo| crate::generated_content::is_in_flow_generated_text(cascade, id, pseudo));
        let generated_width = generated_text.then(|| widths.content_width(id)).flatten();
        let width = first_text_node(doc, id)
            .and_then(|text| widths.authored_containing_width(text))
            .or(generated_width)
            .unwrap_or(max_advance);
        // A box that makes a formatting context of its own is sized by layout,
        // beside a float for one. Breaking again never moves a box, so the
        // lines stay within the content width layout gave it.
        let width = if widths.sized_by_context_root(id) {
            let layout = &doc.nodes[id].layout;
            let content = layout.size.width
                - layout.padding.left
                - layout.padding.right
                - layout.border.left
                - layout.border.right
                - layout.scrollbar_size.width;
            // A box layout has not sized has no width to hold the lines to.
            if content > 0.0 {
                width.min(content)
            } else {
                width
            }
        } else {
            width
        };
        let Some(root) = doc.nodes[id].ifc.as_mut() else {
            continue;
        };
        // Lines laid out beside floats depend on the float context of the
        // performed layout, which is gone here; breaking them again at the
        // full width would run them under the floats. The positions of a
        // root's own boxes are tied to the lines they were placed with, and
        // lines split in columns to the column width.
        if !root.boxes.is_empty()
            || !root.generated_blocks.is_empty()
            || root.multicol_fragments.is_some()
            || root.lines.as_ref().is_some_and(|lines| lines.beside_floats)
        {
            continue;
        }
        root.resolve_percent_edges(width, &mut state.layout_cx, &state.fonts);
        let mut lines = break_lines(root, &mut state.layout_cx, width);
        apply_text_overflow(root, &mut lines, &mut state.layout_cx, width);
        let ifc = root.without_lines();
        root.lines = Some(lines);
        // The inline elements follow the new lines; the root's box keeps the
        // border and padding it was laid out with.
        let layout = doc.nodes[id].layout;
        // The new lines are aligned in the content box as layout aligned the
        // lines they replace (see `align_ifc_content`).
        let horizontal = doc.nodes[id]
            .ifc
            .as_ref()
            .is_some_and(|root| root.writing_mode == shodo::geometry::WritingMode::HorizontalTb);
        let alignment = doc
            .finished_style(cascade, id)
            .filter(|_| !crate::layout::dispatch::is_table_box(doc.nodes[id].display()))
            .map_or(
                raikiri_layout_style::values::AlignContent::NORMAL,
                |style| style.view().align_content(),
            );
        if let Some(lines) = doc.nodes[id]
            .ifc
            .as_mut()
            .and_then(|root| root.lines.as_mut())
        {
            // A vertical paragraph aligns its lines along the width.
            let content = if horizontal {
                layout.size.height
                    - layout.padding.top
                    - layout.padding.bottom
                    - layout.border.top
                    - layout.border.bottom
                    - layout.scrollbar_size.height
            } else {
                layout.size.width
                    - layout.padding.left
                    - layout.padding.right
                    - layout.border.left
                    - layout.border.right
                    - layout.scrollbar_size.width
            };
            let offset = crate::layout::dispatch::block_flow::content_alignment_offset(
                alignment,
                content - lines.height,
            );
            lines.shifts.align(offset);
        }
        let geometry = FlowGeometry::horizontal(
            width,
            (
                layout.padding.left + layout.border.left,
                layout.padding.right + layout.border.right,
            ),
            layout.padding.top + layout.border.top,
        );
        let lines = doc.nodes[id]
            .ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
            .map(|lines| std::sync::Arc::clone(&lines.lines));
        if let Some(lines) = lines {
            // The new session has bound no styles, so the edges come from the
            // records the finished layout left in the document. An element
            // whose record no longer fits `cascade` keeps no edges.
            super::records::record_inline_boxes(
                doc,
                id,
                &ifc,
                &lines,
                &geometry,
                |tree, element, basis| {
                    tree.finished_style(cascade, element)
                        .map_or(BoxEdges::ZERO, |style| BoxEdges::of(&style.view(), basis))
                },
            );
        }
    }
    doc.ifc = Some(state);
}

/// The first in-document text node under `root`, in document order.
fn first_text_node(doc: &Document, root: usize) -> Option<usize> {
    let mut stack: Vec<usize> = doc.nodes[root].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let node = &doc.nodes[id];
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Text => return Some(id),
            NodeKind::Element => stack.extend(node.children.iter().rev().copied()),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests;
