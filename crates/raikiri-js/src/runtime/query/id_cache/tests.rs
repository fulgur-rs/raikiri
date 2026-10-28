use super::*;

fn page() -> (Document, usize, usize) {
    let mut doc = Document::new();
    let root = doc.create_detached_fragment();
    let node = doc.append_element(Some(root), "a", None::<&str>);
    doc.set_element_attribute(node, "id", "found").unwrap();
    (doc, root, node)
}

#[test]
fn entry_and_byte_budgets_apply_across_all_roots() {
    let (mut doc, _, _) = page();
    let roots: Vec<_> = (0..MAX_ENTRIES + 1)
        .map(|_| doc.create_detached_fragment())
        .collect();
    let mut cache = IdLookupCache::default();
    for (i, root) in roots.into_iter().enumerate() {
        assert_eq!(cache.find(&doc, root, "missing"), None);
        assert!(cache.entries <= MAX_ENTRIES);
        assert!(cache.roots.len() <= cache.entries);
        if i == MAX_ENTRIES {
            assert_eq!(cache.entries, 1);
        }
    }
    cache.clear();
    for i in 0..17 {
        let key = format!("{i:04}{}", "é".repeat(510));
        assert_eq!(key.len(), 1024);
        cache.find(&doc, 0, &key);
        assert!(cache.key_bytes <= MAX_KEY_BYTES);
        if i == 15 {
            assert_eq!(cache.entries, 16);
            assert_eq!(cache.key_bytes, MAX_KEY_BYTES);
        }
        if i == 16 {
            assert_eq!(cache.entries, 1);
        }
    }
    assert_eq!(
        cache.key_bytes,
        cache
            .roots
            .values()
            .flat_map(|m| m.keys())
            .map(String::len)
            .sum::<usize>()
    );
}

#[test]
fn oversized_empty_and_invalid_queries_do_not_create_root_maps() {
    let (doc, root, node) = page();
    let mut cache = IdLookupCache::default();
    assert_eq!(
        cache.find(&doc, root, &"x".repeat(MAX_SINGLE_KEY_BYTES + 1)),
        None
    );
    assert_eq!(cache.find(&doc, root, ""), None);
    assert_eq!(cache.find(&doc, usize::MAX, "found"), None);
    assert_eq!(cache.roots.capacity(), 0);
    assert_eq!(cache.entries, 0);
    assert_eq!(cache.find(&doc, root, "found"), Some(node));
    assert_eq!(cache.find(&doc, root, "found"), Some(node));
    assert_eq!(cache.entries, 1);
    assert_eq!(cache.key_bytes, 5);
}

#[test]
fn native_changes_and_empty_invalid_queries_release_old_keys() {
    let (mut doc, root, node) = page();
    let mut cache = IdLookupCache::default();
    assert_eq!(cache.find(&doc, root, "found"), Some(node));
    doc.set_element_attribute(node, "id", "renamed").unwrap();
    assert_eq!(cache.find(&doc, root, ""), None);
    assert_eq!(cache.roots.capacity(), 0);
    assert_eq!(cache.find(&doc, root, "found"), None);
    assert_eq!(cache.find(&doc, root, "renamed"), Some(node));
    doc.detach_from_parent(node);
    assert_eq!(cache.find(&doc, usize::MAX, "renamed"), None);
    assert_eq!(cache.roots.capacity(), 0);
    assert_eq!(cache.find(&doc, root, "renamed"), None);
    doc.attach_child(root, node);
    assert_eq!(cache.find(&doc, root, "renamed"), Some(node));
    let clone = doc.clone();
    assert_ne!(doc.named_element_version(), clone.named_element_version());
    assert_eq!(cache.find(&clone, root, "renamed"), Some(node));
    assert_eq!(cache.entries, 1);
    let (mut replacement, same_root, same_node) = page();
    replacement
        .set_element_attribute(same_node, "id", "other")
        .unwrap();
    assert_eq!(root, same_root);
    assert_eq!(node, same_node);
    assert_eq!(cache.find(&replacement, root, "renamed"), None);
    assert_eq!(cache.find(&replacement, root, "other"), Some(same_node));
}
