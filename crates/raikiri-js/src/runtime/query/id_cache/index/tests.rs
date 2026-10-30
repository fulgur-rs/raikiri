use super::*;

fn fixture(ids: &[&str]) -> (Document, usize, Vec<usize>) {
    let mut doc = Document::new();
    let root = doc.create_detached_fragment();
    let mut nodes = Vec::new();
    for id in ids {
        let node = doc.append_element(Some(root), "a", None::<&str>);
        doc.set_element_attribute(node, "id", *id).unwrap();
        nodes.push(node);
    }
    (doc, root, nodes)
}

#[test]
fn exact_entry_and_byte_limits_complete_after_duplicate_and_skipped_ids() {
    let long = "x".repeat(MAX_SINGLE_KEY_BYTES + 1);
    let (doc, root, nodes) = fixture(&["é", "abc", "é", "", &long]);
    for (entries, bytes, complete, count) in [
        (1, 5, false, 1),
        (2, 4, false, 1),
        (2, 5, true, 2),
        (3, 6, true, 2),
    ] {
        let index = RootIndex::build(&doc, root, entries, bytes);
        assert_eq!(index.complete, complete);
        assert_eq!(index.ids.len(), count);
        assert_eq!(index.ids.get("é"), Some(&nodes[0]));
        assert!(!index.ids.contains_key(&long));
        assert!(!index.ids.contains_key(""));
        assert_eq!(index.key_bytes, if count == 1 { 2 } else { 5 });
    }
}

#[test]
fn exhausted_budget_only_becomes_partial_when_a_new_eligible_key_needs_space() {
    let (empty, root, _) = fixture(&[""]);
    assert!(RootIndex::build(&empty, root, 0, 0).complete);
    let (doc, root, _) = fixture(&["id"]);
    for (entries, bytes) in [(0, 2), (1, 0)] {
        let index = RootIndex::build(&doc, root, entries, bytes);
        assert!(!index.complete);
        assert!(index.ids.is_empty());
        assert_eq!(index.key_bytes, 0);
    }
}

#[test]
fn preorder_prefix_excludes_root_and_preserves_first_duplicate() {
    let (mut doc, root, nodes) = fixture(&["dup", "next"]);
    let nested = doc.append_element(Some(nodes[0]), "b", None::<&str>);
    doc.set_element_attribute(nested, "id", "next").unwrap();
    let index = RootIndex::build(&doc, root, 2, 7);
    assert!(index.complete);
    assert_eq!(index.ids.get("next"), Some(&nested));
    let own = RootIndex::build(&doc, nodes[0], 2, 7);
    assert!(!own.ids.contains_key("dup"));
    assert_eq!(own.ids.get("next"), Some(&nested));
}
