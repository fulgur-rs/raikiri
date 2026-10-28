use super::*;

fn assert_changes_cover_existing_edges(doc: &mut Document, mutate: impl FnOnce(&mut Document)) {
    let before: Vec<_> = (0..doc.node_count())
        .map(|id| {
            (
                doc.parent_of(id),
                doc.get_node(id).unwrap().template_contents(),
            )
        })
        .collect();
    mutate(doc);
    let changed = doc.take_tree_changes();
    for (id, edge) in before.into_iter().enumerate() {
        let after = (
            doc.parent_of(id),
            doc.get_node(id).unwrap().template_contents(),
        );
        if edge != after {
            assert!(changed.contains(&id), "missing changed edge for node {id}");
        }
    }
    assert!(changed.iter().all(|&id| id < doc.node_count()));
    assert!(doc.take_tree_changes().is_empty());
}

#[test]
fn tree_tracking_is_opt_in_and_does_not_follow_document_clones() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", None::<&str>);
    let child = doc.append_element(Some(parent), "span", None::<&str>);
    doc.detach_from_parent(child);
    assert!(doc.take_tree_changes().is_empty());
    doc.enable_tree_change_tracking();
    doc.attach_child(parent, child);
    let mut cloned = doc.clone();
    cloned.detach_from_parent(child);
    assert!(cloned.take_tree_changes().is_empty());
    assert!(doc.take_tree_changes().contains(&child));
    doc.enable_tree_change_tracking();
    assert!(doc.take_tree_changes().is_empty());
}

#[test]
fn tree_tracking_covers_moves_fragments_bulk_replacements_and_templates() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", None::<&str>);
    let other = doc.append_element(Some(parent), "section", None::<&str>);
    let first = doc.append_element(Some(parent), "span", None::<&str>);
    let second = doc.append_element(Some(parent), "i", None::<&str>);
    let template = doc.append_element(Some(parent), "template", None::<&str>);
    doc.enable_tree_change_tracking();
    assert_changes_cover_existing_edges(&mut doc, |doc| {
        doc.pre_insert(other, first, None).unwrap();
    });
    assert_changes_cover_existing_edges(&mut doc, |doc| {
        doc.reparent_children(other, parent);
    });
    assert_changes_cover_existing_edges(&mut doc, |doc| {
        doc.detach_from_parent(first);
    });
    assert_changes_cover_existing_edges(&mut doc, |doc| {
        doc.insert_child_before(parent, second, first);
    });
    assert_changes_cover_existing_edges(&mut doc, |doc| {
        doc.allocate_template_fragment_root(template);
    });
    let fragment = doc.create_detached_fragment();
    doc.pre_insert(fragment, first, None).unwrap();
    doc.pre_insert(fragment, second, None).unwrap();
    doc.take_tree_changes();
    assert_changes_cover_existing_edges(&mut doc, |doc| {
        doc.insert_child_before(parent, other, fragment);
    });
    doc.pre_insert(fragment, first, None).unwrap();
    doc.take_tree_changes();
    assert_changes_cover_existing_edges(&mut doc, |doc| {
        doc.attach_child(parent, fragment);
    });
    assert_changes_cover_existing_edges(&mut doc, |doc| {
        doc.set_element_text_content(parent, "replacement").unwrap();
    });
    let mut source = Document::new();
    source.append_element(Some(0), "p", None::<&str>);
    assert_changes_cover_existing_edges(&mut doc, |doc| {
        doc.replace_children_from(parent, &source, 0);
    });
    let replacement = doc.get_node(parent).unwrap().children[0];
    assert_changes_cover_existing_edges(&mut doc, |doc| {
        doc.retain_children(|id| id != replacement);
    });
}

#[test]
fn repeated_moves_are_deduplicated_and_attribute_writes_add_no_edges() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", None::<&str>);
    let child = doc.append_element(Some(parent), "span", None::<&str>);
    doc.enable_tree_change_tracking();
    for _ in 0..1000 {
        doc.detach_from_parent(child);
        doc.attach_child(parent, child);
    }
    assert_eq!(doc.take_tree_changes(), [child]);
    doc.set_element_attribute(child, "id", "new").unwrap();
    assert!(doc.take_tree_changes().is_empty());
}
