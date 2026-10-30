use super::*;

fn page() -> (Document, usize, usize) {
    let mut doc = Document::new();
    let root = doc.create_detached_fragment();
    let node = doc.append_element(Some(root), "a", None::<&str>);
    doc.set_element_attribute(node, "id", "found").unwrap();
    (doc, root, node)
}

#[test]
fn scalar_global_budgets_and_complete_promotion_release_keys() {
    let (mut doc, _, _) = page();
    let roots: Vec<_> = (0..MAX_ENTRIES + 1)
        .map(|_| doc.create_detached_fragment())
        .collect();
    let mut cache = IdLookupCache::default();
    for (i, root) in roots.into_iter().enumerate() {
        assert_eq!(cache.find(&doc, root, "missing"), None);
        assert!(cache.entries <= MAX_ENTRIES);
        assert!(cache.roots.len() <= cache.entries);
        assert!(cache.indices.is_empty());
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
            assert_eq!(cache.entries, 0);
            assert_eq!(cache.key_bytes, 0);
            assert!(cache.roots.is_empty());
            assert!(cache.indices[&0].complete);
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

fn promote_by_queries(cache: &mut IdLookupCache, doc: &Document, root: usize) {
    for index in 0..=MAX_ENTRIES {
        assert_eq!(cache.find(doc, root, &format!("missing{index}")), None);
    }
    assert!(cache.indices.contains_key(&root));
}

fn assert_accounting(cache: &IdLookupCache) {
    assert_eq!(
        cache.entries,
        cache.roots.values().map(HashMap::len).sum::<usize>()
    );
    assert_eq!(
        cache.key_bytes,
        cache
            .roots
            .values()
            .flat_map(|map| map.keys())
            .map(String::len)
            .sum::<usize>()
    );
    assert_eq!(
        cache.index_entries,
        cache
            .indices
            .values()
            .map(|index| index.ids.len())
            .sum::<usize>()
    );
    assert_eq!(
        cache.index_key_bytes,
        cache
            .indices
            .values()
            .map(|index| index.key_bytes)
            .sum::<usize>()
    );
    assert!(cache.entries <= MAX_ENTRIES);
    assert!(cache.key_bytes <= MAX_KEY_BYTES);
    assert!(cache.index_entries <= MAX_INDEX_ENTRIES);
    assert!(cache.index_key_bytes <= MAX_INDEX_KEY_BYTES);
    assert!(cache.indices.len() <= MAX_INDEX_ROOTS);
}

#[test]
fn complete_index_retains_source_ids_independently_of_missing_query_churn() {
    let (doc, root, node) = page();
    let mut cache = IdLookupCache::default();
    promote_by_queries(&mut cache, &doc, root);
    assert!(cache.indices[&root].complete);
    assert_eq!(cache.index_entries, 1);
    assert_eq!(cache.index_key_bytes, 5);
    for index in 0..4096 {
        assert_eq!(cache.find(&doc, root, &format!("random{index}")), None);
        assert_eq!(cache.find(&doc, root, "found"), Some(node));
    }
    assert!(cache.roots.is_empty());
    assert_eq!(cache.entries, 0);
    assert_eq!(cache.key_bytes, 0);
    assert_accounting(&cache);
}

#[test]
fn positive_entry_quota_keeps_partial_prefix_across_fallback_evictions() {
    let mut doc = Document::new();
    let root = doc.create_detached_fragment();
    let mut nodes = Vec::new();
    for index in 0..MAX_INDEX_ENTRIES + 1 {
        let node = doc.append_element(Some(root), "a", None::<&str>);
        doc.set_element_attribute(node, "id", format!("key{index}"))
            .unwrap();
        nodes.push(node);
    }
    let duplicate = doc.append_element(Some(root), "a", None::<&str>);
    doc.set_element_attribute(duplicate, "id", "key0").unwrap();
    let mut cache = IdLookupCache::default();
    promote_by_queries(&mut cache, &doc, root);
    assert!(!cache.indices[&root].complete);
    assert_eq!(cache.index_entries, MAX_INDEX_ENTRIES);
    let original_key = cache.indices[&root]
        .ids
        .get_key_value("key0")
        .unwrap()
        .0
        .as_ptr();
    assert_eq!(cache.find(&doc, root, "key0"), Some(nodes[0]));
    let tail = format!("key{MAX_INDEX_ENTRIES}");
    assert_eq!(cache.find(&doc, root, &tail), nodes.last().copied());
    for index in 0..MAX_ENTRIES * 2 {
        assert_eq!(cache.find(&doc, root, &format!("outside{index}")), None);
    }
    assert_eq!(cache.find(&doc, root, &tail), nodes.last().copied());
    assert_eq!(cache.find(&doc, root, "key0"), Some(nodes[0]));
    assert_eq!(
        cache.indices[&root]
            .ids
            .get_key_value("key0")
            .unwrap()
            .0
            .as_ptr(),
        original_key
    );
    assert_eq!(cache.index_entries, MAX_INDEX_ENTRIES);
    assert_accounting(&cache);
}

#[test]
fn positive_key_budget_is_shared_across_roots_and_full_keys_use_bytes() {
    let mut doc = Document::new();
    let roots = [
        doc.create_detached_fragment(),
        doc.create_detached_fragment(),
    ];
    for (group, root) in roots.into_iter().enumerate() {
        for index in 0..=MAX_INDEX_KEY_BYTES / MAX_SINGLE_KEY_BYTES / 2 {
            let node = doc.append_element(Some(root), "a", None::<&str>);
            let key = format!("{}{:03}{}", group, index, "é".repeat(510));
            assert_eq!(key.len(), MAX_SINGLE_KEY_BYTES);
            doc.set_element_attribute(node, "id", key).unwrap();
        }
    }
    let mut cache = IdLookupCache::default();
    promote_by_queries(&mut cache, &doc, roots[0]);
    assert!(cache.indices[&roots[0]].complete);
    promote_by_queries(&mut cache, &doc, roots[1]);
    assert!(!cache.indices[&roots[1]].complete);
    assert_eq!(cache.index_key_bytes, MAX_INDEX_KEY_BYTES);
    assert_eq!(
        cache.index_entries,
        MAX_INDEX_KEY_BYTES / MAX_SINGLE_KEY_BYTES
    );
    assert_accounting(&cache);
}

#[test]
fn index_and_scalar_root_limits_remain_independent() {
    let mut doc = Document::new();
    let roots: Vec<_> = (0..MAX_INDEX_ROOTS + MAX_ENTRIES + 1)
        .map(|_| doc.create_detached_fragment())
        .collect();
    let mut cache = IdLookupCache::default();
    for &root in &roots[..MAX_INDEX_ROOTS] {
        promote_by_queries(&mut cache, &doc, root);
    }
    assert_eq!(cache.indices.len(), MAX_INDEX_ROOTS);
    assert_eq!(cache.index_entries, 0);
    for &root in &roots[MAX_INDEX_ROOTS..MAX_INDEX_ROOTS + MAX_ENTRIES] {
        assert_eq!(cache.find(&doc, root, "first"), None);
    }
    assert_eq!(cache.roots.len(), MAX_ENTRIES);
    assert_eq!(
        cache.indices.len() + cache.roots.len(),
        MAX_INDEX_ROOTS + MAX_ENTRIES
    );
    assert_eq!(cache.find(&doc, *roots.last().unwrap(), "first"), None);
    assert_eq!(cache.entries, 1);
    assert_eq!(cache.indices.len(), MAX_INDEX_ROOTS);
    assert_accounting(&cache);
}

#[test]
fn revision_empty_invalid_clone_and_replacement_release_promoted_indices() {
    let (mut doc, root, node) = page();
    let mut cache = IdLookupCache::default();
    promote_by_queries(&mut cache, &doc, root);
    doc.set_element_attribute(node, "id", "changed").unwrap();
    assert_eq!(cache.find(&doc, root, ""), None);
    assert!(cache.indices.is_empty());
    assert_eq!((cache.index_entries, cache.index_key_bytes), (0, 0));
    promote_by_queries(&mut cache, &doc, root);
    doc.detach_from_parent(node);
    assert_eq!(cache.find(&doc, usize::MAX, "changed"), None);
    assert!(cache.indices.is_empty());
    doc.attach_child(root, node);
    promote_by_queries(&mut cache, &doc, root);
    let clone = doc.clone();
    assert_eq!(cache.find(&clone, root, "changed"), Some(node));
    assert!(cache.indices.is_empty());
    promote_by_queries(&mut cache, &clone, root);
    let (replacement, same_root, same_node) = page();
    assert_eq!((same_root, same_node), (root, node));
    assert_eq!(cache.find(&replacement, root, "changed"), None);
    assert!(cache.indices.is_empty());
    assert_eq!(cache.find(&replacement, root, "found"), Some(node));
    assert_accounting(&cache);
}

#[test]
fn complete_index_still_uses_uncached_lookup_for_oversized_existing_ids() {
    let (mut doc, root, _) = page();
    let long = "é".repeat(MAX_SINGLE_KEY_BYTES / 2 + 1);
    let node = doc.append_element(Some(root), "a", None::<&str>);
    doc.set_element_attribute(node, "id", &long).unwrap();
    let mut cache = IdLookupCache::default();
    promote_by_queries(&mut cache, &doc, root);
    assert!(cache.indices[&root].complete);
    assert_eq!(cache.index_entries, 1);
    for _ in 0..2 {
        assert_eq!(cache.find(&doc, root, &long), Some(node));
    }
    assert_eq!(cache.entries, 0);
    assert_eq!(cache.index_key_bytes, 5);
    assert_accounting(&cache);
}

#[test]
fn complete_promotion_subtracts_only_its_own_scalar_root() {
    let (mut doc, root, node) = page();
    let other_root = doc.create_detached_fragment();
    let other = doc.append_element(Some(other_root), "a", None::<&str>);
    doc.set_element_attribute(other, "id", "other").unwrap();
    let mut cache = IdLookupCache::default();
    assert_eq!(cache.find(&doc, other_root, "other"), Some(other));
    for index in 0..MAX_ENTRIES - 1 {
        assert_eq!(cache.find(&doc, root, &format!("missing{index}")), None);
    }
    assert_eq!(cache.entries, MAX_ENTRIES);
    assert_eq!(cache.find(&doc, root, "overflow"), None);
    assert!(cache.indices[&root].complete);
    assert!(!cache.roots.contains_key(&root));
    assert_eq!(cache.entries, 1);
    assert_eq!(cache.key_bytes, 5);
    assert_eq!(cache.roots[&other_root]["other"], Some(other));
    assert_eq!(cache.find(&doc, root, "found"), Some(node));
    assert_eq!(cache.find(&doc, other_root, "other"), Some(other));
    assert_accounting(&cache);
}
