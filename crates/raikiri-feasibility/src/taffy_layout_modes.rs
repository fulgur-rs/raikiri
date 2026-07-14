//! taffy block/flex/grid + column-count parallel, nzv-spike for raikiri-spike-nzv.6
//!
//! Verifies that `taffy 0.12` — configured for Raikiri with features
//! `[block_layout, flexbox, grid, content_size, calc, std]` — can drive all
//! three CSS layout modes (design doc section 4 and section 12 "CSS layout
//! pipeline") and that multiple independent trees can lay out concurrently on
//! separate threads (shape the M2 column-count sharding will take, per the
//! paged-media section).
//!
//! ## Important finding: workspace does NOT enable taffy's `taffy_tree` feature
//!
//! The workspace `taffy` dep enables `[block_layout, flexbox, grid,
//! content_size, calc, std]` — deliberately omitting the `taffy_tree` feature
//! which would pull in `slotmap` and expose the built-in `TaffyTree` struct.
//! This matches how `blitz-dom` uses taffy (blitz owns its DOM arena and
//! implements the taffy traits on it), and it matches the raikiri design
//! doc's `LayoutBuffer` concept — raikiri will own its own node arena.
//!
//! So this spike **cannot** use `taffy::TaffyTree` directly. Instead it uses
//! taffy's **low-level trait API** — `LayoutPartialTree`,
//! `LayoutBlockContainer`, `LayoutFlexboxContainer`, `LayoutGridContainer`,
//! and the free functions `compute_root_layout` / `compute_block_layout` /
//! `compute_flexbox_layout` / `compute_grid_layout` — implemented on a tiny
//! `SpikeTree` (`Vec<Node>` arena, `NodeId = usize`). This is the same shape
//! raikiri-dom will use at M1.
//!
//! ## Verification
//!
//! * Build one `SpikeTree` per layout mode (`Display::Block`, `Display::Flex`,
//!   `Display::Grid`), each with a root + two 100x50 children, and confirm
//!   the root `Layout` has a non-degenerate width and height.
//! * Build N independent `SpikeTree`s and lay them out concurrently via
//!   `std::thread::scope`. The compiler enforces `Send` at the thread
//!   boundary — proving each column can own its own arena for column-count
//!   parallel layout without any interior-mutability workaround.
//!
//! `rayon` is not a dependency of this crate (M0 constraint — no Cargo.toml
//! edits from the spike), so we simulate the pool with `std::thread::scope`.
//! The property proven — `SpikeTree: Send` — is what a `rayon::scope` would
//! also enforce.
//!
//! ## Second finding: `Style: !Send` under the `calc` feature
//!
//! With the `calc` feature enabled (workspace default), taffy's `Dimension`
//! (via `CompactLength`) can hold a `*const ()` opaque pointer into a caller-
//! owned calc expression arena. Raw pointers are `!Send`, so `Style: !Send`
//! and therefore `SpikeTree: !Send` by auto-trait derivation. This is an
//! **M1/M2 design concern** for the column-count shard-per-column pass — the
//! raikiri `LayoutBuffer` will need one of:
//!
//! 1. `unsafe impl Send` for the buffer, upheld by the invariant that any
//!    calc-tree pointers live inside the same buffer (self-contained arena)
//!    and are therefore safe to move to another thread with the buffer.
//! 2. A "no calc across thread boundary" invariant enforced at construction.
//! 3. A newtype wrapper (`SendableStyle`) with the same discipline.
//!
//! For this spike we take approach (1): our `SpikeTree` never populates a
//! calc pointer (only `Dimension::length` and `length()` track sizes are
//! used), so `unsafe impl Send for SpikeTree` is sound *for this test*.
//! We record the finding here so the M0 feasibility report can flag it.

use taffy::prelude::*;
use taffy::{
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_leaf_layout, compute_root_layout, AvailableSpace, Cache, CacheTree, Dimension, Display,
    Layout, LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer, LayoutInput,
    LayoutOutput, LayoutPartialTree, NodeId, Size, Style, TraversePartialTree, TraverseTree,
};

/// One node in the spike's flat arena.
struct Node {
    style: Style,
    children: Vec<usize>,
    cache: Cache,
    unrounded_layout: Layout,
}

impl Node {
    fn new(style: Style) -> Self {
        Self {
            style,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
        }
    }
}

/// A minimal `Vec`-backed tree that implements the taffy layout traits.
/// Mirrors what raikiri-dom will do at M1 — no `slotmap`, no dependency on
/// the `taffy_tree` cargo feature.
struct SpikeTree {
    nodes: Vec<Node>,
}

impl SpikeTree {
    fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    fn add(&mut self, node: Node) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    fn append_child(&mut self, parent: usize, child: usize) {
        self.nodes[parent].children.push(child);
    }

    fn node(&self, id: NodeId) -> &Node {
        &self.nodes[usize::from(id)]
    }
    fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[usize::from(id)]
    }
}

struct ChildIter<'a>(core::slice::Iter<'a, usize>);
impl Iterator for ChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(NodeId::from)
    }
}

impl TraversePartialTree for SpikeTree {
    type ChildIter<'a> = ChildIter<'a>;
    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        ChildIter(self.node(node_id).children.iter())
    }
    fn child_count(&self, node_id: NodeId) -> usize {
        self.node(node_id).children.len()
    }
    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        NodeId::from(self.node(node_id).children[index])
    }
}

impl TraverseTree for SpikeTree {}

impl CacheTree for SpikeTree {
    fn cache_get(&self, node_id: NodeId, inputs: &LayoutInput) -> Option<LayoutOutput> {
        self.node(node_id).cache.get(inputs)
    }
    fn cache_store(&mut self, node_id: NodeId, inputs: &LayoutInput, layout_output: LayoutOutput) {
        self.node_mut(node_id).cache.store(inputs, layout_output)
    }
    fn cache_clear(&mut self, node_id: NodeId) {
        self.node_mut(node_id).cache.clear();
    }
}

impl LayoutPartialTree for SpikeTree {
    type CustomIdent = String;
    type CoreContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_core_container_style(&self, node_id: NodeId) -> Self::CoreContainerStyle<'_> {
        &self.node(node_id).style
    }

    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        self.node_mut(node_id).unrounded_layout = *layout;
    }

    fn resolve_calc_value(&self, _val: *const (), _basis: f32) -> f32 {
        0.0
    }

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        compute_cached_layout(self, node_id, inputs, |tree, node_id, inputs| {
            let is_leaf = tree.node(node_id).children.is_empty();
            let display = tree.node(node_id).style.display;
            if is_leaf {
                let style = tree.node(node_id).style.clone();
                compute_leaf_layout(
                    inputs,
                    &style,
                    |_val, _basis| 0.0,
                    |_known, _avail| Size::ZERO,
                )
            } else {
                match display {
                    Display::Block => compute_block_layout(tree, node_id, inputs, None),
                    Display::Flex => compute_flexbox_layout(tree, node_id, inputs),
                    Display::Grid => compute_grid_layout(tree, node_id, inputs),
                    Display::None => LayoutOutput::HIDDEN,
                }
            }
        })
    }
}

impl LayoutBlockContainer for SpikeTree {
    type BlockContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: NodeId) -> Self::BlockContainerStyle<'_> {
        &self.node(node_id).style
    }
    fn get_block_child_style(&self, child_node_id: NodeId) -> Self::BlockItemStyle<'_> {
        &self.node(child_node_id).style
    }
}

impl LayoutFlexboxContainer for SpikeTree {
    type FlexboxContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node_id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        &self.node(node_id).style
    }
    fn get_flexbox_child_style(&self, child_node_id: NodeId) -> Self::FlexboxItemStyle<'_> {
        &self.node(child_node_id).style
    }
}

impl LayoutGridContainer for SpikeTree {
    type GridContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: NodeId) -> Self::GridContainerStyle<'_> {
        &self.node(node_id).style
    }
    fn get_grid_child_style(&self, child_node_id: NodeId) -> Self::GridItemStyle<'_> {
        &self.node(child_node_id).style
    }
}

// SAFETY: Under the `calc` feature, `Style` contains a `*const ()` that
// points into a caller-owned calc expression arena — so `Style: !Send` and,
// by auto-trait derivation, `SpikeTree: !Send` too. This spike never
// constructs any calc-backed dimensions (only `Dimension::length` and
// `length()` track sizes), so the raw pointer discriminants are never
// populated and it is sound to send a whole `SpikeTree` to another thread.
// See module docs "Second finding" for the full M1/M2 design concern.
#[allow(unsafe_code)]
unsafe impl Send for SpikeTree {}

/// Sentinel: `SpikeTree` must be `Send` so we can move one per worker
/// thread — the same property `raikiri-dom` will need for the M2
/// column-count shard-per-column pass. Compile-time proof (relies on the
/// `unsafe impl Send` above).
#[allow(dead_code)]
fn _assert_spike_tree_send() {
    fn assert_send<T: Send>() {}
    assert_send::<SpikeTree>();
}

/// Build a 3-node tree (root + two 100x50 children) whose root uses
/// `display`. The root has an explicit width of 400 so the layout has room
/// to be non-degenerate under all three algorithms.
fn build_tree(display: Display) -> (SpikeTree, NodeId) {
    let mut tree = SpikeTree::new();

    let leaf_style = Style {
        size: Size {
            width: Dimension::length(100.0),
            height: Dimension::length(50.0),
        },
        ..Default::default()
    };
    let a = tree.add(Node::new(leaf_style.clone()));
    let b = tree.add(Node::new(leaf_style));

    let mut root_style = Style {
        display,
        size: Size {
            width: Dimension::length(400.0),
            height: Dimension::auto(),
        },
        ..Default::default()
    };
    // Grid needs explicit tracks or children get placed in an implicit
    // auto-sized 1×N. Give it a 2-column grid to exercise the algorithm.
    if matches!(display, Display::Grid) {
        root_style.grid_template_columns = vec![length(200.0), length(200.0)];
        root_style.grid_template_rows = vec![length(50.0)];
    }
    let root = tree.add(Node::new(root_style));
    tree.append_child(root, a);
    tree.append_child(root, b);
    (tree, NodeId::from(root))
}

/// Lay out `tree` at 800x600 available space, return the root size.
fn layout_root(tree: &mut SpikeTree, root: NodeId) -> (f32, f32) {
    compute_root_layout(
        tree,
        root,
        Size {
            width: AvailableSpace::Definite(800.0),
            height: AvailableSpace::Definite(600.0),
        },
    );
    let layout = tree.node(root).unrounded_layout;
    (layout.size.width, layout.size.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// All three CSS display modes produce a non-degenerate root box.
    #[test]
    fn all_three_display_modes_produce_non_degenerate_size() {
        for display in [Display::Block, Display::Flex, Display::Grid] {
            let (mut tree, root) = build_tree(display);
            let (w, h) = layout_root(&mut tree, root);
            assert!(
                w > 0.0 && h > 0.0,
                "display={display:?} produced degenerate size ({w}x{h})",
            );
            // Root width was pinned to 400px — sanity check taffy honored it.
            assert!(
                (w - 400.0).abs() < 0.5,
                "display={display:?}: root width should be ~400, got {w}",
            );
        }
    }

    /// N independent trees can be laid out on N threads simultaneously.
    /// This is the shape the M2 column-count shard-per-column pass will use:
    /// each column owns an arena, threads never share tree state, no `Mutex`
    /// or `RefCell` needed. `thread::scope` proves — at compile time — that
    /// `SpikeTree: Send`, which is exactly what a `rayon::scope` would want.
    #[test]
    fn independent_trees_lay_out_in_parallel() {
        const N: usize = 4;

        let trees: Vec<(SpikeTree, NodeId)> = (0..N)
            .map(|i| {
                let display = match i % 3 {
                    0 => Display::Block,
                    1 => Display::Flex,
                    _ => Display::Grid,
                };
                build_tree(display)
            })
            .collect();

        let sizes: Vec<(f32, f32)> = thread::scope(|s| {
            let handles: Vec<_> = trees
                .into_iter()
                .map(|(mut tree, root)| s.spawn(move || layout_root(&mut tree, root)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        assert_eq!(sizes.len(), N);
        for (i, (w, h)) in sizes.iter().enumerate() {
            assert!(
                *w > 0.0 && *h > 0.0,
                "shard {i}: degenerate size ({w}x{h}) from parallel layout",
            );
        }
    }
}
