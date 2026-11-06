use super::*;

#[test]
fn table_discovery_keeps_source_order_and_stops_at_independent_or_bounded_flows() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let table = |doc: &mut Document, parent| {
        doc.append_element(Some(parent), "table", Some("display:table"))
    };
    let first = table(&mut doc, body);
    let outer = doc.append_element(Some(body), "div", Some("display:block;padding-bottom:30px"));
    let inner = doc.append_element(Some(outer), "div", Some("display:flow-root;min-height:0"));
    let second = table(&mut doc, inner);
    let contents = doc.append_element(Some(outer), "div", Some("display:contents"));
    let third = table(&mut doc, contents);
    table(&mut doc, first);
    for css in [
        "display:none",
        "display:flex",
        "display:grid",
        "display:inline-block",
        "display:block;column-count:2",
        "display:block;float:left",
        "display:block;position:absolute",
        "display:block;position:relative",
        "display:block;position:fixed",
        "display:block;writing-mode:vertical-rl",
        "display:block;height:100px",
        "display:block;height:50%",
        "display:block;min-height:1px",
        "display:block;max-height:100px",
        "display:block;overflow:hidden",
        "display:block;overflow:auto",
        "display:block;contain:size",
        "display:block;aspect-ratio:1",
        "display:block;height:min-content",
    ] {
        let blocked = doc.append_element(Some(body), "div", Some(css));
        table(&mut doc, blocked);
    }
    for css in [
        "height:100px",
        "min-height:1px",
        "max-height:100px",
        "overflow:hidden",
        "aspect-ratio:1",
    ] {
        doc.append_element(Some(outer), "table", Some(format!("display:table;{css}")));
    }
    let ignored_containment =
        doc.append_element(Some(outer), "table", Some("display:table;contain:size"));
    let fourth = table(&mut doc, body);
    let direct_bounded =
        doc.append_element(Some(body), "table", Some("display:table;height:100px"));
    doc.mark_in_document_flags();
    let cascade = crate::layout::test_support::cascaded(&doc);
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    assert!(!doc.nodes[ignored_containment].size_containment.height);
    assert_eq!(
        block_flow_tables(&doc, &cascade, body, 200.0),
        [
            first,
            second,
            third,
            ignored_containment,
            fourth,
            direct_bounded
        ]
    );
}

#[test]
fn nested_repeat_discovery_stops_when_the_table_itself_needs_multiple_pages() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let wrapper = doc.append_element(Some(body), "div", Some("display:block"));
    let mut nested = Vec::new();
    for height in [199.0, 200.0, 201.0, f32::INFINITY, f32::NAN] {
        let table = doc.append_element(Some(wrapper), "table", Some("display:table"));
        doc.nodes[table].layout.size.height = height;
        nested.push(table);
    }
    let direct = doc.append_element(Some(body), "table", Some("display:table"));
    doc.nodes[direct].layout.size.height = 300.0;
    doc.mark_in_document_flags();
    let cascade = crate::layout::test_support::cascaded(&doc);
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    assert_eq!(
        block_flow_tables(&doc, &cascade, body, 200.0),
        [nested[0], nested[1], direct]
    );
}
