#[test]
fn source_walk_indexes_fragment_records_and_repeat_descendants() {
    use crate::fragment::{FragmentRect, LayoutFragment};
    use std::collections::HashSet;

    let mut document = crate::document::Document::new();
    let html = document.append_element(
        Some(0),
        "html",
        taffy::Style::default(),
        Some("display:block"),
    );
    let body = document.append_element(
        Some(html),
        "body",
        taffy::Style::default(),
        Some("display:block"),
    );
    let repeated = document.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:block"),
    );
    let descendant = document.append_element(
        Some(repeated),
        "span",
        taffy::Style::default(),
        Some("display:block"),
    );
    let sibling = document.append_element(
        Some(body),
        "aside",
        taffy::Style::default(),
        Some("display:block"),
    );
    document.mark_in_document_flags();

    let fragment = |node_id, parent, fragment_index, fragment_count| LayoutFragment {
        node_id,
        parent,
        fragmentainer: fragment_index,
        rect: FragmentRect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        },
        fragmentainer_clip: None,
        fragment_index,
        fragment_count,
        line_start: None,
        line_end: None,
    };
    for index in 0..2 {
        document
            .fragment_tree
            .try_push(fragment(repeated, None, index, 2))
            .expect("repeated fragment should fit");
    }
    document
        .fragment_tree
        .try_push(fragment(descendant, Some(0), 0, 1))
        .expect("descendant fragment should fit");
    document
        .fragment_tree
        .try_push(fragment(sibling, None, 0, 1))
        .expect("sibling fragment should fit");

    let walk = super::super::SourceWalk::new(&document);

    assert_eq!(walk.fragment_indices(None, repeated).unwrap(), &[0, 1]);
    assert_eq!(walk.fragment_indices(Some(0), descendant).unwrap(), &[2]);
    assert_eq!(walk.fragment_indices(None, sibling).unwrap(), &[3]);
    assert!(walk.is_fragmented(repeated));
    assert!(walk.is_fragmented(descendant));
    assert!(walk.is_fragmented(sibling));
    assert!(walk.is_repeated(repeated));
    assert!(walk.is_repeated(descendant));
    assert!(!walk.is_repeated(sibling));
    assert_eq!(
        walk.into_repeated_placement_nodes(),
        HashSet::from([repeated, descendant])
    );
}
