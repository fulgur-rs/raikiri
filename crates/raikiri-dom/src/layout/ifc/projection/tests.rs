use super::*;
use crate::layout::ifc::test_support::{Fixture, ahem_fonts, block_fixture, span};
use shodo::AtomicSizes;

fn project(fixture: &Fixture) -> Result<ProjectedIfc, IfcError> {
    let mut cx = LayoutContext::new();
    project_ifc(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &mut cx,
        &ahem_fonts(),
        &Limits::default(),
    )
}

/// Trimmed text of every line when broken at `width`.
fn line_texts(projected: &ProjectedIfc, width: f32) -> Vec<String> {
    let mut cx = LayoutContext::new();
    projected
        .paragraph
        .break_all(&mut cx, &projected.options, width, &AtomicSizes::EMPTY)
        .iter()
        .map(|line| {
            line.text()[line.text_range()]
                .replace('\u{FFFC}', "")
                .trim_end()
                .to_owned()
        })
        .collect()
}

#[test]
fn text_breaks_at_the_ahem_advance() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aaaa bbbb cccc");
    });
    let projected = project(&fixture).expect("project");
    // Ahem at 10px: every character is exactly 10px wide.
    assert_eq!(line_texts(&projected, 50.0), ["aaaa", "bbbb", "cccc"]);
    assert_eq!(line_texts(&projected, 100.0), ["aaaa bbbb", "cccc"]);
    assert_eq!(line_texts(&projected, 500.0), ["aaaa bbbb cccc"]);
}

#[test]
fn inline_children_keep_source_order() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline");
        doc.append_text(inner, "bb");
        doc.append_text(root, " cc");
    });
    let projected = project(&fixture).expect("project");
    assert_eq!(line_texts(&projected, 500.0), ["aa bb cc"]);
}

#[test]
fn a_br_forces_a_break() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        doc.append_element(
            Some(root),
            "br",
            taffy::Style::default(),
            Some("display:inline"),
        );
        doc.append_text(root, "bb");
    });
    let projected = project(&fixture).expect("project");
    assert_eq!(line_texts(&projected, 500.0), ["aa", "bb"]);
}

#[test]
fn display_none_children_are_skipped() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        let hidden = span(doc, root, "display:none");
        doc.append_text(hidden, "zz");
        doc.append_text(root, "bb");
    });
    let projected = project(&fixture).expect("project");
    assert_eq!(line_texts(&projected, 500.0), ["aabb"]);
}

#[test]
fn constructs_the_inline_path_cannot_place_yet_are_rejected() {
    let cases: [(&str, &str); 4] = [
        ("inline-block", "display:inline-block"),
        ("block child", "display:block"),
        ("absolute", "display:inline;position:absolute"),
        ("flex child", "display:flex"),
    ];
    for (name, style) in cases {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa");
            let child = span(doc, root, style);
            doc.append_text(child, "bb");
        });
        let error = project(&fixture).expect_err(name);
        assert!(
            matches!(error, IfcError::Unsupported { .. }),
            "{name}: {error}"
        );
    }
}

#[test]
fn a_replaced_element_is_rejected() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        doc.append_element(
            Some(root),
            "img",
            taffy::Style::default(),
            Some("display:inline"),
        );
    });
    let error = project(&fixture).expect_err("img");
    assert!(matches!(error, IfcError::Unsupported { .. }), "{error}");
}

#[test]
fn a_root_that_is_not_a_block_is_rejected() {
    let fixture = block_fixture("display:inline", |doc, root| {
        doc.append_text(root, "aa");
    });
    let error = project(&fixture).expect_err("inline root");
    assert!(matches!(error, IfcError::Unsupported { .. }), "{error}");
}

#[test]
fn an_invalid_root_is_reported() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
    });
    let mut cx = LayoutContext::new();
    let error = project_ifc(
        &fixture.doc,
        &fixture.cascade,
        usize::MAX,
        &mut cx,
        &ahem_fonts(),
        &Limits::default(),
    )
    .expect_err("invalid");
    assert!(matches!(error, IfcError::InvalidNode(_)), "{error}");
}

#[test]
fn generated_content_is_rejected_instead_of_ignored() {
    use raikiri_style::{build_rule_tree, cascade};
    use taffy::Style;
    let mut doc = crate::Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(style, "div::before { content: \"x\" }");
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let root = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px"),
    );
    doc.append_text(root, "aa");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    let mut cx = LayoutContext::new();
    let error = project_ifc(
        &doc,
        &cascade,
        root,
        &mut cx,
        &ahem_fonts(),
        &Limits::default(),
    )
    .expect_err("generated content");
    assert!(matches!(error, IfcError::Unsupported { .. }), "{error}");
}

#[test]
fn text_align_is_inherited_from_ancestors() {
    // `text-align:center` on <body>: the block inherits it.
    let mut doc = crate::Document::new();
    let html = doc.append_element(Some(0), "html", taffy::Style::default(), None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        taffy::Style::default(),
        Some("text-align:center"),
    );
    let root = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px"),
    );
    doc.append_text(root, "aa");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    let mut cx = LayoutContext::new();
    let projected = project_ifc(
        &doc,
        &cascade,
        root,
        &mut cx,
        &ahem_fonts(),
        &Limits::default(),
    )
    .expect("project");
    assert_eq!(
        projected.options.text_align,
        shodo::style::TextAlign::Center
    );
}

#[test]
fn language_is_inherited_normalized_and_stopped_by_an_empty_value() {
    let mut doc = crate::Document::new();
    let html = doc.append_element(Some(0), "html", taffy::Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", taffy::Style::default(), None::<&str>);
    doc.set_element_attribute(html, "lang", " JA-jp ")
        .expect("set language");
    let inherit = doc.append_element(Some(body), "div", taffy::Style::default(), None::<&str>);
    let cleared = doc.append_element(Some(body), "div", taffy::Style::default(), None::<&str>);
    doc.set_element_attribute(cleared, "lang", "")
        .expect("clear language");
    let below = doc.append_element(Some(cleared), "span", taffy::Style::default(), None::<&str>);
    assert_eq!(language_of(&doc, inherit).as_deref(), Some("ja-jp"));
    assert_eq!(language_of(&doc, cleared), None);
    assert_eq!(language_of(&doc, below), None);
}

#[test]
fn language_is_unknown_without_any_lang_attribute() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
    });
    assert_eq!(language_of(&fixture.doc, fixture.root), None);
}

#[test]
fn empty_and_whitespace_only_blocks_produce_no_visible_lines() {
    for text in ["", " ", "   \n  "] {
        let fixture = block_fixture("", |doc, root| {
            if !text.is_empty() {
                doc.append_text(root, text);
            }
        });
        let projected = project(&fixture).expect("project");
        assert!(
            line_texts(&projected, 100.0)
                .iter()
                .all(|line| line.trim().is_empty()),
            "{text:?}"
        );
    }
}

#[test]
fn word_wider_than_the_line_still_terminates() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "abcdefghij");
    });
    let projected = project(&fixture).expect("project");
    let lines = line_texts(&projected, 5.0);
    assert!(!lines.is_empty());
    assert_eq!(lines.concat(), "abcdefghij");
}

#[test]
fn rtl_and_astral_text_do_not_fail() {
    let fixture = block_fixture("direction:rtl", |doc, root| {
        doc.append_text(root, "عربي 😀 mix");
    });
    let projected = project(&fixture).expect("project");
    let lines = line_texts(&projected, 100.0);
    assert!(!lines.is_empty());
}

#[test]
fn non_rendered_html_elements_contribute_no_text() {
    // `display:inline` is forced so only the non-rendered-element check can
    // keep the source text of `<style>` and `<script>` out of the paragraph.
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        for tag in ["style", "script"] {
            let hidden = doc.append_element(
                Some(root),
                tag,
                taffy::Style::default(),
                Some("display:inline"),
            );
            doc.append_text(hidden, "zz");
        }
        doc.append_text(root, "bb");
    });
    let projected = project(&fixture).expect("project");
    assert_eq!(line_texts(&projected, 500.0), ["aabb"]);
}

#[test]
fn a_float_child_is_recorded_as_a_box_and_anchored() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let float = span(doc, root, "display:block;float:left;width:30px;height:20px");
        doc.append_text(float, "ff");
        doc.append_text(root, " bb");
    });
    let projected = project(&fixture).expect("project");
    assert_eq!(projected.boxes.len(), 1);
    assert_eq!(projected.boxes[0].kind, IfcBoxKind::Float);
    // The float's own text is not part of the paragraph. The float anchor is
    // transparent to white-space collapsing, so the two spaces around it
    // collapse into one (and `line_texts` drops the U+FFFC placeholder).
    assert_eq!(line_texts(&projected, 500.0), ["aa bb"]);
}

#[test]
fn unsupported_floats_stay_unsupported() {
    for css in [
        "display:block;float:inline-start",
        "display:block;float:left;clear:inline-start",
        "display:block;float:left;position:relative",
    ] {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa ");
            let float = span(doc, root, css);
            doc.append_text(float, "ff");
        });
        let error = project(&fixture).expect_err(css);
        assert!(
            matches!(error, IfcError::Unsupported { .. }),
            "{css}: {error}"
        );
    }
}
