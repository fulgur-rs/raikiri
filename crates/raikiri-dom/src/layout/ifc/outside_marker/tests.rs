use super::*;

#[test]
fn nested_owners_retain_even_a_small_remaining_gap() {
    let mut doc = Document::new();
    doc.outside_marker_leading.record(1, 3, 9.0, Some((3, 0)));
    doc.outside_marker_leading
        .record(2, 3, 0.0001, Some((3, 0)));
    doc.outside_marker_leading.clear(1);
    assert!((fit_leading(&doc, 3, 0) - 0.0001).abs() < 0.000001);
    assert!((box_leading(&doc, 3) - 0.0001).abs() < 0.000001);
    doc.outside_marker_leading.clear(2);
    assert_eq!(fit_leading(&doc, 3, 0), 0.0);
    assert!(doc.outside_marker_leading.owners.is_empty());
    assert!(doc.outside_marker_leading.lines.is_empty());
    assert!(doc.outside_marker_leading.boxes.is_empty());
}

#[test]
fn box_translation_does_not_require_a_text_line_source() {
    let mut doc = Document::new();
    doc.outside_marker_leading.record(1, 2, 9.0, None);
    assert_eq!(box_leading(&doc, 2), 9.0);
    assert_eq!(fit_leading(&doc, 2, 0), 0.0);
    doc.outside_marker_leading.clear(1);
    assert_eq!(box_leading(&doc, 2), 0.0);
}

#[test]
fn replacing_an_owner_replaces_both_placement_indexes() {
    let mut doc = Document::new();
    doc.outside_marker_leading.record(1, 2, 9.0, Some((3, 0)));
    doc.outside_marker_leading.record(1, 4, 6.0, Some((5, 0)));
    assert_eq!(box_leading(&doc, 2), 0.0);
    assert_eq!(fit_leading(&doc, 3, 0), 0.0);
    assert_eq!(box_leading(&doc, 4), 6.0);
    assert_eq!(fit_leading(&doc, 5, 0), 6.0);
}
