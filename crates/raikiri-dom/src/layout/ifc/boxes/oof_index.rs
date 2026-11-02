//! Per-placement lookups for out-of-flow anchors and missing-anchor positions.

use super::super::inline_boxes::InlineBoxPiece;
use super::*;

/// Keep the first anchor in line and visual-fragment order for each node.
pub(crate) fn first_anchors(lines: &IfcLines) -> HashMap<usize, (usize, f32)> {
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

/// The padding boxes of the inline elements of a paragraph that contain
/// out-of-flow boxes, made from the `pieces` of the inline elements on its
/// lines (CSS 2.1 10.1).
///
/// The pieces are grouped by element once, and an element is reduced to its
/// padding box once however many boxes it contains, so that placing the boxes
/// does not read every piece again for each of them.
pub(super) struct InlinePaddingBoxes {
    pieces: Vec<InlineBoxPiece>,
    /// Indices in `pieces` by the element each piece belongs to.
    own: HashMap<usize, Vec<usize>>,
    /// Indices in `pieces` by each element above them that has no piece of
    /// its own, made when such an element contains a box.
    inside: Option<HashMap<usize, Vec<usize>>>,
    /// The padding box of each element asked for so far.
    known: HashMap<usize, Option<PaddingBox>>,
}

impl InlinePaddingBoxes {
    pub(super) fn new(pieces: Vec<InlineBoxPiece>) -> Self {
        let mut own: HashMap<usize, Vec<usize>> = HashMap::new();
        for (index, piece) in pieces.iter().enumerate() {
            #[cfg(test)]
            tests::OOF_PIECE_VISITS.with(|count| count.set(count.get() + 1));
            own.entry(piece.node).or_default().push(index);
        }
        Self {
            pieces,
            own,
            inside: None,
            known: HashMap::new(),
        }
    }

    /// The padding box of the inline element `inline` of the paragraph rooted
    /// at `idx` (or at `owner`, the source owner of an anonymous cell's
    /// content), in the coordinates of the paragraph's content box: the
    /// bounding box of the padding boxes of its pieces. Its borders are the
    /// authored ones, which the pieces carry, whatever its style view reads.
    /// `line_shift` gives the move pagination made to the lines. An element
    /// with no piece of its own, such as a ruby base container, which has no
    /// box and holds its bases, spans the pieces of the elements in it. `None`
    /// when there is none.
    pub(super) fn padding_box(
        &mut self,
        tree: &LayoutSession<'_>,
        (idx, owner): (usize, usize),
        inline: usize,
        line_shift: impl Fn(usize) -> (f32, f32),
    ) -> Option<PaddingBox> {
        if let Some(&known) = self.known.get(&inline) {
            return known;
        }
        let (border, spanned) = match self.own.get(&inline) {
            Some(own) => {
                let computed = &tree.style_views.cascade().computed[inline].border;
                let border = Edges {
                    top: computed.top.width().px(),
                    right: computed.right.width().px(),
                    bottom: computed.bottom.width().px(),
                    left: computed.left.width().px(),
                };
                (border, own.as_slice())
            }
            None => {
                let pieces = &self.pieces;
                let inside = self.inside.get_or_insert_with(|| {
                    let mut inside: HashMap<usize, Vec<usize>> = HashMap::new();
                    for (index, piece) in pieces.iter().enumerate() {
                        #[cfg(test)]
                        tests::OOF_PIECE_VISITS.with(|count| count.set(count.get() + 1));
                        let above = tree
                            .ancestors(piece.node)
                            .take_while(|&id| id != idx && id != owner);
                        above.for_each(|id| inside.entry(id).or_default().push(index));
                    }
                    inside
                });
                (
                    Edges::ZERO,
                    inside.get(&inline).map_or(&[][..], Vec::as_slice),
                )
            }
        };
        #[cfg(test)]
        tests::OOF_PADDING_BOX_BUILDS.with(|count| count.set(count.get() + 1));
        // The edges of a piece: its border box less its borders. A side that
        // is not an edge of the element has no border, and the content box
        // reaches the border box there.
        let padding_box = spanned
            .iter()
            .map(|&index| {
                #[cfg(test)]
                tests::OOF_PIECE_VISITS.with(|count| count.set(count.get() + 1));
                let piece = &self.pieces[index];
                let (outer, inner) = (piece.border_box, piece.content_box);
                let (dx, dy) = line_shift(piece.line);
                (
                    outer.x + dx + border.left.min((inner.x - outer.x).max(0.0)),
                    outer.y + dy + border.top.min((inner.y - outer.y).max(0.0)),
                    outer.x + dx + outer.width
                        - border
                            .right
                            .min((outer.x + outer.width - inner.x - inner.width).max(0.0)),
                    outer.y + dy + outer.height
                        - border
                            .bottom
                            .min((outer.y + outer.height - inner.y - inner.height).max(0.0)),
                )
            })
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
            .map(|(left, top, right, bottom)| PaddingBox {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            });
        self.known.insert(inline, padding_box);
        padding_box
    }
}
