use super::*;

#[test]
fn a_seed_always_generates_the_same_case() {
    for seed in [0, 1, 7, 12_345] {
        assert_eq!(
            describe(&generate(seed)),
            describe(&generate(seed)),
            "seed {seed}"
        );
    }
}

#[test]
fn distinct_seeds_generate_distinct_cases() {
    let cases: Vec<String> = (0..20).map(|seed| describe(&generate(seed))).collect();
    for (i, a) in cases.iter().enumerate() {
        for b in &cases[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn generated_trees_are_consistent() {
    for seed in 0..200 {
        let case = generate(seed);
        let nodes = &case.doc.nodes;
        assert!(
            case.sheets
                .iter()
                .any(|(_, origin)| *origin == Origin::Author)
        );
        for (id, node) in nodes.iter().enumerate() {
            for &child in &node.children {
                assert_eq!(nodes[child].parent, Some(id), "seed {seed} node {child}");
                // A node outside the document never has a child inside it.
                assert!(
                    node.in_document || !nodes[child].in_document,
                    "seed {seed} node {child}"
                );
            }
        }
    }
}
