use super::*;
use crate::document::Document;
use raikiri_style::{build_rule_tree, cascade};

#[test]
fn physical_height_minima_follow_content_and_restore_the_cell() {
    let mut failures = Vec::new();
    for (style, text, width, expected) in [
        ("writing-mode:vertical-rl", "AA BB", 60.0, 20.0),
        ("writing-mode:vertical-lr", "AA BB", 60.0, 20.0),
        (
            "writing-mode:vertical-rl;white-space:nowrap",
            "AA BB",
            60.0,
            50.0,
        ),
        ("writing-mode:vertical-rl", "", 60.0, 0.0),
        ("writing-mode:vertical-rl;min-height:20px", "x", 60.0, 20.0),
        (
            "writing-mode:vertical-rl;padding:3px;border:1px solid",
            "x",
            60.0,
            18.0,
        ),
        ("writing-mode:horizontal-tb", "AA BB", 20.0, 20.0),
        (
            "writing-mode:horizontal-tb;min-height:20px",
            "x",
            60.0,
            20.0,
        ),
        (
            "writing-mode:vertical-rl;min-height:max-content",
            "AA BB",
            60.0,
            20.0,
        ),
        ("writing-mode:vertical-rl;min-height:75%", "x", 60.0, 10.0),
        (
            "writing-mode:vertical-rl;min-height:20px;padding:3px;border:1px solid;box-sizing:content-box",
            "x",
            60.0,
            28.0,
        ),
    ] {
        for wrapped in [false, true] {
            let mut doc = Document::new();
            crate::layout::test_support::with_ahem(&mut doc);
            let html = doc.append_element(Some(0), "html", Some("display:block"));
            let cell = doc.append_element(Some(html), "td", Some(&format!("display:table-cell;box-sizing:border-box;font-family:Ahem;font-size:10px;line-height:10px;width:60px;height:50px;{style}")));
            let content = if wrapped {
                doc.append_element(Some(cell), "div", Some("display:block"))
            } else {
                cell
            };
            doc.append_text(content, text);
            doc.mark_in_document_flags();
            let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
            crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
            let mut tree = LayoutSession::bridged(&mut doc, &computed);
            let height = tree.table_style(cell).size().height;
            let before = [tree.nodes[cell].layout, tree.nodes[content].layout];
            for _ in 0..2 {
                let actual = minimum_height(&mut tree, cell, width);
                if actual != expected {
                    failures.push(format!(
                        "{style}, wrapped={wrapped}: got {actual}, want {expected}; min={:?}",
                        tree.table_style(cell).min_size().height
                    ));
                }
                assert_eq!(tree.table_style(cell).size().height, height);
                assert_eq!(
                    [tree.nodes[cell].layout, tree.nodes[content].layout],
                    before
                );
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
