use super::LayoutSession;
use crate::document::Document;
use crate::node::Node;
use crate::node::arena::BoxKey;

#[test]
fn a_new_session_starts_with_empty_warning_and_error_scratch() {
    let mut document = Document::new();
    {
        let mut session = LayoutSession::new(&mut document);
        session
            .context
            .layout_warnings
            .push(crate::layout::LayoutWarn::NonFiniteClamped {
                site: "test",
                raw: f32::INFINITY,
                clamped: f32::MAX,
            });
        session.context.table_layout_error = Some("table failed".to_owned());
    }

    let session = LayoutSession::new(&mut document);
    assert!(session.context.layout_warnings.is_empty());
    assert!(session.context.table_layout_error.is_none());
}

#[test]
fn layout_session_state_covers_every_box_key_kind() {
    let mut document = Document::new();
    document
        .nodes
        .set_anonymous_tables(vec![Node::new_document()]);
    document
        .nodes
        .set_anonymous_cells(vec![Node::new_document()]);
    document
        .nodes
        .set_initial_containing_block(Node::new_document());

    let source = 0;
    let table = document.nodes.anonymous_table_key(0);
    let cell = document.nodes.anonymous_cell_key(0);
    let initial_containing_block = usize::MAX;
    let session = LayoutSession::new(&mut document);

    assert_eq!(
        session.node_state(source).unwrap().box_key,
        BoxKey::Source(0)
    );
    session.node_state(source).unwrap().taffy_child_walks = Some(Vec::new());
    assert_eq!(
        session.node_state(table).unwrap().box_key,
        BoxKey::AnonymousTable(0)
    );
    assert!(
        session
            .node_state(table)
            .unwrap()
            .taffy_child_walks
            .is_none()
    );
    assert_eq!(
        session.node_state(cell).unwrap().box_key,
        BoxKey::AnonymousCell(0)
    );
    assert!(
        session
            .node_state(cell)
            .unwrap()
            .taffy_child_walks
            .is_none()
    );
    assert_eq!(
        session
            .node_state(initial_containing_block)
            .unwrap()
            .box_key,
        BoxKey::InitialContainingBlock
    );
    assert!(
        session
            .node_state(initial_containing_block)
            .unwrap()
            .taffy_child_walks
            .is_none()
    );
    assert!(session.node_state(usize::MAX - 1).is_none());
    assert!(
        session
            .node_state(source)
            .unwrap()
            .taffy_child_walks
            .is_some()
    );
}
