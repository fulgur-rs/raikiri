use super::*;

#[test]
fn nested_owners_retain_even_a_small_remaining_gap() {
    let mut doc = Document::new();
    doc.outside_marker_leading.record(1, (3, 0), 9.0);
    doc.outside_marker_leading.record(2, (3, 0), 0.0001);
    doc.outside_marker_leading.clear(1);
    assert!((fit_leading(&doc, 3, 0) - 0.0001).abs() < 0.000001);
    doc.outside_marker_leading.clear(2);
    assert_eq!(fit_leading(&doc, 3, 0), 0.0);
    assert!(doc.outside_marker_leading.owners.is_empty());
    assert!(doc.outside_marker_leading.lines.is_empty());
}
