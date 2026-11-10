use super::*;

#[test]
fn body_atomic_line_crossing_a_page_edge_moves_once_with_its_text() {
    for text in [true, false] {
        let mut atomic = 0;
        let (doc, pages) = paged_body_with_ifc(|doc, body| {
            doc.set_element_attribute(
                body,
                "style",
                "display:block;margin:0;width:100px;font:10px/10px Ahem",
            )
            .unwrap();
            doc.append_element(Some(body), "div", Some("display:block;height:50px"));
            if text {
                doc.append_text(body, "aa");
            }
            atomic = doc.append_element(
                Some(body),
                "span",
                Some("display:inline-block;width:30px;height:60px"),
            );
            if text {
                doc.append_text(body, "bb");
            }
        });
        assert_eq!(page_flow_y(&doc, atomic), 100.0, "text={text}");
        assert_eq!(pages, 2, "text={text}");
    }
}

#[test]
fn body_atomic_line_matches_chromium_print_placement() {
    use raikiri_traits::PageBox;

    // RAIKIRI-1383: Chromium's 800x600 print output puts the whole atomic
    // on page two, while the preceding block remains on page one.
    for display in ["inline-block", "inline-flex", "inline"] {
        for text in [false, true] {
            let mut doc = Document::new();
            with_ahem(&mut doc);
            let html = doc.append_element(Some(0), "html", Some("display:block"));
            let body = doc.append_element(
                Some(html),
                "body",
                Some("display:block;margin:8px;font:10px/10px Ahem"),
            );
            let before = doc.append_element(Some(body), "div", Some("display:block;height:300px"));
            let prefix = text.then(|| doc.append_text(body, "aa"));
            let atomic = doc.append_element(
                Some(body),
                if display == "inline" { "img" } else { "span" },
                Some(format!("display:{display};width:300px;height:400px")),
            );
            let suffix = text.then(|| doc.append_text(body, "bb"));
            doc.mark_in_document_flags();
            let styles =
                raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
            let mut page = PageBox::new();
            page.width = 800.0;
            page.height = 600.0;
            let pages = layout_pages(&mut doc, &styles, page).unwrap();
            assert_eq!(page_flow_y(&doc, before), 8.0);
            assert_eq!(page_flow_y(&doc, atomic), 600.0, "{display}, text={text}");
            assert_eq!(doc.nodes[atomic].layout.size.height, 400.0);
            assert_eq!(pages.len(), 2);
            for source in prefix.into_iter().chain(suffix) {
                let owned = doc.ifc_text_lines(source).unwrap();
                let top = page_flow_y(&doc, owned.root) + owned.lines[0].top;
                assert_eq!(top, 600.0);
            }
        }
    }
}

#[test]
fn forced_break_before_unequal_body_atomics_keeps_the_tallest_on_the_next_page() {
    let (mut short, mut tall) = (0, 0);
    let (doc, pages) = paged_body(|doc, body| {
        block(doc, body, "float:left;width:40px;height:80px");
        block(doc, body, "display:block;height:10px;break-after:page");
        short = block(doc, body, "display:inline-block;width:20px;height:10px");
        tall = block(doc, body, "display:inline-block;width:20px;height:20px");
    });
    assert_eq!(
        [short, tall].map(|id| page_flow_y(&doc, id)),
        [110.0, 100.0]
    );
    assert_eq!(pages, 2);
}

#[test]
fn generated_suffix_moves_with_a_forced_body_atomic_line() {
    let (mut atomic, mut root) = (0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        root = body;
        doc.set_element_attribute(
            body,
            "style",
            "display:block;margin:0;width:100px;font:10px/10px Ahem",
        )
        .unwrap();
        let style = doc.append_element(Some(0), "style", None::<&str>);
        doc.append_text(style, "body::after { content:'M' }");
        block(doc, body, "display:block;height:60px;break-after:page");
        atomic = block(doc, body, "display:inline-block;width:20px;height:20px");
    });
    let lines = doc
        .ifc_layout_node(root)
        .unwrap()
        .ifc
        .as_ref()
        .unwrap()
        .lines
        .as_ref()
        .unwrap();
    assert!(
        lines.lines[0]
            .fragments()
            .any(|fragment| matches!(fragment, shodo::Fragment::GlyphRun(_)))
    );
    assert_eq!(lines.line_top(0), 100.0);
    assert_eq!(page_flow_y(&doc, atomic), 100.0);
    assert_eq!(pages, 2);
}

#[test]
fn relative_body_atomics_do_not_change_the_forced_line_shift() {
    for inset in [-10, 10] {
        let (mut atomic, mut after) = (0, 0);
        let (doc, pages) = paged_body_with_ifc(|doc, body| {
            doc.set_element_attribute(
                body,
                "style",
                "display:block;margin:0;width:100px;font:10px/10px Ahem",
            )
            .unwrap();
            block(doc, body, "display:block;height:60px;break-after:page");
            atomic = block(
                doc,
                body,
                &format!(
                    "display:inline-block;width:20px;height:20px;position:relative;top:{inset}px"
                ),
            );
            after = block(doc, body, "display:block;height:10px");
        });
        assert_eq!(
            page_flow_y(&doc, atomic),
            100.0 + inset as f32,
            "top:{inset}px"
        );
        assert_eq!(page_flow_y(&doc, after), 122.0, "top:{inset}px");
        assert_eq!(pages, 2);
    }
}

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
fn a_grid_row_moves_whole_while_one_item_has_its_first_line_on_the_page() {
    // The first item is pushed down by its padding, the second one is not: the
    // items of the row are cut together, below the border that stays.
    assert_eq!(
        container_after(80, GRID, &["padding-top:30px", ""], 1),
        (80.0, 100.0, 2)
    );
}

#[test]
fn a_flex_line_moves_whole_while_one_item_has_its_first_line_on_the_page() {
    // The first item is pushed down by its padding, the second one is not: the
    // items of the line are cut together, and the first one cannot be cut
    // before its line.
    assert_eq!(
        container_after(80, FLEX, &["padding-top:30px", ""], 1),
        (100.0, 106.0, 2)
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
    // Nothing of the container stays behind alone, and the items sit one above
    // the other or break in turn.
    let (container, _, _) = container_after(94, "display:flex;flex-direction:column", &["", ""], 1);
    assert_eq!(container, 94.0);
}

#[test]
fn a_grid_without_edges_above_its_items_moves_with_its_first_row() {
    // The items share their row, which moves whole, and the grid has nothing
    // to leave behind above it.
    let (container, first, _) = container_after(
        94,
        "display:grid;grid-template-columns:30px 30px",
        &["", ""],
        1,
    );
    assert_eq!((container, first), (100.0, 100.0));
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
    // A block between lines, a float, a table and a positioned box:
    // no piece of their content is known to keep together.
    let unread: [(&str, ItemBuilder); 4] = [
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

/// `body > [div(lead), container(container_css) > items]` on 100px pages, each
/// item a block styled with its entry of `item_css`. Returns the page-flow y
/// of the container and of its items.
fn row_after(lead: u32, container_css: &str, item_css: &[&str]) -> (f32, Vec<f32>) {
    let mut container = 0;
    let mut items = Vec::new();
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, &format!("display:block;height:{lead}px"));
        container = block(doc, body, container_css);
        for css in item_css {
            items.push(block(doc, container, &format!("display:block;{css}")));
        }
    });
    (
        page_flow_y(&doc, container),
        items.iter().map(|&item| page_flow_y(&doc, item)).collect(),
    )
}

const AVOID_20: &str = "height:20px;break-inside:avoid";
const AVOID_40: &str = "height:40px;break-inside:avoid";
const AVOID_50: &str = "height:50px;break-inside:avoid";

#[test]
fn a_flex_line_moves_whole_for_an_item_that_cannot_be_cut_and_does_not_fit() {
    // Whichever item that is, and with or without padding above the items:
    // they all start at the top of the line, and the line cannot be cut
    // before the item that does not fit.
    let padded = "display:flex;padding-top:6px";
    let bare = "display:flex";
    for (css, items, expected) in [
        (padded, ["height:20px", AVOID_40], (100.0, [106.0, 106.0])),
        (padded, [AVOID_40, "height:20px"], (100.0, [106.0, 106.0])),
        (padded, [AVOID_40, AVOID_40], (100.0, [106.0, 106.0])),
        (bare, ["height:20px", AVOID_50], (100.0, [100.0, 100.0])),
        (bare, [AVOID_50, "height:20px"], (100.0, [100.0, 100.0])),
        (bare, [AVOID_20, AVOID_50], (100.0, [100.0, 100.0])),
        (
            "display:flex;flex-direction:row-reverse",
            ["height:20px", AVOID_50],
            (100.0, [100.0, 100.0]),
        ),
    ] {
        let (container, placed) = row_after(60, css, &items);
        assert_eq!(
            (container, placed.as_slice()),
            (expected.0, &expected.1[..]),
            "{css} {items:?}"
        );
    }
}

#[test]
fn a_flex_line_stays_when_its_items_fit_or_can_be_cut() {
    for (css, items) in [
        // The item ends at the page end.
        ("display:flex", ["height:20px", AVOID_40]),
        (
            "display:flex;padding-top:6px",
            ["height:20px", "height:30px;break-inside:avoid"],
        ),
        // Boxes with nothing in them can be cut anywhere.
        ("display:flex", ["height:20px", "height:50px"]),
        (
            "display:flex;padding-top:6px",
            ["height:20px", "height:50px"],
        ),
    ] {
        let (container, _) = row_after(60, css, &items);
        assert_eq!(container, 60.0, "{css} {items:?}");
    }
}

#[test]
fn an_item_that_starts_lower_in_the_line_is_pushed_on_alone() {
    // The line can be cut above the item, so the rest of it stays.
    for (css, items, lower) in [
        (
            "display:flex;align-items:flex-end",
            [AVOID_20, "height:50px"],
            90.0,
        ),
        (
            "display:flex",
            [
                "height:20px;margin-top:30px;break-inside:avoid",
                "height:50px",
            ],
            90.0,
        ),
    ] {
        let (container, placed) = row_after(60, css, &items);
        assert_eq!((container, placed[0]), (60.0, lower), "{css}");
    }
    // The tall item starts at the top, and the one beside it goes with it.
    let (container, placed) = row_after(
        60,
        "display:flex;align-items:center",
        &["height:20px", AVOID_50],
    );
    assert_eq!((container, placed[1]), (100.0, 100.0));
}

#[test]
fn a_flex_line_is_judged_by_the_normal_flow_place_of_its_items() {
    // The page breaks before the relative offset moves the item.
    let (container, _) = row_after(
        60,
        "display:flex",
        &[
            "height:50px;break-inside:avoid;position:relative;top:10px",
            "height:20px",
        ],
    );
    assert_eq!(container, 100.0);
    let (container, _) = row_after(
        60,
        "display:flex",
        &[
            "height:20px;break-inside:avoid;position:relative;top:50px",
            "height:20px",
        ],
    );
    assert_eq!(container, 60.0);
}

#[test]
fn a_flex_container_that_stacks_its_items_is_left_to_them() {
    let (container, _) = row_after(
        60,
        "display:flex;flex-direction:column",
        &["height:20px", AVOID_50],
    );
    assert_eq!(container, 60.0);
}

#[test]
fn a_wrapping_flex_container_moves_whole_for_an_item_of_its_top_line() {
    let wrap = "display:flex;flex-wrap:wrap;width:100px";
    // One line.
    let (container, placed) = row_after(60, wrap, &["height:20px", AVOID_50]);
    assert_eq!((container, placed), (100.0, vec![100.0, 100.0]));
    // Two lines, of which the first holds the item.
    let half = "width:50px";
    let (container, placed) = row_after(
        60,
        wrap,
        &[
            &format!("{half};{AVOID_50}"),
            &format!("{half};height:20px"),
            &format!("{half};height:20px"),
        ],
    );
    assert_eq!((container, placed), (100.0, vec![100.0, 100.0, 150.0]));
}

#[test]
fn a_wrapping_flex_container_stays_for_an_item_of_a_line_below_the_top_one() {
    // The line with the item goes on to the next page alone.
    let half = "width:50px";
    let (container, placed) = row_after(
        60,
        "display:flex;flex-wrap:wrap;width:100px",
        &[
            &format!("{half};height:20px"),
            &format!("{half};height:20px"),
            &format!("{half};{AVOID_50}"),
        ],
    );
    assert_eq!((container, placed[0]), (60.0, 60.0));
}

#[test]
fn a_flex_line_in_a_block_moves_after_the_box_before_it() {
    let (mut before, mut container, mut item) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:50px");
        let wrapper = block(doc, body, "display:block");
        before = block(doc, wrapper, "display:block;height:10px");
        container = block(doc, wrapper, "display:flex");
        block(doc, container, "display:block;height:20px");
        item = block(doc, container, &format!("display:block;{AVOID_50}"));
    });
    assert_eq!(
        [before, container, item].map(|id| page_flow_y(&doc, id)),
        [50.0, 100.0, 100.0]
    );
}

/// `body > [div(lead), wrappers[0] > wrappers[1] > ... > box(item_css)]` on
/// 100px pages, each wrapper a block styled with its entry of `wrappers`.
/// Returns the page-flow y of the wrappers and of the box.
fn wrapped_after(lead: u32, wrappers: &[&str], item_css: &str) -> Vec<f32> {
    let mut ids = Vec::new();
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, &format!("display:block;height:{lead}px"));
        let mut parent = body;
        for css in wrappers {
            parent = block(doc, parent, &format!("display:block;{css}"));
            ids.push(parent);
        }
        ids.push(block(doc, parent, &format!("display:block;{item_css}")));
    });
    ids.iter().map(|&id| page_flow_y(&doc, id)).collect()
}

const PADDED: &str = "padding-top:6px";

#[test]
fn a_block_with_padding_moves_whole_with_a_box_that_avoids_breaks_and_does_not_fit() {
    // The box runs past the page end wherever the padding ends before it: the
    // page breaks before the block, not between its edges and the box.
    for lead in [97, 90, 80] {
        assert_eq!(
            wrapped_after(lead, &[PADDED], AVOID_20),
            [100.0, 106.0],
            "{lead}"
        );
    }
    for css in [
        "border-top:6px solid",
        "display:flow-root;padding-top:6px",
        "display:list-item;padding-top:6px",
    ] {
        assert_eq!(wrapped_after(90, &[css], AVOID_20), [100.0, 106.0], "{css}");
    }
}

#[test]
fn a_block_with_padding_moves_whole_with_a_replaced_box_that_does_not_fit() {
    // No page break goes through a replaced box.
    let (mut wrapper, mut image) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:90px");
        wrapper = block(doc, body, &format!("display:block;{PADDED}"));
        image = doc.append_element(
            Some(wrapper),
            "img",
            Some("display:block;width:30px;height:20px"),
        );
    });
    assert_eq!(
        [wrapper, image].map(|id| page_flow_y(&doc, id)),
        [100.0, 106.0]
    );
}

#[test]
fn a_block_with_padding_stays_with_a_box_that_avoids_breaks_and_fits() {
    // Ends at the page end.
    assert_eq!(wrapped_after(74, &[PADDED], AVOID_20), [74.0, 80.0]);
    assert_eq!(wrapped_after(60, &[PADDED], AVOID_20), [60.0, 66.0]);
}

#[test]
fn a_block_with_padding_moves_whole_with_an_avoid_box_taller_than_the_page() {
    // The box is cut on the page it lands on whichever way the break goes, so
    // the break still goes before the block rather than between its padding
    // and the box. The box then stays below the padding and splits there.
    let tall = "height:120px;break-inside:avoid";
    assert_eq!(wrapped_after(90, &[PADDED], tall), [100.0, 106.0]);
    // One that fits a page but not the space below the padding.
    let nearly = "height:96px;break-inside:avoid";
    assert_eq!(wrapped_after(90, &[PADDED], nearly), [100.0, 106.0]);
    // Outer blocks that begin where the padded one does move with it.
    assert_eq!(
        wrapped_after(90, &["", PADDED], tall),
        [100.0, 100.0, 106.0]
    );
}

#[test]
fn a_margin_above_the_box_adds_to_the_offset_below_the_padding_of_a_moved_block() {
    let tall = "height:120px;break-inside:avoid;margin-top:10px";
    assert_eq!(wrapped_after(90, &[PADDED], tall), [100.0, 116.0]);
    // So does one on a wrapper between them, which collapses with the margin
    // of the box, and the edge of a wrapper keeps its own apart.
    let tall = "height:120px;break-inside:avoid";
    assert_eq!(
        wrapped_after(90, &[PADDED, "margin-top:10px"], tall),
        [100.0, 116.0, 116.0]
    );
    assert_eq!(
        wrapped_after(
            90,
            &[PADDED, "margin-top:10px"],
            "height:120px;break-inside:avoid;margin-top:15px"
        ),
        [100.0, 121.0, 121.0]
    );
    assert_eq!(
        wrapped_after(90, &[PADDED, "margin-top:10px;padding-top:5px"], tall),
        [100.0, 116.0, 121.0]
    );
    // A block that establishes a formatting context keeps the margin of the
    // box inside it apart from its own.
    for wrapper in [
        "display:flow-root;margin-top:10px",
        "overflow:hidden;margin-top:10px",
    ] {
        assert_eq!(
            wrapped_after(
                90,
                &[PADDED, wrapper],
                "height:120px;break-inside:avoid;margin-top:15px"
            ),
            [100.0, 116.0, 131.0],
            "{wrapper}"
        );
    }
    // A wrapper that generates no box keeps no margin or edge of its own.
    let ys = wrapped_after(90, &[PADDED, "display:contents;margin-top:10px"], tall);
    assert_eq!([ys[0], ys[2]], [100.0, 106.0]);
}

#[test]
fn a_block_with_padding_moves_for_an_empty_box_only_when_it_begins_past_the_page_end() {
    // An empty box can be cut anywhere, so it only moves the block when none
    // of it fits: the padding ends at the page end or past it.
    for lead in [97, 94] {
        assert_eq!(
            wrapped_after(lead, &[PADDED], "height:20px"),
            [100.0, 106.0],
            "{lead}"
        );
    }
    for lead in [93, 90, 80] {
        let ys = wrapped_after(lead, &[PADDED], "height:20px");
        assert_eq!(ys, [lead as f32, lead as f32 + 6.0], "{lead}");
    }
}

#[test]
fn the_blocks_around_a_padded_block_move_with_it() {
    // Outer blocks that begin where it does, with or without edges of their own.
    assert_eq!(
        wrapped_after(90, &["", PADDED], AVOID_20),
        [100.0, 100.0, 106.0]
    );
    assert_eq!(
        wrapped_after(94, &["", PADDED], "height:20px"),
        [100.0, 100.0, 106.0]
    );
    assert_eq!(
        wrapped_after(90, &[PADDED, PADDED], AVOID_20),
        [100.0, 106.0, 112.0]
    );
    assert_eq!(
        wrapped_after(90, &[PADDED, ""], AVOID_20),
        [100.0, 106.0, 106.0]
    );
    // The padding of both ends before the page end, and the empty box is cut.
    assert_eq!(
        wrapped_after(86, &[PADDED, PADDED], "height:20px"),
        [86.0, 92.0, 98.0]
    );
    assert_eq!(
        wrapped_after(90, &[PADDED, PADDED], "height:20px"),
        [100.0, 106.0, 112.0]
    );
}

#[test]
fn a_padded_block_is_judged_by_the_normal_flow_place_of_its_box() {
    // The relative offset paints the box lower, but the page breaks before it.
    assert_eq!(
        wrapped_after(
            60,
            &[PADDED],
            "height:20px;break-inside:avoid;position:relative;top:50px"
        )[0],
        60.0
    );
    // The block's own offset moves it after the break.
    assert_eq!(
        wrapped_after(90, &["padding-top:6px;position:relative;top:3px"], AVOID_20)[0],
        103.0
    );
}

#[test]
fn a_padded_block_is_left_to_the_rules_for_lines_when_a_line_decides() {
    // The line that does not fit is placed by the rules for lines, which take
    // the padding along: the lead after which the padding ends the page.
    let (mut wrapper, mut line_box) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:94px");
        wrapper = block(doc, body, &format!("display:block;{PADDED}"));
        line_box = paragraph(doc, wrapper, 1, "");
    });
    assert_eq!(
        [wrapper, line_box].map(|id| page_flow_y(&doc, id)),
        [100.0, 106.0]
    );
}

#[test]
fn a_first_forced_box_keeps_its_plain_holders_leading_edges() {
    // The forced boundary belongs to the first child's holder, including
    // when the child alone would already reach a natural page edge.
    for (lead, item) in [
        (94, "height:20px;break-before:page"),
        (90, "height:20px;break-inside:avoid;break-before:page"),
        (94, "height:20px;break-before:always"),
    ] {
        let (mut wrapper, mut item_id) = (0, 0);
        let (doc, pages) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{lead}px"));
            wrapper = block(doc, body, &format!("display:block;{PADDED}"));
            item_id = block(doc, wrapper, &format!("display:block;{item}"));
        });
        assert_eq!(
            ([wrapper, item_id].map(|id| page_flow_y(&doc, id)), pages),
            ([100.0, 106.0], 2),
            "{lead} {item}"
        );
    }
}

#[test]
fn a_padded_block_is_not_moved_for_a_box_that_avoids_breaks_around_a_forced_break() {
    // The box that avoids a break inside it splits where a box in it forces
    // one, so its height is not a piece to keep with the padding: moving the
    // block would put that box on a third page.
    for inner in [
        "height:20px;break-before:page",
        "height:20px;break-after:page",
    ] {
        let (mut wrapper, mut avoiding, mut inner_id) = (0, 0, 0);
        let (doc, pages) = paged_body_with_ifc(|doc, body| {
            block(doc, body, "display:block;height:90px");
            wrapper = block(doc, body, &format!("display:block;{PADDED}"));
            avoiding = block(doc, wrapper, "display:block;break-inside:avoid");
            inner_id = block(doc, avoiding, &format!("display:block;{inner}"));
        });
        assert_eq!(
            (
                [wrapper, avoiding, inner_id].map(|id| page_flow_y(&doc, id)),
                pages
            ),
            ([90.0, 100.0, 100.0], 2),
            "{inner}"
        );
    }
}

#[test]
fn empty_flex_items_move_with_edges_that_reach_or_cross_the_page_end() {
    for lead in [94, 97] {
        for css in [
            "display:flex;padding-top:6px",
            "display:flex;padding-top:6px;flex-direction:row-reverse",
            "display:flex;border-top:6px solid",
        ] {
            let (container, items) = row_after(lead, css, &["height:20px", "height:20px"]);
            assert_eq!(
                (container, items.as_slice()),
                (100.0, &[106.0, 106.0][..]),
                "{lead} {css}"
            );
        }
    }
}

#[test]
fn empty_flex_items_keep_cutting_when_some_height_fits_after_the_edges() {
    for lead in [80, 90] {
        for css in [
            "display:flex;padding-top:6px",
            "display:flex;padding-top:6px;flex-direction:row-reverse",
            "display:flex;border-top:6px solid",
        ] {
            let (container, items) = row_after(lead, css, &["height:20px", "height:20px"]);
            let top = lead as f32;
            assert_eq!(
                (container, items.as_slice()),
                (top, &[top + 6.0, top + 6.0][..]),
                "{lead} {css}"
            );
        }
    }
}

#[test]
fn a_single_empty_flex_item_moves_with_padding_that_ends_the_page() {
    assert_eq!(
        container_with_item(|doc, container| {
            block(doc, container, "display:block;height:20px");
        }),
        100.0
    );
}

#[test]
fn nested_empty_flex_items_keep_the_edges_only_when_the_first_piece_does_not_fit() {
    for (lead, expected) in [(94, 100.0), (97, 100.0), (90, 90.0)] {
        let (mut container, mut item) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, &format!("display:block;height:{lead}px"));
            container = block(doc, body, FLEX);
            item = block(doc, container, "display:block");
            block(doc, item, "display:block;height:20px");
        });
        assert_eq!(
            (page_flow_y(&doc, container), page_flow_y(&doc, item)),
            (expected, expected + 6.0),
            "{lead}"
        );
    }
}

fn padded_special_wrapper_after(lead: u32, wrapper_css: &str) -> [f32; 2] {
    let (mut wrapper, mut child) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, &format!("display:block;height:{lead}px"));
        wrapper = block(
            doc,
            body,
            &format!(
                "display:block;padding-top:6px;font-family:Ahem;font-size:10px;line-height:10px;{wrapper_css}"
            ),
        );
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
        doc.append_text(child, "z");
    });
    [wrapper, child].map(|id| page_flow_y(&doc, id))
}

#[test]
fn a_padded_float_moves_with_its_first_avoiding_child() {
    assert_eq!(
        padded_special_wrapper_after(90, "float:left;width:100px"),
        [100.0, 106.0]
    );
}

#[test]
fn a_padded_multicol_moves_with_its_first_avoiding_child() {
    assert_eq!(
        padded_special_wrapper_after(90, "columns:2"),
        [100.0, 106.0]
    );
}

#[test]
fn a_padded_sticky_block_moves_with_its_first_avoiding_child() {
    assert_eq!(
        padded_special_wrapper_after(90, "position:sticky;top:0"),
        [100.0, 106.0]
    );
}

#[test]
fn a_padded_inline_block_moves_with_its_first_avoiding_child() {
    assert_eq!(
        padded_special_wrapper_after(90, "display:inline-block;width:100px"),
        [100.0, 106.0]
    );
}

const SPECIAL_LEADING_WRAPPERS: [&str; 4] = [
    "float:left;width:100px",
    "columns:2",
    "position:sticky;top:0",
    "display:inline-block;width:100px",
];

#[test]
fn special_leading_wrappers_stay_at_page_start_and_when_the_first_piece_fits() {
    for css in SPECIAL_LEADING_WRAPPERS {
        for lead in [0, 60, 74] {
            assert_eq!(
                padded_special_wrapper_after(lead, css),
                [lead as f32, lead as f32 + 6.0],
                "{css}, lead={lead}"
            );
        }
    }
}

#[test]
fn special_leading_wrappers_move_when_the_first_piece_exceeds_the_page_by_one_pixel() {
    for css in SPECIAL_LEADING_WRAPPERS {
        assert_eq!(
            padded_special_wrapper_after(75, css),
            [100.0, 106.0],
            "{css}"
        );
    }
}

#[test]
fn nested_special_leading_wrappers_have_their_own_break_candidates() {
    let mut actual = Vec::new();
    for css in SPECIAL_LEADING_WRAPPERS {
        let (mut parent, mut wrapper, mut child) = (0, 0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, "display:block;height:60px");
            parent = block(doc, body, "display:block");
            block(doc, parent, "display:block;height:30px");
            wrapper = block(doc, parent, &format!("display:block;padding-top:6px;{css}"));
            child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
        });
        actual.push([parent, wrapper, child].map(|id| page_flow_y(&doc, id)));
    }
    assert_eq!(actual, vec![[60.0, 100.0, 106.0]; 4]);
}

#[test]
fn a_float_moved_with_its_first_piece_keeps_a_clearing_sibling_below_it() {
    let (mut wrapper, mut child, mut clear) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:90px");
        wrapper = block(doc, body, "float:left;width:100px;padding-top:6px");
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
        clear = block(doc, body, "display:block;clear:both;height:10px");
    });
    assert_eq!(
        [wrapper, child, clear].map(|id| page_flow_y(&doc, id)),
        [100.0, 106.0, 126.0]
    );
}

#[test]
fn an_inline_block_first_piece_moves_once_with_text_on_its_line() {
    let (mut wrapper, mut child) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:90px");
        let line = block(
            doc,
            body,
            "display:block;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(line, "x");
        wrapper = block(doc, line, "display:inline-block;width:60px;padding-top:6px");
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
        doc.append_text(child, "z");
        doc.append_text(line, "q");
    });
    assert_eq!(
        [wrapper, child].map(|id| page_flow_y(&doc, id)),
        [100.0, 106.0]
    );
}

fn special_wrapper_with_trailing_content(css: &str) -> [f32; 2] {
    let (mut wrapper, mut child) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:60px");
        wrapper = block(doc, body, &format!("display:block;padding-top:6px;{css}"));
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
        block(doc, wrapper, "display:block;height:120px");
    });
    [wrapper, child].map(|id| page_flow_y(&doc, id))
}

#[test]
fn breakable_special_wrappers_stay_when_only_their_trailing_content_overflows() {
    for css in &SPECIAL_LEADING_WRAPPERS[..3] {
        assert_eq!(
            special_wrapper_with_trailing_content(css),
            [60.0, 66.0],
            "{css}"
        );
    }
}

#[test]
fn an_inline_block_keeps_its_trailing_content_with_the_first_piece() {
    assert_eq!(
        special_wrapper_with_trailing_content("display:inline-block;width:100px"),
        [100.0, 106.0]
    );
}

#[test]
fn a_multicol_first_piece_uses_the_first_column_and_spanner_placement() {
    for first_css in ["", "column-span:all"] {
        let (mut wrapper, mut first, mut second) = (0, 0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, "display:block;height:90px");
            wrapper = block(doc, body, "display:block;columns:2;padding-top:6px");
            first = block(
                doc,
                wrapper,
                &format!("display:block;height:20px;break-inside:avoid;{first_css}"),
            );
            second = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
        });
        assert_eq!(
            [wrapper, first].map(|id| page_flow_y(&doc, id)),
            [100.0, 106.0],
            "{first_css}"
        );
        if first_css.is_empty() {
            assert_eq!(page_flow_y(&doc, second), 106.0);
            assert!(doc.nodes[second].layout.location.x > doc.nodes[first].layout.location.x);
        }
    }
}

#[test]
fn an_atomic_line_keeps_a_short_peer_with_the_padded_inline_block() {
    let (mut first, mut wrapper, mut child) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:90px");
        first = block(doc, body, "display:inline-block;width:40px;height:26px");
        wrapper = block(doc, body, "display:inline-block;width:60px;padding-top:6px");
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
    });
    assert_eq!(
        [first, wrapper, child].map(|id| page_flow_y(&doc, id)),
        [100.0, 100.0, 106.0]
    );
}

#[test]
fn a_paragraph_keeps_ownership_of_its_leading_atomic_line_and_float() {
    let (mut parent, mut float, mut wrapper, mut child) = (0, 0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:90px");
        parent = block(
            doc,
            body,
            "display:block;font-family:Ahem;font-size:10px;line-height:10px",
        );
        float = block(doc, parent, "float:left;width:10px;height:20px");
        wrapper = block(
            doc,
            parent,
            "display:inline-block;width:60px;padding-top:6px",
        );
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
    });
    assert_eq!(
        [parent, float, wrapper, child].map(|id| page_flow_y(&doc, id)),
        [100.0, 100.0, 100.0, 106.0]
    );
}

#[test]
fn a_leading_atomic_line_uses_the_shift_from_a_prior_forced_break() {
    let (mut before, mut parent, mut wrapper, mut child) = (0, 0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:60px");
        before = block(doc, body, "display:block;height:20px;break-before:page");
        parent = block(
            doc,
            body,
            "display:block;font-family:Ahem;font-size:10px;line-height:10px",
        );
        wrapper = block(
            doc,
            parent,
            "display:inline-block;width:60px;padding-top:6px",
        );
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
    });
    assert_eq!(
        [before, parent, wrapper, child].map(|id| page_flow_y(&doc, id)),
        [100.0, 120.0, 120.0, 126.0]
    );
}

#[test]
fn a_delegated_atomic_line_preserves_the_alignment_of_unequal_peers() {
    let (mut first, mut wrapper, mut child) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:90px");
        first = block(
            doc,
            body,
            "display:inline-block;vertical-align:top;width:40px;height:10px",
        );
        wrapper = block(
            doc,
            body,
            "display:inline-block;vertical-align:top;width:60px;padding-top:6px",
        );
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
    });
    assert_eq!(
        [first, wrapper, child].map(|id| page_flow_y(&doc, id)),
        [100.0, 100.0, 106.0]
    );
}

#[test]
fn a_delegated_atomic_line_moves_following_blocks_once() {
    let (mut parent, mut wrapper, mut child, mut trailing, mut after) = (0, 0, 0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:60px");
        parent = block(
            doc,
            body,
            "display:block;font-family:Ahem;font-size:10px;line-height:10px",
        );
        block(doc, parent, "display:block;height:30px");
        wrapper = block(
            doc,
            parent,
            "display:inline-block;width:60px;padding-top:6px",
        );
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
        doc.append_text(child, "z");
        trailing = block(doc, parent, "display:block;height:10px");
        after = block(doc, body, "display:block;height:10px");
    });
    assert_eq!(
        [parent, wrapper, child, trailing, after].map(|id| page_flow_y(&doc, id)),
        [60.0, 100.0, 106.0, 126.0, 136.0]
    );
}

#[test]
fn a_spanning_first_piece_replans_following_groups_and_survives_relayout() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    let (mut parent, mut wrapper, mut first, mut next, mut after) = (0, 0, 0, 0, 0);
    let (mut doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:60px");
        parent = block(doc, body, "display:block");
        block(doc, parent, "display:block;height:30px");
        wrapper = block(doc, parent, "display:block;columns:2;padding-top:6px");
        first = block(
            doc,
            wrapper,
            "display:block;column-span:all;height:20px;break-inside:avoid",
        );
        next = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
        after = block(doc, parent, "display:block;height:10px");
    });
    for _ in 0..2 {
        assert_eq!(
            [parent, wrapper, first, next, after].map(|id| page_flow_y(&doc, id)),
            [60.0, 100.0, 106.0, 126.0, 146.0]
        );
        let rules = build_rule_tree(&doc);
        let styles = cascade(&doc, &rules).unwrap();
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        layout_pages(&mut doc, &styles, page).unwrap();
    }
}

#[test]
fn a_pending_break_after_moves_a_delegated_atomic_line_together() {
    let (mut first, mut wrapper, mut child, mut suffix) = (0, 0, 0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:60px;break-after:page");
        first = block(
            doc,
            body,
            "display:inline-block;vertical-align:top;width:40px;height:10px",
        );
        wrapper = block(
            doc,
            body,
            "display:inline-block;vertical-align:top;width:60px;padding-top:6px",
        );
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
        doc.append_element(Some(body), "br", None::<&str>);
        suffix = doc.append_text(body, "q");
    });
    assert_eq!(
        [first, wrapper, child].map(|id| page_flow_y(&doc, id)),
        [100.0, 100.0, 106.0]
    );
    assert_eq!(pages, 2);
    let text = doc.ifc_text_lines(suffix).unwrap();
    let top = page_flow_y(&doc, text.root) + text.lines[0].top;
    assert!((126.0..200.0).contains(&top));
}

#[test]
fn a_delegated_atomic_line_after_text_keeps_earlier_text_on_its_page() {
    let (mut root, mut wrapper, mut child) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:80px");
        root = block(
            doc,
            body,
            "display:block;font-family:Ahem;font-size:10px;line-height:10px;orphans:1;widows:1",
        );
        doc.append_text(root, "p");
        doc.append_element(Some(root), "br", None::<&str>);
        wrapper = block(doc, root, "display:inline-block;width:30px;padding-top:6px");
        child = block(doc, wrapper, "display:block;height:20px;break-inside:avoid");
        doc.append_text(child, "z");
        doc.append_element(Some(root), "br", None::<&str>);
        doc.append_text(root, "q");
    });
    assert_eq!(
        [root, wrapper, child].map(|id| page_flow_y(&doc, id)),
        [80.0, 100.0, 106.0]
    );
    let lines = doc.nodes[root]
        .ifc
        .as_ref()
        .unwrap()
        .lines
        .as_ref()
        .unwrap();
    assert_eq!(lines.line_top(0), 0.0);
    assert_eq!(lines.line_top(1), 20.0);
    assert_eq!(lines.line_top(2), 46.0);
}

#[test]
fn a_column_fragmented_paragraph_keeps_leading_atomics_in_their_columns() {
    use raikiri_traits::PageBox;
    for columns_on_body in [false, true] {
        let mut doc = Document::new();
        crate::layout::test_support::with_ahem(&mut doc);
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let column_css =
            "columns:2;column-gap:0;height:40px;font-family:Ahem;font-size:10px;line-height:10px";
        let body_css = format!(
            "display:block;margin:0;width:100px;{}",
            if columns_on_body { column_css } else { "" }
        );
        let body = doc.append_element(Some(html), "body", Some(body_css));
        let columns = if columns_on_body {
            body
        } else {
            block(&mut doc, body, "display:block;height:60px");
            block(&mut doc, body, &format!("display:block;{column_css}"))
        };
        for text in ["z", "q"] {
            let atomic = block(
                &mut doc,
                columns,
                "display:inline-block;width:30px;padding-top:6px",
            );
            let child = block(
                &mut doc,
                atomic,
                "display:block;height:20px;break-inside:avoid",
            );
            doc.append_text(child, text);
        }
        doc.mark_in_document_flags();
        let styles = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        let pages = layout_pages(&mut doc, &styles, page).unwrap();
        assert!(
            doc.nodes[columns]
                .ifc
                .as_ref()
                .unwrap()
                .multicol_fragments
                .is_some()
        );
        assert_eq!(pages.len(), 1);
        doc.project_pages(&styles, page, &pages, &[]).unwrap();
        let runs = doc.page_text_runs(&styles, 0);
        let positions: Vec<_> = ["z", "q"]
            .map(|text| runs.iter().find(|run| run.text == text).unwrap().origin)
            .into();
        let baseline = if columns_on_body { 14.0 } else { 74.0 };
        assert_eq!(
            positions,
            [(0.0, baseline), (50.0, baseline)],
            "columns_on_body={columns_on_body}"
        );
    }
}

#[test]
fn a_delegated_tall_atomic_line_reports_a_page_limit_error() {
    use raikiri_traits::PageBox;
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px"),
    );
    block(&mut doc, body, "display:block;height:60px");
    let atomic = block(
        &mut doc,
        body,
        "display:inline-block;width:100px;padding-top:6px",
    );
    block(
        &mut doc,
        atomic,
        "display:block;height:20px;break-inside:avoid",
    );
    block(&mut doc, atomic, "display:block;height:120px");
    doc.mark_in_document_flags();
    let styles = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let control = PageLayoutControl::new(Some(2));
    let result = layout_pages_with_options(
        &mut doc,
        &styles,
        page,
        PageLayoutOptions {
            control: Some(&control),
            ..Default::default()
        },
    );
    assert!(matches!(
        result,
        Err(LayoutError::PageLimitExceeded { limit: 2, .. })
    ));
}
