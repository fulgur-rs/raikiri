//! Quadratic reference retained only for output comparisons in tests.

use shodo::Fragment;
use shodo::geometry::{Direction, PhysicalConverter, WritingMode};
use std::cell::Cell;

thread_local! {
    static HORIZONTAL_REFERENCE: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn horizontal_reference_enabled() -> bool {
    HORIZONTAL_REFERENCE.get()
}

pub(crate) fn with_reference_horizontal<R>(paint: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            HORIZONTAL_REFERENCE.set(self.0);
        }
    }
    let _restore = Restore(HORIZONTAL_REFERENCE.replace(true));
    paint()
}

/// The previous horizontal loop, retained only for real paint comparisons.
pub(super) fn horizontal_span(
    glyphs: &[raikiri_dom::LineGlyph],
    range: std::ops::Range<usize>,
) -> Option<super::glyph_ranges::GlyphSpan> {
    let mut start = f32::INFINITY;
    let mut end = f32::NEG_INFINITY;
    let mut found = false;
    for glyph in glyphs {
        super::tests::record_glyph_work(1);
        if !range.contains(&(glyph.cluster as usize)) {
            continue;
        }
        start = start.min(glyph.x);
        end = end.max(glyph.x + glyph.advance);
        found = true;
    }
    found.then_some(super::glyph_ranges::GlyphSpan { start, end })
}

fn slot_index(slot: shodo::EmphasisSlot, reversed: bool) -> u32 {
    if reversed {
        slot.count - 1 - slot.index
    } else {
        slot.index
    }
}

fn slot_center(slot: shodo::EmphasisSlot, reversed: bool) -> f32 {
    (slot_index(slot, reversed) as f32 + 0.5) / slot.count as f32
}

/// Test-only reference shared with the real paint fixtures.
pub(crate) fn vertical_mark_centers(
    mark: &shodo::EmphasisMark,
    (line, run): (&shodo::Line, &shodo::GlyphRunView<'_>),
    (writing_mode, converter): (WritingMode, &PhysicalConverter),
) -> Vec<(f32, f32)> {
    // Line-over is the block-start side, except in `vertical-lr`, whose
    // lines stack left to right with their line-over side on the right.
    let over = if writing_mode == WritingMode::VerticalLr {
        1.0
    } else {
        -1.0
    };
    let side = if mark.line_over { over } else { -over };
    let reach = side * (mark.offset + mark.font_size / 2.0);
    let mut centers = Vec::new();
    if run.orientation() == shodo::GlyphOrientation::Combined {
        // A square can span several runs, and its first characters may have
        // no glyph (a preserved tab); only the earliest run inside it marks
        // it, so it takes one mark.
        let text = run.text_range();
        for combination in line.text_combinations() {
            let range = &combination.text_range;
            let first = line
                .fragments()
                .filter_map(|fragment| match fragment {
                    Fragment::GlyphRun(run) => Some(run.text_range()),
                    _ => None,
                })
                .filter(|run| run.start < range.end && range.start < run.end)
                .min_by_key(|run| run.start);
            if first == Some(text.clone()) {
                let square = combination.square;
                centers.push(converter.point(
                    square.inline_start + square.inline_size / 2.0,
                    line.block_offset() + square.block_start + square.block_size / 2.0 + reach,
                ));
            }
        }
        return centers;
    }
    // Clusters follow the run's text, which bidi reordering can reverse;
    // lay their advances out in the order their glyphs are drawn.
    let glyphs: Vec<shodo::Glyph> = run.glyphs().collect();
    let mut clusters: Vec<(f32, shodo::Cluster)> = run
        .clusters()
        .filter_map(|cluster| {
            let key = glyphs
                .iter()
                .filter(|glyph| cluster.text_range.contains(&(glyph.cluster as usize)))
                .map(|glyph| glyph.inline_position)
                .reduce(f32::min)?;
            Some((key, cluster))
        })
        .collect();
    clusters.sort_by(|a, b| a.0.total_cmp(&b.0));
    let block = line.block_offset() + run.baseline() + reach;
    // A run against the line's direction lays its characters out backward.
    let reversed = (run.bidi_level() % 2 == 1) != (line.used_direction() == Direction::Rtl);
    let mut inline = run.inline_start();
    for (_, cluster) in clusters {
        for slot in run.emphasis_slots(&cluster) {
            centers.push(converter.point(
                inline + cluster.advance * slot_center(slot, reversed),
                block,
            ));
        }
        inline += cluster.advance;
    }
    centers
}
