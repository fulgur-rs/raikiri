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
}

/// The pieces and the page groups of a container, and where its columns end
/// in the container's coordinates.
pub(in crate::layout::multicol) struct Paged {
    pub(in crate::layout::multicol) pieces: Vec<PagedPiece>,
    pub(in crate::layout::multicol) groups: Vec<MulticolGroup>,
    pub(in crate::layout::multicol) end: f32,
}

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
    let items: Vec<Item> = children
        .iter()
        .map(|&child| {
            item(
                tree,
                child,
                tree.nodes[child].layout.size.height,
                Some(Insets::Against(context.column_width)),
            )
        })
        .collect();
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
    let height_of = |column: usize| row_at(column / columns).capacity();
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
    let mut pieces = Vec::new();
    for (&child, child_pieces) in children.iter().zip(&placement.pieces) {
        let size = tree.nodes[child].layout.size;
        let own: Vec<_> = child_pieces
            .iter()
            .filter(|piece| piece.column / columns < kept)
            .collect();
        let sliced = own.len() > 1 && slices_decorations(tree, child);
        let mut taken = 0.0f32;
        for (index, piece) in own.iter().enumerate() {
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
            pieces.push(PagedPiece {
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
            });
            taken += piece.height;
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
        pieces,
        groups,
        end: row_at(last).top - owner_y + heights[last],
    }))
}
