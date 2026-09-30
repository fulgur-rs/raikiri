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
    let cases: [(&str, Build); 4] = [
        ("float child", |doc, root| {
            doc.append_text(root, "aa");
            let f = span(doc, root, "float:left");
            doc.append_text(f, "bb");
        }),
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
    // The switch stays on but the root no longer projects: add a float.
    let f = span(&mut fixture.doc, fixture.root, "float:left");
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
