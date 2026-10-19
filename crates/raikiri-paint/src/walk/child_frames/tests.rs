use super::inline_flow_offsets;

#[test]
fn a_child_starts_where_the_generated_text_before_it_ends() {
    // `(child, is_text, advance)`.
    let advances = [(1, false, 10.0), (2, false, 5.0)];
    assert_eq!(inline_flow_offsets(&advances, 3.0), [(1, 0.0), (2, 10.0)]);
}

#[test]
fn a_child_without_generated_text_takes_no_offset() {
    let advances = [(1, false, 0.0), (2, false, 7.0)];
    assert_eq!(inline_flow_offsets(&advances, 3.0), [(2, 0.0)]);
}

#[test]
fn text_between_two_generated_inlines_adds_a_collapsed_space() {
    let advances = [(1, false, 10.0), (2, true, 0.0), (3, false, 5.0)];
    assert_eq!(inline_flow_offsets(&advances, 3.0), [(1, 0.0), (3, 13.0)]);
}

#[test]
fn text_outside_the_generated_inlines_adds_no_space() {
    // No generated inline lies before the first text, and none follows the
    // last one.
    let advances = [(1, true, 0.0), (2, false, 10.0), (3, true, 0.0)];
    assert_eq!(inline_flow_offsets(&advances, 3.0), [(2, 0.0)]);
}
