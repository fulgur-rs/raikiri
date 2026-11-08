use super::*;

fn element(sink: &RaikiriTreeSink, tag: &str) -> usize {
    sink.create_element(
        QualName::new(None, ns!(html), tag.into()),
        Vec::new(),
        ElementFlags::default(),
    )
}

/// `body > [div, table > tr]`, with the table's subtree watched. Returns
/// the sink, the body, the div, the table and the row.
fn watched_table() -> (RaikiriTreeSink, usize, usize, usize, usize) {
    let sink = RaikiriTreeSink::new(None);
    let root = sink.get_document();
    let body = element(&sink, "body");
    sink.append(&root, NodeOrText::AppendNode(body));
    let div = element(&sink, "div");
    sink.append(&body, NodeOrText::AppendNode(div));
    let table = element(&sink, "table");
    sink.append(&body, NodeOrText::AppendNode(table));
    let row = element(&sink, "tr");
    sink.append(&table, NodeOrText::AppendNode(row));
    sink.watch_subtree(table);
    (sink, body, div, table, row)
}

#[test]
fn changes_inside_the_subtree_keep_the_watch() {
    let (sink, _, _, table, row) = watched_table();
    let cell = element(&sink, "td");
    sink.append(&row, NodeOrText::AppendNode(cell));
    sink.append(&cell, NodeOrText::AppendText("text".into()));
    let second = element(&sink, "tr");
    sink.append_before_sibling(&row, NodeOrText::AppendNode(second));
    sink.reparent_children(&row, &second);
    sink.remove_from_parent(&cell);
    assert_eq!(sink.only_watched_subtree_changed(), Some(table));
    sink.stop_watching_subtree();
    assert_eq!(sink.only_watched_subtree_changed(), None);
}

#[test]
fn changes_outside_the_subtree_end_the_watch() {
    type Change = fn(&RaikiriTreeSink, usize, usize, usize, usize);
    let changes: [(&str, Change); 7] = [
        ("element after the table", |sink, body, _, _, _| {
            let p = element(sink, "p");
            sink.append(&body, NodeOrText::AppendNode(p));
        }),
        (
            "element fostered before the table",
            |sink, _, _, table, _| {
                let p = element(sink, "p");
                sink.append_before_sibling(&table, NodeOrText::AppendNode(p));
            },
        ),
        ("text outside", |sink, body, _, _, _| {
            sink.append(&body, NodeOrText::AppendText("text".into()));
        }),
        ("node moved in from outside", |sink, _, div, _, row| {
            sink.append(&row, NodeOrText::AppendNode(div));
        }),
        ("node removed outside", |sink, _, div, _, _| {
            sink.remove_from_parent(&div);
        }),
        ("children moved outside", |sink, body, div, _, _| {
            sink.reparent_children(&body, &div);
        }),
        ("attributes merged into the body", |sink, body, _, _, _| {
            let attribute = Attribute {
                name: QualName::new(None, ns!(), "class".into()),
                value: "late".into(),
            };
            sink.add_attrs_if_missing(&body, vec![attribute]);
        }),
    ];
    for (name, change) in changes {
        let (sink, body, div, table, row) = watched_table();
        change(&sink, body, div, table, row);
        assert_eq!(sink.only_watched_subtree_changed(), None, "{name}");
    }
}
