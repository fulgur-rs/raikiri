use super::*;
use crate::page_projection::continuation::CrossingBox;
use fragment_sources::FlowSlots;
use std::collections::HashMap;

/// A 10px wide box of `height` at `abs_y` in the flow, with no lines.
fn source(abs_y: f32, height: f32) -> PageFragmentSource {
    PageFragmentSource {
        node_id: NodeId::new(1),
        node_kind: NodeKind::Element,
        is_img: false,
        abs_x: 5.0,
        abs_y,
        width: 10.0,
        height,
        content_insets: None,
        line_metrics: None,
        lines_along_x: false,
        line_start_offset: 0,
        is_repeat: false,
        fragmentainer: 0,
        fragment_clip: None,
        overflow_chain: None,
        own_overflow_source: None,
    }
}

/// One empty page per range, `None` for a page with no flow range.
fn pages_with_ranges(ranges: &[Option<(f32, f32)>]) -> Vec<PageFragment> {
    ranges
        .iter()
        .enumerate()
        .map(|(index, &flow_range)| {
            let mut page = PageFragment::new();
            page.page_index = index as u32;
            page.flow_range = flow_range;
            page
        })
        .collect()
}

/// Three pages of 100px, from 0 to 300.
fn three_pages() -> Vec<PageFragment> {
    pages_with_ranges(&[
        Some((0.0, 100.0)),
        Some((100.0, 200.0)),
        Some((200.0, 300.0)),
    ])
}

/// The slots of the pages `source` reaches.
fn reached(slots: &FlowSlots, source: &PageFragmentSource) -> Vec<usize> {
    slots
        .reached_by(source)
        .iter()
        .map(|&(page_slot, _, _)| page_slot)
        .collect()
}

/// Puts `sources` on `pages`, with room for any amount of work.
fn place(pages: &mut [PageFragment], sources: &[PageFragmentSource]) {
    let control = PageLayoutControl::new(None);
    let mut work = ProjectionWork::new(&control, usize::MAX);
    place_sources(&Document::new(), sources, pages, FlowAxis::Y, &mut work).unwrap();
}

#[test]
fn a_source_reaches_the_pages_its_extent_overlaps() {
    let pages = three_pages();
    let slots = FlowSlots::new(&pages);
    assert_eq!(reached(&slots, &source(50.0, 100.0)), [0, 1]);
    // A box ending where a page starts, or starting where one ends, stays
    // off that page.
    assert_eq!(reached(&slots, &source(100.0, 100.0)), [1]);
    assert_eq!(reached(&slots, &source(0.0, 300.0)), [0, 1, 2]);
    // Boxes above the first page or below the last one reach none.
    assert!(reached(&slots, &source(-50.0, 40.0)).is_empty());
    assert!(reached(&slots, &source(300.0, 40.0)).is_empty());
}

#[test]
fn a_source_without_height_reaches_the_pages_that_hold_its_position() {
    let pages = three_pages();
    let slots = FlowSlots::new(&pages);
    // On the edge between two pages, both hold it.
    assert_eq!(reached(&slots, &source(100.0, 0.0)), [0, 1]);
    assert_eq!(reached(&slots, &source(150.0, 0.0)), [1]);
    assert!(reached(&slots, &source(-1.0, 0.0)).is_empty());
}

#[test]
fn pages_without_a_flow_range_hold_no_sources() {
    let pages = pages_with_ranges(&[Some((0.0, 100.0)), None, Some((100.0, 200.0))]);
    let slots = FlowSlots::new(&pages);
    assert_eq!(reached(&slots, &source(50.0, 100.0)), [0, 2]);
}

#[test]
fn a_repeated_source_or_ranges_out_of_order_reach_every_page() {
    let pages = three_pages();
    let slots = FlowSlots::new(&pages);
    let repeated = PageFragmentSource {
        is_repeat: true,
        ..source(50.0, 10.0)
    };
    assert_eq!(reached(&slots, &repeated), [0, 1, 2]);

    // A range that starts before the one ahead of it breaks the order the
    // binary search relies on.
    let unordered = FlowSlots::new(&pages_with_ranges(&[
        Some((100.0, 200.0)),
        Some((0.0, 100.0)),
    ]));
    assert_eq!(reached(&unordered, &source(150.0, 10.0)), [0, 1]);
}

#[test]
fn a_box_is_cut_into_a_fragment_on_each_page_it_crosses() {
    let mut pages = three_pages();
    place(&mut pages, &[source(70.0, 150.0)]);
    let rows = |page: &PageFragment| {
        page.items
            .iter()
            .map(|item| {
                (
                    item.rect.y,
                    item.rect.height,
                    item.box_y,
                    item.fragment_index,
                    item.fragment_count,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&pages[0]), [(70.0, 30.0, 70.0, 0, 3)]);
    assert_eq!(rows(&pages[1]), [(0.0, 100.0, -30.0, 1, 3)]);
    assert_eq!(rows(&pages[2]), [(0.0, 20.0, -130.0, 2, 3)]);
}

#[test]
fn a_repeated_source_lands_whole_on_every_page() {
    let mut pages = three_pages();
    let repeated = PageFragmentSource {
        is_repeat: true,
        line_metrics: Some(vec![(0.0, 10.0), (10.0, 20.0)]),
        ..source(20.0, 30.0)
    };
    place(&mut pages, &[repeated]);
    for page in &pages {
        let [item] = page.items.as_slice() else {
            panic!("one item per page, got {:?}", page.items);
        };
        assert_eq!((item.rect.y, item.rect.height), (20.0, 30.0));
        assert!(item.is_repeat);
        let lines = item.line_range.unwrap();
        assert_eq!((lines.start, lines.end), (0, 2));
    }
}

#[test]
fn the_lines_of_a_text_go_to_the_page_that_holds_their_centers() {
    let mut pages = three_pages();
    // Four 10px lines from y=70 on: three end on the first page.
    let text = PageFragmentSource {
        node_kind: NodeKind::Text,
        line_metrics: Some(vec![(0.0, 10.0), (10.0, 20.0), (20.0, 30.0), (30.0, 40.0)]),
        // The first line of this source is the sixth of its paragraph.
        line_start_offset: 5,
        ..source(70.0, 40.0)
    };
    place(&mut pages, &[text]);
    let range = |page: &PageFragment| {
        let lines = page.items[0].line_range.unwrap();
        (lines.start, lines.end)
    };
    assert_eq!(range(&pages[0]), (5, 8));
    assert_eq!(range(&pages[1]), (8, 9));
    assert!(pages[2].items.is_empty());
    assert_eq!(pages[0].items[0].kind, PageFragmentKind::Text);
}

#[test]
fn an_image_is_a_replaced_fragment_and_a_box_without_height_lands_on_both_pages_of_its_edge() {
    let mut pages = three_pages();
    let image = PageFragmentSource {
        is_img: true,
        node_id: NodeId::new(2),
        ..source(100.0, 0.0)
    };
    place(&mut pages, &[image]);
    for page in &pages[..2] {
        let [item] = page.items.as_slice() else {
            panic!(
                "the edge holds the image on both pages, got {:?}",
                page.items
            );
        };
        assert_eq!(item.kind, PageFragmentKind::Replaced);
        assert_eq!(item.rect.height, 0.0);
    }
    assert_eq!(pages[0].items[0].rect.y, 100.0);
    assert_eq!(pages[1].items[0].rect.y, 0.0);
    assert!(pages[2].items.is_empty());
}

#[test]
fn the_content_box_and_clip_of_a_fragment_follow_its_page() {
    let mut pages = three_pages();
    let clipped = PageFragmentSource {
        content_insets: Some(PageFragmentInsets::new(2.0, 3.0, 4.0, 5.0)),
        fragment_clip: Some(PaintRect::new(1.0, 60.0, 8.0, 80.0)),
        fragmentainer: 3,
        overflow_chain: Some(7),
        own_overflow_source: Some(9),
        ..source(70.0, 60.0)
    };
    place(&mut pages, &[clipped]);
    // The second fragment starts 30px above its page; the content box and the
    // clip keep their distance from the box.
    let item = &pages[1].items[0];
    let content = item.content_rect.unwrap();
    assert_eq!(
        (content.x, content.y, content.width, content.height),
        (10.0, -28.0, 2.0, 54.0)
    );
    let clip = item.fragment_clip.unwrap();
    assert_eq!(
        (clip.x, clip.y, clip.width, clip.height),
        (1.0, -40.0, 8.0, 80.0)
    );
    assert_eq!(
        (
            item.fragmentainer,
            item.overflow_chain,
            item.own_overflow_source
        ),
        (3, Some(7), Some(9))
    );
}

/// A box of node `node` that crosses the start of its page by `-top`, with
/// `earlier_fragments` fragments before it.
fn crossing(node: u64, earlier_fragments: u32, top: f32) -> (NodeId, CrossingBox) {
    let node = NodeId::new(node);
    (
        node,
        CrossingBox {
            node,
            earlier_fragments,
            top,
        },
    )
}

#[test]
fn a_box_crossing_the_start_of_a_resumed_page_keeps_the_top_it_had() {
    let mut doc = Document::new();
    doc.earlier_fragments = Some(HashMap::from([crossing(1, 1, -20.0)]));
    let pages = pages_with_ranges(&[Some((100.0, 200.0))]);
    let restore = |doc: &Document, sources: &mut [PageFragmentSource]| {
        restore_crossing_tops(doc, sources, &pages);
        sources
            .iter()
            .map(|source| (source.abs_y, source.height))
            .collect::<Vec<_>>()
    };
    // Its end stays where it was.
    assert_eq!(restore(&doc, &mut [source(100.0, 50.0)]), [(80.0, 70.0)]);
    // A repeated source, a box that starts below the page start and a box the
    // earlier layout did not cross are left alone.
    let others = [
        PageFragmentSource {
            is_repeat: true,
            ..source(100.0, 50.0)
        },
        source(130.0, 50.0),
        PageFragmentSource {
            node_id: NodeId::new(2),
            ..source(100.0, 50.0)
        },
    ];
    assert_eq!(
        restore(&doc, &mut others.clone()),
        [(100.0, 50.0), (130.0, 50.0), (100.0, 50.0)]
    );
    // A box that ends above its restored top has no height left.
    assert_eq!(restore(&doc, &mut [source(50.0, 10.0)]), [(80.0, 0.0)]);
    // Nothing is restored in a layout that is not resumed, or on pages with no
    // range to start from.
    assert_eq!(
        restore(&Document::new(), &mut [source(100.0, 50.0)]),
        [(100.0, 50.0)]
    );
    let mut rangeless = [source(100.0, 50.0)];
    restore_crossing_tops(&doc, &mut rangeless, &pages_with_ranges(&[None]));
    assert_eq!(rangeless[0].abs_y, 100.0);
}

/// An item of `node` whose box starts `flow_box_y` into the page.
fn item(node: u64, flow_box_y: f32) -> PageFragmentItem {
    PageFragmentItem::new(
        NodeId::new(node),
        PageFragmentRect::new(0.0, 0.0, 10.0, 10.0),
        PageFragmentKind::Box,
        0,
        1,
        false,
    )
    .with_flow_box_y(flow_box_y)
}

#[test]
fn the_body_has_no_fragment_on_the_first_page_of_a_resumed_layout_unless_it_crossed_the_start() {
    let page_with_items = || {
        let mut pages = pages_with_ranges(&[Some((0.0, 100.0))]);
        pages[0].items = vec![item(7, -5.0), item(7, 0.0), item(8, -5.0)];
        pages
    };
    let kept = |pages: &[PageFragment]| {
        pages[0]
            .items
            .iter()
            .map(|item| (item.node_id.0, item.flow_box_y))
            .collect::<Vec<_>>()
    };
    // The body is node 7. Without a resumed layout nothing is dropped.
    let mut doc = Document::new();
    let mut pages = page_with_items();
    drop_uncrossed_body(&doc, &mut pages, 7);
    assert_eq!(kept(&pages), [(7, -5.0), (7, 0.0), (8, -5.0)]);
    // In one, the body fragment that starts above the page goes.
    doc.earlier_fragments = Some(HashMap::new());
    let mut pages = page_with_items();
    drop_uncrossed_body(&doc, &mut pages, 7);
    assert_eq!(kept(&pages), [(7, 0.0), (8, -5.0)]);
    // A body that crossed the start in the earlier layout keeps it.
    doc.earlier_fragments = Some(HashMap::from([crossing(7, 1, -5.0)]));
    let mut pages = page_with_items();
    drop_uncrossed_body(&doc, &mut pages, 7);
    assert_eq!(kept(&pages), [(7, -5.0), (7, 0.0), (8, -5.0)]);
}

/// The `(index, count)` numbers of the items of each page.
fn numbers(pages: &[PageFragment]) -> Vec<Vec<(u64, u32, u32)>> {
    pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .map(|item| (item.node_id.0, item.fragment_index, item.fragment_count))
                .collect()
        })
        .collect()
}

/// Three pages holding node 1 on the first and second and as two column
/// pieces on the third, and node 2 on the second.
fn pages_of_two_nodes() -> Vec<PageFragment> {
    let mut pages = three_pages();
    pages[0].items = vec![item(1, 0.0)];
    pages[1].items = vec![item(2, 0.0), item(1, 0.0)];
    pages[2].items = vec![item(1, 0.0), item(1, 0.0)];
    pages
}

#[test]
fn the_fragments_of_a_node_are_numbered_through_the_pages_in_order() {
    let mut pages = pages_of_two_nodes();
    number_fragments(&Document::new(), &mut pages);
    assert_eq!(
        numbers(&pages),
        [
            vec![(1, 0, 4)],
            vec![(2, 0, 1), (1, 1, 4)],
            vec![(1, 2, 4), (1, 3, 4)],
        ]
    );
}

#[test]
fn the_fragments_of_a_node_count_those_of_the_layouts_around_it() {
    let mut doc = Document::new();
    // Node 1 had three fragments before the page the layout resumes at, and
    // nodes 1 and 2 go on for two and one fragments after the last page.
    doc.earlier_fragments = Some(HashMap::from([crossing(1, 3, -5.0)]));
    doc.later_fragments = HashMap::from([(NodeId::new(1), 2), (NodeId::new(2), 1)]);
    let mut pages = pages_of_two_nodes();
    number_fragments(&doc, &mut pages);
    assert_eq!(
        numbers(&pages),
        [
            vec![(1, 3, 9)],
            vec![(2, 0, 2), (1, 4, 9)],
            vec![(1, 5, 9), (1, 6, 9)],
        ]
    );
}

#[test]
fn only_the_boxes_that_clip_their_overflow_are_kept_as_clip_sources() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let block = |doc: &mut Document, css: &str| {
        doc.append_element(Some(body), "div", Style::default(), Some(css))
    };
    let clipping = block(
        &mut doc,
        "display:block;overflow:hidden;width:50px;height:40px",
    );
    let plain = block(&mut doc, "display:block;width:50px;height:40px");
    let repeated = block(
        &mut doc,
        "display:block;overflow:hidden;width:50px;height:40px",
    );
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
    let sources: Vec<_> = [clipping, plain, repeated]
        .into_iter()
        .map(|id| PageFragmentSource {
            node_id: NodeId::new(id as u64),
            // The clip repeats on every page when its source does.
            is_repeat: id == clipping,
            ..source(30.0, 40.0)
        })
        .collect();
    let clips = overflow_clip_sources(&doc, &cascade, &sources, &HashSet::from([repeated]));
    assert_eq!(clips.len(), 1, "{:?}", clips.keys());
    let clip = &clips[&NodeId::new(clipping as u64)];
    assert_eq!(
        (
            clip.border_box.x,
            clip.border_box.y,
            clip.border_box.width,
            clip.border_box.height
        ),
        (5.0, 30.0, 10.0, 40.0)
    );
    assert!(clip.is_repeat);
}
