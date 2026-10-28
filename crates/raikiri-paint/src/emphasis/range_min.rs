//! A per-run range minimum over glyph source offsets.
//!
//! Source ranges may overlap or arrive out of order. Sorting offsets once
//! and querying an iterative min tree costs O(G log G + C log G), with O(G)
//! storage. Only offset and inline position are retained. Shodo's non-Combined
//! positions come from LayoutUnit and are finite; this is not a general
//! floating-point reduction API.

use std::ops::Range;

pub(super) struct GlyphPositions {
    offsets: Vec<(u32, f32)>,
    minima: Vec<f32>,
}

impl GlyphPositions {
    pub(super) fn new(glyphs: impl Iterator<Item = (u32, f32)>) -> Self {
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
        let mut minima = vec![f32::INFINITY; 2 * n];
        for (i, &(_, position)) in offsets.iter().enumerate() {
            minima[n + i] = position;
        }
        for i in (1..n).rev() {
            minima[i] = minima[i * 2].min(minima[i * 2 + 1]);
            #[cfg(test)]
            {
                work += 1;
            }
        }
        #[cfg(test)]
        super::tests::record_glyph_work(work);
        Self { offsets, minima }
    }

    pub(super) fn minimum(&self, range: Range<usize>) -> Option<f32> {
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
        let mut minimum = f32::INFINITY;
        while left < right {
            #[cfg(test)]
            {
                work += 1;
            }
            if left % 2 == 1 {
                minimum = minimum.min(self.minima[left]);
                left += 1;
            }
            if right % 2 == 1 {
                right -= 1;
                minimum = minimum.min(self.minima[right]);
            }
            left /= 2;
            right /= 2;
        }
        #[cfg(test)]
        super::tests::record_glyph_work(work);
        Some(minimum)
    }
}

#[cfg(test)]
mod tests;
