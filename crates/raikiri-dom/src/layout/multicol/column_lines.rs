//! The breaks between the lines of an absolutely positioned box that the
//! columns of its containing block cut.
//!
//! A column end inside such a box fragments its content (CSS Position 3,
//! 3.3.4; CSS Fragmentation 3, 4.4): a line that the end would cut starts the
//! next column instead, the boxes after it in the flow of the box move down
//! with it, and the boxes around it grow to hold the room this leaves, as
//! Chromium does. The pieces of the box then paint it whole, moved into each
//! column and clipped to it, so no column paints part of a line.

use super::lineless::EPSILON;
use crate::layout::context::LayoutSession;
use crate::layout::ifc::boxes::IfcBoxKind;
use crate::layout::ifc::projection::{ATOMIC_TAGS, REPLACED_BOX_TAGS};
use crate::node::FormattingContext;
use raikiri_layout_geometry::BoxLayout;
use raikiri_style::property::{DisplayValue, PositionValue};
use raikiri_traits::NodeKind;

/// How deep the flow of a box is followed into the blocks it holds.
const MAX_DEPTH: usize = 64;

/// How the content of a box moves down at the column ends that cross its
/// lines.
#[derive(Debug, Default)]
pub(super) struct LineMoves {
    /// Each paragraph whose lines move, with each line that starts a column
    /// and how far it and the lines after it move down.
    lines: Vec<(usize, Vec<(usize, f32)>)>,
    /// Each block that the moved lines before it move down, with how far.
    blocks: Vec<(usize, f32)>,
    /// Each box inside it whose content moves, with how far its border box
    /// grows and how far the content that reaches lowest in it moves down.
    grown: Vec<(usize, f32, f32)>,
    /// The same for the box itself.
    own: (f32, f32),
}

impl LineMoves {
    /// The layout of the box laid out as `layout` once its content has moved:
    /// its border box grows, and the content past it moves down.
    pub(super) fn moved(&self, mut layout: BoxLayout) -> BoxLayout {
        let (growth, moved) = self.own;
        layout.size.height += growth;
        layout.scrollable_overflow_rect.bottom += moved;
        layout
    }
}

/// How the content of the absolutely positioned box `node` moves so that no
/// end of the columns, each `height` tall, cuts one of its lines: a line that
/// a column end crosses, below the start of its column, starts the next one.
/// `top` is where the border box starts in the columns laid end to end, from
/// the start of one of them, and the column `last`, counted from that one,
/// holds the rest. `None` when nothing moves, or when the box holds anything
/// but blocks and their lines, with no float and no positioned box among
/// them, which need more of the flow broken.
pub(super) fn line_moves(
    tree: &LayoutSession<'_>,
    node: usize,
    top: f32,
    height: f32,
    last: Option<usize>,
) -> Option<LineMoves> {
    if !(height > 0.0 && top.is_finite()) || scrolls(tree, node) {
        return None;
    }
    let columns = Columns { height, last };
    let mut moves = LineMoves::default();
    moves.own = plan(tree, node, top, columns, &mut moves, 0)?;
    (!(moves.lines.is_empty() && moves.blocks.is_empty())).then_some(moves)
}

#[derive(Clone, Copy)]
struct Columns {
    height: f32,
    last: Option<usize>,
}

/// Plan the moves inside `node`, whose border box starts at `top`: how far
/// its border box grows, and how far the content that reaches lowest in it
/// moves down. `None` when its content holds what cannot follow the moves.
fn plan(
    tree: &LayoutSession<'_>,
    node: usize,
    top: f32,
    columns: Columns,
    moves: &mut LineMoves,
    depth: usize,
) -> Option<(f32, f32)> {
    let element = tree.ifc_layout_node(node)?;
    if depth > MAX_DEPTH
        || element.used_writing_mode.is_vertical()
        || element.size_containment.height
    {
        return None;
    }
    let layout = element.layout;
    let content_top = top + layout.border.top + layout.padding.top;
    // How far the end of the content moves down, and how far the content
    // that reaches lowest does.
    let (moved, reach) = match element.formatting_context {
        FormattingContext::Inline => {
            let lines = paragraph_moves(tree, node, content_top, columns)?;
            let moved: f32 = lines.iter().map(|&(_, shift)| shift).sum();
            if !lines.is_empty() {
                moves.lines.push((node, lines));
            }
            (moved, moved)
        }
        FormattingContext::Block => {
            let (mut moved, mut reach) = (0.0f32, 0.0f32);
            for child in tree.children_through_contents(node) {
                let child_node = &tree.nodes[child];
                if child_node.kind() != NodeKind::Element || child_node.is_display_none() {
                    continue;
                }
                let mut child_top = top + child_node.layout.location.y + moved;
                let mut delta = moved;
                let child_kind = kind(tree, child, columns);
                if child_kind == Kind::Other {
                    return None;
                }
                // A box whose first part that a column end does not break
                // starts the next column goes there with it, from the start
                // of the column that part starts in.
                if let Some(shift) = leading(tree, child, child_top, columns, depth + 1)
                    .filter(|&(start, size)| {
                        crossing(start, size, columns).is_some()
                            && column_of(start, columns) == column_of(child_top, columns)
                    })
                    .map(|_| (column_of(child_top, columns) + 1.0) * columns.height - child_top)
                {
                    child_top += shift;
                    delta += shift;
                    moved += shift;
                }
                if delta > 0.0 {
                    moves.blocks.push((child, delta));
                }
                let mut child_reach = 0.0;
                if child_kind == Kind::Plain {
                    let (growth, inner) = plan(tree, child, child_top, columns, moves, depth + 1)?;
                    moved += growth;
                    child_reach = inner;
                }
                reach = reach.max(moved).max(delta + child_reach);
            }
            (moved, reach)
        }
        _ => return None,
    };
    let growth = if moved > 0.0 {
        growth(tree, node, moved, depth == 0)
    } else {
        0.0
    };
    if depth > 0 && reach > 0.0 {
        moves.grown.push((node, growth, reach));
    }
    Some((growth, reach))
}

/// How far the border box of `node` grows when the end of its content moves
/// down by `moved`: as far as its height follows its content, between its
/// `min-height` and `max-height` (CSS 2.1, 10.7). A positioned box whose
/// insets size it keeps its height (CSS 2.1, 10.6.4). `root` tells whether
/// `node` is the positioned box itself.
fn growth(tree: &LayoutSession<'_>, node: usize, moved: f32, root: bool) -> f32 {
    use crate::layout::page_pipeline as pages;
    let style = super::style_of(tree, node);
    let inset = style.inset();
    if root && !inset.top.is_auto() && !inset.bottom.is_auto() {
        return 0.0;
    }
    let element = &tree.nodes[node];
    if element.formatting_context == FormattingContext::Inline {
        return pages::lines_shift_growth(tree, node, moved);
    }
    if !pages::lines_move_grows_box(tree, node) {
        return 0.0;
    }
    let layout = element.layout;
    // The content ends where the lowest of its blocks does.
    let bottom = tree
        .children_through_contents(node)
        .into_iter()
        .filter(|&child| tree.nodes[child].kind() == NodeKind::Element)
        .map(|child| {
            let child = tree.nodes[child].layout;
            child.location.y + child.size.height
        })
        .fold(layout.border.top + layout.padding.top, f32::max);
    let (height, _) = pages::grown_height(
        layout.size.height,
        pages::border_box_height_bounds(tree, node),
        layout.padding.bottom + layout.border.bottom,
        (bottom, bottom + moved),
    );
    height - layout.size.height
}

/// How the moves treat a box in the flow of a block.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A block container, whose content the moves follow.
    Plain,
    /// A block that the moves keep whole: a scroll container, a replaced
    /// element, or a box laid out by another algorithm that fits in a
    /// column.
    Monolithic,
    /// A float, a positioned box, an inline-level box, or a box laid out by
    /// another algorithm that is taller than a column, whose content the
    /// moves cannot follow.
    Other,
}

fn kind(tree: &LayoutSession<'_>, node: usize, columns: Columns) -> Kind {
    let element = &tree.nodes[node];
    if !matches!(
        element.position(),
        PositionValue::Static | PositionValue::Relative
    ) || super::style_of(tree, node).float().is_floated()
        || element.used_writing_mode.is_vertical()
    {
        return Kind::Other;
    }
    let fits = element.layout.size.height <= columns.height + EPSILON;
    let block = matches!(
        element.display(),
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
    );
    let own_layout = matches!(element.display(), DisplayValue::Flex | DisplayValue::Grid)
        || element.formatting_context == FormattingContext::Table
        || (block && element.multicol.is_some());
    let monolithic = block
        && (scrolls(tree, node)
            || element
                .tag_name()
                .is_some_and(|tag| REPLACED_BOX_TAGS.contains(&tag) || ATOMIC_TAGS.contains(&tag)));
    let flow = matches!(
        element.formatting_context,
        FormattingContext::Block | FormattingContext::Inline
    );
    match (monolithic, own_layout, block && flow) {
        (true, _, _) => Kind::Monolithic,
        (_, true, _) if fits => Kind::Monolithic,
        (_, false, true) => Kind::Plain,
        _ => Kind::Other,
    }
}

/// Whether `node` is a scroll container, whose content stays inside it.
fn scrolls(tree: &LayoutSession<'_>, node: usize) -> bool {
    use raikiri_layout_style::values::Overflow;
    let overflow = super::style_of(tree, node).overflow();
    [overflow.x, overflow.y]
        .into_iter()
        .any(|overflow| !matches!(overflow, Overflow::Visible | Overflow::Clip))
}

/// Where the first part of the flow of the box `node`, whose border box
/// starts at `top`, that a column end does not break starts, and how tall
/// it is: its first line, or a box the moves keep whole. `None` when it has
/// none, or when it is in a box whose content the moves cannot follow.
fn leading(
    tree: &LayoutSession<'_>,
    node: usize,
    top: f32,
    columns: Columns,
    depth: usize,
) -> Option<(f32, f32)> {
    let element = tree.ifc_layout_node(node)?;
    let layout = element.layout;
    match kind(tree, node, columns) {
        Kind::Monolithic => return Some((top, layout.size.height)),
        Kind::Other => return None,
        Kind::Plain => {}
    }
    let content_top = top + layout.border.top + layout.padding.top;
    if element.formatting_context == FormattingContext::Inline {
        let line = element.ifc.as_ref()?.lines.as_ref()?.lines.first()?;
        return Some((content_top + line.block_offset(), line.block_size()));
    }
    let child = tree
        .children_through_contents(node)
        .into_iter()
        .find(|&child| {
            tree.nodes[child].kind() == NodeKind::Element && !tree.nodes[child].is_display_none()
        })?;
    if depth >= MAX_DEPTH {
        return None;
    }
    leading(
        tree,
        child,
        top + tree.nodes[child].layout.location.y,
        columns,
        depth + 1,
    )
}

/// The index of the column that `offset` falls in, as a float.
fn column_of(offset: f32, columns: Columns) -> f32 {
    ((offset + EPSILON) / columns.height).floor().max(0.0)
}

/// How far a box that starts at `start` and is `size` tall moves down to
/// start the next column, when the end of the column it starts in crosses
/// it below the column's start. The last column holds the rest.
fn crossing(start: f32, size: f32, columns: Columns) -> Option<f32> {
    let height = columns.height;
    let column = column_of(start, columns);
    if columns.last.is_some_and(|last| column as usize >= last) {
        return None;
    }
    let column_start = column * height;
    let column_end = column_start + height;
    (start + size > column_end + EPSILON && start > column_start + EPSILON)
        .then_some(column_end - start)
}

/// Each line of the paragraph `node`, whose content box starts at
/// `content_top`, that starts a column, with how far it and the lines after
/// it move down. `None` when its lines already moved, or when they hold
/// blocks, floats or positioned boxes.
fn paragraph_moves(
    tree: &LayoutSession<'_>,
    node: usize,
    content_top: f32,
    columns: Columns,
) -> Option<Vec<(usize, f32)>> {
    let root = tree.ifc_layout_node(node)?.ifc.as_ref()?;
    let lines = root.lines.as_ref()?;
    // Lines that pagination or the columns already moved are left there: a
    // layout that keeps the box from an earlier one keeps its moves too.
    if !lines.shifts.is_empty()
        || root.multicol_fragments.is_some()
        || lines.fragmentainer_line_ranges.is_some()
        || !lines.block_line_starts.is_empty()
        || !lines.placed_blocks.is_empty()
        || root
            .boxes
            .iter()
            .any(|item| matches!(item.kind, IfcBoxKind::Float | IfcBoxKind::OutOfFlow))
    {
        return None;
    }
    let mut moves = Vec::new();
    let mut moved = 0.0f32;
    for (index, line) in lines.lines.iter().enumerate() {
        let start = content_top + line.block_offset() + moved;
        if let Some(shift) = crossing(start, line.block_size(), columns) {
            moves.push((index, shift));
            moved += shift;
        }
    }
    Some(moves)
}

/// Move the content of `node` as `moves` says.
pub(super) fn apply(tree: &mut LayoutSession<'_>, node: usize, moves: &LineMoves) {
    for (paragraph, lines) in &moves.lines {
        // The atomic inlines on each moved line, with how far the line moves.
        let atomics = tree
            .ifc_layout_node(*paragraph)
            .and_then(|element| element.ifc.as_ref())
            .and_then(|root| root.lines.as_ref())
            .into_iter()
            .flat_map(|paragraph_lines| {
                paragraph_lines
                    .lines
                    .iter()
                    .enumerate()
                    .flat_map(move |(index, line)| {
                        let shift: f32 = lines
                            .iter()
                            .filter(|&&(first, _)| first <= index)
                            .map(|&(_, shift)| shift)
                            .sum();
                        line.fragments().filter_map(move |fragment| match fragment {
                            shodo::Fragment::Atomic(atomic) if shift > 0.0 => {
                                Some((usize::try_from(atomic.node.0).ok()?, shift))
                            }
                            _ => None,
                        })
                    })
            })
            .collect::<Vec<_>>();
        // An atomic inline in several fragments of a line moves once.
        let mut atomics = atomics;
        atomics.sort_unstable_by_key(|&(atomic, _)| atomic);
        atomics.dedup_by_key(|&mut (atomic, _)| atomic);
        for &(line, shift) in lines {
            crate::layout::page_pipeline::shift_ifc_lines_from(tree, *paragraph, line, shift, 0.0);
        }
        for (atomic, shift) in atomics {
            if atomic < tree.nodes.len() {
                tree.translate_pagination_y(atomic, shift);
            }
        }
    }
    for &(block, delta) in &moves.blocks {
        tree.translate_pagination_y(block, delta);
    }
    for &(grown, growth, moved) in &moves.grown {
        grow(tree, grown, growth, moved);
    }
    let (growth, moved) = moves.own;
    grow(tree, node, growth, moved);
}

/// Grow the border box of `node` by `growth`, with its content moved down by
/// `moved`.
fn grow(tree: &mut LayoutSession<'_>, node: usize, growth: f32, moved: f32) {
    let element = tree.table_layout_node_mut(node);
    element.layout.size.height += growth;
    element.layout.scrollable_overflow_rect.bottom += moved;
    tree.mark_pagination_extent_dirty(node);
}
