use super::*;

fn direct(glyphs: &[(u32, f32)], range: Range<usize>) -> Option<f32> {
    glyphs
        .iter()
        .filter(|(offset, _)| range.contains(&(*offset as usize)))
        .map(|(_, position)| *position)
        .reduce(f32::min)
}

#[test]
fn all_ranges_match_a_direct_scan_with_duplicate_and_unordered_offsets() {
    for n in 0..32 {
        let glyphs: Vec<_> = (0..n)
            .rev()
            .map(|i| ((i / 3) as u32, ((i * 17) % 11) as f32 - 6.0))
            .collect();
        let index = GlyphRanges::new(glyphs.iter().copied());
        for start in 0..15 {
            for end in 0..15 {
                assert_eq!(
                    index.query(start..end),
                    direct(&glyphs, start..end),
                    "n={n}, range={start}..{end}"
                );
            }
        }
    }
}

#[test]
fn nested_repeated_ranges_and_offset_boundaries_keep_the_same_minima() {
    let glyphs = [
        (8, 4.0),
        (2, 7.0),
        (2, -3.0),
        (6, -3.0),
        (0, 9.0),
        (12, 0.0),
    ];
    let index = GlyphRanges::new(glyphs.into_iter());
    for range in [
        0..13,
        2..8,
        2..3,
        6..9,
        8..12,
        2..8,
        3..6,
        6..6,
        13..usize::MAX,
        0..usize::MAX,
    ] {
        assert_eq!(index.query(range.clone()), direct(&glyphs, range));
    }
}

#[test]
fn overlapping_ranges_have_logarithmic_query_work() {
    for n in [32_usize, 64, 128, 256, 512, 1024] {
        super::super::tests::GLYPH_WORK.with(|work| work.set(0));
        let index = GlyphRanges::new((0..n).rev().map(|i| (i as u32, -(i as f32))));
        for i in 0..n {
            assert_eq!(index.query(i / 2..n), Some(-((n - 1) as f32)));
        }
        let work = super::super::tests::GLYPH_WORK.with(std::cell::Cell::get);
        assert!(
            work <= 8 * n * (n.ilog2() as usize + 1),
            "n={n}, work={work}"
        );
        assert_eq!(index.offsets.len(), n);
        assert_eq!(index.summaries.len(), 2 * n);
    }
}

#[test]
fn horizontal_spans_match_direct_scans_for_all_ranges() {
    for n in 0..32 {
        let glyphs: Vec<_> = (0..n)
            .rev()
            .map(|i| {
                let start = ((i * 17) % 11) as f32 - 6.0;
                let advance = (i % 5) as f32 - 2.0;
                (
                    (i / 3) as u32,
                    GlyphSpan {
                        start,
                        end: start + advance,
                    },
                )
            })
            .collect();
        let index = GlyphRanges::new(glyphs.iter().copied());
        for start in 0..15 {
            for end in 0..15 {
                let mut found = false;
                let mut left = f32::INFINITY;
                let mut right = f32::NEG_INFINITY;
                for &(offset, span) in &glyphs {
                    if (start..end).contains(&(offset as usize)) {
                        left = left.min(span.start);
                        right = right.max(span.end);
                        found = true;
                    }
                }
                assert_eq!(
                    index.query(start..end),
                    found.then_some(GlyphSpan {
                        start: left,
                        end: right
                    }),
                    "n={n} range={start}..{end}"
                );
            }
        }
    }
}
