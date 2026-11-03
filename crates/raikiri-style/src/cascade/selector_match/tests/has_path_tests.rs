use super::*;

fn ids(raw: &[u64]) -> Vec<StyleNodeId> {
    raw.iter().map(|&id| StyleNodeId::new(id)).collect()
}

/// Seeds `path` with `chain` and returns the work that was charged, which is
/// one unit for each id copied.
fn seeded(path: &[u64], chain: &[u64]) -> (Vec<StyleNodeId>, u64) {
    let doc = TestDoc::new();
    let caches = MatchCaches::default();
    let ctx = MatchContext::new(&doc, StyleQuirksMode::NoQuirks, None, false, &caches);
    let mut path = ids(path);
    seed_ancestor_path(ctx, &mut path, &ids(chain)).unwrap();
    (path, caches.work_count())
}

#[test]
fn seeding_copies_only_what_the_path_does_not_already_hold() {
    // Nothing held yet: the whole chain is copied.
    assert_eq!(seeded(&[], &[1, 2, 3]), (ids(&[1, 2, 3]), 3));
    // The last search went deeper below the same chain: nothing is copied.
    assert_eq!(seeded(&[1, 2, 3, 9, 10], &[1, 2, 3]), (ids(&[1, 2, 3]), 0));
    assert_eq!(seeded(&[1, 2, 3], &[1, 2, 3]), (ids(&[1, 2, 3]), 0));
    // The next anchor is one level deeper: one id is copied.
    assert_eq!(seeded(&[1, 2, 3], &[1, 2, 3, 4]), (ids(&[1, 2, 3, 4]), 1));
    assert_eq!(
        seeded(&[1, 2, 3, 4, 5, 6], &[1, 2, 3, 4]),
        (ids(&[1, 2, 3, 4]), 0)
    );
    // The chains part below a shared top: the rest is copied.
    assert_eq!(
        seeded(&[1, 2, 7, 8], &[1, 2, 3, 4]),
        (ids(&[1, 2, 3, 4]), 2)
    );
    assert_eq!(seeded(&[5, 6], &[1, 2]), (ids(&[1, 2]), 2));
    // A chain with no ancestors empties the path.
    assert_eq!(seeded(&[1, 2, 3], &[]), (ids(&[]), 0));
}

/// A chain of `n` nested `div.x` elements and the id of each with its ancestors.
fn chain_of_divs(n: usize) -> (TestDoc, Vec<(usize, Vec<StyleNodeId>)>) {
    let mut doc = TestDoc::new();
    let mut parent = 0;
    let mut elements: Vec<(usize, Vec<StyleNodeId>)> = Vec::new();
    for _ in 0..n {
        let ancestors = elements
            .iter()
            .map(|(id, _)| StyleNodeId::new(*id as u64))
            .collect();
        parent = doc.push_element(parent, "div", None);
        doc.set_attr(parent, "class", "x");
        elements.push((parent, ancestors));
    }
    (doc, elements)
}

fn subject_matches(
    doc: &TestDoc,
    caches: &MatchCaches,
    list: &SelectorList<RaikiriSelectorImpl>,
    id: usize,
    ancestors: &[StyleNodeId],
) -> bool {
    let id = StyleNodeId::new(id as u64);
    let node = doc.node(id).unwrap();
    let elem = node.as_element().unwrap();
    let ctx = MatchContext::new(doc, StyleQuirksMode::NoQuirks, None, false, caches);
    match_complex_selector_list(list, ctx, &elem, id, ancestors)
        .unwrap()
        .is_some()
}

#[test]
fn has_work_grows_linearly_along_a_deep_chain() {
    let list = crate::parse_selector_list(":has(> .x)").unwrap();
    let mut work = Vec::new();
    for n in [256, 512, 1024, 2048] {
        let (doc, elements) = chain_of_divs(n);
        let caches = MatchCaches::default();
        let matched = elements
            .iter()
            .filter(|(id, ancestors)| subject_matches(&doc, &caches, &list, *id, ancestors))
            .count();
        // Every element but the innermost has an `.x` child.
        assert_eq!(matched, n - 1, "chain of {n}");
        work.push(caches.work_count());
    }
    // Copying each anchor's whole chain would charge about n²/2 here, which
    // is over a million for 2048 elements.
    for (&n, &work) in [256, 512, 1024, 2048].iter().zip(&work) {
        assert!(work <= 16 * n as u64, "chain of {n}: {work} units of work");
    }
    // Twice the chain, about twice the work (four times when it is quadratic).
    for pair in work.windows(2) {
        assert!(
            pair[1] <= 2 * pair[0] + pair[0] / 8,
            "doubling the chain: {work:?}"
        );
    }
}

/// A tree of nested `div`s with classes `a`, `x`, `y` and `z` spread over it,
/// and every element with its ancestors in document order.
fn branching_tree() -> (TestDoc, Vec<(usize, Vec<StyleNodeId>)>) {
    fn grow(
        doc: &mut TestDoc,
        parent: usize,
        ancestors: &[StyleNodeId],
        depth: usize,
        elements: &mut Vec<(usize, Vec<StyleNodeId>)>,
    ) {
        for child in 0..3 {
            let id = doc.push_element(parent, "div", None);
            let class = match (depth + child) % 4 {
                0 => "a",
                1 => "x",
                2 => "y",
                _ => "z",
            };
            doc.set_attr(id, "class", class);
            elements.push((id, ancestors.to_vec()));
            if depth < 3 {
                let mut below = ancestors.to_vec();
                below.push(StyleNodeId::new(id as u64));
                grow(doc, id, &below, depth + 1, elements);
            }
        }
    }
    let mut doc = TestDoc::new();
    let mut elements = Vec::new();
    grow(&mut doc, 0, &[], 0, &mut elements);
    (doc, elements)
}

#[test]
fn has_matches_do_not_depend_on_the_order_the_anchors_are_visited_in() {
    let (doc, elements) = branching_tree();
    let mut shuffled: Vec<usize> = (0..elements.len()).collect();
    shuffled.sort_by_key(|&index| (index * 7919) % 101);
    let mut reversed: Vec<usize> = (0..elements.len()).collect();
    reversed.reverse();
    let document_order: Vec<usize> = (0..elements.len()).collect();
    for source in [
        ":has(> .x)",
        ":has(.y)",
        ":has(+ .x)",
        ":has(~ .z)",
        ":has(> .x .y)",
        ":has(+ .y > .z)",
        ":has(> .a, + .x, ~ .z)",
        ".a:has(> .x) > .y",
        ":is(.a, .z):has(.x) .y",
    ] {
        let list = crate::parse_selector_list(source).unwrap();
        for (name, order) in [
            ("document order", &document_order),
            ("reversed", &reversed),
            ("shuffled", &shuffled),
        ] {
            // A limit of zero records no result, so every evaluation searches,
            // each starting from what the one before it left in the scratch.
            let kept = MatchCaches::with_has_limit(0);
            for &index in order {
                let (id, ancestors) = &elements[index];
                assert_eq!(
                    subject_matches(&doc, &kept, &list, *id, ancestors),
                    subject_matches(&doc, &MatchCaches::default(), &list, *id, ancestors),
                    "{source} on element {index}, {name}"
                );
            }
        }
    }
}
