use super::*;
use crate::layout::test_support::{ahem_paragraph_with, page_box_800x600, with_ahem};
use crate::{PageSlice, layout_pages, layout_single_page};
use raikiri_traits::PageBox;

/// Seven 40px Ahem words: two fit on a 100px line, so a 100px wide page
/// breaks them into four 10px lines starting at bytes 0, 10, 20 and 30.
const WORDS: &str = "aaaa bbbb cccc dddd eeee ffff gggg";

fn page_box(width: f32, height: f32) -> PageBox {
    let mut page = PageBox::new();
    page.width = width;
    page.height = height;
    page
}

fn paragraph() -> (Document, CascadeResult, usize) {
    let mut text = 0;
    // `widows: 1` keeps the natural break, so a page starts at the first line
    // that does not fit.
    let (doc, cascade, _) = ahem_paragraph_with("widows:1", |doc, root| {
        text = doc.append_text(root, WORDS);
    });
    (doc, cascade, text)
}

fn paginate(doc: &mut Document, cascade: &CascadeResult, page: PageBox) -> Vec<PageSlice> {
    let slices = layout_pages(with_ahem(doc), cascade, page).expect("layout");
    let geometries: Vec<_> = slices
        .iter()
        .map(|_| {
            (
                page,
                crate::PageMargins::default(),
                crate::PageContentInsets::default(),
            )
        })
        .collect();
    doc.project_pages(cascade, page, &slices, &geometries)
        .expect("projection");
    slices
}

fn project_one_page(doc: &mut Document, cascade: &CascadeResult, page: PageBox) {
    doc.project_pages(
        cascade,
        page,
        &[PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }],
        &[],
    )
    .expect("projection");
}

#[test]
fn page_start_names_the_line_a_page_begins_with() {
    let (mut doc, cascade, text) = paragraph();
    // Three lines fit on a 30px page, so the second page starts at "gggg".
    let slices = paginate(&mut doc, &cascade, page_box(100.0, 30.0));
    assert_eq!(slices.len(), 2);
    let start = doc.page_start(&cascade, 1).expect("a page start");
    assert_eq!(start.token, PageStartToken::Line { text, offset: 30 });
    assert_eq!(start.offset, 0.0);
    assert_eq!(doc.page_token_offset(1, start.token), Some(0.0));
    // The paragraph itself starts on the first page, before the token.
    assert!(doc.page_content_before(&cascade, 1, start.token).is_empty());
    assert!(doc.page_content_before(&cascade, 5, start.token).is_empty());
    // The first page starts with the paragraph's own box.
    let first = doc.page_start(&cascade, 0).expect("a page start");
    assert!(matches!(first.token, PageStartToken::Node(_)));
    assert!(doc.page_start(&cascade, 2).is_none());
}

#[test]
fn continuation_break_starts_a_line_at_its_offset() {
    let (mut doc, cascade, text) = paragraph();
    // At 800px every word fits on one line.
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(doc.ifc_text_lines(text).expect("lines").lines.len(), 1);

    doc.set_continuation_break(Some((text, 20)));
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let lines = doc.ifc_text_lines(text).expect("lines");
    assert_eq!(lines.lines.len(), 2);
    project_one_page(&mut doc, &cascade, page_box_800x600());
    let token = PageStartToken::Line { text, offset: 20 };
    assert_eq!(doc.page_token_offset(0, token), Some(10.0));
    // No line starts at byte 10.
    let other = PageStartToken::Line { text, offset: 10 };
    assert_eq!(doc.page_token_offset(0, other), None);

    doc.set_continuation_break(None);
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(doc.ifc_text_lines(text).expect("lines").lines.len(), 1);
}

#[test]
fn page_start_names_a_block_that_begins_the_page() {
    let mut blocks = Vec::new();
    let (mut doc, cascade, _) = ahem_paragraph_with("", |doc, root| {
        for _ in 0..3 {
            blocks.push(doc.append_element(Some(root), "div", Some("display:block;height:15px")));
        }
    });
    let slices = paginate(&mut doc, &cascade, page_box(100.0, 30.0));
    assert_eq!(slices.len(), 2);
    let start = doc.page_start(&cascade, 1).expect("a page start");
    assert_eq!(start.token, PageStartToken::Node(blocks[2]));
    assert_eq!(start.offset, 0.0);
    assert_eq!(start.token.node(), blocks[2]);
}

#[test]
fn continuation_cascade_drops_breaks_before_the_token() {
    let mut blocks = Vec::new();
    let (mut doc, mut cascade, root) = ahem_paragraph_with("break-inside:avoid", |doc, root| {
        for style in [
            "display:block;break-after:page;page:cover",
            "display:block;break-before:page;break-inside:avoid",
            "display:block;break-before:page",
        ] {
            blocks.push(doc.append_element(Some(root), "div", Some(style)));
        }
    });
    doc.prepare_continuation_cascade(&mut cascade, PageStartToken::Node(blocks[1]));
    let first = &cascade.computed[blocks[0]];
    assert_eq!(first.break_after, BreakBetween::Auto);
    assert_eq!(cascade.page_values[blocks[0]], PageValue::Auto);
    let token = &cascade.computed[blocks[1]];
    assert_eq!(token.break_before, BreakBetween::Auto);
    assert_eq!(token.break_inside, BreakInside::Auto);
    assert_eq!(cascade.computed[root].break_inside, BreakInside::Auto);
    assert_eq!(cascade.computed[root].orphans, 1);
    // A box after the token keeps its breaks.
    assert_eq!(cascade.computed[blocks[2]].break_before, BreakBetween::Page);
}

#[test]
fn page_start_skips_a_float_that_comes_first() {
    let mut blocks = Vec::new();
    let (mut doc, cascade, _) = ahem_paragraph_with("", |doc, root| {
        for style in [
            "display:block;height:15px",
            "display:block;height:15px",
            "display:block;float:left;width:10px;height:10px",
            "display:block;height:15px",
        ] {
            blocks.push(doc.append_element(Some(root), "div", Some(style)));
        }
    });
    let slices = paginate(&mut doc, &cascade, page_box(100.0, 30.0));
    assert_eq!(slices.len(), 2);
    let start = doc.page_start(&cascade, 1).expect("a page start");
    assert_eq!(start.token, PageStartToken::Node(blocks[3]));
}

#[test]
fn continuation_break_at_the_start_of_a_text_node_breaks_before_it() {
    let mut texts = Vec::new();
    let (mut doc, cascade, _) = ahem_paragraph_with("", |doc, root| {
        texts.push(doc.append_text(root, "aaaa "));
        texts.push(doc.append_text(root, "bbbb"));
    });
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(doc.ifc_text_lines(texts[1]).expect("lines").lines.len(), 1);

    doc.set_continuation_break(Some((texts[1], 0)));
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    project_one_page(&mut doc, &cascade, page_box_800x600());
    let token = PageStartToken::Line {
        text: texts[1],
        offset: 0,
    };
    assert_eq!(doc.page_token_offset(0, token), Some(10.0));
}

#[test]
fn ownership_diagnostics_are_optional_and_preserve_public_query_results() {
    use crate::continuation_diagnostics::{
        ContinuationWork, begin_continuation_diagnostics, finish_continuation_diagnostics,
    };
    let (mut doc, cascade, text) = paragraph();
    paginate(&mut doc, &cascade, page_box(100.0, 30.0));
    let expected = doc.page_start(&cascade, 1).unwrap();
    assert_eq!(
        finish_continuation_diagnostics(),
        ContinuationWork::default()
    );
    begin_continuation_diagnostics();
    assert!(doc.ifc_text_lines(text).is_some());
    let work = finish_continuation_diagnostics();
    assert!(work.text_line_visits > 0);
    assert!(work.glyph_owner_calls > 0);
    assert_eq!(doc.page_start(&cascade, 1), Some(expected));
    assert_eq!(
        finish_continuation_diagnostics(),
        ContinuationWork::default()
    );
}

#[test]
fn glyph_owner_diagnostics_preserve_single_line_owners() {
    use crate::continuation_diagnostics::{
        ContinuationWork, begin_continuation_diagnostics, finish_continuation_diagnostics,
    };
    use crate::layout::ifc::text_lines::glyph_owners;

    let (mut doc, cascade, text) = paragraph();
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    let root = doc.ifc_text_lines(text).unwrap().root;
    let paragraph = doc.ifc_layout_node(root).unwrap().ifc.as_ref().unwrap();
    let lines = &paragraph.lines.as_ref().unwrap().lines;
    assert_eq!(lines.len(), 1);
    let expected = glyph_owners(&lines[0]);
    assert!(expected.contains(&text));
    begin_continuation_diagnostics();
    assert_eq!(glyph_owners(&lines[0]), expected);
    assert_eq!(
        finish_continuation_diagnostics(),
        ContinuationWork {
            text_line_visits: 0,
            glyph_owner_calls: 1,
        }
    );
    assert_eq!(glyph_owners(&lines[0]), expected);
    assert_eq!(
        finish_continuation_diagnostics(),
        ContinuationWork::default()
    );
}

#[test]
fn all_page_queries_do_not_recollect_paragraph_ownership() {
    use crate::continuation_diagnostics::{
        begin_continuation_diagnostics, finish_continuation_diagnostics,
    };
    for count in [8, 16, 32] {
        let mut text = 0;
        let (mut doc, cascade, _) =
            ahem_paragraph_with("white-space:pre;orphans:1;widows:1", |doc, root| {
                text = doc.append_text(root, vec!["a"; count].join("\n"));
            });
        assert_eq!(
            paginate(&mut doc, &cascade, page_box(100.0, 10.0)).len(),
            count
        );
        begin_continuation_diagnostics();
        for index in 1..count {
            let token = PageStartToken::Line {
                text,
                offset: (2 * index) as u32,
            };
            assert_eq!(doc.page_start(&cascade, index as u32).unwrap().token, token);
            assert_eq!(doc.page_token_offset(index as u32, token), Some(0.0));
        }
        let work = finish_continuation_diagnostics();
        assert_eq!(
            work.text_line_visits, 0,
            "querying prepared pages must not rescan paragraph lines"
        );
        assert_eq!(work.glyph_owner_calls, 0);
    }
}

#[test]
fn a_later_text_owner_uses_its_own_first_line_and_local_line_indices() {
    let mut late = 0;
    let (mut doc, cascade, _) =
        ahem_paragraph_with("white-space:pre;orphans:1;widows:1", |doc, root| {
            doc.append_text(root, "a\nb\n");
            let span = doc.append_element(Some(root), "span", None::<&str>);
            late = doc.append_text(span, "c\nd\n");
            doc.append_text(root, "e\nf");
        });
    paginate(&mut doc, &cascade, page_box(100.0, 10.0));
    for (page, offset) in [(2, 0), (3, 2)] {
        let token = PageStartToken::Line { text: late, offset };
        let start = doc.page_start(&cascade, page).unwrap();
        assert_eq!(start.token, token);
        assert_eq!(start.offset, 0.0);
        assert_eq!(doc.page_token_offset(page, token), Some(0.0));
    }
}

#[test]
fn shared_line_owners_keep_the_following_lines_source_token() {
    let mut second = 0;
    let (mut doc, cascade, _) =
        ahem_paragraph_with("white-space:pre;orphans:1;widows:1", |doc, root| {
            doc.append_text(root, "aaaa ");
            let span = doc.append_element(Some(root), "span", None::<&str>);
            second = doc.append_text(span, "bbbb\ncccc");
        });
    paginate(&mut doc, &cascade, page_box(100.0, 10.0));
    let token = PageStartToken::Line {
        text: second,
        offset: 5,
    };
    assert_eq!(doc.page_start(&cascade, 1).unwrap().token, token);
    assert_eq!(doc.page_token_offset(1, token), Some(0.0));
}

fn assert_owner_index_matches_uncached_records(doc: &Document) {
    let owners: std::collections::HashSet<_> = doc
        .page_projection
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .filter(|item| item.kind == PageFragmentKind::Text)
        .map(|item| item.node_id.0 as usize)
        .collect();
    for owner in owners {
        let cached = doc
            .page_projection
            .continuation_lines
            .as_deref()
            .and_then(|index| index.get(owner));
        let uncached = doc.ifc_text_lines(owner);
        assert_eq!(cached, uncached.as_ref());
    }
}

#[test]
fn ownership_snapshot_handles_ruby_generated_text_and_whitespace() {
    let (mut doc, cascade, _) =
        ahem_paragraph_with("white-space:pre;orphans:1;widows:1", |doc, root| {
            doc.add_stylesheet(
                "div::before{content:'G'}",
                raikiri_traits::StylesheetKind::Author,
            );
            doc.append_text(root, " ");
            let ruby = doc.append_element(Some(root), "ruby", Some("display:ruby"));
            let base = doc.append_element(Some(ruby), "rb", Some("display:ruby-base"));
            doc.append_text(base, "AA");
            let annotation =
                doc.append_element(Some(ruby), "rt", Some("display:ruby-text;font-size:5px"));
            doc.append_text(annotation, "xy");
            doc.append_text(root, "\na\nb\nc");
        });
    let slices = paginate(&mut doc, &cascade, page_box(100.0, 20.0));
    assert!(slices.len() > 1);
    assert_owner_index_matches_uncached_records(&doc);
    let before: Vec<_> = slices
        .iter()
        .map(|page| doc.page_start(&cascade, page.page_index))
        .collect();
    for (page, expected) in slices.iter().zip(before) {
        assert_eq!(doc.page_start(&cascade, page.page_index), expected);
        if let Some(start) = expected {
            assert_eq!(
                doc.page_token_offset(page.page_index, start.token),
                uncached_token_offset(&doc, page.page_index, start.token)
            );
        }
    }
}

#[test]
fn clones_share_ownership_until_one_side_is_relaid_out() {
    let (mut doc, cascade, _) = paragraph();
    let wide = page_box(100.0, 10.0);
    let slices = paginate(&mut doc, &cascade, wide);
    let before: Vec<_> = slices
        .iter()
        .map(|p| doc.page_start(&cascade, p.page_index))
        .collect();
    let original = doc.page_projection.continuation_lines.clone().unwrap();
    let diagnostic = format!("{original:?}");
    assert!(diagnostic.contains("root:") && diagnostic.contains("lines:"));
    let mut cloned = doc.clone();
    assert!(std::sync::Arc::ptr_eq(
        &original,
        cloned.page_projection.continuation_lines.as_ref().unwrap()
    ));
    assert_eq!(
        slices
            .iter()
            .map(|p| cloned.page_start(&cascade, p.page_index))
            .collect::<Vec<_>>(),
        before
    );
    paginate(&mut cloned, &cascade, page_box(50.0, 10.0));
    assert!(!std::sync::Arc::ptr_eq(
        &original,
        cloned.page_projection.continuation_lines.as_ref().unwrap()
    ));
    assert_eq!(
        slices
            .iter()
            .map(|p| doc.page_start(&cascade, p.page_index))
            .collect::<Vec<_>>(),
        before
    );
    assert_owner_index_matches_uncached_records(&cloned);
    let surviving = doc.clone();
    drop(doc);
    drop(original);
    assert_eq!(
        slices
            .iter()
            .map(|p| surviving.page_start(&cascade, p.page_index))
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn cancelled_projection_keeps_the_old_ownership_snapshot() {
    let (mut doc, cascade, text) = paragraph();
    let page = page_box(100.0, 10.0);
    let slices = paginate(&mut doc, &cascade, page);
    let geometries: Vec<_> = slices
        .iter()
        .map(|_| {
            (
                page,
                crate::PageMargins::default(),
                crate::PageContentInsets::default(),
            )
        })
        .collect();
    let before = doc.page_start(&cascade, 1);
    let original = doc.page_projection.continuation_lines.clone().unwrap();
    let lines = doc.ifc_text_lines(text).unwrap().lines.len();
    crate::layout::test_support::TEXT_LINE_VISITS.with(|visits| visits.set(0));
    let abort =
        || crate::layout::test_support::TEXT_LINE_VISITS.with(|visits| visits.get() > lines);
    let control = crate::PageLayoutControl::default().with_abort_check(&abort);
    assert!(matches!(
        doc.project_pages_with_control(&cascade, page, &slices, &geometries, &control),
        Err(raikiri_traits::LayoutError::Aborted)
    ));
    assert!(crate::layout::test_support::TEXT_LINE_VISITS.with(|visits| visits.get() > lines));
    assert!(std::sync::Arc::ptr_eq(
        &original,
        doc.page_projection.continuation_lines.as_ref().unwrap()
    ));
    assert_eq!(doc.page_start(&cascade, 1), before);
}

#[test]
fn empty_projection_has_no_ownership_allocation() {
    let mut doc = Document::new();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    doc.project_pages(&cascade, page_box_800x600(), &[], &[])
        .unwrap();
    assert!(doc.page_projection.continuation_lines.is_none());
    assert!(doc.page_start(&cascade, 0).is_none());
}

#[test]
fn fixed_repeats_do_not_replace_the_normal_flows_page_start() {
    let mut text = 0;
    let (mut doc, cascade, _) =
        ahem_paragraph_with("white-space:pre;orphans:1;widows:1", |doc, root| {
            let fixed = doc.append_element(Some(root), "div", Some("position:fixed;top:0"));
            doc.append_text(fixed, "fixed");
            text = doc.append_text(root, "a\nb\nc\nd");
        });
    let slices = paginate(&mut doc, &cascade, page_box(100.0, 10.0));
    assert_eq!(slices.len(), 4);
    assert!(
        doc.page_projection
            .pages
            .iter()
            .skip(1)
            .any(|p| p.items.iter().any(|item| item.is_repeat))
    );
    for index in 1..4 {
        let token = PageStartToken::Line {
            text,
            offset: 2 * index,
        };
        assert_eq!(doc.page_start(&cascade, index).unwrap().token, token);
        assert_eq!(doc.page_token_offset(index, token), Some(0.0));
    }
    assert_owner_index_matches_uncached_records(&doc);
}

#[test]
fn continuation_break_keeps_the_old_snapshot_until_relayout_and_then_rebuilds() {
    let (mut doc, cascade, text) = paragraph();
    let page = page_box_800x600();
    layout_single_page(with_ahem(&mut doc), &cascade, page).unwrap();
    project_one_page(&mut doc, &cascade, page);
    let old = doc.page_projection.continuation_lines.clone().unwrap();
    let token = PageStartToken::Line { text, offset: 20 };
    assert_eq!(doc.page_token_offset(0, token), None);
    doc.set_continuation_break(Some((text, 20)));
    assert!(std::sync::Arc::ptr_eq(
        &old,
        doc.page_projection.continuation_lines.as_ref().unwrap()
    ));
    assert_eq!(doc.page_token_offset(0, token), None);
    layout_single_page(with_ahem(&mut doc), &cascade, page).unwrap();
    assert!(doc.page_projection.continuation_lines.is_none());
    project_one_page(&mut doc, &cascade, page);
    let resumed = doc.page_projection.continuation_lines.clone().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&old, &resumed));
    assert_eq!(doc.page_token_offset(0, token), Some(10.0));
    doc.set_continuation_break(None);
    assert!(std::sync::Arc::ptr_eq(
        &resumed,
        doc.page_projection.continuation_lines.as_ref().unwrap()
    ));
    layout_single_page(with_ahem(&mut doc), &cascade, page).unwrap();
    project_one_page(&mut doc, &cascade, page);
    assert_eq!(doc.page_token_offset(0, token), None);
    assert_owner_index_matches_uncached_records(&doc);
}

#[test]
fn repeated_table_headers_do_not_become_continuation_starts() {
    let mut header = 0;
    let (mut doc, cascade, _) = ahem_paragraph_with("", |doc, root| {
        // Header repetition currently applies to ordinary body-child tables.
        let body_parent = doc.parent_of(root).unwrap();
        let table = doc.append_element(
            Some(body_parent),
            "table",
            Some("display:table;border-spacing:0;width:100px;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        header = doc.append_element(Some(table), "thead", Some("display:table-header-group"));
        let row = doc.append_element(Some(header), "tr", Some("display:table-row"));
        let cell = doc.append_element(
            Some(row),
            "td",
            Some("display:table-cell;padding:0;height:10px"),
        );
        doc.append_text(cell, "H");
        let body = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
        for _ in 0..6 {
            let row = doc.append_element(Some(body), "tr", Some("display:table-row"));
            let cell = doc.append_element(
                Some(row),
                "td",
                Some("display:table-cell;padding:0;height:40px"),
            );
            doc.append_text(cell, "B");
        }
    });
    let slices = paginate(&mut doc, &cascade, page_box(100.0, 100.0));
    assert!(slices.len() > 1);
    assert!(
        doc.page_projection
            .pages
            .iter()
            .skip(1)
            .any(|p| p.items.iter().any(|item| item.is_table_header_repeat))
    );
    for slice in slices.iter().skip(1) {
        let start = doc
            .page_start(&cascade, slice.page_index)
            .expect("body continuation");
        assert!(!doc.is_ancestor_or_self(header, start.token.node()));
    }
    assert_owner_index_matches_uncached_records(&doc);
}

// The old public-path calculation is an oracle for raw line offsets, which
// can be negative even when page_start clamps its resume offset to zero.
fn uncached_token_offset(doc: &Document, page_index: u32, token: PageStartToken) -> Option<f32> {
    let page = doc
        .page_projection
        .pages
        .iter()
        .find(|p| p.page_index == page_index)?;
    page.items
        .iter()
        .filter(|item| {
            item.node_id.0 as usize == token.node()
                && !item.is_repeat
                && !item.is_table_header_repeat
        })
        .find_map(|item| match token {
            PageStartToken::Node(_) => (item.fragment_index == 0).then_some(item.flow_box_y),
            PageStartToken::Line { text, offset } => {
                let range = item.line_range?;
                let lines = doc.ifc_text_lines(text)?;
                let first = lines.lines.first()?;
                lines
                    .lines
                    .get(range.start as usize..range.end as usize)?
                    .iter()
                    .find(|line| line_start(doc, lines.root, line.line) == Some((text, offset)))
                    .map(|line| item.flow_box_y + (line.top - first.top))
            }
        })
}

#[test]
fn over_budget_projection_keeps_the_old_ownership_snapshot() {
    let mut text = 0;
    let (mut doc, cascade, _) = ahem_paragraph_with(
        "white-space:pre;width:100px;column-count:2;column-gap:20px;column-rule:2px solid red;orphans:1;widows:1",
        |doc, root| {
            text = doc.append_text(root, "A\nB\nC\nD");
        },
    );
    let page = page_box_800x600();
    layout_single_page(with_ahem(&mut doc), &cascade, page).unwrap();
    project_one_page(&mut doc, &cascade, page);
    let original = doc.page_projection.continuation_lines.clone().unwrap();
    let owned = doc.ifc_text_lines(text).unwrap();
    let (text, offset) = line_start(&doc, owned.root, owned.lines[1].line).unwrap();
    let token = PageStartToken::Line { text, offset };
    let before = doc.page_token_offset(0, token);
    assert!(before.is_some());
    doc.lower_layout_fragment_limit(0);
    let slices = [PageSlice {
        page_index: 0,
        content_origin_y: 0.0,
        page_name: None,
    }];
    assert!(matches!(
        doc.project_pages(&cascade, page, &slices, &[]),
        Err(raikiri_traits::LayoutError::FragmentLimitExceeded { .. })
    ));
    assert!(std::sync::Arc::ptr_eq(
        &original,
        doc.page_projection.continuation_lines.as_ref().unwrap()
    ));
    assert_eq!(doc.page_token_offset(0, token), before);
}

#[test]
fn indexing_existing_lines_does_not_spend_new_fragment_units() {
    let (mut doc, cascade, _) = paragraph();
    let page = page_box(100.0, 10.0);
    let slices = paginate(&mut doc, &cascade, page);
    let before = doc.page_start(&cascade, 1);
    let geometries: Vec<_> = slices
        .iter()
        .map(|_| {
            (
                page,
                crate::PageMargins::default(),
                crate::PageContentInsets::default(),
            )
        })
        .collect();
    doc.lower_layout_fragment_limit(0);
    doc.project_pages(&cascade, page, &slices, &geometries)
        .expect("plain projection creates no additional fragment expansion");
    assert!(doc.page_projection.continuation_lines.is_some());
    assert_eq!(doc.page_start(&cascade, 1), before);
    assert_owner_index_matches_uncached_records(&doc);
}

#[test]
fn diagnostic_counters_separate_linear_construction_from_borrowed_queries() {
    use crate::continuation_diagnostics::{
        ContinuationWork, begin_continuation_diagnostics, finish_continuation_diagnostics,
    };
    for count in [8, 16, 32] {
        let mut text = 0;
        let (mut doc, cascade, _) =
            ahem_paragraph_with("white-space:pre;orphans:1;widows:1", |doc, root| {
                text = doc.append_text(root, vec!["a"; count].join("\n"));
            });
        let page = page_box(100.0, 10.0);
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).unwrap();
        let geometries: Vec<_> = slices
            .iter()
            .map(|_| {
                (
                    page,
                    crate::PageMargins::default(),
                    crate::PageContentInsets::default(),
                )
            })
            .collect();
        begin_continuation_diagnostics();
        doc.project_pages(&cascade, page, &slices, &geometries)
            .unwrap();
        let work = finish_continuation_diagnostics();
        assert!(work.text_line_visits >= count as u64 && work.text_line_visits <= 3 * count as u64);
        assert!(
            work.glyph_owner_calls >= count as u64 && work.glyph_owner_calls <= 3 * count as u64
        );
        begin_continuation_diagnostics();
        for index in 1..count {
            let token = PageStartToken::Line {
                text,
                offset: (2 * index) as u32,
            };
            assert_eq!(doc.page_start(&cascade, index as u32).unwrap().token, token);
            assert_eq!(doc.page_token_offset(index as u32, token), Some(0.0));
        }
        assert_eq!(
            finish_continuation_diagnostics(),
            ContinuationWork::default()
        );
    }
}
