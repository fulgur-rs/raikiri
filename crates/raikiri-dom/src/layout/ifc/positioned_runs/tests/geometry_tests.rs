use super::super::geometry::{OffsetRanges, PositionedLineGeometry};
use super::*;

#[test]
fn sparse_offsets_preserve_last_range_wins_and_holes() {
    let mut ranges = OffsetRanges::default();
    ranges.insert(2..9, (1.0, 2.0));
    ranges.insert(4..6, (3.0, 4.0));
    ranges.insert(8..12, (5.0, 6.0));
    let ranges = ranges.finish();
    let at = |line| super::super::geometry::offset_at(&ranges, line);
    assert_eq!(at(1), None);
    assert_eq!(at(2), Some((1.0, 2.0)));
    assert_eq!(at(4), Some((3.0, 4.0)));
    assert_eq!(at(6), Some((1.0, 2.0)));
    assert_eq!(at(8), Some((5.0, 6.0)));
    assert_eq!(at(11), Some((5.0, 6.0)));
    assert_eq!(at(12), None);
    assert_eq!(ranges.len(), 4);
}

#[test]
fn sparse_offsets_store_ranges_instead_of_the_spanned_area() {
    let mut ranges = OffsetRanges::default();
    ranges.insert(0..usize::MAX, (0.0, 0.0));
    ranges.insert(100..200, (10.0, 20.0));
    assert_eq!(ranges.finish().len(), 3);
}

#[test]
fn prepared_geometry_matches_fresh_columns_origins_and_overlaps() {
    for recorded in [false, true] {
        let (mut doc, cascade, root) = two_lines();
        let ifc = doc.nodes[root].ifc.as_mut().unwrap();
        ifc.multicol_fragment_origins_recorded = recorded;
        ifc.multicol_fragments = Some(vec![
            MulticolTextFragment {
                line_start: 0,
                line_end: 2,
                fragmentainer: 0,
                x: 10.0,
                y: 20.0,
            },
            MulticolTextFragment {
                line_start: 1,
                line_end: 20,
                fragmentainer: 1,
                x: 30.0,
                y: 40.0,
            },
            MulticolTextFragment {
                line_start: 9,
                line_end: 10,
                fragmentainer: 2,
                x: 0.0,
                y: 0.0,
            },
        ]);
        let geometry = PositionedLineGeometry::new(&doc, root).unwrap();
        for column in [None, Some(0), Some(1), Some(2), Some(99)] {
            let fresh = PositionedLines::new(&doc, &cascade, root, column).unwrap();
            let prepared = geometry.view(&doc, &cascade, column).unwrap();
            let actual: Vec<_> = prepared
                .indices()
                .map(|index| {
                    let line = prepared.line_at(index).unwrap();
                    (
                        index,
                        line.offset,
                        line.runs
                            .iter()
                            .flat_map(|run| {
                                run.glyphs.iter().map(|glyph| (glyph.id, glyph.x, glyph.y))
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            let expected: Vec<_> = fresh
                .lines()
                .map(|line| {
                    (
                        line.index,
                        line.offset,
                        line.runs
                            .iter()
                            .flat_map(|run| {
                                run.glyphs.iter().map(|glyph| (glyph.id, glyph.x, glyph.y))
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            assert_eq!(actual, expected);
        }
        assert!(geometry.range_count() <= 6);
    }
}

#[test]
fn unfragmented_geometry_selects_all_lines_for_any_column() {
    let (doc, cascade, root) = two_lines();
    let geometry = PositionedLineGeometry::new(&doc, root).unwrap();
    for column in [None, Some(999)] {
        let view = geometry.view(&doc, &cascade, column).unwrap();
        assert_eq!(view.indices().collect::<Vec<_>>(), [0, 1]);
        assert_eq!(view.line_offset(1), Some((0.0, 0.0)));
        assert!(view.line_at(2).is_none());
    }
}

#[test]
fn prepared_geometry_keeps_line_shifts_in_each_writing_mode() {
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        let (mut doc, cascade, root) = two_lines();
        let ifc = doc.nodes[root].ifc.as_mut().unwrap();
        ifc.writing_mode = mode;
        ifc.lines.as_mut().unwrap().shifts.push(1, 7.0);
        let geometry = PositionedLineGeometry::new(&doc, root).unwrap();
        let view = geometry.view(&doc, &cascade, None).unwrap();
        let fresh = PositionedLines::new(&doc, &cascade, root, None).unwrap();
        for index in 0..2 {
            assert_eq!(view.line_offset(index), fresh.line_offset(index));
        }
        assert_eq!(view.line_offset(1), Some(block_shift(mode, 7.0)));
    }
}
