//! An immutable effective-binding index shared by custom-property environments.

use smol_str::SmolStr;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use super::custom_property_diagnostics;

#[cfg(test)]
mod tests;

type Link = Option<Arc<Node>>;

#[derive(Default, Clone)]
pub(super) struct CustomPropertyIndex {
    root: Link,
}

struct Node {
    name: SmolStr,
    value: Option<SmolStr>,
    left: Link,
    right: Link,
    height: u8,
    generation: u64,
}

// The index shares text buffers with local/ancestor maps; those maps already
// account for the text. Each fresh node pays its payload and two Arc counters.
const NODE_BYTES: u64 = (std::mem::size_of::<Node>() + 2 * std::mem::size_of::<usize>()) as u64;

fn height(node: &Link) -> u8 {
    node.as_ref().map_or(0, |node| node.height)
}

fn node<const COUNT: bool>(
    name: SmolStr,
    value: Option<SmolStr>,
    left: Link,
    right: Link,
    generation: u64,
) -> Arc<Node> {
    if COUNT {
        custom_property_diagnostics::created_node();
    }
    // An AVL tree addressable on a <=64-bit target cannot reach height255.
    let height = 1 + height(&left).max(height(&right));
    Arc::new(Node {
        name,
        value,
        left,
        right,
        height,
        generation,
    })
}

fn balanced<const COUNT: bool>(
    name: SmolStr,
    value: Option<SmolStr>,
    left: Link,
    right: Link,
    generation: u64,
) -> Arc<Node> {
    let balance = i16::from(height(&left)) - i16::from(height(&right));
    if balance > 1 {
        let left = left.as_ref().expect("left-heavy node has a left child");
        if height(&left.left) >= height(&left.right) {
            let right = node::<COUNT>(name, value, left.right.clone(), right, generation);
            return node::<COUNT>(
                left.name.clone(),
                left.value.clone(),
                left.left.clone(),
                Some(right),
                generation,
            );
        }
        let middle = left
            .right
            .as_ref()
            .expect("inner-heavy left child has a right child");
        let new_left = node::<COUNT>(
            left.name.clone(),
            left.value.clone(),
            left.left.clone(),
            middle.left.clone(),
            generation,
        );
        let new_right = node::<COUNT>(name, value, middle.right.clone(), right, generation);
        return node::<COUNT>(
            middle.name.clone(),
            middle.value.clone(),
            Some(new_left),
            Some(new_right),
            generation,
        );
    }
    if balance < -1 {
        let right = right.as_ref().expect("right-heavy node has a right child");
        if height(&right.right) >= height(&right.left) {
            let left = node::<COUNT>(name, value, left, right.left.clone(), generation);
            return node::<COUNT>(
                right.name.clone(),
                right.value.clone(),
                Some(left),
                right.right.clone(),
                generation,
            );
        }
        let middle = right
            .left
            .as_ref()
            .expect("inner-heavy right child has a left child");
        let new_left = node::<COUNT>(name, value, left, middle.left.clone(), generation);
        let new_right = node::<COUNT>(
            right.name.clone(),
            right.value.clone(),
            middle.right.clone(),
            right.right.clone(),
            generation,
        );
        return node::<COUNT>(
            middle.name.clone(),
            middle.value.clone(),
            Some(new_left),
            Some(new_right),
            generation,
        );
    }
    node::<COUNT>(name, value, left, right, generation)
}

fn insert<const COUNT: bool>(
    root: &Link,
    name: &SmolStr,
    value: &Option<SmolStr>,
    generation: u64,
) -> Arc<Node> {
    let Some(root) = root else {
        return node::<COUNT>(name.clone(), value.clone(), None, None, generation);
    };
    if COUNT {
        custom_property_diagnostics::index_visit();
    }
    match name.as_str().cmp(root.name.as_str()) {
        Ordering::Equal => {
            if value == &root.value {
                Arc::clone(root)
            } else {
                node::<COUNT>(
                    root.name.clone(),
                    value.clone(),
                    root.left.clone(),
                    root.right.clone(),
                    generation,
                )
            }
        }
        Ordering::Less => {
            let left = insert::<COUNT>(&root.left, name, value, generation);
            if root
                .left
                .as_ref()
                .is_some_and(|prior| Arc::ptr_eq(prior, &left))
            {
                return Arc::clone(root);
            }
            balanced::<COUNT>(
                root.name.clone(),
                root.value.clone(),
                Some(left),
                root.right.clone(),
                generation,
            )
        }
        Ordering::Greater => {
            let right = insert::<COUNT>(&root.right, name, value, generation);
            if root
                .right
                .as_ref()
                .is_some_and(|prior| Arc::ptr_eq(prior, &right))
            {
                return Arc::clone(root);
            }
            balanced::<COUNT>(
                root.name.clone(),
                root.value.clone(),
                root.left.clone(),
                Some(right),
                generation,
            )
        }
    }
}

fn owned_nodes<const COUNT: bool>(root: &Link, generation: u64) -> u64 {
    let Some(root) = root else {
        return 0;
    };
    if COUNT {
        custom_property_diagnostics::ownership_visit();
    }
    if root.generation != generation {
        return 0;
    }
    // Immutable older nodes cannot point to nodes created in this generation.
    // Pruning here counts the final batch, including rotations, without a scan
    // of inherited bindings or charges for discarded intermediate roots.
    1 + owned_nodes::<COUNT>(&root.left, generation) + owned_nodes::<COUNT>(&root.right, generation)
}

impl CustomPropertyIndex {
    pub(super) fn with_local(
        parent: &Self,
        local: &HashMap<SmolStr, Option<SmolStr>>,
        generation: u64,
    ) -> (Self, u64) {
        #[cfg(any(test, feature = "custom-property-diagnostics"))]
        if custom_property_diagnostics::enabled() {
            return Self::build::<true>(parent, local, generation);
        }
        Self::build::<false>(parent, local, generation)
    }

    fn build<const COUNT: bool>(
        parent: &Self,
        local: &HashMap<SmolStr, Option<SmolStr>>,
        generation: u64,
    ) -> (Self, u64) {
        let mut root = parent.root.clone();
        for (name, value) in local {
            root = Some(insert::<COUNT>(&root, name, value, generation));
        }
        let bytes = owned_nodes::<COUNT>(&root, generation) * NODE_BYTES;
        (Self { root }, bytes)
    }

    pub(super) fn get<const COUNT: bool>(&self, name: &str) -> Option<SmolStr> {
        let mut current = self.root.as_deref();
        while let Some(node) = current {
            if COUNT {
                custom_property_diagnostics::index_visit();
            }
            match name.cmp(node.name.as_str()) {
                Ordering::Less => current = node.left.as_deref(),
                Ordering::Equal => return node.value.clone(),
                Ordering::Greater => current = node.right.as_deref(),
            }
        }
        None
    }
}
