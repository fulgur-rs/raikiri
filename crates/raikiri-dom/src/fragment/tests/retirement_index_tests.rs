use super::INDEX_WORK;
use crate::Document;
use crate::fragment::{BreakToken, FragmentRect, FragmentTree, LayoutFragment};
use raikiri_style::{Origin, RuleTree};
use std::collections::{HashMap, HashSet};

fn fragment(node_id: usize, parent: Option<usize>) -> LayoutFragment {
    LayoutFragment {
        node_id,
        parent,
        fragmentainer: node_id % 3,
        rect: FragmentRect {
            x: node_id as f32,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        },
        fragmentainer_clip: None,
        fragment_index: 0,
        fragment_count: 1,
        line_start: None,
        line_end: None,
    }
}

// This intentionally rebuilds the original adjacency on every retirement.
fn reference_retire(
    fragments: &[LayoutFragment],
    retired: &mut HashSet<usize>,
    mut pending: Vec<usize>,
) {
    let mut children = HashMap::<usize, Vec<usize>>::new();
    for (index, fragment) in fragments.iter().enumerate() {
        if let Some(parent) = fragment.parent {
            children.entry(parent).or_default().push(index);
        }
    }
    while let Some(index) = pending.pop() {
        if retired.insert(index) {
            pending.extend(children.get(&index).into_iter().flatten().copied());
        }
    }
}

fn compare_retire(tree: &mut FragmentTree, pending: Vec<usize>) {
    let mut expected = tree.retired.clone();
    reference_retire(&tree.fragments, &mut expected, pending.clone());
    tree.retire_subtrees(pending);
    assert_eq!(tree.retired, expected);
    assert_indexes(tree);
}

fn assert_indexes(tree: &FragmentTree) {
    let mut expected = HashMap::<usize, Vec<usize>>::new();
    for (index, fragment) in tree.fragments.iter().enumerate() {
        if let Some(parent) = fragment.parent {
            expected.entry(parent).or_default().push(index);
        }
    }
    let mut actual = tree.children_by_parent.clone();
    for children in actual.values_mut() {
        children.sort_unstable();
    }
    assert_eq!(actual, expected);
    assert!(actual.values().map(Vec::len).sum::<usize>() <= tree.fragments.len());
    let mut roots = HashMap::<usize, Vec<usize>>::new();
    for (index, fragment) in tree.fragments.iter().enumerate() {
        if fragment.parent.is_none() && !tree.retired.contains(&index) {
            roots.entry(fragment.node_id).or_default().push(index);
        }
    }
    let mut indexed = HashMap::<usize, Vec<usize>>::new();
    for (&node, candidates) in &tree.roots_by_node {
        for &index in candidates {
            let fragment = &tree.fragments[index];
            if fragment.node_id == node
                && fragment.parent.is_none()
                && !tree.retired.contains(&index)
            {
                indexed.entry(node).or_default().push(index);
            }
        }
    }
    for entries in indexed.values_mut() {
        entries.sort_unstable();
    }
    assert_eq!(indexed, roots);
    // Every record is indexed under its node, retired ones included.
    let mut by_node = HashMap::<usize, Vec<usize>>::new();
    for (index, fragment) in tree.fragments.iter().enumerate() {
        by_node.entry(fragment.node_id).or_default().push(index);
    }
    assert_eq!(tree.by_node, by_node);
}

// The original compaction and node-order numbering, independent of indexes.
fn reference_finalize(tree: &FragmentTree) -> Vec<LayoutFragment> {
    let mut remap = vec![None; tree.fragments.len()];
    let mut kept = Vec::new();
    for (index, fragment) in tree.fragments.iter().enumerate() {
        if !tree.retired.contains(&index) {
            remap[index] = Some(kept.len());
            kept.push(*fragment);
        }
    }
    for fragment in &mut kept {
        fragment.parent = fragment.parent.and_then(|parent| remap[parent]);
    }
    let mut counts = HashMap::<usize, usize>::new();
    for fragment in &kept {
        *counts.entry(fragment.node_id).or_default() += 1;
    }
    let mut next = HashMap::<usize, usize>::new();
    for fragment in &mut kept {
        let index = next.entry(fragment.node_id).or_default();
        fragment.fragment_index = *index;
        fragment.fragment_count = counts[&fragment.node_id];
        *index += 1;
    }
    kept
}

fn compare_finalize(tree: &mut FragmentTree) {
    let expected = reference_finalize(tree);
    tree.finalize();
    assert_eq!(tree.fragments, expected);
    assert_indexes(tree);
}

#[test]
fn maintained_adjacency_matches_original_retirement_and_compaction() {
    for seed in 0..8 {
        let mut tree = FragmentTree::default();
        for index in 0..48 {
            let parent = (index % 4 != 0).then_some((index * 7 + seed) % 48);
            tree.try_push(fragment(index % 7, parent)).unwrap();
        }
        assert_indexes(&tree);
        for round in 0..8 {
            let root = (round * 5 + seed) % 48;
            compare_retire(&mut tree, vec![root, root, (root + 1) % 48]);
            // Children appended under a retired parent keep the original stop rule.
            tree.try_push(fragment(9, Some(root))).unwrap();
            compare_retire(&mut tree, vec![root]);
            let nodes = HashSet::from([round % 7]);
            let expected = tree
                .fragments
                .iter()
                .enumerate()
                .filter(|(_, fragment)| nodes.contains(&fragment.node_id))
                .map(|(index, _)| index)
                .collect::<HashSet<_>>();
            let mut retired = tree.retired.clone();
            retired.extend(expected);
            tree.retire_nodes(&nodes);
            assert_eq!(tree.retired, retired);
        }
        compare_retire(&mut tree, Vec::new());
        compare_finalize(&mut tree);
        let root = tree.try_push(fragment(12, None)).unwrap();
        tree.try_push(fragment(13, Some(root))).unwrap();
        tree.retire_previous_roots(12);
        assert!(tree.is_retired(root));
        compare_finalize(&mut tree);
    }
}

#[test]
fn root_attachment_registers_forward_edges_once_and_preserves_root_filtering() {
    let mut tree = FragmentTree::default();
    let child = tree.try_push(fragment(1, None)).unwrap();
    let parent = tree.try_push(fragment(2, None)).unwrap();
    tree.attach_root(child, parent);
    for _ in 0..64 {
        tree.attach_root(child, child);
    }
    tree.attach_root(usize::MAX, parent);
    assert_eq!(tree.fragments[child].parent, Some(parent));
    assert_eq!(tree.children_by_parent[&parent], [child]);
    tree.retire_previous_roots(1);
    assert!(!tree.is_retired(child));
    compare_retire(&mut tree, vec![parent]);
    assert!(tree.is_retired(child));
    let later = tree.try_push(fragment(3, Some(parent))).unwrap();
    compare_retire(&mut tree, vec![parent]);
    assert!(!tree.is_retired(later));
    compare_finalize(&mut tree);
    assert_eq!(tree.fragments, [fragment(3, None)]);
}

#[test]
fn subset_retirement_does_not_expand_and_cycles_still_terminate() {
    let mut tree = FragmentTree::default();
    let root = tree.try_push(fragment(1, None)).unwrap();
    let child = tree.try_push(fragment(2, Some(root))).unwrap();
    tree.retire_nodes(&HashSet::from([1]));
    compare_retire(&mut tree, vec![root]);
    assert!(!tree.is_retired(child));
    compare_finalize(&mut tree);
    let a = tree.try_push(fragment(3, None)).unwrap();
    let b = tree.try_push(fragment(4, Some(a))).unwrap();
    tree.attach_root(a, b);
    compare_retire(&mut tree, vec![a, b]);
    let own = tree.try_push(fragment(5, None)).unwrap();
    tree.attach_root(own, own);
    compare_retire(&mut tree, vec![own]);
    compare_finalize(&mut tree);
}

#[test]
fn cloned_and_cleared_trees_keep_independent_indexes_and_limits() {
    let mut tree = FragmentTree {
        limit: 2,
        ..FragmentTree::default()
    };
    let root = tree.try_push(fragment(1, None)).unwrap();
    tree.try_push(fragment(2, Some(root))).unwrap();
    let before = tree.children_by_parent.clone();
    assert_eq!(tree.try_push(fragment(3, Some(root))), None);
    assert_eq!(tree.children_by_parent, before);
    assert_indexes(&tree);
    let mut cloned = tree.clone();
    compare_retire(&mut cloned, vec![root]);
    assert!(!tree.is_retired(root));
    cloned.clear();
    assert!(cloned.children_by_parent.is_empty());
    assert!(cloned.roots_by_node.is_empty());
    assert_eq!(cloned.limit, 2);
    assert!(!cloned.limit_exceeded);
    cloned.try_push(fragment(8, None)).unwrap();
    assert_indexes(&cloned);
    assert_indexes(&tree);
}

#[test]
fn finalized_replacement_rebuilds_indexes_without_resetting_other_state() {
    let mut tree = FragmentTree {
        limit: 8,
        break_flow_work_used: 3,
        limit_exceeded: true,
        ..FragmentTree::default()
    };
    let token = BreakToken {
        node_id: 5,
        child_index: 2,
        line_index: 3,
    };
    tree.record_break(token);
    tree.try_push(fragment(9, None)).unwrap();
    tree.replace_finalized_fragments(vec![fragment(1, Some(1)), fragment(2, None)]);
    assert_eq!(tree.limit, 8);
    assert_eq!(tree.break_flow_work_used, 3);
    assert!(tree.limit_exceeded);
    assert_eq!(tree.break_tokens, [token]);
    assert_indexes(&tree);
    tree.retire_previous_roots(9);
    assert!(tree.retired.is_empty());
    tree.retire_previous_roots(2);
    assert!(tree.is_retired(0));
    assert!(tree.is_retired(1));
    compare_finalize(&mut tree);
    tree.replace_finalized_fragments(Vec::new());
    assert_indexes(&tree);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic]
fn replacement_requires_finalizing_pending_retirements() {
    let mut tree = FragmentTree::default();
    tree.try_push(fragment(1, None)).unwrap();
    tree.retire_subtrees(vec![0]);
    tree.replace_finalized_fragments(Vec::new());
}

fn nested_document(groups: usize) -> Document {
    let mut doc = Document::new();
    doc.set_font_collection(shodo::font::FontCollection::new(
        &shodo::limits::Limits::default(),
    ));
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    for _ in 0..groups {
        let outer = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:100px;columns:4;column-gap:0"),
        );
        let inner = doc.append_element(
            Some(outer),
            "div",
            Some("display:block;columns:2;column-gap:0"),
        );
        doc.append_element(
            Some(inner),
            "div",
            Some("display:block;width:12.5px;height:400px"),
        );
    }
    doc.mark_in_document_flags();
    doc
}

#[test]
fn fragment_retirement_index_work_scales_with_independent_groups() {
    for groups in [16, 32, 64, 128, 256] {
        let mut doc = nested_document(groups);
        let mut rules = RuleTree::empty();
        rules.add_stylesheet("html,body{display:block;margin:0}", Origin::Author);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        let mut page = raikiri_traits::PageBox::new();
        page.width = 800.0;
        page.height = 600.0;
        INDEX_WORK.with(|work| work.set([0; 4]));
        crate::layout_single_page(&mut doc, &cascade, page).unwrap();
        let counts = INDEX_WORK.with(|work| work.get());
        println!("groups={groups} index_work={counts:?}");
        assert_eq!(doc.node_count(), 3 + groups * 3);
        assert_eq!(doc.layout_fragments().len(), groups * 5);
        assert!(counts[1] > 0);
        assert!(counts[2] > 0);
        assert!(
            counts.iter().sum::<usize>() <= 32 * groups,
            "{groups}: {counts:?}"
        );
    }
}

/// A tree of `nodes` source nodes with `per_node` records each, laid out the
/// way the nodes of independent containers are: records are pushed one node
/// after the other.
fn tree_of_nodes(nodes: usize, per_node: usize) -> FragmentTree {
    let mut tree = FragmentTree::default();
    for node in 0..nodes {
        let first = tree.try_push(fragment(node, None)).unwrap();
        for _ in 1..per_node {
            tree.try_push(fragment(node, Some(first))).unwrap();
        }
    }
    tree
}

#[test]
fn retiring_nodes_visits_only_the_records_of_those_nodes() {
    for nodes in [64, 128, 256, 512, 1024] {
        let mut tree = tree_of_nodes(nodes, 3);
        let mut expected = HashSet::new();
        INDEX_WORK.with(|work| work.set([0; 4]));
        // One call per pair of nodes, as one container with a spanner is laid
        // out after another.
        for first in (0..nodes).step_by(2) {
            let set = HashSet::from([first, first + 1, nodes + first]);
            expected.extend(
                tree.fragments
                    .iter()
                    .enumerate()
                    .filter(|(_, fragment)| set.contains(&fragment.node_id))
                    .map(|(index, _)| index),
            );
            tree.retire_nodes(&set);
            assert_eq!(tree.retired, expected, "{nodes}: {first}");
        }
        // Each record of a node is visited once: 3 for each of the nodes
        // below `nodes`, and none for the nodes that have no records.
        let visited = INDEX_WORK.with(|work| work.get())[3];
        assert_eq!(visited, nodes * 3, "{nodes}");
        assert_eq!(tree.retired.len(), nodes * 3);
        assert_indexes(&tree);
    }
}

#[test]
fn a_record_pushed_after_a_retirement_is_found_by_the_next_one() {
    let mut tree = tree_of_nodes(4, 2);
    tree.retire_nodes(&HashSet::from([1]));
    let later = tree.try_push(fragment(1, None)).unwrap();
    assert!(!tree.is_retired(later));
    tree.retire_nodes(&HashSet::from([1]));
    assert!(tree.is_retired(later));
    assert_eq!(tree.retired.len(), 3);
    // A rejected push is not indexed.
    let mut full = FragmentTree {
        limit: 1,
        ..FragmentTree::default()
    };
    full.try_push(fragment(5, None)).unwrap();
    assert_eq!(full.try_push(fragment(6, None)), None);
    assert!(!full.by_node.contains_key(&6));
    // Compaction drops the three retired records of node 1 (two from the
    // start and the later one) and renumbers the others; the index follows.
    tree.finalize();
    assert_eq!(tree.fragments.len(), 6);
    assert_indexes(&tree);
    tree.retire_nodes(&HashSet::from([0, 2]));
    assert_eq!(tree.retired.len(), 4);
}

fn spanner_document(groups: usize) -> Document {
    let mut doc = Document::new();
    doc.set_font_collection(shodo::font::FontCollection::new(
        &shodo::limits::Limits::default(),
    ));
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    for _ in 0..groups {
        let owner = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:100px;columns:2;column-gap:0"),
        );
        doc.append_element(Some(owner), "div", Some("display:block;height:20px"));
        // The spanner holds a container of its own, which leaves records to
        // retire when the owner is laid out again.
        let spanner = doc.append_element(Some(owner), "div", Some("display:block;column-span:all"));
        let nested = doc.append_element(
            Some(spanner),
            "div",
            Some("display:block;columns:2;column-gap:0"),
        );
        doc.append_element(
            Some(nested),
            "div",
            Some("display:block;width:12.5px;height:40px"),
        );
    }
    doc.mark_in_document_flags();
    doc
}

#[test]
fn preliminary_spanner_records_are_retired_in_work_that_scales_with_the_spanners() {
    for groups in [16, 32, 64, 128, 256] {
        let mut doc = spanner_document(groups);
        let mut rules = RuleTree::empty();
        rules.add_stylesheet("html,body{display:block;margin:0}", Origin::Author);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        let mut page = raikiri_traits::PageBox::new();
        page.width = 800.0;
        page.height = 600.0;
        INDEX_WORK.with(|work| work.set([0; 4]));
        crate::layout_single_page(&mut doc, &cascade, page).unwrap();
        let visited = INDEX_WORK.with(|work| work.get())[3];
        println!(
            "groups={groups} visited={visited} fragments={}",
            doc.layout_fragments().len()
        );
        assert!(visited > 0, "{groups}: nothing was retired");
        // The records of the nodes of one spanner, not of all the fragments
        // laid out so far: a scan of the arena for each of the two layouts of
        // an owner would visit about `groups` times as many.
        assert!(
            visited <= 16 * groups,
            "{groups}: {visited} records visited"
        );
    }
}
