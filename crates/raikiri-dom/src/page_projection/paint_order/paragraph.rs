//! The content of a paragraph, or of one of its inline opacity groups, as
//! paint events: the boxes of its inline elements, its generated boxes and
//! images, its `::first-letter` pieces, and its text.

use super::{
    LineBoxes, LinePart, PageItems, PaintEvent, TextLines, generated_group, is_inline_group,
    pseudo_on_paragraph,
};
use crate::page_projection::generated_boxes::{BoxPiece, GeneratedBox, LineBox};
use crate::page_projection::generated_images::GeneratedImageId;
use crate::page_projection::records::PageFragmentKind;
use crate::{Document, Fragment, GeneratedKind, Node, TextLineId, paint_rules};
use raikiri_style::{CascadeResult, PseudoElem};
use raikiri_traits::{NodeId, NodeKind, PaintRect};
use std::collections::HashSet;
use std::rc::Rc;

/// Lists a paragraph's inline element boxes, then its text, in document order.
///
/// The painter draws a paragraph from its lines: the backgrounds of inline
/// elements first, then the text. Boxes laid out beside the lines (floats and
/// atomic inlines) are visited as the paragraph root's children instead, so
/// their subtrees are skipped here.
///
/// With `group`, only the content held by that inline opacity group is
/// listed: the group element's own boxes and generated boxes, and its
/// descendants'. Either way the subtree of each inline opacity group nested
/// directly in the listed content is skipped, and those groups are returned
/// in document order for the caller to list after this content.
///
/// Only the content on the lines of `part` is listed. An inline element
/// only on the first line, or only on later lines, lists its box with
/// those lines; one on both lists its line pieces on the lines of `part`.
pub(super) fn push_paragraph<'a>(
    document: &'a Document,
    items: &PageItems<'a>,
    root: usize,
    group: Option<usize>,
    lines: Option<&TextLines>,
    part: LinePart,
    events: &mut Vec<PaintEvent<'a>>,
) -> Vec<usize> {
    let Some(root_node) = document.ifc_layout_node(root) else {
        return Vec::new(); // cov:ignore: paragraph roots come from the arena
    };
    // The elements with boxes on the first line and on lines after it.
    let line_boxes = if part == LinePart::All {
        Rc::default()
    } else {
        items.line_boxes(root, root_node)
    };
    ParagraphListing {
        document,
        items,
        root,
        root_node,
        group,
        lines,
        part,
        line_boxes,
    }
    .list(events)
}

/// What the listing of a paragraph's content reads: the paragraph, the inline
/// opacity group whose content it lists, and the lines it covers.
struct ParagraphListing<'x, 'a> {
    document: &'a Document,
    items: &'x PageItems<'a>,
    root: usize,
    root_node: &'a Node,
    group: Option<usize>,
    lines: Option<&'x TextLines>,
    part: LinePart,
    /// The elements with boxes on the first line and on lines after it; empty
    /// when the listing covers every line.
    line_boxes: Rc<LineBoxes>,
}

/// The inline content of a paragraph or of one of its inline opacity groups,
/// in document order.
struct InlineContent {
    /// The elements whose own boxes or generated boxes are listed, each with
    /// the pseudo-element it stands for if it is not the element's own box.
    elements: Vec<(usize, Option<GeneratedKind>)>,
    /// The inline opacity groups nested directly in the content.
    nested: Vec<usize>,
    /// The text nodes of the content.
    texts: Vec<usize>,
}

/// One `::first-letter` piece listed under the element whose box it follows.
struct HeldLetter<'p> {
    /// The element the piece is listed under.
    anchor: usize,
    /// Whether the letters are the `::after` text of the anchor.
    after: bool,
    /// The block whose `::first-letter` the piece is.
    owner: usize,
    /// The element that holds the letters, which clips the piece.
    clip: usize,
    piece: &'p BoxPiece,
}

impl<'x, 'a> ParagraphListing<'x, 'a> {
    /// List the content, and return the inline opacity groups nested in it.
    fn list(&self, events: &mut Vec<PaintEvent<'a>>) -> Vec<usize> {
        if self.root_node.kind() == NodeKind::Text {
            if self.lines.is_some() {
                self.push_lines(events);
            } else {
                self.push_kind(events, self.root, PageFragmentKind::Text);
            }
            return Vec::new();
        }
        if let Some(group) = self.group
            && crate::generated_content::generated_origin(group).is_some()
        {
            return self.list_pseudo_group(group, events);
        }
        // The listed content starts at the group element itself, or inside
        // the paragraph root around the root's own generated content.
        let (first, children) = match self.group {
            Some(group) => {
                let Some(node) = self.document.get_node(group) else {
                    return Vec::new(); // cov:ignore: groups are arena elements
                };
                (group, node.children.as_slice())
            }
            None => (self.root, self.root_node.children.as_slice()),
        };
        let InlineContent {
            elements,
            mut nested,
            texts,
        } = self.collect_content(first, children);
        let pieces = first_letter_pieces(self.document, self.items, self.root);
        // A first letter with an opacity of its own is a group nested in the
        // group that holds its letters.
        for own in letter_groups(&pieces, self.lines, self.root, self.group) {
            if !nested.contains(&own) {
                nested.push(own);
            }
        }
        let letters = self.held_letters(&pieces, first, &elements);
        for (node_id, kind) in elements {
            match kind {
                None => self.list_box(events, node_id),
                Some(kind) => self.list_generated(events, node_id, kind, &letters, &mut nested),
            }
        }
        if self.lines.is_some() {
            self.push_lines(events);
        } else {
            for node_id in texts {
                self.push_kind(events, node_id, PageFragmentKind::Text);
            }
        }
        nested
    }

    /// Walk the inline content below `children`, starting at element `first`.
    /// The walk stops at the boxes laid out beside the lines and at the
    /// inline opacity groups, which are returned to list after the content.
    fn collect_content(&self, first: usize, children: &[usize]) -> InlineContent {
        let (document, items) = (self.document, self.items);
        let beside_lines: HashSet<usize> = self.root_node.ifc_boxes().into_iter().collect();
        let mut elements = Vec::new();
        if self.group.is_some() {
            elements.push((first, None));
        }
        elements.push((first, Some(GeneratedKind::Before)));
        // A `::first-letter` box wraps letters inside the element they come
        // from, so it follows that element's box and `::before` box, or its
        // `::after` box when the letters are that pseudo-element's.
        elements.push((first, Some(GeneratedKind::FirstLetter)));
        let mut nested = Vec::new();
        let mut texts = Vec::new();
        let mut stack: Vec<_> = children.iter().rev().map(|&id| (id, false)).collect();
        while let Some((node_id, after)) = stack.pop() {
            if after {
                elements.push((node_id, Some(GeneratedKind::After)));
                continue;
            }
            if beside_lines.contains(&node_id) {
                continue;
            }
            let Some(node) = document.get_node(node_id) else {
                continue; // cov:ignore: child ids come from the arena
            };
            if !node.is_in_document() || node.is_non_rendered_html_element() {
                continue;
            }
            match node.kind() {
                NodeKind::Element
                    if !node.is_display_none() && !node.is_hidden_by_text_overflow() =>
                {
                    if items.cascade.computed.get(node_id).is_some_and(|cv| {
                        is_inline_group(document, items.cascade, self.root, node_id, node, cv)
                    }) {
                        nested.push(node_id);
                        continue;
                    }
                    elements.push((node_id, None));
                    elements.push((node_id, Some(GeneratedKind::Before)));
                    elements.push((node_id, Some(GeneratedKind::FirstLetter)));
                    stack.push((node_id, true));
                    stack.extend(node.children.iter().rev().map(|&id| (id, false)));
                }
                NodeKind::Text => texts.push(node_id),
                _ => {}
            }
        }
        elements.push((first, Some(GeneratedKind::After)));
        InlineContent {
            elements,
            nested,
            texts,
        }
    }

    /// The `::first-letter` pieces this listing covers, each with the element
    /// it is listed under: those whose letters its inline opacity group
    /// holds, as it holds their glyph runs. A `::first-letter` box belongs to
    /// the block whose first letter it is, which can be an ancestor of the
    /// paragraph. Pieces whose letters come from no element listed here, such
    /// as the root's own text, go first.
    fn held_letters<'p>(
        &self,
        pieces: &[LetterPiece<'p>],
        first: usize,
        elements: &[(usize, Option<GeneratedKind>)],
    ) -> Vec<HeldLetter<'p>> {
        let listed: HashSet<usize> = elements
            .iter()
            .filter(|&&(id, kind)| kind.is_none() && id != first)
            .map(|&(id, _)| id)
            .collect();
        pieces
            .iter()
            .filter(|letter| letter.own.is_none() && letter.held == self.group)
            .map(|letter| HeldLetter {
                anchor: letter
                    .anchor
                    .filter(|anchor| listed.contains(anchor))
                    .unwrap_or(first),
                after: letter.after,
                owner: letter.owner,
                clip: letter.clip(),
                piece: letter.piece,
            })
            .collect()
    }

    /// List the box of inline element `node_id`, if it has one on these
    /// lines.
    fn list_box(&self, events: &mut Vec<PaintEvent<'a>>, node_id: usize) {
        if !self
            .items
            .cascade
            .computed
            .get(node_id)
            .is_some_and(paint_rules::generates_box)
        {
            return;
        }
        if let Some(pieces) = self.line_pieces(node_id) {
            self.push_line_boxes(events, node_id, pieces);
        } else if self.admits_box(node_id) {
            self.push_kind(events, node_id, PageFragmentKind::Box);
        }
    }

    /// List what follows from the `kind` pseudo-element slot of `node_id`: its
    /// generated boxes, the first letters taken from its text, and its images.
    /// A pseudo-element with an opacity of its own is a group instead, added
    /// to `nested`.
    fn list_generated(
        &self,
        events: &mut Vec<PaintEvent<'a>>,
        node_id: usize,
        kind: GeneratedKind,
        letters: &[HeldLetter<'a>],
        nested: &mut Vec<usize>,
    ) {
        let (document, items, root) = (self.document, self.items, self.root);
        // A pseudo-element with an opacity of its own is a group, listed
        // after this content with its boxes, images and text. Its
        // `::first-letter` slot still follows.
        if matches!(kind, GeneratedKind::Before | GeneratedKind::After)
            && let Some(pseudo) = generated_group(document, items.cascade, root, node_id, kind)
        {
            if pseudo_on_paragraph(items, root, node_id, kind, self.lines)
                && !nested.contains(&pseudo)
            {
                nested.push(pseudo);
            }
            return;
        }
        let own = items
            .generated_boxes
            .and_then(|boxes| boxes.get(&(root, node_id, kind)))
            .into_iter()
            .flatten()
            .filter(|_| kind != GeneratedKind::FirstLetter)
            .map(|piece| (node_id, kind, node_id, piece));
        // The first letters held by this element's text, or by its `::after`
        // text, which come after the `::after` box.
        let held = letters
            .iter()
            .filter(|letter| {
                letter.anchor == node_id
                    && match kind {
                        GeneratedKind::FirstLetter => !letter.after,
                        GeneratedKind::After => letter.after,
                        _ => false,
                    }
            })
            .map(|letter| {
                (
                    letter.owner,
                    GeneratedKind::FirstLetter,
                    letter.clip,
                    letter.piece,
                )
            });
        for (owner, kind, clip, piece) in own.chain(held) {
            self.push_generated_box(events, owner, kind, clip, piece);
        }
        // The `::before` images wait for the first-letter slot that always
        // follows, so that first-letter boxes come first.
        let after = match kind {
            GeneratedKind::FirstLetter => false,
            GeneratedKind::After => true,
            _ => return,
        };
        // The images of a `::before` group are listed in the group.
        if !after
            && generated_group(
                document,
                items.cascade,
                root,
                node_id,
                GeneratedKind::Before,
            )
            .is_some()
        {
            return;
        }
        // The images are replaced content inside the pseudo-element's inline
        // box, painted above its decoration and above the first-letter boxes
        // wrapping its letters, as the inline boxes of a line are painted
        // before its replaced content (CSS 2.1 Appendix E, inline-level
        // content in tree order).
        self.push_generated_images(events, node_id, after);
    }

    /// List the content of the opacity group of a pseudo-element, named by its
    /// generated id: the decoration of a `::before` or `::after` with the
    /// first letters taken from its text and its images, or the pieces of a
    /// `::first-letter`; then the group's text. Returns the groups of the
    /// first letters nested in it.
    fn list_pseudo_group(&self, group: usize, events: &mut Vec<PaintEvent<'a>>) -> Vec<usize> {
        let Some((element, pseudo)) = crate::generated_content::generated_origin(group) else {
            return Vec::new(); // cov:ignore: the caller checks the id
        };
        let letters = first_letter_pieces(self.document, self.items, self.root);
        let mut nested = Vec::new();
        match pseudo {
            PseudoElem::FirstLetter => {
                for letter in letters.iter().filter(|letter| letter.own == Some(group)) {
                    self.push_generated_box(
                        events,
                        letter.owner,
                        GeneratedKind::FirstLetter,
                        letter.clip(),
                        letter.piece,
                    );
                }
                // The first letter of a nested block wraps letters inside
                // this one's.
                nested = letter_groups(&letters, self.lines, self.root, Some(group));
            }
            PseudoElem::Before | PseudoElem::After => {
                nested = self.list_pseudo_box(group, element, pseudo, &letters, events);
            }
            _ => {} // cov:ignore: only these pseudo-elements form groups
        }
        self.push_lines(events);
        nested
    }

    /// List the content of the group of the `::before` or `::after` of
    /// `element`: its generated boxes, then the first letters taken from its
    /// text, which wrap them inside its box, then its images. Returns the
    /// groups of the first letters nested in it.
    fn list_pseudo_box(
        &self,
        group: usize,
        element: usize,
        pseudo: PseudoElem,
        letters: &[LetterPiece<'a>],
        events: &mut Vec<PaintEvent<'a>>,
    ) -> Vec<usize> {
        let (kind, after) = if pseudo == PseudoElem::After {
            (GeneratedKind::After, true)
        } else {
            (GeneratedKind::Before, false)
        };
        for piece in self
            .items
            .generated_boxes
            .and_then(|boxes| boxes.get(&(self.root, element, kind)))
            .into_iter()
            .flatten()
        {
            self.push_generated_box(events, element, kind, element, piece);
        }
        for letter in letters
            .iter()
            .filter(|letter| letter.own.is_none() && letter.held == Some(group))
        {
            self.push_generated_box(
                events,
                letter.owner,
                GeneratedKind::FirstLetter,
                letter.clip(),
                letter.piece,
            );
        }
        let nested = letter_groups(letters, self.lines, self.root, Some(group));
        self.push_generated_images(events, element, after);
        nested
    }

    /// Whether the box of inline element `element` goes with the lines this
    /// listing covers, rather than with the others.
    fn admits_box(&self, element: usize) -> bool {
        match self.part {
            LinePart::All => true,
            LinePart::First => !self.line_boxes.later.contains(&element),
            LinePart::Rest => self.line_boxes.later.contains(&element),
        }
    }

    /// The pieces of the box of `element`, one for each line it is on, when
    /// the box is listed by its line pieces. An element on the first line and
    /// on later ones is split by line: its pieces on the lines covered stand
    /// for its box. So are the pieces of an element whose color or font size
    /// the `::first-line` changes, even where it has no box on later lines.
    /// Without prepared pieces on the page, its box goes with the other lines.
    fn line_pieces(&self, element: usize) -> Option<&'a Vec<BoxPiece>> {
        let pieces = self.items.inline_boxes?.get(&(self.root, element))?;
        ((self.part != LinePart::All
            && self.line_boxes.later.contains(&element)
            && self.line_boxes.first.contains(&element))
            || crate::layout::first_line_restyles(self.root_node, self.items.cascade, element))
        .then_some(pieces)
    }

    /// List the box of `element` as one event for each of its `pieces` on the
    /// lines covered.
    fn push_line_boxes(
        &self,
        events: &mut Vec<PaintEvent<'a>>,
        element: usize,
        pieces: &[BoxPiece],
    ) {
        let items = self.items;
        let boxes: Vec<Fragment<'a>> = items
            .of(element)
            .iter()
            .filter(|item| item.kind == PageFragmentKind::Box)
            .map(|&item| items.fragment(item))
            .collect();
        // The pieces come in line order and the boxes in flow order, so the
        // search for each piece's box resumes where the last one ended.
        let mut cursor = 0;
        for piece in pieces.iter().filter(|piece| self.part.admits(piece.line)) {
            // The element's box on the page that holds the piece, such as
            // its box in the piece's column; a piece outside every box later
            // in the flow goes with the box of the last piece.
            if let Some(next) = boxes[cursor..]
                .iter()
                .position(|fragment| contains(fragment.paint_rect(), piece.rect))
            {
                cursor += next;
            }
            let Some(&fragment) = boxes.get(cursor) else {
                return; // cov:ignore: an element with line pieces on the page has a box there
            };
            // The first line's boxes take their values from the
            // `::first-line` around them.
            let style = self
                .root_node
                .ifc_first_line_element_style(element)
                .filter(|_| piece.line == 0)
                .unwrap_or(&items.cascade.computed[element]);
            events.push(PaintEvent::LineBox(LineBox {
                fragment,
                style,
                line: TextLineId {
                    root: NodeId::new(self.root as u64),
                    index: piece.line,
                },
                rect: piece.rect,
                has_start_edge: piece.has_start_edge,
                has_end_edge: piece.has_end_edge,
            }));
        }
    }

    /// List the fragments of `node_id` of the given kind.
    fn push_kind(&self, events: &mut Vec<PaintEvent<'a>>, node_id: usize, kind: PageFragmentKind) {
        events.extend(
            self.items
                .of(node_id)
                .iter()
                .filter(|item| item.kind == kind)
                .map(|&item| {
                    let fragment = self.items.fragment(item);
                    match kind {
                        PageFragmentKind::Text => PaintEvent::Text(fragment),
                        _ => PaintEvent::Box(fragment),
                    }
                }),
        );
    }

    /// List the supplied lines of the paragraph, or of the inline opacity
    /// group, that the listing covers.
    fn push_lines(&self, events: &mut Vec<PaintEvent<'a>>) {
        let (root, part) = (self.root, self.part);
        let Some(lines) = self.lines else { return };
        match self.group {
            Some(group) => {
                let owner = NodeId::new(group as u64);
                events.extend(
                    lines
                        .groups
                        .get(&owner)
                        .into_iter()
                        .flatten()
                        .filter(|line| line.root.0 == root as u64 && part.admits(line.index))
                        .map(|&line| PaintEvent::GroupTextLine(line, owner)),
                );
            }
            None => events.extend(
                lines
                    .paragraphs
                    .get(&NodeId::new(root as u64))
                    .into_iter()
                    .flatten()
                    .filter(|line| part.admits(line.index))
                    .copied()
                    .map(PaintEvent::TextLine),
            ),
        }
    }

    /// List one generated box piece of the lines covered: `owner` is the
    /// element of the pseudo-element, `kind` which one it is, and `clip` the
    /// element whose overflow clips it.
    fn push_generated_box(
        &self,
        events: &mut Vec<PaintEvent<'a>>,
        owner: usize,
        kind: GeneratedKind,
        clip: usize,
        piece: &BoxPiece,
    ) {
        if !self.part.admits(piece.line) {
            return;
        }
        let Some(style) = column_projection_style(self.root_node, self.items.cascade, piece) else {
            return; // cov:ignore: pieces are prepared only for boxes with a style
        };
        events.push(PaintEvent::GeneratedBox(GeneratedBox {
            owner: NodeId::new(owner as u64),
            line: TextLineId {
                root: NodeId::new(self.root as u64),
                index: piece.line,
            },
            kind,
            rect: piece.rect,
            style,
            clip_owner: NodeId::new(clip as u64),
            has_start_edge: piece.has_start_edge,
            has_end_edge: piece.has_end_edge,
            has_block_start_edge: piece.has_block_start_edge,
            has_block_end_edge: piece.has_block_end_edge,
        }));
    }

    /// List the images of the `::before` (or, with `after`, the `::after`) of
    /// `element` that sit on the lines covered.
    fn push_generated_images(&self, events: &mut Vec<PaintEvent<'a>>, element: usize, after: bool) {
        events.extend(
            self.items
                .generated_images
                .into_iter()
                .flat_map(|images| images.range((element, after, 0)..=(element, after, usize::MAX)))
                .filter(|(_, placement)| {
                    placement.root == self.root && self.part.admits(placement.line)
                })
                .map(|(&(owner, after, index), _)| {
                    PaintEvent::GeneratedImage(GeneratedImageId::new(owner, after, index))
                }),
        );
    }
}

/// The element of paragraph `root` whose letters a `::first-letter` piece
/// wraps: the parent of the text, or the originating element of the
/// `::before` or `::after` text, they come from; which of those
/// pseudo-elements, if any, the letters are the text of; and the node that
/// supplies the letters.
fn first_letter_anchor(
    document: &Document,
    root: usize,
    piece: &BoxPiece,
) -> Option<(usize, Option<GeneratedKind>, usize)> {
    let source = document
        .ifc_layout_node(root)?
        .ifc_typographic_fragment(piece.node, piece.source_container, piece.source_owner)?
        .1;
    if let Some((element, pseudo)) = crate::generated_content::generated_origin(source) {
        let kind = match pseudo {
            PseudoElem::After => GeneratedKind::After,
            _ => GeneratedKind::Before,
        };
        return Some((element, Some(kind), source));
    }
    match document.get_node(source)?.kind() {
        NodeKind::Text => document
            .parent_of(source)
            .map(|parent| (parent, None, source)),
        _ => Some((source, None, source)), // cov:ignore: letters come from text or generated content
    }
}

/// One `::first-letter` piece of a paragraph, with the opacity groups
/// around it.
struct LetterPiece<'p> {
    /// The element whose letters the piece wraps, when known.
    anchor: Option<usize>,
    /// Whether the letters are the `::after` text of the anchor.
    after: bool,
    /// The block whose `::first-letter` the piece is.
    owner: usize,
    piece: &'p BoxPiece,
    /// The innermost opacity group around the letters, other than the
    /// piece's own: a `::before` or `::after` group they are the text of,
    /// or the inline group of the anchor.
    held: Option<usize>,
    /// The piece's own opacity group, when its `::first-letter` has one.
    own: Option<usize>,
}

impl LetterPiece<'_> {
    /// The element that holds the letters, which clips the piece.
    fn clip(&self) -> usize {
        self.anchor.unwrap_or(self.owner)
    }
}

/// The `::first-letter` pieces of paragraph `root` on the page, in paint
/// order.
fn first_letter_pieces<'p>(
    document: &Document,
    items: &PageItems<'p>,
    root: usize,
) -> Vec<LetterPiece<'p>> {
    let groups = items.letter_groups(root);
    items
        .generated_boxes
        .into_iter()
        .flat_map(|boxes| {
            boxes.range(
                (root, 0, GeneratedKind::FirstLetter)
                    ..=(root, usize::MAX, GeneratedKind::FirstLetter),
            )
        })
        .filter(|((_, _, kind), _)| *kind == GeneratedKind::FirstLetter)
        .flat_map(|(&(_, owner, _), pieces)| pieces.iter().map(move |piece| (owner, piece)))
        .map(|(owner, piece)| {
            let source = first_letter_anchor(document, root, piece);
            let anchor = source.map(|(anchor, _, _)| anchor);
            let pseudo = source.and_then(|(_, pseudo, _)| pseudo);
            // The paragraph retains the fragment of every piece.
            let (own, held) = source
                .and_then(|(_, _, letters)| groups.fragment(piece.node, letters))
                .unwrap_or_default();
            LetterPiece {
                anchor,
                after: pseudo == Some(GeneratedKind::After),
                owner,
                piece,
                held,
                own,
            }
        })
        .collect()
}

/// The `::first-letter` groups of paragraph `root` nested directly in
/// `group`: those of the pieces whose letters it holds, and those of the
/// supplied runs, in that order.
fn letter_groups(
    pieces: &[LetterPiece<'_>],
    lines: Option<&TextLines>,
    root: usize,
    group: Option<usize>,
) -> Vec<usize> {
    let mut found: Vec<usize> = pieces
        .iter()
        .filter(|letter| letter.held == group)
        .filter_map(|letter| letter.own)
        .collect();
    let mut from_runs: Vec<usize> = lines
        .into_iter()
        .flat_map(|lines| &lines.letters)
        .filter(|&(&(letter_root, _), &held)| letter_root == root && held == group)
        .map(|(&(_, letter), _)| letter)
        .collect();
    from_runs.sort_unstable();
    found.extend(from_runs);
    let mut seen = HashSet::new();
    found.retain(|&group| seen.insert(group));
    found
}

/// Whether rectangle `outer` contains the centre of `inner`.
fn contains(outer: PaintRect, inner: PaintRect) -> bool {
    let (x, y) = (inner.x + inner.width * 0.5, inner.y + inner.height * 0.5);
    x >= outer.x && x <= outer.x + outer.width && y >= outer.y && y <= outer.y + outer.height
}

/// The style of a prepared generated box piece.
fn column_projection_style<'a>(
    root: &'a crate::Node,
    cascade: &'a CascadeResult,
    piece: &BoxPiece,
) -> Option<&'a raikiri_style::ComputedValues> {
    crate::layout::generated_piece_style(
        root,
        cascade,
        piece.node,
        piece.source_container,
        piece.source_owner,
        piece.line,
    )
}
