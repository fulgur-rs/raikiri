use super::*;
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

#[test]
fn deep_contents_wrappers_keep_source_order_on_a_small_stack() {
    let mut doc = Document::new();
    let owner = doc.append_element(Some(0), "div", Style::default(), Some("display:table-row"));
    let first = doc.append_text(owner, "A");
    let mut parent = owner;
    for _ in 0..2048 {
        parent = doc.append_element(
            Some(parent),
            "span",
            Style::default(),
            Some("display:contents"),
        );
    }
    let middle = doc.append_text(parent, "B");
    let last = doc.append_text(owner, "C");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    let actual = std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || children(&doc, owner))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(actual, vec![first, middle, last]);
}

#[test]
fn source_owner_enumeration_is_linear_across_many_anonymous_rows() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let table = doc.append_element(Some(body), "div", Style::default(), Some("display:table"));
    let mut owners = Vec::new();
    for _ in 0..200 {
        let row = doc.append_element(
            Some(table),
            "div",
            Style::default(),
            Some("display:table-row"),
        );
        doc.append_text(row, "A");
        doc.append_element(
            Some(row),
            "div",
            Style::default(),
            Some("display:table-cell"),
        );
        owners.push(row);
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    for (node, cv) in doc.nodes.iter_mut().zip(&computed.computed) {
        node.display = cv.display;
    }
    prepare(&mut doc, &computed);
    OWNER_CELL_VISITS.with(|visits| visits.set(0));
    for owner in owners {
        let cells: Vec<_> = doc.anonymous_table_cells(owner).collect();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].1.children, vec![doc.nodes[owner].children[0]]);
        assert_eq!(
            doc.anonymous_table_paint_children(owner).unwrap(),
            doc.nodes[owner].children
        );
    }
    assert_eq!(doc.anonymous_table_cells(body).count(), 0);
    assert_eq!(doc.anonymous_table_paint_children(body), None);
    let visited = OWNER_CELL_VISITS.with(|visits| visits.get());
    assert!(
        visited <= doc.node_count() * 3,
        "owner lookup visited {visited} cells, exceeding the document-wide work bound"
    );
}
