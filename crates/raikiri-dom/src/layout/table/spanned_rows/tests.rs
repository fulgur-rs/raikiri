use super::*;

/// Collapse patterns, heights and gaps from a small generator, compared
/// with what visiting the rows gives for every range of rows.
#[test]
fn lookups_match_a_visit_of_every_row() {
    let mut state = 0x2545_f491_u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let sizes = [0.0, 10.0, 33.333, 0.1, 7.7, 1.0e-3];
    for _ in 0..200 {
        let rows = 1 + next() as usize % 12;
        let collapsed: Vec<bool> = (0..rows).map(|_| next() % 3 == 0).collect();
        let heights: Vec<f32> = (0..rows)
            .map(|_| sizes[next() as usize % sizes.len()])
            .collect();
        let gaps: Vec<f32> = (0..rows)
            .map(|_| if next() % 2 == 0 { 0.0 } else { 2.5 })
            .collect();
        let mut spanned = SpannedRows::new(&collapsed, &heights, &gaps);
        for start in 0..=rows {
            for end in start..=rows {
                let range = start..end;
                assert_eq!(
                    spanned.any_collapsed(range.clone()),
                    collapsed[range.clone()].iter().any(|&c| c),
                    "{collapsed:?} {range:?}"
                );
                assert_eq!(
                    spanned.last_kept(range.clone()),
                    range.clone().rev().find(|&row| !collapsed[row]),
                    "{collapsed:?} {range:?}"
                );
                assert_eq!(
                    spanned.nonzero_gaps(range.clone()),
                    gaps[range.clone()].iter().filter(|g| **g != 0.0).count(),
                    "{gaps:?} {range:?}"
                );
                let expected = heights[range.clone()].iter().sum::<f32>();
                // Twice, the second time from the stored sum.
                for _ in 0..2 {
                    assert_eq!(
                        spanned.height(range.clone()).to_bits(),
                        expected.to_bits(),
                        "{heights:?} {range:?}"
                    );
                }
            }
        }
    }
}
