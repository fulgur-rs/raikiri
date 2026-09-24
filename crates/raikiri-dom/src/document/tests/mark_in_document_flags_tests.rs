use super::*;

#[test]
fn mark_in_document_flags_clears_detached_arena_nodes() {
    // Regression check: arena に存在するが
    // Document root から reachable でない node (foster-parenting transient
    // state / unimplemented-node 除去後の孤児 等) は mark 後 is_in_document=false に落ちる
    // (Node::new_* の default true を step 1 の全 clear が上書きする)。
    let mut doc = Document::new();
    let root = doc.root_index();
    let attached = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
    // parent=None で detached を作る (append_element の primitive contract)
    let detached = doc.append_element(None::<usize>, "span", Style::default(), None::<&str>);

    // constructor default はどちらも true
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
    // Regression check: mutation primitives が flags_dirty を
    // set することで、observation-side が mark_in_document_flags を呼ぶ contract
    // に依存できる。fresh Document は dirty=false からスタート。
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
    // retain_children はいずれも tree topology を変えるため flags_dirty を
    // set する契約。
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
    // mark_in_document_flags は !flags_dirty のとき
    // 何もしない。invariant: 一度 mark した後 mutation が無ければ再 mark は
    // 高速で idempotent。
    let mut doc = Document::new();
    let e = doc.append_element(Some(0), "e", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    assert!(!doc.flags_dirty);
    assert!(doc.get_node(e).unwrap().is_in_document());
    // 再 mark は no-op、状態不変。
    doc.mark_in_document_flags();
    assert!(doc.get_node(e).unwrap().is_in_document());
    assert!(!doc.flags_dirty);
}

#[test]
fn set_element_namespace_dirties_flags_for_template_only() {
    // Regression check: `set_element_namespace` は
    // `<template>` element の namespace を変更した場合のみ flags_dirty を
    // set する。template 以外は set しない (pure metadata、layout 無影響)。
    let mut doc = Document::new();
    let tmpl = doc.append_element(Some(0), "template", Style::default(), None::<&str>);
    let div = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    assert!(!doc.flags_dirty);

    // template の namespace を変更 → dirty set される
    doc.set_element_namespace(tmpl, Some(SmolStr::new("http://www.w3.org/2000/svg")));
    assert!(
        doc.flags_dirty,
        "template namespace change must dirty flags"
    );
    doc.mark_in_document_flags();
    assert!(!doc.flags_dirty);

    // div の namespace を変更 → dirty set されない (template 判定に無関係)
    doc.set_element_namespace(div, Some(SmolStr::new("http://www.w3.org/2000/svg")));
    assert!(
        !doc.flags_dirty,
        "non-template namespace change must NOT dirty flags"
    );

    // 同じ namespace を再度 set → 変化無しなら dirty set しない
    doc.set_element_namespace(tmpl, Some(SmolStr::new("http://www.w3.org/2000/svg")));
    assert!(!doc.flags_dirty, "no-op namespace set must NOT dirty flags");
}

#[test]
fn post_mark_attach_under_template_becomes_out_of_document_after_remark() {
    // Regression check: mutation → 再 mark で正しい bit 状態が復元
    // されることを end-to-end で pin。post-parse mutation の contract。
    let mut doc = Document::new();
    let root = doc.root_index();
    let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    assert!(doc.get_node(tmpl).unwrap().is_in_document());

    // 新規 append_element は default IS_IN_DOCUMENT=true で作られる。
    // template 配下に attach するので flags_dirty=true になり、mark で
    // false に落ちるべき。
    let new_child = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
    assert!(doc.flags_dirty, "mutation → dirty");
    // mark 前は default true (bit reset は mark でしか起きない)
    assert!(doc.get_node(new_child).unwrap().is_in_document());
    doc.mark_in_document_flags();
    assert!(
        !doc.get_node(new_child).unwrap().is_in_document(),
        "child attached under template must become out-of-document after remark"
    );
}
