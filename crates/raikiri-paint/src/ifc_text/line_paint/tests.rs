use super::*;

fn piece(node: usize, owner: Option<usize>, line: usize) -> InlineBoxPiece {
    let rect = raikiri_dom::BoxRect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    InlineBoxPiece {
        node,
        line,
        source_owner: owner,
        source_container: None,
        parent: None,
        border_box: rect,
        content_box: rect,
        has_start_edge: true,
        has_end_edge: true,
    }
}

#[test]
fn first_piece_preserves_the_first_source_owner_before_later_duplicates() {
    for owners in [[None, Some(7)], [Some(7), Some(8)], [Some(8), None]] {
        let pieces = [
            piece(10, owners[0], 0),
            piece(11, Some(9), 0),
            piece(10, owners[1], 0),
        ];
        let first = first_content_image_pieces::<false>(&pieces);
        assert_eq!(first.get(&10), Some(&0));
        assert_eq!(pieces[first[&10]].source_owner, owners[0]);
        for node in [10, 11, 12] {
            assert_eq!(
                first.get(&node).map(|&index| &pieces[index]),
                pieces.iter().find(|piece| piece.node == node)
            );
        }
    }
}

#[test]
fn first_piece_is_local_to_each_line_and_accepts_empty_pieces() {
    assert!(first_content_image_pieces::<false>(&[]).is_empty());
    let lines = [
        [piece(10, Some(1), 0), piece(10, Some(2), 0)],
        [piece(10, Some(3), 1), piece(10, Some(4), 1)],
    ];
    for (line, expected) in lines.iter().zip([1, 3]) {
        let first = first_content_image_pieces::<false>(line);
        assert_eq!(line[first[&10]].source_owner, Some(expected));
    }
}

#[test]
fn transformed_block_slices_keep_only_the_page_edges() {
    let mut original = piece(10, None, 0);
    original.border_box = raikiri_dom::BoxRect {
        x: 3.0,
        y: 10.0,
        width: 40.0,
        height: 60.0,
    };
    original.content_box = raikiri_dom::BoxRect {
        x: 5.0,
        y: 15.0,
        width: 36.0,
        height: 50.0,
    };
    original.has_start_edge = true;
    original.has_end_edge = true;
    let base = IfcPosition {
        x: 20.0,
        y: 100.0,
        shift_y: 5.0,
    };
    let page = |start, end| {
        Some(crate::walk::PageLines::AlongY {
            start,
            end,
            paint_shift: 5.0,
        })
    };
    let middle = transformed_block_piece(original, page(120.0, 160.0), base).unwrap();
    assert_eq!(
        middle.border_box,
        raikiri_dom::BoxRect {
            x: 3.0,
            y: 20.0,
            width: 40.0,
            height: 40.0
        }
    );
    assert_eq!(
        middle.content_box,
        raikiri_dom::BoxRect {
            x: 5.0,
            y: 20.0,
            width: 36.0,
            height: 40.0
        }
    );
    assert!(!middle.has_start_edge && !middle.has_end_edge);
    let first = transformed_block_piece(original, page(110.0, 140.0), base).unwrap();
    assert!(first.has_start_edge && !first.has_end_edge);
    let last = transformed_block_piece(original, page(140.0, 170.0), base).unwrap();
    assert!(!last.has_start_edge && last.has_end_edge);
    assert!(transformed_block_piece(original, page(180.0, 200.0), base).is_none());
    assert_eq!(
        transformed_block_piece(original, None, base)
            .unwrap()
            .border_box,
        original.border_box
    );
    let along_x = crate::walk::PageLines::AlongX(crate::walk::FlowBand {
        left: 0.0,
        right: 100.0,
        right_to_left: false,
        root_transform: Affine::IDENTITY,
    });
    assert_eq!(
        transformed_block_piece(original, Some(along_x), base)
            .unwrap()
            .border_box,
        original.border_box
    );
}
