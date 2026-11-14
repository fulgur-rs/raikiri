//! Boxes that hold no lines, in the columns of a container that fills them in
//! turn, across the pages that cut the container.
//!
//! A container with `column-fill: auto` and an auto height is laid out with
//! one column that nothing limits, because the pages that cut it are not known
//! yet. Once they are, each page holds a set of columns as tall as what the
//! page has left, filled one after the other, and the container goes on at the
//! top of the next page (CSS Multi-column 1, 8.1 and 8.2; CSS Fragmentation 3,
//! 2). [`paginate`] fills those columns again from the boxes the container laid
//! out.

use super::*;
use std::cell::RefCell;

/// One piece of a child box in the columns of a page.
pub(in crate::layout::multicol) struct PagedPiece {
    pub(in crate::layout::multicol) child: usize,
    pub(in crate::layout::multicol) fragmentainer: usize,
    pub(in crate::layout::multicol) rect: FragmentRect,
    /// The cut of the column, or `None` for a box that stays in one column,
    /// which has no neighbour to keep out.
    pub(in crate::layout::multicol) clip: Option<FragmentRect>,
    /// Index of the piece among those of its child, and their count.
    pub(in crate::layout::multicol) index: usize,
    pub(in crate::layout::multicol) count: usize,
    /// The wrapper piece this one is placed in, as the wrapper and the index
    /// of its piece, whose rect `rect` and `clip` are relative to; `None` for
    /// a piece placed in the container.
    pub(in crate::layout::multicol) parent: Option<(usize, usize)>,
}

/// The pieces and the page groups of a container, and where its columns end
/// in the container's coordinates.
pub(in crate::layout::multicol) struct Paged {
    pub(in crate::layout::multicol) pieces: Vec<PagedPiece>,
    /// Each box or wrapper inside a wrapper that has pieces, with where it now
    /// stands in the wrapper.
    pub(in crate::layout::multicol) nested: Vec<(usize, Point<f32>)>,
    pub(in crate::layout::multicol) groups: Vec<MulticolGroup>,
    pub(in crate::layout::multicol) end: f32,
}

/// The block size of the columns past the pages that are planned.
const UNBOUNDED: f32 = 1.0e30;

/// A page's share of the container: the page, where the columns start on it
/// and where it ends, in the coordinates of the flow.
#[derive(Clone, Copy)]
struct Row {
    page: u32,
    top: f32,
    end: f32,
}

impl Row {
    fn capacity(self) -> f32 {
        self.end - self.top
    }
}

/// Fill the columns of a container with the boxes `children` it laid out, page
/// by page. Its content starts at `owner_y` plus `context.origin_y` in the
/// flow, and `page_for` tells the page of a position. `None` when the
/// container keeps the columns it has: a box in it must stay whole where a
/// page would cut it, or the pages do not hold it.
pub(in crate::layout::multicol) fn paginate(
    tree: &LayoutSession<'_>,
    children: &[usize],
    context: FragmentationContext,
    owner_y: f32,
    page_for: &impl Fn(f32) -> (u32, f32, f32),
    work: &mut ProjectionWork<'_, '_>,
) -> Result<Option<Paged>, LayoutError> {
    work.charge(children.len())?;
    let columns = context.column_count;
    // A wrapper puts the boxes it holds in the stream in its place, as the
    // layout of the container did, so each child has a range of items.
    let mut wrapped: Vec<Option<WrappedBlockTree>> = children
        .iter()
        .map(|&child| wrapped_blocks(tree, child, true))
        .collect();
    let mut items = Vec::new();
    let mut spans = Vec::with_capacity(children.len());
    for (&child, wrapped_tree) in children.iter().zip(&mut wrapped) {
        let first = items.len();
        let Some(wrapped_tree) = wrapped_tree else {
            let height = tree.nodes[child].layout.size.height;
            let insets = Insets::Against(context.column_width);
            items.push(item(tree, child, height, Some(insets)));
            spans.push(first..items.len());
            continue;
        };
        items.extend(wrapped_tree.leaf_ids().iter().map(|&leaf| {
            item(
                tree,
                leaf,
                tree.nodes[leaf].layout.size.height,
                Some(Insets::Laid),
            )
        }));
        // A wrapper left without a layout has no pieces below, so the
        // container keeps its columns.
        apply_wrapped_layout(tree, wrapped_tree, &mut items, first, context.column_width);
        spans.push(first..items.len());
    }
    // The container starts partway down its page, so the first set of columns
    // has what is left of it. The pages after the first are looked up as the
    // stream reaches them.
    let start = owner_y + context.origin_y;
    let (page, origin, height) = page_for(start);
    let rows = RefCell::new(vec![Row {
        page,
        top: start,
        end: origin + height,
    }]);
    let row_at = |row: usize| {
        let mut rows = rows.borrow_mut();
        while rows.len() <= row {
            let (page, top, height) = page_for(rows[rows.len() - 1].end);
            rows.push(Row {
                page,
                top,
                end: top + height,
            });
        }
        rows[row]
    };
    let max_pieces = tree
        .fragment_tree
        .limit
        .saturating_sub(tree.fragment_tree.fragments.len());
    // Past the first page the control refuses there is nothing to cut, so the
    // rows after that one have room for all that is left: the placement ends
    // there instead of planning pages that are not made, which could spend
    // the fragment budget before the limit is reached.
    let allowed_rows = work
        .page_limit()
        .map(|limit| usize::try_from(limit.saturating_sub(page)).unwrap_or(usize::MAX));
    let height_of = |column: usize| {
        let row = column / columns;
        if allowed_rows.is_some_and(|allowed| row > allowed) {
            return UNBOUNDED;
        }
        row_at(row).capacity()
    };
    let placement = match place_columns(&items, &height_of, max_pieces, usize::MAX, true) {
        Ok(placement) => placement,
        // A box that must stay whole would be cut by a page.
        Err(Stop::Cut) => return Ok(None),
        // More pieces than the fragment arena holds is a failure, as it is
        // for the pieces of every other paged container.
        Err(Stop::Exhausted) => {
            return Err(LayoutError::FragmentLimitExceeded {
                limit: tree.fragment_tree.limit,
            });
        }
    };
    // The pieces of each child: those of its box, or for a wrapper, those
    // from the first box it holds to the last, with its own insets.
    // Where each column starts in the stream, which places the pieces of the
    // wrappers.
    let starts: Vec<f32> = std::iter::once(0.0)
        .chain((0..placement.columns_used()).scan(0.0, |end, column| {
            *end += height_of(column);
            Some(*end)
        }))
        .collect();
    let grid = Columns::Starts(&starts);
    let planned = planned_child_pieces(&wrapped, &spans, &placement, grid);
    let Some((child_pieces, leaf_pieces)) = planned else {
        return Ok(None);
    };
    // A scroll container taller than the page it lands on would run past the
    // page edge with nothing to carry it on, so the content would be lost; the
    // container keeps the single column, which the pages cut.
    for (&child, pieces) in children.iter().zip(&child_pieces) {
        if is_monolithic_scroll_box(tree, child)
            && pieces
                .iter()
                .any(|piece| piece.height > row_at(piece.column / columns).capacity() + EPSILON)
        {
            return Ok(None);
        }
    }
    work.charge(placement.pieces.iter().map(Vec::len).sum::<usize>())?;
    let natural_last = (placement.columns_used().max(1) - 1) / columns;
    // Past the last page the control allows there is nothing to cut.
    let mut kept = natural_last + 1;
    for index in 0..=natural_last {
        if !work.page_available(row_at(index).page)? {
            kept = index;
            break;
        }
    }
    if kept == 0 {
        return Ok(None);
    }
    let last = kept - 1;
    // The last set of columns is as tall as its content; the others reach the
    // end of their page.
    let heights: Vec<f32> = (0..kept)
        .map(|row| {
            if row == natural_last {
                placement.extent_after(row * columns)
            } else {
                row_at(row).capacity()
            }
        })
        .collect();
    let mut occupied = vec![std::collections::BTreeSet::new(); kept];
    let tops: Vec<f32> = (0..kept).map(|row| row_at(row).top - owner_y).collect();
    let mut out = Out {
        pieces: Vec::new(),
        nested: Vec::new(),
    };
    for (order, (&child, child_pieces)) in children.iter().zip(&child_pieces).enumerate() {
        let size = tree.nodes[child].layout.size;
        let own: Vec<_> = child_pieces
            .iter()
            .filter(|piece| piece.column / columns < kept)
            .collect();
        let sliced = own.len() > 1 && slices_decorations(tree, child);
        // The pieces of a wrapper that slices its decorations show what its
        // boxes take of each column, with the margins a break cuts.
        let consumed: Vec<f32> = match &leaf_pieces[order] {
            Some(_) if sliced => consumed_rows(
                child_pieces,
                &placement.pieces[spans[order].clone()],
                &items[spans[order].clone()],
            ),
            _ => child_pieces.iter().map(|piece| piece.height).collect(),
        };
        let first_piece = out.pieces.len();
        // How far each piece of a sliced box is moved up.
        let mut shifts = Vec::with_capacity(own.len());
        let mut taken = 0.0f32;
        for (index, piece) in own.iter().enumerate() {
            shifts.push(if sliced { taken } else { 0.0 });
            let row = piece.column / columns;
            let column = piece.column % columns;
            let flow = row_at(row);
            let top = flow.top - owner_y;
            let x = context.origin_x + context.column_offset_x(column);
            let fragmentainer = (flow.page as usize)
                .checked_mul(columns)
                .and_then(|first| first.checked_add(column))
                .ok_or(LayoutError::FragmentLimitExceeded {
                    limit: tree.fragment_tree.limit,
                })?;
            if piece.height > 0.0 {
                occupied[row].insert(column);
            }
            out.pieces.push(PagedPiece {
                child,
                fragmentainer,
                rect: sliced_rect(
                    FragmentRect {
                        x,
                        y: top + piece.top,
                        width: size.width,
                        height: piece.height,
                    },
                    sliced.then_some((taken, size.height)),
                ),
                clip: (own.len() > 1).then_some(FragmentRect {
                    x,
                    y: top,
                    width: size.width.max(context.column_width),
                    height: heights[row],
                }),
                index,
                count: own.len(),
                parent: None,
            });
            taken += consumed[index];
        }
        if let (Some(wrapped_tree), Some(leaf_pieces)) = (&wrapped[order], &leaf_pieces[order]) {
            let frame = Frame {
                columns,
                kept,
                heights: &heights,
                tops: &tops,
                column_width: context.column_width,
                grid,
            };
            let parent = Parent {
                node: child,
                first: first_piece,
                pieces: child_pieces,
                shifts: &shifts,
            };
            let pushed = push_nested(tree, &frame, wrapped_tree, leaf_pieces, &parent, &mut out);
            let Some(()) = pushed else { return Ok(None) };
        }
    }
    let groups = occupied
        .into_iter()
        .enumerate()
        .map(|(row, occupied)| MulticolGroup {
            context: FragmentationContext {
                origin_y: row_at(row).top - owner_y,
                available_height: Some(heights[row]),
                ..context
            },
            height: heights[row],
            occupied,
            slice: None,
        })
        .collect();
    Ok(Some(Paged {
        pieces: out.pieces,
        nested: out.nested,
        groups,
        end: row_at(last).top - owner_y + heights[last],
    }))
}

/// The pieces of each child of a container in the columns `grid`: those of
/// its box, or for a wrapper, those from the first box it holds to the last,
/// along with the pieces of each box it holds.
#[allow(clippy::type_complexity)]
fn planned_child_pieces(
    wrapped: &[Option<WrappedBlockTree>],
    spans: &[std::ops::Range<usize>],
    placement: &Placement,
    grid: Columns<'_>,
) -> Option<(Vec<Vec<Piece>>, Vec<Option<Vec<Vec<Piece>>>>)> {
    let mut child_pieces = Vec::with_capacity(wrapped.len());
    let mut leaf_pieces = Vec::with_capacity(wrapped.len());
    for (wrapped_tree, span) in wrapped.iter().zip(spans) {
        let stream = &placement.pieces[span.clone()];
        match wrapped_tree {
            None => {
                child_pieces.push(stream.first()?.clone());
                leaf_pieces.push(None);
            }
            Some(wrapped_tree) => {
                let leaves = make_wrapped_leaf_pieces(wrapped_tree, stream)?;
                child_pieces.push(wrapper_pieces_for_tree(wrapped_tree, &leaves, grid)?);
                leaf_pieces.push(Some(leaves));
            }
        }
    }
    Some((child_pieces, leaf_pieces))
}

/// What the pieces of the boxes inside a wrapper are placed with.
struct Frame<'a> {
    columns: usize,
    /// The number of pages kept, past which pieces are left out.
    kept: usize,
    /// The block size of the columns of each page kept.
    heights: &'a [f32],
    /// Where the columns of each page kept start in the container.
    tops: &'a [f32],
    column_width: f32,
    grid: Columns<'a>,
}

impl Frame<'_> {
    /// Where `piece` starts in the container's block axis.
    fn top_of(&self, piece: &Piece) -> f32 {
        self.tops[piece.column / self.columns] + piece.top
    }
}

/// The pieces made so far, and where each box or wrapper inside a wrapper
/// now stands (see [`Paged::nested`]).
struct Out {
    pieces: Vec<PagedPiece>,
    nested: Vec<(usize, Point<f32>)>,
}

/// The wrapper the pieces of a box or wrapper inside it are placed in.
struct Parent<'a> {
    node: usize,
    /// Where the wrapper's own pieces start in the pieces made so far.
    first: usize,
    /// The wrapper's pieces in the columns.
    pieces: &'a [Piece],
    /// How far each of the wrapper's pieces sits above its place in the
    /// column, which a wrapper that slices its decorations is moved up by.
    shifts: &'a [f32],
}

/// Add to `out` the pieces of what `wrapped` holds: each box, from
/// `leaf_pieces`, and each wrapper inside it, with its own boxes after it.
/// They are placed in the pieces of the wrapper `parent`.
fn push_nested(
    tree: &LayoutSession<'_>,
    frame: &Frame<'_>,
    wrapped: &WrappedBlockTree,
    leaf_pieces: &[Vec<Piece>],
    parent: &Parent<'_>,
    out: &mut Out,
) -> Option<()> {
    let mut offset = 0usize;
    for child in &wrapped.children {
        match child {
            WrappedBlockChild::Leaf(leaf) => {
                push_inside(tree, frame, *leaf, leaf_pieces.get(offset)?, parent, out)?;
                offset += 1;
            }
            WrappedBlockChild::Wrapper(nested) => {
                let end = offset.checked_add(nested.leaf_count)?;
                let nested_leaf_pieces = leaf_pieces.get(offset..end)?;
                let pieces = wrapper_pieces_for_tree(nested, nested_leaf_pieces, frame.grid)?;
                let first = out.pieces.len();
                push_inside(tree, frame, nested.node, &pieces, parent, out)?;
                let inner = Parent {
                    node: nested.node,
                    first,
                    pieces: &pieces,
                    shifts: &[],
                };
                push_nested(tree, frame, nested, nested_leaf_pieces, &inner, out)?;
                offset = end;
            }
        }
    }
    Some(())
}

/// Add to `out` the pieces `pieces` of `node`, a box or wrapper that the
/// wrapper `parent` holds, each placed in the piece of the wrapper in the
/// same column.
fn push_inside(
    tree: &LayoutSession<'_>,
    frame: &Frame<'_>,
    node: usize,
    pieces: &[Piece],
    parent: &Parent<'_>,
    out: &mut Out,
) -> Option<()> {
    let own: Vec<_> = pieces
        .iter()
        .filter(|piece| piece.column / frame.columns < frame.kept)
        .collect();
    let layout = tree.nodes[node].layout;
    let x = layout.location.x;
    let size = layout.size;
    let sliced = own.len() > 1 && slices_decorations(tree, node);
    let first_parent = parent.pieces.first()?;
    // The node stands as far down the wrapper as its first piece starts below
    // the wrapper's, in the flow of the container.
    out.nested.extend(own.first().map(|first| {
        let y = frame.top_of(first) - frame.top_of(first_parent);
        (node, Point { x, y })
    }));
    let mut taken = 0.0f32;
    for (index, piece) in own.iter().enumerate() {
        // The pieces of a wrapper cover one run of columns.
        let parent_index = piece.column.checked_sub(first_parent.column)?;
        let parent_piece = parent.pieces.get(parent_index)?;
        let shift = parent.shifts.get(parent_index).copied().unwrap_or(0.0);
        let fragmentainer = out.pieces.get(parent.first + parent_index)?.fragmentainer;
        out.pieces.push(PagedPiece {
            child: node,
            fragmentainer,
            rect: sliced_rect(
                FragmentRect {
                    x,
                    y: piece.top - parent_piece.top + shift,
                    width: size.width,
                    height: piece.height,
                },
                sliced.then_some((taken, size.height)),
            ),
            clip: (own.len() > 1).then_some(FragmentRect {
                x: 0.0,
                y: shift - parent_piece.top,
                width: (x + size.width).max(frame.column_width),
                height: frame.heights[piece.column / frame.columns],
            }),
            index,
            count: own.len(),
            parent: Some((parent.node, parent_index)),
        });
        taken += piece.height;
    }
    Some(())
}
