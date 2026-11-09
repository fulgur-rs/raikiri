use super::*;

#[test]
fn shrinking_node_storage_drops_the_room_kept_for_more_nodes() {
    let mut doc = Document::new();
    let body = doc.append_element(Some(0), "body", None::<&str>);
    for _ in 0..100 {
        doc.append_element(Some(body), "p", None::<&str>);
    }
    assert!(doc.nodes.capacity() > doc.node_count());
    let ids: Vec<usize> = doc.preorder(doc.root_index()).collect();
    doc.shrink_node_storage();
    assert_eq!(doc.nodes.capacity(), doc.node_count());
    assert_eq!(doc.preorder(doc.root_index()).collect::<Vec<_>>(), ids);
}
