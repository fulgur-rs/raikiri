//! Remap metadata with source work bounded by retained nodes.

use std::collections::HashMap;
use std::hash::Hash;

use crate::StyleNodeId;

pub(super) fn node_map<V: Clone>(
    source: &HashMap<StyleNodeId, V>,
    new_of_old: &HashMap<u64, u64>,
) -> HashMap<StyleNodeId, V> {
    let mut result = HashMap::new();
    if source.is_empty() {
        return result;
    }
    // HashMap iteration visits empty buckets too, so bound capacity, not len.
    if source.capacity() <= new_of_old.len() {
        for (&old, value) in source {
            #[cfg(test)]
            super::tests::record_extract_work(0);
            if let Some(&new) = new_of_old.get(&old.0) {
                result.insert(StyleNodeId(new), value.clone());
            }
        }
    } else {
        for (&old, &new) in new_of_old {
            #[cfg(test)]
            super::tests::record_extract_work(1);
            if let Some(value) = source.get(&StyleNodeId(old)) {
                result.insert(StyleNodeId(new), value.clone());
            }
        }
    }
    result
}

pub(super) fn pseudo_map<K: Copy + Eq + Hash, V: Clone>(
    source: &HashMap<(StyleNodeId, K), V>,
    new_of_old: &HashMap<u64, u64>,
    kinds: impl Iterator<Item = K> + Clone,
) -> HashMap<(StyleNodeId, K), V> {
    let mut result = HashMap::new();
    if source.is_empty() {
        return result;
    }
    let lookup_budget = new_of_old.len().saturating_mul(kinds.clone().count());
    if source.capacity() <= lookup_budget {
        for (&(old, kind), value) in source {
            #[cfg(test)]
            super::tests::record_extract_work(0);
            if let Some(&new) = new_of_old.get(&old.0)
                && kinds.clone().any(|candidate| candidate == kind)
            {
                result.insert((StyleNodeId(new), kind), value.clone());
            }
        }
    } else {
        for (&old, &new) in new_of_old {
            for kind in kinds.clone() {
                #[cfg(test)]
                super::tests::record_extract_work(1);
                if let Some(value) = source.get(&(StyleNodeId(old), kind)) {
                    result.insert((StyleNodeId(new), kind), value.clone());
                }
            }
        }
    }
    result
}
