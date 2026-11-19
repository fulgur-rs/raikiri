//! The margin between a block child of a paragraph and the line after it,
//! and the lines of white space that the style preserves.

use super::*;

/// `body > div(20px wide, Ahem 20/30) > [div(60px), div(paragraph_css) >
/// [div(height:20px;block_css), after...], div > "Z"]` on 100px pages: the
/// paragraph's block child ends at 80px. Returns the page-flow y of the block
/// child and of the block holding `Z`.
fn line_after_block(paragraph_css: &str, block_css: &str, after: &[&str]) -> (f32, f32) {
    let (mut child, mut z) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        let outer = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:20px;font-family:Ahem;font-size:20px;line-height:30px"),
        );
        doc.append_element(Some(outer), "div", Some("display:block;height:60px"));
        let paragraph = doc.append_element(
            Some(outer),
            "div",
            Some(&format!("display:block;{paragraph_css}")),
        );
        child = doc.append_element(
            Some(paragraph),
            "div",
            Some(&format!("display:block;height:20px;{block_css}")),
        );
        doc.append_text(child, "T");
        for text in after {
            doc.append_text(paragraph, *text);
        }
        z = doc.append_element(Some(outer), "div", Some("display:block"));
        doc.append_text(z, "Z");
    });
    (page_flow_y(&doc, child), page_flow_y(&doc, z))
}

#[test]
fn a_page_edge_in_the_margin_above_a_line_after_a_block_truncates_it() {
    // The line starts the second page at 100, not at 120 where the margin
    // ends; the block holding `Z` follows it.
    assert_eq!(
        line_after_block("", "margin-bottom:40px", &["Y"]),
        (60.0, 130.0)
    );
    // The moved lines take the flow after the paragraph up with them: its
    // last line ends at 190, and `Z`, which then crosses the page end,
    // starts the third page.
    assert_eq!(
        line_after_block("", "margin-bottom:40px", &["Y Y Y"]),
        (60.0, 200.0)
    );
    // The block that forces a break after it opens the page the line starts.
    assert_eq!(
        line_after_block("", "margin-bottom:40px;break-after:page", &["Y"]),
        (60.0, 130.0)
    );
}

#[test]
fn a_line_of_preserved_white_space_is_placed_like_a_line_of_text() {
    // The line of one space after the block starts the second page: the
    // block stays on the first, and the margin above the line is truncated.
    assert_eq!(
        line_after_block("white-space:pre", "margin-bottom:40px", &[" "]),
        (60.0, 130.0)
    );
    // A line that crosses the page end moves to the next page alone.
    assert_eq!(
        line_after_block("white-space:pre", "margin-bottom:10px", &[" "]),
        (60.0, 130.0)
    );
    assert_eq!(
        line_after_block("", "margin-bottom:10px", &["\u{a0}"]),
        (60.0, 130.0)
    );
    // A line feed that only ends the line before it lays out no line of its
    // own, and a white space that the style collapses no line at all: the
    // line of `Z` is the one that crosses the page end then.
    assert_eq!(
        line_after_block("white-space:pre-line", "", &["Y", "\n"]),
        (60.0, 130.0)
    );
    assert_eq!(line_after_block("", "", &[" "]), (60.0, 100.0));
}

#[test]
fn a_margin_that_escapes_the_box_before_a_paragraph_is_truncated_above_its_line() {
    let (mut t, mut z) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        let outer = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:20px;font-family:Ahem;font-size:20px;line-height:30px"),
        );
        doc.append_element(Some(outer), "div", Some("display:block;height:60px"));
        let wrapper = doc.append_element(Some(outer), "div", Some("display:block"));
        t = doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;height:20px;margin-bottom:40px"),
        );
        let paragraph = doc.append_element(Some(outer), "div", Some("display:block"));
        doc.append_text(paragraph, "Y");
        z = doc.append_element(Some(outer), "div", Some("display:block"));
        doc.append_text(z, "Z");
    });
    assert_eq!((page_flow_y(&doc, t), page_flow_y(&doc, z)), (60.0, 130.0));
}

#[test]
fn only_white_space_that_the_style_collapses_places_no_line() {
    let (doc, cascade, root) = crate::layout::test_support::ahem_paragraph("x", "white-space:pre");
    let text = doc.nodes[root].children[0];
    assert!(crate::layout::ifc::text_lines::places_lines(
        &cascade, text, " "
    ));
    assert!(crate::layout::ifc::text_lines::places_lines(
        &cascade, text, "\u{a0}"
    ));
    assert!(!crate::layout::ifc::text_lines::places_lines(
        &cascade, text, ""
    ));
    let (doc, cascade, root) = crate::layout::test_support::ahem_paragraph("x", "");
    let text = doc.nodes[root].children[0];
    assert!(!crate::layout::ifc::text_lines::places_lines(
        &cascade, text, " \n"
    ));
    assert!(crate::layout::ifc::text_lines::places_lines(
        &cascade, text, "\u{a0}"
    ));
    drop(doc);
}
