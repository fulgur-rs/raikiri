use super::*;

#[test]
fn layout_fragment_limit_can_only_be_lowered() {
    let mut doc = Document::new();
    let built_in = doc.fragment_tree.limit;
    doc.lower_layout_fragment_limit(built_in + 1);
    assert_eq!(doc.fragment_tree.limit, built_in);
    doc.lower_layout_fragment_limit(32);
    assert_eq!(doc.fragment_tree.limit, 32);
    doc.lower_layout_fragment_limit(64);
    assert_eq!(doc.fragment_tree.limit, 32);
    assert_eq!(doc.clone().fragment_tree.limit, 32);
}

#[test]
fn layout_work_limit_grows_with_the_document_unless_lowered() {
    let mut doc = Document::new();
    let built_in = doc.fragment_tree.limit;
    assert_eq!(doc.fragment_tree.work_limit(10), built_in);
    assert!(doc.fragment_tree.work_limit(1_000_000) > built_in);
    doc.lower_layout_fragment_limit(32);
    assert_eq!(doc.fragment_tree.work_limit(1_000_000), 32);
}
