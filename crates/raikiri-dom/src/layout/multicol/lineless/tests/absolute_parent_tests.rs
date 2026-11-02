use super::ABSOLUTE_PARENT_WORK;
use crate::Document;
use raikiri_style::{Origin, RuleTree};

/// A multicol container with `columns` columns, 100px high and 10px wide
/// each, that holds a relatively positioned block as high as all of them and
/// `boxes` absolutely positioned boxes in it that reach down to the middle of
/// the last column. Returns the document and the ids of the boxes.
fn document(columns: usize, boxes: usize) -> (Document, Vec<usize>) {
    let mut doc = Document::new();
    doc.set_font_collection(shodo::font::FontCollection::new(
        &shodo::limits::Limits::default(),
    ));
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some(format!(
            "display:block;width:{}px;height:100px;columns:{columns};column-gap:0;column-fill:auto",
            columns * 10
        )),
    );
    let block = doc.append_element(
        Some(container),
        "div",
        Some(format!(
            "display:block;position:relative;height:{}px",
            columns * 100
        )),
    );
    let ids = (0..boxes)
        .map(|_| {
            doc.append_element(
                Some(block),
                "div",
                Some(format!(
                    "position:absolute;top:0;width:10px;height:{}px",
                    columns * 100 - 50
                )),
            )
        })
        .collect();
    doc.mark_in_document_flags();
    (doc, ids)
}

fn layout(doc: &mut Document, columns: usize) {
    let mut rules = RuleTree::empty();
    rules.add_stylesheet("html,body{display:block;margin:0}", Origin::Author);
    let cascade = raikiri_style::cascade(doc, &rules).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = columns as f32 * 10.0 + 20.0;
    page.height = 600.0;
    crate::layout_single_page(doc, &cascade, page).unwrap();
}

/// The columns the pieces of `node` are in, in the order they were made.
fn fragment_columns(doc: &Document, node: usize) -> Vec<usize> {
    doc.fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == node)
        .map(|fragment| fragment.fragmentainer)
        .collect()
}

#[test]
fn absolute_parent_piece_work_scales_with_emitted_pieces() {
    for columns in [128, 256, 512, 1024, 4096] {
        let (mut doc, ids) = document(columns, 1);
        ABSOLUTE_PARENT_WORK.with(|work| work.set([0; 3]));
        layout(&mut doc, columns);
        let work = ABSOLUTE_PARENT_WORK.with(|work| work.get());
        // The box has a piece in each column, next to the one of its
        // containing block there.
        assert_eq!(
            fragment_columns(&doc, ids[0]),
            (0..columns).collect::<Vec<_>>(),
            "{columns}"
        );
        // The pieces of the containing block are read once, and one piece of
        // them is looked up for each piece of the box.
        assert_eq!(work, [columns, columns, columns], "{columns}");
    }
}

#[test]
fn boxes_of_one_containing_block_share_the_index_of_its_pieces() {
    let columns = 64;
    let (mut doc, ids) = document(columns, 3);
    ABSOLUTE_PARENT_WORK.with(|work| work.set([0; 3]));
    layout(&mut doc, columns);
    let work = ABSOLUTE_PARENT_WORK.with(|work| work.get());
    for id in ids {
        assert_eq!(
            fragment_columns(&doc, id),
            (0..columns).collect::<Vec<_>>(),
            "{id}"
        );
    }
    assert_eq!(work, [columns, 3 * columns, 3 * columns]);
}

#[test]
fn the_first_piece_in_a_column_is_the_one_found() {
    use super::super::{Piece, first_piece_per_column};
    let piece = |column| Piece {
        column,
        top: 0.0,
        height: 1.0,
    };
    let pieces = [3, 1, 3, 0, 1].map(piece);
    let first = first_piece_per_column(&pieces);
    let mut found: Vec<_> = first.into_iter().collect();
    found.sort_unstable();
    assert_eq!(found, [(0, 3), (1, 1), (3, 0)]);
    for column in 0..5 {
        let by_search = pieces.iter().position(|piece| piece.column == column);
        assert_eq!(
            first_piece_per_column(&pieces).get(&column).copied(),
            by_search,
            "{column}"
        );
    }
}

#[test]
fn the_clip_of_a_piece_reaches_as_far_as_the_pieces_of_boxes_that_sit_in_it() {
    use super::super::{AbsolutePiece, piece_clip_extents};
    use crate::fragment::FragmentRect;
    let rect = |x: f32, width: f32| FragmentRect {
        x,
        y: 0.0,
        width,
        height: 10.0,
    };
    // Pieces in the pieces 2, 0 and 2 again, in one beyond the last, and
    // with clips that reach out on either side or stay inside.
    let absolute: Vec<AbsolutePiece> = [
        (2, -7.0, 30.0),
        (0, 4.0, 5.0),
        (2, 1.0, 90.0),
        (9, -50.0, 500.0),
        (2, -3.0, 2.0),
    ]
    .into_iter()
    .map(|(parent_piece, x, width)| AbsolutePiece {
        node: 0,
        column: parent_piece,
        parent_piece,
        rect: rect(0.0, 1.0),
        clip: rect(x, width),
    })
    .collect();
    for count in [2, 3, 5] {
        for start in [(0.0f32, 20.0f32), (-4.0, 100.0)] {
            // Each piece takes the pieces in it one after another.
            let by_filter: Vec<(f32, f32)> = (0..count)
                .map(|index| {
                    absolute
                        .iter()
                        .filter(|piece| piece.parent_piece == index)
                        .fold(start, |(left, right), piece| {
                            (
                                left.min(piece.clip.x),
                                right.max(piece.clip.x + piece.clip.width),
                            )
                        })
                })
                .collect();
            let extents = piece_clip_extents(count, start, &absolute);
            assert_eq!(extents.len(), count);
            for (index, (found, expected)) in extents.iter().zip(&by_filter).enumerate() {
                assert_eq!(found.0.to_bits(), expected.0.to_bits(), "{count} {index}");
                assert_eq!(found.1.to_bits(), expected.1.to_bits(), "{count} {index}");
            }
        }
    }
}
