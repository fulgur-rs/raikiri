//! Records of the out-of-flow boxes that sit on the lines of a paragraph the
//! columns broke.
//!
//! Each such box goes under the piece of the paragraph in the column of its
//! line. A box whose containing block lies inside the columns is part of
//! their fragmentation context, so a box that reaches past the end of its
//! column goes on in the next columns, overflow columns included (CSS
//! Multi-column 1, 8.1; CSS Fragmentation 3, 4.1). Its pieces paint the whole
//! box, moved up by the columns before them and clipped to their own column.
//! A box positioned against the container or a box outside it is laid out
//! outside that context and stays whole.

use super::*;
use crate::fragment::{FragmentRect, FragmentationContext, LayoutFragment};
use crate::layout::ifc::boxes::IfcBoxKind;
use crate::layout::ifc::root::IfcRoot;
use crate::node::MulticolTextFragment;
use raikiri_layout_geometry::Point;

/// Give each out-of-flow box on the lines of a paragraph that the columns broke
/// a record of its own, under the piece of the paragraph in the column it
/// ends up in, once the layout has placed it. Without one, the box would be
/// drawn under every piece of the paragraph, at its place from the first.
/// A box that its containing block puts in the columns and that reaches past
/// the end of its column gets a record in each column it reaches.
pub(crate) fn record_out_of_flow_boxes_on_column_lines(
    tree: &mut Document,
    cascade: &CascadeResult,
) {
    let mut placed = Vec::new();
    for paragraph in 0..tree.nodes.len() {
        let Some(root) = tree.nodes[paragraph]
            .ifc
            .as_ref()
            .filter(|root| root.multicol_fragments.is_some())
        else {
            continue;
        };
        let records = &tree.fragment_tree;
        let live = |index: &&usize| !records.is_retired(**index);
        let pieces: Vec<(usize, Point<f32>)> = records
            .records_of(paragraph)
            .iter()
            .filter(live)
            .filter_map(|&piece| Some((piece, record_origin(tree, piece)?)))
            .collect();
        if pieces.is_empty() {
            continue;
        }
        let last_piece = *pieces
            .iter()
            .max_by_key(|(piece, _)| records.fragments[*piece].fragmentainer)
            .unwrap_or(&pieces[0]);
        let positioned = root
            .boxes
            .iter()
            .filter(|box_| box_.kind == IfcBoxKind::OutOfFlow)
            .filter(|box_| {
                !records
                    .records_of(box_.node)
                    .iter()
                    .any(|index| live(&index))
            });
        // The columns of the container whose fragmentation context holds the
        // paragraph, found once for the paragraph.
        let mut columns: Option<Option<(usize, Columns)>> = None;
        for box_ in positioned {
            let node = box_.node;
            let origin = tree.layout_origin_of(node).unwrap_or_default();
            // The piece in the column of the line the box sits on, else the
            // first.
            let line = line_fragment(tree, root, node);
            let line_piece = line.and_then(|line| {
                pieces
                    .iter()
                    .find(|(piece, _)| {
                        records.fragments[*piece].fragmentainer == line.fragmentainer
                    })
                    .map(|&piece| (line, piece))
            });
            let (piece, piece_origin) = line_piece.map_or(pieces[0], |(_, piece)| piece);
            let size = tree.nodes[node].layout.size;
            let mut rect = FragmentRect {
                x: origin.x - piece_origin.x,
                y: origin.y - piece_origin.y,
                width: size.width,
                height: size.height,
            };
            // A box that follows its line goes with it into the piece of
            // its range. A piece whose record holds the origin of the range
            // draws the range's first line where its content box begins, so
            // the box moves up by the range's offset from the first piece;
            // any other piece draws the lines at their offsets in the
            // paragraph, from the origin of the piece.
            if let Some((line, _)) = line_piece
                && let Some(paragraph_origin) = tree.layout_origin_of(paragraph)
                && follows_line_block_offset(tree, cascade, paragraph, node)
            {
                let range_offset = if root.multicol_fragment_origins_recorded {
                    line.y
                } else {
                    0.0
                };
                rect.y += piece_origin.y - paragraph_origin.y - range_offset;
            }
            let columns = columns
                .get_or_insert_with(|| {
                    let container = tree
                        .inclusive_ancestors(paragraph)
                        .find(|&id| tree.nodes[id].multicol.is_some())?;
                    Some((container, Columns::of(tree, container)?))
                })
                .as_ref()
                .filter(|(container, _)| breaks_in_columns(tree, *container, node));
            let budget = records
                .limit
                .saturating_sub(records.fragments.len() + placed.len());
            let split = columns.and_then(|(_, columns)| {
                columns.pieces(
                    &tree.nodes[node].layout,
                    FragmentRect {
                        x: piece_origin.x + rect.x,
                        y: piece_origin.y + rect.y,
                        ..rect
                    },
                    budget,
                )
            });
            match split {
                Some(split) => {
                    let count = split.len();
                    for (index, (column, rect, clip)) in split.into_iter().enumerate() {
                        // The piece of the paragraph in the column, else its
                        // last piece, which holds the columns after its own.
                        let (parent, parent_origin) = pieces
                            .iter()
                            .find(|(piece, _)| records.fragments[*piece].fragmentainer == column)
                            .copied()
                            .unwrap_or(last_piece);
                        let local = |rect: FragmentRect| FragmentRect {
                            x: rect.x - parent_origin.x,
                            y: rect.y - parent_origin.y,
                            ..rect
                        };
                        placed.push(LayoutFragment {
                            node_id: node,
                            parent: Some(parent),
                            fragmentainer: column,
                            rect: local(rect),
                            fragmentainer_clip: Some(local(clip).into()),
                            fragment_index: index,
                            fragment_count: count,
                            line_start: None,
                            line_end: None,
                        });
                    }
                }
                None if pieces.len() > 1 => placed.push(LayoutFragment {
                    node_id: node,
                    parent: Some(piece),
                    fragmentainer: records.fragments[piece].fragmentainer,
                    rect,
                    fragmentainer_clip: None,
                    fragment_index: 0,
                    fragment_count: 1,
                    line_start: None,
                    line_end: None,
                }),
                None => {}
            }
        }
    }
    for record in placed {
        // A layout past the cap of records fails as a whole: the caller
        // checks the cap after this.
        let _ = tree.fragment_tree.try_push(record);
    }
}

/// The range of the columns that holds the line the box `node` of the
/// paragraph `root` sits on.
fn line_fragment(tree: &Document, root: &IfcRoot, node: usize) -> Option<MulticolTextFragment> {
    let (line, _) = crate::layout::ifc::boxes::find_oof_anchor(tree, root.lines.as_ref()?, node)?;
    root.multicol_fragments
        .as_ref()?
        .iter()
        .find(|fragment| (fragment.line_start..fragment.line_end).contains(&line))
        .copied()
}

/// Whether the block offset of the out-of-flow box `node` on the lines of
/// `paragraph` comes from its line: it has a static block position (CSS 2,
/// 10.6.4), or its containing block is an inline element of the paragraph,
/// which lies on the lines.
fn follows_line_block_offset(
    tree: &Document,
    cascade: &CascadeResult,
    paragraph: usize,
    node: usize,
) -> bool {
    let style = &cascade.computed[node];
    let is_auto = |inset: &ComputedLengthPercentageOrAuto| {
        matches!(inset, ComputedLengthPercentageOrAuto::Auto)
    };
    (is_auto(&style.top) && is_auto(&style.bottom))
        || containing_block(tree, node).is_some_and(|block| {
            block != paragraph && tree.ancestors(block).any(|id| id == paragraph)
        })
}

/// The box that is the containing block of the out-of-flow box `node`, when
/// it is an element of the document.
fn containing_block(tree: &Document, node: usize) -> Option<usize> {
    let fixed = tree.nodes[node].position() == &PositionValue::Fixed;
    tree.ancestors(node).find(|&id| {
        let ancestor = &tree.nodes[id];
        if fixed {
            ancestor.claims_fixed_oof
        } else {
            ancestor.claims_absolute_oof
        }
    })
}

/// Whether the columns of `container` break the out-of-flow box `node`: its
/// containing block lies inside them. A box positioned against the container
/// itself, or against a box outside it, is laid out outside the columns.
fn breaks_in_columns(tree: &Document, container: usize, node: usize) -> bool {
    containing_block(tree, node)
        .is_some_and(|block| block != container && tree.ancestors(block).any(|id| id == container))
}

/// The columns of a container that is laid out in one piece.
struct Columns {
    context: FragmentationContext,
    /// The top left corner of the container's content box in the layout.
    origin: Point<f32>,
    /// The block size of each column.
    height: f32,
}

impl Columns {
    /// The columns of the multicol container `container`, when it has a
    /// single record and horizontal columns.
    fn of(tree: &Document, container: usize) -> Option<Self> {
        let style = tree.nodes[container]
            .multicol
            .filter(|style| style.horizontal)?;
        let records = &tree.fragment_tree;
        let [record] = records
            .records_of(container)
            .iter()
            .copied()
            .filter(|&index| !records.is_retired(index))
            .collect::<Vec<_>>()[..]
        else {
            return None;
        };
        let origin = record_origin(tree, record)?;
        let border_box = records.fragments[record].rect;
        let layout = &tree.nodes[container].layout;
        let (border, padding) = (layout.border, layout.padding);
        let width = border_box.width - border.left - border.right - padding.left - padding.right;
        let height = border_box.height - border.top - border.bottom - padding.top - padding.bottom;
        (height > 0.0).then_some(())?;
        Some(Self {
            context: FragmentationContext::resolve(width, Some(height), style)?,
            origin: Point {
                x: origin.x + border.left + padding.left,
                y: origin.y + border.top + padding.top,
            },
            height,
        })
    }

    /// The pieces of a box of `layout` at `rect`, in layout coordinates, when
    /// it reaches past the end of the column it starts in: for each column it
    /// reaches, the column, the whole box moved there and up by the columns
    /// before it, and the clip of that column, which lets what the box holds
    /// reach past it inline. `None` when the box ends in its
    /// column, starts outside it, or needs more than `budget` pieces.
    fn pieces(
        &self,
        layout: &raikiri_layout_geometry::BoxLayout,
        rect: FragmentRect,
        budget: usize,
    ) -> Option<Vec<(usize, FragmentRect, FragmentRect)>> {
        let overflow = layout.scrollable_overflow_rect;
        let top = rect.y - self.origin.y;
        // Descendants that overflow the box are cut with it. The overflow
        // rect is measured from the padding box.
        let painted_top = top + (layout.border.top + overflow.top).min(0.0);
        let end = top + rect.height.max(layout.border.top + overflow.bottom);
        if !(painted_top >= -lineless::EPSILON && top < self.height && end.is_finite())
            || end <= self.height + lineless::EPSILON
        {
            return None;
        }
        let count = ((end - lineless::EPSILON) / self.height).floor() as usize + 1;
        if count > budget {
            return None;
        }
        let left = rect.x + (layout.border.left + overflow.left).min(0.0);
        let right = rect.x + rect.width.max(layout.border.left + overflow.right);
        let column = self.column_at(if self.context.rtl {
            rect.x + rect.width
        } else {
            rect.x
        });
        let first_x = self.context.column_offset_x(column);
        Some(
            (0..count)
                .map(|index| {
                    let at = column + index;
                    let dx = self.context.column_offset_x(at) - first_x;
                    let column_left = self.origin.x + self.context.column_offset_x(at);
                    let clip_left = column_left.min(left + dx);
                    let clip_right = (column_left + self.context.column_width).max(right + dx);
                    (
                        at,
                        FragmentRect {
                            x: rect.x + dx,
                            y: rect.y - index as f32 * self.height,
                            ..rect
                        },
                        FragmentRect {
                            x: clip_left,
                            y: self.origin.y,
                            width: clip_right - clip_left,
                            height: self.height,
                        },
                    )
                })
                .collect(),
        )
    }

    /// The column, overflow columns included, that holds the inline-start
    /// edge `start` of a box, with the gap after it; the first one for an
    /// edge before the first column.
    fn column_at(&self, start: f32) -> usize {
        let context = self.context;
        let from_left = start - self.origin.x;
        // Columns run from the right edge in a right-to-left container.
        let advance = if context.rtl {
            context.available_width - from_left
        } else {
            from_left
        };
        let step = context.column_width + context.column_gap;
        ((advance + lineless::EPSILON) / step).floor().max(0.0) as usize
    }
}

/// Where the record `index` places the origin of its box in the layout: a
/// record without a parent is relative to the layout origin of its box, and
/// one with a parent to the origin of its parent.
fn record_origin(tree: &Document, index: usize) -> Option<Point<f32>> {
    let record = &tree.fragment_tree.fragments[index];
    let base = match record.parent {
        Some(parent) => record_origin(tree, parent)?,
        None => tree.layout_origin_of(record.node_id)?,
    };
    Some(Point {
        x: base.x + record.rect.x,
        y: base.y + record.rect.y,
    })
}
