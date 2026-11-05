use super::*;
use crate::layout::relayout_text_for_width;

#[test]
fn relayout_keeps_unprojected_contexts_within_the_room_beside_a_float() {
    for (css, width) in [
        ("display:flow-root", 70.0),
        ("display:inline-block;max-width:70px", 70.0),
        ("overflow:hidden;margin-left:5px;margin-right:5px", 65.0),
        ("contain:paint;margin-left:5px;margin-right:5px", 65.0),
        (
            "align-content:center;margin-left:5px;margin-right:5px",
            65.0,
        ),
    ] {
        let (mut doc, cascade, _, root) = ahem_paragraph_beside_floats(
            "aaaa bbbb cccc",
            &["float:left;width:30px;height:60px"],
            css,
        );
        lay_out(&mut doc, &cascade);
        assert_eq!(doc.nodes[root].layout.size.width, width, "{css}");
        assert_eq!(stored_lines(&doc, root).lines.len(), 3, "{css}");
        let geometry = doc.nodes[root].layout;
        for max_advance in [50.0, 800.0] {
            relayout_text_for_width(&mut doc, &cascade, max_advance);
            assert_eq!(stored_lines(&doc, root).lines.len(), 3, "{css}");
            assert_eq!(stored_lines(&doc, root).width, width, "{css}");
            assert_eq!(doc.nodes[root].layout, geometry, "{css}");
        }
    }
}

fn context_paragraph(
    css: &str,
    nested: bool,
    contents: bool,
    inline_parent: bool,
) -> (Document, CascadeResult, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    doc.append_element(
        Some(parent),
        "div",
        Some("display:block;float:left;width:30px;height:60px"),
    );
    if inline_parent {
        doc.append_text(parent, "x");
    }
    let context = doc.append_element(Some(parent), "div", Some(&format!("display:block;{css}")));
    let holder = if contents {
        doc.append_element(Some(context), "div", Some("display:contents"))
    } else {
        context
    };
    let root = if nested {
        doc.append_element(Some(holder), "div", Some("display:block"))
    } else {
        holder
    };
    doc.append_text(root, "aaaa bbbb cccc");
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    (doc, cascade, parent, root)
}

#[test]
fn relayout_keeps_context_descendants_and_ifc_children_within_used_widths() {
    for css in [
        "display:flow-root",
        "overflow:hidden;margin-left:5px;margin-right:5px",
        "display:inline-block;max-width:70px",
    ] {
        for (nested, contents, inline_parent) in [
            (true, false, false),
            (true, true, false),
            (false, false, true),
        ] {
            let (mut doc, cascade, parent, root) =
                context_paragraph(css, nested, contents, inline_parent);
            lay_out(&mut doc, &cascade);
            if inline_parent {
                assert!(doc.nodes[parent].is_ifc_root(), "{css}");
            }
            let width = if css.contains("margin-left") {
                65.0
            } else {
                70.0
            };
            assert_eq!(doc.nodes[root].layout.size.width, width, "{css}");
            assert_eq!(stored_lines(&doc, root).lines.len(), 3, "{css}");
            let geometry = doc.nodes[root].layout;
            relayout_text_for_width(&mut doc, &cascade, 800.0);
            assert_eq!(stored_lines(&doc, root).lines.len(), 3, "{css}");
            assert_eq!(stored_lines(&doc, root).width, width, "{css}");
            assert_eq!(doc.nodes[root].layout, geometry, "{css}");
        }
    }
}

#[test]
fn relayout_caps_the_content_box_after_padding_and_borders() {
    let css = "display:flow-root;padding-left:3px;padding-right:4px;\
               border-left:1px solid;border-right:2px solid";
    let (mut doc, cascade, _, root) = ahem_paragraph_beside_floats(
        "aaaa bbbb cccc",
        &["float:left;width:30px;height:60px"],
        css,
    );
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.nodes[root].layout.size.width, 70.0);
    let geometry = doc.nodes[root].layout;
    relayout_text_for_width(&mut doc, &cascade, 800.0);
    assert_eq!(stored_lines(&doc, root).width, 60.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 3);
    assert_eq!(doc.nodes[root].layout, geometry);
}

#[test]
fn relayout_does_not_treat_clip_or_boxless_overflow_as_a_context_root() {
    for css in ["overflow:clip", "display:contents;overflow:hidden"] {
        let (mut doc, cascade, _wrapper, root) =
            ahem_paragraph_in_block_wrapper("aaaa bbbb cccc", css);
        lay_out(&mut doc, &cascade);
        let geometry = doc.nodes[root].layout;
        for (width, count) in [(50.0, 3), (1000.0, 1)] {
            relayout_text_for_width(&mut doc, &cascade, width);
            assert_eq!(stored_lines(&doc, root).lines.len(), count, "{css}");
            assert_eq!(stored_lines(&doc, root).width, width, "{css}");
            assert_eq!(doc.nodes[root].layout, geometry, "{css}");
        }
    }
}

#[test]
fn relayout_does_not_cap_a_plain_root_by_viewport_propagated_body_overflow() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;overflow:hidden;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let root = doc.append_element(Some(body), "div", Some("display:block"));
    doc.append_text(root, "aaaa bbbb cccc");
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    lay_out(&mut doc, &cascade);
    let geometry = doc.nodes[root].layout;
    for (width, count) in [(50.0, 3), (1000.0, 1)] {
        relayout_text_for_width(&mut doc, &cascade, width);
        assert_eq!(stored_lines(&doc, root).lines.len(), count);
        assert_eq!(stored_lines(&doc, root).width, width);
        assert_eq!(doc.nodes[root].layout, geometry);
    }
}

#[test]
fn relayout_respects_a_recorded_scrollbar_gutter() {
    let (mut doc, cascade, _, root) = ahem_paragraph_beside_floats(
        "aaaa bbbb cccc",
        &["float:left;width:30px;height:60px"],
        "display:flow-root",
    );
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.nodes[root].layout.size.width, 70.0);
    // Rebreaking reads the recorded geometry; emulate a 10px vertical gutter
    // without depending on a platform's scrollbar policy during layout.
    doc.nodes[root].layout.scrollbar_size.width = 10.0;
    let geometry = doc.nodes[root].layout;
    relayout_text_for_width(&mut doc, &cascade, 800.0);
    assert_eq!(stored_lines(&doc, root).width, 60.0);
    assert_eq!(doc.nodes[root].layout, geometry);
}

#[test]
fn relayout_keeps_intrinsic_context_lines_unchanged() {
    for width in ["min-content", "max-content", "fit-content"] {
        let (mut doc, cascade, root) = ahem_paragraph(
            "aaaa bbbb cccc",
            &format!("display:flow-root;width:{width}"),
        );
        lay_out(&mut doc, &cascade);
        let before = stored_lines(&doc, root).lines.clone();
        let geometry = doc.nodes[root].layout;
        for max_advance in [50.0, 1000.0] {
            relayout_text_for_width(&mut doc, &cascade, max_advance);
            assert!(std::sync::Arc::ptr_eq(
                &before,
                &stored_lines(&doc, root).lines
            ));
            assert_eq!(doc.nodes[root].layout, geometry);
        }
    }
}

#[test]
fn relayout_distinguishes_viewport_overflow_from_the_bodys_own_context() {
    for (html_overflow, body_overflow, wide) in [
        ("visible", "hidden", 1000.0),
        ("hidden", "visible", 1000.0),
        ("hidden", "hidden", 800.0),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(
            Some(0),
            "html",
            Some(&format!("display:block;overflow:{html_overflow}")),
        );
        let body = doc.append_element(
            Some(html),
            "body",
            Some(&format!(
                "display:block;margin:0;overflow:{body_overflow};\
                 font-family:Ahem;font-size:10px;line-height:10px"
            )),
        );
        let root = doc.append_element(Some(body), "div", Some("display:block"));
        doc.append_text(root, "aaaa bbbb cccc");
        doc.mark_in_document_flags();
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        lay_out(&mut doc, &cascade);
        let geometry = doc.nodes[root].layout;
        for (requested, expected) in [(50.0, 50.0), (1000.0, wide)] {
            relayout_text_for_width(&mut doc, &cascade, requested);
            assert_eq!(
                stored_lines(&doc, root).width,
                expected,
                "html:{html_overflow}/body:{body_overflow}"
            );
            assert_eq!(doc.nodes[root].layout, geometry);
        }
    }
}
