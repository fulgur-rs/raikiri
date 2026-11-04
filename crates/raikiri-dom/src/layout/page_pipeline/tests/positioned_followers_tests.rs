use super::*;

fn block(document: &mut Document, parent: usize, style: &str) -> usize {
    document.append_element(Some(parent), "div", Some(style))
}

#[test]
fn height_dependent_positioned_subtrees_are_recomputed_on_repeated_layouts() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    let body = document.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px"),
    );
    let containing = block(
        &mut document,
        body,
        "display:block;position:relative;height:300px",
    );
    let owner = block(
        &mut document,
        containing,
        "display:block;position:absolute;top:75px;left:0;width:80px;font-family:Ahem;font-size:20px;line-height:30px",
    );
    for text in ["A", "B"] {
        let paragraph = block(&mut document, owner, "display:block;margin:0");
        document.append_text(paragraph, text);
    }
    let child = block(
        &mut document,
        owner,
        "display:block;position:absolute;bottom:0;left:60px;width:10px;height:50%",
    );
    document.mark_in_document_flags();
    with_ahem(&mut document);
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    for pass in 0..3 {
        layout_pages(&mut document, &cascade, page).unwrap();
        assert_eq!(
            document.nodes[child].layout.size.height, 42.5,
            "pass {pass}"
        );
        assert_eq!(
            document.layout_origin_of(child).unwrap().y,
            117.5,
            "pass {pass}"
        );
    }
}
