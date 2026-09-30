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

fn minimal_snapshot() -> LogicalSnapshot {
    Document::new().logical_snapshot()
}

fn element_node(tag: &str, template: Option<usize>) -> LogicalNode {
    LogicalNode {
        data: LogicalData::Element {
            tag: tag.to_owned(),
            namespace: Some("http://www.w3.org/1999/xhtml".to_owned()),
            prefix: None,
            attributes: Vec::new(),
            inline_style: None,
            template,
        },
        children: Vec::new(),
    }
}

#[test]
fn reject_empty_arena_and_root_out_of_bounds() {
    let empty = LogicalSnapshot {
        root: 0,
        nodes: Vec::new(),
        quirks: 0,
        stylesheets: Vec::new(),
    };
    assert!(empty.validate(100).is_err());
    assert!(Document::from_logical_snapshot(empty, 100).is_err());

    let mut out_of_bounds = minimal_snapshot();
    out_of_bounds.root = out_of_bounds.nodes.len() + 5;
    assert!(out_of_bounds.validate(100).is_err());
    assert!(Document::from_logical_snapshot(out_of_bounds, 100).is_err());

    let too_many = minimal_snapshot();
    assert!(too_many.validate(0).is_err());
}

#[test]
fn reject_non_document_root_kind() {
    let mut bad = minimal_snapshot();
    bad.nodes[bad.root].data = LogicalData::Text("root text".to_owned());
    assert!(bad.validate(100).is_err());
    assert!(Document::from_logical_snapshot(bad, 100).is_err());

    let mut fragment_root = LogicalSnapshot {
        root: 1,
        nodes: vec![
            LogicalNode {
                data: LogicalData::Document,
                children: Vec::new(),
            },
            LogicalNode {
                data: LogicalData::Text("not a root kind".to_owned()),
                children: Vec::new(),
            },
        ],
        quirks: 0,
        stylesheets: Vec::new(),
    };
    // Root points at Text, so validation must fail even though node 0 is a Document.
    assert!(fragment_root.validate(100).is_err());
    fragment_root.root = 0;
    // Node 1 is detached Text without children, node 0 is Document: valid.
    assert!(fragment_root.validate(100).is_ok());
}

#[test]
fn reject_bad_document_metadata() {
    let mut bad_quirks = minimal_snapshot();
    bad_quirks.quirks = 3;
    assert!(bad_quirks.validate(100).is_err());

    let mut bad_stylesheet = minimal_snapshot();
    bad_stylesheet.stylesheets = vec![("p { color: red; }".to_owned(), 3)];
    assert!(bad_stylesheet.validate(100).is_err());

    // Boundary values 0..=2 remain valid.
    let mut good = minimal_snapshot();
    good.quirks = 2;
    good.stylesheets = vec![
        ("u".to_owned(), 0),
        ("v".to_owned(), 1),
        ("w".to_owned(), 2),
    ];
    assert!(good.validate(100).is_ok());
}

#[test]
fn reject_extra_document_root() {
    let mut bad = minimal_snapshot();
    bad.nodes.push(LogicalNode {
        data: LogicalData::Document,
        children: Vec::new(),
    });
    assert!(bad.validate(100).is_err());
    assert!(Document::from_logical_snapshot(bad, 100).is_err());
}

#[test]
fn reject_leaf_children() {
    for leaf in [
        LogicalData::Text("leaf".to_owned()),
        LogicalData::Comment("leaf".to_owned()),
        LogicalData::ProcessingInstruction("target".to_owned(), "data".to_owned()),
    ] {
        let bad = LogicalSnapshot {
            root: 0,
            nodes: vec![
                LogicalNode {
                    data: LogicalData::Document,
                    children: vec![1],
                },
                LogicalNode {
                    data: leaf,
                    children: vec![2],
                },
                LogicalNode {
                    data: LogicalData::Fragment,
                    children: Vec::new(),
                },
            ],
            quirks: 0,
            stylesheets: Vec::new(),
        };
        assert!(bad.validate(100).is_err());
    }
}

#[test]
fn reject_template_link_variants() {
    // Non-template tag carrying a template link.
    let mut bad_tag = LogicalSnapshot {
        root: 0,
        nodes: vec![
            LogicalNode {
                data: LogicalData::Document,
                children: vec![1],
            },
            LogicalNode {
                data: LogicalData::Element {
                    tag: "div".to_owned(),
                    namespace: Some("http://www.w3.org/1999/xhtml".to_owned()),
                    prefix: None,
                    attributes: Vec::new(),
                    inline_style: None,
                    template: Some(2),
                },
                children: Vec::new(),
            },
            LogicalNode {
                data: LogicalData::Fragment,
                children: Vec::new(),
            },
        ],
        quirks: 0,
        stylesheets: Vec::new(),
    };
    assert!(bad_tag.validate(100).is_err());

    // Wrong namespace on a template element.
    bad_tag.nodes[1].data = LogicalData::Element {
        tag: "template".to_owned(),
        namespace: Some("http://example.com/wrong".to_owned()),
        prefix: None,
        attributes: Vec::new(),
        inline_style: None,
        template: Some(2),
    };
    assert!(bad_tag.validate(100).is_err());

    // Template target out of bounds.
    let mut bad_bounds = minimal_snapshot();
    bad_bounds.nodes[0].children.clear();
    bad_bounds.nodes.clear();
    bad_bounds.nodes.push(LogicalNode {
        data: LogicalData::Document,
        children: vec![1],
    });
    bad_bounds.nodes.push(element_node("template", Some(99)));
    bad_bounds.root = 0;
    assert!(bad_bounds.validate(100).is_err());

    // Template target is not a fragment.
    let mut bad_target = LogicalSnapshot {
        root: 0,
        nodes: vec![
            LogicalNode {
                data: LogicalData::Document,
                children: vec![1],
            },
            element_node("template", Some(2)),
            LogicalNode {
                data: LogicalData::Text("not a fragment".to_owned()),
                children: Vec::new(),
            },
        ],
        quirks: 0,
        stylesheets: Vec::new(),
    };
    assert!(bad_target.validate(100).is_err());
    // Pointing at a real fragment makes the same shape valid.
    bad_target.nodes[2].data = LogicalData::Fragment;
    assert!(bad_target.validate(100).is_ok());
}

#[test]
fn reject_edge_to_root_and_self_loop() {
    // Child points back at the document root.
    let back_to_root = LogicalSnapshot {
        root: 0,
        nodes: vec![
            LogicalNode {
                data: LogicalData::Document,
                children: vec![1],
            },
            LogicalNode {
                data: LogicalData::Fragment,
                children: vec![0],
            },
        ],
        quirks: 0,
        stylesheets: Vec::new(),
    };
    assert!(back_to_root.validate(100).is_err());

    // Detached self-loop: single parent, but a cycle.
    let self_loop = LogicalSnapshot {
        root: 0,
        nodes: vec![
            LogicalNode {
                data: LogicalData::Document,
                children: Vec::new(),
            },
            LogicalNode {
                data: LogicalData::Fragment,
                children: vec![1],
            },
        ],
        quirks: 0,
        stylesheets: Vec::new(),
    };
    assert!(self_loop.validate(100).is_err());
}

#[test]
fn reject_detached_cycle_without_repeated_parent() {
    // Nodes 1 and 2 reference each other but each has exactly one parent,
    // so only cycle detection (not the repeated-parent check) can reject this.
    let cycle = LogicalSnapshot {
        root: 0,
        nodes: vec![
            LogicalNode {
                data: LogicalData::Document,
                children: Vec::new(),
            },
            LogicalNode {
                data: LogicalData::Fragment,
                children: vec![2],
            },
            LogicalNode {
                data: LogicalData::Fragment,
                children: vec![1],
            },
        ],
        quirks: 0,
        stylesheets: Vec::new(),
    };
    assert!(cycle.validate(100).is_err());
    assert!(Document::from_logical_snapshot(cycle, 100).is_err());
}

#[test]
fn from_logical_snapshot_maps_quirks_and_stylesheets() {
    for (quirks, mode) in [
        (0u8, QuirksMode::NoQuirks),
        (1u8, QuirksMode::LimitedQuirks),
        (2u8, QuirksMode::Quirks),
    ] {
        let mut snap = minimal_snapshot();
        snap.quirks = quirks;
        let doc = Document::from_logical_snapshot(snap, 100).unwrap();
        assert_eq!(doc.quirks_mode(), mode);
    }

    let mut snap = minimal_snapshot();
    snap.stylesheets = vec![
        ("ua".to_owned(), 0),
        ("user".to_owned(), 1),
        ("author".to_owned(), 2),
    ];
    let restored = Document::from_logical_snapshot(snap.clone(), 100).unwrap();
    assert_eq!(restored.logical_snapshot().stylesheets, snap.stylesheets);
}

#[test]
fn snapshot_error_displays_its_reason() {
    let bad = LogicalSnapshot {
        root: 0,
        nodes: Vec::new(),
        quirks: 0,
        stylesheets: Vec::new(),
    };
    let err = bad.validate(100).unwrap_err();
    assert_eq!(format!("{err}"), "arena bounds");
    assert_eq!(err.0, "arena bounds");
}

#[test]
fn logical_snapshot_maps_all_quirks_modes() {
    let mut doc = Document::new();
    doc.set_quirks_mode(QuirksMode::NoQuirks);
    assert_eq!(doc.logical_snapshot().quirks, 0);
    doc.set_quirks_mode(QuirksMode::LimitedQuirks);
    assert_eq!(doc.logical_snapshot().quirks, 1);
    doc.set_quirks_mode(QuirksMode::Quirks);
    assert_eq!(doc.logical_snapshot().quirks, 2);
}
