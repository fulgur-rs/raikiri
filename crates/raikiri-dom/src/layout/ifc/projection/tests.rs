use super::*;
use crate::layout::ifc::test_support::{Fixture, ahem_fonts, block_fixture, span};
use shodo::AtomicSizes;

#[test]
fn marker_eligibility_preserves_counter_snapshot_errors() {
    let fixture = crate::layout::ifc::test_support::sheet_fixture(
        "div::marker {content:counters(section, '.')}",
        "display:list-item;list-style:inside none",
        |_, _| {},
    );
    let counters = GeneratedCounters::default();
    counters
        .counters
        .set(Err(CounterSnapshotLimitExceeded {
            limit: 32,
            actual: 33,
        }))
        .unwrap();
    assert!(matches!(
        has_in_flow_generated_text(&fixture.doc, &fixture.cascade, fixture.root, &counters),
        Err(IfcError::CounterSnapshots(CounterSnapshotLimitExceeded {
            limit: 32,
            actual: 33
        }))
    ));
}

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
                .replace(['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'], "")
                .trim_end()
                .to_owned()
        })
        .collect()
}

#[test]
fn projected_paragraph_shapes_with_its_computed_writing_mode() {
    for (css, expected) in [
        (
            "writing-mode:vertical-rl",
            shodo::geometry::WritingMode::VerticalRl,
        ),
        (
            "writing-mode:vertical-lr",
            shodo::geometry::WritingMode::VerticalLr,
        ),
    ] {
        let fixture = block_fixture(css, |doc, root| {
            doc.append_text(root, "x");
        });
        let projected = project(&fixture).expect("project");
        assert_eq!(projected.writing_mode, expected, "{css}");
        let line = projected
            .paragraph
            .break_all(
                &mut LayoutContext::new(),
                &projected.options,
                100.0,
                &AtomicSizes::EMPTY,
            )
            .into_iter()
            .next()
            .expect("line");
        assert_eq!(line.writing_mode(), expected, "{css}");
    }
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
fn wbr_is_transformed_when_its_computed_style_enables_word_space_transform() {
    let fixture = block_fixture("word-space-transform:space", |doc, root| {
        doc.append_text(root, "a");
        doc.append_element(
            Some(root),
            "wbr",
            taffy::Style::default(),
            Some("display:inline"),
        );
        doc.append_text(root, "b");
    });
    let projected = project(&fixture).expect("project");
    assert_eq!(projected.paragraph.text(), "a b");
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
fn an_inline_block_is_recorded_as_an_atomic_box() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let atomic = span(doc, root, "display:inline-block;width:30px;height:10px");
        doc.append_text(atomic, "ii");
        doc.append_text(root, " bb");
    });
    let projected = project(&fixture).expect("project");
    assert_eq!(projected.boxes.len(), 1);
    assert_eq!(projected.boxes[0].kind, IfcBoxKind::Atomic);
    // The atomic's own text is not part of the paragraph.
    assert!(
        line_texts(&projected, 500.0)
            .iter()
            .all(|t| !t.contains("ii"))
    );
}

#[test]
fn an_img_and_an_inline_svg_are_atomic() {
    for tag in ["img", "svg"] {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa ");
            doc.append_element(
                Some(root),
                tag,
                taffy::Style::default(),
                Some("display:inline;width:10px;height:10px"),
            );
        });
        let projected = project(&fixture).expect(tag);
        assert_eq!(projected.boxes.len(), 1, "{tag}");
        assert_eq!(projected.boxes[0].kind, IfcBoxKind::Atomic, "{tag}");
    }
}

#[test]
fn replaced_elements_and_form_controls_are_boxes_of_the_paragraph() {
    for (tag, css, kind) in [
        ("video", "display:inline", IfcBoxKind::Atomic),
        ("iframe", "display:inline", IfcBoxKind::Atomic),
        ("canvas", "display:inline", IfcBoxKind::Atomic),
        ("audio", "display:inline", IfcBoxKind::Atomic),
        ("object", "display:inline", IfcBoxKind::Atomic),
        ("embed", "display:inline", IfcBoxKind::Atomic),
        ("math", "display:inline", IfcBoxKind::Atomic),
        ("input", "display:inline", IfcBoxKind::Atomic),
        ("button", "display:inline", IfcBoxKind::Atomic),
        ("textarea", "display:inline", IfcBoxKind::Atomic),
        (
            "input",
            "display:inline-block;width:20px;height:10px",
            IfcBoxKind::Atomic,
        ),
        ("select", "display:inline-block", IfcBoxKind::Atomic),
        (
            "input",
            "display:block;float:left;width:20px;height:10px",
            IfcBoxKind::Float,
        ),
    ] {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa ");
            doc.append_element(Some(root), tag, taffy::Style::default(), Some(css));
        });
        let projected = project(&fixture).expect(css);
        assert_eq!(projected.boxes.len(), 1, "{tag} {css}");
        assert_eq!(projected.boxes[0].kind, kind, "{tag} {css}");
    }
}

#[test]
fn positioned_boxes_of_every_kind_are_out_of_flow_boxes() {
    // An absolutely positioned or fixed box takes no room on the lines,
    // whatever its display and float (CSS 2.1 9.7).
    for (tag, css) in [
        (
            "span",
            "display:inline-block;vertical-align:middle;position:absolute",
        ),
        ("img", "display:inline;position:fixed"),
        ("span", "display:inline;position:absolute"),
        ("span", "display:block;position:absolute"),
        ("span", "display:block;float:left;position:absolute"),
    ] {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa ");
            doc.append_element(Some(root), tag, taffy::Style::default(), Some(css));
        });
        let projected = project(&fixture).expect(css);
        assert_eq!(projected.boxes.len(), 1, "{tag} {css}");
        assert_eq!(
            projected.boxes[0].kind,
            IfcBoxKind::OutOfFlow,
            "{tag} {css}"
        );
        assert_eq!(line_texts(&projected, 500.0), ["aa"], "{tag} {css}");
    }
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
fn generated_text_of_the_root_is_projected_before_its_content() {
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
    let projected = project_ifc(
        &doc,
        &cascade,
        root,
        &mut cx,
        &ahem_fonts(),
        &Limits::default(),
    )
    .expect("generated content");
    assert_eq!(projected.paragraph.text(), "xaa");
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
fn a_block_child_is_recorded_as_a_block_box() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        let block = span(doc, root, "display:block;height:20px");
        doc.append_text(block, "bb");
        doc.append_text(root, "cc");
    });
    let projected = project(&fixture).expect("project");
    assert_eq!(projected.boxes.len(), 1);
    assert_eq!(projected.boxes[0].kind, IfcBoxKind::Block);
    // The block's text is not part of the paragraph; "aa" and "cc" are on
    // separate lines on either side of it.
    assert_eq!(line_texts(&projected, 500.0), ["aa", "cc"]);
}

#[test]
fn a_block_child_with_vertical_margins_is_a_box_of_the_paragraph() {
    for css in [
        "display:block;margin-top:5px",
        "display:block;margin-bottom:-5px",
        "display:block;margin-top:10%",
    ] {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa");
            let block = span(doc, root, css);
            doc.append_text(block, "bb");
        });
        let projected = project(&fixture).expect(css);
        assert_eq!(projected.boxes.len(), 1, "{css}");
        assert_eq!(projected.boxes[0].kind, IfcBoxKind::Block, "{css}");
    }
}

#[test]
fn a_block_level_image_or_svg_is_a_block_child() {
    for tag in ["img", "svg"] {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa ");
            doc.append_element(
                Some(root),
                tag,
                taffy::Style::default(),
                Some("display:block;width:10px;height:10px"),
            );
        });
        let projected = project(&fixture).expect(tag);
        assert_eq!(projected.boxes.len(), 1, "{tag}");
        assert_eq!(projected.boxes[0].kind, IfcBoxKind::Block, "{tag}");
    }
}

#[test]
fn every_block_level_box_is_a_block_child() {
    for css in [
        "display:flow-root",
        "display:flex",
        "display:grid",
        "display:table",
        "display:list-item",
        "display:block;overflow:hidden",
        "display:block;clear:left",
        // Logical float sides are not mapped: the box is not floated.
        "display:block;float:inline-start",
        "display:block;float:inline-end",
    ] {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa");
            let block = span(doc, root, css);
            doc.append_text(block, "bb");
        });
        let projected = project(&fixture).expect(css);
        assert_eq!(projected.boxes.len(), 1, "{css}");
        assert_eq!(projected.boxes[0].kind, IfcBoxKind::Block, "{css}");
    }
}

#[test]
fn a_cleared_line_break_is_recorded_with_its_physical_side() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        for clear in ["left", "inline-start", "both"] {
            doc.append_element(
                Some(root),
                "br",
                taffy::Style::default(),
                Some(format!("display:inline;clear:{clear}").as_str()),
            );
        }
    });
    let projected = project(&fixture).expect("project");
    let sides: Vec<taffy::Clear> = projected
        .cleared_breaks
        .iter()
        .map(|(_, clear)| *clear)
        .collect();
    assert_eq!(sides, [taffy::Clear::Left, taffy::Clear::Both]);
}

#[test]
fn a_relative_block_child_with_no_offset_is_accepted() {
    // `position: relative` without an offset moves nothing.
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        let b = span(doc, root, "display:block;position:relative;left:0");
        doc.append_text(b, "bb");
    });
    let projected = project(&fixture).expect("a relative block with no offset");
    assert_eq!(projected.boxes.len(), 1);
    assert_eq!(projected.boxes[0].kind, IfcBoxKind::Block);
}

#[test]
fn a_relative_block_child_with_an_offset_or_a_stacking_context_is_a_block_child() {
    // It is laid out in place and moved by its offsets.
    for css in [
        "display:block;position:relative;top:3px",
        "display:block;position:relative;right:1px",
        "display:block;position:relative;bottom:-2px",
        "display:block;position:relative;left:calc(10% + 1px)",
        "display:block;position:relative;z-index:2",
        "display:block;position:sticky;top:2px",
    ] {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa");
            let b = span(doc, root, css);
            doc.append_text(b, "bb");
        });
        let projected = project(&fixture).expect(css);
        assert_eq!(projected.boxes[0].kind, IfcBoxKind::Block, "{css}");
    }
}

#[test]
fn a_float_inside_an_inline_element_is_a_box_of_the_paragraph() {
    // A box inside an inline element is laid out relative to the root.
    let fixture = block_fixture("", |doc, root| {
        let inner = span(doc, root, "display:inline");
        doc.append_text(inner, "a");
        let float = span(doc, inner, "float:left;width:10px;height:10px");
        doc.append_text(float, "x");
    });
    let projected = project(&fixture).expect("a float inside an inline");
    assert_eq!(projected.boxes.len(), 1);
    assert_eq!(projected.boxes[0].kind, IfcBoxKind::Float);
}

#[test]
fn an_atomic_inside_an_inline_element_is_a_box_of_the_paragraph() {
    let fixture = block_fixture("", |doc, root| {
        let inner = span(doc, root, "display:inline");
        doc.append_text(inner, "a");
        span(doc, inner, "display:inline-block;width:10px;height:10px");
    });
    let projected = project(&fixture).expect("an atomic inside an inline");
    assert_eq!(projected.boxes.len(), 1);
    assert_eq!(projected.boxes[0].kind, IfcBoxKind::Atomic);
}

#[test]
fn a_float_and_an_atomic_directly_under_the_root_are_still_projected() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "a");
        span(doc, root, "float:left;width:10px;height:10px");
        span(doc, root, "display:inline-block;width:10px;height:10px");
    });
    let projected = project(&fixture).expect("boxes directly under the root");
    assert_eq!(projected.boxes.len(), 2);
}

#[test]
fn generated_text_of_a_contents_element_is_projected_in_place() {
    // A `display: contents` element has no box, but its pseudo-elements do:
    // they sit before and after its children (CSS Display 3, 2.5).
    use raikiri_style::{build_rule_tree, cascade};
    use taffy::Style;
    let mut doc = crate::Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(style, "span::before { content: \"x\" }");
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let root = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px"),
    );
    doc.append_text(root, "aa");
    let wrapper = doc.append_element(
        Some(root),
        "span",
        Style::default(),
        Some("display:contents"),
    );
    doc.append_text(wrapper, "bb");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    let projected = project_ifc(
        &doc,
        &cascade,
        root,
        &mut LayoutContext::new(),
        &ahem_fonts(),
        &Limits::default(),
    )
    .expect("generated content on a contents element");
    assert_eq!(projected.paragraph.text(), "aaxbb");
}

#[test]
fn a_contents_element_that_is_floated_or_positioned_is_still_transparent() {
    // `float` and `position` apply to a box; a contents element has none, so
    // its children are part of the paragraph as for any contents element.
    for css in [
        "display:contents;float:left",
        "display:contents;position:absolute",
    ] {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa");
            let wrapper = span(doc, root, css);
            doc.append_text(wrapper, "bb");
        });
        let projected = project(&fixture).expect(css);
        assert!(projected.boxes.is_empty(), "{css}");
        assert_eq!(line_texts(&projected, 500.0), ["aabb"], "{css}");
    }
}

#[test]
fn boxes_inside_inline_and_contents_elements_are_boxes_of_the_paragraph() {
    // Wherever a box sits below the root's inline content, it is laid out
    // relative to the root: under an inline element, a contents element, or
    // both.
    for css in [
        "display:inline-block;width:10px;height:10px",
        "float:left;width:10px;height:10px",
        "display:block",
    ] {
        for wrappers in [
            ["display:contents", ""],
            ["display:inline", "display:contents"],
        ] {
            let fixture = block_fixture("", |doc, root| {
                doc.append_text(root, "a");
                let mut parent = root;
                for wrapper in wrappers.iter().filter(|w| !w.is_empty()) {
                    parent = span(doc, parent, wrapper);
                }
                span(doc, parent, css);
            });
            let projected = project(&fixture).expect(css);
            assert_eq!(projected.boxes.len(), 1, "{css} in {wrappers:?}");
        }
    }
}

#[test]
fn a_contents_child_contributes_its_text() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        let wrapper = span(doc, root, "display:contents");
        doc.append_text(wrapper, "bb");
        doc.append_text(root, "cc");
    });
    let projected = project(&fixture).expect("project");
    assert_eq!(line_texts(&projected, 500.0), ["aabbcc"]);
}

fn project_in_two_steps(fixture: &Fixture) -> Result<ProjectedIfc, IfcError> {
    let fonts = ahem_fonts();
    let limits = Limits::default();
    let projected = project_ifc_builder(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &fonts,
        &limits,
    )?;
    let mut cx = LayoutContext::new();
    projected.build(&mut cx, &fonts)
}

#[test]
fn building_in_two_steps_gives_the_same_paragraph() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aaaa ");
        let inner = span(doc, root, "display:inline");
        doc.append_text(inner, "bbbb cccc");
    });
    let whole = project(&fixture).expect("project");
    let split = project_in_two_steps(&fixture).expect("project in two steps");
    assert_eq!(line_texts(&split, 50.0), line_texts(&whole, 50.0));
    assert_eq!(line_texts(&split, 500.0), line_texts(&whole, 500.0));
    assert_eq!(split.rtl, whole.rtl);
    assert_eq!(split.boxes.len(), whole.boxes.len());
}

#[test]
fn a_builder_can_cross_threads() {
    // A builder is plain data: it may be built on another thread while the
    // document stays where it is.
    fn assert_send<T: Send>() {}
    assert_send::<ProjectedBuilder>();
    assert_send::<shodo::ParagraphBuilder>();
}

#[test]
fn a_builder_error_is_reported_before_any_shaping() {
    // An unsupported shape is refused while the builder is made, so a caller
    // never pays for shaping a paragraph it will not use.
    let fixture = block_fixture("display:inline", |doc, root| {
        doc.append_text(root, "aa");
    });
    let fonts = ahem_fonts();
    let error = project_ifc_builder(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &fonts,
        &Limits::default(),
    )
    .err()
    .expect("refused");
    assert!(matches!(error, IfcError::Unsupported { .. }), "{error}");
}

#[test]
fn a_text_node_projects_as_a_paragraph_of_its_own() {
    let fixture = block_fixture("display:flex", |doc, root| {
        doc.append_text(root, "aaaa bbbb cccc");
    });
    let text = fixture.doc.nodes[fixture.root].children[0];
    let mut cx = LayoutContext::new();
    let projected = project_ifc_text(
        &fixture.doc,
        &fixture.cascade,
        text,
        &mut cx,
        &ahem_fonts(),
        &Limits::default(),
    )
    .expect("project");
    assert!(projected.boxes.is_empty());
    assert_eq!(line_texts(&projected, 50.0), ["aaaa", "bbbb", "cccc"]);
    // An element is not a text node.
    let mut cx = LayoutContext::new();
    assert!(matches!(
        project_ifc_text(
            &fixture.doc,
            &fixture.cascade,
            fixture.root,
            &mut cx,
            &ahem_fonts(),
            &Limits::default(),
        ),
        Err(IfcError::InvalidNode(_))
    ));
}

#[test]
fn an_inline_table_is_an_atomic_and_table_internal_boxes_are_blocks() {
    for (css, kind) in [
        ("display:inline-table", Some(IfcBoxKind::Atomic)),
        ("display:table-row", Some(IfcBoxKind::Block)),
        ("display:table-cell", Some(IfcBoxKind::Block)),
        ("display:table-caption", Some(IfcBoxKind::Block)),
        ("display:table-row-group", Some(IfcBoxKind::Block)),
        ("display:table-column", None),
    ] {
        let fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa");
            let child = span(doc, root, css);
            doc.append_text(child, "bb");
        });
        let projected = project(&fixture).expect(css);
        assert_eq!(projected.boxes.first().map(|b| b.kind), kind, "{css}");
    }
}

#[test]
fn generated_first_letter_retains_the_originating_elements_language() {
    let fixture = crate::layout::ifc::test_support::sheet_fixture(
        "div::before {content:'ix'} div::first-letter {text-transform:uppercase}",
        "",
        |doc, root| {
            doc.set_element_attribute(root, "lang", "tr").unwrap();
        },
    );
    let projected = project(&fixture).unwrap();
    let lines = projected.paragraph.break_all(
        &mut LayoutContext::new(),
        &projected.options,
        100.0,
        &AtomicSizes::EMPTY,
    );
    assert_eq!(lines[0].text(), "İx");
    assert_eq!(projected.paragraph.text(), "ix");
}

#[test]
fn first_letter_adjacent_source_scan_enforces_text_and_item_limits() {
    for (text_limit, item_limit, kind) in [
        (Some(3), None, shodo::limits::LimitKind::TextBytes),
        (None, Some(1), shodo::limits::LimitKind::Items),
    ] {
        let fixture = crate::layout::ifc::test_support::sheet_fixture(
            "div::first-letter{font-size:20px}",
            "",
            |doc, root| {
                doc.append_text(root, "\"");
                doc.append_comment(Some(root), "gap");
                doc.append_text(root, " ");
                doc.append_text(root, "XY");
            },
        );
        let limits = Limits {
            max_text_bytes: text_limit,
            max_items: item_limit,
            ..Limits::default()
        };
        let error = match project_ifc_builder(
            &fixture.doc,
            &fixture.cascade,
            fixture.root,
            &ahem_fonts(),
            &limits,
        ) {
            Err(error) => error,
            Ok(_) => panic!("source scan must enforce its budget"),
        };
        assert!(matches!(error,IfcError::Limit(limit) if limit.kind==kind));
    }
}

#[test]
fn inside_marker_is_first_inline_content_and_only_indents_the_first_line() {
    let fixture = block_fixture(
        "display:list-item;list-style-position:inside;list-style-type:'X '",
        |doc, root| {
            doc.append_text(root, "aaaa bbbb");
        },
    );
    let projected = project(&fixture).expect("project");
    assert_eq!(projected.paragraph.text(), "\u{2066}X \u{2069}aaaa bbbb");
    assert_eq!(line_texts(&projected, 70.0), ["X aaaa", "bbbb"]);
    assert_eq!(
        fixture.doc.nodes[fixture.root]
            .style
            .padding
            .left
            .into_raw()
            .value(),
        0.0
    );
}

#[test]
fn inside_marker_precedes_before_and_increases_empty_item_height() {
    let mut fixture = block_fixture(
        "display:list-item;list-style-position:inside;list-style-type:'X'",
        |doc, root| {
            doc.append_text(root, "b");
        },
    );
    let head = fixture
        .doc
        .append_element(Some(0), "style", taffy::Style::default(), None::<&str>);
    fixture.doc.append_text(
        head,
        r#"div::before { content: "a" } div::marker { color:red;font-size:20px }"#,
    );
    fixture.doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&fixture.doc);
    fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
    let projected = project(&fixture).expect("project");
    assert_eq!(projected.paragraph.text(), "\u{2066}X\u{2069}ab");
    let empty = block_fixture(
        "display:list-item;list-style-position:inside;list-style-type:'X'",
        |_, _| {},
    );
    let projected_empty = project(&empty).expect("project");
    assert_eq!(line_texts(&projected_empty, 100.0), ["X"]);
    let lines = projected_empty.paragraph.break_all(
        &mut LayoutContext::new(),
        &projected_empty.options,
        100.0,
        &AtomicSizes::EMPTY,
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].block_size() >= 10.0);
}

#[test]
fn inside_marker_preserves_spaces_does_not_wrap_or_inherit_text_transform() {
    for author_marker in ["", "div::marker {color:red}"] {
        let mut fixture = block_fixture(
            "display:list-item;list-style:inside '  x  y  ';text-transform:uppercase",
            |doc, root| {
                doc.append_text(root, "ab");
            },
        );
        let sheet =
            fixture
                .doc
                .append_element(Some(0), "style", taffy::Style::default(), None::<&str>);
        fixture.doc.append_text(sheet, author_marker);
        fixture.doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&fixture.doc);
        fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
        let projected = project(&fixture).expect("project");
        assert_eq!(projected.paragraph.text(), "\u{2066}  x  y  \u{2069}AB");
        assert_eq!(line_texts(&projected, 30.0).first().unwrap(), "  x  y");
    }
}

#[test]
fn suppressed_inside_image_marker_consumes_no_inline_space() {
    struct Pixels;
    impl raikiri_traits::ImagePixelSource for Pixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 16,
                height: 8,
                rgba: [0, 128, 0, 255].repeat(16 * 8),
            }))
        }
    }
    let mut fixture = crate::layout::ifc::test_support::sheet_fixture(
        "div::marker {display:none}",
        "display:list-item;list-style:inside url(https://images.test/marker.png)",
        |doc, root| {
            doc.append_text(root, "a");
        },
    );
    fixture
        .doc
        .prepare_list_marker_images(&fixture.cascade, &Pixels, None);
    let projected = project(&fixture).unwrap();
    assert!(projected.marker_atomic.is_none());
    assert_eq!(projected.paragraph.text(), "a");
}

#[test]
fn image_marker_has_intrinsic_extents_and_uses_authored_style_in_vertical_and_rtl_layout() {
    struct Pixels;
    impl raikiri_traits::ImagePixelSource for Pixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: 16,
                height: 8,
                rgba: [0, 128, 0, 255].repeat(16 * 8),
            }))
        }
    }
    for (extra, expected) in [
        ("direction:rtl", (16.0, 8.0)),
        ("writing-mode:vertical-rl", (8.0, 16.0)),
    ] {
        let mut fixture = crate::layout::ifc::test_support::sheet_fixture(
            "div::marker {color:red}",
            &format!(
                "display:list-item;list-style:inside url(https://images.test/marker.png);{extra}"
            ),
            |doc, root| {
                doc.append_text(root, "a");
            },
        );
        fixture
            .doc
            .prepare_list_marker_images(&fixture.cascade, &Pixels, None);
        let projected = project(&fixture).expect("project marker");
        let (_, size) = projected.marker_atomic.unwrap();
        assert_eq!((size.inline_size, size.block_size), expected);
        fixture.doc.set_font_collection(ahem_fonts());
        crate::layout::layout_single_page(
            &mut fixture.doc,
            &fixture.cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let inputs =
            crate::layout::ifc::boxes::intrinsics_of_boxes(&mut fixture.doc, fixture.root, 100.0);
        let root = fixture.doc.nodes[fixture.root].ifc.as_ref().unwrap();
        let extents = root.paragraph.intrinsic_sizes(
            &mut LayoutContext::new(),
            &root.options,
            &inputs.engine,
        );
        assert!(extents.min_content >= expected.0);
        assert!(extents.max_content >= expected.0 + 10.0);
    }
}

#[test]
fn floated_first_letter_requires_a_drop_cap_box_instead_of_an_inline_approximation() {
    for side in ["left", "right", "inline-start", "inline-end"] {
        let fixture = crate::layout::ifc::test_support::sheet_fixture(
            &format!("div::first-letter{{font-size:40px;float:{side}}}"),
            "font:10px/10px Ahem;white-space:pre",
            |doc, root| {
                doc.append_text(root, "XX\nXX\nXX");
            },
        );
        let error = match project(&fixture) {
            Err(error) => error,
            Ok(_) => panic!("a floated letter must not be drawn as an ordinary inline"),
        };
        assert!(matches!(
            error,
            IfcError::Unsupported {
                reason: "floating ::first-letter requires drop-cap box layout",
                ..
            }
        ));
    }
}

#[test]
fn an_atomic_or_out_of_flow_letter_does_not_inherit_ancestor_pseudo_boxes() {
    for css in [
        "display:inline-block",
        "display:table-cell",
        "float:left",
        "position:absolute",
        "position:fixed",
    ] {
        let fixture = crate::layout::ifc::test_support::sheet_fixture(
            "body::first-letter{font-size:2em} div::first-letter{font-size:2em}",
            css,
            |doc, root| {
                doc.append_text(root, "XX");
            },
        );
        let projected = project(&fixture).unwrap();
        assert_eq!(projected.letter_styles.len(), 1, "{css}");
        assert_eq!(
            projected.letter_styles[0].computed.font_size.0, 20.0,
            "{css}"
        );
    }
}

#[test]
fn an_ancestors_generated_prefix_keeps_a_childs_letter_local() {
    let fixture = crate::layout::ifc::test_support::sheet_fixture(
        "body::before{content:'prefix'} body::first-letter{font-size:2em} div::first-letter{font-size:2em}",
        "",
        |doc, root| {
            doc.append_text(root, "XX");
        },
    );
    let projected = project(&fixture).unwrap();
    assert_eq!(projected.letter_styles.len(), 1);
    assert_eq!(projected.letter_styles[0].computed.font_size.0, 20.0);
}

#[test]
fn unsupported_marker_style_returns_an_explicit_projection_error() {
    let mut fixture = crate::layout::ifc::test_support::sheet_fixture(
        "div::marker {font-variation-settings:\"wdth\" 1}",
        "display:list-item;list-style:inside 'x'",
        |_, _| {},
    );
    let marker = fixture
        .cascade
        .pseudo
        .get_mut(&(
            raikiri_style::StyleNodeId::new(fixture.root as u64),
            PseudoElem::Marker,
        ))
        .unwrap();
    let raikiri_style::property::FontVariationSettings::Settings(values) =
        &mut marker.font_variation_settings
    else {
        panic!("parsed axis")
    };
    values[0].tag = "bad".into();
    assert!(matches!(
        project(&fixture),
        Err(IfcError::Unsupported { .. })
    ));
}

#[test]
fn inside_marker_does_not_consume_the_first_letter_of_body_or_before_content() {
    for (marker, before) in [
        ("", ""),
        ("div::marker{content:'M '}", ""),
        ("div::marker{content:'M '}", "div::before{content:'XY'}"),
        ("div::marker{display:inline-block;content:'M '}", ""),
        (
            "div::marker{display:inline-block;content:'M '}",
            "div::before{content:'XY'}",
        ),
        ("div::marker{display:inline-flex;content:'M '}", ""),
        ("div::marker{display:inline-grid;content:'M '}", ""),
        ("div::marker{display:inline-table;content:'M '}", ""),
    ] {
        let fixture = crate::layout::ifc::test_support::sheet_fixture(
            &format!("div::first-letter{{font-size:20px;color:red}} {marker} {before}"),
            "display:list-item;list-style:inside 'M ';font:10px/30px Ahem",
            |doc, root| {
                doc.append_text(root, "AB");
            },
        );
        let body_text = fixture.doc.get_node(fixture.root).unwrap().children[0];
        let projected = project(&fixture).unwrap();
        assert_eq!(projected.letter_styles.len(), 1);
        let letter = &projected.letter_styles[0];
        assert_eq!(letter.computed.font_size.0, 20.0);
        assert_eq!(
            letter.source_owner,
            if before.is_empty() {
                body_text
            } else {
                generated_node_id(fixture.root, PseudoElem::Before)
            }
        );
        assert_eq!(letter.source_range, Some(0..1));
        assert_ne!(
            letter.source_owner,
            generated_node_id(fixture.root, PseudoElem::Marker)
        );
        assert!(projected.paragraph.text().contains("M "));
        assert!(projected.paragraph.text().ends_with("AB"));
        let lines = projected.paragraph.break_all(
            &mut LayoutContext::new(),
            &projected.options,
            100.0,
            &AtomicSizes::EMPTY,
        );
        let sizes: Vec<_> = lines
            .iter()
            .flat_map(|line| line.fragments())
            .filter_map(|fragment| match fragment {
                shodo::Fragment::GlyphRun(run) => Some(run.font_size()),
                _ => None,
            })
            .collect();
        assert_eq!(sizes.iter().filter(|size| **size == 20.0).count(), 1);
        assert!(sizes.iter().filter(|size| **size == 10.0).count() >= 2);
    }
}

#[test]
fn typographic_punctuation_crosses_non_atomic_inline_boundaries() {
    for trailing in [false, true] {
        let fixture = crate::layout::ifc::test_support::sheet_fixture(
            "div::first-letter{font-size:20px;color:red}",
            "font:10px/30px Ahem",
            |doc, root| {
                if trailing {
                    doc.append_text(root, "A");
                    let span = span(doc, root, "display:inline");
                    doc.append_text(span, "”");
                } else {
                    let span = span(doc, root, "display:inline");
                    doc.append_text(span, "“");
                    doc.append_text(root, "A");
                }
                doc.append_text(root, "B");
            },
        );
        let projected = project(&fixture).unwrap();
        let lines = projected.paragraph.break_all(
            &mut LayoutContext::new(),
            &projected.options,
            100.0,
            &AtomicSizes::EMPTY,
        );
        let glyphs: Vec<_> = lines
            .iter()
            .flat_map(|line| line.fragments())
            .filter_map(|fragment| match fragment {
                shodo::Fragment::GlyphRun(run) => {
                    Some((run.node().unwrap().0 as usize, run.font_size()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            glyphs.iter().map(|(_, size)| *size).collect::<Vec<_>>(),
            [20.0, 20.0, 10.0]
        );
        assert_eq!(projected.letter_styles.len(), 2);
        for style in &projected.letter_styles {
            assert!(
                fixture
                    .doc
                    .get_node(style.source_owner)
                    .unwrap()
                    .text_content()
                    .is_some()
            );
        }
    }
}

#[test]
fn transparent_contents_ancestors_preserve_the_first_block_descendant_letter() {
    let fixture = crate::layout::ifc::test_support::sheet_fixture(
        "body::first-letter{font-size:20px}",
        "display:contents",
        |doc, root| {
            let inner = span(doc, root, "display:block;font:10px/30px Ahem");
            doc.append_text(inner, "AB");
        },
    );
    let inner = fixture.doc.get_node(fixture.root).unwrap().children[0];
    let projected = project_ifc(
        &fixture.doc,
        &fixture.cascade,
        inner,
        &mut LayoutContext::new(),
        &ahem_fonts(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(projected.letter_styles.len(), 1);
    assert_eq!(projected.letter_styles[0].computed.font_size.0, 20.0);
}

#[test]
fn blockified_inline_items_and_absolute_roots_admit_their_own_first_letter() {
    for container in ["display:flex", "display:grid", "display:block"] {
        let item_style = if container == "display:block" {
            "display:inline;position:absolute;font:10px/30px Ahem"
        } else {
            "display:inline;font:10px/30px Ahem"
        };
        let fixture = crate::layout::ifc::test_support::sheet_fixture(
            "span::first-letter{font-size:20px}",
            container,
            |doc, root| {
                let item = span(doc, root, item_style);
                doc.append_text(item, "AB");
            },
        );
        let item = fixture.doc.get_node(fixture.root).unwrap().children[0];
        assert_eq!(fixture.cascade.computed[item].display, DisplayValue::Inline);
        assert!(super::super::assign::can_be_ifc_root(
            &fixture.doc,
            &fixture.cascade,
            item
        ));
        let projected = project_ifc(
            &fixture.doc,
            &fixture.cascade,
            item,
            &mut LayoutContext::new(),
            &ahem_fonts(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(projected.letter_styles.len(), 1, "{container}");
        assert_eq!(projected.letter_styles[0].computed.font_size.0, 20.0);
    }
}

#[test]
fn generated_punctuation_lookahead_reuses_counters_and_keeps_each_original_owner() {
    for (before, expected) in [
        ("content:'('", "(3)Y"),
        (
            "content:open-quote no-open-quote close-quote;quotes:'(' ')' '[' ']'",
            "(]3)Y",
        ),
    ] {
        let fixture = crate::layout::ifc::test_support::sheet_fixture(
            &format!(
                "div{{counter-reset:n 2}} div::before{{{before}}} span::before{{counter-increment:n;content:counter(n)}} span::after{{content:')'}} div::first-letter{{font-size:20px}}"
            ),
            "",
            |doc, root| {
                span(doc, root, "display:inline");
                doc.append_text(root, "Y");
            },
        );
        let span = fixture.doc.get_node(fixture.root).unwrap().children[0];
        let counters = GeneratedCounters::default();
        let snapshots = counters.get(&fixture.doc, &fixture.cascade).unwrap();
        let pointer = snapshots.as_ptr();
        let mut cx = LayoutContext::new();
        let projected = project_ifc_builder_with(
            &fixture.doc,
            &fixture.cascade,
            fixture.root,
            &ahem_fonts(),
            &Limits::default(),
            &counters,
        )
        .unwrap()
        .build(&mut cx, &ahem_fonts())
        .unwrap();
        assert_eq!(projected.paragraph.text(), expected);
        assert_eq!(
            counters
                .get(&fixture.doc, &fixture.cascade)
                .unwrap()
                .as_ptr(),
            pointer
        );
        assert_eq!(
            projected
                .letter_styles
                .iter()
                .map(|style| style.source_owner)
                .collect::<Vec<_>>(),
            [
                generated_node_id(fixture.root, PseudoElem::Before),
                generated_node_id(span, PseudoElem::Before),
                generated_node_id(span, PseudoElem::After),
            ]
        );
        assert!(
            projected
                .letter_styles
                .iter()
                .all(|style| style.computed.font_size.0 == 20.0)
        );
    }
}

#[test]
fn contents_siblings_block_ancestor_selection_only_when_they_render_content() {
    for (prefix, expected) in [("", 1), ("prefix", 0)] {
        let fixture = crate::layout::ifc::test_support::sheet_fixture(
            "body::first-letter{font-size:20px}",
            "display:contents",
            |doc, root| {
                let contents = span(doc, root, "display:contents;float:left;position:absolute");
                doc.append_text(contents, prefix);
                let inner = span(doc, root, "display:block;font:10px/30px Ahem");
                doc.append_text(inner, "AB");
            },
        );
        let inner = fixture.doc.get_node(fixture.root).unwrap().children[1];
        let projected = project_ifc(
            &fixture.doc,
            &fixture.cascade,
            inner,
            &mut LayoutContext::new(),
            &ahem_fonts(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(projected.letter_styles.len(), expected, "{prefix}");
    }
}
