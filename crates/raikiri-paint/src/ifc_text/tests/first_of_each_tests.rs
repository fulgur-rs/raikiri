use super::super::first_of_each;

#[test]
fn an_item_is_kept_when_no_earlier_item_has_its_key() {
    let items = [(1, 'a'), (2, 'b'), (1, 'c'), (3, 'd'), (2, 'e'), (1, 'f')];
    assert_eq!(
        first_of_each(items, |item| item.0),
        [(1, 'a'), (2, 'b'), (3, 'd')]
    );
    assert_eq!(first_of_each(Vec::<(i32, char)>::new(), |item| item.0), []);
}

#[test]
fn the_key_can_be_where_an_item_points_to() {
    // Two lines with the same content are two lines; one line seen twice is
    // one.
    let lines = [String::from("same"), String::from("same")];
    let items = [&lines[0], &lines[1], &lines[0], &lines[1], &lines[0]];
    let kept = first_of_each(items, |line| std::ptr::from_ref(*line));
    assert_eq!(kept.len(), 2);
    assert!(std::ptr::eq(kept[0], &lines[0]) && std::ptr::eq(kept[1], &lines[1]));
}

#[test]
fn the_items_kept_are_the_ones_a_scan_of_the_kept_items_keeps() {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let (mut dropped, mut kept) = (0, 0);
    for _ in 0..1_000 {
        let items: Vec<(usize, usize)> = (0..next(30)).map(|n| (next(12), n)).collect();
        let mut by_scan: Vec<(usize, usize)> = Vec::new();
        for item in &items {
            if !by_scan.iter().any(|seen| seen.0 == item.0) {
                by_scan.push(*item);
            }
        }
        let first = first_of_each(items.iter().copied(), |item| item.0);
        assert_eq!(first, by_scan, "{items:?}");
        kept += first.len();
        dropped += items.len() - first.len();
    }
    // The cases reach both what is kept and what is dropped.
    assert!(
        kept > 1_000 && dropped > 1_000,
        "kept {kept}, dropped {dropped}"
    );
}

#[test]
fn many_distinct_keys_are_kept_in_linear_time() {
    let count = 200_000;
    let items: Vec<usize> = (0..count).collect();
    let start = std::time::Instant::now();
    let kept = first_of_each(items.iter().copied(), |&item| item);
    let elapsed = start.elapsed();
    assert_eq!(kept, items);
    // Looking each key up takes milliseconds; scanning the kept items for
    // every item takes seconds at this size.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "{elapsed:?} for {count} items"
    );
}
