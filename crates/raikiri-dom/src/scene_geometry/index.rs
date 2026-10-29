//! Conservative interval pruning with the same f32 addition order as extraction.
use super::SceneRect;
#[derive(Debug)]
struct Branch {
    piece: usize,
    start: f32,
    extent: f32,
    min_start: f32,
    max_start: f32,
    max_extent: f32,
    left: Option<usize>,
    right: Option<usize>,
}
#[derive(Debug)]
pub(super) struct RectIndex {
    branches: Vec<Branch>,
    root: Option<usize>,
    nonfinite: Vec<usize>,
    count: usize,
}
impl RectIndex {
    pub(super) fn new(pieces: &[(usize, SceneRect)], horizontal: bool) -> Self {
        let mut finite = Vec::new();
        let mut nonfinite = Vec::new();
        for (piece, &(_, (x, y, w, h))) in pieces.iter().enumerate() {
            let (start, extent) = if horizontal {
                (x, w.max(0.0))
            } else {
                (y, h.max(0.0))
            };
            if start.is_finite() {
                finite.push((piece, start, extent));
            } else {
                nonfinite.push(piece);
            }
        }
        finite.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
        let mut result = Self {
            branches: Vec::with_capacity(finite.len()),
            root: None,
            nonfinite,
            count: pieces.len(),
        };
        result.root = result.build(&finite);
        result
    }
    fn build(&mut self, entries: &[(usize, f32, f32)]) -> Option<usize> {
        if entries.is_empty() {
            return None;
        }
        let middle = entries.len() / 2;
        let (piece, start, extent) = entries[middle];
        let left = self.build(&entries[..middle]);
        let right = self.build(&entries[middle + 1..]);
        let index = self.branches.len();
        self.branches.push(Branch {
            piece,
            start,
            extent,
            min_start: entries[0].1,
            max_start: entries[entries.len() - 1].1,
            max_extent: entries.iter().map(|e| e.2).fold(0.0, f32::max),
            left,
            right,
        });
        Some(index)
    }
    pub(super) fn query(&self, translation: f32, end: f32) -> Vec<usize> {
        if !translation.is_finite() || !end.is_finite() {
            return (0..self.count).collect();
        }
        let mut output = self.nonfinite.clone();
        self.visit(self.root, translation, end, &mut output);
        output
    }
    fn visit(&self, index: Option<usize>, translation: f32, end: f32, output: &mut Vec<usize>) {
        let Some(index) = index else {
            return;
        };
        #[cfg(test)]
        tests::count_visit();
        let branch = &self.branches[index];
        // f32 addition is monotone. These bounds enclose the old arithmetic
        // even when addition overflows; strict pruning retains zero extents
        // at either inclusive page edge. No subtraction or regrouping occurs.
        if branch.min_start + translation > end
            || (branch.max_start + translation) + branch.max_extent < 0.0
        {
            return;
        }
        let start = branch.start + translation;
        if if branch.extent == 0.0 {
            start >= 0.0 && start <= end
        } else {
            start < end && start + branch.extent > 0.0
        } {
            output.push(branch.piece);
        }
        self.visit(branch.left, translation, end, output);
        self.visit(branch.right, translation, end, output);
    }
}
#[cfg(test)]
pub(super) fn take_query_visits() -> usize {
    tests::take_visits()
}
#[cfg(test)]
impl RectIndex {
    pub(super) fn retained_bytes(&self) -> usize {
        self.branches.capacity() * std::mem::size_of::<Branch>()
            + self.nonfinite.capacity() * std::mem::size_of::<usize>()
    }
}

#[cfg(test)]
mod tests;
