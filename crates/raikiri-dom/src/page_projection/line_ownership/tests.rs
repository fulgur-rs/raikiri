use super::*;

fn page(range: Option<(f32, f32)>, x_axis: bool) -> PageFragment {
    let mut page = PageFragment::new();
    page.flow_range = range;
    if x_axis {
        page.flow_axis = FlowAxis::X {
            right_to_left: false,
            width: 100.,
            page_width: 100.,
        };
    }
    page
}

#[test]
fn memoized_ownership_matches_the_original_predicate_for_arbitrary_ranges() {
    let ranges = [
        None,
        Some((0., 100.)),
        Some((49., 51.)),
        Some((40., 48.)),
        Some((100., 200.)),
        Some((0., 100.)),
        Some((300., 500.)),
        Some((100_000_000., 100_000_128.)),
        Some((0., f32::INFINITY)),
        Some((f32::NAN, 100.)),
        Some((50., 50.)),
    ];
    let lines = [
        (-10., 210.),
        (0., 200.),
        (45., 55.),
        (49.999, 50.001),
        (99.999, 100.),
        (100., 100.),
        (201., 220.),
        (0., -0.),
        (-0., 0.),
        (100., 0.),
        (f32::NAN, 100.),
        (0., f32::NAN),
        (f32::NEG_INFINITY, f32::INFINITY),
        (0., f32::INFINITY),
        (f32::MAX, f32::MAX),
        (100_000_032., 100_000_100.),
    ];
    for mixed in [false, true] {
        for rotation in 0..ranges.len() {
            let mut pages = ranges
                .iter()
                .enumerate()
                .map(|(i, &range)| page(range, mixed && i % 2 == 0))
                .collect::<Vec<_>>();
            pages.rotate_left(rotation);
            let mut ownership = LineOwnership::new(&pages);
            for _ in 0..2 {
                for &(top, bottom) in &lines {
                    for candidate in &pages {
                        assert_eq!(
                            ownership.owns(candidate, top, bottom),
                            candidate.owns_line(&pages, top, bottom),
                            "mixed={mixed},rotation={rotation},top={top},bottom={bottom},range={:?},axis={:?}",
                            candidate.flow_range,
                            candidate.flow_axis
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn first_match_height_and_negative_results_are_reused() {
    let broad = page(Some((0., 100.)), false);
    let narrow = page(Some((49., 51.)), true);
    let side = page(Some((40., 48.)), false);
    for narrow_first in [false, true] {
        let pages = if narrow_first {
            vec![narrow.clone(), broad.clone(), side.clone()]
        } else {
            vec![broad.clone(), narrow.clone(), side.clone()]
        };
        let mut ownership = LineOwnership::new(&pages);
        let before = super::super::work_tests::line_owner_probes();
        for _ in 0..8 {
            assert_eq!(ownership.owns(&side, 45., 55.), narrow_first);
        }
        assert_eq!(super::super::work_tests::line_owner_probes() - before, 1);
        assert_eq!(ownership.center_heights.len(), 1);
        let missing = page(Some((200., 210.)), false);
        let before = super::super::work_tests::line_owner_probes();
        for _ in 0..8 {
            assert!(!ownership.owns(&missing, 205., 215.));
        }
        assert_eq!(
            super::super::work_tests::line_owner_probes() - before,
            pages.len()
        );
        assert_eq!(
            ownership
                .center_heights
                .get(&(205f32.to_bits(), 215f32.to_bits())),
            Some(&None)
        );
    }
}

#[test]
fn center_axis_and_intersection_shortcircuits_need_no_memo_entry() {
    let pages = vec![
        page(Some((0., 100.)), false),
        page(Some((100., 200.)), false),
    ];
    let mut ownership = LineOwnership::new(&pages);
    assert!(ownership.owns(&pages[1], 0., 200.));
    assert!(!ownership.owns(&page(None, false), 0., 200.));
    assert!(!ownership.owns(&page(Some((0., 100.)), true), 0., 200.));
    assert!(!ownership.owns(&page(Some((300., 400.)), false), 0., 200.));
    assert!(ownership.center_heights.is_empty());
    assert!(ownership.owns(&pages[0], 0., 200.));
    assert_eq!(ownership.center_heights.len(), 1);
}
