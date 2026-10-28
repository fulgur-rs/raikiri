use super::*;

#[test]
fn only_queried_nonempty_parents_are_cached_including_zero_counts() {
    let mut document = Document::new();
    let parent = document.append_element(Some(0), "section", None::<&str>);
    for _ in 0..32 {
        document.append_element(Some(parent), "div", None::<&str>);
    }
    let text_parent = document.append_element(Some(0), "section", None::<&str>);
    document.append_text(text_parent, "text");
    let leaves: Vec<_> = (0..10_000)
        .map(|_| document.append_element(None, "aside", None::<&str>))
        .collect();
    let mut memo = ElementCounts::default();
    for leaf in leaves {
        assert_eq!(memo.count(&document, leaf), 0);
    }
    assert_eq!(memo.counts.capacity(), 0);
    for _ in 0..2 {
        assert_eq!(memo.count(&document, parent), 32);
        assert_eq!(memo.count(&document, text_parent), 0);
    }
    assert_eq!(memo.counts.len(), 2);
    assert_eq!(memo.counts[&text_parent], 0);
}

#[test]
fn version_changes_release_old_capacity_before_empty_and_invalid_returns() {
    let mut document = Document::new();
    let parent = document.append_element(Some(0), "div", None::<&str>);
    let child = document.append_element(Some(parent), "span", None::<&str>);
    let mut memo = ElementCounts::default();
    assert_eq!(memo.count(&document, parent), 1);
    document.detach_from_parent(child);
    assert_eq!(memo.count(&document, parent), 0);
    assert_eq!(memo.counts.capacity(), 0);
    document.attach_child(parent, child);
    assert_eq!(memo.count(&document, parent), 1);
    document
        .set_element_attribute(parent, "id", "changed")
        .unwrap();
    assert_eq!(memo.count(&document, usize::MAX), 0);
    assert_eq!(memo.counts.capacity(), 0);
}

fn assert_counts_match_source(document: &Document, memo: &mut ElementCounts) {
    for parent in 0..document.node_count() {
        let expected = document
            .get_node(parent)
            .unwrap()
            .children
            .iter()
            .filter(|&&child| {
                document
                    .get_node(child)
                    .is_some_and(|n| n.kind() == NodeKind::Element)
            })
            .count();
        for _ in 0..2 {
            assert_eq!(memo.count(document, parent), expected, "parent={parent}");
        }
    }
}

#[test]
fn native_moves_fragments_and_bulk_replacements_refresh_counts() {
    let mut doc = Document::new();
    let a = doc.append_element(Some(0), "section", None::<&str>);
    let b = doc.append_element(Some(0), "section", None::<&str>);
    let text = doc.append_text(a, "text");
    let child = doc.append_element(Some(a), "div", None::<&str>);
    let comment = doc.append_comment(Some(a), "comment");
    doc.append_processing_instruction(Some(a), "target", "data");
    let fragment = doc.create_detached_fragment();
    doc.append_element(Some(fragment), "span", None::<&str>);
    let mut memo = ElementCounts::default();
    assert_counts_match_source(&doc, &mut memo);
    doc.detach_from_parent(child);
    assert_counts_match_source(&doc, &mut memo);
    doc.pre_insert(b, child, None).unwrap();
    assert_counts_match_source(&doc, &mut memo);
    doc.pre_insert(a, fragment, Some(text)).unwrap();
    assert_counts_match_source(&doc, &mut memo);
    doc.reparent_children(a, b);
    assert_counts_match_source(&doc, &mut memo);
    doc.retain_children(|id| id != comment);
    assert_counts_match_source(&doc, &mut memo);
    doc.set_element_text_content(b, "replacement").unwrap();
    assert_counts_match_source(&doc, &mut memo);
    let mut source = Document::new();
    source.append_element(Some(0), "em", None::<&str>);
    source.append_text(0, "copied");
    doc.replace_children_from(b, &source, 0);
    assert_counts_match_source(&doc, &mut memo);
    let clone = doc.clone();
    assert_ne!(doc.named_element_version(), clone.named_element_version());
    assert_counts_match_source(&clone, &mut memo);
}

#[test]
fn another_document_with_equal_mutation_count_cannot_reuse_counts() {
    fn page(element: bool) -> (Document, usize) {
        let mut doc = Document::new();
        let parent = doc.create_detached_fragment();
        if element {
            doc.append_element(Some(parent), "a", None::<&str>);
        } else {
            doc.append_comment(Some(parent), "comment");
        }
        (doc, parent)
    }
    let (first, parent) = page(true);
    let (second, same_id) = page(false);
    assert_eq!(parent, same_id);
    let mut memo = ElementCounts::default();
    assert_eq!(memo.count(&first, parent), 1);
    assert_eq!(memo.count(&second, parent), 0);
    assert_eq!(memo.counts.len(), 1);
}
