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
        .spawn(move || children(&doc, &computed, owner))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        actual,
        vec![
            Content::Node(first),
            Content::Node(middle),
            Content::Node(last)
        ]
    );
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

#[test]
fn generated_contents_classification_preserves_css22_visibility_and_rebuilds() {
    let mut doc = Document::new();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(sheet, "span::before{content:'X'}");
    let table = doc.append_element(
        Some(0),
        "div",
        Style::default(),
        Some("display:table;border-collapse:separate;empty-cells:hide"),
    );
    let row = doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:table-row"),
    );
    let span = doc.append_element(
        Some(row),
        "span",
        Style::default(),
        Some("display:contents"),
    );
    for (owner_visibility, pseudo_visibility, expected) in [
        ("visible", "visible", false),
        ("hidden", "visible", true),
        ("visible", "hidden", true),
        ("visible", "visible", false),
    ] {
        doc.set_element_inline_style(
            row,
            Some(format!("display:table-row;visibility:{owner_visibility}").into()),
        );
        doc.set_element_inline_style(
            span,
            Some(format!("display:contents;visibility:{pseudo_visibility}").into()),
        );
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
        assert_eq!(doc.table_objects.cells.len(), 1);
        let cell = &doc.table_objects.cells[0];
        assert!(cell.node.children.is_empty());
        assert_eq!(cell.content, vec![Content::Before(span)]);
        assert_eq!(cell.node.hides_empty_table_cell, expected);
    }
}

#[test]
fn deep_generated_contents_stream_is_bounded_and_keeps_real_children() {
    let mut doc = Document::new();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(sheet, "span::before{content:'X'}span::after{content:'Y'}");
    let owner = doc.append_element(Some(0), "div", Style::default(), Some("display:table-row"));
    let mut parent = owner;
    let mut wrappers = Vec::new();
    for _ in 0..2048 {
        parent = doc.append_element(
            Some(parent),
            "span",
            Style::default(),
            Some("display:contents"),
        );
        wrappers.push(parent);
    }
    let text = doc.append_text(parent, "A");
    let sources: Vec<_> = doc.nodes.iter().map(|node| node.children.clone()).collect();
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    for (node, cv) in doc.nodes.iter_mut().zip(&computed.computed) {
        node.display = cv.display;
    }
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            let actual = children(&doc, &computed, owner);
            let expected: Vec<_> = wrappers
                .iter()
                .copied()
                .map(Content::Before)
                .chain(std::iter::once(Content::Node(text)))
                .chain(wrappers.iter().rev().copied().map(Content::After))
                .collect();
            assert_eq!(actual, expected);
            assert!(actual.len() <= doc.node_count() * 3);
            assert_eq!(
                doc.nodes
                    .iter()
                    .map(|node| node.children.clone())
                    .collect::<Vec<_>>(),
                sources
            );
        })
        .unwrap()
        .join()
        .unwrap();
}
