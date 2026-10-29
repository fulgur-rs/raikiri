use super::*;

#[test]
fn first_query_scans_and_second_query_indexes_only_the_queried_parent() {
    let mut document = Document::new();
    let parent = document.append_element(Some(0), "section", None::<&str>);
    let children: Vec<_> = (0..32)
        .map(|_| document.append_element(Some(parent), "div", None::<&str>))
        .collect();
    let other = document.append_element(Some(0), "section", None::<&str>);
    let another = document.append_element(Some(other), "span", None::<&str>);
    for _ in 0..10_000 {
        document.append_element(None, "aside", None::<&str>);
    }
    let mut index = SiblingIndex::default();
    assert_eq!(index.position(&document, children[0]), Some((parent, 0)));
    assert!(index.positions.is_empty());
    assert_eq!(index.positions.capacity(), 0);
    assert_eq!(index.seen_parents.len(), 1);
    assert_eq!(index.position(&document, children[1]), Some((parent, 1)));
    assert_eq!(index.positions.len(), 32);
    assert_eq!(index.position(&document, another), Some((other, 0)));
    assert_eq!(index.positions.len(), 32);
    assert_eq!(index.seen_parents.len(), 1);
    assert_eq!(index.position(&document, another), Some((other, 0)));
    assert_eq!(index.positions.len(), 32);
    for (at, &child) in children.iter().enumerate() {
        assert_eq!(index.position(&document, child), Some((parent, at)));
    }
    assert_eq!(index.positions.len(), 32);
    assert_eq!(index.seen_parents.len(), 1);
}

#[test]
fn new_revision_discards_old_entries_even_on_detached_or_invalid_queries() {
    let mut document = Document::new();
    let node = document.append_element(Some(0), "div", None::<&str>);
    document.append_text(0, "a");
    document.append_text(0, "b");
    let mut index = SiblingIndex::default();
    index.position(&document, node);
    index.position(&document, node);
    assert_eq!(index.positions.len(), 3);
    document.detach_from_parent(node);
    assert_eq!(index.position(&document, node), None);
    assert_eq!(index.positions.capacity(), 0);
    assert_eq!(index.seen_parents.capacity(), 0);
    document.attach_child(0, node);
    index.position(&document, node);
    index.position(&document, node);
    let replacement = Document::new();
    assert_eq!(index.position(&replacement, usize::MAX), None);
    assert_eq!(index.positions.capacity(), 0);
    assert_eq!(index.seen_parents.capacity(), 0);
}

fn assert_positions_match_source(document: &Document, index: &mut SiblingIndex) {
    for _ in 0..2 {
        for node in 0..document.node_count() {
            let expected = document.parent_of(node).and_then(|parent| {
                document
                    .get_node(parent)?
                    .children
                    .iter()
                    .position(|&child| child == node)
                    .map(|at| (parent, at))
            });
            assert_eq!(index.position(document, node), expected, "node={node}");
        }
    }
}

#[test]
fn native_moves_fragments_reorders_and_bulk_replacements_refresh_positions() {
    let mut doc = Document::new();
    let a = doc.append_element(Some(0), "section", None::<&str>);
    let b = doc.append_element(Some(0), "section", None::<&str>);
    let text = doc.append_text(a, "text");
    let first = doc.append_element(Some(a), "div", None::<&str>);
    let comment = doc.append_comment(Some(a), "comment");
    doc.append_processing_instruction(Some(a), "target", "data");
    let last = doc.append_element(Some(a), "div", None::<&str>);
    let fragment = doc.create_detached_fragment();
    doc.append_text(fragment, "fragment text");
    doc.append_element(Some(fragment), "span", None::<&str>);
    let template = doc.append_element(Some(b), "template", None::<&str>);
    let contents = doc.allocate_template_fragment_root(template);
    doc.append_element(Some(contents), "span", None::<&str>);
    doc.append_comment(Some(contents), "inert comment");
    let mut index = SiblingIndex::default();
    assert_positions_match_source(&doc, &mut index);
    doc.detach_from_parent(first);
    assert_positions_match_source(&doc, &mut index);
    doc.pre_insert(b, first, None).unwrap();
    assert_positions_match_source(&doc, &mut index);
    doc.pre_insert(a, last, Some(text)).unwrap();
    assert_positions_match_source(&doc, &mut index);
    doc.pre_insert(a, fragment, Some(comment)).unwrap();
    assert_positions_match_source(&doc, &mut index);
    doc.reparent_children(a, b);
    assert_positions_match_source(&doc, &mut index);
    doc.retain_children(|id| id != comment);
    assert_positions_match_source(&doc, &mut index);
    doc.set_element_text_content(b, "replacement").unwrap();
    assert_positions_match_source(&doc, &mut index);
    let mut source = Document::new();
    source.append_element(Some(0), "em", None::<&str>);
    source.append_text(0, "copied");
    doc.replace_children_from(b, &source, 0);
    assert_positions_match_source(&doc, &mut index);
    assert_positions_match_source(&doc.clone(), &mut index);
}

#[test]
fn identical_generation_in_another_document_cannot_reuse_positions() {
    fn page(reverse: bool) -> (Document, usize) {
        let mut document = Document::new();
        let fragment = document.create_detached_fragment();
        let a = document.create_detached_element("a").unwrap();
        let b = document.create_detached_element("b").unwrap();
        let c = document.create_detached_element("c").unwrap();
        for child in if reverse { [b, a, c] } else { [a, b, c] } {
            document.attach_child(fragment, child);
        }
        (document, a)
    }
    let (first, a) = page(false);
    let (second, same_id) = page(true);
    assert_eq!(a, same_id);
    let mut index = SiblingIndex::default();
    assert_eq!(index.position(&first, a).unwrap().1, 0);
    assert_eq!(index.position(&first, a).unwrap().1, 0);
    assert_eq!(index.position(&second, a).unwrap().1, 1);
    assert!(index.positions.is_empty());
    assert_eq!(index.position(&second, a).unwrap().1, 1);
}

#[test]
fn small_parent_queries_keep_position_metadata_unallocated() {
    for count in [1, 2] {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "div", None::<&str>);
        let children: Vec<_> = (0..count).map(|_| doc.append_text(parent, "x")).collect();
        let mut index = SiblingIndex::default();
        for _ in 0..3 {
            for (position, &child) in children.iter().enumerate() {
                assert_eq!(index.position(&doc, child), Some((parent, position)));
            }
        }
        assert_eq!(index.positions.capacity(), 0);
        assert_eq!(index.seen_parents.capacity(), 0);
    }
}
