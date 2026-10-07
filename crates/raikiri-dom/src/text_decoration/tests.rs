use super::*;

#[test]
fn an_out_of_range_root_has_no_decoration_context() {
    let document = Document::new();
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    assert!(
        context_for_root(&document, &cascade, usize::MAX)
            .specs()
            .is_empty()
    );
}
