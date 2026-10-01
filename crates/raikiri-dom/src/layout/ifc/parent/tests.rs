use crate::layout::ifc::test_support::{block_fixture, span};
use crate::node::NodeFlags;

#[test]
fn a_box_under_inline_elements_is_located_from_the_root() {
    let mut fixture = block_fixture("", |doc, root| {
        let outer = span(doc, root, "");
        let inner = span(doc, outer, "");
        span(doc, inner, "display:inline-block");
    });
    let root = fixture.root;
    let outer = fixture.doc.nodes[root].children[0];
    let inner = fixture.doc.nodes[outer].children[0];
    let atomic = fixture.doc.nodes[inner].children[0];
    // Mark the paragraph as the assignment would: the inline elements are
    // its content, the inline-block one of its boxes.
    fixture.doc.nodes[root].flags.insert(NodeFlags::IS_IFC_ROOT);
    for id in [outer, inner] {
        fixture.doc.nodes[id]
            .flags
            .insert(NodeFlags::IN_IFC_SUBTREE);
    }
    let doc = &fixture.doc;
    assert_eq!(doc.layout_parent_of(atomic), Some(root));
    assert_eq!(doc.layout_parent_of(inner), Some(outer));
    assert_eq!(doc.layout_parent_of(outer), Some(root));
    assert!(!doc.contributes_layout_offset(inner));
    assert!(doc.contributes_layout_offset(root));
    assert!(doc.contributes_layout_offset(atomic));
}
