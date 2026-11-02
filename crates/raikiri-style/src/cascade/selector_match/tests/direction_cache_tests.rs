use super::*;

#[test]
fn first_direction_chain_pass_is_linear_and_repeated_subjects_keep_their_memo() {
    for depth in [100, 500, 2_000, 8_000] {
        let mut doc = TestDoc::new();
        let html = doc.push_element_with_attrs(0, "html", None, &[("dir", "ltr"), ("lang", "en")]);
        let mut ids = vec![html];
        let mut parent = html;
        for _ in 0..depth {
            parent = doc.push_element(parent, "div", None);
            ids.push(parent);
        }
        for source in [":lang(en)", ":dir(ltr)"] {
            let query = crate::SelectorQuery::parse(source).unwrap();
            let budget = crate::SelectorWorkBudget::new(Some(1 << 26));
            let matcher = query.bounded_matcher(&doc, None, &budget);
            let mut ancestors = Vec::new();
            for &id in &ids {
                assert!(matcher.try_matches(node_id(id), &ancestors).unwrap());
                ancestors.push(node_id(id));
            }
            let first = budget.count();
            assert_eq!(first, 3 * depth as u64 + 2, "depth={depth}, {source}");
            ancestors.clear();
            for &id in &ids {
                assert!(matcher.try_matches(node_id(id), &ancestors).unwrap());
                ancestors.push(node_id(id));
            }
            let repeated = budget.count() - first;
            assert_eq!(repeated, 2 * ids.len() as u64);
            eprintln!(
                "direction work: depth={depth}, {source}, first={first}, repeated={repeated}"
            );
        }
    }
}

#[test]
fn leaf_first_queries_compress_only_the_explicit_inheritance_path() {
    for depth in [100, 500, 2_000, 8_000] {
        let mut doc = TestDoc::new();
        let root = doc.push_element_with_attrs(0, "html", None, &[("dir", "RTL")]);
        let mut ids = vec![node_id(root)];
        let mut parent = root;
        for _ in 0..depth {
            parent = doc.push_element(parent, "div", None);
            ids.push(node_id(parent));
        }
        let query = crate::SelectorQuery::parse(":dir(rtl)").unwrap();
        let budget = crate::SelectorWorkBudget::new(Some(1 << 26));
        let matcher = query.bounded_matcher(&doc, None, &budget);
        for index in (0..ids.len()).rev() {
            assert!(matcher.try_matches(ids[index], &ids[..index]).unwrap());
        }
        assert!(
            budget.count() <= 4 * ids.len() as u64,
            "depth={depth}, work={}",
            budget.count()
        );
    }
}

#[test]
fn auto_own_results_and_inheritance_memos_stay_separate_in_both_query_orders() {
    let mut doc = TestDoc::new();
    let root = doc.push_element_with_attrs(0, "html", None, &[("dir", "RTL")]);
    let auto = doc.push_element_with_attrs(root, "div", None, &[("dir", "aUtO")]);
    doc.push_text(auto, "abc");
    let child = doc.push_element_with_attrs(auto, "span", None, &[("dir", "invalid")]);
    let empty_auto = doc.push_element_with_attrs(root, "bdi", None, &[("dir", "auto")]);
    let empty_child = doc.push_element(empty_auto, "span", None);
    let foreign = doc.push_element_with_attrs(root, "svg", None, &[("dir", "ltr")]);
    doc.set_namespace(foreign, "http://www.w3.org/2000/svg");
    let explicit = doc.push_element_with_attrs(child, "em", None, &[("dir", "lTr")]);
    let explicit_child = doc.push_element(explicit, "i", None);
    let default_root = doc.push_element(0, "div", None);
    let default_auto = doc.push_element_with_attrs(default_root, "div", None, &[("dir", "auto")]);
    doc.push_text(default_auto, "א");
    let default_child = doc.push_element(default_auto, "span", None);
    let cases = [
        (default_root, vec![], Direction::Ltr),
        (default_auto, vec![default_root], Direction::Rtl),
        (
            default_child,
            vec![default_root, default_auto],
            Direction::Ltr,
        ),
        (root, vec![], Direction::Rtl),
        (auto, vec![root], Direction::Ltr),
        (child, vec![root, auto], Direction::Rtl),
        (empty_auto, vec![root], Direction::Ltr),
        (empty_child, vec![root, empty_auto], Direction::Rtl),
        (foreign, vec![root], Direction::Rtl),
        (explicit, vec![root, auto, child], Direction::Ltr),
        (
            explicit_child,
            vec![root, auto, child, explicit],
            Direction::Ltr,
        ),
    ];
    for reverse in [false, true] {
        let caches = MatchCaches::default();
        let order: Vec<usize> = if reverse {
            (0..cases.len()).rev().collect()
        } else {
            (0..cases.len()).collect()
        };
        for index in order {
            let (id, ancestors, expected) = &cases[index];
            let ancestors: Vec<_> = ancestors.iter().map(|&id| node_id(id)).collect();
            let node = doc.node(node_id(*id)).unwrap();
            let elem = node.as_element().unwrap();
            assert_eq!(
                caches
                    .directionality(&doc, &elem, node_id(*id), &ancestors)
                    .unwrap(),
                *expected,
                "id={id}, reverse={reverse}"
            );
        }
        assert_eq!(
            caches.directions.borrow().get(&node_id(auto)),
            Some(&Direction::Ltr)
        );
        assert_eq!(
            caches.inherited_directions.borrow().get(&node_id(auto)),
            Some(&Direction::Rtl)
        );
    }
}

#[test]
fn failed_direction_walks_publish_neither_own_nor_inherited_partial_results() {
    let mut doc = TestDoc::new();
    let outer = doc.push_element_with_attrs(0, "html", None, &[("dir", "ltr")]);
    let root = doc.push_element_with_attrs(outer, "div", None, &[("dir", "rtl")]);
    let middle = doc.push_element(root, "div", None);
    let child = doc.push_element(middle, "span", None);
    for limit in [0, 1, 2] {
        let caches = MatchCaches::with_work_limit(Some(limit));
        for (id, ancestors) in [(outer, vec![]), (root, vec![node_id(outer)])] {
            let node = doc.node(node_id(id)).unwrap();
            let elem = node.as_element().unwrap();
            caches
                .directionality(&doc, &elem, node_id(id), &ancestors)
                .unwrap();
        }
        let before_own = caches.directions.borrow().clone();
        let before_inherited = caches.inherited_directions.borrow().clone();
        let node = doc.node(node_id(child)).unwrap();
        let elem = node.as_element().unwrap();
        let result = caches.directionality(
            &doc,
            &elem,
            node_id(child),
            &[node_id(outer), node_id(root), node_id(middle)],
        );
        if limit < 2 {
            assert!(
                matches!(result, Err(CascadeError::LimitExceeded { kind: CascadeLimitKind::SelectorWork, limit: actual, .. }) if actual == limit)
            );
            assert_eq!(*caches.directions.borrow(), before_own);
            assert_eq!(*caches.inherited_directions.borrow(), before_inherited);
        } else {
            assert_eq!(result.unwrap(), Direction::Rtl);
            assert!(!caches.directions.borrow().contains_key(&node_id(middle)));
            assert_eq!(
                caches.inherited_directions.borrow().get(&node_id(middle)),
                Some(&Direction::Rtl)
            );
            assert_eq!(
                caches.inherited_directions.borrow().get(&node_id(outer)),
                Some(&Direction::Ltr)
            );
        }
        assert_eq!(caches.work_count(), limit);
    }
}

#[test]
fn public_direction_budget_charges_cached_ancestors_and_stays_terminal() {
    let mut doc = TestDoc::new();
    let root = doc.push_element_with_attrs(0, "html", None, &[("dir", "rtl")]);
    let child = doc.push_element(root, "span", None);
    let query = crate::SelectorQuery::parse(":dir(rtl)").unwrap();
    let short = crate::SelectorWorkBudget::new(Some(4));
    let matcher = query.bounded_matcher(&doc, None, &short);
    assert!(matcher.try_matches(node_id(root), &[]).unwrap());
    for _ in 0..2 {
        assert!(matches!(
            matcher.try_matches(node_id(child), &[node_id(root)]),
            Err(CascadeError::LimitExceeded {
                kind: CascadeLimitKind::SelectorWork,
                limit: 4,
                actual: 5
            })
        ));
    }
    assert!(matches!(
        matcher.try_matches(node_id(root), &[]),
        Err(CascadeError::LimitExceeded {
            limit: 4,
            actual: 5,
            ..
        })
    ));
    assert!(short.charge(0).is_err());
    assert_eq!(short.count(), 4);
    let exact = crate::SelectorWorkBudget::new(Some(5));
    let matcher = query.bounded_matcher(&doc, None, &exact);
    assert!(matcher.try_matches(node_id(root), &[]).unwrap());
    assert!(
        matcher
            .try_matches(node_id(child), &[node_id(root)])
            .unwrap()
    );
    assert_eq!(exact.count(), 5);
}

#[test]
fn scoped_and_detached_queries_keep_real_ancestor_inheritance_and_fresh_memos() {
    let mut doc = TestDoc::new();
    let root = doc.push_element_with_attrs(0, "html", None, &[("dir", "rtl")]);
    let scope = doc.push_element_with_attrs(root, "section", None, &[("dir", "auto")]);
    doc.push_text(scope, "abc");
    let child = doc.push_element(scope, "span", None);
    let detached = doc.push_element(0, "div", None);
    doc.set_in_document(detached, false);
    let query = crate::SelectorQuery::parse(":dir(rtl)").unwrap();
    {
        let matcher = query.matcher(&doc, Some(node_id(scope)));
        assert!(!matcher.matches(node_id(scope), &[node_id(root)]));
        assert!(matcher.matches(node_id(child), &[node_id(root), node_id(scope)]));
    }
    assert!(
        crate::SelectorQuery::parse(":scope > :dir(rtl)")
            .unwrap()
            .matches_scoped(
                &doc,
                node_id(child),
                &[node_id(root), node_id(scope)],
                Some(node_id(scope))
            )
    );
    assert!(crate::SelectorQuery::parse(":dir(ltr)").unwrap().matches(
        &doc,
        node_id(detached),
        &[]
    ));
    doc.nodes[root].attrs = vec![("dir".to_owned(), "ltr".to_owned())];
    assert!(!query.matches(&doc, node_id(child), &[node_id(root), node_id(scope)]));
}
