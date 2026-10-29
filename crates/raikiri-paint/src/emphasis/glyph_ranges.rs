//! Per-run range summaries over glyph source offsets.
//!
//! Sorting offsets once and querying an iterative tree costs
//! O(G log G + C log G), with O(G) storage. Duplicate offsets and overlapping
//! or unordered source ranges are retained. The scalar summary keeps the
//! vertical path's storage unchanged; horizontal spans retain both edges.
//! Layout produces finite coordinates; this is not a general float reduction API.

use std::ops::Range;

/// A commutative layout-coordinate summary with an empty identity.
pub(super) trait Summary: Copy {
    const EMPTY: Self;
    fn combine(self, other: Self) -> Self;
}

impl Summary for f32 {
    const EMPTY: Self = f32::INFINITY;

    fn combine(self, other: Self) -> Self {
        self.min(other)
    }
}

/// The leftmost glyph origin and rightmost advance edge of a source range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GlyphSpan {
    pub(super) start: f32,
    pub(super) end: f32,
}

impl Summary for GlyphSpan {
    const EMPTY: Self = Self {
        start: f32::INFINITY,
        end: f32::NEG_INFINITY,
    };

    fn combine(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

pub(super) struct GlyphRanges<T: Summary> {
    offsets: Vec<(u32, T)>,
    summaries: Vec<T>,
}

impl<T: Summary> GlyphRanges<T> {
    pub(super) fn new(glyphs: impl Iterator<Item = (u32, T)>) -> Self {
        let mut offsets: Vec<_> = glyphs.collect();
        let n = offsets.len();
        #[cfg(test)]
        let mut work = 2 * n;
        offsets.sort_unstable_by(|a, b| {
            #[cfg(test)]
            {
                work += 1;
            }
            a.0.cmp(&b.0)
        });
        let mut summaries = vec![T::EMPTY; 2 * n];
        for (i, &(_, position)) in offsets.iter().enumerate() {
            summaries[n + i] = position;
        }
        for i in (1..n).rev() {
            summaries[i] = summaries[i * 2].combine(summaries[i * 2 + 1]);
        }
        #[cfg(test)]
        {
            work += n.saturating_sub(1);
            super::tests::record_glyph_work(work);
        }
        Self { offsets, summaries }
    }

    pub(super) fn query(&self, range: Range<usize>) -> Option<T> {
        #[cfg(test)]
        let mut work = 0;
        let mut left = self.offsets.partition_point(|&(offset, _)| {
            #[cfg(test)]
            {
                work += 1;
            }
            (offset as usize) < range.start
        });
        let mut right = self.offsets.partition_point(|&(offset, _)| {
            #[cfg(test)]
            {
                work += 1;
            }
            (offset as usize) < range.end
        });
        if left >= right {
            #[cfg(test)]
            super::tests::record_glyph_work(work);
            return None;
        }
        left += self.offsets.len();
        right += self.offsets.len();
        let mut summary = T::EMPTY;
        while left < right {
            #[cfg(test)]
            {
                work += 1;
            }
            if left % 2 == 1 {
                summary = summary.combine(self.summaries[left]);
                left += 1;
            }
            if right % 2 == 1 {
                right -= 1;
                summary = summary.combine(self.summaries[right]);
            }
            left /= 2;
            right /= 2;
        }
        #[cfg(test)]
        super::tests::record_glyph_work(work);
        Some(summary)
    }
}

#[cfg(test)]
mod tests;
