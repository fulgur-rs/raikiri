use super::centered_offset;

#[test]
fn a_mark_is_centered_in_whole_layout_units() {
    // 16px - 7.99px leaves 513/64 px; half of it truncates to 256/64 px,
    // where an exact half would be 4.005px.
    assert_eq!(centered_offset(16.0, 7.99), 4.0);
    // A single unit of free space is not split.
    assert_eq!(centered_offset(10.0, 9.98), 0.0);
    // A mark wider than its character overhangs both sides evenly.
    assert_eq!(centered_offset(4.0, 5.0), -0.5);
}

mod scaling_tests;
