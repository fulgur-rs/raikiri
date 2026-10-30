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
