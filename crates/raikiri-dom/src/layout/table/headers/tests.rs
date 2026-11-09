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
        "display:flex;flex-direction:column-reverse",
        "display:flex;flex-direction:column;flex-wrap:wrap",
        "display:flex;flex-direction:column;height:100px",
        "display:flex;flex-wrap:wrap",
        "display:flex;height:100px",
        "display:flex;overflow:hidden",
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
    // A single-line row flex container holding one table and nothing else
    // grows with it as a block does.
    let flex = doc.append_element(Some(body), "div", Some("display:flex"));
    let flexed = table(&mut doc, flex);
    for item in ["display:block", "display:table"] {
        let shared = doc.append_element(Some(body), "div", Some("display:flex"));
        table(&mut doc, shared);
        doc.append_element(Some(shared), "div", Some(item));
    }
    // A single-line column flex container stacks its items as block flow
    // does, so every table item shares it. A table inside another item
    // fragments with that item.
    let column = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;flex-direction:column"),
    );
    let wrapper = doc.append_element(Some(column), "div", Some("display:block"));
    table(&mut doc, wrapper);
    let contents = doc.append_element(Some(column), "div", Some("display:contents"));
    table(&mut doc, contents);
    let stacked = table(&mut doc, column);
    let texted = doc.append_element(Some(body), "div", Some("display:flex"));
    table(&mut doc, texted);
    doc.append_text(texted, "anonymous item");
    let hidden_sibling = doc.append_element(Some(body), "div", Some("display:flex"));
    let beside_hidden = table(&mut doc, hidden_sibling);
    doc.append_element(Some(hidden_sibling), "div", Some("display:none"));
    doc.append_text(hidden_sibling, " \n ");
    let flexed_block = doc.append_element(Some(body), "div", Some("display:flex"));
    doc.append_element(Some(flexed_block), "div", Some("display:block"));
    let fourth = table(&mut doc, body);
    let direct_bounded =
        doc.append_element(Some(body), "table", Some("display:table;height:100px"));
    doc.mark_in_document_flags();
    let cascade = crate::layout::test_support::cascaded(&doc);
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    assert!(!doc.nodes[ignored_containment].size_containment.height);
    assert_eq!(
        block_flow_tables(&doc, &cascade, body),
        [
            first,
            second,
            third,
            ignored_containment,
            flexed,
            stacked,
            beside_hidden,
            fourth,
            direct_bounded
        ]
    );
}

#[test]
fn nested_repeat_discovery_takes_tables_of_any_laid_out_height() {
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
        block_flow_tables(&doc, &cascade, body),
        [nested[0], nested[1], nested[2], direct]
    );
}
