use super::*;
use crate::Node;
use crate::node::arena::BoxKey;

#[test]
fn layout_boxes_take_keys_after_the_source_nodes() {
    let mut document = Document::new();
    let fixed = document.append_element(None, "div", Some("position:fixed"));
    let text = document.create_detached_text("text");
    let fragment = document.create_detached_fragment();
    let comment = document.create_detached_comment("comment");
    let instruction = document.append_processing_instruction(None, "target", "data");

    for id in [0, fixed, text, fragment, comment, instruction] {
        assert_eq!(
            document.nodes.box_key(id),
            Some(BoxKey::Source(id)),
            "source node {id}"
        );
    }
    assert_eq!(document.nodes.box_key(usize::MAX - 1), None);

    let source_count = document.node_count();
    let initial_containing_block = document.set_initial_containing_block(Node::new_document());
    assert!(initial_containing_block >= source_count);
    assert_eq!(
        document.nodes.box_key(initial_containing_block),
        Some(BoxKey::InitialContainingBlock)
    );

    document.set_anonymous_tables(vec![Node::new_document()]);
    let anonymous_table = source_count;
    assert_eq!(
        document.nodes.box_key(anonymous_table),
        Some(BoxKey::AnonymousTable(0))
    );

    document.set_anonymous_cells(vec![Node::new_document()]);
    let anonymous_cell = anonymous_table + 1;
    assert_eq!(
        document.nodes.box_key(anonymous_cell),
        Some(BoxKey::AnonymousCell(0))
    );
}
