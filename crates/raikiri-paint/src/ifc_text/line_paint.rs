//! The parts of a paragraph's lines that [`super::hold_ifc_lines`] paints:
//! the boxes beside the lines, the inline boxes on them, and the paint they
//! go into.

use super::typographic::{self, BackgroundSlice, Paragraph, TypographicPaint};
use super::{
    IfcPosition, draw_custom_highlights, line_rect, piece_fixed_viewport, transform_applies,
};
use crate::text::DecorationContext;
use anyrender::PaintScene;
use kurbo::{Affine, Rect};
use peniko::Fill;
use raikiri_dom::generated_content::{computed_for_id, generated_origin};
use raikiri_dom::{
    Document, InlineBoxPiece, Node, PositionedContentImage, PositionedLine, PositionedLines,
    PositionedMarker, cumulative_offset,
};
use raikiri_style::CascadeResult;
use shodo::geometry::WritingMode;
use shodo::hit::LineLayout;
use std::collections::HashSet;

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
                    f64::from(node.layout().content_box_width()),
                    f64::from(node.layout().content_box_height()),
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
    pub(super) writing_mode: WritingMode,
    /// Whether the lines run along the vertical axis.
    pub(super) vertical: bool,
    pub(super) base_decorations: &'r DecorationContext,
    /// How far pagination moved each line along the block axis.
    pub(super) shifts: &'r [f32],
    pub(super) root_node: &'a Node,
    pub(super) pieces: PiecesByLine,
    /// The nodes of the block-level boxes, which paint beside the lines
    /// rather than on them.
    pub(super) block_nodes: HashSet<usize>,
    /// The pages that hold the paragraph's lines, when it is cut for pages.
    pub(super) page_lines: Option<crate::walk::PageLines>,
    /// The content box of a paragraph that a page keeps whole.
    pub(super) paragraph_box: Option<Rect>,
    /// The line layout that locates custom highlights, when there are any.
    pub(super) hit_layout: Option<LineLayout<'a>>,
    pub(super) custom_highlights: &'r [crate::TextHighlightRange],
    pub(super) paint_transform: Affine,
    pub(super) fixed_viewport: Option<Rect>,
    pub(super) pixel_source: Option<&'r dyn raikiri_traits::ImagePixelSource>,
    pub(super) warnings: &'r mut Vec<raikiri_traits::RenderWarning>,
}

impl<'a> LinePaint<'a, '_> {
    /// Paint `line` into `paint`: its markers, the boxes of its inline
    /// elements, the images of its generated content, its custom highlights
    /// and its text, in that order. A line a page does not hold paints
    /// nothing.
    pub(super) fn paint_line(&mut self, paint: &mut TypographicPaint, line: PositionedLine<'a>) {
        let line_index = line.index;
        let (column_x, column_y) = line.offset;
        let base = self.base;
        let position = IfcPosition {
            x: base.x + column_x,
            y: base.y + column_y,
            shift_y: base.shift_y,
        };
        if let Some(page_lines) = self.page_lines
            && !page_lines.holds_line(
                self.paint_transform,
                self.paragraph_box
                    .unwrap_or_else(|| line_rect(&line, position)),
            )
        {
            return;
        }
        self.paint_marker_images(paint, &line.markers, position);
        self.paint_line_boxes(paint, line_index, position);
        self.paint_content_images(paint, &line.content_images, line_index, position);
        if !self.vertical
            && let Some(hit_layout) = self.hit_layout.as_ref()
        {
            draw_custom_highlights(
                paint,
                self.paragraph,
                line_index,
                line.line,
                hit_layout,
                position,
                self.offsets,
                self.custom_highlights,
                &line.runs,
                &line.converter,
            );
        }
        self.paint_text(paint, line, position);
    }

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
            // down the page, or across it in a vertical paragraph, from the
            // side its lines start on.
            crate::walk::paint_inline_box(
                self.image_cache,
                paint.box_target(group, piece),
                cv,
                piece,
                Some(joined),
                crate::walk::InlineProgression {
                    vertical: !self.vertical,
                    reversed: matches!(
                        self.writing_mode,
                        WritingMode::VerticalRl | WritingMode::SidewaysRl
                    ),
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

    /// Paint the image markers of a line, below its boxes and text.
    fn paint_marker_images(
        &self,
        paint: &mut TypographicPaint,
        markers: &[PositionedMarker],
        position: IfcPosition,
    ) {
        let document = self.paragraph.document;
        // Positioned lines list only the markers whose image the document
        // holds.
        let images = markers.iter().filter_map(|marker| {
            document
                .list_marker_image_shared(marker.owner)
                .map(|image| (marker, image))
        });
        for (marker, image) in images {
            let rect = marker.rect;
            let brush = self.image_cache.whole(image);
            paint.target(None).fill(
                Fill::NonZero,
                Affine::translate((
                    f64::from(position.x + rect.x),
                    f64::from(position.y + position.shift_y + rect.y),
                )) * Affine::scale_non_uniform(
                    f64::from(rect.width) / f64::from(image.width),
                    f64::from(rect.height) / f64::from(image.height),
                ),
                brush.as_ref(),
                None,
                &Rect::new(0.0, 0.0, f64::from(image.width), f64::from(image.height)),
            );
        }
    }

    /// Paint the boxes of the inline elements on line `line_index`, which
    /// sit at `position`.
    ///
    /// They go below the line's text (CSS 2.1 Appendix E: an inline box's
    /// background and borders, then its text).
    fn paint_line_boxes(
        &mut self,
        paint: &mut TypographicPaint,
        line_index: usize,
        position: IfcPosition,
    ) {
        let Paragraph {
            document,
            cascade,
            root_id,
        } = self.paragraph;
        let background_slices = &self.pieces.slices[line_index];
        let root_node = self.root_node;
        let block_nodes = &self.block_nodes;
        let styled_pieces = self.pieces.pieces[line_index]
            .iter()
            .enumerate()
            .filter(|(_, piece)| {
                !(block_nodes.contains(&piece.node)
                    || generated_origin(piece.node)
                        .is_some_and(|(_, pseudo)| pseudo == raikiri_style::PseudoElem::Marker))
            })
            .filter_map(|(piece_index, piece)| {
                piece_style(root_node, cascade, piece, line_index)
                    .map(|(cv, source)| (piece_index, piece, cv, source))
            });
        for (piece_index, piece, cv, source) in styled_pieces {
            let (dx, dy) = cumulative_offset(document, root_id, self.offsets, source);
            // The pieces already carry the line's pagination shift, which
            // `position` holds too.
            let line_shift = self.shifts.get(line_index).copied().unwrap_or(0.0);
            let group = paint.piece_group(piece);
            // An atomic `::before` or `::after` turns about its own box, as
            // painted here (CSS Transforms 1 §6).
            let mut piece_transform = self.paint_transform;
            if let Some(turned) = raikiri_dom::paint_rules::pseudo_transform(cascade, piece.node) {
                let outer = piece.border_box;
                let turn = crate::walk::element_transform(
                    turned,
                    position.x + dx + outer.x,
                    position.y + position.shift_y + dy - line_shift + outer.y,
                    outer.width,
                    outer.height,
                );
                piece_transform *= turn;
                paint.turn(group, piece, turn);
            }
            if raikiri_dom::paint_rules::hides_own_box(cv) {
                continue;
            }
            crate::walk::paint_inline_box(
                self.image_cache,
                paint.box_target(group, piece),
                cv,
                piece,
                background_slices[piece_index].as_ref(),
                crate::walk::InlineProgression::new(
                    self.writing_mode,
                    cv.direction == raikiri_style::property::Direction::Rtl,
                ),
                transform_applies(document, cascade, piece.node, cv),
                position.x + dx,
                position.y + position.shift_y + dy - line_shift,
                self.pixel_source,
                self.warnings,
                piece_transform,
                piece_fixed_viewport(self.paragraph, cv, piece, self.fixed_viewport),
            );
        }
    }

    /// Paint the images of `::before` and `::after` content on line
    /// `line_index`, which sits at `position`.
    ///
    /// They are replaced content inside their pseudo-element's inline box:
    /// above its background, below the line's text. They move with the
    /// pseudo-element's relatively positioned inline ancestors.
    fn paint_content_images(
        &self,
        paint: &mut TypographicPaint,
        images: &[PositionedContentImage],
        line_index: usize,
        position: IfcPosition,
    ) {
        let Paragraph {
            document, root_id, ..
        } = self.paragraph;
        // Positioned lines list only the content images whose pixels the
        // document holds.
        let pixel_images = images.iter().filter_map(|image| {
            document
                .content_image_shared(image.owner, image.pseudo, image.index)
                .map(|pixels| (image, pixels))
        });
        for (image, pixels) in pixel_images {
            let pseudo_node =
                raikiri_dom::generated_content::generated_node_id(image.owner, image.pseudo);
            let group = self.pieces.pieces[line_index]
                .iter()
                .find(|piece| piece.node == pseudo_node)
                .and_then(|piece| paint.piece_group(piece));
            let (dx, dy) = cumulative_offset(document, root_id, self.offsets, pseudo_node);
            let rect = image.rect;
            let (left, top) = (
                f64::from(position.x + dx + rect.x),
                f64::from(position.y + position.shift_y + dy + rect.y),
            );
            // An image that replaces its pseudo-element is drawn at its
            // `object-fit` rectangle, clipped to its box.
            let object = document
                .content_image_object_rect(image.owner, image.pseudo, image.index)
                .unwrap_or(raikiri_traits::PaintRect::new(
                    0.0,
                    0.0,
                    rect.width,
                    rect.height,
                ));
            let clipped = object.x < 0.0
                || object.y < 0.0
                || object.x + object.width > rect.width
                || object.y + object.height > rect.height;
            let target = paint.target(group);
            if clipped {
                target.push_clip_layer(
                    Affine::IDENTITY,
                    &Rect::new(
                        left,
                        top,
                        left + f64::from(rect.width),
                        top + f64::from(rect.height),
                    ),
                );
            }
            let brush = self.image_cache.whole(pixels);
            target.fill(
                Fill::NonZero,
                Affine::translate((left + f64::from(object.x), top + f64::from(object.y)))
                    * Affine::scale_non_uniform(
                        f64::from(object.width) / f64::from(pixels.width),
                        f64::from(object.height) / f64::from(pixels.height),
                    ),
                brush.as_ref(),
                None,
                &Rect::new(0.0, 0.0, f64::from(pixels.width), f64::from(pixels.height)),
            );
            if clipped {
                target.pop_layer();
            }
        }
    }
}

/// The style the box of `piece`, on line `line_index`, is painted with, and
/// the node whose relative offset moves the box. `None` when no style applies
/// to it.
fn piece_style<'c>(
    root_node: &'c Node,
    cascade: &'c CascadeResult,
    piece: &InlineBoxPiece,
    line_index: usize,
) -> Option<(&'c raikiri_style::ComputedValues, usize)> {
    let retained =
        root_node.ifc_typographic_fragment(piece.node, piece.source_container, piece.source_owner);
    let cv = retained
        .map(|(style, _)| style)
        .or_else(|| {
            // The first line's boxes take their values from the
            // `::first-line` around them.
            root_node
                .ifc_first_line_element_style(piece.node)
                .filter(|_| line_index == 0)
        })
        .or_else(|| computed_for_id(cascade, piece.node))?;
    let source = retained.map(|(_, owner)| owner).unwrap_or(piece.node);
    Some((cv, source))
}
