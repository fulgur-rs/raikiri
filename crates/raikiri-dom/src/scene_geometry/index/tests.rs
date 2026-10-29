use super::*;
use std::cell::Cell;
thread_local! { static VISITS: Cell<usize> = const { Cell::new(0) }; }
pub(super) fn count_visit() {
    VISITS.with(|v| v.set(v.get() + 1));
}
#[test]
fn candidates_conservatively_include_the_exact_float_predicate() {
    let values = [
        f32::NEG_INFINITY,
        -f32::MAX,
        -1000.0,
        -1.0,
        -f32::MIN_POSITIVE,
        -0.0,
        0.0,
        f32::from_bits(1),
        1.0,
        1.0_f32.next_up(),
        1000.0,
        f32::MAX,
        f32::INFINITY,
        f32::NAN,
    ];
    let mut pieces = Vec::new();
    for start in values {
        for extent in values {
            pieces.push((0, (start, start, extent, extent)));
        }
    }
    for horizontal in [false, true] {
        let index = RectIndex::new(&pieces, horizontal);
        for translation in values {
            for end in values {
                let candidates = index.query(translation, end);
                for (i, &(_, (start, _, extent, _))) in pieces.iter().enumerate() {
                    let start = start + translation;
                    let extent = extent.max(0.0);
                    let selected = if extent == 0.0 {
                        start >= 0.0 && start <= end
                    } else {
                        start < end && start + extent > 0.0
                    };
                    assert!(
                        !selected || candidates.contains(&i),
                        "{start:?} {extent:?} {translation:?} {end:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn sparse_piece_queries_do_not_scan_prior_or_later_pieces() {
    for count in [100, 200, 400, 800] {
        let pieces: Vec<_> = (0..count)
            .map(|i| (0, (0.0, i as f32 * 440.0, 20.0, 10.0)))
            .collect();
        let index = RectIndex::new(&pieces, false);
        VISITS.with(|v| v.set(0));
        for i in 0..count {
            assert_eq!(index.query(-(i as f32 * 440.0), 440.0), vec![i]);
        }
        let visits = VISITS.with(Cell::get);
        eprintln!("index-work pieces={count} visits={visits}");
        assert!(visits < count * 32);
    }
}

pub(super) fn take_visits() -> usize {
    VISITS.with(|v| v.replace(0))
}
