//! Batch insertion for independent ruby segments with stable base ancestry.
//!
//! Parents precede their children, except a target container's self-parent
//! wrapper. Repeated non-target nodes retain their parent. A target absorbs
//! ancestry at its own ID, so its wrappers may have different parents, but no
//! occurrence may inherit another target. This keeps their first contiguous
//! base groups disjoint without assuming visual order is a DOM preorder.
//!
//! Annotation nodes cannot shadow base nodes or their already known parents.
//! Earlier insertions therefore cannot change later base ancestry or enter
//! another target's first group. Rejected shapes use sequential insertion.

use super::InlineBoxPiece;
use std::collections::HashMap;

struct Node {
    parent: Option<usize>,
    owner: Option<usize>,
}

pub(super) struct Batch {
    containers: HashMap<usize, usize>,
    slots: Vec<Option<usize>>,
    extra: usize,
}

pub(super) fn plan(
    line: &[InlineBoxPiece],
    annotations: &[(usize, Vec<InlineBoxPiece>)],
) -> Option<Batch> {
    let mut containers = HashMap::with_capacity(annotations.len());
    for (container, _) in annotations {
        #[cfg(test)] // cov:ignore: Non-executable test-only attribute.
        super::work::record(|work| work.inspected += 1);
        let next = containers.len();
        containers.entry(*container).or_insert(next);
    } // cov:ignore: Compiler loop-closing region; both the counter and entry insertion are covered.
    let mut seen = vec![false; containers.len()];
    let mut ends = vec![None; containers.len()];
    let mut nodes: HashMap<usize, Node> = HashMap::with_capacity(line.len());
    let mut active = None;
    for (index, piece) in line.iter().enumerate() {
        #[cfg(test)]
        super::work::record(|work| work.inspected += 1);
        let target = containers.get(&piece.node).copied();
        let parent_owner = match piece.parent {
            Some(parent) if parent == piece.node && target.is_some() => target,
            Some(parent) => nodes.get(&parent)?.owner,
            None => None,
        };
        if let Some(target) = target
            && parent_owner.is_some_and(|owner| owner != target)
        {
            return None;
        }
        match nodes.get(&piece.node) {
            Some(previous) if target.is_none() && previous.parent != piece.parent => return None,
            Some(_) => {}
            None => {
                nodes.insert(
                    piece.node,
                    Node {
                        parent: piece.parent,
                        owner: target.or(parent_owner),
                    },
                );
            }
        }
        // A target is an absorbing ancestry owner, even when its final piece
        // is a self-parent wrapper. Non-target duplicate parents stay equal.
        if let Some(current) = active
            && parent_owner != Some(current)
        {
            ends[current] = Some(index);
            active = None;
        }
        if let Some(target) = target
            && !seen[target]
        {
            seen[target] = true;
            active = Some(target);
        }
    }
    if let Some(current) = active {
        ends[current] = Some(line.len());
    }
    let mut extra = 0_usize;
    for (_, own) in annotations {
        extra = extra.checked_add(own.len())?;
        for piece in own {
            #[cfg(test)]
            super::work::record(|work| work.inspected += 1);
            // Every base parent was already present during the scan. New
            // annotation metadata therefore cannot change any base ancestry.
            if nodes.contains_key(&piece.node) {
                return None;
            }
        }
    }
    line.len().checked_add(extra)?;
    let mut slots = vec![None; line.len() + 1];
    for (container, end) in ends.into_iter().enumerate() {
        #[cfg(test)]
        super::work::record(|work| work.inspected += 1);
        let slot = &mut slots[end?];
        if slot.replace(container).is_some() {
            return None; // cov:ignore: Defensive check; distinct first groups end at strictly increasing slots.
        }
    }
    Some(Batch {
        containers,
        slots,
        extra,
    })
}

impl Batch {
    pub(super) fn insert(
        self,
        pieces: &mut Vec<InlineBoxPiece>,
        line_start: usize,
        annotations: Vec<(usize, Vec<InlineBoxPiece>)>,
    ) {
        let mut buckets = vec![Vec::new(); self.containers.len()];
        for (container, mut own) in annotations {
            let bucket = &mut buckets[self.containers[&container]];
            if bucket.is_empty() {
                *bucket = own;
            } else {
                #[cfg(test)] // cov:ignore: Non-executable test-only attribute.
                super::work::record(|work| work.copied += own.len());
                bucket.append(&mut own);
            } // cov:ignore: Compiler closing-brace region; both the counter and bucket append are covered.
        }
        let line = pieces.split_off(line_start);
        pieces.reserve(line.len() + self.extra);
        #[cfg(test)]
        super::work::record(|work| {
            work.batch_lines += 1;
            work.copied += 2 * line.len() + self.extra;
        });
        // Only the first contiguous base group receives annotations. Later
        // descendants in visual order never reopen an already closed group.
        let last = line.len();
        for (index, piece) in line.into_iter().enumerate() {
            if let Some(bucket) = self.slots[index] {
                pieces.append(&mut buckets[bucket]);
            }
            pieces.push(piece);
        }
        if let Some(bucket) = self.slots[last] {
            pieces.append(&mut buckets[bucket]);
        }
    }
}
