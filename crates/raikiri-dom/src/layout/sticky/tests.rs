use super::axis_offset;

#[test]
fn an_end_inset_pulls_the_box_back_into_view_within_its_block() {
    // A box at 700..720 with `bottom: 10px` in a 600px view and a block
    // from 0 moves up to 570..590.
    assert_eq!(
        axis_offset(
            (700.0, 720.0),
            (0.0, 720.0),
            (0.0, 600.0),
            (None, Some(10.0))
        ),
        -130.0
    );
    // Its block starts at 650, so it cannot move up at all.
    assert_eq!(
        axis_offset(
            (650.0, 670.0),
            (650.0, 670.0),
            (0.0, 600.0),
            (None, Some(10.0))
        ),
        0.0
    );
}

#[test]
fn opposing_insets_add_their_moves() {
    assert_eq!(
        axis_offset(
            (0.0, 20.0),
            (0.0, 100.0),
            (0.0, 20.0),
            (Some(10.0), Some(10.0))
        ),
        10.0
    );
}
