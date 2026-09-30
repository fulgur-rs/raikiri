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
