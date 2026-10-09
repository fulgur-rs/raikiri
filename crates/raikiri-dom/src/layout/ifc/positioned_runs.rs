//! Where the glyphs of an ifc root's lines are drawn.
//!
//! The painter and the page text runs both read glyph positions from here, so
//! the two cannot place a glyph differently. Positions are relative to the
//! root's content-box origin; each consumer adds its own page origin.

use crate::Document;
use crate::generated_content::{computed_for_id, generated_origin};
use raikiri_style::CascadeResult;
use raikiri_style::{PseudoElem, property::Visibility};
use raikiri_traits::PaintRect;
use shodo::Fragment;
use shodo::geometry::{PhysicalConverter, PhysicalSize, WritingMode};

/// One glyph of a run, at its outline origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineGlyph {
    /// Glyph id in the run's font.
    pub id: u32,
    /// Horizontal position from the line's origin.
    pub x: f32,
    /// Vertical position from the line's origin.
    pub y: f32,
}

/// A glyph run the painter draws, with its glyph positions.
#[derive(Clone, Debug)]
pub struct PositionedRun<'a> {
    /// The run as shodo laid it out.
    ///
    /// In an IFC with first-letter styling, generated text retains its original
    /// pseudo-element's virtual ID and UTF-8 byte offset using
    /// [`shodo::node::TextSource::Dom`]. That ID does not index the DOM arena;
    /// decode it with [`crate::generated_content::generated_origin`]. The
    /// typographic box ID is distinct from the text's original owner.
    pub run: shodo::GlyphRunView<'a>,
    /// The node the run is painted for: a text node, the id of a generated
    /// text, or an element that supplied text of its own.
    pub owner: usize,
    /// The box supplying paint, distinct from the caller's text source.
    pub style_owner: usize,
    /// Resolved paint including typographic pseudo inheritance.
    pub style: &'a raikiri_style::ComputedValues,
    /// Glyph positions in the run's glyph order, never empty.
    pub glyphs: Vec<LineGlyph>,
    /// The sum of the relative offsets of the run's inline ancestors, which
    /// move it after its line is laid out.
    pub offset: (f32, f32),
}

/// One image marker at its physical rectangle relative to the line origin.
#[derive(Clone, Copy, Debug)]
pub struct PositionedMarker {
    /// List item owning the marker.
    pub owner: usize,
    /// Marker rectangle, including the line's block offset.
    pub rect: PaintRect,
}

/// One line of an ifc root and the runs drawn on it.
#[derive(Clone, Debug)]
pub struct PositionedLine<'a> {
    /// Index of the line in the paragraph.
    pub index: usize,
    /// The line as shodo laid it out.
    pub line: &'a shodo::Line,
    /// Where the line's origin lies from the root's content-box origin: the
    /// offset of its column plus the distance pagination moved it down.
    pub offset: (f32, f32),
    /// Converts the line's logical coordinates to physical ones.
    pub converter: PhysicalConverter,
    /// The runs with a computed style, a font and at least one glyph.
    pub runs: Vec<PositionedRun<'a>>,
    /// Prepared image markers drawn before the line's inline boxes and text.
    pub markers: Vec<PositionedMarker>,
}

/// The lines of an ifc root, with what positions their glyphs.
#[derive(Debug)]
pub struct PositionedLines<'a> {
    document: &'a Document,
    cascade: &'a CascadeResult,
    root_id: usize,
    lines: &'a [shodo::Line],
    /// The root's writing mode.
    pub writing_mode: WritingMode,
    /// Content width the lines were broken at, and their total height.
    pub size: (f32, f32),
    converter_size: PhysicalSize,
    line_offsets: Vec<Option<(f32, f32)>>,
    /// How far pagination moved each line down, by line index.
    pub shifts: Vec<f32>,
    /// Paint offsets of the relatively positioned inline elements.
    pub offsets: &'a [(usize, (f32, f32))],
}

impl<'a> PositionedLines<'a> {
    /// The lines of `root_id`, or `None` when it has none.
    ///
    /// With `fragmentainer`, only the lines of that column are kept, each at
    /// the block offset of its column and at the column's inline start.
    /// Without it, lines split across columns sit where their column puts
    /// them; lines that are not split stay where they were laid out.
    pub fn new(
        document: &'a Document,
        cascade: &'a CascadeResult,
        root_id: usize,
        fragmentainer: Option<usize>,
    ) -> Option<Self> {
        let root_node = document.ifc_layout_node(root_id)?;
        let lines = root_node.ifc_lines()?;
        let writing_mode = root_node
            .ifc_writing_mode()
            .unwrap_or(WritingMode::HorizontalTb);
        let physical_content_size = root_node
            .ifc_physical_content_size()
            .unwrap_or(PhysicalSize::default());
        // Lines split across columns are drawn where their column puts them:
        // the first line of a range at the range's offset. Without a split
        // every line stays where it was laid out.
        let mut line_offsets: Vec<Option<(f32, f32)>> = vec![None; lines.len()];
        match root_node.ifc_multicol_fragments() {
            Some(fragments) => {
                for fragment in fragments {
                    if fragmentainer.is_some_and(|requested| requested != fragment.fragmentainer) {
                        continue;
                    }
                    let Some(first) = lines.get(fragment.line_start) else {
                        continue;
                    };
                    // A selected root fragment supplies its own physical origin.
                    // The unfiltered view still uses offsets from the normal box.
                    let origin_recorded = fragmentainer.is_some()
                        && root_node
                            .ifc
                            .as_ref()
                            .is_some_and(|root| root.multicol_fragment_origins_recorded);
                    let dy = if origin_recorded { 0.0 } else { fragment.y } - first.block_offset();
                    let end = fragment.line_end.min(lines.len());
                    for offset in &mut line_offsets[fragment.line_start.min(end)..end] {
                        *offset = Some((
                            if fragmentainer.is_some() {
                                0.0
                            } else {
                                fragment.x
                            },
                            dy,
                        ));
                    }
                }
            }
            None => line_offsets.fill(Some((0.0, 0.0))),
        }
        // Lines that pagination moved down with a block before them.
        let shifts = root_node.ifc_line_shifts();
        for (offset, shift) in line_offsets.iter_mut().zip(&shifts) {
            if let Some((_, dy)) = offset {
                *dy += shift;
            }
        }
        let size = root_node.ifc_size().unwrap_or((0.0, 0.0));
        let mut converter_size = physical_content_size;
        if writing_mode == WritingMode::HorizontalTb {
            converter_size.width = size.0;
        }
        Some(Self {
            document,
            cascade,
            root_id,
            lines,
            writing_mode,
            size,
            converter_size,
            line_offsets,
            shifts,
            offsets: root_node.ifc_relative_offsets(),
        })
    }

    /// Every line of the root, including those of other columns.
    pub fn all_lines(&self) -> &'a [shodo::Line] {
        self.lines
    }

    /// Shared line offsets for page decoration ownership without extracting glyphs.
    pub(crate) fn line_offset(&self, index: usize) -> Option<(f32, f32)> {
        self.line_offsets.get(index).copied().flatten()
    }

    /// The lines to draw, in order, with their runs.
    pub fn lines(&self) -> impl Iterator<Item = PositionedLine<'a>> + '_ {
        self.lines
            .iter()
            .enumerate()
            .filter_map(move |(index, line)| {
                let offset = self.line_offsets[index]?;
                Some(self.line(index, line, offset))
            })
    }

    fn line(&self, index: usize, line: &'a shodo::Line, offset: (f32, f32)) -> PositionedLine<'a> {
        let converter = PhysicalConverter::new(
            self.writing_mode,
            line.used_direction(),
            self.converter_size,
        );
        let mut runs = Vec::new();
        let mut markers = Vec::new();
        for fragment in line.fragments() {
            if let Fragment::Atomic(atomic) = fragment {
                if let Some((owner, PseudoElem::Marker)) = generated_origin(atomic.node.0 as usize)
                    && self.document.list_marker_image(owner).is_some()
                    && computed_for_id(self.cascade, atomic.node.0 as usize)
                        .is_some_and(|style| style.visibility == Visibility::Visible)
                {
                    let mut logical = atomic.border_rect;
                    logical.block_start += line.block_offset();
                    let rect = converter.rect(logical);
                    markers.push(PositionedMarker {
                        owner,
                        rect: PaintRect::new(rect.x, rect.y, rect.width, rect.height),
                    });
                }
                continue;
            }
            let Fragment::GlyphRun(run) = fragment else {
                continue;
            };
            // The paint owner is a text node; its computed color is the
            // inherited one. A `text-overflow` ellipsis has no node and is
            // painted in the root's style (CSS Overflow 3 §5.1).
            let owner = match run.node() {
                Some(node) => node.0 as usize,
                None if run.is_ellipsis() => self.root_id,
                None => continue, // cov:ignore: only the ellipsis run has no node.
            };
            // Text inherits `visibility` from its element; hidden and
            // collapsed text is laid out but not painted (CSS 2.1 §11.2).
            let letter = self
                .document
                .get_node(self.root_id)
                .filter(|_| index == 0)
                .and_then(|node| node.ifc.as_ref())
                .and_then(|ifc| {
                    run.source()
                        .and_then(|source| ifc.typographic_source(source))
                });
            let Some(style) = letter
                .map(|letter| &letter.computed)
                .or_else(|| computed_for_id(self.cascade, owner))
            else {
                continue;
            };
            let visible = style.visibility == Visibility::Visible;
            if !visible || run.font_data().is_none() {
                continue;
            }
            let glyphs: Vec<LineGlyph> = run
                .glyphs()
                .enumerate()
                .filter_map(|(glyph_index, glyph)| {
                    let (inline, block) = run.glyph_origin(glyph_index)?;
                    let (x, y) = converter.point(inline, block + line.block_offset());
                    Some(LineGlyph { id: glyph.id, x, y })
                })
                .collect();
            // A run without glyphs draws nothing.
            if !glyphs.is_empty() {
                runs.push(PositionedRun {
                    run,
                    owner: letter.map_or(owner, |letter| letter.source_owner),
                    style_owner: letter.map_or(owner, |letter| letter.box_id),
                    style,
                    glyphs,
                    offset: cumulative_offset(
                        self.document,
                        self.root_id,
                        self.offsets,
                        letter.map_or(owner, |letter| letter.source_owner),
                    ),
                });
            }
        }
        PositionedLine {
            index,
            line,
            offset,
            converter,
            runs,
            markers,
        }
    }
}

/// The sum of the relative offsets of `node` and its ancestors up to the ifc
/// root (the root excluded). An offset moves an inline element together with
/// everything inside it.
pub fn cumulative_offset(
    document: &Document,
    root_id: usize,
    offsets: &[(usize, (f32, f32))],
    node: usize,
) -> (f32, f32) {
    let (mut dx, mut dy) = (0.0, 0.0);
    // A pseudo-element moves with its element.
    let node = generated_origin(node).map_or(node, |(element, _)| element);
    let mut current = Some(node);
    while let Some(id) = current {
        if id == root_id {
            break;
        }
        if let Some((_, (x, y))) = offsets.iter().find(|(owner, _)| *owner == id) {
            dx += x;
            dy += y;
        }
        current = document.parent_of(id);
    }
    (dx, dy)
}

#[cfg(test)]
mod tests;
