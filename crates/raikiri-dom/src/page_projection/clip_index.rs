//! Sparse interval lookup from an event line to its paragraph placements.

use std::ops::Range;

#[derive(Debug, Clone)]
struct PlacementRange {
    lines: Range<usize>,
    placement: usize,
    subtree_end: usize,
}

/// A balanced interval tree stored in sorted-array order. A placement keeps
/// ranges rather than one record for every line it covers.
#[derive(Debug, Clone, Default)]
pub(super) struct PlacementRanges(Vec<PlacementRange>);

impl PlacementRanges {
    pub(super) fn push(&mut self, lines: Range<usize>, placement: usize) {
        if !lines.is_empty() {
            self.0.push(PlacementRange {
                subtree_end: lines.end,
                lines,
                placement,
            });
        }
    }

    pub(super) fn finish(&mut self) {
        self.0
            .sort_unstable_by_key(|range| (range.lines.start, range.lines.end, range.placement));
        fn set_ends(ranges: &mut [PlacementRange]) -> usize {
            if ranges.is_empty() {
                return 0;
            }
            let middle = ranges.len() / 2;
            let (left, tail) = ranges.split_at_mut(middle);
            let (node, right) = tail.split_first_mut().unwrap();
            node.subtree_end = node.lines.end.max(set_ends(left)).max(set_ends(right));
            node.subtree_end
        }
        set_ends(&mut self.0);
    }

    pub(super) fn at(&self, line: usize) -> Vec<usize> {
        let mut placements = Vec::new();
        let mut pending = vec![(0, self.0.len())];
        while let Some((start, end)) = pending.pop() {
            if start == end {
                continue;
            }
            let middle = start + (end - start) / 2;
            let node = &self.0[middle];
            #[cfg(test)]
            super::work_tests::clip_probe();
            if node.subtree_end <= line {
                continue;
            }
            if node.lines.start <= line {
                if line < node.lines.end {
                    placements.push(node.placement);
                }
                pending.push((middle + 1, end));
            }
            pending.push((start, middle));
        }
        placements
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}

#[cfg(test)]
mod tests;
