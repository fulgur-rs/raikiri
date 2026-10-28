use super::*;

fn check_linear_graph(graph: &[Vec<usize>], expected: &[bool]) {
    let mut work = CycleWork::default();
    assert_eq!(cycle_members_in_graph(graph, &mut work), expected);
    let edges: usize = graph.iter().map(Vec::len).sum();
    assert!(
        work.steps <= 16 * (graph.len() + edges),
        "{} steps for {} vertices and {edges} edges",
        work.steps,
        graph.len()
    );
}

#[test]
fn cycles_include_finished_dfs_vertices_still_in_the_same_component() {
    check_linear_graph(&[vec![1, 2], vec![0], vec![1]], &[true, true, true]);
}

#[test]
fn graph_detection_handles_self_edges_duplicates_components_and_isolated_nodes() {
    check_linear_graph(&[], &[]);
    check_linear_graph(&[vec![]], &[false]);
    check_linear_graph(&[vec![0, 0]], &[true]);
    check_linear_graph(
        &[
            vec![1, 1],
            vec![0, 2],
            vec![3],
            vec![2],
            vec![4],
            vec![4],
            vec![],
        ],
        &[true, true, true, true, true, false, false],
    );
}

#[test]
fn dense_graph_work_is_linear_in_its_vertices_and_edges() {
    for count in [32, 64, 128] {
        let graph: Vec<_> = (0..count).map(|_| (0..count).collect()).collect();
        check_linear_graph(&graph, &vec![true; count]);
    }
}

#[test]
fn many_backedges_do_not_rescan_active_paths() {
    let count = 256;
    let mut graph: Vec<_> = (0..count - 1).map(|node| vec![node + 1]).collect();
    graph.push((0..count - 1).collect());
    check_linear_graph(&graph, &vec![true; count]);
}

#[test]
fn long_sparse_graph_uses_heap_stacks() {
    let count = 20_000;
    let mut graph: Vec<_> = (0..count - 1).map(|node| vec![node + 1]).collect();
    graph.push(Vec::new());
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || check_linear_graph(&graph, &vec![false; count]))
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn small_graphs_match_a_transitive_closure_oracle_for_both_edge_orders() {
    let count = 4;
    for mask in 0usize..1 << (count * count) {
        let mut graph: Vec<Vec<usize>> = (0..count)
            .map(|source| {
                (0..count)
                    .filter(|&target| mask & (1 << (source * count + target)) != 0)
                    .collect()
            })
            .collect();
        let mut reachable = vec![vec![false; count]; count];
        for (source, targets) in graph.iter().enumerate() {
            for &target in targets {
                reachable[source][target] = true;
            }
        }
        for via in 0..count {
            for source in 0..count {
                for target in 0..count {
                    if reachable[source][via] && reachable[via][target] {
                        reachable[source][target] = true;
                    }
                }
            }
        }
        let expected: Vec<_> = (0..count).map(|node| reachable[node][node]).collect();
        check_linear_graph(&graph, &expected);
        for edges in &mut graph {
            edges.reverse();
        }
        check_linear_graph(&graph, &expected);
    }
}

#[test]
fn map_and_declaration_order_do_not_change_cycle_members_or_linear_work() {
    let entries = [
        ("--a", "var(--b) var(--c)"),
        ("--b", "var(--a)"),
        ("--c", "var(--b)"),
        ("--dependent", "var(--a, green)"),
        ("--safe", "blue"),
    ];
    let expected: HashSet<SmolStr> = ["--a", "--b", "--c"].into_iter().map(Into::into).collect();
    for rotation in 0..entries.len() {
        for reverse in [false, true] {
            let mut order: Vec<_> = (0..entries.len()).collect();
            order.rotate_left(rotation);
            if reverse {
                order.reverse();
            }
            for _ in 0..8 {
                let local: HashMap<SmolStr, SmolStr> = order
                    .iter()
                    .map(|&index| (entries[index].0.into(), entries[index].1.into()))
                    .collect();
                let mut work = CycleWork::default();
                assert_eq!(find_cycle_members_with_work(&local, &mut work), expected);
                assert!(work.steps <= 16 * (local.len() + work.references));
            }
        }
    }
}

#[test]
fn fallback_edges_are_cyclic_and_invalid_raw_values_discard_all_edges() {
    let local: HashMap<SmolStr, SmolStr> = [
        ("--fallback", "var(--missing, var(--peer))"),
        ("--peer", "var(--fallback)"),
        ("--safe", "var(--outside, green)"),
        ("--broken", "var(--peer) var(not-a-custom-property)"),
        ("--ref-broken", "var(--broken, red)"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    assert_eq!(
        find_cycle_members(&local),
        HashSet::from(["--fallback".into(), "--peer".into()])
    );
    let inherited = CustomPropertyEnvironment::from_map(HashMap::new());
    let result = resolve_local_custom_properties(&inherited, &local);
    assert_eq!(result["--broken"], None);
    assert_eq!(result["--ref-broken"].as_deref(), Some("red"));
    assert_eq!(result["--safe"].as_deref(), Some("green"));
}

#[test]
fn inherited_values_are_not_graph_edges_and_local_cycles_mask_them() {
    let inherited = CustomPropertyEnvironment::from_map(HashMap::from([
        ("--a".into(), "blue".into()),
        ("--parent".into(), "green".into()),
    ]));
    let local = HashMap::from([
        ("--a".into(), "var(--a, red)".into()),
        ("--b".into(), "var(--parent)".into()),
        ("--c".into(), "var(--b)".into()),
    ]);
    assert_eq!(find_cycle_members(&local), HashSet::from(["--a".into()]));
    let result = resolve_local_custom_properties(&inherited, &local);
    assert_eq!(result["--a"], None);
    assert_eq!(result["--b"].as_deref(), Some("green"));
    assert_eq!(result["--c"].as_deref(), Some("green"));
}

#[test]
fn work_counts_input_bytes_and_nonlocal_references_too() {
    let local: HashMap<SmolStr, SmolStr> = HashMap::from([
        ("--a".into(), "var(--outside) ".repeat(128).into()),
        ("--b".into(), "red".into()),
    ]);
    let mut work = CycleWork::default();
    assert!(find_cycle_members_with_work(&local, &mut work).is_empty());
    assert_eq!(work.references, 128);
    assert_eq!(work.input_bytes, local.values().map(SmolStr::len).sum());
    assert!(work.steps <= 16 * (local.len() + work.references));
}

#[test]
fn cyclic_finished_branches_cannot_escape_through_their_own_fallbacks() {
    for declarations in [
        "--a: var(--b) var(--c); --b: var(--a); --c: var(--b, red); color: var(--c, blue)",
        "--c: var(--b, red); --b: var(--a); --a: var(--b) var(--c); color: var(--c, blue)",
    ] {
        for _ in 0..16 {
            assert_eq!(cascade_doc("", "p", Some(declarations)).color, BLUE);
        }
    }
}
