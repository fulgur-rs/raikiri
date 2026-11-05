use super::*;

#[test]
fn rebreak_context_membership_is_separate_from_width_projection() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block;width:400px"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;overflow:hidden;margin:0"),
    );
    let mut cases = Vec::new();
    for (tag, css, expected) in [
        ("div", "display:flow-root", true),
        ("div", "display:inline-block", true),
        ("div", "display:table-cell", true),
        ("div", "display:table-caption", true),
        ("fieldset", "display:block", true),
        ("div", "display:block;float:left", true),
        ("div", "display:block;position:absolute", true),
        ("div", "display:block;position:fixed", true),
        ("div", "display:block;overflow:auto;margin-left:5px", true),
        ("div", "display:contents;overflow:auto", false),
        ("div", "display:none;contain:paint", false),
        ("span", "display:inline;overflow:auto", false),
        ("div", "display:block;overflow:clip", false),
    ] {
        let node = doc.append_element(Some(body), tag, Some(css));
        cases.push((node, css, expected));
    }
    let vertical = doc.append_element(
        Some(body),
        "div",
        Some("display:flow-root;writing-mode:vertical-rl"),
    );
    let text = doc.append_text(body, "text");
    doc.mark_in_document_flags();
    let cascade = bridged(&mut doc);
    let projected = AncestorFacts::new(&doc, &cascade, 800.0);
    let rebreak = AncestorFacts::for_rebreak(&doc, &cascade, 800.0);
    for (node, css, expected) in cases {
        assert!(!projected.context_root[node], "{css}");
        assert_eq!(rebreak.sized_by_context_root(node), expected, "{css}");
    }
    assert!(!rebreak.sized_by_context_root(body));
    assert!(!rebreak.sized_by_context_root(text));
    assert!(!rebreak.sized_by_context_root(vertical));
    assert_eq!(projected.context_root, rebreak.context_root);
    assert_eq!(projected.projected_auto_width, rebreak.projected_auto_width);
    assert_eq!(projected.content_width, rebreak.content_width);
    assert_eq!(projected.laid_out_container, rebreak.laid_out_container);
}

#[test]
fn viewport_overflow_keeps_existing_projection_classification() {
    for (html_overflow, body_overflow, body_context) in [
        ("visible", "hidden", false),
        ("hidden", "visible", false),
        ("hidden", "hidden", true),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(
            Some(0),
            "html",
            Some(&format!(
                "display:block;width:400px;overflow:{html_overflow}"
            )),
        );
        let body = doc.append_element(
            Some(html),
            "body",
            Some(&format!("display:block;margin:0;overflow:{body_overflow}")),
        );
        doc.mark_in_document_flags();
        let cascade = bridged(&mut doc);
        let projected = AncestorFacts::new(&doc, &cascade, 800.0);
        let rebreak = AncestorFacts::for_rebreak(&doc, &cascade, 800.0);
        assert!(!projected.context_root[html]);
        assert!(!rebreak.sized_by_context_root(html));
        assert_eq!(projected.context_root[body], body_context);
        assert_eq!(rebreak.sized_by_context_root(body), body_context);
        assert_eq!(projected.context_root, rebreak.context_root);
        assert_eq!(projected.content_width, rebreak.content_width);
    }
}
