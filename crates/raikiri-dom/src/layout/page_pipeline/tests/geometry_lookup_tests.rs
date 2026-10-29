use super::*;
use raikiri_style::{build_rule_tree, cascade};

fn geometry(index: u32, seed: u16) -> PageFragmentPageGeometry {
    let x = f32::from(seed);
    let mut page = PageBox::A4;
    page.width = 500.0 + x;
    page.height = 700.0 + x;
    PageFragmentPageGeometry::new(
        index,
        page,
        PageFragmentInsets::new(x, x + 1.0, x + 2.0, x + 3.0),
        PageFragmentInsets::new(1.0, 2.0, 3.0, x + 4.0),
        PageFragmentRect::new(x + 5.0, x + 6.0, 300.0 + x, 400.0 + x),
        PageFragmentOrientation::Landscape,
    )
}

fn slice(index: u32, origin: f32) -> PageSlice {
    PageSlice {
        page_index: index,
        content_origin_y: origin,
        page_name: None,
    }
}

fn assert_geometry(page: &PageFragment, expected: PageFragmentPageGeometry) {
    assert_eq!(page.page_index, expected.page_index);
    assert_eq!(page.page_box, expected.page_box);
    assert_eq!(page.margins, expected.margins);
    assert_eq!(page.content_insets, expected.content_insets);
    assert_eq!(page.content_box, expected.content_box);
    assert_eq!(page.orientation, expected.orientation);
}

#[test]
fn geometry_lookup_preserves_sparse_missing_and_first_duplicate_metadata() {
    let document = Document::new();
    let cascade = cascade(&document, &build_rule_tree(&document)).expect("cascade");
    let geometries = [
        geometry(u32::MAX, 1),
        geometry(8, 2),
        geometry(2, 3),
        geometry(8, 4),
    ];
    let slices = [
        slice(u32::MAX, 500.0),
        slice(8, 300.0),
        slice(0, 0.0),
        slice(8, 200.0),
        slice(2, 100.0),
        slice(9, 400.0),
    ];
    let (pages, _) = build_page_fragments(&document, &cascade, PageBox::A4, &slices, &geometries);
    assert_eq!(pages.len(), 6);
    assert_geometry(&pages[1], geometries[2]);
    assert_geometry(&pages[2], geometries[1]);
    assert_geometry(&pages[3], geometries[1]);
    assert_geometry(&pages[5], geometries[0]);
    assert_eq!(pages[2].content_origin_y, 200.0);
    assert_eq!(pages[3].content_origin_y, 300.0);
    // A4 has no page-rule margins or padding in this empty document.
    for slot in [0, 4] {
        let page = &pages[slot];
        assert_eq!(page.page_index, if slot == 0 { 0 } else { 9 });
        assert_eq!(page.page_box, PageBox::A4);
        assert_eq!(page.margins, PageFragmentInsets::default());
        assert_eq!(page.content_insets, PageFragmentInsets::default());
        assert_eq!(
            page.content_box,
            PageFragmentRect::new(0.0, 0.0, PageBox::A4.width, PageBox::A4.height)
        );
        assert_eq!(page.orientation, PageFragmentOrientation::Portrait);
    }
    let (empty, _) = build_page_fragments(&document, &cascade, PageBox::A4, &[], &geometries);
    assert!(empty.is_empty());
    let (fallback, _) =
        build_page_fragments(&document, &cascade, PageBox::A4, &[slice(7, 0.0)], &[]);
    assert_eq!(fallback[0].page_index, 7);
    assert_eq!(fallback[0].page_box, PageBox::A4);
}

#[test]
fn geometry_lookup_scales_with_page_count_and_keeps_all_page_metadata() {
    let document = Document::new();
    let cascade = cascade(&document, &build_rule_tree(&document)).expect("cascade");
    let mut subquadratic = true;
    for n in [100_u16, 1000, 2000, 4000] {
        let geometries: Vec<_> = (0..n).map(|i| geometry(u32::from(i), i)).collect();
        let slices: Vec<_> = (0..n)
            .map(|i| slice(u32::from(i), f32::from(i) * 500.0))
            .collect();
        for reversed in [false, true] {
            let mut input = geometries.clone();
            if reversed {
                input.reverse();
            }
            GEOMETRY_COMPARISONS.set(0);
            let start = std::time::Instant::now();
            let (pages, _) =
                build_page_fragments(&document, &cascade, PageBox::A4, &slices, &input);
            let elapsed = start.elapsed();
            let comparisons = GEOMETRY_COMPARISONS.get();
            eprintln!(
                "geometry pages={n} reversed={reversed} comparisons={comparisons} elapsed={elapsed:?}"
            );
            assert_eq!(pages.len(), usize::from(n));
            for (page, expected) in pages.iter().zip(&geometries) {
                assert_geometry(page, *expected);
            }
            let bound = usize::from(n) * (2 * n.ilog2() as usize + 4);
            subquadratic &= comparisons <= bound;
        }
    }
    assert!(
        subquadratic,
        "page geometry lookup must not rescan each prefix"
    );
}
