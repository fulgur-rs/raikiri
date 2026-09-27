use super::*;

fn rich_document() -> Document {
    let mut doc = Document::new();
    let e = doc.append_element(Some(0), "template", Default::default(), Some("color:red"));
    let fragment = doc.allocate_template_fragment_root(e);
    doc.append_text(fragment, "template text");
    doc.append_comment(Some(0), "comment");
    doc.append_processing_instruction(Some(0), "target", "data");
    let detached = doc.append_element(None, "svg", Default::default(), None::<&str>);
    let element = doc.nodes[detached].data.as_element_mut().unwrap();
    element.namespace = Some("http://www.w3.org/2000/svg".into());
    element.prefix = Some("s".into());
    element.attributes = vec![Attr {
        namespace: Some("urn:test".into()),
        prefix: Some("x".into()),
        local: "name".into(),
        value: "value".into(),
    }];
    doc.set_quirks_mode(QuirksMode::LimitedQuirks);
    doc.add_stylesheet("p {color:blue}", StylesheetKind::Author);
    doc
}

#[test]
fn roundtrip_all_logical_kinds() {
    let before = rich_document().logical_snapshot();
    assert_eq!(
        Document::from_logical_snapshot(before.clone(), 100)
            .unwrap()
            .logical_snapshot(),
        before
    );
}

#[test]
#[cfg(feature = "snapshot-serde")]
fn roundtrip_detached_template_namespaced_attributes() {
    let before = rich_document().logical_snapshot();
    let wire = serde_json::to_vec(&before).unwrap();
    let decoded = serde_json::from_slice(&wire).unwrap();
    assert_eq!(
        Document::from_logical_snapshot(decoded, 100)
            .unwrap()
            .logical_snapshot(),
        before
    );
}

#[test]
fn reject_corrupt_arena() {
    let good = rich_document().logical_snapshot();
    let mut bad = good.clone();
    bad.nodes[0].children.push(42);
    assert!(bad.validate(100).is_err());
    let mut bad = good.clone();
    let child = bad.nodes[0].children[0];
    bad.nodes[0].children.push(child);
    assert!(bad.validate(100).is_err());
    let mut bad = good.clone();
    bad.root = 2;
    assert!(bad.validate(100).is_err());
    let mut bad = good.clone();
    if let LogicalData::Element { template, .. } = &mut bad.nodes[1].data {
        *template = Some(0);
    }
    assert!(bad.validate(100).is_err());
    let mut bad = good.clone();
    bad.nodes.push(LogicalNode {
        data: LogicalData::Fragment,
        children: vec![good.nodes.len()],
    });
    assert!(bad.validate(100).is_err());
    assert!(good.validate(1).is_err());
}

#[test]
fn deep_validation_is_iterative() {
    let mut s = Document::new().logical_snapshot();
    for i in 1..20_000 {
        s.nodes[i - 1].children.push(i);
        s.nodes.push(LogicalNode {
            data: LogicalData::Fragment,
            children: vec![],
        });
    }
    s.validate(20_000).unwrap();
}

#[test]
fn layout_roundtrip_preserves_logical_ids() {
    let mut doc = rich_document();
    let before = doc.logical_snapshot();
    for node in &mut doc.nodes {
        node.cache = Default::default();
    }
    let mut restored = Document::from_logical_snapshot(doc.logical_snapshot(), 100).unwrap();
    restored.mark_in_document_flags();
    assert_eq!(restored.logical_snapshot(), before);
}
