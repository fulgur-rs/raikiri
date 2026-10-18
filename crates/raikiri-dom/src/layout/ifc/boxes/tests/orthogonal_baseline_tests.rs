//! An inline-block in a writing mode orthogonal to its paragraph's sits on
//! its bottom margin edge: its own baselines run across the line.

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// The height of a 10px Ahem paragraph (strut ascent 8px, descent 2px) that
/// holds one inline-block styled by `box_css` with the text `text`.
fn paragraph_height(box_css: &str, text: &str) -> f32 {
    let mut root_id = 0;
    let fixture = block_fixture("line-height:10px", |doc, root| {
        root_id = root;
        let atomic = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some(&format!("display:inline-block;{box_css}")),
        );
        if !text.is_empty() {
            doc.append_text(atomic, text);
        }
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    absolute_rect(&doc, root_id).3
}

#[test]
fn an_orthogonal_inline_block_sits_on_its_bottom_margin_edge() {
    // The box stands 50px above the baseline and the strut's descent hangs
    // below it, as for an inline-block without a line.
    assert_eq!(paragraph_height("height:50px", ""), 52.0);
    assert_eq!(
        paragraph_height("writing-mode:vertical-lr;height:50px", "A"),
        52.0
    );
    assert_eq!(
        paragraph_height("writing-mode:vertical-rl;height:50px", "A"),
        52.0
    );
}

#[test]
fn a_parallel_inline_block_sits_on_the_baseline_of_its_last_line() {
    // The line's baseline is 8px below the box top, so the box top lines up
    // with the strut's and the box runs 42px below the baseline.
    assert_eq!(paragraph_height("height:50px", "A"), 50.0);
}

#[test]
fn a_display_contents_ancestor_does_not_make_a_parallel_inline_block_orthogonal() {
    // The vertical wrapper generates no box, so the horizontal inline-block
    // still belongs to the horizontal paragraph and keeps its baseline.
    let mut root_id = 0;
    let fixture = block_fixture("line-height:10px", |doc, root| {
        root_id = root;
        let wrapper = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some("display:contents;writing-mode:vertical-lr"),
        );
        let atomic = doc.append_element(
            Some(wrapper),
            "div",
            taffy::Style::default(),
            Some("display:inline-block;writing-mode:horizontal-tb;height:50px"),
        );
        doc.append_text(atomic, "A");
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, root_id).3, 50.0);
}
