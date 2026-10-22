use super::*;

#[test]
fn parent_lookup_lifts_contents_but_keeps_a_layout_root_parent() {
    let mut doc = Document::new();
    let parent = child(&mut doc, 0, DisplayValue::Block);
    let outer = child(&mut doc, parent, DisplayValue::Contents);
    let inner = child(&mut doc, outer, DisplayValue::Contents);
    let leaf = child(&mut doc, inner, DisplayValue::Block);
    doc.mark_in_document_flags();
    assert_eq!(doc.parent_of(leaf), Some(inner));
    assert_eq!(doc.taffy_parent_of(leaf), Some(parent));
    doc.nodes[inner].flags.insert(NodeFlags::IS_ROOT_ELEMENT);
    assert_eq!(doc.taffy_parent_of(leaf), Some(inner));
}

fn child(doc: &mut Document, parent: usize, display: DisplayValue) -> usize {
    let id = doc.append_element(Some(parent), "div", None::<&str>);
    doc.set_display(id, display);
    id
}

#[test]
fn nested_contents_resume_outer_siblings_and_stay_exhausted() {
    let mut doc = Document::new();
    let parent = child(&mut doc, 0, DisplayValue::Block);
    let first = child(&mut doc, parent, DisplayValue::Block);
    child(&mut doc, parent, DisplayValue::Contents);
    let outer = child(&mut doc, parent, DisplayValue::Contents);
    let second = child(&mut doc, outer, DisplayValue::Block);
    let inner = child(&mut doc, outer, DisplayValue::Contents);
    child(&mut doc, inner, DisplayValue::Contents);
    doc.append_text(inner, " ");
    let third = child(&mut doc, inner, DisplayValue::Block);
    let fourth = child(&mut doc, outer, DisplayValue::Block);
    let last = child(&mut doc, parent, DisplayValue::Block);
    doc.mark_in_document_flags();
    let mut walk = TaffyChildIter::new(&doc, NodeId::from(parent), false);
    for expected in [first, second, third, fourth, last] {
        assert_eq!(walk.next(), Some(NodeId::from(expected)));
    }
    assert_eq!(walk.next(), None);
    assert_eq!(walk.next(), None);
    let mut hidden = TaffyChildIter::new(&doc, NodeId::from(parent), true);
    assert_eq!(hidden.next(), None);
    assert_eq!(hidden.next(), None);
}

#[test]
fn contents_with_ifc_or_root_roles_are_not_flattened() {
    let mut doc = Document::new();
    let parent = child(&mut doc, 0, DisplayValue::Block);
    let ifc = child(&mut doc, parent, DisplayValue::Contents);
    child(&mut doc, ifc, DisplayValue::Block);
    let root = child(&mut doc, parent, DisplayValue::Contents);
    child(&mut doc, root, DisplayValue::Block);
    doc.mark_in_document_flags();
    doc.nodes[ifc].flags.insert(NodeFlags::IS_IFC_ROOT);
    doc.nodes[root].flags.insert(NodeFlags::IS_ROOT_ELEMENT);
    let actual: Vec<_> = TaffyChildIter::new(&doc, NodeId::from(parent), false)
        .map(usize::from)
        .collect();
    assert_eq!(actual, [ifc, root]);
}
