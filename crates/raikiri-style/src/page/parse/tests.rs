use super::*;

#[test]
fn page_descriptor_values_validate_against_their_grammar() {
    assert!(is_valid_page_descriptor("page-orientation", "rotate-left"));
    assert!(is_valid_page_descriptor("PAGE-ORIENTATION", " upright "));
    assert!(!is_valid_page_descriptor("page-orientation", "sideways"));
    assert!(is_valid_page_descriptor("size", "A4 landscape"));
    assert!(!is_valid_page_descriptor("size", "A4 landscape 1px"));
    assert!(is_valid_page_descriptor("marks", "crop cross"));
    assert!(is_valid_page_descriptor("bleed", "6pt"));
    assert!(!is_valid_page_descriptor("bleed", "6pt !important"));
    assert!(!is_valid_page_descriptor("margin", "1in"));
}
