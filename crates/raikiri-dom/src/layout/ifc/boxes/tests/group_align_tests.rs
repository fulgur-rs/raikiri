//! `text-group-align` (CSS Text 4): the lines of a root are padded by their
//! shortest remaining space before `text-align` applies.

use crate::layout::layout_single_page;
use crate::layout::test_support::{page_box_800x600, with_ahem};

/// The inline start of each line's first glyph run.
fn starts(root_css: &str, build: impl FnOnce(&mut crate::Document, usize)) -> Vec<f32> {
    let mut fixture = crate::layout::ifc::test_support::sheet_fixture(
        "",
        &format!("font: 10px/10px Ahem; width: 100px; white-space: pre; {root_css}"),
        build,
    );
    layout_single_page(
        with_ahem(&mut fixture.doc),
        &fixture.cascade,
        page_box_800x600(),
    )
    .expect("layout");
    let lines = fixture.doc.nodes[fixture.root]
        .ifc
        .as_ref()
        .and_then(|root| root.lines.as_ref())
        .expect("lines");
    lines
        .lines
        .iter()
        .map(|line| {
            line.fragments()
                .find_map(|fragment| match fragment {
                    shodo::Fragment::GlyphRun(run) => Some(run.inline_start()),
                    _ => None,
                })
                .expect("a glyph run")
        })
        .collect()
}

fn text(text: &'static str) -> impl FnOnce(&mut crate::Document, usize) {
    move |doc, root| {
        doc.append_text(root, text);
    }
}

#[test]
fn lines_keep_their_alignment_inside_the_group() {
    // The lines leave 60px and 80px, so each gives up 60px.
    assert_eq!(starts("text-align: end", text("AAAA\nAA")), [60.0, 80.0]);
    assert_eq!(
        starts("text-group-align: end", text("AAAA\nAA")),
        [60.0, 60.0]
    );
    assert_eq!(
        starts(
            "text-group-align: center; text-align: center",
            text("AAAA\nAA")
        ),
        [30.0, 40.0]
    );
    assert_eq!(
        starts("text-group-align: start; text-align: end", text("AAAA\nAA")),
        [0.0, 20.0]
    );
    // Right-to-left positions run from the right: the group sits there and
    // the lines align to its left end.
    assert_eq!(
        starts(
            "text-group-align: right; text-align: left; direction: rtl",
            text("AAAA\nAA")
        ),
        [0.0, 20.0]
    );
}

#[test]
fn a_group_wider_than_the_root_stays_at_its_start() {
    // The first line overflows by 20px, so the group is 120px wide.
    assert_eq!(
        starts(
            "text-group-align: end; text-align: end",
            text("AAAAAAAAAAAA\nAA")
        ),
        [0.0, 100.0]
    );
    assert_eq!(
        starts(
            "text-group-align: center; text-align: center",
            text("AAAAAAAAAAAA\nAA")
        ),
        [0.0, 50.0]
    );
}

#[test]
fn the_measuring_pass_leaves_the_floats_to_the_real_one() {
    // The float shortens only the first line: it leaves 30px there and 80px
    // on the second. A float placed twice would push the first line further.
    let starts = starts("text-group-align: end", |doc, root| {
        crate::layout::ifc::test_support::span(
            doc,
            root,
            "float: left; display: block; width: 30px; height: 10px",
        );
        doc.append_text(root, "AAAA\nAA");
    });
    assert_eq!(starts, [60.0, 30.0]);
}

/// Lay out a group root with block children built by `build`, and return
/// the inline start of each line's first glyph run in each of `paragraphs`.
fn block_starts(
    root_css: &str,
    build: impl FnOnce(&mut crate::Document, usize) -> Vec<usize>,
) -> Vec<Vec<f32>> {
    let mut paragraphs = Vec::new();
    let mut fixture = crate::layout::ifc::test_support::sheet_fixture(
        "",
        &format!("font: 10px/10px Ahem; width: 100px; white-space: pre; {root_css}"),
        |doc, root| paragraphs = build(doc, root),
    );
    layout_single_page(
        with_ahem(&mut fixture.doc),
        &fixture.cascade,
        page_box_800x600(),
    )
    .expect("layout");
    paragraphs
        .iter()
        .map(|&paragraph| {
            let lines = fixture.doc.nodes[paragraph]
                .ifc
                .as_ref()
                .and_then(|root| root.lines.as_ref())
                .expect("lines");
            lines
                .lines
                .iter()
                .filter_map(|line| {
                    line.fragments().find_map(|fragment| match fragment {
                        shodo::Fragment::GlyphRun(run) => Some(run.inline_start()),
                        _ => None,
                    })
                })
                .collect()
        })
        .collect()
}

fn paragraph(doc: &mut crate::Document, parent: usize, style: &str, text: &str) -> usize {
    let block = doc.append_element(
        Some(parent),
        "p",
        taffy::Style::default(),
        Some(format!("display:block;margin:0;{style}").as_str()),
    );
    doc.append_text(block, text);
    block
}

#[test]
fn the_lines_of_block_children_align_as_one_group() {
    // The paragraphs leave 60px, 80px and 70px: every line gives up 60px.
    let starts = block_starts("text-group-align: end", |doc, root| {
        vec![
            paragraph(doc, root, "", "AAAA\nAA"),
            paragraph(doc, root, "", "AAA"),
        ]
    });
    assert_eq!(starts, [vec![60.0, 60.0], vec![60.0]]);
    // A nested block in the same formatting context joins the group too.
    let starts = block_starts("text-group-align: end", |doc, root| {
        let outer = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some("display:block"),
        );
        vec![
            paragraph(doc, outer, "", "AAAA"),
            paragraph(doc, root, "", "AA"),
        ]
    });
    assert_eq!(starts, [vec![60.0], vec![60.0]]);
}

#[test]
fn a_block_with_its_own_formatting_context_is_left_out_of_the_group() {
    // The `display: flow-root` paragraph aligns on its own, and its 90px of
    // free space does not narrow the group's 60px.
    let starts = block_starts("text-group-align: end", |doc, root| {
        vec![
            paragraph(doc, root, "", "AAAA"),
            paragraph(doc, root, "display: flow-root", "A"),
        ]
    });
    assert_eq!(starts, [vec![60.0], vec![0.0]]);
}

#[test]
fn a_block_inside_a_paragraph_joins_the_paragraphs_group() {
    // The root's own line leaves 80px and its block child's 60px.
    let starts = block_starts("text-group-align: end", |doc, root| {
        doc.append_text(root, "AA");
        vec![root, paragraph(doc, root, "", "AAAA")]
    });
    assert_eq!(starts, [vec![60.0], vec![60.0]]);
}

#[test]
fn a_group_root_that_is_its_own_formatting_context_groups_its_blocks() {
    let starts = block_starts("text-group-align: end; overflow: hidden", |doc, root| {
        vec![
            paragraph(doc, root, "", "AAAA"),
            paragraph(doc, root, "", "AA"),
        ]
    });
    assert_eq!(starts, [vec![60.0], vec![60.0]]);
}

#[test]
fn the_group_of_a_nested_root_is_its_own() {
    // The inner block declares a group of its own, so its single line pads
    // by its own 80px and the outer group's 60px stays with the first block.
    let starts = block_starts("text-group-align: end", |doc, root| {
        vec![
            paragraph(doc, root, "", "AAAA"),
            paragraph(doc, root, "text-group-align: end", "AA"),
        ]
    });
    assert_eq!(starts, [vec![60.0], vec![80.0]]);
}

#[test]
fn a_root_without_a_parent_block_context_groups_its_own_lines() {
    assert_eq!(
        starts("text-group-align: end; overflow: hidden", text("AAAA\nAA")),
        [60.0, 60.0]
    );
}

#[test]
fn physical_group_sides_place_the_group_left_or_right() {
    assert_eq!(
        starts("text-group-align: left", text("AAAA\nAA")),
        [0.0, 0.0]
    );
    assert_eq!(
        starts("text-group-align: right", text("AAAA\nAA")),
        [60.0, 60.0]
    );
}
