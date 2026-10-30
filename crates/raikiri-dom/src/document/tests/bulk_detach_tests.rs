use super::*;

#[test]
fn bulk_removal_does_not_shift_the_tail_for_each_child() {
    for count in [16, 32, 64] {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "div", None::<&str>);
        let children: Vec<_> = (0..count)
            .map(|_| doc.append_element(Some(parent), "span", None::<&str>))
            .collect();
        // Unrelated arena entries must not participate in a one-parent detach.
        for _ in 0..4096 {
            doc.append_element(None, "aside", None::<&str>);
        }
        DETACH_SHIFT_WORK.set(0);
        BULK_CHILD_VISITS.set(0);
        assert_eq!(doc.detach_children(parent, children.iter().copied()), count);
        assert!(doc.nodes[parent].children.is_empty());
        assert!(children.iter().all(|&id| doc.parent_of(id).is_none()));
        assert_eq!(DETACH_SHIFT_WORK.get(), 0);
        assert_eq!(BULK_CHILD_VISITS.get(), count * 2);
    }
}

#[test]
fn bulk_removal_keeps_other_children_and_reports_only_changed_edges() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", None::<&str>);
    let first = doc.append_text(parent, "first");
    let removed = doc.append_text(parent, "removed");
    let last = doc.append_text(parent, "last");
    let other = doc.append_element(Some(0), "section", None::<&str>);
    let foreign = doc.append_text(other, "foreign");
    doc.enable_tree_change_tracking();
    assert_eq!(
        doc.detach_children(parent, [removed, removed, foreign, usize::MAX]),
        1
    );
    assert_eq!(doc.nodes[parent].children, [first, last]);
    assert_eq!(doc.parent_of(first), Some(parent));
    assert_eq!(doc.parent_of(last), Some(parent));
    assert_eq!(doc.parent_of(foreign), Some(other));
    assert_eq!(doc.take_tree_changes(), [removed]);
    assert_eq!(doc.detach_children(parent, []), 0);
    assert_eq!(doc.detach_children(usize::MAX, [first]), 0);
    assert!(doc.take_tree_changes().is_empty());
    assert_eq!(doc.nodes[parent].children, [first, last]);
}

#[test]
fn bulk_removal_invalidates_focus_stylesheet_and_layout() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "style", None::<&str>);
    let removed = doc.append_element(Some(parent), "span", None::<&str>);
    let focused = doc.append_element(Some(removed), "button", None::<&str>);
    doc.set_focused_element(Some(focused));
    doc.set_stylesheet_source(parent, "span {}".into()).unwrap();
    doc.mark_in_document_flags();
    doc.layout_dirty = false;
    assert_eq!(doc.detach_children(parent, [removed]), 1);
    assert_eq!(doc.focused_element(), None);
    assert_eq!(doc.stylesheet_source(parent), None);
    assert!(doc.layout_dirty);
    assert!(doc.flags_dirty);
    doc.mark_in_document_flags();
    assert!(!doc.nodes[focused].is_in_document());
    doc.attach_child(parent, removed);
    assert_eq!(doc.focused_element(), None);
    assert_eq!(doc.parent_of(focused), Some(removed));
}
