//! A line that starts with many left floats. Each float narrows the space of
//! the line, and the engine scans a line again when the start of its space
//! moves, so the line loop must not ask about a new space for every float.

use super::ASKED_START_CHANGES;
use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{ifc_ahem_fonts, page_box_800x600};

/// The number of times the start of the space changed, and the float nodes,
/// after laying out `count` 1px left floats followed by text in a container
/// of 400px, `before` being the text ahead of them.
fn lay_out(before: &str, count: usize) -> (usize, Vec<usize>, Document) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0;font:10px/10px Ahem"));
    let root = doc.append_element(Some(body), "div", Some("display:block;width:400px"));
    doc.append_text(root, before);
    let floats: Vec<usize> = (0..count)
        .map(|_| doc.append_element(Some(root), "span", Some("float:left;width:1px;height:1px")))
        .collect();
    doc.append_text(root, "A");
    doc.set_font_collection(ifc_ahem_fonts());
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    ASKED_START_CHANGES.with(|count| count.set(0));
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    let changes = ASKED_START_CHANGES.with(|count| count.get());
    (changes, floats, doc)
}

#[test]
fn the_engine_is_not_asked_about_a_new_space_for_every_float_at_the_start_of_a_line() {
    let (few, ..) = lay_out("", 64);
    let (many, ..) = lay_out("", 256);
    // The first floats narrow the space as they are met, and the line is asked
    // for again in the narrowed space once the floats end, in each pass over
    // the paragraph: far fewer changes than floats, and the same for more.
    assert!(few < 32, "{few} changes for 64 floats");
    assert_eq!(few, many, "the changes grow with the floats");
}

#[test]
fn floats_met_in_a_held_space_sit_where_each_narrowed_space_puts_them() {
    for count in [3, 5, 6, 40] {
        let (_, floats, doc) = lay_out("", count);
        for (index, &float) in floats.iter().enumerate() {
            let location = doc.nodes[float].layout.location;
            assert_eq!(
                (location.x, location.y),
                (index as f32, 0.0),
                "float {index} of {count}"
            );
        }
    }
}

#[test]
fn floats_after_content_of_the_line_are_met_in_the_space_the_content_narrows() {
    // Content comes before the floats, so none starts the line, and the space
    // is never held: each float moves the start of the space it is met in.
    let (changes, floats, doc) = lay_out("B", 12);
    assert!(changes >= 12, "{changes} changes");
    for (index, &float) in floats.iter().enumerate() {
        let location = doc.nodes[float].layout.location;
        assert_eq!(
            (location.x, location.y),
            (index as f32, 0.0),
            "float {index}"
        );
    }
}
