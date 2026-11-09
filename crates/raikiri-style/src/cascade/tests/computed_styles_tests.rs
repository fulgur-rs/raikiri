//! Nodes whose results the cascade copies from a sibling or a cousin share
//! one allocation of [`ComputedValues`], and a write to one node never shows
//! through another.

use super::*;
use crate::property::{DisplayValue, WritingMode};
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;

/// A table of `rows` rows of three cells, each cell holding text, under
/// `<body>`; returns the cascade and the ids of the cells in document order.
fn table(rows: usize) -> (CascadeResult, Vec<usize>, Vec<usize>) {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let style = doc.push_element(html, "style", None);
    doc.push_text(style, "td { padding: 2px; border: 1px solid black }");
    let body = doc.push_element(html, "body", None);
    let table = doc.push_element(body, "table", None);
    let mut cells = Vec::new();
    let mut texts = Vec::new();
    for row in 0..rows {
        let tr = doc.push_element(table, "tr", None);
        for column in 0..3 {
            let td = doc.push_element(tr, "td", None);
            texts.push(doc.push_text(td, &format!("{row}.{column}")));
            cells.push(td);
        }
    }
    let tree = build_rule_tree(&doc);
    (cascade(&doc, &tree).expect("cascade Ok"), cells, texts)
}

#[test]
fn cells_of_repeated_rows_share_one_allocation() {
    let (result, cells, texts) = table(4);
    for &cell in &cells[1..] {
        assert!(result.computed.shares(cells[0], cell), "cell {cell}");
    }
    for &text in &texts[1..] {
        assert!(result.computed.shares(texts[0], text), "text {text}");
    }
    assert!(!result.computed.shares(cells[0], texts[0]));
}

#[test]
fn a_write_to_one_node_leaves_the_nodes_it_shared_with_alone() {
    let (mut result, cells, _) = table(2);
    let before = result.computed[cells[1]].clone();
    result.computed[cells[0]].writing_mode = WritingMode::VerticalRl;
    assert_eq!(
        result.computed[cells[0]].writing_mode,
        WritingMode::VerticalRl
    );
    assert_eq!(result.computed[cells[1]], before);
    assert!(!result.computed.shares(cells[0], cells[1]));
    assert!(result.computed.shares(cells[1], cells[2]));

    result.computed.get_mut(cells[2]).expect("in range").display = DisplayValue::None;
    assert_eq!(result.computed[cells[3]], before);
    assert!(result.computed.get_mut(result.computed.len()).is_none());
}

#[test]
fn an_extracted_cascade_keeps_sharing() {
    let (result, cells, _) = table(2);
    let extracted = result.extract(&[0, cells[0], cells[4]]);
    assert_eq!(extracted.computed.len(), 3);
    assert!(extracted.computed.shares(1, 2));
    assert_eq!(extracted.computed[1], result.computed[cells[4]]);
}

#[test]
fn computed_styles_reads_and_writes_like_a_vector() {
    let initial = ComputedValues::initial();
    let mut other = ComputedValues::initial();
    other.display = DisplayValue::None;

    let mut styles = ComputedStyles::new();
    assert!(styles.is_empty());
    assert_eq!(styles.last(), None);
    styles.push(initial.clone());
    styles.resize(4, other.clone());
    assert_eq!(styles.len(), 4);
    assert!(styles.shares(1, 3));
    assert_eq!(styles.get(0), Some(&initial));
    assert_eq!(styles.get(4), None);
    assert_eq!(styles.last(), Some(&other));

    styles.set(2, initial.clone());
    assert!(!styles.shares(1, 2));
    assert_eq!(
        styles
            .iter()
            .map(|values| values.display)
            .collect::<Vec<_>>(),
        [
            initial.display,
            DisplayValue::None,
            initial.display,
            DisplayValue::None
        ]
    );
    assert_eq!(styles.iter().rev().len(), 4);

    for values in &mut styles {
        values.display = DisplayValue::Contents;
    }
    assert!(
        (&styles)
            .into_iter()
            .all(|values| values.display == DisplayValue::Contents)
    );
    for values in styles.iter_mut() {
        values.display = DisplayValue::Block;
    }
    assert!(
        styles
            .iter()
            .all(|values| values.display == DisplayValue::Block)
    );

    styles.resize(2, other.clone());
    assert_eq!(styles.len(), 2);
    styles.truncate(1);
    styles.extend([other.clone()]);
    assert_eq!(
        styles,
        ComputedStyles::from(vec![
            {
                let mut block = initial.clone();
                block.display = DisplayValue::Block;
                block
            },
            other,
        ])
    );
}
