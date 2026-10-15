use super::*;

fn element(tag: &str) -> Node {
    Node::new_element(tag.into(), taffy::Style::default(), None)
}

fn arena() -> NodeArena {
    let mut arena = NodeArena::from(vec![element("a"), element("b")]);
    arena.set_anonymous_cells(vec![element("cell")]);
    arena
}

#[test]
fn keys_past_the_source_nodes_name_anonymous_cells() {
    let arena = arena();
    assert_eq!(arena.box_key(1), Some(BoxKey::Source(1)));
    assert_eq!(arena.box_key(2), Some(BoxKey::AnonymousCell(0)));
    assert_eq!(arena.box_key(3), None);
    assert_eq!(arena.anonymous_cell_key(0), 2);
    assert!(arena.is_anonymous_cell(2));
    assert!(!arena.is_anonymous_cell(1));
    assert!(!arena.is_anonymous_cell(3));
}

#[test]
fn indexing_resolves_both_kinds_while_the_dom_sees_source_nodes_only() {
    let mut arena = arena();
    assert_eq!(arena[0].tag_name(), Some("a"));
    assert_eq!(arena[2].tag_name(), Some("cell"));
    assert_eq!(arena.layout_node(2).and_then(Node::tag_name), Some("cell"));
    assert!(arena.layout_node(3).is_none());
    arena[2].children.push(1);
    assert_eq!(arena.anonymous_cells()[0].children, vec![1]);
    arena[1].children.push(0);
    assert_eq!(arena.len(), 2);
    assert!(arena.get(2).is_none());
    assert_eq!((&arena).into_iter().count(), 2);
    assert_eq!((&mut arena).into_iter().count(), 2);
    assert_eq!(
        arena
            .iter()
            .filter(|node| !node.children.is_empty())
            .count(),
        1
    );
}

#[test]
#[should_panic]
fn indexing_past_every_node_panics() {
    let arena = arena();
    let _ = &arena[3];
}

#[test]
#[should_panic]
fn mutable_indexing_past_every_node_panics() {
    let mut arena = arena();
    arena[3].children.clear();
}

#[test]
fn a_layout_pass_reaches_every_node_and_keys_are_fixed_when_cells_are_set() {
    let mut arena: NodeArena = [element("a")].into_iter().collect();
    arena.set_anonymous_cells(vec![element("cell")]);
    assert_eq!(arena.anonymous_cell_key(0), 1);
    assert_eq!(arena.layout_nodes_mut().count(), 2);
    arena.push(element("b"));
    assert_eq!(arena.anonymous_cell_key(0), 1);
    assert_eq!(arena.box_key(1), Some(BoxKey::Source(1)));
    arena.set_anonymous_cells(vec![element("cell")]);
    assert_eq!(arena.anonymous_cell_key(0), 2);
    assert_eq!(arena.box_key(2), Some(BoxKey::AnonymousCell(0)));
}
