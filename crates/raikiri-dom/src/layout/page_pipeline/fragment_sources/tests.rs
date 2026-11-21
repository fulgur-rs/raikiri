use super::*;

/// A page `page_index` whose flow range is `range`.
fn page(page_index: u32, range: (f32, f32)) -> PageFragment {
    let mut page = PageFragment::new();
    page.page_index = page_index;
    page.flow_range = Some(range);
    page
}

#[test]
fn a_header_origin_shared_by_an_empty_page_lands_on_the_page_after_it() {
    // Page 4 is empty and starts where page 5 does.
    let pages = [
        page(3, (0.0, 100.0)),
        page(4, (100.0, 100.0)),
        page(5, (100.0, 200.0)),
        page(6, (200.0, 300.0)),
    ];
    let slots = FlowSlots::new(&pages);
    assert_eq!(slots.starting_at(100.0, &pages), vec![(2, 100.0)]);
    assert_eq!(slots.starting_at(200.0, &pages), vec![(3, 200.0)]);
    assert_eq!(slots.starting_at(150.0, &pages), Vec::new());
}

#[test]
fn a_header_origin_is_found_among_unordered_pages() {
    let pages = [page(3, (100.0, 200.0)), page(4, (0.0, 100.0))];
    let slots = FlowSlots::new(&pages);
    assert_eq!(slots.starting_at(0.0, &pages), vec![(1, 0.0)]);
}
