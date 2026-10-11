use super::*;
use crate::{PageSlice, layout_pages};
use raikiri_traits::PageBox;

/// A document whose parts fall on different pages of a 300 by 100 page:
/// a translucent block, a clipped block, a list, a table and a float.
fn spread_document() -> (Document, CascadeResult, Vec<PageSlice>) {
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0;font:10px/10px Ahem"),
    );
    let block = |document: &mut Document, parent, style: &str| {
        document.append_element(Some(parent), "div", taffy::Style::default(), Some(style))
    };
    let translucent = block(&mut document, body, "display:block;opacity:.5");
    let text = block(&mut document, translucent, "display:block;width:10px");
    document.append_text(text, ["X"; 12].join(" "));
    let clipped = block(
        &mut document,
        body,
        "display:block;overflow:hidden;height:60px;background:blue",
    );
    let inner = block(&mut document, clipped, "display:block;width:10px");
    document.append_text(inner, ["X"; 8].join(" "));
    let list = document.append_element(
        Some(body),
        "ol",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    for _ in 0..6 {
        let item = document.append_element(
            Some(list),
            "li",
            taffy::Style::default(),
            Some("display:list-item;margin-left:20px"),
        );
        document.append_text(item, "X");
    }
    let table = block(&mut document, body, "display:table");
    for _ in 0..4 {
        let row = block(&mut document, table, "display:table-row");
        let cell = block(&mut document, row, "display:table-cell;background:green");
        document.append_text(cell, "X X");
    }
    let wrapper = block(&mut document, body, "display:block");
    let float = block(
        &mut document,
        wrapper,
        "float:left;width:20px;height:20px;background:red",
    );
    document.append_text(float, "X");
    document.append_text(wrapper, ["X"; 6].join(" "));
    document.mark_in_document_flags();
    let fonts = crate::build_wpt_font_collection(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/text-autospace"
    )))
    .unwrap();
    document.set_font_collection(fonts);
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    let mut page_box = PageBox::new();
    page_box.width = 300.0;
    page_box.height = 100.0;
    let slices = layout_pages(&mut document, &cascade, page_box).unwrap();
    document
        .project_pages(&cascade, page_box, &slices, &[])
        .unwrap();
    (document, cascade, slices)
}

#[test]
fn skipping_subtrees_without_events_keeps_every_page_order() {
    let (document, cascade, slices) = spread_document();
    assert!(slices.len() >= 3, "{} pages", slices.len());
    let mut skipped = false;
    for slice in &slices {
        let page = slice.page_index;
        let runs = document.page_text_runs(&cascade, page);
        let mut lines = TextLines::default();
        for run in &runs {
            let paragraph = lines.paragraphs.entry(run.line.root).or_default();
            if !paragraph.contains(&run.line) {
                paragraph.push(run.line);
            }
        }
        for lines in [None, Some(&lines)] {
            let full = document.page_paint_order_impl(&cascade, page, lines, false);
            let pruned = document.page_paint_order_impl(&cascade, page, lines, true);
            assert_eq!(format!("{pruned:?}"), format!("{full:?}"), "page {page}");
        }
        let position = document
            .page_projection
            .pages
            .iter()
            .position(|fragment| fragment.page_index == page)
            .unwrap();
        let fragment = &document.page_projection.pages[position];
        let mut items = PageItems {
            by_node: HashMap::new(),
            content_box: fragment.content_box,
            overflow_clips: document.overflow_clips_on_page(fragment),
            generated_boxes: document.page_projection.generated_boxes.get(&page),
            cascade: &cascade,
            document: &document,
            page: fragment,
        };
        for item in &fragment.items {
            items
                .by_node
                .entry(item.node_id.0 as usize)
                .or_default()
                .push(item);
        }
        let entered = document
            .page_paint_nodes(&items, page, Some(&lines))
            .unwrap();
        skipped |= entered.len() < document.nodes.len();
    }
    assert!(skipped, "some page should leave a subtree out");
}

#[test]
fn an_opacity_group_off_the_page_is_still_listed() {
    let (document, cascade, slices) = spread_document();
    let last = slices.last().unwrap().page_index;
    let events = document.page_paint_order_impl(&cascade, last, None, true);
    // The translucent block is on the first page only, and its group is
    // listed empty on the last page as before.
    assert!(
        events
            .iter()
            .any(|event| matches!(event, PaintEvent::PushOpacity(_))),
        "{events:?}"
    );
}

#[test]
fn an_untraceable_event_source_walks_every_node() {
    let (mut document, cascade, _) = spread_document();
    document
        .page_projection
        .opacity_layers
        .push(usize::MAX >> 2);
    let fragment = &document.page_projection.pages[0];
    let items = PageItems {
        by_node: HashMap::new(),
        content_box: fragment.content_box,
        overflow_clips: BTreeMap::new(),
        generated_boxes: None,
        cascade: &cascade,
        document: &document,
        page: fragment,
    };
    assert!(document.page_paint_nodes(&items, 0, None).is_none());
}
