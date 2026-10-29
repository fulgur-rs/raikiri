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
fn a_heading_moves_with_a_paragraph_that_has_no_more_lines_than_its_orphans() {
    // Two lines cannot break between them with `orphans` of 2, so the
    // paragraph moves whole, and the heading with it.
    assert_eq!(
        heading_then(80, |doc, body| paragraph(doc, body, "", 2)),
        (100.0, 110.0)
    );
    // The two lines fit after a heading that ends earlier.
    assert_eq!(
        heading_then(70, |doc, body| paragraph(doc, body, "", 2)),
        (70.0, 80.0)
    );
}

#[test]
fn a_heading_stays_with_the_orphans_of_a_paragraph_too_short_for_its_widows() {
    // Three lines cannot have two orphans and two widows: the paragraph keeps
    // its orphans and the last line goes to the next page, so the heading,
    // which ends at 80, stays with the two lines that fit after it.
    assert_eq!(
        heading_then(70, |doc, body| paragraph(doc, body, "", 3)),
        (70.0, 80.0)
    );
    // One line fits after it, which is too few, so the paragraph moves whole,
    // and the heading with it.
    assert_eq!(
        heading_then(80, |doc, body| paragraph(doc, body, "", 3)),
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

#[test]
fn a_heading_moves_with_a_box_whose_closing_edges_would_be_alone_on_the_next_page() {
    // The heading ends at 80 and the line at 90, but the 30px of padding or
    // border that close the box do not fit: they would start the next page.
    for edges in ["padding-bottom:30px", "border-bottom:30px solid black"] {
        let wrapped = heading_then(70, |doc, body| {
            let wrapper = block(doc, body, &format!("display:block;{edges}"));
            paragraph(doc, wrapper, "", 1);
            wrapper
        });
        assert_eq!(wrapped, (100.0, 110.0), "{edges}");
        // A paragraph holds them as well, and so does one too short to break
        // between its lines.
        for lines in [1, 2] {
            assert_eq!(
                heading_then(70, |doc, body| paragraph(doc, body, edges, lines)),
                (100.0, 110.0),
                "{edges} {lines}"
            );
        }
    }
    // The first box of a wrapper holds them too.
    assert_eq!(
        heading_then(70, |doc, body| {
            let wrapper = block(doc, body, "display:block");
            let inner = block(doc, wrapper, "display:block;padding-bottom:30px");
            paragraph(doc, inner, "", 1);
            block(doc, wrapper, "display:block;height:10px");
            wrapper
        }),
        (100.0, 110.0)
    );
    // The same holds when the wrapper that holds the box has edges of its
    // own, and when a definite height is just the line and the padding.
    assert_eq!(
        heading_then(70, |doc, body| {
            let wrapper = block(doc, body, "display:block;padding-bottom:30px");
            let inner = block(doc, wrapper, "display:block;padding-bottom:30px");
            paragraph(doc, inner, "", 1);
            wrapper
        }),
        (100.0, 110.0)
    );
    assert_eq!(
        heading_then(70, |doc, body| paragraph(
            doc,
            body,
            "height:10px;padding-bottom:30px",
            1
        )),
        (100.0, 110.0)
    );
    // Padding that fits after the line is no reason to move, and neither is
    // a paragraph that breaks between its lines.
    assert_eq!(
        heading_then(70, |doc, body| paragraph(
            doc,
            body,
            "padding-bottom:10px",
            1
        )),
        (70.0, 80.0)
    );
    assert_eq!(
        heading_then(70, |doc, body| paragraph(
            doc,
            body,
            "padding-bottom:30px",
            6
        )),
        (70.0, 80.0)
    );
}

#[test]
fn a_heading_moves_with_a_paragraph_that_cannot_move_its_lines_down_inside_it() {
    // Three of the four lines fit after the heading, which ends at 70, and
    // `widows` is 2. A paragraph that can move lines down splits two and two.
    assert_eq!(
        heading_then(60, |doc, body| paragraph(doc, body, "", 4)),
        (60.0, 70.0)
    );
    // One with a definite height cannot: the page breaks where its lines
    // fall, or it moves whole.
    for css in ["height:50px", "min-height:50px"] {
        assert_eq!(
            heading_then(60, |doc, body| paragraph(doc, body, css, 4)),
            (100.0, 110.0),
            "{css}"
        );
    }
    // Nor can one that lays out a box of its own among the lines.
    assert_eq!(
        heading_then(60, |doc, body| {
            let mixed = paragraph(doc, body, "", 3);
            doc.append_element(
                Some(mixed),
                "span",
                Some("display:inline-block;width:10px;height:10px"),
            );
            mixed
        }),
        (100.0, 110.0)
    );
}

/// An empty collapsed-border table.
fn collapsed_table(doc: &mut Document, parent: usize, css: &str) -> usize {
    block(
        doc,
        parent,
        &format!("display:table;border-collapse:collapse;border-spacing:0;{css}"),
    )
}

/// A row of `parent` with one cell, one 30px Ahem word tall, styled with
/// `cell_css`. Returns the cell.
fn row_with_cell(doc: &mut Document, parent: usize, cell_css: &str) -> usize {
    row_with_cell_in(doc, parent, "", cell_css)
}

/// [`row_with_cell`] with `row_css` on the row.
fn row_with_cell_in(doc: &mut Document, parent: usize, row_css: &str, cell_css: &str) -> usize {
    let tr = block(doc, parent, &format!("display:table-row;{row_css}"));
    let td = block(
        doc,
        tr,
        &format!(
            "display:table-cell;padding:0;width:30px;font-family:Ahem;font-size:10px;line-height:10px;{cell_css}"
        ),
    );
    doc.append_text(td, "aaa");
    td
}

/// A collapsed-border table of `rows` rows with one cell each, styled with
/// `cell_css`. Returns the first cell.
fn table_of_rows(
    doc: &mut Document,
    parent: usize,
    css: &str,
    rows: usize,
    cell_css: &str,
) -> usize {
    let table = collapsed_table(doc, parent, css);
    let first_cell = row_with_cell(doc, table, cell_css);
    for _ in 1..rows {
        row_with_cell(doc, table, cell_css);
    }
    first_cell
}

#[test]
fn a_heading_moves_with_the_first_row_of_the_table_after_it() {
    // The row is the part of the table the page cannot break away from the
    // heading: it moves whole, so the heading moves with it.
    for (spacer, expected) in [
        (70, (70.0, 80.0)),
        (80, (80.0, 90.0)),
        (85, (100.0, 110.0)),
        (88, (100.0, 110.0)),
    ] {
        assert_eq!(
            heading_then(spacer, |doc, body| table_of_rows(doc, body, "", 1, "")),
            expected,
            "{spacer}"
        );
    }
}

#[test]
fn a_heading_moves_with_a_tall_first_row_that_fits_the_next_page() {
    // The row would cross the edge, so it starts the next page.
    assert_eq!(
        heading_then(70, |doc, body| table_of_rows(
            doc,
            body,
            "",
            1,
            "height:50px"
        )),
        (100.0, 110.0)
    );
}

#[test]
fn a_heading_stays_when_the_first_row_of_the_table_after_it_fits() {
    assert_eq!(
        heading_then(80, |doc, body| table_of_rows(doc, body, "", 2, "")),
        (80.0, 90.0)
    );
}

#[test]
fn a_heading_stays_when_the_first_row_of_the_table_after_it_fills_no_page() {
    // The heading and the row together are taller than a page, so no page
    // could hold both and the avoided break is given up.
    assert_eq!(
        heading_then(70, |doc, body| table_of_rows(
            doc,
            body,
            "",
            1,
            "height:120px"
        )),
        (70.0, 80.0)
    );
}

#[test]
fn a_table_avoiding_a_break_before_it_takes_the_box_before_it_to_the_next_page() {
    let (mut before, mut cell) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:85px");
        before = block(doc, body, "display:block;height:10px");
        cell = table_of_rows(doc, body, "break-before:avoid", 1, "");
    });
    assert_eq!(
        (page_flow_y(&doc, before), page_flow_y(&doc, cell)),
        (100.0, 110.0)
    );
}

#[test]
fn a_heading_moves_with_a_first_row_group_that_the_page_cannot_break_inside() {
    // The first row alone fits after the heading, but its group moves whole.
    assert_eq!(
        heading_then(70, |doc, body| {
            let table = collapsed_table(doc, body, "");
            let group = block(doc, table, "display:table-row-group;break-inside:avoid");
            let first = row_with_cell(doc, group, "");
            row_with_cell(doc, group, "");
            row_with_cell(doc, group, "");
            first
        }),
        (100.0, 110.0)
    );
}

#[test]
fn a_heading_moves_with_a_table_that_the_page_cannot_break_inside() {
    assert_eq!(
        heading_then(70, |doc, body| table_of_rows(
            doc,
            body,
            "break-inside:avoid",
            3,
            ""
        )),
        (100.0, 110.0)
    );
    // A table taller than a page breaks inside anyway, so the heading stays.
    let (heading, _) = heading_then(70, |doc, body| {
        table_of_rows(doc, body, "break-inside:avoid", 3, "height:40px")
    });
    assert_eq!(heading, 70.0);
}

#[test]
fn a_heading_moves_with_the_first_row_of_a_table_that_holds_an_inline_table() {
    // Only the row of the table around the inline table is what moves whole.
    assert_eq!(
        heading_then(70, |doc, body| {
            let cell = table_of_rows(doc, body, "", 1, "height:30px");
            let inline = block(doc, cell, "display:inline-table;border-spacing:0");
            row_with_cell(doc, inline, "");
            cell
        }),
        (100.0, 110.0)
    );
}

#[test]
fn a_heading_stays_before_a_table_that_does_not_start_with_its_first_row() {
    // A caption above the grid comes before the first row, so the first
    // row's end is not what the page has to hold with the heading.
    assert_eq!(
        heading_then(85, |doc, body| {
            let table = collapsed_table(doc, body, "");
            block(doc, table, "display:table-caption;height:5px");
            row_with_cell(doc, table, "")
        }),
        (85.0, 100.0)
    );
    // Nor does the first row of a header group repeated on every page.
    assert_eq!(
        heading_then(85, |doc, body| {
            let table = collapsed_table(doc, body, "");
            let header = block(doc, table, "display:table-header-group");
            row_with_cell(doc, header, "");
            let group = block(doc, table, "display:table-row-group");
            let first = row_with_cell(doc, group, "");
            row_with_cell(doc, group, "");
            first
        }),
        (85.0, 110.0)
    );
}

#[test]
fn a_heading_stays_before_a_table_whose_first_row_starts_a_page() {
    // The forced break sends the row to the next page whatever the heading
    // does, so the heading has nothing to move with.
    assert_eq!(
        heading_then(85, |doc, body| {
            let table = collapsed_table(doc, body, "");
            row_with_cell_in(doc, table, "break-before:page", "")
        }),
        (85.0, 100.0)
    );
    assert_eq!(
        heading_then(85, |doc, body| {
            let table = collapsed_table(doc, body, "");
            let group = block(doc, table, "display:table-row-group;break-before:page");
            row_with_cell(doc, group, "")
        }),
        (85.0, 100.0)
    );
}

/// `body > [div(spacer px), wrapper > [div(10px, break-after:avoid), box]]` on
/// 100px pages, `box` being the box `next` builds in the wrapper. Returns the
/// page-flow y of the heading and of the box.
fn heading_in_wrapper_then(
    spacer: u32,
    wrapper_css: &str,
    next: impl FnOnce(&mut Document, usize) -> usize,
) -> (f32, f32) {
    let (mut heading, mut next_box) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, &format!("display:block;height:{spacer}px"));
        let wrapper = block(doc, body, &format!("display:block;{wrapper_css}"));
        heading = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
        next_box = next(doc, wrapper);
    });
    (page_flow_y(&doc, heading), page_flow_y(&doc, next_box))
}

/// The page-flow y of the heading and of the paragraph of `lines` lines that
/// follows it in the wrapper `wrapper_css` after a spacer of `spacer`px, with
/// `css` on the paragraph.
fn heading_and_paragraph_in_wrapper(
    spacer: u32,
    wrapper_css: &str,
    css: &str,
    lines: usize,
) -> (f32, f32) {
    heading_in_wrapper_then(spacer, wrapper_css, |doc, wrapper| {
        paragraph(doc, wrapper, css, lines)
    })
}

#[test]
fn a_heading_at_the_start_of_a_wrapper_moves_with_the_paragraph_after_it() {
    // The wrapper holds the heading and the paragraph, as the body does in
    // `a_heading_stays_with_the_lines_that_orphans_keeps_together`.
    let nested =
        |spacer, css: &str, lines| heading_and_paragraph_in_wrapper(spacer, "", css, lines);
    // Two lines fit after the heading, which ends at 80: it stays.
    assert_eq!(nested(70, "", 6), (70.0, 80.0));
    // One line fits, which is too few: the heading and the paragraph move.
    assert_eq!(nested(80, "", 6), (100.0, 110.0));
    // Nothing fits after the heading.
    assert_eq!(nested(85, "", 6), (100.0, 110.0));
    // With `orphans: 1` one line is enough.
    assert_eq!(nested(80, "orphans:1;widows:1", 6), (80.0, 90.0));
    // A paragraph too short to break between its lines moves whole.
    assert_eq!(nested(80, "", 2), (100.0, 110.0));
    assert_eq!(nested(70, "", 2), (70.0, 80.0));
    // The same holds in a wrapper inside a wrapper.
    let deeper = |spacer| {
        let (mut heading, mut paragraph_id) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{spacer}px"));
            let outer = block(doc, body, "display:block");
            let inner = block(doc, outer, "display:block");
            heading = block(doc, inner, "display:block;height:10px;break-after:avoid");
            paragraph_id = paragraph(doc, inner, "", 6);
        });
        (page_flow_y(&doc, heading), page_flow_y(&doc, paragraph_id))
    };
    assert_eq!(deeper(85), (100.0, 110.0));
    assert_eq!(deeper(70), (70.0, 80.0));
}

#[test]
fn the_edges_of_a_wrapper_move_with_the_heading_at_its_start() {
    // The wrapper's padding and border are above the heading. The page breaks
    // before the wrapper, as there is no break between them and the heading.
    let (mut wrapper, mut heading) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:70px");
        wrapper = block(
            doc,
            body,
            "display:block;padding-top:5px;border-top:2px solid black",
        );
        heading = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
        paragraph(doc, wrapper, "", 6);
    });
    // The heading would end at 87 and no line fits after it.
    assert_eq!(page_flow_y(&doc, wrapper), 100.0);
    assert_eq!(page_flow_y(&doc, heading), 107.0);
}

#[test]
fn a_heading_in_the_middle_of_a_wrapper_moves_with_the_paragraph_after_it() {
    let after_a_box = |spacer, lines| {
        let (mut before, mut heading, mut paragraph_id) = (0, 0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{spacer}px"));
            let wrapper = block(doc, body, "display:block");
            before = block(doc, wrapper, "display:block;height:10px");
            heading = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
            paragraph_id = paragraph(doc, wrapper, "", lines);
        });
        [before, heading, paragraph_id].map(|id| page_flow_y(&doc, id))
    };
    // The box before the heading stays: the page breaks between the two.
    assert_eq!(after_a_box(60, 6), [60.0, 70.0, 80.0]);
    assert_eq!(after_a_box(70, 6), [70.0, 100.0, 110.0]);
    assert_eq!(after_a_box(80, 6), [80.0, 100.0, 110.0]);
    // Two lines fit after the heading, which ends at 80, so nothing moves.
    assert_eq!(after_a_box(60, 3), [60.0, 70.0, 80.0]);
}

#[test]
fn the_headings_before_a_paragraph_move_with_it_wherever_they_are() {
    let chain = |spacer| {
        let mut ids = [0; 3];
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{spacer}px"));
            let wrapper = block(doc, body, "display:block");
            ids[0] = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
            ids[1] = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
            ids[2] = paragraph(doc, wrapper, "", 6);
        });
        ids.map(|id| page_flow_y(&doc, id))
    };
    assert_eq!(chain(60), [60.0, 70.0, 80.0]);
    assert_eq!(chain(70), [100.0, 110.0, 120.0]);
    // A break before the paragraph does the same as a break after the
    // heading.
    let break_before = |spacer| {
        heading_in_wrapper_then(spacer, "", |doc, wrapper| {
            paragraph(doc, wrapper, "break-before:avoid", 6)
        })
    };
    // The heading in this helper avoids a break after it as well, so use a
    // plain box before the paragraph here.
    let plain_heading = |spacer| {
        let (mut heading, mut paragraph_id) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{spacer}px"));
            let wrapper = block(doc, body, "display:block");
            heading = block(doc, wrapper, "display:block;height:10px");
            paragraph_id = paragraph(doc, wrapper, "break-before:avoid", 6);
        });
        (page_flow_y(&doc, heading), page_flow_y(&doc, paragraph_id))
    };
    assert_eq!(break_before(85), (100.0, 110.0));
    assert_eq!(plain_heading(85), (100.0, 110.0));
    assert_eq!(plain_heading(70), (70.0, 80.0));
}

#[test]
fn a_box_before_a_wrapper_that_starts_with_a_tied_heading_moves_with_them() {
    // `B` avoids a break after it, the wrapper starts with a heading that does
    // too, and no line of the paragraph fits after the heading.
    let ids = |spacer, in_block| {
        let mut ids = [0; 3];
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{spacer}px"));
            let parent = if in_block {
                block(doc, body, "display:block")
            } else {
                body
            };
            ids[0] = block(doc, parent, "display:block;height:10px;break-after:avoid");
            let wrapper = block(doc, parent, "display:block");
            ids[1] = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
            ids[2] = paragraph(doc, wrapper, "", 6);
        });
        ids.map(|id| page_flow_y(&doc, id))
    };
    for in_block in [false, true] {
        assert_eq!(ids(70, in_block), [100.0, 110.0, 120.0], "{in_block}");
        assert_eq!(ids(60, in_block), [60.0, 70.0, 80.0], "{in_block}");
    }
}

#[test]
fn a_float_between_the_heading_and_the_paragraph_does_not_untie_them() {
    // A float lies in a flow of its own: the heading still moves with the
    // paragraph.
    let (mut heading, mut paragraph_id) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:85px");
        let wrapper = block(doc, body, "display:block");
        heading = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
        block(doc, wrapper, "float:right;width:10px;height:10px");
        paragraph_id = paragraph(doc, wrapper, "", 6);
    });
    assert_eq!(page_flow_y(&doc, heading), 100.0);
    assert_eq!(page_flow_y(&doc, paragraph_id), 110.0);
    // A forced break before the paragraph wins over the avoided one.
    assert_eq!(
        heading_and_paragraph_in_wrapper(85, "", "break-before:page", 6),
        (85.0, 100.0)
    );
}

#[test]
fn a_wrapper_after_a_paragraph_moves_for_the_heading_at_its_start() {
    // The lines before the wrapper are a place to break from: the page breaks
    // between the two, whether the wrapper is in the body or in a block of its
    // own.
    let positions = |spacer, in_block| {
        let (mut heading, mut paragraph_id) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{spacer}px"));
            let parent = if in_block {
                block(doc, body, "display:block")
            } else {
                body
            };
            paragraph(doc, parent, "", 1);
            let wrapper = block(doc, parent, "display:block");
            heading = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
            paragraph_id = paragraph(doc, wrapper, "", 6);
        });
        [heading, paragraph_id].map(|id| page_flow_y(&doc, id))
    };
    for in_block in [false, true] {
        // One line fits after the heading: it moves with the paragraph.
        assert_eq!(positions(70, in_block), [100.0, 110.0], "{in_block}");
        // Two lines fit.
        assert_eq!(positions(60, in_block), [70.0, 80.0], "{in_block}");
    }
}

#[test]
fn a_heading_in_a_wrapper_with_closing_edges_moves_once() {
    // The wrapper ends its parent with padding and border below the
    // paragraph, which keep the heading and the paragraph together as well.
    let positions = |spacer, lines| {
        let (mut heading, mut paragraph_id) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{spacer}px"));
            let parent = block(doc, body, "display:block");
            let wrapper = block(
                doc,
                parent,
                "display:block;padding-bottom:10px;border-bottom:2px solid black",
            );
            heading = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
            paragraph_id = paragraph(doc, wrapper, "", lines);
        });
        [heading, paragraph_id].map(|id| page_flow_y(&doc, id))
    };
    // Two lines fit after the heading: nothing moves.
    assert_eq!(positions(65, 5), [65.0, 75.0]);
    assert_eq!(positions(65, 6), [65.0, 75.0]);
    // One line fits: the heading moves with the paragraph.
    assert_eq!(positions(75, 5), [100.0, 110.0]);
    assert_eq!(positions(75, 6), [100.0, 110.0]);
}

#[test]
fn a_document_that_names_pages_ties_a_heading_to_the_first_box_of_a_wrapper_only() {
    // The wrapper holds a one-line paragraph that avoids a break after it, then
    // a long paragraph. Without named pages the heading moves with the first
    // lines of the long paragraph too; named pages break the flow between
    // boxes of their own, so the heading is tied to the first paragraph alone.
    let positions = |names_pages| {
        let (mut heading, mut first) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, "display:block;height:75px");
            heading = block(doc, body, "display:block;height:10px;break-after:avoid");
            let wrapper = block(doc, body, "display:block");
            first = paragraph(doc, wrapper, "break-after:avoid", 1);
            paragraph(doc, wrapper, "", 6);
            if names_pages {
                block(doc, body, "display:block;height:10px;page:chapter");
            }
        });
        [heading, first].map(|id| page_flow_y(&doc, id))
    };
    assert_eq!(positions(false), [100.0, 110.0]);
    assert_eq!(positions(true), [75.0, 85.0]);
}

#[test]
fn a_wrapper_that_starts_a_run_and_holds_one_moves_with_the_longer_of_the_two() {
    // The wrapper avoids a break after it, so it and the box after it move
    // together; it also starts with a heading tied to its paragraph. The run
    // of the wrapper and the box is the longer: it crosses the page edge where
    // the heading and the first lines of the paragraph do not.
    let (mut wrapper, mut heading, mut after) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:50px");
        let outer = block(doc, body, "display:block");
        block(doc, outer, "display:block;height:10px");
        wrapper = block(doc, outer, "display:block;break-after:avoid");
        heading = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
        paragraph(doc, wrapper, "", 6);
        after = block(doc, outer, "display:block;height:10px");
    });
    assert_eq!(
        [wrapper, heading, after].map(|id| page_flow_y(&doc, id)),
        [100.0, 100.0, 170.0]
    );
}

#[test]
fn a_wrapper_that_clips_its_content_does_not_move_for_the_run_it_hides() {
    // The paragraph after the heading lies past the clip of the wrapper: it is
    // not drawn, so it cannot take the heading to the next page. The wrapper
    // sits in the body or in a block of its own.
    for in_block in [false, true] {
        let positions = |wrapper_css: &str| {
            let (mut wrapper, mut heading) = (0, 0);
            let (doc, _) = paged_body_with_ifc(|doc, body| {
                block(doc, body, "display:block;height:85px");
                let parent = if in_block {
                    block(doc, body, "display:block")
                } else {
                    body
                };
                wrapper = block(doc, parent, &format!("display:block;{wrapper_css}"));
                heading = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
                paragraph(doc, wrapper, "", 6);
            });
            [wrapper, heading].map(|id| page_flow_y(&doc, id))
        };
        assert_eq!(
            positions("height:10px;overflow:hidden"),
            [85.0, 85.0],
            "{in_block}"
        );
        // A clip below the end of the run leaves it whole: it moves.
        assert_eq!(
            positions("height:80px;overflow:hidden"),
            [100.0, 100.0],
            "{in_block}"
        );
        // Without a clip the paragraph is drawn past the wrapper.
        assert_eq!(positions("height:80px"), [100.0, 100.0], "{in_block}");
    }
}

#[test]
fn a_heading_in_a_wrapper_moves_with_the_first_row_of_the_table_after_it() {
    // The row is the part of the table the page cannot break away from the
    // heading, as it is when the heading and the table lie in the body.
    for (spacer, expected) in [
        (70, (70.0, 80.0)),
        (80, (80.0, 90.0)),
        (85, (100.0, 110.0)),
        (88, (100.0, 110.0)),
    ] {
        assert_eq!(
            heading_in_wrapper_then(spacer, "", |doc, wrapper| table_of_rows(
                doc, wrapper, "", 1, ""
            )),
            expected,
            "{spacer}"
        );
    }
    // Rows after the first one are free to go to the next page.
    assert_eq!(
        heading_in_wrapper_then(80, "", |doc, wrapper| table_of_rows(
            doc, wrapper, "", 2, ""
        )),
        (80.0, 90.0)
    );
    // A tall first row that fits the next page starts it.
    assert_eq!(
        heading_in_wrapper_then(70, "", |doc, wrapper| table_of_rows(
            doc,
            wrapper,
            "",
            1,
            "height:50px"
        )),
        (100.0, 110.0)
    );
}

#[test]
fn a_heading_in_a_wrapper_moves_with_a_table_that_the_page_cannot_break_inside() {
    assert_eq!(
        heading_in_wrapper_then(70, "", |doc, wrapper| table_of_rows(
            doc,
            wrapper,
            "break-inside:avoid",
            3,
            ""
        )),
        (100.0, 110.0)
    );
    // The first row alone fits after the heading, but its group moves whole.
    assert_eq!(
        heading_in_wrapper_then(70, "", |doc, wrapper| {
            let table = collapsed_table(doc, wrapper, "");
            let group = block(doc, table, "display:table-row-group;break-inside:avoid");
            let first = row_with_cell(doc, group, "");
            row_with_cell(doc, group, "");
            row_with_cell(doc, group, "");
            first
        }),
        (100.0, 110.0)
    );
    // A forced break between the rows splits the group: the heading goes with
    // the rows before it, and these fit after the heading.
    assert_eq!(
        heading_in_wrapper_then(70, "", |doc, wrapper| {
            let table = collapsed_table(doc, wrapper, "");
            let group = block(doc, table, "display:table-row-group;break-inside:avoid");
            let first = row_with_cell(doc, group, "");
            row_with_cell_in(doc, group, "break-before:page", "");
            row_with_cell(doc, group, "");
            first
        }),
        (70.0, 80.0)
    );
    assert_eq!(
        heading_in_wrapper_then(70, "", |doc, wrapper| {
            let table = collapsed_table(doc, wrapper, "");
            let group = block(doc, table, "display:table-row-group;break-inside:avoid");
            let first = row_with_cell_in(doc, group, "break-after:page", "");
            row_with_cell(doc, group, "");
            row_with_cell(doc, group, "");
            first
        }),
        (70.0, 80.0)
    );
}

#[test]
fn a_heading_in_a_wrapper_moves_with_the_first_row_of_a_table_in_the_wrapper_after_it() {
    // The wrapper starts with the table: the heading is tied to its first row.
    let positions = |spacer, rows| {
        let (mut heading, mut cell) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{spacer}px"));
            heading = block(doc, body, "display:block;height:10px;break-after:avoid");
            let wrapper = block(doc, body, "display:block");
            cell = table_of_rows(doc, wrapper, "", rows, "");
        });
        [heading, cell].map(|id| page_flow_y(&doc, id))
    };
    assert_eq!(positions(85, 2), [100.0, 110.0]);
    assert_eq!(positions(80, 2), [80.0, 90.0]);
}

#[test]
fn a_heading_among_the_boxes_of_a_block_moves_with_the_first_row_of_the_table_after_it() {
    let positions = |spacer| {
        let (mut heading, mut cell) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{spacer}px"));
            let outer = block(doc, body, "display:block");
            block(doc, outer, "display:block;height:10px");
            heading = block(doc, outer, "display:block;height:10px;break-after:avoid");
            cell = table_of_rows(doc, outer, "", 2, "");
        });
        [heading, cell].map(|id| page_flow_y(&doc, id))
    };
    // The first row fits after the heading.
    assert_eq!(positions(65), [75.0, 85.0]);
    // The first row would cross the edge: the heading goes with it, and the
    // box before the heading stays.
    assert_eq!(positions(75), [100.0, 110.0]);
}

#[test]
fn a_heading_in_a_wrapper_stays_before_a_table_that_does_not_start_with_its_first_row() {
    // A caption above the grid comes before the first row.
    assert_eq!(
        heading_in_wrapper_then(85, "", |doc, wrapper| {
            let table = collapsed_table(doc, wrapper, "");
            block(doc, table, "display:table-caption;height:5px");
            row_with_cell(doc, table, "")
        }),
        (85.0, 100.0)
    );
    // Nor does the first row of a header group repeated on every page.
    assert_eq!(
        heading_in_wrapper_then(85, "", |doc, wrapper| {
            let table = collapsed_table(doc, wrapper, "");
            let header = block(doc, table, "display:table-header-group");
            row_with_cell(doc, header, "");
            let group = block(doc, table, "display:table-row-group");
            let first = row_with_cell(doc, group, "");
            row_with_cell(doc, group, "");
            first
        }),
        (85.0, 110.0)
    );
    // The forced break sends the row to the next page whatever the heading
    // does, so the heading has nothing to move with.
    assert_eq!(
        heading_in_wrapper_then(85, "", |doc, wrapper| {
            let table = collapsed_table(doc, wrapper, "");
            row_with_cell_in(doc, table, "break-before:page", "")
        }),
        (85.0, 100.0)
    );
}

#[test]
fn a_heading_in_a_wrapper_that_is_not_a_plain_block_is_left_as_before() {
    // The items of a row flex container sit side by side: no heading there
    // ties to the box after it.
    let (mut heading, mut paragraph_id) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:85px");
        let wrapper = block(doc, body, "display:flex");
        heading = block(doc, wrapper, "display:block;height:10px;break-after:avoid");
        paragraph_id = paragraph(doc, wrapper, "", 6);
    });
    assert_eq!(page_flow_y(&doc, heading), 85.0);
    let _ = paragraph_id;
}
