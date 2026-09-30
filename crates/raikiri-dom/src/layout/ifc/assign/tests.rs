use super::*;
use crate::layout::ifc::test_support::{ahem_fonts, block_fixture, span};
use shodo::limits::Limits;

fn enable(fixture: &mut crate::layout::ifc::test_support::Fixture) {
    fixture
        .doc
        .enable_inline_formatting(ahem_fonts(), Limits::default());
}

fn assign(fixture: &mut crate::layout::ifc::test_support::Fixture) {
    assign_ifc_roots(&mut fixture.doc, &fixture.cascade);
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
    type Build = fn(&mut crate::Document, usize);
    let cases: [(&str, Build); 3] = [
        ("block child", |doc, root| {
            doc.append_text(root, "aa");
            let b = span(doc, root, "display:block");
            doc.append_text(b, "bb");
        }),
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
    // The switch stays on but the root no longer projects: add a block child.
    let f = span(&mut fixture.doc, fixture.root, "display:block");
    fixture.doc.append_text(f, "x");
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
fn a_root_beside_inline_text_stays_on_the_parley_path() {
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "aa");
    });
    // Put loose text next to the block under `body`.
    let body = fixture.doc.parent_of(fixture.root).expect("body");
    fixture.doc.append_text(body, "loose");
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(!is_root(&fixture, fixture.root));
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
    assert!(!is_root(&fixture, fixture.root));
}

#[test]
fn a_multicol_root_stays_on_the_parley_path() {
    let mut fixture = block_fixture("column-count:2", |doc, root| {
        doc.append_text(root, "aa bb cc dd");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    // The multicol dispatch runs before the ifc dispatch and would find no
    // children on a hidden-children root.
    assert!(!is_root(&fixture, fixture.root));
}

#[test]
fn a_vertical_root_stays_on_the_parley_path() {
    let mut fixture = block_fixture("writing-mode:vertical-rl", |doc, root| {
        doc.append_text(root, "aa bb");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    // `project_ifc` accepts a vertical text-only block; the measurement here
    // only knows the horizontal axis.
    assert!(!is_root(&fixture, fixture.root));
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
fn inline_level_siblings_still_disqualify_a_paragraph() {
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
        assert!(!is_root(&fixture, fixture.root), "{display}");
    }
}

/// Assert that the paragraph `build` fills under a root styled `css` stays on
/// the parley path.
fn assert_stays_on_parley(css: &str, build: impl FnOnce(&mut crate::Document, usize)) {
    let mut fixture = block_fixture(css, build);
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(!is_root(&fixture, fixture.root));
}

fn text_only(text: &'static str) -> impl FnOnce(&mut crate::Document, usize) {
    move |doc, root| {
        doc.append_text(root, text);
    }
}

#[test]
fn an_rtl_root_stays_on_the_parley_path() {
    assert_stays_on_parley("direction:rtl", text_only("aa"));
}

#[test]
fn an_rtl_character_stays_on_the_parley_path() {
    assert_stays_on_parley("", text_only("aa \u{05d0}\u{05d1}"));
}

#[test]
fn text_shadow_stays_on_the_parley_path() {
    assert_stays_on_parley("text-shadow:1px 1px red", text_only("aa"));
}

#[test]
fn text_emphasis_stays_on_the_parley_path() {
    assert_stays_on_parley("text-emphasis-style:dot", text_only("aa"));
}

#[test]
fn hanging_punctuation_stays_on_the_parley_path() {
    assert_stays_on_parley("hanging-punctuation:first", text_only("aa"));
}

#[test]
fn an_rtl_descendant_keeps_the_paragraph_on_the_parley_path() {
    assert_stays_on_parley("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;direction:rtl");
        doc.append_text(inner, "bb");
    });
}

#[test]
fn a_relatively_positioned_inline_keeps_the_paragraph_on_the_parley_path() {
    assert_stays_on_parley("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;position:relative;top:2px");
        doc.append_text(inner, "bb");
    });
}

#[test]
fn an_inline_with_opacity_keeps_the_paragraph_on_the_parley_path() {
    assert_stays_on_parley("", |doc, root| {
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
fn word_space_transform_stays_on_the_parley_path() {
    assert_stays_on_parley("word-space-transform:ideographic-space", text_only("aa bb"));
}

#[test]
fn a_full_width_text_transform_stays_on_the_parley_path() {
    // shodo's full-width mapping covers fewer characters than the parley path.
    for value in [
        "full-width",
        "uppercase full-width",
        "full-width full-size-kana",
    ] {
        assert_stays_on_parley(&format!("text-transform:{value}"), text_only("aa"));
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
fn background_clip_text_stays_on_the_parley_path() {
    // The clip needs the shape of the text, which the lines do not provide.
    assert_stays_on_parley("background-clip:text", text_only("aa"));
}

#[test]
fn background_clip_text_on_an_inline_keeps_the_paragraph_on_the_parley_path() {
    assert_stays_on_parley("", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;background-clip:text");
        doc.append_text(inner, "bb");
    });
}

#[test]
fn a_fixed_position_root_stays_on_the_parley_path() {
    // taffy sizes a fixed box against its nearest positioned ancestor, which
    // can be zero wide, and breaking at that width wraps every word. The
    // parley path shapes at the page width in advance and hides the error.
    assert_stays_on_parley("position:fixed;top:0", text_only("aa bb cc"));
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
fn a_paragraph_inside_a_fixed_box_without_a_width_stays_on_the_parley_path() {
    // taffy sizes the fixed box against its nearest positioned ancestor, which
    // can be zero wide, and the paragraph inside would wrap at that width.
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
    assert!(!is_root(&fixture, fixture.root));
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
fn a_raised_inline_under_a_decoration_stays_on_the_parley_path() {
    // A decoration that starts above a raised inline belongs at the parent's
    // baseline, but the painter places it at the baseline of each run.
    assert_stays_on_parley("text-decoration:underline", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = span(doc, root, "display:inline;vertical-align:super");
        doc.append_text(inner, "bb");
    });
}

#[test]
fn a_raised_inline_under_an_ancestor_decoration_stays_on_the_parley_path() {
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
    assert!(!is_root(&fixture, fixture.root));
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
    let text = fixture.doc.nodes[float].children[0];
    assert!(
        !fixture.doc.nodes[text]
            .flags
            .contains(NodeFlags::IN_IFC_SUBTREE)
    );
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
    // a raised inline inside the float does not matter to the painter.
    let mut fixture = block_fixture("text-decoration-line:underline", |doc, root| {
        doc.append_text(root, "aa ");
        let float = span(doc, root, "display:block;float:left;width:30px");
        let raised = span(doc, float, "display:inline;vertical-align:super");
        doc.append_text(raised, "ff");
    });
    enable(&mut fixture);
    assign(&mut fixture);
    assert!(is_root(&fixture, fixture.root));
}
