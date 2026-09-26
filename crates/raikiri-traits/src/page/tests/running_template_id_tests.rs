use super::super::*;

#[test]
fn running_template_id_wraps_node_id() {
    let id = RunningTemplateId::new(NodeId::new(42));
    assert_eq!(id.0, NodeId::new(42));
}

#[test]
fn running_template_id_copy_hash_eq_derives() {
    use std::collections::HashMap;
    let id1 = RunningTemplateId::new(NodeId::new(1));
    let id2 = id1; // Copy
    assert_eq!(id1, id2);
    // Hash + Eq — HashMap key として使える (raikiri-dom
    // RunningTemplateStore.parsed_templates keying rationale)。
    let mut m: HashMap<RunningTemplateId, &'static str> = HashMap::new();
    m.insert(id1, "template-1");
    assert_eq!(m.get(&id2), Some(&"template-1"));
}

#[test]
fn running_template_id_ord_derives() {
    let id1 = RunningTemplateId::new(NodeId::new(1));
    let id2 = RunningTemplateId::new(NodeId::new(2));
    assert!(id1 < id2);
}
