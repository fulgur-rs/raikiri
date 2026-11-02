//! Compact counter snapshots with immutable, non-recursive arena ownership.

use super::*;
use crate::generated_content::{CounterSnapshotView, CounterStack};

const EMPTY: usize = usize::MAX;

#[derive(Clone, Copy, Debug)]
struct StackEntry {
    parent: usize,
    value: i32,
    depth: u32,
}

#[derive(Clone, Copy, Debug)]
enum MapEntry {
    Leaf { name: u32, head: usize },
    Branch { bit: u32, children: [usize; 2] },
}

/// Named counters shared by immutable prefix and name-map arenas.
///
/// Unlike [`CounterSnapshots`], a node stores only a root handle. The compact
/// and legacy owned factories each enforce their own operation budget; calling
/// both can retain both results. The estimate covers retained capacities,
/// including obsolete arena entries, rather than allocator overhead or peak RSS.
#[derive(Debug, Default)]
pub struct CompactCounterSnapshots {
    nodes: Vec<usize>,
    pseudos: HashMap<(usize, PseudoElem), usize>,
    quote_starts: HashMap<(usize, PseudoElem), usize>,
    names: HashMap<Symbol, u32>,
    stacks: Vec<StackEntry>,
    maps: Vec<MapEntry>,
    estimated_bytes: u64,
}

impl CompactCounterSnapshots {
    /// Borrow the counters at an arena node without expanding its name map.
    pub fn get(&self, node: usize) -> Option<CounterSnapshotView<'_>> {
        self.nodes
            .get(node)
            .map(|&root| CounterSnapshotView::compact(self, root))
    }

    /// Borrow a counter-reading pseudo's snapshot after its own directives.
    pub fn pseudo(&self, node: usize, pseudo: PseudoElem) -> Option<CounterSnapshotView<'_>> {
        self.pseudos
            .get(&(node, pseudo))
            .map(|&root| CounterSnapshotView::compact(self, root))
    }

    /// Estimated retained bytes charged by this construction, including spare capacity.
    pub fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub(crate) fn quote_depth(&self, node: usize, pseudo: PseudoElem) -> usize {
        self.quote_starts
            .get(&(node, pseudo))
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn stack(&self, root: usize, name: &str) -> CounterStack<'_> {
        let head = self
            .names
            .get(&Symbol::new(name))
            .map_or(EMPTY, |&id| self.head(root, id));
        CounterStack::Prefix(self, head)
    }

    pub(crate) fn last(&self, head: usize) -> Option<i32> {
        self.stacks.get(head).map(|entry| entry.value)
    }

    pub(crate) fn values(&self, mut head: usize) -> Vec<i32> {
        let depth = self
            .stacks
            .get(head)
            .map_or(0, |entry| entry.depth as usize);
        let mut values = Vec::with_capacity(depth);
        while let Some(entry) = self.stacks.get(head) {
            values.push(entry.value);
            head = entry.parent;
        }
        values.reverse();
        values
    }

    fn head(&self, mut root: usize, name: u32) -> usize {
        while let Some(entry) = self.maps.get(root) {
            match *entry {
                MapEntry::Leaf { name: found, head } => {
                    return if found == name { head } else { EMPTY };
                }
                MapEntry::Branch { bit, children } => root = children[((name >> bit) & 1) as usize],
            }
        }
        EMPTY
    }
}

fn reserve<T>(
    values: &mut Vec<T>,
    budget: &mut CounterSnapshotBudget,
) -> Result<(), CounterSnapshotLimitExceeded> {
    if values.len() == values.capacity() {
        let capacity = values.capacity().saturating_mul(2).max(4);
        let additional = capacity.checked_sub(values.capacity());
        budget.charge(
            additional
                .and_then(|count| count.checked_mul(size_of::<T>()))
                .and_then(|bytes| u64::try_from(bytes).ok()),
        )?;
        values.reserve_exact(capacity - values.len());
    }
    Ok(())
}

fn reserve_table<K, V>(
    table: &mut HashMap<K, V>,
    budget: &mut CounterSnapshotBudget,
) -> Result<(), CounterSnapshotLimitExceeded>
where
    K: Eq + std::hash::Hash,
{
    if table.len() == table.capacity() {
        // HashMap's load factor and bucket rounding fit this conservative bound.
        // Reserve before insertion; charge the bound rather than logical entries.
        let requested = table.len().checked_add(1);
        let bound = requested.and_then(|n| n.checked_mul(4));
        let old = table.capacity();
        let bytes = bound
            .and_then(|n| n.checked_sub(old))
            .and_then(|n| n.checked_mul(size_of::<(K, V)>() + 1))
            .and_then(|n| n.checked_add(16));
        budget.charge(bytes.and_then(|n| u64::try_from(n).ok()))?;
        table.reserve(1);
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct CompactBuilder {
    snapshots: CompactCounterSnapshots,
    root: usize,
    budget: CounterSnapshotBudget,
    starting_bytes: u64,
    error: Option<CounterSnapshotLimitExceeded>,
}

impl CompactBuilder {
    fn new(
        node_count: usize,
        budget: &CounterSnapshotBudget,
    ) -> Result<Self, CounterSnapshotLimitExceeded> {
        let mut budget = CounterSnapshotBudget {
            estimated_bytes: budget.estimated_bytes,
            limit: budget.limit,
        };
        let starting_bytes = budget.estimated_bytes;
        budget.charge(Some(size_of::<CompactCounterSnapshots>() as u64 + 16))?;
        budget.charge((node_count as u64).checked_mul(size_of::<usize>() as u64))?;
        Ok(Self {
            snapshots: CompactCounterSnapshots {
                nodes: vec![EMPTY; node_count],
                ..Default::default()
            },
            root: EMPTY,
            budget,
            starting_bytes,
            error: None,
        })
    }

    pub(super) fn change(&mut self, name: &Symbol, value: Option<i32>, push: bool) {
        if self.error.is_none() {
            self.error = self.try_change(name, value, push).err();
        }
    }

    fn try_change(
        &mut self,
        name: &Symbol,
        value: Option<i32>,
        push: bool,
    ) -> Result<(), CounterSnapshotLimitExceeded> {
        let id = if let Some(&id) = self.snapshots.names.get(name) {
            id
        } else {
            // cov:ignore: public rendering's 128 MiB budget rejects growth before u32 name IDs overflow; larger private test budgets need impractical billions of names
            let id = u32::try_from(self.snapshots.names.len()).map_err(|_| {
                CounterSnapshotLimitExceeded {
                    limit: self.budget.limit,
                    actual: u64::MAX,
                }
            })?;
            reserve_table(&mut self.snapshots.names, &mut self.budget)?;
            // Inline names need no allocation; heap names can share SmolStr storage,
            // but charging their complete length is conservative for retained ownership.
            self.budget
                .charge((name.as_str().len() as u64).checked_add(16))?;
            self.snapshots.names.insert(name.clone(), id);
            id
        };
        let old = self.snapshots.head(self.root, id);
        let head = if let Some(value) = value {
            let parent = if push {
                old
            } else {
                self.snapshots
                    .stacks
                    .get(old)
                    .map_or(EMPTY, |entry| entry.parent)
            };
            let depth = self
                .snapshots
                .stacks
                .get(parent)
                .map_or(Some(1), |entry| entry.depth.checked_add(1));
            let depth = depth.ok_or(CounterSnapshotLimitExceeded {
                limit: self.budget.limit,
                actual: u64::MAX,
            })?;
            reserve(&mut self.snapshots.stacks, &mut self.budget)?;
            let index = self.snapshots.stacks.len();
            self.snapshots.stacks.push(StackEntry {
                parent,
                value,
                depth,
            });
            index
        } else {
            self.snapshots
                .stacks
                .get(old)
                .map_or(EMPTY, |entry| entry.parent)
        };
        self.root = self.binding(id, head)?;
        Ok(())
    }

    fn add_map(&mut self, entry: MapEntry) -> Result<usize, CounterSnapshotLimitExceeded> {
        reserve(&mut self.snapshots.maps, &mut self.budget)?;
        let index = self.snapshots.maps.len();
        self.snapshots.maps.push(entry);
        Ok(index)
    }

    fn binding(&mut self, id: u32, head: usize) -> Result<usize, CounterSnapshotLimitExceeded> {
        let leaf = self.add_map(MapEntry::Leaf { name: id, head })?;
        if self.root == EMPTY {
            return Ok(leaf);
        }
        let mut cursor = self.root;
        let existing = loop {
            match self.snapshots.maps[cursor] {
                MapEntry::Leaf { name, .. } => break name,
                MapEntry::Branch { bit, children } => cursor = children[((id >> bit) & 1) as usize],
            }
        };
        let differing = if existing == id {
            None
        } else {
            Some(31 - (existing ^ id).leading_zeros())
        };
        let mut path = [(0_usize, 0_usize); 32];
        let mut count = 0;
        cursor = self.root;
        while let MapEntry::Branch { bit, children } = self.snapshots.maps[cursor] {
            if differing.is_some_and(|new_bit| bit <= new_bit) {
                break;
            }
            let direction = ((id >> bit) & 1) as usize;
            path[count] = (cursor, direction);
            count += 1;
            cursor = children[direction];
        }
        let mut root = if let Some(bit) = differing {
            let direction = ((id >> bit) & 1) as usize;
            let mut children = [cursor; 2];
            children[direction] = leaf;
            self.add_map(MapEntry::Branch { bit, children })?
        } else {
            leaf
        };
        for &(ancestor, direction) in path[..count].iter().rev() {
            if let MapEntry::Branch { bit, mut children } = self.snapshots.maps[ancestor] {
                children[direction] = root;
                root = self.add_map(MapEntry::Branch { bit, children })?;
            } // cov:ignore: path entries are recorded only from Branch nodes, and arena entries are immutable
        }
        Ok(root)
    }

    fn snapshot(&mut self, target: CounterTarget) -> Result<(), CounterSnapshotLimitExceeded> {
        if let Some(error) = self.error {
            return Err(error);
        }
        match target {
            CounterTarget::Element(node) => self.snapshots.nodes[node] = self.root,
            CounterTarget::Pseudo(node, pseudo) => {
                reserve_table(&mut self.snapshots.pseudos, &mut self.budget)?;
                self.snapshots.pseudos.insert((node, pseudo), self.root);
            }
            CounterTarget::QuoteStart(node, pseudo, depth) => {
                reserve_table(&mut self.snapshots.quote_starts, &mut self.budget)?;
                self.snapshots.quote_starts.insert((node, pseudo), depth);
            }
        }
        Ok(())
    }
}

/// Build compact named-counter snapshots without expanding any owned maps.
#[doc(hidden)]
pub fn compact_counter_snapshots(
    doc: &Document,
    cascade: &CascadeResult,
) -> Result<CompactCounterSnapshots, CounterSnapshotLimitExceeded> {
    compact_counter_snapshots_with_budget(doc, cascade, &mut CounterSnapshotBudget::default())
}

/// Build compact snapshots against a caller-owned cumulative retained estimate.
/// Arena spare capacity, obsolete entries, roots, tables and names count once.
/// Temporary walk state and output strings are not retained snapshot storage.
#[doc(hidden)]
pub fn compact_counter_snapshots_with_budget(
    doc: &Document,
    cascade: &CascadeResult,
    budget: &mut CounterSnapshotBudget,
) -> Result<CompactCounterSnapshots, CounterSnapshotLimitExceeded> {
    let mut scopes = resolved_scopes(doc, cascade)?;
    scopes.compact = Some(std::cell::RefCell::new(CompactBuilder::new(
        doc.node_count(),
        budget,
    )?));
    let result = walk_counter_scopes(doc, cascade, &mut scopes, |target, scopes| {
        scopes
            .compact
            .as_ref()
            .expect("compact builder installed")
            .borrow_mut()
            .snapshot(target)
    });
    let mut builder = scopes
        .compact
        .take()
        .expect("compact builder installed")
        .into_inner();
    budget.estimated_bytes = builder.budget.estimated_bytes;
    result?;
    if let Some(error) = builder.error {
        return Err(error);
    }
    builder.snapshots.estimated_bytes = builder.budget.estimated_bytes - builder.starting_bytes;
    Ok(builder.snapshots)
}

pub(crate) type SharedCompactCounterSnapshots =
    Result<std::sync::Arc<CompactCounterSnapshots>, CounterSnapshotLimitExceeded>;

impl Document {
    /// Share compact snapshots for this element cascade across built-in rendering.
    /// Tree mutations and layout generation transitions invalidate this cache.
    /// The legacy owned snapshot cache remains a separately budgeted operation.
    pub fn shared_compact_counter_snapshots(
        &self,
        cascade: &CascadeResult,
    ) -> SharedCompactCounterSnapshots {
        let generation = cascade.element_generation();
        let build = || compact_counter_snapshots(self, cascade).map(std::sync::Arc::new);
        let (cached_generation, cached) = self
            .compact_counter_snapshot_cache
            .get_or_init(|| (generation, build()));
        if *cached_generation == generation {
            cached.clone()
        } else {
            build()
        }
    }
}

#[cfg(test)]
mod tests;
