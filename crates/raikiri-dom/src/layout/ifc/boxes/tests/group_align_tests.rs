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
