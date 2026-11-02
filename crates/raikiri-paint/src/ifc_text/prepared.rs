//! What painting a paragraph prepares from all of its lines.
//!
//! A paragraph split into columns is painted once for each column, and each
//! of those paints needs the same preparation: the inline boxes of every line,
//! the joined box that each of them is sliced from, and the groups that those
//! boxes form. A column paints only its own lines, but a box split across
//! columns is sliced from one joined box and composites as one group, so the
//! preparation reads every line. It is made once for the paint call and shared,
//! and dropped when the last column of the paragraph has been painted.

use super::line_paint::{BlockPiece, LineShapes, PiecesByLine};
use super::typographic::Paragraph;
use raikiri_dom::{Document, Node, PositionedLines};
use raikiri_style::CascadeResult;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[cfg(test)]
mod tests;

/// Whether the parts of a paragraph share what is prepared for it, given that
/// `in_parts` says it is painted in parts.
fn shares(in_parts: bool) -> bool {
    #[cfg(test)]
    {
        in_parts && !tests::sharing_is_off()
    }
    #[cfg(not(test))]
    {
        in_parts
    }
}

/// What every fragmentainer of a paragraph paints from.
pub(super) struct Prepared {
    pub(super) pieces: Rc<PiecesByLine>,
    pub(super) shapes: Rc<LineShapes>,
}

struct Entry {
    pieces: Rc<PiecesByLine>,
    shapes: Option<Rc<LineShapes>>,
    /// How many fragmentainers are still to paint the paragraph.
    remaining: usize,
}

/// The preparations of the paragraphs that a paint call has painted in part.
#[derive(Default)]
pub(crate) struct ParagraphPreparations {
    entries: RefCell<HashMap<usize, Entry>>,
    /// Whether each paragraph painted in parts holds content positioned below
    /// its blocks.
    negative_content: RefCell<HashMap<usize, bool>>,
}

impl ParagraphPreparations {
    /// What the fragmentainer `fragmentainer` of the paragraph `paragraph`
    /// paints from, made from `positioned` and `blocks`, the boxes beside its
    /// lines.
    ///
    /// A paragraph painted in fragmentainers is kept until the last of them
    /// is painted. One that is painted again after that is prepared again.
    /// A paragraph painted whole, and one with boxes beside its lines, whose
    /// groups the boxes shape, are not kept.
    pub(super) fn prepare(
        &self,
        positioned: &PositionedLines<'_>,
        paragraph: Paragraph<'_>,
        root_node: &Node,
        blocks: &[BlockPiece],
        fragmentainer: Option<usize>,
    ) -> Prepared {
        let pieces_of = || {
            #[cfg(test)]
            {
                tests::note_work(tests::Work::Pieces, 1);
                tests::note_work(tests::Work::Lines, positioned.all_lines().len());
            }
            Rc::new(PiecesByLine::new(
                positioned,
                paragraph.document,
                paragraph.cascade,
                paragraph.root_id,
            ))
        };
        let shapes_of = |pieces: &PiecesByLine| {
            #[cfg(test)]
            tests::note_work(tests::Work::Shapes, 1);
            Rc::new(LineShapes::new(
                paragraph,
                root_node,
                &pieces.pieces,
                blocks,
            ))
        };
        if !shares(fragmentainer.is_some() && blocks.is_empty()) {
            let pieces = pieces_of();
            let shapes = shapes_of(&pieces);
            return Prepared { pieces, shapes };
        }
        let mut entries = self.entries.borrow_mut();
        let entry = entries.entry(paragraph.root_id).or_insert_with(|| Entry {
            pieces: pieces_of(),
            shapes: None,
            remaining: fragmentainers_of(root_node),
        });
        let pieces = Rc::clone(&entry.pieces);
        let shapes = Rc::clone(entry.shapes.get_or_insert_with(|| shapes_of(&pieces)));
        entry.remaining = entry.remaining.saturating_sub(1);
        if entry.remaining == 0 {
            entries.remove(&paragraph.root_id);
        }
        Prepared { pieces, shapes }
    }

    /// Whether the inline content of the paragraph `root` holds content
    /// positioned below its blocks, as
    /// [`raikiri_dom::paint_rules::holds_negative_stack_level`] finds. The
    /// answer is the same for every part of a paragraph painted `in_parts`,
    /// which asks once for each, so it is kept.
    pub(crate) fn holds_negative_stack_level(
        &self,
        document: &Document,
        cascade: &CascadeResult,
        root: usize,
        in_parts: bool,
    ) -> bool {
        let find = || {
            #[cfg(test)]
            tests::note_work(tests::Work::NegativeWalks, 1);
            raikiri_dom::paint_rules::holds_negative_stack_level(document, cascade, root)
        };
        if !shares(in_parts) {
            return find();
        }
        if let Some(&known) = self.negative_content.borrow().get(&root) {
            return known;
        }
        let found = find();
        self.negative_content.borrow_mut().insert(root, found);
        found
    }
}

/// How many fragmentainers hold lines of the paragraph `root_node`: the
/// number of paints that prepare it.
fn fragmentainers_of(root_node: &Node) -> usize {
    let mut held: Vec<_> = root_node
        .ifc_multicol_fragments()
        .unwrap_or_default()
        .iter()
        .map(|fragment| fragment.fragmentainer)
        .collect();
    held.sort_unstable();
    held.dedup();
    held.len().max(1)
}
