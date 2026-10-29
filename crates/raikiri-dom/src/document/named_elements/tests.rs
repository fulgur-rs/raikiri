use super::*;

#[test]
fn overflowing_generation_replaces_identity_without_reusing_a_snapshot() {
    let mut document = Document::new();
    let initial = document.named_element_version().clone();
    document.named_elements.version.generation = u64::MAX;
    let last = document.named_element_version().clone();
    document.note_named_element_change();
    assert_ne!(&last, document.named_element_version());
    assert_ne!(&initial, document.named_element_version());
    assert_eq!(document.named_element_version().generation, 0);
}
