use super::*;
use crate::Node;
use taffy::Style;

#[test]
fn source_constructors_keep_styles_and_public_getter_excludes_layout_boxes() {
    let mut document = Document::new();
    let fixed = document.append_element(
        None,
        "div",
        Style {
            position: taffy::Position::Fixed,
            ..Style::default()
        },
        None::<&str>,
    );
    let text = document.create_detached_text("text");
    let fragment = document.create_detached_fragment();
    let comment = document.create_detached_comment("comment");
    let instruction = document.append_processing_instruction(None, "target", "data");

    for id in [0, fixed, text, fragment, comment, instruction] {
        assert!(document.layout_style(id).is_some(), "source node {id}");
    }
    assert_eq!(
        document.layout_style(fixed).unwrap().position,
        taffy::Position::Absolute
    );
    assert!(document.layout_style(usize::MAX).is_none());

    let source_count = document.node_count();
    let initial_containing_block =
        document.set_initial_containing_block(Node::new_document(), Style::default());
    assert!(document.layout_style(initial_containing_block).is_none());
    assert!(
        document
            .layout_style_for_key(initial_containing_block)
            .is_some()
    );

    document.set_anonymous_tables(vec![(Node::new_document(), Style::default())]);
    let anonymous_table = source_count;
    assert!(document.layout_style(anonymous_table).is_none());
    assert!(document.layout_style_for_key(anonymous_table).is_some());

    document.set_anonymous_cells(vec![(Node::new_document(), Style::default())]);
    let anonymous_cell = anonymous_table + 1;
    assert!(document.layout_style(anonymous_cell).is_none());
    assert!(document.layout_style_for_key(anonymous_cell).is_some());
}
