use super::*;
use raikiri_dom::{BundledFace, Document, build_bundled_font_collection, layout_single_page};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::PageBox;
use taffy::Style;

#[test]
fn inside_text_marker_group_lookup_has_a_work_bound() {
    let mut doc = Document::new();
    doc.set_font_collection(
        build_bundled_font_collection(
            vec![BundledFace {
                family: "Ahem".into(),
                bytes: include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
                ))
                .to_vec(),
            }],
            false,
        )
        .unwrap(),
    );
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let ol = doc.append_element(
        Some(body),
        "ol",
        Style::default(),
        Some("display:block;list-style:decimal-leading-zero inside;font:10px/20px Ahem"),
    );
    let li = doc.append_element(Some(ol), "li", Style::default(), Some("display:list-item"));
    doc.append_text(li, "Filler Text");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let root = doc.get_node(li).unwrap();
    let pieces = root.ifc_inline_boxes().unwrap();
    assert!(!pieces.is_empty());
    assert!(pieces.iter().any(|piece| piece.parent == Some(piece.node)));
    PARENT_GROUP_VISITS.with(|visits| visits.set(0));
    let paint = TypographicPaint::new(root, &pieces);
    for piece in &pieces {
        assert_eq!(paint.piece_group(piece), None);
    }
    let visits = PARENT_GROUP_VISITS.with(|visits| visits.get());
    assert!(visits <= 4 * pieces.len());
}

#[test]
fn nested_first_letter_group_lookup_has_a_document_wide_work_bound() {
    for opacity in [
        "",
        "body::first-letter{opacity:.5}",
        "div::first-letter{opacity:.5}",
    ] {
        let mut doc = Document::new();
        doc.set_font_collection(
            build_bundled_font_collection(
                vec![BundledFace {
                    family: "Ahem".into(),
                    bytes: include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
                    ))
                    .to_vec(),
                }],
                false,
            )
            .unwrap(),
        );
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let style = doc.append_element(Some(html), "style", Style::default(), Some("display:none"));
        doc.append_text(style, format!("div::first-letter{{color:red}} {opacity}"));
        let mut root = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;width:100px;font:10px/40px Ahem"),
        );
        for _ in 0..64 {
            root = doc.append_element(Some(root), "div", Style::default(), Some("display:block"));
        }
        doc.append_text(root, "X");
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 80.0;
        layout_single_page(&mut doc, &computed, page).unwrap();
        let root = doc.get_node(root).unwrap();
        let pieces = root.ifc_inline_boxes().unwrap();
        assert!(pieces.len() >= 64);
        for piece in &pieces {
            if let Some(parent) = root.ifc_typographic_parent(piece.node) {
                assert!(
                    pieces
                        .iter()
                        .any(|ancestor| { ancestor.node == parent && ancestor.line == piece.line })
                );
            }
        }
        PARENT_GROUP_VISITS.with(|visits| visits.set(0));
        let paint = TypographicPaint::new(root, &pieces);
        for piece in &pieces {
            paint.piece_group(piece);
        }
        let visits = PARENT_GROUP_VISITS.with(|visits| visits.get());
        assert!(
            visits <= 4 * pieces.len(),
            "{visits} ancestor visits for {} pieces",
            pieces.len()
        );
    }
}
