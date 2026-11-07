use super::*;

#[derive(Clone, Copy, Debug)]
enum Tail {
    Footer,
    Header,
    BottomCaption,
    FooterCaption,
    HeaderCaption,
    FooterTwoAuto,
    DirectFooterTwoAuto,
    DirectFooterTwoAvoid,
    FooterTwoAvoid,
    MultipleCaptions,
    HeaderFirstBody,
}

fn printed_table(
    tail: Tail,
    padding: u32,
    reverse_source: bool,
) -> (Document, usize, usize, Vec<usize>, usize, usize) {
    printed_table_with_followers(tail, padding, reverse_source, 0, 10, None)
}

fn printed_table_with_followers(
    tail: Tail,
    padding: u32,
    reverse_source: bool,
    wrapper_depth: u32,
    last_body_height: u32,
    following_height: Option<u32>,
) -> (Document, usize, usize, Vec<usize>, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px;font-family:Ahem;font-size:8px;line-height:10px"),
    );
    let holder = if matches!(tail, Tail::DirectFooterTwoAuto | Tail::DirectFooterTwoAvoid) {
        body
    } else {
        doc.append_element(
            Some(body),
            "div",
            Some(format!("display:block;padding-bottom:{padding}px")),
        )
    };
    let mut content = holder;
    for _ in 0..wrapper_depth {
        content = doc.append_element(Some(content), "div", Some("display:block"));
    }
    let before_table = if matches!(
        tail,
        Tail::FooterTwoAuto
            | Tail::DirectFooterTwoAuto
            | Tail::DirectFooterTwoAvoid
            | Tail::FooterTwoAvoid
    ) {
        50
    } else if matches!(tail, Tail::HeaderFirstBody) {
        95
    } else {
        60
    };
    for (height, text) in [(100, "A"), (before_table, "B")] {
        let block = doc.append_element(
            Some(content),
            "div",
            Some(format!("display:block;height:{height}px")),
        );
        doc.append_text(block, text);
    }
    let table = doc.append_element(
        Some(content),
        "table",
        Some(if matches!(tail, Tail::DirectFooterTwoAuto | Tail::DirectFooterTwoAvoid) {
            format!(
                "display:table;border-spacing:0;border-collapse:separate;padding-bottom:{padding}px"
            )
        } else {
            "display:table;border-spacing:0;border-collapse:collapse".to_owned()
        }),
    );
    let row = |doc: &mut Document, parent, text, height| {
        let row_css = if height > 10 {
            "display:table-row;break-inside:avoid"
        } else {
            "display:table-row"
        };
        let row = doc.append_element(Some(parent), "tr", Some(row_css));
        let cell = doc.append_element(
            Some(row),
            "td",
            Some(format!("display:table-cell;padding:0;height:{height}px")),
        );
        doc.append_text(cell, text);
        row
    };
    let mut rows = Vec::new();
    if matches!(
        tail,
        Tail::Header | Tail::HeaderCaption | Tail::HeaderFirstBody
    ) {
        let header = doc.append_element(Some(table), "thead", Some("display:table-header-group"));
        rows.push(row(&mut doc, header, "h", 10));
    }
    let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
    rows.push(row(&mut doc, group, "c", 10));
    rows.push(row(&mut doc, group, "d", last_body_height));
    if matches!(
        tail,
        Tail::Footer
            | Tail::FooterCaption
            | Tail::FooterTwoAuto
            | Tail::DirectFooterTwoAuto
            | Tail::DirectFooterTwoAvoid
            | Tail::FooterTwoAvoid
    ) {
        let footer_css = if matches!(tail, Tail::FooterTwoAvoid | Tail::DirectFooterTwoAvoid) {
            "display:table-footer-group;break-inside:avoid"
        } else {
            "display:table-footer-group;break-inside:auto"
        };
        let footer = doc.append_element(Some(table), "tfoot", Some(footer_css));
        rows.push(row(&mut doc, footer, "f", 10));
        if matches!(
            tail,
            Tail::FooterTwoAuto
                | Tail::DirectFooterTwoAuto
                | Tail::DirectFooterTwoAvoid
                | Tail::FooterTwoAvoid
        ) {
            rows.push(row(&mut doc, footer, "g", 10));
        }
    }
    if matches!(
        tail,
        Tail::BottomCaption | Tail::FooterCaption | Tail::HeaderCaption | Tail::MultipleCaptions
    ) {
        let caption = doc.append_element(
            Some(table),
            "caption",
            Some("display:table-caption;caption-side:bottom;height:10px"),
        );
        doc.append_text(caption, "k");
        rows.push(caption);
    }
    if matches!(tail, Tail::MultipleCaptions) {
        let extra = doc.append_element(
            Some(table),
            "caption",
            Some("display:table-caption;caption-side:bottom;height:100px"),
        );
        doc.append_text(extra, "second");
    }
    if reverse_source {
        doc.nodes[table].children.reverse();
    }
    if let Some(height) = following_height {
        let following = doc.append_element(
            Some(content),
            "div",
            Some(format!("display:block;height:{height}px")),
        );
        doc.append_text(following, "T");
    }
    let after = doc.append_element(Some(body), "div", Some("display:block"));
    doc.append_text(after, "Z");
    doc.mark_in_document_flags();
    let cascade = crate::layout::test_support::cascaded(&doc);
    let mut page = raikiri_traits::PageBox::new();
    page.width = 200.0;
    page.height = 200.0;
    let pages = layout_pages(with_ahem(&mut doc), &cascade, page)
        .unwrap()
        .len();
    (doc, holder, table, rows, after, pages)
}

fn assert_tail(tail: Tail, expected_rows: &[f32], expected_after: f32) {
    let (doc, holder, table, rows, after, pages) = printed_table(tail, 30, false);
    let actual: Vec<_> = rows.iter().map(|&row| page_flow_y(&doc, row)).collect();
    assert_eq!(actual, expected_rows, "{tail:?}");
    assert_eq!(page_flow_y(&doc, after), expected_after, "{tail:?}");
    assert_eq!(
        doc.nodes[holder].layout.size.height, expected_after,
        "{tail:?}"
    );
    let (_, table_y, _, table_height) = crate::layout::test_support::absolute_rect(&doc, table);
    let last = *rows.last().unwrap();
    let (_, last_y, _, last_height) = crate::layout::test_support::absolute_rect(&doc, last);
    assert!(table_y + table_height >= last_y + last_height, "{tail:?}");
    assert_eq!(pages, 2, "{tail:?}");
}

#[test]
fn a_footer_row_moves_with_the_closing_edges() {
    assert_tail(Tail::Footer, &[160.0, 170.0, 200.0], 240.0);
}

#[test]
fn a_last_body_row_follows_its_repeated_header_and_the_closing_edges() {
    assert_tail(Tail::Header, &[160.0, 170.0, 210.0], 250.0);
}

#[test]
fn a_bottom_caption_moves_with_the_closing_edges() {
    assert_tail(Tail::BottomCaption, &[160.0, 170.0, 200.0], 240.0);
}

#[test]
fn a_bottom_caption_alone_carries_the_edges_after_a_footer() {
    assert_tail(Tail::FooterCaption, &[160.0, 170.0, 180.0, 200.0], 240.0);
}

#[test]
fn table_tail_selection_follows_visual_group_order() {
    for (tail, expected) in [
        (Tail::Footer, [160.0, 170.0, 200.0]),
        (Tail::Header, [160.0, 170.0, 210.0]),
    ] {
        let (doc, _, _, rows, after, _) = printed_table(tail, 30, true);
        let actual: Vec<_> = rows.into_iter().map(|row| page_flow_y(&doc, row)).collect();
        assert_eq!(actual, expected, "{tail:?}");
        let expected_after = if matches!(tail, Tail::Header) {
            250.0
        } else {
            240.0
        };
        assert_eq!(page_flow_y(&doc, after), expected_after, "{tail:?}");
    }
}

#[test]
fn table_tails_stay_when_the_closing_edges_fit_exactly() {
    for tail in [Tail::Footer, Tail::Header, Tail::BottomCaption] {
        let (doc, holder, _, rows, after, _) = printed_table(tail, 10, false);
        let actual: Vec<_> = rows.into_iter().map(|row| page_flow_y(&doc, row)).collect();
        assert_eq!(actual, [160.0, 170.0, 180.0], "{tail:?}");
        assert_eq!(doc.nodes[holder].layout.size.height, 200.0, "{tail:?}");
        assert_eq!(page_flow_y(&doc, after), 200.0, "{tail:?}");
    }
}

#[test]
fn a_multirow_footer_with_auto_breaks_moves_only_the_last_row() {
    assert_tail(Tail::FooterTwoAuto, &[150.0, 160.0, 170.0, 200.0], 240.0);
}

#[test]
fn a_direct_table_with_auto_footer_breaks_moves_only_its_last_row() {
    let (doc, _, table, rows, after, pages) = printed_table(Tail::DirectFooterTwoAuto, 30, false);
    let actual: Vec<_> = rows.into_iter().map(|row| page_flow_y(&doc, row)).collect();
    assert_eq!(actual, [150.0, 160.0, 170.0, 200.0]);
    assert_eq!(page_flow_y(&doc, after), 240.0);
    let (_, y, _, height) = crate::layout::test_support::absolute_rect(&doc, table);
    assert_eq!(y + height, 240.0);
    assert_eq!(pages, 2);
}

#[test]
fn direct_table_footer_controls_keep_avoid_and_no_tail_behavior() {
    for (tail, padding, expected_rows, expected_after) in [
        (
            Tail::DirectFooterTwoAuto,
            0,
            [150.0, 160.0, 170.0, 180.0],
            190.0,
        ),
        (
            Tail::DirectFooterTwoAvoid,
            0,
            [150.0, 160.0, 170.0, 180.0],
            190.0,
        ),
        (
            Tail::DirectFooterTwoAvoid,
            30,
            [150.0, 160.0, 200.0, 210.0],
            250.0,
        ),
    ] {
        let (doc, _, _, rows, after, _) = printed_table(tail, padding, false);
        let actual: Vec<_> = rows.into_iter().map(|row| page_flow_y(&doc, row)).collect();
        assert_eq!(actual, expected_rows, "{tail:?}, padding={padding}");
        assert_eq!(
            page_flow_y(&doc, after),
            expected_after,
            "{tail:?}, padding={padding}"
        );
    }
}

#[test]
fn an_avoided_multirow_footer_moves_as_a_group_without_counting_rows_twice() {
    assert_tail(Tail::FooterTwoAvoid, &[150.0, 160.0, 200.0, 210.0], 250.0);
    let (doc, holder, _, rows, after, _) = printed_table(Tail::FooterTwoAvoid, 10, false);
    let actual: Vec<_> = rows.into_iter().map(|row| page_flow_y(&doc, row)).collect();
    assert_eq!(actual, [150.0, 160.0, 170.0, 180.0]);
    assert_eq!(doc.nodes[holder].layout.size.height, 200.0);
    assert_eq!(page_flow_y(&doc, after), 200.0);
}

#[test]
fn a_header_is_copied_above_a_bottom_caption_that_moves_alone() {
    // The caption fits on page one but not together with the holder's end padding, so it
    // moves alone and a copy of the header goes above it on the next page (Chromium 152).
    let (doc, holder, table, rows, after, pages) = printed_table(Tail::HeaderCaption, 30, false);
    let actual: Vec<_> = rows.iter().map(|&row| page_flow_y(&doc, row)).collect();
    assert_eq!(actual, [160.0, 170.0, 180.0, 210.0]);
    assert_eq!(page_flow_y(&doc, after), 250.0);
    assert_eq!(doc.nodes[holder].layout.size.height, 250.0);
    assert_eq!(pages, 2);
    let (_, table_y, _, table_height) = crate::layout::test_support::absolute_rect(&doc, table);
    assert_eq!((table_y, table_height), (160.0, 60.0));
    let header = doc
        .table_objects
        .headers
        .headers
        .values()
        .find(|header| !header.footer)
        .unwrap();
    assert_eq!(header.placements.get(&1), Some(&(200.0, 200.0)));
}

#[test]
fn a_caption_gets_no_header_copy_on_a_page_that_has_one_or_cannot_hold_it() {
    let (mut doc, _, _, rows, _, _) = printed_table(Tail::HeaderCaption, 30, false);
    let caption = *rows.last().unwrap();
    let mut headers = doc.table_objects.headers.clone();
    let copies = |headers: &HeaderRepeats| {
        headers
            .headers
            .values()
            .filter(|header| header.placements.contains_key(&1))
            .count()
    };
    // The caption sits at the top of page one, which starts at `step`.
    let room = |doc: &mut Document, headers: &mut HeaderRepeats, step: f32| {
        let pages = PageOrigins::new(&[], step);
        make_room_for_caption_header(doc, headers, caption, step, 10.0, 1, &pages)
    };
    // The layout already placed the copy of the second page.
    assert_eq!(copies(&headers), 1);
    assert_eq!(room(&mut doc, &mut headers, 200.0), 0.0);
    // The 10px copy and the 10px caption do not fit a 15px page.
    for header in headers.headers.values_mut() {
        header.placements.clear();
    }
    assert_eq!(room(&mut doc, &mut headers, 15.0), 0.0);
    assert_eq!(copies(&headers), 0);
    // They fit a page of 20px, and the caption goes below the copy.
    assert_eq!(room(&mut doc, &mut headers, 20.0), 10.0);
    assert_eq!(copies(&headers), 1);
}

#[test]
fn repeated_headers_shift_followers_inside_and_outside_nested_auto_height_blocks() {
    let (doc, holder, table, rows, after, _) =
        printed_table_with_followers(Tail::Header, 30, false, 2, 40, Some(10));
    let actual: Vec<_> = rows.iter().map(|&row| page_flow_y(&doc, row)).collect();
    assert_eq!(actual, [160.0, 170.0, 210.0]);
    let parent = doc.parent_of(table).unwrap();
    let siblings = &doc.nodes[parent].children;
    let index = siblings.iter().position(|&id| id == table).unwrap();
    assert_eq!(page_flow_y(&doc, siblings[index + 1]), 250.0);
    assert_eq!(doc.nodes[holder].layout.size.height, 290.0);
    assert_eq!(page_flow_y(&doc, after), 290.0);
    let header = doc
        .table_objects
        .headers
        .headers
        .values()
        .find(|header| !header.footer)
        .unwrap();
    assert_eq!(header.placements.get(&1), Some(&(200.0, 200.0)));
}

#[test]
fn a_tail_row_does_not_move_to_a_shorter_page_without_room_for_its_header() {
    let (mut doc, _, _, rows, _, _) = printed_table(Tail::Header, 30, false);
    let last = *rows.last().unwrap();
    let headers = doc.table_objects.headers.clone();
    assert_eq!(headers.repeated_height_before(last), 10.0);
    let cascade = crate::layout::test_support::cascaded(&doc);
    let tree = crate::layout::context::LayoutSession::bridged(&mut doc, &cascade);
    let candidate = |node_id, raw_y, edges| PageCandidate {
        node_id,
        raw_y,
        height: 10.0,
        context: element_candidate_context(candidates::ElementContext {
            is_table_row: true,
            item_tail_edges: edges,
            ..Default::default()
        }),
        page_name: None,
        named_flex_context: None,
        ifc_root: None,
    };
    let base_fit = BoxFit {
        effective_y: 180.0,
        flow_y: 180.0,
        margin_top: 0.0,
        trailing_margin: 0.0,
        holder_lead: 0.0,
        holder_margin_top: 0.0,
        avoided_lead: 0.0,
        avoided_margin_top: 0.0,
        page_start: 0.0,
        page_end: 200.0,
        next_page_step: 15.0,
    };
    let mut trailing = TrailingFlexChildren::default();
    for group_height in [None, Some(10.0), Some(20.0)] {
        let mut breaks = AvoidedBreaks::default();
        if let Some(height) = group_height {
            breaks.row_groups.insert(last, (180.0 + height, 0.0));
        }
        let height = group_height.unwrap_or(10.0);
        let tail = candidate(last, 180.0, 30.0 + height - 10.0);
        let needed = tail.height + headers.repeated_height_before(last);
        let short = BoxFit {
            next_page_step: needed - 5.0,
            ..base_fit
        };
        assert!(item_tail_move(&tail, &short));
        let overflow = breaks.measure(&tree, &cascade, &tail, &short, &headers, &mut trailing);
        assert!(!overflow.table_row, "{group_height:?}");
        assert!(!overflow.moves_box(), "{group_height:?}");
        let exact = BoxFit {
            next_page_step: needed,
            ..base_fit
        };
        let overflow = breaks.measure(&tree, &cascade, &tail, &exact, &headers, &mut trailing);
        assert!(overflow.table_row, "{group_height:?}");
        assert!(overflow.moves_box(), "{group_height:?}");
    }
    // An unfitting group does not prevent its final fallback row moving alone.
    let first = rows[1];
    let mut breaks = AvoidedBreaks::default();
    breaks.row_groups.insert(first, (200.0, 0.0));
    let short = BoxFit {
        effective_y: 170.0,
        flow_y: 170.0,
        next_page_step: 25.0,
        ..base_fit
    };
    let group = PageCandidate {
        height: 20.0,
        ..candidate(first, 170.0, 40.0)
    };
    assert!(item_tail_move(&group, &short));
    assert!(
        !breaks
            .measure(&tree, &cascade, &group, &short, &headers, &mut trailing)
            .moves_box()
    );
    let fallback = candidate(last, 190.0, 30.0);
    let fit = BoxFit {
        effective_y: 190.0,
        flow_y: 190.0,
        next_page_step: 25.0,
        ..base_fit
    };
    assert!(
        breaks
            .measure(&tree, &cascade, &fallback, &fit, &headers, &mut trailing)
            .moves_box()
    );
}

#[test]
fn a_source_footer_at_the_page_top_does_not_move_again_for_oversized_edges() {
    let (mut doc, _, _, _, _, _) = printed_table(Tail::Footer, 30, false);
    let mut headers = doc.table_objects.headers.clone();
    let footer_id = *headers
        .headers
        .iter()
        .find(|(_, header)| header.footer)
        .unwrap()
        .0;
    let footer = headers.headers.get_mut(&footer_id).unwrap();
    assert_eq!(footer.source_y, 200.0);
    footer.closing_edges = 300.0;
    let shift = place_table_footer(
        &mut doc,
        &mut headers,
        footer_id,
        0.0,
        &PageOrigins::new(&[], 200.0),
    );
    assert_eq!(shift, 0.0);
    let footer = &headers.headers[&footer_id];
    assert_eq!(footer.source_y, 200.0);
    assert_eq!(footer.placements.len(), 1);
    assert_eq!(footer.placements.get(&1), Some(&(200.0, 200.0)));
}

#[test]
fn table_tail_selection_rejects_relative_vertical_and_breakable_captions() {
    for (css, text, supported) in [
        ("", "k", true),
        ("position:relative", "k", false),
        ("writing-mode:vertical-rl", "k", false),
        ("width:8px", "a a a", false),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Some("display:block;font-family:Ahem;font-size:8px;line-height:10px"),
        );
        let table = doc.append_element(Some(body), "table", Some("display:table;border-spacing:0"));
        for _ in 0..2 {
            let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
            doc.append_element(
                Some(row),
                "td",
                Some("display:table-cell;height:10px;padding:0"),
            );
        }
        let caption = doc.append_element(
            Some(table),
            "caption",
            Some(format!("display:table-caption;caption-side:bottom;{css}")),
        );
        doc.append_text(caption, text);
        doc.mark_in_document_flags();
        let cascade = crate::layout::test_support::cascaded(&doc);
        crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600())
            .unwrap();
        let tree = crate::layout::context::LayoutSession::resumed(&mut doc, &cascade);
        let tail = container_item_tail(&tree, &cascade, table, false);
        assert_eq!(tail, supported.then_some((caption, None)), "{css}");
    }
}

#[test]
fn a_second_bottom_caption_follows_the_first_and_runs_over_the_page_end() {
    // Chromium puts both captions below the grid, in source order: the second
    // one, 100px high, starts at the end of the page and runs over onto the
    // next, where the wrapper's padding and the following block come after it.
    // The first caption is not the table's visual tail, so it stays where it is.
    let (doc, holder, table, rows, after, pages) = printed_table(Tail::MultipleCaptions, 30, false);
    let actual: Vec<_> = rows.into_iter().map(|row| page_flow_y(&doc, row)).collect();
    assert_eq!(actual, [160.0, 170.0, 180.0]);
    let second = *doc.nodes[table].children.last().unwrap();
    assert_eq!(page_flow_y(&doc, second), 190.0);
    assert_eq!(doc.nodes[holder].layout.size.height, 320.0);
    assert_eq!(page_flow_y(&doc, after), 320.0);
    assert_eq!(pages, 2);
}

#[test]
fn a_nested_tables_first_header_stays_with_its_first_body_row() {
    assert_tail(Tail::HeaderFirstBody, &[200.0, 210.0, 220.0], 260.0);
}
