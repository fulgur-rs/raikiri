use super::*;
use crate::node::NodeData;

fn original_dfs(d: &Document, node: usize, parent: usize) -> bool {
    let mut stack = vec![node];
    let mut visited = std::collections::HashSet::new();
    while let Some(current) = stack.pop() {
        if current == parent {
            return true;
        }
        if visited.insert(current)
            && let Some(value) = d.get_node(current)
        {
            stack.extend(value.children.iter().copied());
            if let NodeData::Element(element) = &value.data
                && let Some(contents) = element.template_contents
            {
                stack.push(contents);
            }
        }
    }
    false
}

#[test]
fn fresh_wrapper_cycle_checks_do_not_visit_moved_subtrees_or_allocate_scratch() {
    let mut d = Document::new();
    let mut current = d.create_detached_element("div").unwrap();
    crate::insertion_diagnostics::begin_insertion_diagnostics();
    for _ in 0..64 {
        let fresh = d.create_detached_element("div").unwrap();
        d.append_child(fresh, current).unwrap();
        current = fresh;
    }
    let work = crate::insertion_diagnostics::finish_insertion_diagnostics();
    assert_eq!(work.calls, 64);
    assert_eq!(work.dfs_visits, 0);
    assert_eq!(work.fast_paths, 64);
    assert_eq!(work.parent_reads, 64);
    assert_eq!(work.scratch_growths, 0);
}

#[test]
fn cycle_check_matches_literal_dfs_for_valid_ordinary_fragment_and_template_trees() {
    for seed in 0..40_u64 {
        let mut state = seed + 1;
        let mut d = Document::new();
        let html = d.create_detached_element("html").unwrap();
        d.attach_child(0, html);
        let mut destinations = vec![html, d.create_detached_fragment()];
        for i in 0..24 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let template = i % 5 == 0;
            let id = d
                .create_detached_element(if template { "template" } else { "div" })
                .unwrap();
            if i % 4 != 0 {
                let parent = destinations[(state as usize) % destinations.len()];
                d.attach_child(parent, id);
            }
            destinations.push(id);
            if template {
                destinations.push(d.allocate_template_fragment_root(id));
            }
        }
        crate::insertion_diagnostics::begin_insertion_diagnostics();
        for node in (0..d.node_count()).chain([usize::MAX]) {
            for parent in (0..d.node_count()).chain([usize::MAX]) {
                assert_eq!(
                    d.is_inclusive_ancestor(node, parent),
                    original_dfs(&d, node, parent)
                );
            }
        }
        let work = crate::insertion_diagnostics::finish_insertion_diagnostics();
        assert!(work.dfs_visits > 0 && work.fast_paths > 0);
    }
}

#[test]
fn detached_destination_with_children_and_template_contents_is_still_safe() {
    let mut d = Document::new();
    let template = d.create_detached_element("template").unwrap();
    let contents = d.allocate_template_fragment_root(template);
    let content_child = d.create_detached_element("span").unwrap();
    d.attach_child(contents, content_child);
    let ordinary = d.create_detached_element("p").unwrap();
    d.attach_child(template, ordinary);
    let node = d.create_detached_element("div").unwrap();
    crate::insertion_diagnostics::begin_insertion_diagnostics();
    assert!(!d.is_inclusive_ancestor(node, template));
    assert!(d.is_inclusive_ancestor(template, template));
    let work = crate::insertion_diagnostics::finish_insertion_diagnostics();
    assert_eq!(work.fast_paths, 1);
    assert_eq!(work.dfs_searches, 0);
    d.append_child(template, node).unwrap();
    assert_eq!(d.parent_of(node), Some(template));
    assert_eq!(d.parent_of(contents), None);
    assert_eq!(d.parent_of(content_child), Some(contents));
}

fn shape(d: &Document) -> Vec<(Option<usize>, Vec<usize>, Option<usize>)> {
    (0..d.node_count())
        .map(|id| {
            let node = d.get_node(id).unwrap();
            (
                d.parent_of(id),
                node.children.clone(),
                node.template_contents(),
            )
        })
        .collect()
}

#[test]
fn nested_host_cycles_and_invalid_insertions_leave_the_tree_unchanged() {
    let mut d = Document::new();
    let outer = d.create_detached_element("div").unwrap();
    let host = d.create_detached_element("template").unwrap();
    d.attach_child(outer, host);
    let contents = d.allocate_template_fragment_root(host);
    let nested = d.create_detached_element("template").unwrap();
    d.attach_child(contents, nested);
    let inner_contents = d.allocate_template_fragment_root(nested);
    let inner = d.create_detached_element("span").unwrap();
    d.attach_child(inner_contents, inner);
    let before = shape(&d);
    for (parent, node) in [
        (outer, outer),
        (inner, outer),
        (contents, host),
        (inner, host),
        (inner_contents, outer),
    ] {
        assert!(matches!(
            d.pre_insert(parent, node, None),
            Err(DomMutationError::HierarchyRequest(_))
        ));
        assert_eq!(shape(&d), before);
    }
    assert!(d.append_child(outer, usize::MAX).is_err());
    assert!(d.pre_insert(outer, inner, Some(usize::MAX)).is_err());
    assert_eq!(shape(&d), before);
}

#[test]
fn deep_leaf_checks_stay_linear_and_fragment_destinations_keep_dfs() {
    let mut d = Document::new();
    let mut parent = d.create_detached_element("div").unwrap();
    crate::insertion_diagnostics::begin_insertion_diagnostics();
    for _ in 0..128 {
        let leaf = d.create_detached_element("div").unwrap();
        d.append_child(parent, leaf).unwrap();
        parent = leaf;
    }
    let work = crate::insertion_diagnostics::finish_insertion_diagnostics();
    assert_eq!(work.parent_reads, 128);
    assert_eq!(work.fast_paths, 1);
    assert_eq!(work.dfs_visits, 127);
    let fragment = d.create_detached_fragment();
    crate::insertion_diagnostics::begin_insertion_diagnostics();
    assert!(!d.is_inclusive_ancestor(parent, fragment));
    let work = crate::insertion_diagnostics::finish_insertion_diagnostics();
    assert_eq!(work.dfs_searches, 1);
    assert_eq!(work.dfs_visits, 1);
    assert_eq!(work.fast_paths, 0);
}

#[test]
fn legacy_dfs_still_terminates_on_unchecked_primitive_cycles() {
    let mut d = Document::new();
    let a = d.create_detached_element("div").unwrap();
    let b = d.create_detached_element("div").unwrap();
    d.attach_child(a, b);
    d.attach_child(b, a);
    let missing = d.create_detached_fragment();
    assert!(!d.is_inclusive_ancestor(a, missing));
    assert!(d.is_inclusive_ancestor(a, b));
}
