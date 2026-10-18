use super::*;
use fragment_sources::FlowSlots;

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
