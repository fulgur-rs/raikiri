use super::super::oof_index::InlinePaddingBoxes;
use super::*;
use crate::layout::ifc::inline_boxes::InlineBoxPiece;
use crate::layout::test_support::{
    ahem_paragraph_with, page_box_800x600, with_ahem, with_resumed_session,
};

#[derive(Clone, Copy, Debug)]
struct Work {
    pieces: usize,
    piece_visits: usize,
    padding_boxes: usize,
    offset_steps: usize,
}

fn reset() {
    OOF_PIECE_VISITS.with(|count| count.set(0));
    OOF_PADDING_BOX_BUILDS.with(|count| count.set(0));
    OOF_OFFSET_STEPS.with(|count| count.set(0));
    OOF_PLACE_CALLS.with(|count| count.set(0));
}

/// Lay `document` out and read the work placing its out-of-flow boxes did.
fn laid_out(
    document: &mut crate::Document,
    cascade: &raikiri_style::CascadeResult,
    root: usize,
) -> Work {
    with_ahem(document);
    reset();
    crate::layout_single_page(document, cascade, page_box_800x600()).unwrap();
    assert!(
        OOF_PLACE_CALLS.with(|count| count.get()) > 0,
        "the production placement path must run"
    );
    Work {
        pieces: document.nodes[root]
            .ifc_inline_boxes()
            .expect("pieces")
            .len(),
        piece_visits: OOF_PIECE_VISITS.with(|count| count.get()),
        padding_boxes: OOF_PADDING_BOX_BUILDS.with(|count| count.get()),
        offset_steps: OOF_OFFSET_STEPS.with(|count| count.get()),
    }
}

/// One paragraph with `count` relatively positioned inlines of one "x " each,
/// every one holding an absolutely positioned box pinned to its padding box's
/// corner. Returns the work done and the placed boxes with their inlines.
fn distinct_inlines(count: usize) -> (Work, Vec<(usize, usize)>, crate::Document) {
    let mut placed = Vec::new();
    let (mut document, cascade, root) =
        ahem_paragraph_with("position:relative;width:800px", |doc, root| {
            for _ in 0..count {
                let inline =
                    doc.append_element(Some(root), "span", Some("position:relative;left:1px"));
                doc.append_text(inline, "x ");
                let child = doc.append_element(
                    Some(inline),
                    "span",
                    Some("position:absolute;left:0;top:0;width:1px;height:1px"),
                );
                placed.push((inline, child));
            }
        });
    let work = laid_out(&mut document, &cascade, root);
    (work, placed, document)
}

#[test]
fn boxes_in_many_inlines_read_each_piece_a_constant_number_of_times() {
    for count in [128, 256, 512, 1024] {
        let (work, placed, document) = distinct_inlines(count);
        eprintln!("distinct count={count} work={work:?}");
        // Every element is reduced once, from a group of the pieces made once.
        assert_eq!(work.padding_boxes, count, "{work:?}");
        assert!(
            work.pieces >= count && work.piece_visits <= 2 * work.pieces,
            "{count}: {work:?}"
        );
        // One ancestor step per box when placing it and when moving it.
        assert!(work.offset_steps <= 4 * count, "{count}: {work:?}");
        let root = document.parent_of(placed[0].0).expect("parent");
        let pieces = document.nodes[root].ifc_inline_boxes().expect("pieces");
        for (inline, child) in placed {
            // The box sits at the corner of its element's padding box, the
            // bounding box of the pieces of the element on its lines, which
            // the element's own offset moves along with it.
            let (left, top) = pieces
                .iter()
                .filter(|piece| piece.node == inline)
                .fold((f32::MAX, f32::MAX), |(left, top), piece| {
                    (left.min(piece.border_box.x), top.min(piece.border_box.y))
                });
            let location = document.nodes[child].layout.location;
            assert_eq!((location.x, location.y), (left + 1.0, top), "{inline}");
        }
    }
}

#[test]
fn boxes_in_one_inline_share_its_padding_box() {
    for count in [128, 256, 512, 1024] {
        let mut children = Vec::new();
        let (mut document, cascade, root) =
            ahem_paragraph_with("position:relative;width:800px", |doc, root| {
                let inline = doc.append_element(Some(root), "span", Some("position:relative"));
                doc.append_text(inline, "xx ");
                for _ in 0..count {
                    children.push(doc.append_element(
                        Some(inline),
                        "span",
                        Some("position:absolute;left:50%;top:0;width:1px;height:1px"),
                    ));
                }
                doc.append_text(inline, "xx");
            });
        let work = laid_out(&mut document, &cascade, root);
        eprintln!("shared count={count} work={work:?}");
        assert_eq!(work.padding_boxes, 1, "{work:?}");
        assert!(work.piece_visits <= 2 * work.pieces, "{count}: {work:?}");
        // The element is "xx xx": 50px wide, and 50% of it is 25px.
        for child in children {
            let location = document.nodes[child].layout.location;
            assert_eq!((location.x, location.y), (25.0, 0.0));
        }
    }
}

#[test]
fn a_box_in_a_nested_relative_inline_adds_the_offsets_of_its_containing_inlines() {
    let mut child = 0;
    let (mut document, cascade, root) =
        ahem_paragraph_with("position:relative;width:800px", |doc, root| {
            let mut parent = root;
            for (index, css) in [
                "position:relative;left:3px;top:1px",
                "position:relative;left:5px",
                "position:static",
                "position:relative;top:7px;left:11px",
            ]
            .into_iter()
            .enumerate()
            {
                let inline = doc.append_element(Some(parent), "span", Some(css));
                doc.append_text(inline, if index == 0 { "xxxx " } else { "x" });
                parent = inline;
            }
            child = doc.append_element(
                Some(parent),
                "span",
                Some("position:absolute;left:0;top:0;width:1px;height:1px"),
            );
        });
    laid_out(&mut document, &cascade, root);
    // The innermost positioned element starts after "xxxx " and two more
    // "x", at x 70, and every offset of an element around it moves it too
    // (CSS 2.1 9.4.3): 3 + 5 + 11 and 1 + 7.
    let location = document.nodes[child].layout.location;
    assert_eq!((location.x, location.y), (70.0 + 19.0, 8.0));
}

/// The padding box of `inline` as the scan of every piece makes it: the
/// bounding box of the padding boxes of its pieces, or of the pieces of the
/// elements in it when it has none.
fn scanned_padding_box(
    tree: &crate::layout::context::LayoutSession<'_>,
    pieces: &[InlineBoxPiece],
    inline: usize,
    line_shift: impl Fn(usize) -> (f32, f32),
) -> Option<(f32, f32, f32, f32)> {
    let computed = &tree.style_views.cascade().computed[inline].border;
    let mut border = raikiri_layout_geometry::Edges {
        top: computed.top.width().px(),
        right: computed.right.width().px(),
        bottom: computed.bottom.width().px(),
        left: computed.left.width().px(),
    };
    let mut spanned: Vec<_> = pieces.iter().filter(|piece| piece.node == inline).collect();
    if spanned.is_empty() {
        border = raikiri_layout_geometry::Edges::ZERO;
        spanned = pieces
            .iter()
            .filter(|piece| tree.ancestors(piece.node).any(|id| id == inline))
            .collect();
    }
    let (left, top, right, bottom) = spanned
        .into_iter()
        .map(|piece| {
            let (outer, inner) = (piece.border_box, piece.content_box);
            let (dx, dy) = line_shift(piece.line);
            (
                outer.x + dx + border.left.min((inner.x - outer.x).max(0.0)),
                outer.y + dy + border.top.min((inner.y - outer.y).max(0.0)),
                outer.x + dx + outer.width
                    - border
                        .right
                        .min((outer.x + outer.width - inner.x - inner.width).max(0.0)),
                outer.y + dy + outer.height
                    - border
                        .bottom
                        .min((outer.y + outer.height - inner.y - inner.height).max(0.0)),
            )
        })
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))?;
    Some((left, top, right - left, bottom - top))
}

/// Check that the grouped pieces give the padding box of every element under
/// `root` that the scan of every piece does, with the lines moved.
fn assert_padding_boxes_match_the_scan(
    mut document: crate::Document,
    cascade: raikiri_style::CascadeResult,
    root: usize,
) -> Vec<InlineBoxPiece> {
    with_ahem(&mut document);
    crate::layout_single_page(&mut document, &cascade, page_box_800x600()).unwrap();
    let pieces = document.nodes[root].ifc_inline_boxes().expect("pieces");
    assert!(!pieces.is_empty());
    let elements: Vec<usize> = document.preorder(root).skip(1).collect();
    let shift = |line: usize| (0.5 * line as f32, 0.25 * line as f32 + 1.0);
    let mut spanning_none = 0;
    let mut checked = 0;
    with_resumed_session(&mut document, |session| {
        let owner = session.ifc_source_owner(root);
        // Ask in two orders: the answers do not depend on what came before.
        for order in [elements.clone(), elements.iter().rev().copied().collect()] {
            let mut grouped = InlinePaddingBoxes::new(pieces.clone());
            for &inline in &order {
                let scanned = scanned_padding_box(session, &pieces, inline, shift);
                let made = grouped
                    .padding_box(session, (root, owner), inline, shift)
                    .map(|b| (b.x, b.y, b.width, b.height));
                assert_eq!(
                    made.map(|b| [b.0, b.1, b.2, b.3].map(f32::to_bits)),
                    scanned.map(|b| [b.0, b.1, b.2, b.3].map(f32::to_bits)),
                    "{inline}: {made:?} {scanned:?}"
                );
                // Asking again reads the same answer.
                let again = grouped
                    .padding_box(session, (root, owner), inline, shift)
                    .map(|b| (b.x, b.y, b.width, b.height));
                assert_eq!(made, again);
                spanning_none += usize::from(made.is_none());
                checked += 1;
            }
        }
    });
    assert!(checked > 0 && spanning_none < checked, "{checked}");
    pieces
}

const BORDERED: &str = "display:inline;position:relative;border:2px solid;padding:3px 4px";

fn bordered_paragraph(root_css: &str) -> (crate::Document, raikiri_style::CascadeResult, usize) {
    ahem_paragraph_with(root_css, |doc, root| {
        doc.append_text(root, "aaaa ");
        let outer = doc.append_element(Some(root), "span", Some(BORDERED));
        doc.append_text(outer, "bbbb cccc dd ");
        let inner = doc.append_element(
            Some(outer),
            "span",
            Some("display:inline;position:relative;border-left:5px solid;border-bottom:1px solid"),
        );
        doc.append_text(inner, "eeee ffff");
        doc.append_element(
            Some(inner),
            "span",
            Some("display:inline;position:relative"),
        );
        doc.append_text(root, " gggg hh");
        let plain = doc.append_element(Some(root), "span", Some("display:inline"));
        doc.append_text(plain, "ii jj");
    })
}

#[test]
fn grouped_padding_boxes_match_the_scan_over_lines() {
    let (doc, cascade, root) = bordered_paragraph("width:100px;position:relative");
    assert_padding_boxes_match_the_scan(doc, cascade, root);
}

#[test]
fn grouped_padding_boxes_match_the_scan_in_right_to_left_text() {
    let (doc, cascade, root) = bordered_paragraph("width:100px;position:relative;direction:rtl");
    assert_padding_boxes_match_the_scan(doc, cascade, root);
}

#[test]
fn grouped_padding_boxes_match_the_scan_in_vertical_text() {
    for mode in ["vertical-rl", "vertical-lr"] {
        let (doc, cascade, root) = bordered_paragraph(&format!(
            "width:200px;height:100px;position:relative;writing-mode:{mode}"
        ));
        assert_padding_boxes_match_the_scan(doc, cascade, root);
    }
}

#[test]
fn grouped_padding_boxes_match_the_scan_for_an_element_with_no_piece() {
    let (mut base, mut container) = (0, 0);
    let (doc, cascade, root) = ahem_paragraph_with("width:200px;position:relative", |doc, root| {
        doc.append_text(root, "aaaa ");
        let ruby = doc.append_element(Some(root), "ruby", Some("display:ruby"));
        container = doc.append_element(
            Some(ruby),
            "rbc",
            Some("display:ruby-base-container;position:relative"),
        );
        base = doc.append_element(Some(container), "rb", Some("display:ruby-base"));
        doc.append_text(base, "bb");
        let annotation =
            doc.append_element(Some(ruby), "rt", Some("display:ruby-text;font-size:5px"));
        doc.append_text(annotation, "x");
    });
    let pieces = assert_padding_boxes_match_the_scan(doc, cascade, root);
    // The container has no piece of its own and spans its base.
    assert!(pieces.iter().all(|piece| piece.node != container));
    assert!(pieces.iter().any(|piece| piece.node == base));
}
