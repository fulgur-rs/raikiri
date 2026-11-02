use super::*;

#[test]
fn tree_order_puts_ancestors_and_earlier_siblings_first() {
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "div", None::<&str>);
    let first = doc.append_element(Some(root), "div", None::<&str>);
    let inner = doc.append_element(Some(first), "div", None::<&str>);
    let second = doc.append_element(Some(root), "div", None::<&str>);
    assert!(precedes_in_tree(&doc, first, second));
    assert!(precedes_in_tree(&doc, inner, second));
    assert!(!precedes_in_tree(&doc, second, inner));
    // An ancestor comes before its descendants, and a node does not come
    // before itself.
    assert!(precedes_in_tree(&doc, root, inner));
    assert!(!precedes_in_tree(&doc, inner, root));
    assert!(!precedes_in_tree(&doc, inner, inner));
    // Nodes of separate trees are not ordered.
    doc.nodes[second].parent = None;
    assert!(!precedes_in_tree(&doc, first, second));
}
