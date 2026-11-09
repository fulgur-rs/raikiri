//! A float right after the spaces that end a line goes on that line when
//! the line's content leaves room for it. The expected positions are
//! Chromium's, with Ahem 20px on 30px lines in a 100px block.

use super::*;
use crate::layout::test_support::line_text;

/// The float rectangles relative to the paragraph, and the start and text
/// of each line.
type Laid = (Vec<(f32, f32)>, Vec<(Option<f32>, String)>);

/// A 100px paragraph of `parts`, where each `Ok` is text and each `Err` the
/// style of an empty float. Returns the float rectangles relative to the
/// paragraph, and the start and text of each line.
fn lay_out(parts: &[Result<&str, &str>]) -> Laid {
    lay_out_in("", parts)
}

/// [`lay_out`] with `css` added to the paragraph's style.
fn lay_out_in(css: &str, parts: &[Result<&str, &str>]) -> Laid {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let root = doc.append_element(
        Some(body),
        "div",
        Some(&format!(
            "display:block;width:100px;font-family:Ahem;font-size:20px;line-height:30px;{css}"
        )),
    );
    let mut floats = Vec::new();
    for part in parts {
        match *part {
            Ok(text) => {
                doc.append_text(root, text);
            }
            Err(css) => floats.push(doc.append_element(
                Some(root),
                "div",
                Some(&format!("display:block;{css}")),
            )),
        }
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("layout");
    let origin = absolute_rect(&doc, root);
    let rects = floats
        .iter()
        .map(|&float| {
            let rect = absolute_rect(&doc, float);
            (rect.0 - origin.0, rect.1 - origin.1)
        })
        .collect();
    let lines = doc.nodes[root]
        .ifc_lines()
        .expect("ifc lines")
        .iter()
        .map(|line| (line_start_x(line), line_text(line)))
        .collect();
    (rects, lines)
}

const FLOAT_20: &str = "float:left;width:20px;height:20px";

#[test]
fn a_float_after_the_spaces_that_end_a_line_stays_on_it() {
    // The space before the float hangs, so the line takes 60px and the float
    // its 20px beside it.
    let (floats, lines) = lay_out(&[Ok("z z "), Err(FLOAT_20), Ok("z z z z")]);
    assert_eq!(floats, [(0.0, 0.0)]);
    assert_eq!(lines[0], (Some(20.0), "z z".to_owned()));
    assert_eq!(lines[1], (Some(0.0), "z z z".to_owned()));
    // A right float sits at the right edge of the same line.
    let (floats, lines) = lay_out(&[
        Ok("z z "),
        Err("float:right;width:20px;height:20px"),
        Ok("z z z z"),
    ]);
    assert_eq!(floats, [(80.0, 0.0)]);
    assert_eq!(lines[0].0, Some(0.0));
    // The float may take all the room the line's content leaves.
    let (floats, _) = lay_out(&[
        Ok("z z "),
        Err("float:left;width:40px;height:20px"),
        Ok("z z z z"),
    ]);
    assert_eq!(floats, [(0.0, 0.0)]);
    // So may it when spaces are preserved, as they hang too.
    let (floats, _) = lay_out_in(
        "white-space:pre-wrap",
        &[Ok("z z "), Err(FLOAT_20), Ok("z z z z")],
    );
    assert_eq!(floats, [(0.0, 0.0)]);
}

#[test]
fn a_float_the_line_breaks_before_without_meeting_it_joins_the_line() {
    // `zzzz z` does not fit 100px, so the line breaks before the float; the
    // float still fits beside `zzzz` and goes on the line.
    let (floats, lines) = lay_out(&[Ok("zzzz "), Err(FLOAT_20), Ok("z z z z")]);
    assert_eq!(floats, [(0.0, 0.0)]);
    assert_eq!(lines[0], (Some(20.0), "zzzz".to_owned()));
    assert_eq!(lines[1], (Some(0.0), "z z z".to_owned()));
}

#[test]
fn floats_after_the_line_end_join_it_while_they_fit() {
    let (floats, lines) = lay_out(&[Ok("z z "), Err(FLOAT_20), Err(FLOAT_20), Ok("z z z z")]);
    assert_eq!(floats, [(0.0, 0.0), (20.0, 0.0)]);
    assert_eq!(lines[0].0, Some(40.0));
    // The second does not fit beside the first and the line, so it starts
    // the next line.
    let (floats, lines) = lay_out(&[
        Ok("z z "),
        Err(FLOAT_20),
        Err("float:left;width:30px;height:20px"),
        Ok("z z z z"),
    ]);
    assert_eq!(floats, [(0.0, 0.0), (0.0, 30.0)]);
    assert_eq!(lines[0].0, Some(20.0));
    assert_eq!(lines[1].0, Some(30.0));
}

#[test]
fn a_float_after_the_line_end_that_does_not_fit_goes_to_the_next_line() {
    // Too wide for the room beside the line's content.
    for parts in [
        [
            Ok("z z "),
            Err("float:left;width:50px;height:20px"),
            Ok("z z z z"),
        ],
        [
            Ok("zzzz "),
            Err("float:left;width:30px;height:20px"),
            Ok("z z z z"),
        ],
        // A no-break space does not hang, so the line takes 80px.
        [Ok("z z\u{a0}"), Err(FLOAT_20), Ok("z z z z")],
    ] {
        let (floats, _) = lay_out(&parts);
        assert_eq!(floats, [(0.0, 30.0)], "{parts:?}");
    }
    // A forced break ends the line before the float.
    let (floats, _) = lay_out_in(
        "white-space:pre-line",
        &[Ok("z z\n"), Err(FLOAT_20), Ok("z z z z")],
    );
    assert_eq!(floats, [(0.0, 30.0)]);
}

#[test]
fn a_float_after_the_line_end_keeps_the_rules_of_the_floats_it_follows() {
    // One that clears a float beside the line goes to the next line.
    let (floats, _) = lay_out(&[
        Err(FLOAT_20),
        Ok("z z "),
        Err("float:left;width:20px;height:20px;clear:left"),
        Ok("z z z z"),
    ]);
    assert_eq!(floats, [(0.0, 0.0), (0.0, 30.0)]);
    // One after a float placed below the line does not go above it.
    let (floats, _) = lay_out(&[
        Err("float:left;width:60px;height:20px"),
        Err("float:left;width:60px;height:20px"),
        Ok("z "),
        Err(FLOAT_20),
        Ok("z z z z"),
    ]);
    assert_eq!(floats, [(0.0, 0.0), (0.0, 20.0), (60.0, 30.0)]);
}

#[test]
fn a_float_after_the_line_end_counts_the_indent_and_direction() {
    // A 20px indent leaves the float its room; a 30px one does not.
    for (css, top) in [("text-indent:20px", 0.0), ("text-indent:30px", 30.0)] {
        let (floats, _) = lay_out_in(css, &[Ok("z z "), Err(FLOAT_20), Ok("z z z z")]);
        assert_eq!(floats, [(0.0, top)], "{css}");
    }
    let (floats, _) = lay_out_in("direction:rtl", &[Ok("z z "), Err(FLOAT_20), Ok("z z z z")]);
    assert_eq!(floats, [(0.0, 0.0)]);
}
