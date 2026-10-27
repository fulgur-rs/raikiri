//! Where a block of a paragraph sits across the width of its container when its
//! side margins leave it room or over-constrain it (CSS 2.2 10.3.3), in both
//! directions. The positions were measured in Chromium.

use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{ifc_ahem_fonts, page_box_800x600};

/// The left edge of the block of `css` and `display`, between two lines of
/// text in a container of 100px with the given direction.
fn block_x(direction: &str, display: &str, css: &str) -> f32 {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0;font:20px/20px Ahem"));
    let root = doc.append_element(
        Some(body),
        "div",
        Some(format!("display:block;width:100px;direction:{direction}").as_str()),
    );
    doc.append_text(root, "A");
    doc.append_element(Some(root), "br", None::<&str>);
    let block = doc.append_element(
        Some(root),
        "div",
        Some(format!("display:{display};height:20px;{css}").as_str()),
    );
    doc.append_text(root, "B");
    doc.set_font_collection(ifc_ahem_fonts());
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    doc.nodes[block].layout.location.x
}

#[test]
fn a_block_that_overflows_a_right_to_left_container_ends_at_its_right_edge() {
    // An overflowing block counts its auto margins as zero, so its margin on
    // the left gives, whether the block shares the formatting context of the
    // paragraph (`block`) or has one of its own (`flow-root`).
    for display in ["block", "flow-root"] {
        for css in [
            "width:140px",
            "width:140px;margin:0 auto",
            "width:140px;margin-left:auto",
            "width:140px;margin-right:auto",
        ] {
            assert_eq!(block_x("rtl", display, css), -40.0, "{display} {css}");
            assert_eq!(block_x("ltr", display, css), 0.0, "{display} {css}");
        }
    }
}

#[test]
fn a_block_that_fits_a_right_to_left_container_keeps_its_right_margin_unless_it_is_auto() {
    for display in ["block", "flow-root"] {
        for (direction, css, left) in [
            ("rtl", "width:40px", 60.0),
            ("rtl", "width:40px;margin-right:10px", 50.0),
            ("rtl", "width:40px;margin-left:10px", 60.0),
            ("rtl", "width:40px;margin:0 auto", 30.0),
            ("ltr", "width:40px", 0.0),
            ("ltr", "width:40px;margin-left:10px", 10.0),
            ("ltr", "width:40px;margin:0 auto", 30.0),
        ] {
            assert_eq!(block_x(direction, display, css), left, "{display} {css}");
        }
    }
}
