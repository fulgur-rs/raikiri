//! Ownership of retained annotation content, independent of segment IDs.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Unknown,
    Anonymous,
    Container(usize),
    Ambiguous,
}

impl Owner {
    fn of(owner: Option<usize>) -> Self {
        owner.map_or(Self::Anonymous, Self::Container)
    }
}

#[derive(Debug, Default)]
pub(crate) struct RubyContainers {
    sources: HashMap<usize, Owner>,
    roots: HashMap<usize, Owner>,
    explicit_roots: HashSet<usize>,
    parents: HashMap<usize, Option<usize>>,
    wrappers: HashSet<usize>,
}

fn record(map: &mut HashMap<usize, Owner>, node: usize, owner: Option<usize>) {
    let owner = Owner::of(owner);
    map.entry(node)
        .and_modify(|old| {
            if *old != owner {
                *old = Owner::Ambiguous;
            }
        })
        .or_insert(owner);
}

impl RubyContainers {
    pub(crate) fn is_empty(&self) -> bool {
        self.parents.is_empty()
    }
    pub(super) fn source(&mut self, node: usize, owner: Option<usize>) {
        record(&mut self.sources, node, owner);
    }

    pub(super) fn root(&mut self, node: usize, owner: Option<usize>, explicit: bool) {
        record(&mut self.roots, node, owner);
        if explicit {
            self.explicit_roots.insert(node);
        }
    }

    pub(super) fn container(&mut self, node: usize, parent: Option<usize>) {
        self.parents.insert(node, parent);
    }

    pub(super) fn wrapper(&mut self, node: usize) {
        self.wrappers.insert(node);
    }

    pub(crate) fn is_ruby_wrapper(&self, node: usize) -> bool {
        self.wrappers.contains(&node)
    }

    pub(crate) fn is_container(&self, node: usize) -> bool {
        self.parents.contains_key(&node)
    }

    pub(crate) fn parent(&self, node: usize) -> Option<usize> {
        self.parents.get(&node).copied().flatten()
    }

    pub(crate) fn resolve(
        &self,
        sources: impl IntoIterator<Item = usize>,
        root: Option<usize>,
    ) -> Owner {
        let mut owner = Owner::Unknown;
        for source in sources {
            let Some(&candidate) = self.sources.get(&source) else {
                continue;
            };
            if candidate == Owner::Ambiguous || (owner != Owner::Unknown && owner != candidate) {
                return Owner::Ambiguous;
            }
            owner = candidate;
        }
        if owner != Owner::Unknown {
            return owner;
        }
        root.filter(|root| self.explicit_roots.contains(root))
            .and_then(|root| self.roots.get(&root).copied())
            .unwrap_or(Owner::Unknown)
    }
}

#[cfg(test)]
mod tests;
