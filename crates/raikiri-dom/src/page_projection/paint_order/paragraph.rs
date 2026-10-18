//! The content of a paragraph, or of one of its inline opacity groups, as
//! paint events: the boxes of its inline elements, its generated boxes and
//! images, its `::first-letter` pieces, and its text.

use super::{
    LinePart, PageItems, PaintEvent, TextLines, generated_group, is_inline_group,
    pseudo_on_paragraph,
};
use crate::page_projection::generated_boxes::{BoxPiece, GeneratedBox, LineBox};
use crate::page_projection::generated_images::GeneratedImageId;
use crate::page_projection::records::PageFragmentKind;
use crate::{Document, Fragment, GeneratedKind, TextLineId, paint_rules};
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
    let lines_of_boxes = if part == LinePart::All {
        Rc::default()
    } else {
        items.line_boxes(root, root_node)
    };
    let later = &lines_of_boxes.later;
    let admits_box = |element: usize| match part {
        LinePart::All => true,
        LinePart::First => !later.contains(&element),
        LinePart::Rest => later.contains(&element),
    };
    // An element on the first line and on later ones is split by line: its
    // pieces on the lines of `part` stand for its box. So are the pieces of
    // an element whose color or font size the `::first-line` changes, even
    // where it has no box on later lines. Without prepared pieces on the
    // page, its box goes with the other lines.
    let line_pieces = |element: usize| {
        let pieces = items.inline_boxes?.get(&(root, element))?;
        ((part != LinePart::All
            && later.contains(&element)
            && lines_of_boxes.first.contains(&element))
            || crate::layout::first_line_restyles(root_node, items.cascade, element))
        .then_some(pieces)
    };
    let push_line_boxes =
        |events: &mut Vec<PaintEvent<'a>>, element: usize, pieces: &[BoxPiece]| {
            let boxes: Vec<Fragment<'a>> = items
                .of(element)
                .iter()
                .filter(|item| item.kind == PageFragmentKind::Box)
                .map(|&item| items.fragment(item))
                .collect();
            // The pieces come in line order and the boxes in flow order, so the
            // search for each piece's box resumes where the last one ended.
            let mut cursor = 0;
            for piece in pieces.iter().filter(|piece| part.admits(piece.line)) {
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
                let style = root_node
                    .ifc_first_line_element_style(element)
                    .filter(|_| piece.line == 0)
                    .unwrap_or(&items.cascade.computed[element]);
                events.push(PaintEvent::LineBox(LineBox {
                    fragment,
                    style,
                    line: TextLineId {
                        root: NodeId::new(root as u64),
                        index: piece.line,
                    },
                    rect: piece.rect,
                    has_start_edge: piece.has_start_edge,
                    has_end_edge: piece.has_end_edge,
                }));
            }
        };
    let push_kind = |events: &mut Vec<PaintEvent<'a>>, node_id: usize, kind| {
        for &item in items.of(node_id) {
            if item.kind == kind {
                let fragment = items.fragment(item);
                events.push(match kind {
                    PageFragmentKind::Text => PaintEvent::Text(fragment),
                    _ => PaintEvent::Box(fragment),
                });
            }
        }
    };
    let push_lines = |events: &mut Vec<PaintEvent<'a>>| {
        let Some(lines) = lines else { return };
        match group {
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
    };
    if root_node.kind() == NodeKind::Text {
        if lines.is_some() {
            push_lines(events);
        } else {
            push_kind(events, root, PageFragmentKind::Text);
        }
        return Vec::new();
    }
    if let Some(group) = group
        && crate::generated_content::generated_origin(group).is_some()
    {
        return push_pseudo_group(
            document,
            items,
            root,
            group,
            lines,
            part,
            &push_lines,
            events,
        );
    }
    let beside_lines: HashSet<usize> = root_node.ifc_boxes().into_iter().collect();
    // The listed content starts at the group element itself, or inside the
    // paragraph root around the root's own generated content.
    let (first, children) = match group {
        Some(group) => {
            let Some(node) = document.get_node(group) else {
                return Vec::new(); // cov:ignore: groups are arena elements
            };
            (group, node.children.as_slice())
        }
        None => (root, root_node.children.as_slice()),
    };
    let mut elements = Vec::new();
    if group.is_some() {
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
            NodeKind::Element if !node.is_display_none() && !node.is_hidden_by_text_overflow() => {
                if items.cascade.computed.get(node_id).is_some_and(|cv| {
                    is_inline_group(document, items.cascade, root, node_id, node, cv)
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
    let root_node_ifc = document.ifc_layout_node(root);
    // The `::first-letter` pieces this pass lists: those whose letters its
    // inline opacity group holds, as it holds their glyph runs. A
    // `::first-letter` box belongs to the block whose first letter it is,
    // which can be an ancestor of the paragraph. Pieces whose letters come
    // from no element listed here, such as the root's own text, go first.
    let listed: HashSet<usize> = elements
        .iter()
        .filter(|&&(id, kind)| kind.is_none() && id != first)
        .map(|&(id, _)| id)
        .collect();
    let pieces = first_letter_pieces(document, items, root);
    // A first letter with an opacity of its own is a group nested in the
    // group that holds its letters.
    for own in letter_groups(&pieces, lines, root, group) {
        if !nested.contains(&own) {
            nested.push(own);
        }
    }
    // Each first-letter piece with the element it is listed under, whether
    // its letters come from `::after`, its owner, and the element that holds
    // its letters, which clips it.
    let letters: Vec<(usize, bool, usize, usize, &BoxPiece)> = pieces
        .iter()
        .filter(|letter| letter.own.is_none() && letter.held == group)
        .map(|letter| {
            let clip = letter.anchor.unwrap_or(letter.owner);
            let anchor = letter
                .anchor
                .filter(|anchor| listed.contains(anchor))
                .unwrap_or(first);
            (anchor, letter.after, letter.owner, clip, letter.piece)
        })
        .collect();
    for (node_id, kind) in elements {
        match kind {
            None if items
                .cascade
                .computed
                .get(node_id)
                .is_some_and(paint_rules::generates_box) =>
            {
                if let Some(pieces) = line_pieces(node_id) {
                    push_line_boxes(events, node_id, pieces);
                } else if admits_box(node_id) {
                    push_kind(events, node_id, PageFragmentKind::Box);
                }
            }
            None => {}
            Some(kind) => {
                // A pseudo-element with an opacity of its own is a group,
                // listed after this content with its boxes, images and text.
                // Its `::first-letter` slot still follows.
                if matches!(kind, GeneratedKind::Before | GeneratedKind::After)
                    && let Some(pseudo) =
                        generated_group(document, items.cascade, root, node_id, kind)
                {
                    if pseudo_on_paragraph(items, root, node_id, kind, lines)
                        && !nested.contains(&pseudo)
                    {
                        nested.push(pseudo);
                    }
                    continue;
                }
                let own = items
                    .generated_boxes
                    .and_then(|boxes| boxes.get(&(root, node_id, kind)))
                    .into_iter()
                    .flatten()
                    .filter(|_| kind != GeneratedKind::FirstLetter)
                    .map(|piece| (node_id, kind, node_id, piece));
                // The first letters held by this element's text, or by its
                // `::after` text, which come after the `::after` box.
                let held = letters
                    .iter()
                    .filter(|&&(anchor, after, _, _, _)| {
                        anchor == node_id
                            && match kind {
                                GeneratedKind::FirstLetter => !after,
                                GeneratedKind::After => after,
                                _ => false,
                            }
                    })
                    .map(|&(_, _, owner, clip, piece)| {
                        (owner, GeneratedKind::FirstLetter, clip, piece)
                    });
                for (owner, kind, clip, piece) in own.chain(held) {
                    if !part.admits(piece.line) {
                        continue;
                    }
                    let owner_id = NodeId::new(owner as u64);
                    let Some(style) = root_node_ifc
                        .and_then(|node| column_projection_style(node, items.cascade, piece))
                    else {
                        continue; // cov:ignore: pieces are prepared only for boxes with a style
                    };
                    events.push(PaintEvent::GeneratedBox(GeneratedBox {
                        owner: owner_id,
                        line: TextLineId {
                            root: NodeId::new(root as u64),
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
                // The `::before` images wait for the first-letter slot that
                // always follows, so that first-letter boxes come first.
                let after = match kind {
                    GeneratedKind::FirstLetter => false,
                    GeneratedKind::After => true,
                    _ => continue,
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
                    continue;
                }
                // The images are replaced content inside the pseudo-element's
                // inline box, painted above its decoration and above the
                // first-letter boxes wrapping its letters, as the inline
                // boxes of a line are painted before its replaced content
                // (CSS 2.1 Appendix E, inline-level content in tree order).
                events.extend(
                    items
                        .generated_images
                        .into_iter()
                        .flat_map(|images| {
                            images.range((node_id, after, 0)..=(node_id, after, usize::MAX))
                        })
                        .filter(|(_, placement)| {
                            placement.root == root && part.admits(placement.line)
                        })
                        .map(|(&(owner, after, index), _)| {
                            PaintEvent::GeneratedImage(GeneratedImageId::new(owner, after, index))
                        }),
                );
            }
        }
    }
    if lines.is_some() {
        push_lines(events);
    } else {
        for node_id in texts {
            push_kind(events, node_id, PageFragmentKind::Text);
        }
    }
    nested
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

/// Lists the content of the opacity group of a pseudo-element of paragraph
/// `root`, named by its generated id: the decoration of a `::before` or
/// `::after` with the first letters taken from its text and its images, or
/// the pieces of a `::first-letter`; then the group's text. Returns the
/// groups of the first letters nested in it.
#[allow(clippy::too_many_arguments)]
fn push_pseudo_group<'a>(
    document: &'a Document,
    items: &PageItems<'a>,
    root: usize,
    group: usize,
    lines: Option<&TextLines>,
    part: LinePart,
    push_lines: &dyn Fn(&mut Vec<PaintEvent<'a>>),
    events: &mut Vec<PaintEvent<'a>>,
) -> Vec<usize> {
    let Some((element, pseudo)) = crate::generated_content::generated_origin(group) else {
        return Vec::new(); // cov:ignore: the caller checks the id
    };
    let root_node = document.ifc_layout_node(root);
    let push_piece = |events: &mut Vec<PaintEvent<'a>>,
                      owner: usize,
                      kind: GeneratedKind,
                      clip: usize,
                      piece: &BoxPiece| {
        if !part.admits(piece.line) {
            return;
        }
        let Some(style) =
            root_node.and_then(|node| column_projection_style(node, items.cascade, piece))
        else {
            return; // cov:ignore: pieces are prepared only for boxes with a style
        };
        events.push(PaintEvent::GeneratedBox(GeneratedBox {
            owner: NodeId::new(owner as u64),
            line: TextLineId {
                root: NodeId::new(root as u64),
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
    };
    let letters = first_letter_pieces(document, items, root);
    let mut nested = Vec::new();
    match pseudo {
        PseudoElem::FirstLetter => {
            for letter in letters.iter().filter(|letter| letter.own == Some(group)) {
                let clip = letter.anchor.unwrap_or(letter.owner);
                push_piece(
                    events,
                    letter.owner,
                    GeneratedKind::FirstLetter,
                    clip,
                    letter.piece,
                );
            }
            // The first letter of a nested block wraps letters inside this
            // one's.
            nested = letter_groups(&letters, lines, root, Some(group));
        }
        PseudoElem::Before | PseudoElem::After => {
            let (kind, after) = if pseudo == PseudoElem::After {
                (GeneratedKind::After, true)
            } else {
                (GeneratedKind::Before, false)
            };
            for piece in items
                .generated_boxes
                .and_then(|boxes| boxes.get(&(root, element, kind)))
                .into_iter()
                .flatten()
            {
                push_piece(events, element, kind, element, piece);
            }
            // The first letters taken from the pseudo-element's text wrap
            // them inside its box, below its images.
            for letter in letters
                .iter()
                .filter(|letter| letter.own.is_none() && letter.held == Some(group))
            {
                push_piece(
                    events,
                    letter.owner,
                    GeneratedKind::FirstLetter,
                    letter.anchor.unwrap_or(letter.owner),
                    letter.piece,
                );
            }
            for own in letter_groups(&letters, lines, root, Some(group)) {
                if !nested.contains(&own) {
                    nested.push(own);
                }
            }
            events.extend(
                items
                    .generated_images
                    .into_iter()
                    .flat_map(|images| {
                        images.range((element, after, 0)..=(element, after, usize::MAX))
                    })
                    .filter(|(_, placement)| placement.root == root && part.admits(placement.line))
                    .map(|(&(owner, after, index), _)| {
                        PaintEvent::GeneratedImage(GeneratedImageId::new(owner, after, index))
                    }),
            );
        }
        _ => {} // cov:ignore: only these pseudo-elements form groups
    }
    push_lines(events);
    nested
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
