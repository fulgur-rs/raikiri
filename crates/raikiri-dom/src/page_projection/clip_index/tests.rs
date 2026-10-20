use super::*;

#[test]
fn interval_queries_keep_overlaps_holes_and_large_shared_ranges_sparse() {
    let mut ranges = PlacementRanges::default();
    for (lines, placement) in [
        (0..usize::MAX, 0),
        (4..8, 1),
        (10..20, 2),
        (6..12, 3),
        (9..9, 4),
    ] {
        ranges.push(lines, placement);
    }
    ranges.finish();
    assert_eq!(ranges.len(), 4);
    for (line, expected) in [
        (0, vec![0]),
        (4, vec![0, 1]),
        (6, vec![0, 1, 3]),
        (8, vec![0, 3]),
        (9, vec![0, 3]),
        (10, vec![0, 2, 3]),
        (12, vec![0, 2]),
        (20, vec![0]),
        (usize::MAX, vec![]),
    ] {
        let mut actual = ranges.at(line);
        actual.sort_unstable();
        assert_eq!(actual, expected);
    }
}
