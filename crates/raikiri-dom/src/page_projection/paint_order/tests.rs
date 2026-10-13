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
            inline_boxes: document.page_projection.inline_boxes.get(&page),
            generated_images: document.page_projection.generated_images.get(&page),
            cascade: &cascade,
            document: &document,
            page: fragment,
            line_boxes: Default::default(),
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
        inline_boxes: None,
        generated_images: None,
        cascade: &cascade,
        document: &document,
        page: fragment,
        line_boxes: Default::default(),
    };
    assert!(document.page_paint_nodes(&items, 0, None).is_none());
}

/// A paragraph whose `::first-letter` takes the leading quote of each of
/// `spans` translucent spans, laid out and projected on one page.
fn first_letter_spans(spans: usize) -> (Document, raikiri_style::CascadeResult) {
    let mut document = Document::new();
    let style = document.append_element(
        Some(0),
        "style",
        taffy::Style::default(),
        Some("display:none"),
    );
    document.append_text(style, "p::first-letter{opacity:.5}");
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0;font:10px/10px Ahem"),
    );
    let paragraph = document.append_element(
        Some(body),
        "p",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    for _ in 0..spans {
        let span = document.append_element(
            Some(paragraph),
            "span",
            taffy::Style::default(),
            Some("opacity:.5"),
        );
        document.append_text(span, "\"");
    }
    document.append_text(paragraph, "X X");
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
    (document, cascade)
}

#[test]
fn run_groups_share_one_letter_table_per_paragraph() {
    let (document, cascade) = first_letter_spans(3);
    let runs = document.page_text_runs(&cascade, 0);
    assert!(runs.iter().any(|run| run.letter_box.is_some()), "{runs:?}");
    for run in &runs {
        document.text_run_opacity_group(&cascade, run);
    }
    let root = usize::try_from(runs[0].line.root.0).unwrap();
    let cache = document.page_projection.letter_groups.0.borrow();
    assert_eq!(cache.0, cascade.element_generation());
    assert_eq!(cache.1.len(), 1);
    let table = Arc::clone(&cache.1[&root]);
    drop(cache);
    assert!(Arc::ptr_eq(
        &table,
        &document.cached_letter_groups(&cascade, root)
    ));
}

#[test]
fn letter_groups_past_the_id_limit_stay_inside_one_parent_group() {
    let spans = crate::generated_content::MAX_LETTER_GROUPS + 1;
    let (document, cascade) = first_letter_spans(spans);
    let runs = document.page_text_runs(&cascade, 0);
    let root = usize::try_from(runs[0].line.root.0).unwrap();
    let table = document.letter_groups(&cascade, root);
    assert_eq!(table.fragments.len(), spans + 1);
    let mut parents = HashMap::new();
    for &(own, held) in table.fragments.values() {
        if let Some(own) = own {
            assert_eq!(table.around(own), held);
            assert_eq!(*parents.entry(own).or_insert(held), held);
        }
    }
    // The last span's quote and the paragraph's own letter come past the
    // limit, each with no group of its own inside its parent to share.
    assert_eq!(
        table
            .fragments
            .values()
            .filter(|(own, _)| own.is_none())
            .count(),
        2
    );
}
