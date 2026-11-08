use super::*;
use std::collections::HashMap;

type Measurement = (usize, LayoutOutput, BoxLayout, f32, f32, f32);

pub(super) struct Group {
    /// One entry per paragraph visited, from the chunk's first paragraph.
    pub(super) placements: Vec<Option<Placement>>,
    pub(super) height: f32,
    /// The flow the placements follow when wrappers joined it; without one,
    /// they follow the entries the group was planned from.
    pub(super) flow: Option<Flow>,
}

#[derive(Clone)]
pub(super) struct Placement {
    pub(super) first_column: usize,
    pub(super) first_y: f32,
    pub(super) first_height: f32,
    pub(super) last_column: usize,
    pub(super) last_y: f32,
    pub(super) fragments: Vec<MulticolTextFragment>,
    /// The column and block offset the flow had reached before the entry and
    /// the margins above it: for the entry that ends a wrapper, where the
    /// wrapper's border box ends.
    pub(super) entry_column: usize,
    pub(super) entry_y: f32,
}

impl Group {
    /// The placement of the `entry`th of the entries the group was planned
    /// from. A wrapper whose boxes joined the flow is placed from the top of
    /// its first box to the end of its last.
    pub(super) fn placement_of(&self, entry: usize) -> Option<Placement> {
        match &self.flow {
            None => self.placements.get(entry)?.clone(),
            Some(flow) => self.flow_placement(flow, *flow.direct.get(entry)?),
        }
    }

    fn flow_placement(&self, flow: &Flow, index: usize) -> Option<Placement> {
        let Role::Open(close) = flow.roles[index] else {
            return self.placements.get(index)?.clone();
        };
        let start = self.placements.get(index)?.as_ref()?;
        let end = self.placements.get(close)?.as_ref()?;
        // The margins of the wrapper and of its first box adjoin, so its
        // border box starts where that box does.
        let first = (index + 1..close)
            .filter(|&inner| flow.roles[inner] == Role::Box)
            .find_map(|inner| self.placements.get(inner)?.as_ref())
            .unwrap_or(start);
        // A wrapper that runs into a later column reaches the end of the ones
        // it leaves, as a paragraph broken there does.
        let first_height = if end.entry_column == first.first_column {
            end.entry_y - first.first_y
        } else {
            self.height - first.first_y
        };
        Some(Placement {
            first_column: first.first_column,
            first_y: first.first_y,
            first_height: first_height.max(0.0),
            last_column: end.last_column,
            last_y: end.last_y,
            fragments: Vec::new(),
            entry_column: start.entry_column,
            entry_y: start.entry_y,
        })
    }

    /// The boxes and wrappers inside the wrapper that the `entry`th entry the
    /// group was planned from is, wrappers before what they hold, with their
    /// placements and those of the wrappers holding them.
    pub(super) fn wrapped(&self, entry: usize) -> Vec<Wrapped> {
        self.try_wrapped(entry).unwrap_or_default()
    }

    fn try_wrapped(&self, entry: usize) -> Option<Vec<Wrapped>> {
        let flow = self.flow.as_ref()?;
        let open = *flow.direct.get(entry)?;
        let Role::Open(close) = flow.roles[open] else {
            return None;
        };
        let mut wrappers = HashMap::from([(open, self.flow_placement(flow, open)?)]);
        let mut boxes = Vec::new();
        for index in open + 1..close {
            if flow.roles[index] == Role::Close {
                continue;
            }
            // Every box inside a wrapper has its wrapper placed before it.
            let parent_index = flow.parents[index].unwrap_or(open);
            let parent = wrappers.get(&parent_index)?.clone();
            // Source white space has no box to place.
            let Some(placement) = self.flow_placement(flow, index) else {
                continue;
            };
            if matches!(flow.roles[index], Role::Open(_)) {
                wrappers.insert(index, placement.clone());
            }
            boxes.push(Wrapped {
                node: flow.entries[index].0,
                placement,
                parent_node: flow.entries[parent_index].0,
                parent,
            });
        }
        Some(boxes)
    }
}

/// A box inside a wrapper whose boxes joined the flow of the columns.
pub(super) struct Wrapped {
    pub(super) node: usize,
    pub(super) placement: Placement,
    /// The wrapper that holds the box, and its placement.
    pub(super) parent_node: usize,
    pub(super) parent: Placement,
}

/// What an entry of a flow stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    /// A box the columns place: a paragraph, an empty box or a box they hold
    /// whole.
    Box,
    /// The start of a wrapper whose boxes join the flow, with the margin above
    /// it. Its value is the index of the entry that ends the wrapper.
    Open(usize),
    /// The end of a wrapper, with the margin below it.
    Close,
}

/// The entries of a group with the boxes of its wrappers in their place, in
/// the order the columns take them.
///
/// A wrapper is a block that holds only blocks and leaves nothing of its own
/// to break or paint (see [`is_flow_wrapper`]), so its boxes break between
/// the columns as the container's own do. Its margins adjoin those of its
/// first and last boxes, so they stand in the flow as boxes with no room,
/// whose margins collapse through them.
pub(super) struct Flow {
    entries: Vec<Measurement>,
    roles: Vec<Role>,
    /// For each entry, the index of the entry that starts the wrapper holding
    /// it, if a wrapper does.
    parents: Vec<Option<usize>>,
    /// For each entry the flow was made from, its index in the flow.
    direct: Vec<usize>,
    /// For each entry, how far right of the container's column the content
    /// box of the wrapper holding it starts.
    insets: Vec<f32>,
}

/// How deep wrappers may nest in a flow. Deeper ones keep the other
/// strategies.
const MAX_WRAPPER_DEPTH: usize = 32;

impl Flow {
    /// The flow of `entries` with no wrapper joining it.
    fn plain(entries: &[Measurement]) -> Self {
        Self {
            entries: entries.to_vec(),
            roles: vec![Role::Box; entries.len()],
            parents: vec![None; entries.len()],
            direct: (0..entries.len()).collect(),
            insets: vec![0.0; entries.len()],
        }
    }

    fn has_wrappers(&self) -> bool {
        self.entries.len() != self.direct.len()
    }

    /// The flow of `entries`, or `None` when no wrapper joins it, or when one
    /// nests too deep.
    fn of(tree: &LayoutSession<'_>, entries: &[Measurement]) -> Option<Self> {
        let mut flow = Self {
            entries: Vec::with_capacity(entries.len()),
            roles: Vec::with_capacity(entries.len()),
            parents: Vec::with_capacity(entries.len()),
            direct: Vec::with_capacity(entries.len()),
            insets: Vec::with_capacity(entries.len()),
        };
        for entry in entries {
            flow.direct.push(flow.entries.len());
            flow.push(tree, entry.clone(), None, 0.0, 0)?;
        }
        flow.roles
            .iter()
            .any(|role| matches!(role, Role::Open(_)))
            .then_some(flow)
    }

    fn push(
        &mut self,
        tree: &LayoutSession<'_>,
        entry: Measurement,
        parent: Option<usize>,
        inset: f32,
        depth: usize,
    ) -> Option<()> {
        let id = entry.0;
        if !is_flow_wrapper(tree, id) {
            // Inside a wrapper, the flow takes paragraphs whose pieces hold
            // nothing but lines, between no padding or border, and white
            // space; the pieces are recorded without the boxes of the lines.
            if parent.is_some() && !is_wrapped_box(tree, id) {
                return None;
            }
            self.entries.push(entry);
            self.roles.push(Role::Box);
            self.parents.push(parent);
            self.insets.push(inset);
            return Some(());
        }
        if depth >= MAX_WRAPPER_DEPTH {
            return None;
        }
        let open = self.entries.len();
        let (_, ref output, layout, margin_top, margin_bottom, _) = entry;
        self.entries
            .push((id, output.clone(), layout, margin_top, 0.0, margin_top));
        self.roles.push(Role::Open(open));
        self.parents.push(parent);
        self.insets.push(inset);
        let content_inset = inset + layout.margin.left + layout.border.left + layout.padding.left;
        for child in tree.children_through_contents(id) {
            let node = &tree.nodes[child];
            if !node.is_in_document() || node.is_display_none() {
                continue;
            }
            let layout = node.layout;
            let measurement = (
                child,
                LayoutOutput::from_outer_size(layout.size),
                layout,
                layout.margin.top,
                layout.margin.bottom,
                layout.margin.top + layout.size.height + layout.margin.bottom,
            );
            self.push(tree, measurement, Some(open), content_inset, depth + 1)?;
        }
        let close = self.entries.len();
        self.roles[open] = Role::Open(close);
        self.entries.push((
            id,
            output.clone(),
            layout,
            0.0,
            margin_bottom,
            margin_bottom,
        ));
        self.roles.push(Role::Close);
        self.parents.push(parent);
        self.insets.push(inset);
        Some(())
    }
}

/// The number of paragraphs inside `id` when it is a wrapper whose boxes all
/// join the flow of the columns, as [`Flow`] takes them.
pub(super) fn wrapped_paragraphs(tree: &LayoutSession<'_>, id: usize) -> Option<usize> {
    let mut pending = vec![(id, 0usize)];
    let mut paragraphs = 0;
    while let Some((wrapper, depth)) = pending.pop() {
        if depth >= MAX_WRAPPER_DEPTH || !is_flow_wrapper(tree, wrapper) {
            return None;
        }
        for child in tree.children_through_contents(wrapper) {
            let node = &tree.nodes[child];
            if !node.is_in_document() || node.is_display_none() {
                continue;
            }
            if is_flow_wrapper(tree, child) {
                pending.push((child, depth + 1));
            } else if !is_wrapped_box(tree, child) {
                return None;
            } else if node.kind() != NodeKind::Text {
                paragraphs += 1;
            }
        }
    }
    Some(paragraphs)
}

/// Whether `id`, inside a wrapper, can join the flow of the columns: white
/// space, or a paragraph of lines alone, with no padding or border above or
/// below them, that spans no columns.
fn is_wrapped_box(tree: &LayoutSession<'_>, id: usize) -> bool {
    let node = layout_node(tree, id);
    if node.kind() == NodeKind::Text {
        return node.ifc.is_none();
    }
    node.ifc.is_some()
        && node.ifc_boxes().is_empty()
        && !node.column_span_all
        && !lineless::has_block_edge_decorations(&style_of(tree, id))
        && !is_sized_paragraph(tree, id)
}

/// Whether `id` is a wrapper whose boxes join the flow of the columns: a
/// block that holds only blocks, has no border and no padding above or below
/// them, takes the width of the column and paints nothing, so that cutting it
/// between the columns where its boxes break leaves nothing of it to show. Its margins adjoin those of its
/// first and last boxes, as it starts no formatting context. Padding at its
/// sides narrows its boxes, which are laid out in it.
pub(super) fn is_flow_wrapper(tree: &LayoutSession<'_>, id: usize) -> bool {
    let node = layout_node(tree, id);
    if node.kind() != NodeKind::Element
        || node.ifc.is_some()
        || node.formatting_context != FormattingContext::Block
        || node.layout_position() != Position::Static
        || node.paints_whole_box_effects
        || node.paints_background_color
        || node.column_span_all
        || node.break_inside != raikiri_style::property::BreakInside::Auto
        || !matches!(node.display(), DisplayValue::Block | DisplayValue::ListItem)
        || !lineless::is_plain_box_apart_from_display(tree, id, false)
    {
        return false;
    }
    let style = style_of(tree, id);
    let margin = style.margin();
    style.border() == Edges::splat(LengthPercentage::ZERO)
        && style.padding().top == LengthPercentage::ZERO
        && style.padding().bottom == LengthPercentage::ZERO
        && !margin.top.is_auto()
        && !margin.bottom.is_auto()
        && node.multicol_auto_width
        && style.min_size().width.is_auto()
        && style.max_size().width.is_auto()
        && style.size().height.is_auto()
        && style.min_size().height.is_auto()
        && style.max_size().height.is_auto()
        && lineless::rendered_elements(tree, id).next().is_some()
}

struct Paragraph {
    extents: Vec<(f32, f32)>,
    margin_top: f32,
    margin_bottom: f32,
    orphans: usize,
    widows: usize,
    /// Block size of the padding and border above the first line.
    edge_start: f32,
    /// Block size of the padding and border below the last line.
    edge_end: f32,
    /// For a box the columns hold whole, the number of lines it holds: the one
    /// fragment of such a box reaches all of them, however many extents the
    /// plan gives it.
    held_lines: Option<usize>,
}

impl Paragraph {
    /// A box the columns hold whole: its block size is the one extent of a
    /// paragraph of one line, so no break falls inside it. `lines` is the
    /// number of lines it holds, if any.
    fn whole_box(height: f32, margin_top: f32, margin_bottom: f32, lines: Option<usize>) -> Self {
        Self {
            extents: vec![(0.0, height)],
            margin_top,
            margin_bottom,
            orphans: 1,
            widows: 1,
            edge_start: 0.0,
            edge_end: 0.0,
            held_lines: lines,
        }
    }

    /// The first of the last `widows` lines: a break before it leaves the
    /// widows, and a break after it leaves fewer.
    fn widows_cut(&self) -> usize {
        self.extents.len().saturating_sub(self.widows)
    }
}

pub(super) fn balance(
    tree: &LayoutSession<'_>,
    entries: &[Measurement],
    context: FragmentationContext,
) -> Option<Group> {
    let flow = Flow::of(tree, entries).unwrap_or_else(|| Flow::plain(entries));
    let (paragraphs, paragraph_count, has_empty, high) = collect(
        tree,
        &flow.entries,
        Some(&flow.roles),
        true,
        BlockEdges::Refused,
        false,
    )?;
    if paragraph_count == 0
        || (paragraph_count == 1 && !has_empty)
        || !high.is_finite()
        || context.column_count == 0
    {
        return None;
    }
    let (group, height) = balanced(&paragraphs, context, high)?;
    let mut group = finish_group(group.placements, height);
    group.flow = flow.has_wrappers().then_some(flow);
    Some(group)
}

/// The paragraphs placed in the columns of `context` at the height that
/// balances them, with that height. `ceiling` is a height that holds them in
/// one column.
fn balanced(
    paragraphs: &[Option<Paragraph>],
    context: FragmentationContext,
    ceiling: f32,
) -> Option<(Group, f32)> {
    let divided = divided_height(paragraphs, context, FlowCursor::default(), ceiling)?;
    // Trial work and storage depend on shaped lines, never the declared count.
    let mut low = 0.0;
    let mut high = ceiling;
    for _ in 0..32 {
        if high - low <= 0.001 {
            break;
        }
        let trial = (low + high) * 0.5;
        if fill(paragraphs, context, trial, false).is_some() {
            high = trial;
        } else {
            low = trial;
        }
    }
    // The columns are as tall as the content in one column divided among them,
    // or, when the breaks between the boxes leave no room for that, as the
    // least height that holds them. The margin below the last box does not
    // have to fit in the columns, but counts in what is divided.
    let group = fill(paragraphs, context, divided.max(high), false)?;
    let height = divided.max(group.height);
    Some((group, height))
}

/// The height of the paragraphs from `resume` in one column, with the margins
/// between them and the one below the last, divided among the columns of
/// `context`. `ceiling` is a height that holds them in one column.
fn divided_height(
    paragraphs: &[Option<Paragraph>],
    context: FragmentationContext,
    resume: FlowCursor,
    ceiling: f32,
) -> Option<f32> {
    let one_column = FragmentationContext {
        column_count: 1,
        ..context
    };
    let stacked = fill_chunk(paragraphs, one_column, ceiling, resume, true)?
        .group
        .height;
    Some(stacked / context.column_count as f32)
}

/// Place paragraphs in columns of the definite height `height` in source
/// order, continuing in overflow columns past the last one (CSS Multi-column
/// 1, 8.2).
pub(super) fn fill_definite(
    tree: &LayoutSession<'_>,
    entries: &[Measurement],
    context: FragmentationContext,
    height: f32,
) -> Option<Group> {
    let flow = Flow::of(tree, entries).unwrap_or_else(|| Flow::plain(entries));
    let (paragraphs, paragraph_count, _, ceiling) = collect(
        tree,
        &flow.entries,
        Some(&flow.roles),
        true,
        BlockEdges::Sliced,
        true,
    )?;
    let flow = flow.has_wrappers().then_some(flow);
    // A chain of wrappers around one paragraph keeps the strategies of such
    // chains.
    if paragraph_count == 0
        || flow.is_some() && paragraph_count == 1
        || !height.is_finite()
        || height <= 0.0
    {
        return None;
    }
    // Columns that balance are as tall as their content needs, up to the
    // height of the container (CSS Multi-column 1, 8.1). Content that needs
    // more fills the container's height, and continues in overflow columns
    // below, so only content that fits is balanced.
    if context.column_fill != ColumnFillValue::Auto
        && context.column_count > 0
        && fill(&paragraphs, context, height, false).is_some()
    {
        // The padding and border of the paragraphs are part of what one
        // column holds.
        let edges: f32 = paragraphs
            .iter()
            .flatten()
            .map(|paragraph| paragraph.edge_start + paragraph.edge_end)
            .sum();
        if let Some((group, balanced)) = balanced(&paragraphs, context, ceiling + edges)
            && let Some(balanced) = balanced_below(balanced, height)
        {
            let mut group = finish_group(group.placements, balanced);
            group.flow = flow;
            return Some(group);
        }
    }
    let unbounded = FragmentationContext {
        column_count: usize::MAX,
        ..context
    };
    // A piece broken at a column's end runs to that end, as the columns
    // share the container's height.
    let group = fill(&paragraphs, unbounded, height, false)?;
    let mut group = finish_group(group.placements, height);
    group.flow = flow;
    Some(group)
}

/// The paragraphs of `entries` with their line extents, when each entry is a
/// plain paragraph. With `atomics`, a paragraph may hold atomic inlines, which
/// go to the column of their line. The row and page streams place only lines,
/// so they pass `false` and leave such a paragraph to the other strategies.
/// With `edges` sliced, a paragraph may have padding and border above its
/// first line and below its last, which its pieces slice; only a stream that
/// places those edges with the lines passes that. With `boxes`, a sized box with
/// nothing inside it may stand between the paragraphs; a column holds it
/// whole, and only a stream that fills definite-height columns passes that.
fn collect(
    tree: &LayoutSession<'_>,
    entries: &[Measurement],
    roles: Option<&[Role]>,
    atomics: bool,
    edges: BlockEdges,
    boxes: bool,
) -> Option<(Vec<Option<Paragraph>>, usize, bool, f32)> {
    let mut paragraphs = Vec::with_capacity(entries.len());
    let mut paragraph_count = 0;
    let mut has_empty = false;
    let mut high = 0.0;
    for (index, &(child, ref output, layout, margin_top, margin_bottom, _)) in
        entries.iter().enumerate()
    {
        // The start and the end of a wrapper take no room; only their margins
        // count, collapsing through them with the ones around them.
        if roles.is_some_and(|roles| roles[index] != Role::Box) {
            (margin_top.is_finite() && margin_bottom.is_finite()).then_some(())?;
            high += margin_top.abs() + margin_bottom.abs();
            paragraphs.push(Some(Paragraph {
                extents: Vec::new(),
                margin_top,
                margin_bottom,
                orphans: 1,
                widows: 1,
                edge_start: 0.0,
                edge_end: 0.0,
                held_lines: None,
            }));
            continue;
        }
        let node = layout_node(tree, child);
        // Source whitespace between block elements has no formatting box.
        if node.kind() == NodeKind::Text && output.size.height == 0.0 && node.ifc.is_none() {
            paragraphs.push(None);
            continue;
        }
        // A box with no lines has nowhere to break, so its block size is the
        // one extent of a one-line paragraph: it goes whole to a column that
        // holds it, and the plan fails when none does.
        if boxes && output.size.height > 0.0 && lineless::is_sized_leaf(tree, child) {
            high += output.size.height + margin_top.abs() + margin_bottom.abs();
            paragraphs.push(Some(Paragraph::whole_box(
                output.size.height,
                margin_top,
                margin_bottom,
                None,
            )));
            continue;
        }
        // A box of lines with a height of its own goes whole to a column that
        // holds it, as a box with no lines does, and is cut nowhere. Its lines
        // must lie inside its height, or they would reach past the column.
        if boxes && output.size.height > 0.0 && is_sized_paragraph(tree, child) {
            let lines = node.ifc.as_ref()?.lines.as_ref()?;
            let (before, after) = block_edges(&layout);
            let reach = lines
                .lines
                .last()
                .map(|line| line.block_offset() + line.block_size())?;
            if node.ifc_writing_mode() != Some(shodo::geometry::WritingMode::HorizontalTb)
                || !paragraph_boxes_are_atomic(tree, child)
                || multicol_subtree_has_float(tree, child)
                || node.has_before_or_after_content
                || !reach.is_finite()
                || before + reach + after > output.size.height + f32::EPSILON
                || !margin_top.is_finite()
                || !margin_bottom.is_finite()
            {
                return None;
            }
            high += output.size.height + margin_top.abs() + margin_bottom.abs();
            paragraphs.push(Some(Paragraph::whole_box(
                output.size.height,
                margin_top,
                margin_bottom,
                Some(lines.lines.len()),
            )));
            // The plan needs a paragraph, and this box holds lines, unlike one
            // with none.
            paragraph_count += 1;
            continue;
        }
        let empty = output.size.height == 0.0
            && node.ifc.as_ref().is_none_or(|root| {
                root.lines
                    .as_ref()
                    .is_none_or(|lines| lines.lines.is_empty())
            })
            && tree
                .children_through_contents(child)
                .into_iter()
                .all(|child| {
                    tree.nodes[child].kind() == NodeKind::Text
                        && tree.nodes[child].layout.size.height == 0.0
                })
            && node
                .authored_writing_mode
                .is_none_or(|mode| !mode.is_vertical())
            && {
                let overflow = style_of(tree, child).overflow();
                overflow.x == Overflow::Visible && overflow.y == Overflow::Visible
            };
        if !can_balance_paragraph_box(tree, child, edges)
            || (!empty
                && !if atomics {
                    can_balance_atomic_paragraph(tree, child, edges)
                } else {
                    can_balance_single_paragraph(tree, child)
                })
            || !matches!(node.display(), DisplayValue::Block | DisplayValue::ListItem)
            || is_right_aligned_in_parent(tree, child)
            || node.break_before != BreakBetween::Auto
            || node.break_after != BreakBetween::Auto
            || node.has_before_or_after_content
            || multicol_subtree_has_float(tree, child)
            || !margin_top.is_finite()
            || !margin_bottom.is_finite()
        {
            return None;
        }
        if empty {
            has_empty = true;
            high += margin_top.abs() + margin_bottom.abs();
            paragraphs.push(Some(Paragraph {
                extents: Vec::new(),
                margin_top,
                margin_bottom,
                orphans: node.paragraph_orphans,
                widows: node.paragraph_widows,
                edge_start: 0.0,
                edge_end: 0.0,
                held_lines: None,
            }));
            continue;
        }
        let lines = node.ifc.as_ref()?.lines.as_ref()?;
        let extents: Vec<_> = lines
            .lines
            .iter()
            .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
            .collect();
        let first = extents.first()?;
        let last = extents.last()?;
        let height = last.1 - first.0;
        if !height.is_finite() || height <= 0.0 {
            return None;
        }
        high += height + margin_top.abs() + margin_bottom.abs();
        let (edge_start, edge_end) = block_edges(&layout);
        paragraphs.push(Some(Paragraph {
            extents,
            margin_top,
            margin_bottom,
            orphans: node.paragraph_orphans,
            widows: node.paragraph_widows,
            edge_start,
            edge_end,
            held_lines: None,
        }));
        paragraph_count += 1;
    }
    Some((paragraphs, paragraph_count, has_empty, high))
}

#[derive(Clone, Copy, Default)]
struct Margins {
    positive: f32,
    negative: f32,
}

impl Margins {
    fn with(self, value: f32) -> Self {
        Self {
            positive: self.positive.max(value),
            negative: self.negative.min(value),
        }
    }

    fn value(self) -> f32 {
        self.positive + self.negative
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct FlowCursor {
    paragraph: usize,
    line: usize,
}

struct Chunk {
    /// Placements start at the resumed paragraph, so a page of a long group
    /// costs that page's paragraphs rather than the whole group's.
    group: Group,
    next: Option<FlowCursor>,
}

fn fill(
    paragraphs: &[Option<Paragraph>],
    context: FragmentationContext,
    height: f32,
    trailing_margin: bool,
) -> Option<Group> {
    let chunk = fill_chunk(
        paragraphs,
        context,
        height,
        FlowCursor::default(),
        trailing_margin,
    )?;
    chunk.next.is_none().then_some(chunk.group)
}

/// Fill columns of `height` from `resume`. The margin below the last
/// paragraph adds to the height used when `trailing_margin` is set; without
/// it, the margin ends with the content, as it does at a break.
fn fill_chunk(
    paragraphs: &[Option<Paragraph>],
    context: FragmentationContext,
    height: f32,
    resume: FlowCursor,
    trailing_margin: bool,
) -> Option<Chunk> {
    let mut placements: Vec<Option<Placement>> = Vec::new();
    let mut column = 0usize;
    let mut cursor = 0.0f32;
    let mut pending_margin = Margins::default();
    let mut maximum = 0.0f32;
    let mut column_has_content = false;
    // The margins that adjoin through empty boxes and the paragraph after them
    // were cut at a break, and stay cut until that paragraph.
    let mut cut_margins = resume != FlowCursor::default()
        && paragraphs
            .get(resume.paragraph)
            .is_some_and(|resumed| resumed.as_ref().is_some_and(|box_| box_.extents.is_empty()));
    for (paragraph_index, paragraph) in paragraphs.iter().enumerate().skip(resume.paragraph) {
        placements.push(None);
        let slot = placements.len() - 1;
        let Some(paragraph) = paragraph else {
            continue;
        };
        let (entry_column, entry_y) = (column, cursor);
        let initial_line = if paragraph_index == resume.paragraph {
            resume.line
        } else {
            0
        };
        // A break discards the margin above the paragraph it ends up before,
        // so a paragraph resumed from its start has none either.
        let at_resumed_start =
            paragraph_index == resume.paragraph && resume != FlowCursor::default();
        let mut adjoining = if cut_margins {
            Margins::default()
        } else {
            pending_margin.with(if initial_line == 0 && !at_resumed_start {
                paragraph.margin_top
            } else {
                0.0
            })
        };
        let mut top = cursor + adjoining.value();
        if paragraph.extents.is_empty() {
            // An empty box takes no room, but its margins put it below the
            // lines before it, and a place past the end of the column is an
            // unforced break: the box starts the next column, and the margins
            // adjoining through it are cut.
            if top > height + f32::EPSILON && column_has_content {
                maximum = maximum.max(cursor);
                column = column.checked_add(1)?;
                if column >= context.column_count {
                    return Some(Chunk {
                        group: finish_group(placements, height),
                        next: Some(FlowCursor {
                            paragraph: paragraph_index,
                            line: 0,
                        }),
                    });
                }
                cursor = 0.0;
                top = 0.0;
                column_has_content = false;
                cut_margins = true;
                adjoining = Margins::default();
            }
            // Retain both extrema across empty boxes: collapsing their summed
            // margins again would lose the negative member of the chain.
            pending_margin = if cut_margins {
                Margins::default()
            } else {
                adjoining.with(paragraph.margin_bottom)
            };
            placements[slot] = Some(Placement {
                first_column: column,
                first_y: top,
                first_height: 0.0,
                last_column: column,
                last_y: cursor + pending_margin.value(),
                fragments: Vec::new(),
                entry_column,
                entry_y,
            });
            continue;
        }
        cut_margins = false;
        let mut start = initial_line;
        let mut fragments = Vec::new();
        let mut first_column = column;
        let mut first_y = top;
        let mut first_height = 0.0;
        let line_count = paragraph.extents.len();
        while start < line_count {
            // The padding and border above the first line sit in the piece
            // that holds it, and those below the last line in the piece that
            // holds that.
            let lead = if start == 0 {
                paragraph.edge_start
            } else {
                0.0
            };
            let origin = paragraph.extents[start].0;
            let available = height - top;
            let mut end = start
                + paragraph.extents[start..]
                    .partition_point(|extent| extent.1 - origin + lead <= available + f32::EPSILON);
            // The last line stays with the edge below it: when the edge does
            // not fit after it, the line goes to the next column, to be
            // followed there by the edge, so that the paragraph does not
            // break between its last line and its bottom padding or border.
            if end == line_count
                && paragraph.extents[line_count - 1].1 - origin + lead + paragraph.edge_end
                    > available + f32::EPSILON
            {
                end -= 1;
            }
            let remaining = line_count - end;
            if remaining > 0 {
                let fitting = end;
                end = end.min(paragraph.widows_cut());
                let minimum = if start == 0 {
                    paragraph.orphans
                } else {
                    paragraph.orphans.max(paragraph.widows)
                };
                if end.saturating_sub(start) < minimum {
                    // A paragraph with padding or border has no other path
                    // that places its edges, so in a column with nothing
                    // else the lines that fit stay and orphans and widows
                    // yield, as no later column would do better.
                    let decorated = paragraph.edge_start > 0.0 || paragraph.edge_end > 0.0;
                    end = if decorated && !column_has_content && fitting > start {
                        fitting
                    } else {
                        relaxed_piece_end(paragraph, start, fitting, height).unwrap_or(start)
                    };
                }
            }
            if end <= start {
                // An unforced break at a block boundary discards both adjoining
                // margins. A too-tall paragraph at an empty column fails this trial.
                if start != 0 || !column_has_content {
                    return None;
                }
                maximum = maximum.max(cursor);
                column = column.checked_add(1)?;
                if column >= context.column_count {
                    return Some(Chunk {
                        group: finish_group(placements, height),
                        next: Some(FlowCursor {
                            paragraph: paragraph_index,
                            line: start,
                        }),
                    });
                }
                top = 0.0;
                column_has_content = false;
                first_column = column;
                first_y = top;
                continue;
            }
            let trail = if end == line_count {
                paragraph.edge_end
            } else {
                0.0
            };
            let used = paragraph.extents[end - 1].1 - origin + lead + trail;
            if fragments.is_empty() {
                first_height = used;
            }
            fragments.push(MulticolTextFragment {
                line_start: start,
                line_end: paragraph.held_lines.unwrap_or(end),
                fragmentainer: column,
                x: context.column_offset_x(column) - context.column_offset_x(first_column),
                // Lines are measured from the content box, which lies below
                // the edge above them in the piece that holds the first line
                // only.
                y: top
                    - first_y
                    - if start == 0 {
                        0.0
                    } else {
                        paragraph.edge_start
                    },
            });
            cursor = top + used;
            column_has_content = true;
            maximum = maximum.max(cursor);
            start = end;
            if start < line_count {
                column = column.checked_add(1)?;
                if column >= context.column_count {
                    placements[slot] = Some(Placement {
                        first_column,
                        first_y,
                        first_height,
                        last_column: column - 1,
                        last_y: cursor,
                        fragments,
                        entry_column,
                        entry_y,
                    });
                    return Some(Chunk {
                        group: finish_group(placements, height),
                        next: Some(FlowCursor {
                            paragraph: paragraph_index,
                            line: start,
                        }),
                    });
                }
                top = 0.0;
                column_has_content = false;
            }
        }
        pending_margin = Margins::default().with(paragraph.margin_bottom);
        placements[slot] = Some(Placement {
            first_column,
            first_y,
            first_height,
            last_column: column,
            last_y: cursor + pending_margin.value(),
            fragments,
            entry_column,
            entry_y,
        });
    }
    let used = maximum.max(if trailing_margin {
        cursor + pending_margin.value()
    } else {
        cursor
    });
    (used <= height + f32::EPSILON).then(|| Chunk {
        group: finish_group(placements, used),
        next: None,
    })
}

/// Where a piece that starts at line `start` ends when orphans and widows
/// leave it too short, `fitting` lines being all the column holds. Orphans
/// win over widows: the piece keeps at least `orphans` lines, and the break
/// moves up toward the widows only that far. A paragraph whose orphans and
/// widows cannot both be honored thus splits where its lines allow instead of
/// moving whole to the next column. `None` when the lines that fit do not hold
/// the orphans.
///
/// A continued piece ignores widows that no break inside it can honor: the
/// last lines they ask for begin at or before its start, or do not fit one
/// column together.
fn relaxed_piece_end(
    paragraph: &Paragraph,
    start: usize,
    fitting: usize,
    height: f32,
) -> Option<usize> {
    let kept = start + paragraph.orphans;
    if fitting < kept {
        return None;
    }
    let cut = paragraph.widows_cut();
    // A continued piece begins at the top of its column, so the lines the
    // widows ask for fit it when they fit the column's height.
    if start > 0 && (cut <= start || !lines_fit_column(paragraph, cut, height)) {
        return Some(fitting);
    }
    Some(fitting.min(cut.max(kept)))
}

/// Whether the lines of `paragraph` from `first` on, with the edge below the
/// last, fit a column of `height` together.
fn lines_fit_column(paragraph: &Paragraph, first: usize, height: f32) -> bool {
    paragraph.extents.get(first).is_some_and(|&(top, _)| {
        paragraph.extents[paragraph.extents.len() - 1].1 - top + paragraph.edge_end
            <= height + f32::EPSILON
    })
}

fn finish_group(mut placements: Vec<Option<Placement>>, used: f32) -> Group {
    for placement in placements.iter_mut().flatten() {
        if placement.fragments.len() > 1 {
            placement.first_height = (used - placement.first_y).max(0.0);
        }
    }
    Group {
        placements,
        height: used,
        flow: None,
    }
}

/// One line range of a nested container's paragraph in a row of its
/// columns. Row `n` lies in the `n`th outer column the container reaches.
pub(super) struct RowFragment {
    /// Index of the paragraph's entry.
    pub(super) entry: usize,
    pub(super) line_start: usize,
    pub(super) line_end: usize,
    pub(super) row: usize,
    /// Column of the nested container within the row.
    pub(super) column: usize,
    /// Block offset of the fragment's border box from the row's top.
    pub(super) y: f32,
    pub(super) height: f32,
}

pub(super) struct Rows {
    pub(super) fragments: Vec<RowFragment>,
    /// Used block size of each row, from the first.
    pub(super) heights: Vec<f32>,
}

/// Why [`fill_rows`] made no rows.
#[derive(Debug, PartialEq)]
pub(super) enum RowsStop {
    /// The entries are not plain paragraphs or do not fit.
    Unsupported,
    /// The records of the rows outgrow the fragment budget.
    Exhausted,
}

/// Fill the paragraphs of a nested column container into rows of its
/// columns: the first row holds `first` of block size, every later row
/// `later`. The last row is balanced unless the container fills its columns
/// sequentially, and every row with `column-fill: balance-all`. Fails when the
/// entries are not plain paragraphs or do not fit.
///
/// Rows after the first need a box record each, and every fragment needs one;
/// `budget` is the number of records left. Planning stops as soon as the rows
/// outgrow it, so the work and memory of a plan stay within the budget and one
/// row, however many lines follow. This reports exhaustion whether or not the
/// rows past that point could have been placed, as telling the two apart
/// would take the planning the budget is there to stop.
pub(super) fn fill_rows(
    tree: &LayoutSession<'_>,
    entries: &[Measurement],
    context: FragmentationContext,
    first: f32,
    later: f32,
    budget: usize,
) -> Result<Rows, RowsStop> {
    let (paragraphs, count, _, ceiling) =
        collect(tree, entries, None, false, BlockEdges::Refused, false)
            .ok_or(RowsStop::Unsupported)?;
    if count == 0 || context.column_count == 0 {
        return Err(RowsStop::Unsupported);
    }
    let mut result = Rows {
        fragments: Vec::new(),
        heights: Vec::new(),
    };
    let mut resume = FlowCursor::default();
    loop {
        let row = result.heights.len();
        let capacity = if row == 0 { first } else { later };
        if !capacity.is_finite() || capacity <= 0.0 {
            return Err(RowsStop::Unsupported);
        }
        let mut chunk = fill_chunk(&paragraphs, context, capacity, resume, true)
            .ok_or(RowsStop::Unsupported)?;
        // `balance` balances the last row only, `balance-all` every row; a
        // balanced row keeps the content the full row would hold.
        let balance = context.column_fill == ColumnFillValue::BalanceAll
            || (context.column_fill != ColumnFillValue::Auto && chunk.next.is_none());
        let mut divided = 0.0;
        if balance {
            let next = chunk.next;
            let mut low = 0.0;
            let mut high = capacity;
            for _ in 0..32 {
                if high - low <= 0.001 {
                    break;
                }
                let trial = (low + high) * 0.5;
                if let Some(candidate) = fill_chunk(&paragraphs, context, trial, resume, false)
                    && candidate.next == next
                {
                    high = trial;
                    chunk = candidate;
                } else {
                    low = trial;
                }
            }
            // The last row is as tall as what remains in one column divided
            // among the columns, if that is more than the least height that
            // holds it; the margin below its last paragraph is part of what is
            // divided, and does not have to fit.
            if next.is_none() {
                divided = divided_height(&paragraphs, context, resume, ceiling)
                    .unwrap_or(0.0)
                    .min(capacity);
                if divided > chunk.group.height
                    && let Some(candidate) =
                        fill_chunk(&paragraphs, context, divided, resume, false)
                    && candidate.next.is_none()
                {
                    chunk = candidate;
                }
            }
        }
        let height = if chunk.next.is_some() && !balance {
            capacity
        } else {
            chunk.group.height.max(divided)
        };
        for (offset, placement) in chunk.group.placements.into_iter().enumerate() {
            let entry = resume.paragraph + offset;
            let (Some(placement), Some(paragraph)) = (placement, &paragraphs[entry]) else {
                continue;
            };
            for range in placement.fragments {
                let y = placement.first_y + range.y;
                let natural =
                    paragraph.extents[range.line_end - 1].1 - paragraph.extents[range.line_start].0;
                result.fragments.push(RowFragment {
                    entry,
                    line_start: range.line_start,
                    line_end: range.line_end,
                    row,
                    column: range.fragmentainer,
                    y,
                    height: if range.line_end < paragraph.extents.len() {
                        (height - y).max(0.0)
                    } else {
                        natural
                    },
                });
            }
        }
        result.heights.push(height);
        // One row needs no records, as the container keeps its own. Longer
        // plans need a box record for each row after the first and one for
        // each fragment, and at least one more fragment for every row still
        // to come.
        let still_to_come = usize::from(chunk.next.is_some());
        let rows = result.heights.len() + still_to_come;
        if rows > 1 && rows - 1 + result.fragments.len() + still_to_come > budget {
            return Err(RowsStop::Exhausted);
        }
        let Some(next) = chunk.next else {
            return Ok(result);
        };
        // A continuation only follows an advanced line or paragraph.
        debug_assert!(next != resume);
        resume = next;
    }
}

/// The existing finite allowance beyond the outer block edges of a
/// paragraph. The inline axis is represented separately and has no limit.
pub(super) const BLOCK_INK_REACH: f32 = 1.0e5;

/// The column's block cut with an unbounded inline axis.
pub(super) fn column_window(
    rect: crate::fragment::FragmentRect,
    context: FragmentationContext,
    height: f32,
) -> raikiri_traits::PaintClip {
    inline_open_clip(crate::fragment::FragmentRect {
        x: rect.x,
        y: context.origin_y,
        width: rect.width,
        height,
    })
}

pub(super) fn inline_open_clip(rect: crate::fragment::FragmentRect) -> raikiri_traits::PaintClip {
    let mut clip: raikiri_traits::PaintClip = rect.into();
    clip.clip_x = false;
    clip
}

pub(super) struct PagedFragment {
    pub(super) child: usize,
    /// The index of the child's entry in the flow.
    entry: usize,
    /// The wrapper that holds the child, if one does.
    pub(super) wrapper: Option<usize>,
    pub(super) line_start: usize,
    pub(super) line_end: usize,
    pub(super) fragmentainer: usize,
    pub(super) rect: crate::fragment::FragmentRect,
    /// The cut of the column, or `None` for a fragment that holds its whole
    /// paragraph, which has no neighbour to keep out.
    pub(super) clip: Option<raikiri_traits::PaintClip>,
}

/// A wrapper whose boxes joined the flow of a container filled page by page,
/// by its border box from the top of its first piece to the bottom of its
/// lowest, in the container's space.
pub(super) struct PagedWrapper {
    pub(super) node: usize,
    /// The wrapper that holds it, if one does.
    pub(super) wrapper: Option<usize>,
    pub(super) rect: crate::fragment::FragmentRect,
}

pub(super) struct PagedGroup {
    /// The wrappers, each before the boxes and wrappers it holds.
    pub(super) wrappers: Vec<PagedWrapper>,
    pub(super) fragments: Vec<PagedFragment>,
    pub(super) groups: Vec<crate::fragment::MulticolGroup>,
    pub(super) end: f32,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paginate(
    tree: &LayoutSession<'_>,
    entries: &[Measurement],
    context: FragmentationContext,
    owner_y: f32,
    page_for: &impl Fn(f32) -> (u32, f32, f32),
    work: &mut ProjectionWork<'_, '_>,
) -> Result<Option<PagedGroup>, LayoutError> {
    let line_count = entries
        .iter()
        .map(|(child, ..)| {
            tree.nodes[*child]
                .ifc
                .as_ref()
                .and_then(|root| root.lines.as_ref())
                .map_or(0, |lines| lines.lines.len())
        })
        .sum::<usize>();
    work.charge(entries.len().saturating_add(line_count))?;
    let flow = Flow::of(tree, entries).unwrap_or_else(|| Flow::plain(entries));
    // The lines of the paragraphs inside wrappers are charged here, as those
    // of the container's own are above.
    let wrapped_lines = flow
        .entries
        .iter()
        .zip(&flow.parents)
        .filter(|(_, parent)| parent.is_some())
        .map(|((child, ..), _)| {
            tree.nodes[*child]
                .ifc
                .as_ref()
                .and_then(|root| root.lines.as_ref())
                .map_or(0, |lines| lines.lines.len())
        })
        .sum::<usize>();
    work.charge(flow.entries.len().saturating_add(wrapped_lines))?;
    let entries = &flow.entries;
    let Some((paragraphs, count, _, ceiling)) = collect(
        tree,
        entries,
        Some(&flow.roles),
        false,
        BlockEdges::Refused,
        false,
    ) else {
        return Ok(None);
    };
    if count == 0 || context.column_count == 0 {
        return Ok(None);
    }
    let mut result = PagedGroup {
        wrappers: Vec::new(),
        fragments: Vec::new(),
        groups: Vec::new(),
        end: context.origin_y,
    };
    let mut resume = FlowCursor::default();
    let mut absolute_y = owner_y + context.origin_y;
    loop {
        let (page_index, page_origin, page_height) = page_for(absolute_y);
        if !work.page_available(page_index)? {
            break;
        }
        let capacity = page_origin + page_height - absolute_y;
        if !capacity.is_finite() || capacity <= 0.0 {
            return Ok(None);
        }
        // A fill visits the paragraphs from `resume` until the page's columns
        // are full; charge what it visited. Placing lines is logarithmic in
        // a paragraph's lines, and each placed range is charged below.
        let filled = fill_chunk(&paragraphs, context, capacity, resume, false);
        work.charge(visited(&paragraphs, resume, filled.as_ref()))?;
        let Some(mut chunk) = filled else {
            // A short remainder can contain fewer lines than the paragraph's
            // break minima. Retry at the next complete page, without a soft
            // break inside the paragraph or a zero-progress page loop.
            if absolute_y > page_origin + 0.001 {
                absolute_y = page_origin + page_height;
                continue;
            }
            return Ok(None);
        };
        let mut divided = 0.0;
        if chunk.next.is_none() && context.column_fill != ColumnFillValue::Auto {
            let mut low = 0.0;
            let mut high = capacity;
            for _ in 0..32 {
                if high - low <= 0.001 {
                    break;
                }
                let trial = (low + high) * 0.5;
                let filled = fill_chunk(&paragraphs, context, trial, resume, false);
                work.charge(visited(&paragraphs, resume, filled.as_ref()))?;
                if let Some(candidate) = filled
                    && candidate.next.is_none()
                {
                    high = trial;
                    chunk = candidate;
                } else {
                    low = trial;
                }
            }
            // The last page's columns are as tall as what remains in one
            // column divided among them, if that is more than the least
            // height that holds it.
            work.charge(paragraphs.len().saturating_sub(resume.paragraph) + 1)?;
            divided = divided_height(&paragraphs, context, resume, ceiling)
                .unwrap_or(0.0)
                .min(capacity);
            if divided > chunk.group.height {
                let filled = fill_chunk(&paragraphs, context, divided, resume, false);
                work.charge(visited(&paragraphs, resume, filled.as_ref()))?;
                if let Some(candidate) = filled
                    && candidate.next.is_none()
                {
                    chunk = candidate;
                }
            }
        }
        let height = if chunk.next.is_some() {
            capacity
        } else {
            chunk.group.height.max(divided)
        };
        let group_y = absolute_y - owner_y;
        let mut occupied = std::collections::BTreeSet::new();
        for (offset, placement) in chunk.group.placements.into_iter().enumerate() {
            let index = resume.paragraph + offset;
            let Some(placement) = placement else {
                continue;
            };
            let Some(paragraph) = &paragraphs[index] else {
                continue; // cov:ignore: fill_chunk only creates placements for present paragraphs.
            };
            for range in placement.fragments {
                work.charge(1)?;
                let y = placement.first_y + range.y;
                let natural_height =
                    paragraph.extents[range.line_end - 1].1 - paragraph.extents[range.line_start].0;
                let box_height = if range.line_end < paragraph.extents.len() {
                    (height - y).max(0.0)
                } else {
                    natural_height
                };
                let column = range.fragmentainer;
                let fragmentainer = (page_index as usize)
                    .checked_mul(context.column_count)
                    .and_then(|first| first.checked_add(column))
                    .ok_or(LayoutError::FragmentLimitExceeded {
                        limit: tree.fragment_tree.limit,
                    })?;
                let x = context.origin_x + context.column_offset_x(column);
                occupied.insert(column);
                let rect = crate::fragment::FragmentRect {
                    x: x + flow.insets[index] + entries[index].2.margin.left,
                    y: group_y + y,
                    width: entries[index].2.size.width,
                    height: box_height,
                };
                result.fragments.push(PagedFragment {
                    child: entries[index].0,
                    entry: index,
                    wrapper: flow.parents[index].map(|open| entries[open].0),
                    line_start: range.line_start,
                    line_end: range.line_end,
                    fragmentainer,
                    rect,
                    // The clip cuts a fragment at the block edges of its
                    // column, so the lines of its neighbours stay out, and
                    // leaves the inline axis open. The edge where the
                    // paragraph begins or ends has no neighbour behind it,
                    // so what the box paints there, an outline for one,
                    // stays visible too.
                    clip: (range.line_start > 0 || range.line_end < paragraph.extents.len()).then(
                        || {
                            let top = if range.line_start == 0 {
                                group_y - BLOCK_INK_REACH
                            } else {
                                group_y
                            };
                            let bottom = if range.line_end >= paragraph.extents.len() {
                                group_y + height + BLOCK_INK_REACH
                            } else {
                                group_y + height
                            };
                            inline_open_clip(crate::fragment::FragmentRect {
                                x,
                                y: top,
                                width: context.column_width,
                                height: bottom - top,
                            })
                        },
                    ),
                });
            }
        }
        result.groups.push(crate::fragment::MulticolGroup {
            context: FragmentationContext {
                origin_y: group_y,
                available_height: Some(height),
                ..context
            },
            height,
            slice: None,
            occupied,
        });
        result.end = group_y + height;
        let Some(next) = chunk.next else {
            break;
        };
        if next == resume {
            return Ok(None); // cov:ignore: fill_chunk returns a continuation only after advancing a line or a paragraph.
        }
        resume = next;
        absolute_y = page_origin + page_height;
    }
    result.wrappers = paged_wrappers(&flow, &result.fragments);
    work.charge(result.wrappers.len())?;
    Ok(Some(result))
}

/// The border boxes of the wrappers of `flow`, from the pieces of the boxes
/// they hold, which are in flow order. A wrapper whose boxes have no piece
/// has none.
fn paged_wrappers(flow: &Flow, fragments: &[PagedFragment]) -> Vec<PagedWrapper> {
    let wrapper = |(open, role): (usize, &Role)| {
        let Role::Open(close) = *role else {
            return None;
        };
        let start = fragments.partition_point(|fragment| fragment.entry < open);
        let end = fragments.partition_point(|fragment| fragment.entry < close);
        let first = fragments[start..end].first()?;
        let (layout, first_layout) = (flow.entries[open].2, flow.entries[first.entry].2);
        let bottom = fragments[start..end]
            .iter()
            .map(|fragment| fragment.rect.y + fragment.rect.height)
            .fold(first.rect.y, f32::max);
        // The pieces start at the left of a column, past the insets of their
        // wrappers.
        let x = first.rect.x - flow.insets[first.entry] - first_layout.margin.left
            + flow.insets[open]
            + layout.margin.left;
        Some(PagedWrapper {
            node: flow.entries[open].0,
            wrapper: flow.parents[open].map(|parent| flow.entries[parent].0),
            rect: crate::fragment::FragmentRect {
                x,
                y: first.rect.y,
                width: layout.size.width,
                height: bottom - first.rect.y,
            },
        })
    };
    flow.roles.iter().enumerate().filter_map(wrapper).collect()
}

/// Paragraphs a fill from `resume` visited. A failed fill stops anywhere up
/// to the end of the group, so it is charged as if it visited the rest.
fn visited(paragraphs: &[Option<Paragraph>], resume: FlowCursor, chunk: Option<&Chunk>) -> usize {
    chunk.map_or(paragraphs.len().saturating_sub(resume.paragraph), |chunk| {
        chunk.group.placements.len()
    }) + 1
}
