use super::*;

fn block(doc: &mut Document, parent: usize, css: &str) -> usize {
    doc.append_element(Some(parent), "div", Some(css))
}

/// A paragraph of `lines` lines of 10px Ahem text, one 30px word to a line.
fn paragraph(doc: &mut Document, parent: usize, css: &str, lines: usize) -> usize {
    let paragraph = block(
        doc,
        parent,
        &format!("display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;{css}"),
    );
    doc.append_text(paragraph, vec!["aaa"; lines].join(" "));
    paragraph
}

/// `body > [div(spacer px), div(10px, break-after:avoid), box]` on 100px
/// pages, `box` being the box `next` builds. Returns the page-flow y of the
/// heading and of the box.
fn heading_then(spacer: u32, next: impl FnOnce(&mut Document, usize) -> usize) -> (f32, f32) {
    let (mut heading, mut next_box) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, &format!("display:block;height:{spacer}px"));
        heading = block(doc, body, "display:block;height:10px;break-after:avoid");
        next_box = next(doc, body);
    });
    (page_flow_y(&doc, heading), page_flow_y(&doc, next_box))
}

#[test]
fn a_box_taller_than_its_one_line_stays_with_the_box_tied_to_it() {
    // The heading ends at 80 and the one line of the box fits after it. The
    // rest of the box runs on to the next page, so there is no break between
    // the heading and the box to avoid.
    for css in ["height:50px", "min-height:50px"] {
        assert_eq!(
            heading_then(70, |doc, body| paragraph(doc, body, css, 1)),
            (70.0, 80.0),
            "{css}"
        );
    }
    // The padding above the line counts: the line ends at 91 + 10.
    let padded = |spacer| {
        heading_then(spacer, |doc, body| {
            paragraph(doc, body, "height:50px;padding-top:5px", 1)
        })
    };
    assert_eq!(padded(70), (70.0, 80.0));
    assert_eq!(padded(76), (100.0, 110.0));
}

#[test]
fn a_heading_moves_with_a_box_whose_first_line_does_not_fit_after_it() {
    assert_eq!(
        heading_then(85, |doc, body| paragraph(doc, body, "height:50px", 1)),
        (100.0, 110.0)
    );
}

#[test]
fn a_heading_stays_with_the_lines_that_orphans_keeps_together() {
    // Two lines fit after the heading, which ends at 80, and `orphans` is 2.
    assert_eq!(
        heading_then(70, |doc, body| paragraph(doc, body, "", 6)),
        (70.0, 80.0)
    );
    // One line fits after it, which is too few.
    assert_eq!(
        heading_then(80, |doc, body| paragraph(doc, body, "", 6)),
        (100.0, 110.0)
    );
    // With `orphans: 1` it is enough.
    assert_eq!(
        heading_then(80, |doc, body| paragraph(
            doc,
            body,
            "orphans:1;widows:1",
            6
        )),
        (80.0, 90.0)
    );
}

#[test]
fn a_heading_moves_with_a_paragraph_too_short_to_break_between_its_lines() {
    // Three lines are too few for `orphans` and `widows` of 2 to let a break
    // fall between them, so the paragraph moves whole, and the heading with it.
    assert_eq!(
        heading_then(70, |doc, body| paragraph(doc, body, "", 3)),
        (100.0, 110.0)
    );
    // The three lines fit after a heading that ends earlier.
    assert_eq!(
        heading_then(60, |doc, body| paragraph(doc, body, "", 3)),
        (60.0, 70.0)
    );
}

#[test]
fn a_heading_stays_with_the_first_paragraph_of_the_box_tied_to_it() {
    // The wrapper holds the paragraph, below its own padding.
    let wrapped = |spacer, css: &str| {
        heading_then(spacer, |doc, body| {
            let wrapper = block(doc, body, &format!("display:block;height:50px;{css}"));
            paragraph(doc, wrapper, "", 1);
            wrapper
        })
    };
    assert_eq!(wrapped(70, ""), (70.0, 80.0));
    assert_eq!(wrapped(70, "padding-top:5px"), (70.0, 80.0));
    assert_eq!(wrapped(76, "padding-top:5px"), (100.0, 110.0));
    // A relative offset moves the paragraph on the page but not in the flow
    // the page breaks by: its line still ends at 90.
    assert_eq!(
        heading_then(70, |doc, body| {
            let wrapper = block(doc, body, "display:block;height:50px");
            paragraph(doc, wrapper, "position:relative;top:15px", 1);
            wrapper
        }),
        (70.0, 80.0)
    );
    // A child that generates no box does not start the wrapper.
    assert_eq!(
        heading_then(70, |doc, body| {
            let wrapper = block(doc, body, "display:block;height:50px");
            block(doc, wrapper, "display:none");
            paragraph(doc, wrapper, "", 1);
            wrapper
        }),
        (70.0, 80.0)
    );
    // An element that generates no box of its own does not stand between the
    // wrapper and the paragraph.
    assert_eq!(
        heading_then(70, |doc, body| {
            let wrapper = block(doc, body, "display:block;height:50px");
            let contents = block(doc, wrapper, "display:contents");
            paragraph(doc, contents, "", 1);
            wrapper
        }),
        (70.0, 80.0)
    );
}

#[test]
fn a_heading_moves_with_a_box_that_the_page_cannot_break_inside() {
    // The box avoids breaks inside it.
    assert_eq!(
        heading_then(70, |doc, body| paragraph(
            doc,
            body,
            "height:50px;break-inside:avoid",
            1
        )),
        (100.0, 110.0)
    );
    // The box has no line to hold, and a box without a box in its flow is a
    // unit of its own.
    assert_eq!(
        heading_then(70, |doc, body| block(
            doc,
            body,
            "display:block;height:50px"
        )),
        (100.0, 110.0)
    );
    // The lines of a vertical paragraph are columns: it is kept whole.
    assert_eq!(
        heading_then(70, |doc, body| paragraph(
            doc,
            body,
            "writing-mode:vertical-rl;height:50px",
            1
        )),
        (100.0, 110.0)
    );
}

#[test]
fn a_heading_moves_with_a_box_that_starts_with_something_the_lines_do_not_hold() {
    // A float, a box that breaks before itself, one that takes no block space,
    // and a block between the lines of a paragraph: the box is kept whole.
    let wrapped = |first: &str| {
        heading_then(70, |doc, body| {
            let wrapper = block(doc, body, "display:block;height:50px");
            block(doc, wrapper, first);
            paragraph(doc, wrapper, "", 1);
            wrapper
        })
    };
    for first in [
        "float:left;width:10px;height:10px",
        "display:block;height:10px;break-before:avoid",
        "display:block;height:0",
    ] {
        assert_eq!(wrapped(first), (100.0, 110.0), "{first}");
    }
    assert_eq!(
        heading_then(70, |doc, body| {
            let mixed = paragraph(doc, body, "height:50px", 1);
            block(doc, mixed, "display:block;height:10px");
            doc.append_text(mixed, "aaa");
            mixed
        }),
        (100.0, 110.0)
    );
}
