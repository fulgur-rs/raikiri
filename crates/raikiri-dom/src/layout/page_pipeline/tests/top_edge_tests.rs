use super::*;

fn block(doc: &mut Document, parent: usize, css: &str) -> usize {
    doc.append_element(Some(parent), "div", Some(css))
}

/// A paragraph of `lines` lines of 10px Ahem text, 30px wide, styled with
/// `css`, in `parent`.
fn paragraph(doc: &mut Document, parent: usize, lines: usize, css: &str) -> usize {
    let paragraph = block(
        doc,
        parent,
        &format!("display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;{css}"),
    );
    doc.append_text(paragraph, ["aaa"; 8][..lines].join(" "));
    paragraph
}

/// `body > [div(lead), paragraph(edge)]` on 100px pages. Returns the page-flow
/// y of the paragraph and the page count.
fn paragraph_after(lead: u32, lines: usize, edge_css: &str) -> (f32, usize) {
    let mut paragraph_id = 0;
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        block(doc, body, &format!("display:block;height:{lead}px"));
        paragraph_id = paragraph(doc, body, lines, edge_css);
    });
    (page_flow_y(&doc, paragraph_id), pages)
}

#[test]
fn a_paragraph_whose_padding_ends_the_page_starts_the_next_one() {
    // The padding or border above the lines reaches the page end or runs
    // short of it, and the first line starts the next page: the edges would be
    // left alone at the foot of this one, so the page breaks before the
    // paragraph instead of between them and its lines.
    for (lead, edge) in [
        (97, "padding-top:6px"),
        (94, "padding-top:6px"),
        (93, "padding-top:7px"),
        (99, "border-top:1px solid"),
        (95, "border-top:2px solid;padding-top:3px"),
    ] {
        assert_eq!(paragraph_after(lead, 1, edge), (100.0, 2), "{lead} {edge}");
    }
}

#[test]
fn a_paragraph_whose_first_line_straddles_the_page_end_moves_with_its_padding() {
    // The padding fits, but the line does not: the lines move, and the padding
    // goes with them.
    for lines in [1, 2, 3] {
        assert_eq!(
            paragraph_after(90, lines, "padding-top:6px"),
            (100.0, 2),
            "{lines} lines"
        );
    }
}

#[test]
fn a_paragraph_stays_when_its_first_line_fits_after_the_padding() {
    assert_eq!(paragraph_after(80, 1, "padding-top:6px"), (80.0, 1));
}

/// `body > [div(lead), wrapper(wrapper_css) > paragraph]`. Returns the
/// page-flow y of the wrapper and of the paragraph.
fn wrapper_after(lead: u32, wrapper_css: &str) -> (f32, f32) {
    let (mut wrapper_id, mut paragraph_id) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, &format!("display:block;height:{lead}px"));
        wrapper_id = block(doc, body, &format!("display:block;{wrapper_css}"));
        paragraph_id = paragraph(doc, wrapper_id, 1, "");
    });
    (
        page_flow_y(&doc, wrapper_id),
        page_flow_y(&doc, paragraph_id),
    )
}

#[test]
fn a_block_moves_with_the_padding_or_border_above_a_paragraph_that_moves() {
    // The edges of the block that holds the paragraph are no place to break
    // at either: the page breaks before the block, whether its padding fits
    // the page (the first line then straddles the end) or reaches it.
    for lead in [97, 94, 90] {
        assert_eq!(
            wrapper_after(lead, "padding-top:6px"),
            (100.0, 106.0),
            "{lead}"
        );
        assert_eq!(
            wrapper_after(lead, "border-top:4px solid"),
            (100.0, 104.0),
            "{lead}"
        );
    }
}

#[test]
fn every_block_with_edges_above_a_paragraph_that_moves_moves_with_it() {
    let (mut outer, mut inner, mut paragraph_id) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:94px");
        outer = block(doc, body, "display:block;border-top:4px solid");
        inner = block(doc, outer, "display:block;padding-top:3px");
        paragraph_id = paragraph(doc, inner, 1, "");
    });
    assert_eq!(
        (
            page_flow_y(&doc, outer),
            page_flow_y(&doc, inner),
            page_flow_y(&doc, paragraph_id)
        ),
        (100.0, 104.0, 107.0)
    );
}

#[test]
fn a_block_at_the_top_of_the_page_stays_when_its_paragraph_moves() {
    // The block's padding opens the page, so moving the block gains nothing:
    // the paragraph alone goes to the next page.
    let (mut wrapper_id, mut paragraph_id) = (0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        wrapper_id = block(doc, body, "display:block;padding-top:6px");
        paragraph_id = paragraph(doc, wrapper_id, 1, "margin-top:88px");
    });
    assert_eq!(
        (
            page_flow_y(&doc, wrapper_id),
            page_flow_y(&doc, paragraph_id),
            pages
        ),
        (0.0, 100.0, 2)
    );
}

#[test]
fn a_block_with_content_before_the_paragraph_stays_when_the_paragraph_moves() {
    // The first box of the block fits on the page, so the break before the
    // paragraph is a break between boxes.
    let (mut wrapper_id, mut paragraph_id) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:20px");
        wrapper_id = block(doc, body, "display:block;padding-top:6px");
        block(doc, wrapper_id, "display:block;height:66px");
        paragraph_id = paragraph(doc, wrapper_id, 1, "");
    });
    assert_eq!(
        (
            page_flow_y(&doc, wrapper_id),
            page_flow_y(&doc, paragraph_id)
        ),
        (20.0, 100.0)
    );
}

#[test]
fn a_block_with_a_relative_offset_moves_with_the_offset_kept() {
    let (mut wrapper_id, mut paragraph_id) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:94px");
        wrapper_id = block(
            doc,
            body,
            "display:block;position:relative;top:2px;padding-top:6px",
        );
        paragraph_id = paragraph(doc, wrapper_id, 1, "");
    });
    assert_eq!(
        (
            page_flow_y(&doc, wrapper_id),
            page_flow_y(&doc, paragraph_id)
        ),
        (102.0, 108.0)
    );
}

#[test]
fn a_paragraph_that_keeps_its_orphans_moves_with_the_padding_above_it() {
    // Two lines have to stay before the page end, and the padding leaves room
    // for one: the paragraph moves whole, and the padding with it.
    let (mut wrapper_id, mut paragraph_id) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:84px");
        wrapper_id = block(doc, body, "display:block;padding-top:6px");
        paragraph_id = paragraph(doc, wrapper_id, 3, "orphans:2;widows:1");
    });
    assert_eq!(
        (
            page_flow_y(&doc, wrapper_id),
            page_flow_y(&doc, paragraph_id)
        ),
        (100.0, 106.0)
    );
}

#[test]
fn a_block_without_edges_around_a_block_with_them_moves_with_it() {
    // The outer block begins where the padded block does, so it goes along
    // rather than staying behind with an empty start.
    let (mut outer, mut inner, mut paragraph_id) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:94px");
        outer = block(doc, body, "display:block");
        inner = block(doc, outer, "display:block;padding-top:6px");
        paragraph_id = paragraph(doc, inner, 1, "");
    });
    assert_eq!(
        (
            page_flow_y(&doc, outer),
            page_flow_y(&doc, inner),
            page_flow_y(&doc, paragraph_id)
        ),
        (100.0, 100.0, 106.0)
    );
}

/// `body > [div(lead), container(container_css) > paragraphs]` on 100px pages,
/// one paragraph of `lines` lines styled with each of `item_css`. Returns the
/// page-flow y of the container and of its first item, and the page count.
fn container_after(
    lead: u32,
    container_css: &str,
    item_css: &[&str],
    lines: usize,
) -> (f32, f32, usize) {
    let (mut container, mut first) = (0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        block(doc, body, &format!("display:block;height:{lead}px"));
        container = block(doc, body, container_css);
        for (index, css) in item_css.iter().enumerate() {
            let item = paragraph(doc, container, lines, css);
            if index == 0 {
                first = item;
            }
        }
    });
    (
        page_flow_y(&doc, container),
        page_flow_y(&doc, first),
        pages,
    )
}

const FLEX: &str = "display:flex;padding-top:6px";
const GRID: &str = "display:grid;grid-template-columns:30px 30px;border-top:6px solid";

#[test]
fn a_container_whose_first_items_do_not_fit_after_its_padding_moves_whole() {
    // The padding fits and the lines of the items do not, or the padding
    // ends the page: the page breaks before the container, not between its
    // edges and the items.
    for lead in [97, 94, 90] {
        for css in [FLEX, GRID] {
            assert_eq!(
                container_after(lead, css, &["", ""], 1),
                (100.0, 106.0, 2),
                "{lead} {css}"
            );
        }
    }
}

#[test]
fn a_container_stays_when_the_first_line_of_an_item_fits_after_its_padding() {
    for css in [FLEX, GRID] {
        assert_eq!(
            container_after(80, css, &["", ""], 1),
            (80.0, 86.0, 1),
            "{css}"
        );
    }
}

#[test]
fn a_container_moves_when_the_orphans_of_every_item_do_not_fit() {
    // Two lines have to stay before the page end, and only one fits.
    assert_eq!(
        container_after(84, FLEX, &["orphans:2", "orphans:2"], 3),
        (100.0, 106.0, 2)
    );
    assert_eq!(
        container_after(84, FLEX, &["orphans:1", "orphans:1"], 3),
        (84.0, 90.0, 2)
    );
}

#[test]
fn a_container_stays_while_one_item_has_its_first_line_on_the_page() {
    // The first item is pushed down by its padding, the second one is not.
    assert_eq!(
        container_after(80, FLEX, &["padding-top:30px", ""], 1),
        (80.0, 86.0, 2)
    );
}

#[test]
fn a_container_stays_when_the_next_page_cannot_hold_the_first_piece() {
    // An item that avoids breaks inside it and is taller than a page would
    // cross a page edge wherever it went.
    let (container, _, _) = container_after(94, FLEX, &["break-inside:avoid;height:120px"], 1);
    assert_eq!(container, 94.0);
}

#[test]
fn a_container_without_padding_or_border_above_its_items_is_left_to_its_items() {
    let (container, _, _) = container_after(94, "display:flex", &["", ""], 1);
    assert_eq!(container, 94.0);
}

#[test]
fn a_container_whose_break_before_is_avoided_stays() {
    let (container, _, _) = container_after(94, &format!("{FLEX};break-before:avoid"), &[""], 1);
    assert_eq!(container, 94.0);
}

#[test]
fn a_container_moves_for_the_first_line_of_a_block_in_its_item() {
    let (mut container, mut item) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:94px");
        container = block(doc, body, FLEX);
        item = block(doc, container, "display:block");
        let inner = block(doc, item, "display:block;padding-top:2px");
        paragraph(doc, inner, 1, "");
    });
    assert_eq!(
        (page_flow_y(&doc, container), page_flow_y(&doc, item)),
        (100.0, 106.0)
    );
}

/// Adds the one item of a container given its id.
type ItemBuilder = Box<dyn FnOnce(&mut Document, usize)>;

/// A flex container with padding on top at 94px, whose one item is built by
/// `build_item`. Returns the page-flow y of the container.
fn container_with_item(build_item: impl FnOnce(&mut Document, usize)) -> f32 {
    let mut container = 0;
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:94px");
        container = block(doc, body, FLEX);
        build_item(doc, container);
    });
    page_flow_y(&doc, container)
}

#[test]
fn a_container_is_left_alone_when_its_first_piece_cannot_be_read() {
    // A block between lines, a float, a table, a positioned box and a box with
    // nothing in it: no piece of their content is known to keep together.
    let unread: [(&str, ItemBuilder); 5] = [
        (
            "block between lines",
            Box::new(|doc, container| {
                let item = paragraph(doc, container, 1, "");
                block(doc, item, "display:block;height:10px");
                doc.append_text(item, "aaa");
            }),
        ),
        (
            "float",
            Box::new(|doc, container| {
                let item = block(doc, container, "display:block");
                block(doc, item, "float:left;width:10px;height:10px");
                paragraph(doc, item, 1, "");
            }),
        ),
        (
            "table",
            Box::new(|doc, container| {
                let table = block(doc, container, "display:table");
                let row = block(doc, table, "display:table-row");
                let cell = block(doc, row, "display:table-cell");
                paragraph(doc, cell, 1, "");
            }),
        ),
        (
            "positioned",
            Box::new(|doc, container| {
                paragraph(doc, container, 1, "position:sticky;top:0");
            }),
        ),
        (
            "empty",
            Box::new(|doc, container| {
                block(doc, container, "display:block;height:20px");
            }),
        ),
    ];
    for (name, build) in unread {
        assert_eq!(container_with_item(build), 94.0, "{name}");
    }
}

#[test]
fn a_container_moves_for_the_line_of_text_it_holds_directly() {
    // The text is an item of its own, which the rules for lines place.
    assert_eq!(
        container_with_item(|doc, container| {
            doc.append_text(container, "aaa");
        }),
        100.0
    );
}

#[test]
fn a_block_stays_when_a_box_before_the_moved_text_fills_the_page() {
    // The padded block holds a box that ends the page and text after it. The
    // text moves on its own: the box and the padding above it stay.
    let (mut holder, mut first) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:86px");
        holder = block(
            doc,
            body,
            "display:block;padding-top:4px;width:100px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        first = block(doc, holder, "display:block;height:10px");
        doc.append_text(holder, "bbb bbb bbb bbb bbb bbb");
    });
    assert_eq!(
        (page_flow_y(&doc, holder), page_flow_y(&doc, first)),
        (86.0, 90.0)
    );
}

#[test]
fn a_paragraph_moved_with_a_negative_relative_offset_keeps_the_offset() {
    let (mut wrapper_id, mut paragraph_id) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:94px");
        wrapper_id = block(doc, body, "display:block;padding-top:6px");
        paragraph_id = paragraph(doc, wrapper_id, 1, "position:relative;top:-3px");
    });
    assert_eq!(
        (
            page_flow_y(&doc, wrapper_id),
            page_flow_y(&doc, paragraph_id)
        ),
        (100.0, 103.0)
    );
}

#[test]
fn a_paragraph_that_a_relative_offset_paints_on_a_later_page_stays_in_the_flow() {
    // The offset only paints the paragraph on the second page; the flow puts it
    // on the first, where its line fits, so it is not moved for its place.
    let (mut paragraph_id, mut after) = (0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:20px");
        paragraph_id = paragraph(
            doc,
            body,
            1,
            "position:relative;top:100px;height:20px;break-after:page",
        );
        after = block(doc, body, "display:block;height:20px");
    });
    assert_eq!(
        (
            pages,
            page_flow_y(&doc, paragraph_id),
            page_flow_y(&doc, after)
        ),
        (2, 120.0, 100.0)
    );
}

#[test]
fn a_block_at_the_top_of_the_page_with_a_relative_offset_stays_when_its_paragraph_moves() {
    // The offset only paints the block 2px below the page top; in the flow it
    // starts the page, so moving it gains nothing.
    let (mut wrapper_id, mut paragraph_id) = (0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        wrapper_id = block(
            doc,
            body,
            "display:block;position:relative;top:2px;padding-top:94px",
        );
        paragraph_id = paragraph(doc, wrapper_id, 1, "");
    });
    assert_eq!(
        (
            page_flow_y(&doc, wrapper_id),
            page_flow_y(&doc, paragraph_id),
            pages
        ),
        (2.0, 102.0, 2)
    );
}

#[test]
fn a_container_is_judged_by_the_normal_flow_place_of_its_items() {
    // The relative offset paints the item 30px lower; its line ends at 96 in
    // the flow, where it fits the page, so the container stays.
    assert_eq!(
        container_after(80, FLEX, &["position:relative;top:30px"], 1),
        (80.0, 116.0, 2)
    );
}

#[test]
fn a_paragraph_moves_even_when_its_padding_and_line_do_not_fit_one_page() {
    // The padding above the lines runs over a page, and the page still breaks
    // before the paragraph rather than between that padding and its line, as
    // Chromium does with 250px of padding on pages of 200px: the paragraph
    // starts the second page and its lines land on a third.
    for lines in [1, 3] {
        for edge in [
            "padding-top:110px",
            "border-top:110px solid",
            "border-top:50px solid;padding-top:60px",
        ] {
            assert_eq!(
                paragraph_after(50, lines, edge),
                (100.0, 3),
                "{lines} lines, {edge}"
            );
        }
    }
    // Padding that ends the second page, or a line short of it, sends the line
    // alone to the third page.
    for edge in ["padding-top:95px", "padding-top:100px", "padding-top:105px"] {
        assert_eq!(paragraph_after(50, 1, edge), (100.0, 3), "{edge}");
    }
    // A paragraph that avoids a break inside it moves too, and so does one
    // that starts the second page when the pages before it are full.
    assert_eq!(
        paragraph_after(50, 1, "padding-top:110px;break-inside:avoid"),
        (100.0, 3)
    );
    assert_eq!(paragraph_after(150, 1, "padding-top:110px"), (200.0, 4));
}

#[test]
fn a_paragraph_at_the_top_of_the_page_stays_when_its_padding_runs_over_a_page() {
    // Nothing before the paragraph is there to break from: the padding spans
    // the pages, and the line starts the second one.
    assert_eq!(paragraph_after(0, 1, "padding-top:110px"), (0.0, 2));
}

#[test]
fn a_paragraph_whose_margin_is_all_that_comes_before_it_moves_with_its_padding() {
    // The margin above the paragraph is not part of the page: the padding
    // starts below its top, and Chromium sends the paragraph to the next page
    // as it does after a box.
    let mut paragraph_id = 0;
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        paragraph_id = paragraph(doc, body, 1, "margin-top:5px;padding-top:110px");
    });
    assert_eq!((page_flow_y(&doc, paragraph_id), pages), (100.0, 3));
}

#[test]
fn a_paragraph_in_a_wrapper_moves_with_the_wrapper_when_its_padding_runs_over_a_page() {
    // The wrapper has padding below the paragraph, and the paragraph's own
    // padding above its line runs over a page: the page breaks before the
    // paragraph, which starts the wrapper's content.
    let (mut wrapper_id, mut paragraph_id) = (0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:50px");
        wrapper_id = block(doc, body, "display:block;padding-bottom:15px");
        paragraph_id = paragraph(doc, wrapper_id, 1, "padding-top:110px");
    });
    assert_eq!(
        (
            page_flow_y(&doc, wrapper_id),
            page_flow_y(&doc, paragraph_id),
            pages
        ),
        (100.0, 100.0, 3)
    );
}

#[test]
fn a_block_moves_even_when_its_padding_and_line_do_not_fit_one_page() {
    // The wrapper's padding runs over a page, and the page still breaks before
    // the wrapper rather than between that padding and its line, as Chromium
    // does: the wrapper starts the second page and its line lands on a third.
    let (mut wrapper_id, mut paragraph_id) = (0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:50px");
        wrapper_id = block(doc, body, "display:block;padding-top:110px");
        paragraph_id = paragraph(doc, wrapper_id, 1, "");
    });
    assert_eq!(
        (
            page_flow_y(&doc, wrapper_id),
            page_flow_y(&doc, paragraph_id),
            pages
        ),
        (100.0, 210.0, 3)
    );
}
