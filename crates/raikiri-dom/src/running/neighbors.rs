//! Rendered neighbors, with one sparse scan before building linear indices.

use crate::running_index_diagnostics::RunningIndexWork;
use std::collections::HashMap;

type Candidate = Option<(usize, u32)>;

pub(super) struct NeighborPages {
    pub next: Option<u32>,
    pub previous: Option<u32>,
    pub own: Option<u32>,
}

pub(super) struct RenderedNeighbors<'a> {
    order: &'a [(usize, usize)],
    rendered: &'a HashMap<usize, (u32, u32)>,
    scanned: bool,
    indexed: Option<IndexedNeighbors>,
}

impl<'a> RenderedNeighbors<'a> {
    pub(super) fn new(
        order: &'a [(usize, usize)],
        rendered: &'a HashMap<usize, (u32, u32)>,
    ) -> Self {
        Self {
            order,
            rendered,
            scanned: false,
            indexed: None,
        }
    }

    pub(super) fn query<const COUNT: bool>(
        &mut self,
        position: usize,
        end: usize,
        needs_own: bool,
        work: &mut RunningIndexWork,
    ) -> NeighborPages {
        if !self.scanned {
            self.scanned = true;
            let next = self.order[end..]
                .iter()
                .inspect(|_| {
                    if COUNT {
                        work.next_candidate_visits = work.next_candidate_visits.saturating_add(1);
                    }
                })
                .find_map(|(id, _)| self.rendered.get(id).map(|(first, _)| *first));
            let previous = self.order[..position]
                .iter()
                .rev()
                .inspect(|_| {
                    if COUNT {
                        work.previous_candidate_visits =
                            work.previous_candidate_visits.saturating_add(1);
                    }
                })
                .filter(|(_, end)| *end <= position)
                .find_map(|(id, _)| self.rendered.get(id).map(|(_, last)| *last));
            let own = if needs_own {
                self.order[position..end]
                    .iter()
                    .inspect(|_| {
                        if COUNT {
                            work.own_candidate_visits = work.own_candidate_visits.saturating_add(1);
                        }
                    })
                    .find_map(|(id, _)| self.rendered.get(id).map(|(first, _)| *first))
            } else {
                None
            };
            return NeighborPages {
                next,
                previous,
                own,
            };
        }
        let indexed = self.indexed.get_or_insert_with(|| {
            IndexedNeighbors::build::<COUNT>(self.order, self.rendered, work)
        });
        if COUNT {
            work.indexed_probes = work.indexed_probes.saturating_add(2 + u64::from(needs_own));
        }
        NeighborPages {
            next: indexed.next[end].map(|(_, page)| page),
            previous: indexed.previous[position].map(|(_, page)| page),
            own: if needs_own {
                indexed.next[position]
                    .filter(|(at, _)| *at < end)
                    .map(|(_, page)| page)
            } else {
                None
            },
        }
    }
}

struct IndexedNeighbors {
    next: Vec<Candidate>,
    previous: Vec<Candidate>,
}

// Pages can be nonmonotonic. Only the greater preorder position wins.
fn later(a: Candidate, b: Candidate) -> Candidate {
    if b.is_some_and(|(position, _)| a.is_none_or(|(old, _)| position > old)) {
        b
    } else {
        a
    }
}

impl IndexedNeighbors {
    fn build<const COUNT: bool>(
        order: &[(usize, usize)],
        rendered: &HashMap<usize, (u32, u32)>,
        work: &mut RunningIndexWork,
    ) -> Self {
        let mut next = vec![None; order.len() + 1];
        let mut previous = vec![None; order.len() + 1];
        if COUNT {
            work.index_builds = work.index_builds.saturating_add(1);
            // Both initializations write one slot per element in each array.
            work.prepared_slots = work
                .prepared_slots
                .saturating_add((next.len() as u64).saturating_add(previous.len() as u64));
            work.scratch_bytes = work
                .scratch_bytes
                .max((next.capacity() + previous.capacity()) * std::mem::size_of::<Candidate>());
        }
        for position in (0..order.len()).rev() {
            if COUNT {
                work.prepared_nodes = work.prepared_nodes.saturating_add(1);
            }
            next[position] = next[position + 1];
            let (id, end) = order[position];
            if let Some(&(first, last)) = rendered.get(&id) {
                next[position] = Some((position, first));
                // Multiple ancestors and children may close at the same end.
                previous[end] = later(previous[end], Some((position, last)));
            }
        }
        let mut closed = None;
        for slot in &mut previous {
            if COUNT {
                work.prepared_slots = work.prepared_slots.saturating_add(1);
            }
            // A node qualifies at end == query position. Closing an ancestor
            // must not replace its child, whose preorder position is later.
            closed = later(closed, *slot);
            *slot = closed;
        }
        Self { next, previous }
    }
}
