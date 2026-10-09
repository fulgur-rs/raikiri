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
    let (doc, cascade, _) = ahem_paragraph_with("", |doc, root| {
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
            blocks.push(doc.append_element(
                Some(root),
                "div",
                taffy::Style::default(),
                Some("display:block;height:15px"),
            ));
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
    let (doc, mut cascade, root) = ahem_paragraph_with("break-inside:avoid", |doc, root| {
        for style in [
            "display:block;break-after:page;page:cover",
            "display:block;break-before:page;break-inside:avoid",
            "display:block;break-before:page",
        ] {
            blocks.push(doc.append_element(
                Some(root),
                "div",
                taffy::Style::default(),
                Some(style),
            ));
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
