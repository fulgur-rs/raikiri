use super::*;

#[test]
fn mark_in_document_flags_clears_detached_arena_nodes() {
    // Regression check: nodes present in the arena but not
    // reachable from the Document root (foster-parenting transient state,
    // orphans after removal of unimplemented nodes, etc.) become
    // is_in_document=false after marking (step 1 clears their default true flag).
    let mut doc = Document::new();
    let root = doc.root_index();
    let attached = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
    // Create a detached node with parent=None (append_element primitive contract).
    let detached = doc.append_element(None::<usize>, "span", Style::default(), None::<&str>);

    // Both constructors default to true.
    assert!(doc.get_node(attached).unwrap().is_in_document());
    assert!(doc.get_node(detached).unwrap().is_in_document());

    doc.mark_in_document_flags();

    assert!(
        doc.get_node(attached).unwrap().is_in_document(),
        "attached div should remain in_document after mark"
    );
    assert!(
        !doc.get_node(detached).unwrap().is_in_document(),
        "detached span must be cleared to !in_document after mark"
    );
}

#[test]
fn mark_in_document_flags_keeps_template_element_but_clears_descendants() {
    // Regression check for the existing contract: template element
    // itself stays in_document=true, its descendants get cleared. Redundant
    // with the raikiri-html integration test but locally verifies the DFS
    // shape (in_template state propagation) without going through parse.
    let mut doc = Document::new();
    let root = doc.root_index();
    let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
    let inner = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
    let text = doc.append_text(inner, "hi");

    doc.mark_in_document_flags();

    assert!(
        doc.get_node(tmpl).unwrap().is_in_document(),
        "template stays in doc"
    );
    assert!(
        !doc.get_node(inner).unwrap().is_in_document(),
        "<p> cleared"
    );
    assert!(
        !doc.get_node(text).unwrap().is_in_document(),
        "text cleared"
    );
}

#[test]
fn append_operations_set_flags_dirty() {
    // Regression check: mutation primitives set flags_dirty,
    // allowing observers to rely on calling mark_in_document_flags.
    // A fresh Document starts with dirty=false.
    let mut doc = Document::new();
    assert!(!doc.flags_dirty, "fresh Document has clean flags");
    let e = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    assert!(doc.flags_dirty, "append_element sets dirty");
    doc.flags_dirty = false;
    doc.append_text(e, "hi");
    assert!(doc.flags_dirty, "append_text sets dirty");
}

#[test]
fn attach_and_detach_set_flags_dirty() {
    // attach_child / detach_from_parent / reparent_children / insert_child_before /
    // Both retain_children operations change tree topology and must set
    // flags_dirty.
    let mut doc = Document::new();
    let a = doc.append_element(Some(0), "a", Style::default(), None::<&str>);
    let b = doc.append_element(Some(0), "b", Style::default(), None::<&str>);
    let d = doc.append_element(None::<usize>, "d", Style::default(), None::<&str>);
    doc.mark_in_document_flags(); // clean
    assert!(!doc.flags_dirty);

    doc.attach_child(a, d);
    assert!(doc.flags_dirty, "attach_child sets dirty");
    doc.mark_in_document_flags();

    doc.detach_from_parent(d);
    assert!(doc.flags_dirty, "detach_from_parent sets dirty");
    doc.mark_in_document_flags();

    doc.attach_child(a, d);
    doc.mark_in_document_flags();
    doc.reparent_children(a, b);
    assert!(doc.flags_dirty, "reparent_children sets dirty");
    doc.mark_in_document_flags();

    doc.retain_children(|c| c != d);
    assert!(
        doc.flags_dirty,
        "retain_children sets dirty when a child is removed"
    );
}

#[test]
fn mark_in_document_flags_is_noop_when_clean() {
    // mark_in_document_flags does nothing when !flags_dirty.
    // Invariant: without mutation after marking, marking again is
    // fast and idempotent.
    let mut doc = Document::new();
    let e = doc.append_element(Some(0), "e", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    assert!(!doc.flags_dirty);
    assert!(doc.get_node(e).unwrap().is_in_document());
    // Marking again is a no-op; the state is unchanged.
    doc.mark_in_document_flags();
    assert!(doc.get_node(e).unwrap().is_in_document());
    assert!(!doc.flags_dirty);
}

#[test]
fn set_element_namespace_dirties_flags_for_template_only() {
    // Regression check: `set_element_namespace` sets
    // flags_dirty only when changing the namespace of a `<template>` element.
    // Other elements do not set it (pure metadata, no layout effect).
    let mut doc = Document::new();
    let tmpl = doc.append_element(Some(0), "template", Style::default(), None::<&str>);
    let div = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    assert!(!doc.flags_dirty);

    // Changing a template’s namespace → set dirty.
    doc.set_element_namespace(tmpl, Some(SmolStr::new("http://www.w3.org/2000/svg")));
    assert!(
        doc.flags_dirty,
        "template namespace change must dirty flags"
    );
    doc.mark_in_document_flags();
    assert!(!doc.flags_dirty);

    // Changing a div’s namespace → do not set dirty (unrelated to template detection).
    doc.set_element_namespace(div, Some(SmolStr::new("http://www.w3.org/2000/svg")));
    assert!(
        !doc.flags_dirty,
        "non-template namespace change must NOT dirty flags"
    );

    // Setting the same namespace again → no change, so do not set dirty.
    doc.set_element_namespace(tmpl, Some(SmolStr::new("http://www.w3.org/2000/svg")));
    assert!(!doc.flags_dirty, "no-op namespace set must NOT dirty flags");
}

#[test]
fn post_mark_attach_under_template_becomes_out_of_document_after_remark() {
    // Regression check: pin end-to-end restoration of correct flags on
    // marking again after mutation. This is the post-parse mutation contract.
    let mut doc = Document::new();
    let root = doc.root_index();
    let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    assert!(doc.get_node(tmpl).unwrap().is_in_document());

    // A newly appended element starts with default IS_IN_DOCUMENT=true.
    // Attaching it below a template makes flags_dirty=true, so marking
    // must clear the flag.
    let new_child = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
    assert!(doc.flags_dirty, "mutation → dirty");
    // Before marking, the flag still has its default true value.
    assert!(doc.get_node(new_child).unwrap().is_in_document());
    doc.mark_in_document_flags();
    assert!(
        !doc.get_node(new_child).unwrap().is_in_document(),
        "child attached under template must become out-of-document after remark"
    );
}
