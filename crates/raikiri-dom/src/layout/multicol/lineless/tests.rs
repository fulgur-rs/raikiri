use super::{Item, balanced_height, place, stacked_height};

fn shape(items: &[Item], height: f32) -> Vec<Vec<(usize, f32, f32)>> {
    place(items, height, usize::MAX)
        .unwrap()
        .pieces
        .iter()
        .map(|pieces| {
            pieces
                .iter()
                .map(|piece| (piece.column, piece.top, piece.height))
                .collect()
        })
        .collect()
}

fn split(height: f32) -> Item {
    Item {
        height,
        margin_top: 0.0,
        margin_bottom: 0.0,
        splittable: true,
    }
}

fn avoid(height: f32) -> Item {
    Item {
        splittable: false,
        ..split(height)
    }
}

#[test]
fn splittable_boxes_continue_in_the_next_column() {
    assert_eq!(
        shape(&[split(50.0), split(270.0)], 100.0),
        vec![
            vec![(0, 0.0, 50.0)],
            vec![
                (0, 50.0, 50.0),
                (1, 0.0, 100.0),
                (2, 0.0, 100.0),
                (3, 0.0, 20.0)
            ]
        ]
    );
}

#[test]
fn a_box_ending_at_a_column_edge_stays_in_that_column() {
    assert_eq!(
        shape(&[split(100.0), split(50.0)], 100.0),
        vec![vec![(0, 0.0, 100.0)], vec![(1, 0.0, 50.0)]]
    );
}

#[test]
fn boxes_avoiding_breaks_move_or_overflow_whole() {
    assert_eq!(
        shape(&[avoid(20.0), avoid(40.0), avoid(40.0)], 30.0),
        vec![
            vec![(0, 0.0, 20.0)],
            vec![(1, 0.0, 40.0)],
            vec![(2, 0.0, 40.0)]
        ]
    );
}

#[test]
fn sibling_margins_collapse_and_truncate_at_a_break() {
    let first = Item {
        margin_bottom: 20.0,
        ..avoid(40.0)
    };
    let second = Item {
        margin_top: 10.0,
        ..avoid(10.0)
    };
    // 40 + max(20, 10) + 10 = 70 fits a 70px column.
    assert_eq!(
        shape(&[first, second], 70.0),
        vec![vec![(0, 0.0, 40.0)], vec![(0, 60.0, 10.0)]]
    );
    // In a 60px column the second box moves, and loses its margin.
    assert_eq!(
        shape(&[first, second], 60.0),
        vec![vec![(0, 0.0, 40.0)], vec![(1, 0.0, 10.0)]]
    );
}

#[test]
fn balancing_shares_breakable_content_and_respects_avoidance() {
    assert_eq!(balanced_height(&[split(400.0)], 4, usize::MAX), 100.0);
    assert_eq!(
        balanced_height(&[avoid(40.0), avoid(40.0)], 3, usize::MAX),
        40.0
    );
    // Three 30px boxes that avoid breaks need 60px in two columns.
    let height = balanced_height(&[avoid(30.0), avoid(30.0), avoid(30.0)], 2, usize::MAX);
    assert!((height - 60.0).abs() < 0.01, "{height}");
}

#[test]
fn a_gap_taller_than_the_column_moves_the_box_to_the_next_column() {
    let item = Item {
        margin_top: 120.0,
        ..split(20.0)
    };
    assert_eq!(shape(&[item], 100.0), vec![vec![(1, 0.0, 20.0)]]);
    let empty = Item {
        margin_top: 120.0,
        ..split(0.0)
    };
    // A self-collapsing box takes no room, so it stays at the column end.
    assert_eq!(shape(&[empty], 100.0), vec![vec![(0, 100.0, 0.0)]]);
}

#[test]
fn placement_stops_at_the_piece_budget() {
    assert!(place(&[split(1_000_000_000.0)], 1.0, 8).is_none());
    assert!(place(&[avoid(10.0), avoid(10.0)], 10.0, 1).is_none());
    assert!(place(&[split(4.0)], 1.0, 4).is_some());
    // Columns too short to hold any of the box count against the budget.
    assert!(place(&[split(1.0)], 0.0005, 8).is_none());
}

#[test]
fn balancing_ignores_margins_truncated_at_the_break() {
    let first = Item {
        margin_bottom: 20.0,
        ..avoid(40.0)
    };
    let second = Item {
        margin_top: 10.0,
        ..avoid(40.0)
    };
    assert_eq!(balanced_height(&[first, second], 2, usize::MAX), 40.0);
}

#[test]
fn the_trailing_margin_stays_inside_the_container() {
    let item = Item {
        margin_bottom: 20.0,
        ..split(40.0)
    };
    assert_eq!(balanced_height(&[item], 1, usize::MAX), 60.0);
    assert_eq!(stacked_height(&[item], usize::MAX), Some(60.0));
}

#[test]
fn the_stacked_height_collapses_adjoining_margins() {
    let first = Item {
        margin_bottom: 20.0,
        ..avoid(40.0)
    };
    let second = Item {
        margin_top: 10.0,
        ..avoid(40.0)
    };
    assert_eq!(stacked_height(&[first, second], usize::MAX), Some(100.0));
}

#[test]
fn self_collapsing_boxes_join_the_surrounding_margins() {
    let empty = Item {
        margin_top: 20.0,
        margin_bottom: 30.0,
        ..split(0.0)
    };
    assert_eq!(stacked_height(&[empty], usize::MAX), Some(30.0));
    let first = Item {
        margin_bottom: 10.0,
        ..split(40.0)
    };
    assert_eq!(
        shape(&[first, empty, split(40.0)], 200.0),
        vec![
            vec![(0, 0.0, 40.0)],
            vec![(0, 60.0, 0.0)],
            vec![(0, 70.0, 40.0)]
        ]
    );
}

#[test]
fn a_gap_taller_than_the_column_moves_a_box_avoiding_breaks() {
    let item = Item {
        margin_top: 120.0,
        ..avoid(20.0)
    };
    assert_eq!(shape(&[item], 100.0), vec![vec![(1, 0.0, 20.0)]]);
    // A gap that leaves some room keeps the box in the first column.
    let item = Item {
        margin_top: 90.0,
        ..avoid(20.0)
    };
    assert_eq!(shape(&[item], 100.0), vec![vec![(0, 90.0, 20.0)]]);
}

#[test]
fn balancing_keeps_every_column_within_the_height() {
    let first = Item {
        margin_top: 30.0,
        ..avoid(40.0)
    };
    assert_eq!(balanced_height(&[first, avoid(40.0)], 2, usize::MAX), 70.0);
}
