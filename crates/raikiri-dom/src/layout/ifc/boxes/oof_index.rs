//! Per-placement lookups for out-of-flow anchors and missing-anchor positions.

use super::*;

/// Keep the first anchor in line and visual-fragment order for each node.
pub(super) fn first_anchors(lines: &IfcLines) -> HashMap<usize, (usize, f32)> {
    #[cfg(test)]
    tests::OOF_ANCHOR_INDEX_BUILDS.with(|count| count.set(count.get() + 1));
    let mut anchors = HashMap::new();
    for (index, line) in lines.lines.iter().enumerate() {
        for fragment in line.fragments() {
            #[cfg(test)]
            tests::OOF_ANCHOR_VISITS.with(|count| count.set(count.get() + 1));
            if let shodo::Fragment::OutOfFlowAnchor(anchor) = fragment {
                anchors
                    .entry(anchor.node.0 as usize)
                    .or_insert((index, anchor.inline_position));
            }
        }
    }
    anchors
}

struct Neighbours {
    first: usize,
    before: Option<usize>,
    after: Option<usize>,
}

/// The original missing-anchor rule evaluated once per distinct box node.
/// Geometry is the same as [`super::static_block_offset`], including duplicate
/// IDs, first metadata matches, and original-order generated block thresholds.
pub(super) struct StaticFallbackIndex {
    offsets: HashMap<usize, f32>,
    absent: f32,
}

impl StaticFallbackIndex {
    pub(super) fn new(lines: &IfcLines, boxes: &[IfcBox]) -> Self {
        #[cfg(test)]
        tests::OOF_FALLBACK_INDEX_BUILDS.with(|count| count.set(count.get() + 1));
        let mut neighbours = HashMap::with_capacity(boxes.len());
        let mut nearest = None;
        let mut different = None;
        for (index, box_) in boxes.iter().enumerate().rev() {
            #[cfg(test)]
            tests::OOF_BOX_VISITS.with(|count| count.set(count.get() + 1));
            let after = if nearest == Some(box_.node) {
                different
            } else {
                nearest
            };
            // Reverse overwrite preserves the first occurrence of any node ID.
            neighbours.insert(
                box_.node,
                Neighbours {
                    first: index,
                    before: None,
                    after,
                },
            );
            if box_.kind == IfcBoxKind::Block && nearest != Some(box_.node) {
                different = nearest;
                nearest = Some(box_.node);
            }
        }
        let mut before = None;
        for (index, box_) in boxes.iter().enumerate() {
            #[cfg(test)]
            tests::OOF_BOX_VISITS.with(|count| count.set(count.get() + 1));
            let entry = neighbours
                .get_mut(&box_.node)
                .expect("reverse pass indexed every box");
            if entry.first == index {
                entry.before = before;
            }
            // Record the prefix before the current Block changes it.
            if box_.kind == IfcBoxKind::Block {
                before = Some(box_.node);
            }
        }
        let mut ends = HashMap::new();
        for &(node, next_line, end) in &lines.block_ends {
            #[cfg(test)]
            tests::OOF_METADATA_VISITS.with(|count| count.set(count.get() + 1));
            ends.entry(node).or_insert((next_line, end));
        }
        let mut starts = HashMap::new();
        for (index, &(node, first)) in lines.block_line_starts.iter().enumerate() {
            #[cfg(test)]
            tests::OOF_METADATA_VISITS.with(|count| count.set(count.get() + 1));
            starts.entry(node).or_insert((first, index + 1));
        }
        // Each threshold is filled by its first matching source entry, even
        // when generated counts or line indices are not monotone.
        let mut generated = vec![None; lines.block_line_starts.len() + 1];
        let mut threshold = 0;
        for &(count, first) in &lines.generated_block_starts {
            #[cfg(test)]
            tests::OOF_METADATA_VISITS.with(|visits| visits.set(visits.get() + 1));
            let through = count.min(lines.block_line_starts.len());
            while threshold <= through {
                #[cfg(test)] // cov:ignore: non-executable test attribute.
                tests::OOF_METADATA_VISITS.with(|visits| visits.set(visits.get() + 1));
                generated[threshold] = Some(first);
                threshold += 1;
            } // cov:ignore: synthetic loop edge; its body has positive coverage.
        }
        let offset = |before: Option<usize>, after: Option<usize>| {
            let Some(block) = before else {
                return if lines.lines.is_empty() && lines.placed_blocks.is_empty() {
                    0.0
                } else {
                    lines.height
                };
            };
            let Some(&(next_line, end)) = ends.get(&block) else {
                return lines.height;
            };
            let next_block = after
                .and_then(|id| starts.get(&id))
                .map(|&(first, _)| first);
            let blocks_before = starts.get(&block).map_or(0, |&(_, ordinal)| ordinal);
            let generated_block = generated[blocks_before];
            let lines_end = next_block
                .into_iter()
                .chain(generated_block)
                .min()
                .unwrap_or(lines.lines.len());
            match lines_end.checked_sub(1).filter(|&last| last >= next_line) {
                Some(last) => lines.line_top(last) + lines.lines[last].block_size(),
                None => end + lines.shift_of(next_line),
            }
        };
        let absent = offset(before, None);
        let offsets = neighbours
            .into_iter()
            .map(|(node, neighbours)| {
                #[cfg(test)]
                tests::OOF_BOX_VISITS.with(|count| count.set(count.get() + 1));
                (node, offset(neighbours.before, neighbours.after))
            })
            .collect();
        Self { offsets, absent }
    }

    pub(super) fn offset(&self, node: usize) -> f32 {
        self.offsets.get(&node).copied().unwrap_or(self.absent)
    }
}
