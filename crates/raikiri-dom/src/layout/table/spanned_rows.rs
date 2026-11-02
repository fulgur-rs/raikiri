//! Lookups over the rows a cell spans, for the final placement of cells.
//!
//! A cell that spans N rows asks which of them are collapsed, how tall they
//! are together and how many gaps lie between them. Answering each question
//! by visiting the N rows makes N cells that span N rows cost N² visits, so
//! the counts are taken from prefix tables and the sum of the heights is kept
//! per range of rows. The sum is still added row by row, from the first row of
//! the range, so that it is the same number a sequential sum gives: the cell
//! heights that follow from it are compared with the heights the cells were
//! measured at.

use std::collections::HashMap;
use std::ops::Range;

/// Ranges of fewer rows than this are summed each time they are asked for.
const SUMMED_DIRECTLY: usize = 8;

pub(super) struct SpannedRows<'a> {
    heights: &'a [f32],
    /// How many rows before row `i` are collapsed, at index `i`.
    collapsed_before: Vec<usize>,
    /// The last row at or before row `i` that is not collapsed, at index `i`.
    last_kept_at: Vec<Option<usize>>,
    /// How many row gaps before gap `i` are not zero, at index `i`.
    gaps_before: Vec<usize>,
    height_sums: HashMap<(usize, usize), f32>,
}

impl<'a> SpannedRows<'a> {
    pub(super) fn new(collapsed: &[bool], heights: &'a [f32], gaps: &[f32]) -> Self {
        let mut collapsed_before = Vec::with_capacity(collapsed.len() + 1);
        let mut last_kept_at = Vec::with_capacity(collapsed.len());
        collapsed_before.push(0);
        let mut last_kept = None;
        for (row, &is_collapsed) in collapsed.iter().enumerate() {
            collapsed_before.push(collapsed_before[row] + usize::from(is_collapsed));
            if !is_collapsed {
                last_kept = Some(row);
            }
            last_kept_at.push(last_kept);
        }
        let mut gaps_before = Vec::with_capacity(gaps.len() + 1);
        gaps_before.push(0);
        for (gap, &size) in gaps.iter().enumerate() {
            gaps_before.push(gaps_before[gap] + usize::from(size != 0.0));
        }
        Self {
            heights,
            collapsed_before,
            last_kept_at,
            gaps_before,
            height_sums: HashMap::new(),
        }
    }

    /// Whether any of `rows` is collapsed.
    pub(super) fn any_collapsed(&self, rows: Range<usize>) -> bool {
        self.collapsed_before[rows.end] > self.collapsed_before[rows.start]
    }

    /// The last of `rows` that is not collapsed.
    pub(super) fn last_kept(&self, rows: Range<usize>) -> Option<usize> {
        let last = rows.end.checked_sub(1)?;
        self.last_kept_at[last].filter(|&row| row >= rows.start)
    }

    /// How many of the gaps in `gaps` are not zero.
    pub(super) fn nonzero_gaps(&self, gaps: Range<usize>) -> usize {
        self.gaps_before[gaps.end] - self.gaps_before[gaps.start]
    }

    /// The heights of `rows` added from the first, as a sequential sum does.
    pub(super) fn height(&mut self, rows: Range<usize>) -> f32 {
        let heights = self.heights;
        // A few rows are cheaper to add than to look up.
        if rows.len() < SUMMED_DIRECTLY {
            return heights[rows].iter().sum::<f32>();
        }
        *self
            .height_sums
            .entry((rows.start, rows.end))
            .or_insert_with(|| heights[rows].iter().sum::<f32>())
    }
}

#[cfg(test)]
mod tests;
