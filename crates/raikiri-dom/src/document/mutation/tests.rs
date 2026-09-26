use crate::{Document, DomMutationError};

fn doc_with_html_body() -> (Document, usize, usize, usize) {
    let mut d = Document::new();
    let root = d.root_index();
    let html = d.create_detached_element("html").unwrap();
    d.attach_child(root, html);
    let body = d.create_detached_element("body").unwrap();
    d.attach_child(html, body);
    (d, root, html, body)
}

#[test]
fn pre_insert_appends_and_moves() {
    let (mut d, _, html, body) = doc_with_html_body();
    let p = d.create_detached_element("p").unwrap();
    d.pre_insert(body, p, None).unwrap();
    assert_eq!(d.get_node(body).unwrap().children, vec![p]);
    d.pre_insert(html, p, Some(body)).unwrap();
    assert_eq!(d.get_node(html).unwrap().children, vec![p, body]);
    assert!(d.get_node(body).unwrap().children.is_empty());
}

#[test]
fn pre_insert_splices_fragment_children_in_order() {
    let (mut d, _, _, body) = doc_with_html_body();
    let frag = d.create_detached_fragment();
    let a = d.create_detached_element("a").unwrap();
    let b = d.create_detached_text("b");
    d.pre_insert(frag, a, None).unwrap();
    d.pre_insert(frag, b, None).unwrap();
    let c = d.create_detached_comment("c");
    d.pre_insert(body, c, None).unwrap();
    d.pre_insert(body, frag, Some(c)).unwrap();
    assert_eq!(d.get_node(body).unwrap().children, vec![a, b, c]);
    assert!(d.get_node(frag).unwrap().children.is_empty());
}

#[test]
fn pre_insert_rejects_hierarchy_violations() {
    let (mut d, root, html, body) = doc_with_html_body();
    let text = d.create_detached_text("t");
    assert!(matches!(
        d.pre_insert(text, body, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
    assert!(matches!(
        d.pre_insert(body, html, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
    assert!(matches!(
        d.pre_insert(body, root, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
    assert!(matches!(
        d.pre_insert(root, text, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
    let second = d.create_detached_element("div").unwrap();
    assert!(matches!(
        d.pre_insert(root, second, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
    let orphan = d.create_detached_element("i").unwrap();
    let x = d.create_detached_element("x").unwrap();
    assert!(matches!(
        d.pre_insert(body, x, Some(orphan)),
        Err(DomMutationError::NotFound(_))
    ));
}

#[test]
fn pre_insert_rejects_out_of_range_indices() {
    let (mut d, _, _, body) = doc_with_html_body();
    let child = d.create_detached_element("span").unwrap();
    assert!(matches!(
        d.pre_insert(usize::MAX, child, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
    assert!(matches!(
        d.pre_insert(body, usize::MAX, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
}

#[test]
fn pre_insert_rejects_the_document_root_as_a_node_even_when_unreachable() {
    let (mut d, root, _, _) = doc_with_html_body();
    // A detached parent has no path back to `root` through the tree, so the
    // ancestor-cycle check alone would not catch inserting the Document
    // root as a plain node -- the dedicated "node must be a DocumentFragment
    // / Element / CharacterData node" check must.
    let detached = d.create_detached_element("free").unwrap();
    assert!(matches!(
        d.pre_insert(detached, root, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
}

#[test]
fn pre_insert_before_self_is_a_position_preserving_no_op() {
    let (mut d, _, _, body) = doc_with_html_body();
    let a = d.create_detached_element("a").unwrap();
    let b = d.create_detached_element("b").unwrap();
    let c = d.create_detached_element("c").unwrap();
    d.pre_insert(body, a, None).unwrap();
    d.pre_insert(body, b, None).unwrap();
    d.pre_insert(body, c, None).unwrap();
    // `insertBefore(b, b)` keeps b's position: the reference is adjusted to
    // b's own next sibling instead of resolving against a stale `b`.
    d.pre_insert(body, b, Some(b)).unwrap();
    assert_eq!(d.get_node(body).unwrap().children, vec![a, b, c]);
    // Same adjustment when the self-referenced node is the last child (its
    // "next sibling" is `None`, exercising the tail-append fallback).
    d.pre_insert(body, c, Some(c)).unwrap();
    assert_eq!(d.get_node(body).unwrap().children, vec![a, b, c]);
}

#[test]
fn pre_insert_document_parent_element_arity() {
    let mut d = Document::new();
    let root = d.root_index();
    let html = d.create_detached_element("html").unwrap();
    // Root has no Element child yet: inserting the first one succeeds.
    d.pre_insert(root, html, None).unwrap();
    assert_eq!(d.get_node(root).unwrap().children, vec![html]);

    let second = d.create_detached_element("div").unwrap();
    assert!(matches!(
        d.pre_insert(root, second, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
}

#[test]
fn pre_insert_document_parent_fragment_arity() {
    let mut d = Document::new();
    let root = d.root_index();

    // A fragment holding more than one Element is always rejected under a
    // Document parent.
    let frag_two_elements = d.create_detached_fragment();
    let e1 = d.create_detached_element("a").unwrap();
    let e2 = d.create_detached_element("b").unwrap();
    d.pre_insert(frag_two_elements, e1, None).unwrap();
    d.pre_insert(frag_two_elements, e2, None).unwrap();
    assert!(matches!(
        d.pre_insert(root, frag_two_elements, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));

    // A fragment holding a Text node is always rejected under a Document
    // parent, even with no Element children at all.
    let frag_with_text = d.create_detached_fragment();
    let t = d.create_detached_text("stray");
    d.pre_insert(frag_with_text, t, None).unwrap();
    assert!(matches!(
        d.pre_insert(root, frag_with_text, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));

    // A fragment holding exactly one Element succeeds when root has none yet...
    let frag_one_element = d.create_detached_fragment();
    let html = d.create_detached_element("html").unwrap();
    d.pre_insert(frag_one_element, html, None).unwrap();
    d.pre_insert(root, frag_one_element, None).unwrap();
    assert_eq!(d.get_node(root).unwrap().children, vec![html]);

    // ...but is rejected once root already has an Element child.
    let frag_second = d.create_detached_fragment();
    let div = d.create_detached_element("div").unwrap();
    d.pre_insert(frag_second, div, None).unwrap();
    assert!(matches!(
        d.pre_insert(root, frag_second, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
}

#[test]
fn pre_insert_document_parent_accepts_comment_and_processing_instruction() {
    // Unlike Element and DocumentFragment, Comment and ProcessingInstruction
    // are unconstrained under a Document parent: DOM §4.2.3 step 6 only
    // narrows the DocumentFragment and Element cases.
    let mut d = Document::new();
    let root = d.root_index();
    let comment = d.create_detached_comment("note");
    d.pre_insert(root, comment, None).unwrap();
    let pi = d.append_processing_instruction(None, "xml-stylesheet", "href=\"a.css\"");
    d.pre_insert(root, pi, None).unwrap();
    assert_eq!(d.get_node(root).unwrap().children, vec![comment, pi]);
}

#[test]
fn pre_insert_rejects_inserting_a_template_into_its_own_contents_fragment() {
    let (mut d, _, _, body) = doc_with_html_body();
    let template = d.create_detached_element("template").unwrap();
    d.pre_insert(body, template, None).unwrap();
    let fragment = d.allocate_template_fragment_root(template);
    // The fragment's host is `template` itself: DOM's host-including
    // ancestor check must reject inserting the template element into its
    // own contents fragment, even though the two are not linked through
    // ordinary `children`.
    assert!(matches!(
        d.pre_insert(fragment, template, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
    // An ancestor of the template (here, `body`) is host-including
    // reachable from the fragment too.
    assert!(matches!(
        d.pre_insert(fragment, body, None),
        Err(DomMutationError::HierarchyRequest(_))
    ));
}

#[test]
fn pre_remove_and_replace_child() {
    let (mut d, _, _, body) = doc_with_html_body();
    let a = d.create_detached_element("a").unwrap();
    let b = d.create_detached_element("b").unwrap();
    d.pre_insert(body, a, None).unwrap();
    assert!(matches!(
        d.pre_remove(body, b),
        Err(DomMutationError::NotFound(_))
    ));
    d.replace_child(body, b, a).unwrap();
    assert_eq!(d.get_node(body).unwrap().children, vec![b]);
    d.replace_child(body, b, b).unwrap();
    d.pre_remove(body, b).unwrap();
    assert!(d.get_node(body).unwrap().children.is_empty());
}

#[test]
fn replace_child_rejects_a_stale_child_reference() {
    let (mut d, _, _, body) = doc_with_html_body();
    let a = d.create_detached_element("a").unwrap();
    let stray = d.create_detached_element("stray").unwrap();
    d.pre_insert(body, a, None).unwrap();
    assert!(matches!(
        d.replace_child(body, stray, stray),
        Err(DomMutationError::NotFound(_))
    ));
}

#[test]
fn replace_child_moves_a_node_before_its_captured_next_sibling() {
    let (mut d, _, _, body) = doc_with_html_body();
    let a = d.create_detached_element("a").unwrap();
    let child = d.create_detached_element("child").unwrap();
    let z = d.create_detached_element("z").unwrap();
    d.pre_insert(body, a, None).unwrap();
    d.pre_insert(body, child, None).unwrap();
    d.pre_insert(body, z, None).unwrap();
    let node = d.create_detached_element("node").unwrap();
    d.replace_child(body, node, child).unwrap();
    assert_eq!(d.get_node(body).unwrap().children, vec![a, node, z]);
}

#[test]
fn replace_child_adjusts_reference_when_node_is_childs_next_sibling() {
    let (mut d, _, _, body) = doc_with_html_body();
    let a = d.create_detached_element("a").unwrap();
    let child = d.create_detached_element("child").unwrap();
    let node = d.create_detached_element("node").unwrap();
    let z = d.create_detached_element("z").unwrap();
    d.pre_insert(body, a, None).unwrap();
    d.pre_insert(body, child, None).unwrap();
    d.pre_insert(body, node, None).unwrap();
    d.pre_insert(body, z, None).unwrap();
    // body's children are now [a, child, node, z]; node is child's
    // immediate next sibling, so replacing child with node must resolve the
    // reinsertion point against node's OWN next sibling (z), not the
    // about-to-move `node` reference itself.
    d.replace_child(body, node, child).unwrap();
    assert_eq!(d.get_node(body).unwrap().children, vec![a, node, z]);
}

/// `replace_child` with a `DocumentFragment` `node` (DOM §4.2.3 "replace a
/// child with node within parent", combined with [`Document::attach_child`]/
/// [`Document::insert_child_before`]'s existing fragment-aware "splice, don't
/// insert the fragment itself" semantics): the fragment's own children take
/// `child`'s place in source order, and the fragment itself is left emptied
/// rather than becoming a child of `parent`.
#[test]
fn replace_child_splices_a_document_fragments_children_in_place() {
    let (mut d, _, _, body) = doc_with_html_body();
    let a = d.create_detached_element("a").unwrap();
    let child = d.create_detached_element("child").unwrap();
    let z = d.create_detached_element("z").unwrap();
    d.pre_insert(body, a, None).unwrap();
    d.pre_insert(body, child, None).unwrap();
    d.pre_insert(body, z, None).unwrap();

    let fragment = d.create_detached_fragment();
    let f1 = d.create_detached_element("f1").unwrap();
    let f2 = d.create_detached_element("f2").unwrap();
    d.attach_child(fragment, f1);
    d.attach_child(fragment, f2);

    d.replace_child(body, fragment, child).unwrap();

    assert_eq!(d.get_node(body).unwrap().children, vec![a, f1, f2, z]);
    assert!(d.get_node(fragment).unwrap().children.is_empty());
}

#[test]
fn replace_child_replaces_the_documents_sole_element_child() {
    let mut d = Document::new();
    let root = d.root_index();
    let html = d.create_detached_element("html").unwrap();
    d.pre_insert(root, html, None).unwrap();
    let other = d.create_detached_element("other-html").unwrap();
    // `html` is root's only Element child, but it is excluded from its own
    // "Document may only have one Element child" check because it is the
    // node being replaced.
    d.replace_child(root, other, html).unwrap();
    assert_eq!(d.get_node(root).unwrap().children, vec![other]);
}

#[test]
fn character_data_and_attribute_names() {
    let (mut d, _, _, body) = doc_with_html_body();
    let t = d.create_detached_text("hi");
    assert_eq!(d.character_data(t), Some("hi"));
    d.set_character_data(t, "yo").unwrap();
    assert_eq!(d.character_data(t), Some("yo"));
    let c = d.create_detached_comment("note");
    assert_eq!(d.character_data(c), Some("note"));
    assert!(d.set_character_data(body, "x").is_err());
    assert_eq!(d.character_data(body), None);
    d.set_element_attribute(body, "id", "b").unwrap();
    d.set_element_attribute(body, "class", "c").unwrap();
    d.set_element_inline_style(body, Some("color: red".into()));
    assert_eq!(
        d.element_attribute_names(body),
        vec!["id", "class", "style"]
    );
}

#[test]
fn character_data_covers_comments_and_processing_instructions() {
    let (mut d, _, _, body) = doc_with_html_body();
    let pi = d.append_processing_instruction(Some(body), "xml-stylesheet", "href=\"a.css\"");
    assert_eq!(d.character_data(pi), Some("href=\"a.css\""));
    assert_eq!(d.processing_instruction_target(pi), Some("xml-stylesheet"));
    assert_eq!(d.processing_instruction_target(body), None);
    assert_eq!(d.character_data(usize::MAX), None);

    d.set_character_data(pi, "href=\"b.css\"").unwrap();
    assert_eq!(d.character_data(pi), Some("href=\"b.css\""));

    let comment = d.create_detached_comment("note");
    d.set_character_data(comment, "updated").unwrap();
    assert_eq!(d.character_data(comment), Some("updated"));

    assert!(d.set_character_data(usize::MAX, "x").is_err());
}

#[test]
fn element_attribute_names_is_empty_for_non_elements_and_missing_ids() {
    let (mut d, _, _, _) = doc_with_html_body();
    let t = d.create_detached_text("hi");
    assert_eq!(d.element_attribute_names(t), Vec::<String>::new());
    assert_eq!(d.element_attribute_names(usize::MAX), Vec::<String>::new());
}
