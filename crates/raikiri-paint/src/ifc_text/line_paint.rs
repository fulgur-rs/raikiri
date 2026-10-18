//! The parts of a paragraph's lines that [`super::hold_ifc_lines`] paints:
//! the boxes beside the lines, the inline boxes on them, and the paint they
//! go into.

use super::typographic::{self, BackgroundSlice, Paragraph, TypographicPaint};
use super::{IfcPosition, piece_fixed_viewport, transform_applies};
use kurbo::{Affine, Rect};
use raikiri_dom::generated_content::computed_for_id;
use raikiri_dom::{Document, InlineBoxPiece, Node, PositionedLines, cumulative_offset};
use raikiri_style::CascadeResult;

/// A box laid out beside the lines, as one piece on its line and as the joined
/// box it is sliced from.
pub(super) type BlockPiece = (InlineBoxPiece, BackgroundSlice);

/// Where a page that keeps the paragraph whole holds it: the paragraph's
/// content box, whose position in the page `base` gives. `None` when no page
/// keeps the paragraph whole.
pub(super) fn kept_whole_box(
    document: &Document,
    root_id: usize,
    page_lines: Option<crate::walk::PageLines>,
    vertical: bool,
    base: IfcPosition,
) -> Option<Rect> {
    page_lines
        .filter(|lines| lines.keeps_paragraph_whole(vertical))
        .and_then(|_| document.ifc_layout_node(root_id))
        .map(|node| {
            Rect::from_origin_size(
                (f64::from(base.x), f64::from(base.y + base.shift_y)),
                (
                    f64::from(node.unrounded_layout.content_box_width()),
                    f64::from(node.unrounded_layout.content_box_height()),
                ),
            )
        })
}

/// The inline boxes of a paragraph by the line they sit on.
pub(super) struct PiecesByLine {
    pub(super) pieces: Vec<Vec<InlineBoxPiece>>,
    /// The slice of the joined box that each piece in `pieces` is painted
    /// from, in the same order.
    pub(super) slices: Vec<Vec<Option<BackgroundSlice>>>,
}

impl PiecesByLine {
    pub(super) fn new(
        positioned: &PositionedLines<'_>,
        document: &Document,
        cascade: &CascadeResult,
        root_id: usize,
    ) -> Self {
        let writing_mode = positioned.writing_mode;
        let pieces = document
            .ifc_layout_node(root_id)
            .and_then(|node| node.ifc_inline_boxes())
            .unwrap_or_default();
        // A box split across lines is sliced from one joined box, which spans
        // every line of the paragraph, including those on other pages, along
        // the paragraph's inline axis.
        let progression = |node| {
            crate::walk::InlineProgression::new(
                writing_mode,
                computed_for_id(cascade, node)
                    .is_some_and(|cv| cv.direction == raikiri_style::property::Direction::Rtl),
            )
        };
        let all_slices = typographic::background_slices(&pieces, progression);
        let mut pieces_by_line = vec![Vec::new(); positioned.all_lines().len()];
        let mut slices_by_line = vec![Vec::new(); positioned.all_lines().len()];
        for (piece, slice) in pieces.into_iter().zip(all_slices) {
            if let (Some(line_pieces), Some(line_slices)) = (
                pieces_by_line.get_mut(piece.line),
                slices_by_line.get_mut(piece.line),
            ) {
                line_pieces.push(piece);
                line_slices.push(slice);
            }
        }
        Self {
            pieces: pieces_by_line,
            slices: slices_by_line,
        }
    }
}

/// The block-level `::before` and `::after` boxes of a paragraph, none when
/// `off_band`.
///
/// A block-level `::before` or `::after` paints its own box, cut along the
/// block axis where pagination cuts its lines. Its pieces on the lines hold
/// only its text. Like other block boxes, it is sliced even under
/// `box-decoration-break: clone`, as pagination reserves no room for cloned
/// edges.
pub(super) fn block_pieces(root_node: &Node, off_band: bool) -> Vec<BlockPiece> {
    root_node
        .ifc_generated_block_pieces()
        .into_iter()
        .filter(|_| !off_band)
        .map(|block| {
            (
                InlineBoxPiece {
                    node: block.node,
                    line: block.line,
                    border_box: block.border_box,
                    content_box: block.content_box,
                    has_start_edge: block.has_block_start_edge,
                    has_end_edge: block.has_block_end_edge,
                    parent: None,
                    source_container: None,
                    source_owner: None,
                },
                BackgroundSlice {
                    outer: block.joined_border_box,
                    content: block.joined_content_box,
                },
            )
        })
        .collect()
}

/// The paint of a paragraph's lines, before it is composited.
pub(super) struct LineGroups {
    /// The paint of a translucent `::first-line`, with its opacity.
    pub(super) first_line: Option<(TypographicPaint, f32)>,
    pub(super) rest: TypographicPaint,
}

impl LineGroups {
    pub(super) fn new(
        paragraph: Paragraph<'_>,
        root_node: &Node,
        pieces_by_line: &[Vec<InlineBoxPiece>],
        blocks: &[BlockPiece],
    ) -> Self {
        // A translucent `::first-line` composites the paint of the first line
        // as one group. It splits the inline elements on that line from their
        // other lines (CSS Pseudo 4 §2.1.1), so the first line has groups of
        // its own.
        let first_line_opacity = raikiri_dom::paint_rules::first_line_opacity_layer(
            paragraph.document,
            paragraph.root_id,
        );
        // A box goes with the paint of its first line, below the text of the
        // lines after it.
        let on_first_line =
            |piece: &InlineBoxPiece| first_line_opacity.is_some() && piece.line == 0;
        let first_line = first_line_opacity.map(|opacity| {
            let mut pieces = pieces_by_line.first().cloned().unwrap_or_default();
            pieces.extend(blocks.iter().map(|(piece, _)| *piece).filter(on_first_line));
            (
                TypographicPaint::new(paragraph, root_node, &pieces),
                opacity,
            )
        });
        // One set of groups serves every other line, so an element or
        // pseudo-element split over lines composites its complete paint at
        // once (CSS Color 4 §3.3).
        let mut rest_pieces: Vec<_> = pieces_by_line
            .iter()
            .skip(usize::from(first_line.is_some()))
            .flatten()
            .cloned()
            .collect();
        rest_pieces.extend(
            blocks
                .iter()
                .map(|(piece, _)| *piece)
                .filter(|piece| !on_first_line(piece)),
        );
        let rest = TypographicPaint::new(paragraph, root_node, &rest_pieces);
        Self { first_line, rest }
    }

    /// The paint that line `line_index`, and a box on it, goes into.
    pub(super) fn for_line(&mut self, line_index: usize) -> &mut TypographicPaint {
        match self.first_line.as_mut() {
            Some((first, _)) if line_index == 0 => first,
            _ => &mut self.rest,
        }
    }
}

/// What the lines of a paragraph are painted from.
pub(super) struct LinePaint<'a, 'r> {
    pub(super) image_cache: &'r crate::walk::ImageBrushCache,
    pub(super) paragraph: Paragraph<'a>,
    /// The paint offsets of the relatively positioned inline elements.
    pub(super) offsets: &'a [(usize, (f32, f32))],
    /// The content-box origin of the paragraph in page coordinates.
    pub(super) base: IfcPosition,
    pub(super) paint_transform: Affine,
    pub(super) fixed_viewport: Option<Rect>,
    pub(super) pixel_source: Option<&'r dyn raikiri_traits::ImagePixelSource>,
    pub(super) warnings: &'r mut Vec<raikiri_traits::RenderWarning>,
}

impl LinePaint<'_, '_> {
    /// Paint the boxes of `blocks` into `groups`.
    pub(super) fn paint_block_pieces(&mut self, groups: &mut LineGroups, blocks: &[BlockPiece]) {
        let Paragraph {
            document,
            cascade,
            root_id,
        } = self.paragraph;
        let base = self.base;
        for (piece, joined) in blocks {
            let Some(cv) = computed_for_id(cascade, piece.node)
                .filter(|cv| cv.visibility == raikiri_style::property::Visibility::Visible)
            else {
                continue;
            };
            let outer = piece.border_box;
            if outer.width <= 0.0 || outer.height <= 0.0 {
                continue;
            }
            let (dx, dy) = cumulative_offset(document, root_id, self.offsets, piece.node);
            let (x, y) = (base.x + dx, base.y + base.shift_y + dy);
            let paint = groups.for_line(piece.line);
            let group = paint.piece_group(piece);
            // The piece's edges are along the block axis: its progression runs
            // down the page.
            crate::walk::paint_inline_box(
                self.image_cache,
                paint.box_target(group, piece),
                cv,
                piece,
                Some(joined),
                crate::walk::InlineProgression {
                    vertical: true,
                    reversed: false,
                },
                transform_applies(document, cascade, piece.node, cv),
                x,
                y,
                self.pixel_source,
                self.warnings,
                self.paint_transform,
                piece_fixed_viewport(self.paragraph, cv, piece, self.fixed_viewport),
            );
        }
    }
}
