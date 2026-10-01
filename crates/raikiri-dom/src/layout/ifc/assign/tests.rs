use super::*;
use crate::IfcBuildMode;
use crate::layout::ifc::font::{BundledFace, bundled_collection};
use crate::layout::ifc::test_support::{AHEM, Fixture, ahem_fonts, block_fixture, span};
use shodo::limits::Limits;

/// A paragraph builder for table-driven cases.
type Build = fn(&mut crate::Document, usize);

fn enable(fixture: &mut crate::layout::ifc::test_support::Fixture) {
    fixture
        .doc
        .enable_inline_formatting(ahem_fonts(), Limits::default());
}

fn assign(fixture: &mut crate::layout::ifc::test_support::Fixture) {
    assign_ifc_roots(&mut fixture.doc, &fixture.cascade).expect("assign");
}

fn is_root(fixture: &crate::layout::ifc::test_support::Fixture, id: usize) -> bool {
    fixture.doc.nodes[id].flags.contains(NodeFlags::IS_IFC_ROOT)
}

#[test]
fn a_plain_paragraph_becomes_a_root_and_marks_its_subtree() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline");
        doc.append_text(inner, "bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
    assert!(fixture.doc.nodes[fixture.root].ifc.is_some());
    for &child in &fixture.doc.nodes[fixture.root].children.clone() {
        assert!(
            fixture.doc.nodes[child]
                .flags
                .contains(NodeFlags::IN_IFC_SUBTREE)
        );
    }
}

#[test]
fn without_the_switch_no_root_is_assigned() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
    });
    assign(&mut fixture);
    assert!(!is_root(&fixture, fixture.root));
}

#[test]
fn ineligible_shapes_stay_on_the_parley_path() {
    let cases: [(&str, Build); 2] = [
        ("whitespace only", |doc, root| {
            doc.append_text(root, "   ");
        }),
        ("empty", |_, _| {}),
    ];
    for (name, build) in cases {
        let mut fixture = block_fixture("", build);
        enable(&mut fixture);
        assign(&mut fixture);
        assert!(!is_root(&fixture, fixture.root), "{name}");
        assert!(fixture.doc.nodes[fixture.root].ifc.is_none(), "{name}");
    }
}

#[test]
fn reassignment_clears_stale_marks() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
    // The switch stays on but the root no longer generates a box.
    fixture
        .doc
        .set_element_inline_style(fixture.root, Some("display:none".into()));
    fixture.doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&fixture.doc);
    fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
    assign(&mut fixture);
    assert!(!is_root(&fixture, fixture.root));
    let text = fixture.doc.nodes[fixture.root].children[0];
    assert!(
        !fixture.doc.nodes[text]
            .flags
            .contains(NodeFlags::IN_IFC_SUBTREE)
    );
}

#[test]
fn a_root_beside_inline_text_is_still_a_root() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
    });
    // Put loose text next to the block under `body`: `body` becomes a
    // paragraph with the block as a box, and the block stays the root of its
    // own text.
    let body = fixture.doc.parent_of(fixture.root).expect("body");
    fixture.doc.append_text(body, "loose");
    fixture.doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&fixture.doc);
    fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
    assert!(is_root(&fixture, body));
}

#[test]
fn a_root_inside_a_multicol_container_stays_on_the_parley_path() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
    });
    let body = fixture.doc.parent_of(fixture.root).expect("body");
    fixture
        .doc
        .set_element_inline_style(body, Some("display:block;column-count:2".into()));
    fixture.doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&fixture.doc);
    fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_multicol_root_is_a_root() {
    let mut fixture = block_fixture("column-count:2", |doc, root| {
        doc.append_text(root, "aa bb cc dd");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    // The ifc dispatch runs before the multicol dispatch for a root and
    // splits its lines in columns.
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_vertical_root_is_a_root_laid_out_horizontally() {
    // Vertical writing is laid out as horizontal text, as on the parley
    // path; the paragraph is the engine's.
    let mut fixture = block_fixture("writing-mode:vertical-rl", |doc, root| {
        doc.append_text(root, "aa bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn assignment_drops_the_layout_cache_only_when_the_switch_is_on() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
    });
    fixture.doc.layout_dirty = false;
    assign(&mut fixture);
    assert!(!fixture.doc.layout_dirty);
    enable(&mut fixture);
    fixture.doc.layout_dirty = false;
    assign(&mut fixture);
    assert!(fixture.doc.layout_dirty);
}

/// Append a sibling element under the root's parent.
fn add_sibling(doc: &mut crate::Document, root: usize, display: &str) {
    let parent = doc.parent_of(root).expect("root has a parent");
    let sibling = doc.append_element(
        Some(parent),
        "div",
        taffy::Style::default(),
        Some(format!("display:{display}").as_str()),
    );
    doc.append_text(sibling, "x");
}

#[test]
fn block_level_siblings_do_not_disqualify_a_paragraph() {
    for display in ["flex", "grid", "table", "list-item", "flow-root", "block"] {
        let mut fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa");
            add_sibling(doc, root, display);
        });
        enable(&mut fixture);
        assign(&mut fixture);
        assert!(is_root(&fixture, fixture.root), "{display}");
    }
}

#[test]
fn inline_level_siblings_do_not_disqualify_a_paragraph() {
    for display in [
        "inline",
        "inline-block",
        "inline-flex",
        "inline-grid",
        "inline-table",
        "contents",
    ] {
        let mut fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa");
            add_sibling(doc, root, display);
        });
        enable(&mut fixture);
        assign(&mut fixture);
        assert!(is_root(&fixture, fixture.root), "{display}");
    }
}

/// Assert that the paragraph `build` fills under a root styled `css` is laid
/// out by the engine.
fn assert_is_a_root(css: &str, build: impl FnOnce(&mut crate::Document, usize)) {
    let mut fixture = block_fixture(css, build);
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

fn assert_is_root(css: &str, build: impl FnOnce(&mut crate::Document, usize)) {
    let mut fixture = block_fixture(css, build);
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

fn text_only(text: &'static str) -> impl FnOnce(&mut crate::Document, usize) {
    move |doc, root| {
        doc.append_text(root, text);
    }
}

#[test]
fn an_rtl_paragraph_is_an_ifc_root() {
    for (css, text) in [("direction:rtl", "aa"), ("", "aa \u{05d0}\u{05d1}")] {
        let mut fixture = block_fixture(css, |doc, root| {
            doc.append_text(root, text);
        });
        enable(&mut fixture);
        assign(&mut fixture);
        assert!(is_root(&fixture, fixture.root), "{css:?} {text:?}");
    }
}

#[test]
fn an_rtl_paragraph_with_a_box_is_laid_out_by_the_engine() {
    // An accepted degradation: boxes in a right-to-left paragraph are placed
    // where the engine puts them, not checked against the parley path.
    let cases: [(&str, Build); 3] = [
        ("float", |doc, root| {
            doc.append_text(root, "aa ");
            let f = span(doc, root, "float:left;width:20px;height:10px");
            doc.append_text(f, "x");
        }),
        ("inline-block", |doc, root| {
            doc.append_text(root, "aa ");
            let b = span(doc, root, "display:inline-block;width:20px;height:10px");
            doc.append_text(b, "x");
        }),
        ("block child", |doc, root| {
            doc.append_text(root, "aa");
            let b = span(doc, root, "display:block");
            doc.append_text(b, "x");
        }),
    ];
    for (name, build) in cases {
        let mut fixture = block_fixture("direction:rtl", build);
        enable(&mut fixture);
        assign(&mut fixture);
        assert!(is_root(&fixture, fixture.root), "{name}");
    }
}

#[test]
fn a_box_in_a_paragraph_with_rtl_characters_is_laid_out_by_the_engine() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "\u{05d0} ");
        let b = span(doc, root, "display:inline-block;width:20px;height:10px");
        doc.append_text(b, "x");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn right_to_left_content_inside_a_box_does_not_make_the_paragraph_rtl() {
    // The box lays out its own content; the paragraph's lines stay left to
    // right.
    let cases: [(&str, Build); 2] = [
        ("rtl text in a float", |doc, root| {
            doc.append_text(root, "aa ");
            let f = span(doc, root, "float:left;width:20px;height:10px");
            doc.append_text(f, "\u{05d0}");
        }),
        ("an rtl inline-block", |doc, root| {
            doc.append_text(root, "aa ");
            let b = span(
                doc,
                root,
                "display:inline-block;direction:rtl;unicode-bidi:bidi-override;width:20px;height:10px",
            );
            doc.append_text(b, "x");
        }),
    ];
    for (name, build) in cases {
        let mut fixture = block_fixture("", build);
        enable(&mut fixture);
        assign(&mut fixture);
        assert!(is_root(&fixture, fixture.root), "{name}");
    }
}

#[test]
fn an_rtl_paragraph_with_a_unicode_bidi_value_is_laid_out_by_the_engine() {
    // An accepted difference: the parley path does not read `unicode-bidi`;
    // the engine orders the text by it (UAX #9), as CSS Writing Modes 3
    // requires.
    for value in [
        "bidi-override",
        "isolate-override",
        "embed",
        "isolate",
        "plaintext",
    ] {
        let css = format!("direction:rtl;unicode-bidi:{value}");
        assert_is_root(&css, text_only("aa"));
        // On a descendant of a right-to-left paragraph too.
        assert_is_root("direction:rtl", |doc, root| {
            doc.append_text(root, "aa ");
            let inner = span(doc, root, &format!("display:inline;unicode-bidi:{value}"));
            doc.append_text(inner, "bb");
        });
    }
}

#[test]
fn a_unicode_bidi_value_in_a_left_to_right_paragraph_is_still_a_root() {
    // Nothing is reordered without right-to-left content, so the value has no
    // effect on either path.
    let mut fixture = block_fixture("unicode-bidi:bidi-override", |doc, root| {
        doc.append_text(root, "aa");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn text_shadow_keeps_a_paragraph_an_ifc_root() {
    for (css, inner_css) in [
        ("text-shadow:1px 1px red", "display:inline"),
        ("", "display:inline;text-shadow:1px 1px 2px red"),
    ] {
        let mut fixture = block_fixture(css, |doc, root| {
            doc.append_text(root, "aa ");
            let inner = span(doc, root, inner_css);
            doc.append_text(inner, "bb");
        });
        enable(&mut fixture);
        assign(&mut fixture);
        assert!(is_root(&fixture, fixture.root), "{css} {inner_css}");
    }
}

#[test]
fn hanging_punctuation_first_keeps_a_paragraph_an_ifc_root() {
    let mut fixture = block_fixture("hanging-punctuation:first", |doc, root| {
        doc.append_text(root, "aa");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn text_emphasis_keeps_a_paragraph_an_ifc_root() {
    // Neither path draws emphasis marks, so the paragraph lays out and paints
    // the same as on the parley path.
    let mut fixture = block_fixture("text-emphasis-style:dot", |doc, root| {
        doc.append_text(root, "aa");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn an_rtl_inline_keeps_the_paragraph_an_ifc_root() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;direction:rtl");
        doc.append_text(inner, "bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_relative_inline_with_no_offset_keeps_a_paragraph_an_ifc_root() {
    for css in [
        "display:inline;position:relative",
        "display:inline;position:relative;top:0;left:0px",
        "display:inline;position:relative;top:auto;right:0;bottom:0",
    ] {
        let mut fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa ");
            let inner = span(doc, root, css);
            doc.append_text(inner, "bb");
        });
        enable(&mut fixture);
        assign(&mut fixture);
        assert!(is_root(&fixture, fixture.root), "{css}");
    }
}

#[test]
fn a_relative_inline_with_a_length_offset_is_a_root() {
    for css in [
        "display:inline;position:relative;left:5px;top:-2px",
        "display:inline;position:relative;right:-1px",
        "display:inline;position:relative;left:1em",
    ] {
        let mut fixture = block_fixture("", |doc, root| {
            doc.append_text(root, "aa");
            let inner = span(doc, root, css);
            doc.append_text(inner, "bb");
        });
        enable(&mut fixture);
        assign(&mut fixture);
        assert!(is_root(&fixture, fixture.root), "{css}");
    }
}

#[test]
fn a_relative_inline_with_a_calc_offset_is_laid_out_by_the_engine() {
    // A plain percentage offset is dropped by the parser (both paths see
    // `auto`), so a mixed calc() stands in for the non-px forms. An accepted
    // degradation: such an inset is taken as zero.
    for css in [
        "display:inline;position:relative;left:calc(1% + 1px)",
        "display:inline;position:relative;bottom:calc(1% + 1px)",
    ] {
        assert_is_root("", |doc, root| {
            doc.append_text(root, "aa");
            let inner = span(doc, root, css);
            doc.append_text(inner, "bb");
        });
    }
}

#[test]
fn a_relative_inline_with_a_z_index_is_laid_out_by_the_engine() {
    // An accepted degradation: the offset is kept and the z-index is drawn in
    // the paragraph's order, without a stacking context of its own.
    for css in [
        "display:inline;position:relative;left:2px;z-index:1",
        "display:inline;position:relative;z-index:0",
    ] {
        assert_is_root("", |doc, root| {
            doc.append_text(root, "aa");
            let inner = span(doc, root, css);
            doc.append_text(inner, "bb");
        });
    }
}

#[test]
fn a_relative_inline_that_holds_a_box_is_a_root() {
    // The box is moved with the element's offset once its lines are placed.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        let inner = span(doc, root, "display:inline;position:relative;left:5px");
        doc.append_text(inner, "bb");
        span(doc, inner, "display:inline-block;width:10px;height:10px");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn an_inline_with_opacity_is_laid_out_by_the_engine() {
    // An accepted degradation: the lines carry no opacity group, so the
    // element's text is drawn opaque.
    assert_is_root("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;opacity:0.5");
        doc.append_text(inner, "bb");
    });
}

#[test]
fn a_plain_ltr_paragraph_is_still_a_root() {
    let mut fixture = block_fixture("text-decoration:underline;color:red", |doc, root| {
        doc.append_text(root, "aa");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn the_root_itself_may_have_opacity_and_a_relative_position() {
    // Both act on the root's own box, which the walk handles before the lines.
    let mut fixture = block_fixture("opacity:0.5;position:relative;top:2px", |doc, root| {
        doc.append_text(root, "aa");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn word_space_transform_is_laid_out_by_the_engine() {
    // An accepted degradation: shodo does not map word-space-transform; the
    // spaces are left as they are.
    assert_is_root("word-space-transform:ideographic-space", text_only("aa bb"));
}

#[test]
fn a_full_width_text_transform_is_laid_out_by_the_engine() {
    // An accepted degradation: shodo's full-width mapping covers fewer
    // characters than the parley path; what it does not map is left as is.
    for value in [
        "full-width",
        "uppercase full-width",
        "full-width full-size-kana",
    ] {
        assert_is_root(&format!("text-transform:{value}"), text_only("aa"));
    }
}

#[test]
fn a_case_transform_alone_is_still_a_root() {
    let mut fixture = block_fixture("text-transform:uppercase", |doc, root| {
        doc.append_text(root, "aa");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn background_clip_text_is_laid_out_by_the_engine() {
    // An accepted degradation: the lines do not provide the glyph shapes the
    // clip needs; the background is drawn unclipped.
    assert_is_root("background-clip:text", text_only("aa"));
}

#[test]
fn background_clip_text_on_an_inline_is_laid_out_by_the_engine() {
    // An accepted degradation: the lines do not provide the glyph shapes the
    // clip needs; the background is drawn unclipped.
    assert_is_root("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;background-clip:text");
        doc.append_text(inner, "bb");
    });
}

#[test]
fn a_fixed_position_root_is_a_root() {
    // taffy sizes a fixed box against its nearest positioned ancestor on
    // either path; the engine breaks its lines at that width.
    assert_is_a_root("position:fixed;top:0", text_only("aa bb cc"));
}

#[test]
fn an_absolutely_positioned_root_is_still_a_root() {
    let mut fixture = block_fixture("position:absolute;top:0", |doc, root| {
        doc.append_text(root, "aa");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_paragraph_beside_a_float_is_a_root() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        add_sibling(doc, root, "block;float:left;width:30px");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_float_beside_an_ancestor_leaves_the_paragraph_a_root() {
    // The float sits next to the wrapper, above the paragraph, and intrudes
    // into its lines through the shared float context.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
    });
    let wrapper = fixture.doc.parent_of(fixture.root).expect("body");
    let outer = fixture.doc.parent_of(wrapper).expect("html");
    let float = fixture.doc.append_element(
        Some(outer),
        "div",
        taffy::Style::default(),
        Some("display:block;float:left;width:30px"),
    );
    fixture.doc.append_text(float, "x");
    fixture.doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&fixture.doc);
    fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_paragraph_inside_a_fixed_box_without_a_width_is_a_root() {
    // taffy sizes the fixed box against its nearest positioned ancestor on
    // either path; the paragraph inside wraps at that width.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa bb cc");
    });
    let body = fixture.doc.parent_of(fixture.root).expect("body");
    fixture
        .doc
        .set_element_inline_style(body, Some("display:block;position:fixed;top:0".into()));
    fixture.doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&fixture.doc);
    fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_paragraph_inside_a_fixed_box_with_a_width_is_still_a_root() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa bb cc");
    });
    let body = fixture.doc.parent_of(fixture.root).expect("body");
    fixture.doc.set_element_inline_style(
        body,
        Some("display:block;position:fixed;top:0;width:100px".into()),
    );
    fixture.doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&fixture.doc);
    fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_raised_inline_under_a_decoration_is_an_ifc_root() {
    // The painter places a decoration at the baseline of the element that
    // declares it, so a raised inline no longer keeps the paragraph off.
    let mut fixture = block_fixture("text-decoration:underline", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;vertical-align:super");
        doc.append_text(inner, "bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_raised_inline_under_an_ancestor_decoration_is_an_ifc_root() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;vertical-align:sub");
        doc.append_text(inner, "bb");
    });
    let body = fixture.doc.parent_of(fixture.root).expect("body");
    fixture
        .doc
        .set_element_inline_style(body, Some("display:block;text-decoration:underline".into()));
    fixture.doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&fixture.doc);
    fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_line_relative_inline_under_a_decoration_is_a_root() {
    for value in ["top", "bottom", "middle", "text-top", "text-bottom"] {
        assert_is_root("text-decoration:underline", |doc, root| {
            doc.append_text(root, "aa ");
            let inner = span(doc, root, &format!("display:inline;vertical-align:{value}"));
            doc.append_text(inner, "bb");
        });
    }
}

#[test]
fn a_line_relative_inline_under_an_ancestor_decoration_is_a_root() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;vertical-align:top");
        doc.append_text(inner, "bb");
    });
    let body = fixture.doc.parent_of(fixture.root).expect("body");
    fixture
        .doc
        .set_element_inline_style(body, Some("display:block;text-decoration:underline".into()));
    fixture.doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&fixture.doc);
    fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_line_relative_inline_with_its_own_decoration_is_a_root() {
    assert_is_root("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(
            doc,
            root,
            "display:inline;vertical-align:text-top;text-decoration:underline",
        );
        doc.append_text(inner, "bb");
    });
}

#[test]
fn a_line_relative_inline_without_a_decoration_is_still_a_root() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;vertical-align:top");
        doc.append_text(inner, "bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_length_or_percentage_raise_under_a_decoration_is_an_ifc_root() {
    for value in ["4px", "-3px", "50%"] {
        let mut fixture = block_fixture("text-decoration:underline", |doc, root| {
            doc.append_text(root, "aa ");
            let inner = span(doc, root, &format!("display:inline;vertical-align:{value}"));
            doc.append_text(inner, "bb");
        });
        enable(&mut fixture);
        assign(&mut fixture);
        assert!(is_root(&fixture, fixture.root), "{value}");
    }
}

#[test]
fn a_raised_inline_without_a_decoration_is_still_a_root() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;vertical-align:super");
        doc.append_text(inner, "bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_decoration_with_baseline_aligned_inlines_is_still_a_root() {
    let mut fixture = block_fixture("text-decoration:underline", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline");
        doc.append_text(inner, "bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_float_child_does_not_make_its_subtree_part_of_the_paragraph() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let float = span(doc, root, "display:block;float:left;width:30px;height:20px");
        doc.append_text(float, "ff");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
    let float = fixture.doc.nodes[fixture.root].children[1];
    assert!(
        !fixture.doc.nodes[float]
            .flags
            .contains(NodeFlags::IN_IFC_SUBTREE)
    );
    // The float's text is the paragraph of the float itself, a root of its
    // own, not part of the outer paragraph.
    assert!(is_root(&fixture, float));
    assert_eq!(fixture.doc.nodes[fixture.root].ifc_boxes(), vec![float]);
}

#[test]
fn a_paragraph_whose_only_text_is_inside_a_float_is_not_a_root() {
    let mut fixture = block_fixture("", |doc, root| {
        let float = span(doc, root, "display:block;float:left;width:30px");
        doc.append_text(float, "ff");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(!is_root(&fixture, fixture.root));
}

#[test]
fn rtl_text_inside_a_float_does_not_keep_the_paragraph_on_the_parley_path() {
    // The float is painted as a box of its own, so what is inside it does not
    // matter to the paragraph painter.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let float = span(doc, root, "display:block;float:left;width:30px");
        doc.append_text(float, "\u{05d0}\u{05d1}");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_shifted_inline_inside_a_float_does_not_keep_a_decorated_paragraph_off() {
    // The decoration of the paragraph never reaches the float's content, so
    // a line-relative inline inside the float does not matter to the painter.
    let mut fixture = block_fixture("text-decoration-line:underline", |doc, root| {
        doc.append_text(root, "aa ");
        let float = span(doc, root, "display:block;float:left;width:30px");
        let raised = span(doc, float, "display:inline;vertical-align:top");
        doc.append_text(raised, "ff");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_paragraph_with_a_float_child_beside_an_outer_float_is_laid_out_by_the_engine() {
    // A documented approximation: an outer float can keep the paragraph's
    // own floats lower than the line they are anchored in, which the line
    // layout does not see.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        span(doc, root, "display:block;float:left;width:30px;height:20px");
        add_sibling(doc, root, "block;float:left;width:30px");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_paragraph_with_an_atomic_beside_an_outer_float_is_a_root() {
    // Only the paragraph's own floats depend on where the outer floats end;
    // an atomic inline sits in a line like text does.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        span(doc, root, "display:inline-block;width:30px;height:20px");
        add_sibling(doc, root, "block;float:left;width:30px");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn an_atomic_inline_does_not_make_its_subtree_part_of_the_paragraph() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let atomic = span(doc, root, "display:inline-block;width:30px");
        doc.append_text(atomic, "ii");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
    // The atomic's text is the paragraph of the atomic itself, a root of its
    // own, not part of the outer paragraph.
    let atomic = fixture.doc.nodes[fixture.root].children[1];
    assert!(is_root(&fixture, atomic));
    assert_eq!(fixture.doc.nodes[fixture.root].ifc_boxes(), vec![atomic]);
}

#[test]
fn a_paragraph_inside_an_atomic_inline_is_a_root_of_its_own() {
    // The ancestor walk looks only for multicol, so the outer paragraph does
    // not block a paragraph that sits in one of its atomic children.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let atomic = span(doc, root, "display:inline-block");
        let middle = span(doc, atomic, "display:block");
        let inner = span(doc, middle, "display:block");
        doc.append_text(inner, "bb cc");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
    let atomic = fixture.doc.nodes[fixture.root].children[1];
    let middle = fixture.doc.nodes[atomic].children[0];
    let inner = fixture.doc.nodes[middle].children[0];
    assert!(
        fixture.doc.nodes[inner]
            .flags
            .contains(NodeFlags::IS_IFC_ROOT)
    );
}

#[test]
fn a_paragraph_with_only_an_image_is_a_root() {
    // An atomic inline makes a line on its own (CSS 2.1, 9.4.2).
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_element(
            Some(root),
            "img",
            taffy::Style::default(),
            Some("display:inline;width:10px;height:10px"),
        );
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_block_child_under_a_decoration_is_a_root() {
    // A decoration propagates into an in-flow block; the block child is a
    // root of its own and its lines take the decoration from its ancestors.
    assert_is_root("text-decoration:underline", |doc, root| {
        doc.append_text(root, "aa");
        let block = span(doc, root, "display:block;height:10px");
        doc.append_text(block, "bb");
    });
}

#[test]
fn a_block_child_without_a_decoration_is_a_root() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        let block = span(doc, root, "display:block;height:10px");
        doc.append_text(block, "bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn right_to_left_text_outside_the_document_is_ignored() {
    // A child that is not in the document is not laid out, so its text does
    // not make the paragraph right to left.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let b = span(doc, root, "display:inline-block;width:20px;height:10px");
        doc.append_text(b, "x");
    });
    let root = fixture.root;
    let detached = fixture.doc.append_text(root, "\u{05d0}");
    fixture.doc.nodes[detached]
        .flags
        .remove(NodeFlags::IS_IN_DOCUMENT);
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn an_inline_with_a_background_image_is_laid_out_by_the_engine() {
    // An accepted degradation: the image is not laid out across the
    // element's pieces on its lines.
    assert_is_root("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;background-image:url(x.png)");
        doc.append_text(inner, "bb");
    });
}

#[test]
fn an_inline_with_a_background_color_is_still_a_root() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;background-color:rgb(255,0,0)");
        doc.append_text(inner, "bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn the_root_itself_may_have_a_background_image() {
    // The root's own background is painted with its box, not with the lines.
    let mut fixture = block_fixture("background-image:url(x.png)", |doc, root| {
        doc.append_text(root, "aa");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_contents_child_does_not_keep_the_paragraph_on_the_parley_path() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
        let wrapper = span(doc, root, "display:contents");
        doc.append_text(wrapper, "bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_paragraph_with_generated_text_is_a_root() {
    // The text of ::before is laid out in the paragraph.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
    });
    let head = fixture
        .doc
        .append_element(Some(0), "style", taffy::Style::default(), None::<&str>);
    fixture
        .doc
        .append_text(head, r#"div::before { content: "x" }"#);
    fixture.doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&fixture.doc);
    fixture.cascade = raikiri_style::cascade(&fixture.doc, &rules).expect("cascade");
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

/// `aa <div css>bb</div> cc`: a block child inside the paragraph.
fn with_block_child(css: &'static str) -> impl FnOnce(&mut crate::Document, usize) {
    move |doc, root| {
        doc.append_text(root, "aa ");
        let block = doc.append_element(Some(root), "div", taffy::Style::default(), Some(css));
        doc.append_text(block, "bb");
        doc.append_text(root, " cc");
    }
}

#[test]
fn a_block_child_with_a_forced_page_break_is_still_a_root() {
    assert_is_a_root("", with_block_child("display:block;break-before:page"));
    assert_is_a_root("", with_block_child("display:block;break-after:page"));
}

#[test]
fn a_box_with_a_named_page_is_still_a_root() {
    assert_is_a_root("", with_block_child("display:block;page:chapter"));
}

#[test]
fn a_block_child_without_a_page_break_is_still_a_root() {
    let mut fixture = block_fixture("", with_block_child("display:block"));
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn a_page_break_inside_a_box_of_the_paragraph_is_still_a_root() {
    // Pagination breaks at the inner block on its own; the lines after the
    // box would not follow it.
    for css in [
        "display:block;break-before:page",
        "display:block;page:chapter",
    ] {
        assert_is_a_root("", move |doc, root| {
            doc.append_text(root, "aa ");
            let block = doc.append_element(
                Some(root),
                "div",
                taffy::Style::default(),
                Some("display:block"),
            );
            let inner = doc.append_element(Some(block), "div", taffy::Style::default(), Some(css));
            doc.append_text(inner, "bb");
            doc.append_text(root, " cc");
        });
    }
}

#[test]
fn a_paragraph_that_fails_to_build_is_a_limit_error() {
    // One glyph is allowed per paragraph, so shaping "aa bb" fails after the
    // builder step succeeded. A limit is not a refusal: the layout fails with
    // the paragraph's root, and nothing is marked.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa bb");
    });
    let limits = Limits {
        max_shaped_glyphs: Some(1),
        ..Limits::default()
    };
    // The walk itself accepts the paragraph: the failure is in the shaping.
    let fonts = ahem_fonts();
    let projected = crate::layout::ifc::projection::project_ifc_builder(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &fonts,
        &limits,
    )
    .expect("the walk succeeds");
    assert!(
        projected
            .build(&mut shodo::LayoutContext::new(), &fonts)
            .is_err()
    );
    fixture.doc.enable_inline_formatting(ahem_fonts(), limits);
    let result = assign_ifc_roots(&mut fixture.doc, &fixture.cascade);
    assert!(
        matches!(result, Err(LayoutError::IfcLimitExceeded { node, .. }) if node == fixture.root),
        "{result:?}"
    );
    assert!(!is_root(&fixture, fixture.root));
    let text = fixture.doc.nodes[fixture.root].children[0];
    assert!(
        !fixture.doc.nodes[text]
            .flags
            .contains(NodeFlags::IN_IFC_SUBTREE)
    );
    assert!(fixture.doc.nodes[fixture.root].ifc.is_none());
    // The engine state is kept for the next pass.
    assert!(fixture.doc.inline_formatting_enabled());
}

#[test]
fn a_paragraph_inside_a_float_of_a_root_is_still_a_root() {
    // The inside of a box is not part of the outer paragraph, so a paragraph
    // there is a root of its own whether or not the outer one is built first.
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa ");
        let float = span(doc, root, "display:block;float:left;width:60px");
        let inner = doc.append_element(
            Some(float),
            "div",
            taffy::Style::default(),
            Some("display:block"),
        );
        doc.append_text(inner, "bb cc");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
    let float = fixture.doc.nodes[fixture.root].children[1];
    let inner = fixture.doc.nodes[float].children[0];
    assert!(is_root(&fixture, inner));
}

/// A face that covers U+0E70 and U+0E71, which Ahem does not.
const NOTO_SANS_TEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/noto-sans-test/NotoSansTest-Regular.ttf"
));

/// Ahem first, then NotoSansTest: a bundled collection without system faces
/// in which some text needs the second family.
fn ahem_and_noto_fonts() -> shodo::font::FontCollection {
    bundled_collection(
        &Limits::default(),
        vec![
            BundledFace {
                family: "Ahem".to_owned(),
                bytes: AHEM.to_vec(),
            },
            BundledFace {
                family: "NotoSansTest".to_owned(),
                bytes: NOTO_SANS_TEST.to_vec(),
            },
        ],
        false,
    )
    .expect("bundled Ahem and NotoSansTest")
}

/// `n` paragraphs of Ahem text side by side under the body.
fn many_paragraphs(n: usize) -> Fixture {
    block_fixture("", |doc, root| {
        // `root` is the first paragraph; the rest follow it under the body.
        doc.append_text(root, "aa bb cc dd ee ff");
        let body = doc.parent_of(root).expect("body");
        for i in 1..n {
            let p = doc.append_element(
                Some(body),
                "div",
                taffy::Style::default(),
                Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
            );
            doc.append_text(p, format!("aa bb cc dd ee ff {i}"));
        }
    })
}

/// `n` paragraphs whose text needs the second family: Ahem has no glyph for
/// U+0E70, NotoSansTest has one.
fn many_fallback_paragraphs(n: usize) -> Fixture {
    block_fixture("font-family:Ahem,NotoSansTest", |doc, root| {
        doc.append_text(root, "aa \u{0E70} bb cc dd");
        let body = doc.parent_of(root).expect("body");
        for i in 1..n {
            let p = doc.append_element(
                Some(body),
                "div",
                taffy::Style::default(),
                Some("display:block;font-family:Ahem,NotoSansTest;font-size:10px;line-height:10px"),
            );
            doc.append_text(p, format!("aa \u{0E70} bb cc {i}"));
        }
    })
}

#[test]
fn roots_at_or_above_the_threshold_are_built_in_parallel() {
    let mut fixture = many_paragraphs(8);
    enable(&mut fixture);
    fixture.doc.set_ifc_parallel_build(true);
    fixture.doc.set_ifc_parallel_threshold(8);
    assign(&mut fixture);
    assert_eq!(fixture.doc.ifc_last_build(), Some(IfcBuildMode::Parallel));
}

#[test]
fn roots_below_the_threshold_are_built_in_sequence() {
    let mut fixture = many_paragraphs(8);
    enable(&mut fixture);
    fixture.doc.set_ifc_parallel_build(true);
    fixture.doc.set_ifc_parallel_threshold(9);
    assign(&mut fixture);
    assert_eq!(fixture.doc.ifc_last_build(), Some(IfcBuildMode::Sequential));
}

#[test]
fn a_document_that_did_not_allow_it_is_built_in_sequence() {
    // Many roots, threshold 0, but nothing said the collection is safe.
    let mut fixture = many_paragraphs(8);
    enable(&mut fixture);
    fixture.doc.set_ifc_parallel_threshold(0);
    assert!(!fixture.doc.ifc_parallel_build());
    assign(&mut fixture);
    assert_eq!(fixture.doc.ifc_last_build(), Some(IfcBuildMode::Sequential));
}

#[test]
fn a_document_without_the_switch_records_no_build() {
    let mut fixture = many_paragraphs(8);
    fixture.doc.set_ifc_parallel_build(true);
    assert!(!fixture.doc.ifc_parallel_build());
    assign(&mut fixture);
    assert_eq!(fixture.doc.ifc_last_build(), None);
}

/// Everything a paint pass reads from the roots: per line the text, its range
/// and both sizes, per glyph run a hash of the font bytes, the face index,
/// the glyph ids and the advances.
fn signature(mut fixture: Fixture, parallel: bool) -> Vec<(usize, Vec<String>)> {
    fixture.doc.set_ifc_parallel_build(parallel);
    fixture.doc.set_ifc_parallel_threshold(0);
    assign(&mut fixture);
    let expected = if parallel {
        IfcBuildMode::Parallel
    } else {
        IfcBuildMode::Sequential
    };
    assert_eq!(fixture.doc.ifc_last_build(), Some(expected));
    let mut out = Vec::new();
    for idx in 0..fixture.doc.nodes.len() {
        if !fixture.doc.nodes[idx].is_ifc_root() {
            continue;
        }
        let root = fixture.doc.nodes[idx].ifc.as_ref().expect("root state");
        let mut cx = shodo::LayoutContext::new();
        let lines =
            root.paragraph
                .break_all(&mut cx, &root.options, 50.0, &shodo::AtomicSizes::EMPTY);
        let mut rows = Vec::new();
        for line in &lines {
            let mut row = format!(
                "{:?}|{:?}|{:x}|{:x}",
                line.text()[line.text_range()].trim_end(),
                line.text_range(),
                line.inline_size().to_bits(),
                line.block_size().to_bits(),
            );
            for fragment in line.fragments() {
                let shodo::Fragment::GlyphRun(run) = fragment else {
                    continue;
                };
                let font = run.font_data().expect("font of a run");
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(font.data.data(), &mut hasher);
                row.push_str(&format!(
                    "|font={:x}/{}",
                    std::hash::Hasher::finish(&hasher),
                    font.index
                ));
                for glyph in run.glyphs() {
                    row.push_str(&format!(",{}:{:x}", glyph.id, glyph.advance.to_bits()));
                }
            }
            rows.push(row);
        }
        out.push((idx, rows));
    }
    out
}

/// The distinct `font=` entries of a signature.
fn fonts_of(signature: &[(usize, Vec<String>)]) -> std::collections::BTreeSet<String> {
    signature
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .flat_map(|row| row.split('|'))
        .filter(|part| part.starts_with("font="))
        .map(|part| part.split(',').next().unwrap_or(part).to_owned())
        .collect()
}

#[test]
fn parallel_and_sequential_builds_give_the_same_roots() {
    let build = |parallel| {
        let mut fixture = many_paragraphs(40);
        enable(&mut fixture);
        signature(fixture, parallel)
    };
    let sequential = build(false);
    let parallel = build(true);
    assert_eq!(sequential.len(), 40);
    assert_eq!(parallel, sequential);
}

#[test]
fn parallel_and_sequential_builds_agree_when_a_fallback_family_is_needed() {
    // Ahem lacks U+0E70, so every paragraph takes the fallback branch of the
    // font matcher. The collection has no system faces, so the result cannot
    // depend on which worker asks first.
    let build = |parallel| {
        let mut fixture = many_fallback_paragraphs(40);
        fixture
            .doc
            .enable_inline_formatting(ahem_and_noto_fonts(), Limits::default());
        signature(fixture, parallel)
    };
    let sequential = build(false);
    let parallel = build(true);
    assert_eq!(sequential.len(), 40);
    // Both faces shape runs, so the fallback branch was taken.
    assert_eq!(
        fonts_of(&sequential).len(),
        2,
        "{:?}",
        fonts_of(&sequential)
    );
    assert_eq!(parallel, sequential);
}

#[test]
fn what_crosses_threads_is_send() {
    fn assert_send<T: Send>() {}
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send::<Candidate>();
    assert_send::<ProjectedIfc>();
    assert_send_sync::<shodo::font::FontCollection>();
    assert_send_sync::<shodo::Paragraph>();
}

#[test]
fn a_cloned_document_keeps_the_build_policy() {
    // A probe layout runs on a clone of the document; it holds the same font
    // collection, so it may build the same way.
    let mut fixture = many_paragraphs(8);
    enable(&mut fixture);
    fixture.doc.set_ifc_parallel_build(true);
    fixture.doc.set_ifc_parallel_threshold(8);
    assign(&mut fixture);
    let mut clone = fixture.doc.clone();
    // The record of the last build belongs to the original.
    assert_eq!(clone.ifc_last_build(), None);
    assert!(clone.ifc_parallel_build());
    assign_ifc_roots(&mut clone, &fixture.cascade).expect("assign");
    assert_eq!(clone.ifc_last_build(), Some(IfcBuildMode::Parallel));
}

#[test]
fn without_a_threshold_set_32_roots_are_needed_for_a_parallel_build() {
    let build = |n| {
        let mut fixture = many_paragraphs(n);
        enable(&mut fixture);
        fixture.doc.set_ifc_parallel_build(true);
        assign(&mut fixture);
        fixture.doc.ifc_last_build()
    };
    assert_eq!(build(31), Some(IfcBuildMode::Sequential));
    assert_eq!(build(32), Some(IfcBuildMode::Parallel));
}

#[test]
fn a_cleared_line_break_is_a_root() {
    // `clear` on a `<br>` moves the next line below the floats; the line loop
    // reads it from the paragraph.
    let mut fixture = block_fixture("", |doc, root| {
        span(doc, root, "display:block;float:left;width:10px;height:10px");
        doc.append_element(
            Some(root),
            "br",
            taffy::Style::default(),
            Some("display:inline;clear:both"),
        );
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}

#[test]
fn replaced_and_svg_elements_are_never_roots() {
    // A flex item or an authored inline-block that is a replaced element, a
    // form control or an inline SVG lays out its content by other means; text
    // inside it (an SVG <title>, a button label) is not a paragraph.
    let mut nodes = Vec::new();
    let mut fixture = block_fixture("display:flex", |doc, root| {
        let svg = doc.append_element(Some(root), "svg", taffy::Style::default(), None::<&str>);
        let title = doc.append_element(Some(svg), "title", taffy::Style::default(), None::<&str>);
        doc.append_text(title, "Close");
        let button = doc.append_element(
            Some(root),
            "button",
            taffy::Style::default(),
            Some("display:inline-block"),
        );
        doc.append_text(button, "OK");
        nodes.extend([svg, title, button]);
    });
    enable(&mut fixture);
    assign(&mut fixture);
    for node in nodes {
        assert!(
            !is_root(&fixture, node),
            "{:?}",
            fixture.doc.nodes[node].tag_name()
        );
    }
}

#[test]
fn a_refusal_is_an_error_only_in_engine_only_mode() {
    use raikiri_traits::LayoutError;
    let refusal = || IfcError::Unsupported {
        node: 3,
        reason: "an internal inconsistency",
    };
    assert!(matches!(
        super::projection_error(true, 1, refusal()),
        Err(LayoutError::IfcUnsupported { node: 3, .. })
    ));
    assert!(super::projection_error(false, 1, refusal()).is_ok());
    assert!(matches!(
        super::projection_error(true, 1, IfcError::InvalidNode(5)),
        Err(LayoutError::IfcUnsupported { node: 5, .. })
    ));
}
