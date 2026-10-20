use super::{
    Item, Piece, Rows, Stop, balance_rows, balanced_height, fill_item_rows, place, place_columns,
    stacked_height,
};
use raikiri_style::property::ColumnFillValue;

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
        whole: false,
        slice: None,
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
fn boxes_avoiding_breaks_move_then_break_anyway_when_taller_than_a_column() {
    assert_eq!(
        shape(&[avoid(20.0), avoid(40.0), avoid(40.0)], 30.0),
        vec![
            vec![(0, 0.0, 20.0)],
            vec![(1, 0.0, 30.0), (2, 0.0, 10.0)],
            vec![(3, 0.0, 30.0), (4, 0.0, 10.0)]
        ]
    );
}

#[test]
fn boxes_avoiding_breaks_that_must_stay_whole_overflow_whole() {
    let whole = |height| Item {
        whole: true,
        ..avoid(height)
    };
    assert_eq!(
        shape(&[whole(20.0), whole(40.0), whole(40.0)], 30.0),
        vec![
            vec![(0, 0.0, 20.0)],
            vec![(1, 0.0, 40.0)],
            vec![(2, 0.0, 40.0)]
        ]
    );
}

#[test]
fn a_splittable_box_that_must_stay_whole_moves_to_the_next_column_when_it_fits_one() {
    let whole = |height| Item {
        whole: true,
        ..split(height)
    };
    // 95 + 100 does not fit a 100px column, but the bordered box fits a
    // whole column, so it starts the next one instead of being cut.
    assert_eq!(
        shape(&[split(95.0), whole(100.0)], 100.0),
        vec![vec![(0, 0.0, 95.0)], vec![(1, 0.0, 100.0)]]
    );
}

/// A box taller than a column whose pieces slice its decorations, which take
/// `leading` at its block start and `trailing` at its block end.
fn sliced(height: f32, leading: f32, trailing: f32) -> Item {
    Item {
        whole: true,
        slice: Some((leading, trailing)),
        ..split(height)
    }
}

#[test]
fn a_box_that_slices_its_decorations_is_cut_where_it_is_taller_than_a_column() {
    assert_eq!(
        shape(&[sliced(250.0, 2.0, 2.0)], 100.0),
        vec![vec![(0, 0.0, 100.0), (1, 0.0, 100.0), (2, 0.0, 50.0)]]
    );
    // One that fits the next column still moves there whole.
    assert_eq!(
        shape(&[split(95.0), sliced(100.0, 10.0, 10.0)], 100.0),
        vec![vec![(0, 0.0, 95.0)], vec![(1, 0.0, 100.0)]]
    );
}

#[test]
fn a_box_that_slices_its_decorations_starts_where_they_fit_with_some_content() {
    // The 10px border does not fit the 5px left in the first column.
    assert_eq!(
        shape(&[split(95.0), sliced(250.0, 10.0, 0.0)], 100.0),
        vec![
            vec![(0, 0.0, 95.0)],
            vec![(1, 0.0, 100.0), (2, 0.0, 100.0), (3, 0.0, 50.0)]
        ]
    );
    // The border and 5px of content fit the 15px left.
    assert_eq!(
        shape(&[split(85.0), sliced(250.0, 10.0, 0.0)], 100.0),
        vec![
            vec![(0, 0.0, 85.0)],
            vec![
                (0, 85.0, 15.0),
                (1, 0.0, 100.0),
                (2, 0.0, 100.0),
                (3, 0.0, 35.0)
            ]
        ]
    );
}

#[test]
fn a_box_that_slices_its_decorations_does_not_break_inside_the_block_end_ones() {
    // 5px would be left for the 10px border in the third column, so the
    // second column ends 5px early and the border follows whole.
    assert_eq!(
        shape(&[sliced(205.0, 0.0, 10.0)], 100.0),
        vec![vec![(0, 0.0, 100.0), (1, 0.0, 95.0), (2, 0.0, 10.0)]]
    );
    // A box of which only the border is left moves to the next column.
    assert_eq!(
        shape(&[sliced(105.0, 0.0, 10.0)], 100.0),
        vec![vec![(0, 0.0, 95.0), (1, 0.0, 10.0)]]
    );
}

#[test]
fn block_end_decorations_taller_than_a_column_are_cut_like_any_other_box() {
    // The 101px of padding cannot follow whole in a 100px column, so the box
    // is cut inside it instead of making no progress, which a piece budget
    // would run out on.
    assert!(place(&[sliced(102.0, 0.0, 101.0)], 100.0, 8).is_ok());
    assert_eq!(
        shape(&[sliced(102.0, 0.0, 101.0)], 100.0),
        vec![vec![(0, 0.0, 1.0), (1, 0.0, 100.0), (2, 0.0, 1.0)]]
    );
    // After content in the column, the box first moves to a new column.
    assert_eq!(
        shape(&[split(95.0), sliced(105.0, 0.0, 150.0)], 100.0),
        vec![vec![(0, 0.0, 95.0)], vec![(1, 0.0, 100.0), (2, 0.0, 5.0)]]
    );
}

#[test]
fn a_box_that_avoids_breaks_and_slices_its_decorations_breaks_anyway_when_taller_than_a_column() {
    let avoiding = Item {
        splittable: false,
        ..sliced(150.0, 2.0, 2.0)
    };
    assert_eq!(
        shape(&[avoiding], 100.0),
        vec![vec![(0, 0.0, 100.0), (1, 0.0, 50.0)]]
    );
}

#[test]
fn a_box_that_must_stay_whole_is_cut_when_its_margin_leaves_no_room_at_the_top() {
    let whole = Item {
        whole: true,
        margin_top: 10.0,
        ..split(95.0)
    };
    // Nothing precedes it, so moving on gains nothing; it is cut instead of
    // overflowing the column end.
    assert_eq!(place(&[whole], 100.0, 64).err(), Some(super::Stop::Cut));
}

#[test]
fn balancing_keeps_boxes_that_must_stay_whole_within_the_columns() {
    let whole = Item {
        whole: true,
        ..split(40.0)
    };
    let items = [split(50.0), whole, split(50.0)];
    let height = balanced_height(&items, 2, usize::MAX);
    let placement = place(&items, height, usize::MAX).unwrap();
    assert!(placement.columns_used() <= 2, "height={height}");
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
    assert!(place(&[split(1_000_000_000.0)], 1.0, 8).is_err());
    assert!(place(&[avoid(10.0), avoid(10.0)], 10.0, 1).is_err());
    assert!(place(&[split(4.0)], 1.0, 4).is_ok());
    // Columns too short to hold any of the box count against the budget.
    assert!(place(&[split(1.0)], 0.0005, 8).is_err());
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

#[test]
fn balancing_does_not_mistake_the_fragment_budget_for_short_columns() {
    // Two 50px pieces exceed a one-piece budget; the final placement reports
    // that instead of the search settling on one taller column.
    assert_eq!(balanced_height(&[split(100.0)], 2, 1), 50.0);
    assert!(place(&[split(100.0)], 50.0, 1).is_err());
}

#[test]
fn a_box_that_must_stay_whole_stops_the_placement_only_when_cut() {
    let whole = |height| Item {
        whole: true,
        ..split(height)
    };
    // Cut at the first column end, long before the budget runs out.
    assert_eq!(
        place(&[whole(100.0)], 10.0, 64).err(),
        Some(super::Stop::Cut)
    );
    // Whole in one column, with the budget spent on the plain box after it.
    assert_eq!(
        place(&[whole(1.0), split(100.0)], 1.0, 8).err(),
        Some(super::Stop::Exhausted)
    );
    // The budget runs out on a plain box before the whole box is reached.
    assert_eq!(
        place(&[split(100.0), whole(1.0)], 1.0, 8).err(),
        Some(super::Stop::Exhausted)
    );
    // The budget runs out exactly on a whole box that is not cut.
    assert_eq!(
        place(&[split(8.0), whole(1.0)], 1.0, 8).err(),
        Some(super::Stop::Exhausted)
    );
    // Columns too short for any of the box cut it rather than spend the
    // budget skipping them.
    assert_eq!(
        place(&[whole(1.0)], 0.0005, 8).err(),
        Some(super::Stop::Cut)
    );
    assert!(place(&[whole(1.0), split(3.0)], 1.0, 8).is_ok());
}

#[test]
fn self_collapsing_boxes_and_skipped_columns_count_toward_the_limits() {
    // Every box takes a piece, even one with no block size.
    assert_eq!(
        place(&[split(0.0), split(0.0)], 1.0, 1).err(),
        Some(super::Stop::Exhausted)
    );
    // A gap taller than the column moves the box to a column past the limit.
    let gapped = Item {
        margin_top: 5.0,
        ..split(1.0)
    };
    assert_eq!(
        super::place_within(&[gapped], 1.0, usize::MAX, 1, true).err(),
        Some(super::Stop::Exhausted)
    );
}

#[test]
fn balancing_boxes_with_no_block_size_gives_empty_columns() {
    assert_eq!(balanced_height(&[split(0.0)], 2, usize::MAX), 0.0);
}

fn row_shape(rows: &Rows) -> Vec<(usize, usize, usize, f32, f32)> {
    rows.pieces
        .iter()
        .map(|piece| (piece.entry, piece.row, piece.column, piece.y, piece.height))
        .collect()
}

#[test]
fn a_definite_nested_container_ends_with_overflow_columns_in_its_last_row() {
    // 500px of columns cut at a 300px outer column leave a 200px row, whose
    // overflow column takes the 100px left of the box.
    let rows = fill_item_rows(
        &[split(600.0)],
        1,
        ColumnFillValue::Auto,
        Some(500.0),
        300.0,
        300.0,
        usize::MAX,
    )
    .unwrap();
    assert_eq!(rows.heights, vec![300.0, 200.0]);
    assert_eq!(
        row_shape(&rows),
        vec![
            (0, 0, 0, 0.0, 300.0),
            (0, 1, 0, 0.0, 200.0),
            (0, 1, 1, 0.0, 100.0)
        ]
    );
}

#[test]
fn an_auto_height_nested_container_fills_rows_and_balances_the_last() {
    let rows = fill_item_rows(
        &[split(400.0)],
        2,
        ColumnFillValue::Balance,
        None,
        50.0,
        250.0,
        usize::MAX,
    )
    .unwrap();
    assert_eq!(rows.heights, vec![50.0, 150.0]);
    assert_eq!(
        row_shape(&rows),
        vec![
            (0, 0, 0, 0.0, 50.0),
            (0, 0, 1, 0.0, 50.0),
            (0, 1, 0, 0.0, 150.0),
            (0, 1, 1, 0.0, 150.0)
        ]
    );

    let rows = fill_item_rows(
        &[split(250.0)],
        1,
        ColumnFillValue::Auto,
        None,
        100.0,
        100.0,
        usize::MAX,
    )
    .unwrap();
    assert_eq!(rows.heights, vec![100.0, 100.0, 50.0]);
}

#[test]
fn a_nested_container_that_fits_its_first_row_has_no_rows() {
    let fill = |content| {
        fill_item_rows(
            &[split(80.0)],
            2,
            ColumnFillValue::Balance,
            content,
            50.0,
            100.0,
            usize::MAX,
        )
    };
    assert_eq!(fill(None), None);
    assert_eq!(fill(Some(40.0)), None);
    // No room in the first row, or past the fragment budget, gives none.
    assert_eq!(
        fill_item_rows(
            &[split(80.0)],
            1,
            ColumnFillValue::Auto,
            None,
            0.0,
            100.0,
            usize::MAX
        ),
        None
    );
    assert_eq!(
        fill_item_rows(
            &[split(800.0)],
            1,
            ColumnFillValue::Auto,
            None,
            10.0,
            10.0,
            4
        ),
        None
    );
}

#[test]
fn a_balanced_last_row_keeps_a_box_that_avoids_breaks_whole() {
    let rows = fill_item_rows(
        &[split(100.0), avoid(80.0)],
        2,
        ColumnFillValue::Balance,
        None,
        50.0,
        200.0,
        usize::MAX,
    )
    .unwrap();
    assert_eq!(rows.heights, vec![50.0, 80.0]);
    assert_eq!(row_shape(&rows)[2..], [(1, 1, 0, 0.0, 80.0)]);
}

#[test]
fn a_definite_nested_container_balances_its_last_row_when_it_fits() {
    let rows = fill_item_rows(
        &[split(300.0)],
        2,
        ColumnFillValue::Balance,
        Some(250.0),
        100.0,
        200.0,
        usize::MAX,
    )
    .unwrap();
    // The row keeps the height left of the container.
    assert_eq!(rows.heights, vec![100.0, 150.0]);
    assert_eq!(
        row_shape(&rows),
        vec![
            (0, 0, 0, 0.0, 100.0),
            (0, 0, 1, 0.0, 100.0),
            (0, 1, 0, 0.0, 50.0),
            (0, 1, 1, 0.0, 50.0)
        ]
    );
}

#[test]
fn a_box_avoiding_breaks_taller_than_a_row_breaks_anyway_in_the_last_row() {
    let rows = fill_item_rows(
        &[avoid(400.0)],
        2,
        ColumnFillValue::Balance,
        None,
        100.0,
        100.0,
        usize::MAX,
    )
    .unwrap();
    assert_eq!(rows.heights, vec![100.0, 100.0]);
    assert_eq!(rows.pieces.len(), 4);
}

#[test]
fn boxes_of_earlier_rows_do_not_bound_the_balanced_last_row() {
    let rows = fill_item_rows(
        &[avoid(100.0), split(250.0)],
        2,
        ColumnFillValue::Balance,
        None,
        100.0,
        200.0,
        usize::MAX,
    )
    .unwrap();
    // The 150px left after the first row share the last row equally.
    assert_eq!(rows.heights, vec![100.0, 75.0]);
}

#[test]
fn balance_all_balances_every_row_of_a_nested_container() {
    let fill = |column_fill, content| {
        fill_item_rows(
            &[split(150.0), avoid(100.0)],
            2,
            column_fill,
            content,
            100.0,
            200.0,
            usize::MAX,
        )
        .unwrap()
    };
    // `balance` leaves the first row sequential.
    let rows = fill(ColumnFillValue::Balance, None);
    assert_eq!(rows.heights, vec![100.0, 100.0]);
    assert_eq!(
        row_shape(&rows),
        vec![
            (0, 0, 0, 0.0, 100.0),
            (0, 0, 1, 0.0, 50.0),
            (1, 1, 0, 0.0, 100.0)
        ]
    );
    // `balance-all` balances it too, keeping the content it holds.
    let rows = fill(ColumnFillValue::BalanceAll, None);
    assert_eq!(rows.heights, vec![75.0, 100.0]);
    assert_eq!(
        row_shape(&rows),
        vec![
            (0, 0, 0, 0.0, 75.0),
            (0, 0, 1, 0.0, 75.0),
            (1, 1, 0, 0.0, 100.0)
        ]
    );
    // A definite container's row keeps its share of the container's height.
    let rows = fill(ColumnFillValue::BalanceAll, Some(300.0));
    assert_eq!(rows.heights, vec![100.0, 200.0]);
    assert_eq!(
        row_shape(&rows),
        vec![
            (0, 0, 0, 0.0, 75.0),
            (0, 0, 1, 0.0, 75.0),
            (1, 1, 0, 0.0, 100.0)
        ]
    );
}

#[test]
fn balance_all_keeps_a_row_tall_enough_for_the_boxes_avoiding_breaks_in_it() {
    let fill = |column_fill| {
        fill_item_rows(
            &[avoid(70.0), avoid(30.0), avoid(90.0), avoid(50.0)],
            2,
            column_fill,
            None,
            120.0,
            200.0,
            usize::MAX,
        )
        .unwrap()
    };
    assert_eq!(fill(ColumnFillValue::Balance).heights, vec![120.0, 50.0]);
    // An even share of the first row's 190px does not hold its boxes.
    let rows = fill(ColumnFillValue::BalanceAll);
    assert_eq!(rows.heights, vec![100.0, 50.0]);
    assert_eq!(
        row_shape(&rows),
        vec![
            (0, 0, 0, 0.0, 70.0),
            (1, 0, 0, 70.0, 30.0),
            (2, 0, 1, 0.0, 90.0),
            (3, 1, 0, 0.0, 50.0)
        ]
    );
}

#[test]
fn a_definite_nested_container_needing_more_rows_than_the_budget_has_no_rows() {
    let fill = |max_pieces| {
        fill_item_rows(
            &[split(1000.0)],
            2,
            ColumnFillValue::Balance,
            Some(1000.0),
            10.0,
            10.0,
            max_pieces,
        )
    };
    assert!(fill(3).is_none());
    assert_eq!(fill(usize::MAX).unwrap().heights.len(), 100);
}

#[test]
fn a_whole_box_leaves_a_shorter_first_column_for_the_next_one() {
    let whole = Item {
        whole: true,
        ..split(80.0)
    };
    let height_of = |column: usize| if column == 0 { 50.0 } else { 100.0 };
    let placement = place_columns(&[whole], &height_of, usize::MAX, usize::MAX, true).unwrap();
    assert_eq!(
        placement.pieces,
        vec![vec![Piece {
            column: 1,
            top: 0.0,
            height: 80.0
        }]]
    );
    // Columns of one height leave nothing to gain by moving it.
    assert_eq!(
        place_columns(&[whole], &|_| 50.0, usize::MAX, usize::MAX, true).err(),
        Some(Stop::Cut)
    );
}

#[test]
fn balance_all_stops_balancing_rows_once_its_probes_use_up_the_budget() {
    let items = [split(150.0), avoid(100.0)];
    let balance = |budget| {
        let mut heights = vec![100.0, 200.0];
        balance_rows(&items, 2, 1, &mut heights, usize::MAX, budget);
        heights
    };
    assert_eq!(balance(usize::MAX), vec![75.0, 200.0]);
    assert_eq!(balance(0), vec![75.0, 200.0]);
    let mut heights = vec![100.0, 100.0, 200.0];
    balance_rows(&items, 1, 2, &mut heights, usize::MAX, 0);
    // The first row's probes spend the budget, so the second keeps its height.
    assert_eq!(heights[1], 100.0);
}
