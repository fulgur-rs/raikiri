use super::*;

fn many_names(count: usize, reversed: bool) -> (Document, CascadeResult, usize) {
    let reset = (0..count)
        .map(|i| {
            if reversed {
                format!("reversed(c{i})")
            } else {
                format!("c{i} 1")
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let increments = (0..count)
        .map(|i| format!("c{i} 1"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let div = doc.append_element(Some(body), "div", Some(&format!("counter-reset:{reset}")));
    let leaf = doc.append_element(
        Some(div),
        "p",
        Some(&format!("counter-increment:{increments}")),
    );
    doc.append_text(leaf, "X");
    doc.mark_in_document_flags();
    assert_eq!(doc.node_count(), 7);
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    assert_eq!(cascade.computed[leaf].counter_increment.len(), count);
    (doc, cascade, leaf)
}

fn name_work(count: usize, reversed: bool) -> [usize; 3] {
    let (doc, cascade, leaf) = many_names(count, reversed);
    COUNTER_NAME_WORK.with(|work| work.set([0; 3]));
    let snapshots = counter_snapshots(&doc, &cascade).unwrap();
    let work = COUNTER_NAME_WORK.with(std::cell::Cell::get);
    for i in 0..count {
        assert_eq!(
            snapshots[leaf][&Symbol::new(format!("c{i}"))].last(),
            Some(&if reversed { -1 } else { 2 })
        );
    }
    eprintln!("counter-work n={count} reversed={reversed} work={work:?}");
    work
}

#[test]
fn scope_bucket_name_work_grows_linearly() {
    for n in [32, 64, 128] {
        let work = name_work(n, false);
        assert!(work[0] <= 8 * n, "n={n} work={work:?}");
    }
}

#[test]
fn reversed_directive_name_work_grows_linearly() {
    for n in [32, 64, 128] {
        let work = name_work(n, true);
        assert!(work[1] + work[2] <= 8 * n, "n={n} work={work:?}");
    }
}

#[test]
fn reversed_aggregation_matches_ordered_directives_for_duplicates_and_saturation() {
    use std::sync::Arc;
    let (doc, cascade, leaf) = many_names(3, true);
    let mut reset = cascade.computed[doc.parent_of(leaf).unwrap()].clone();
    let names = ["c0", "c1", "list-item"];
    reset.counter_reset = Arc::new(
        names
            .iter()
            .map(|name| (SmolStr::new(name), CounterResetValue::Reversed(None)))
            .collect(),
    );
    for deltas in [
        vec![],
        vec![0],
        vec![3, -3],
        vec![i32::MAX, 1, -1],
        vec![i32::MIN, -1, 1],
    ] {
        for sets in [vec![], vec![7], vec![3, -5], vec![i32::MIN, i32::MAX]] {
            for implicit in [None, Some((SmolStr::new_static("list-item"), -1))] {
                let mut scopes = CounterScopes::default();
                scopes.apply(&reset, false, None);
                let mut cv = cascade.computed[leaf].clone();
                let mut increments = Vec::new();
                for &delta in &deltas {
                    increments.extend([
                        (SmolStr::new_static("c0"), delta),
                        (SmolStr::new_static("c1"), delta.saturating_neg()),
                        (SmolStr::new_static("unscoped"), 99),
                    ]);
                }
                cv.counter_increment = Arc::new(increments);
                cv.counter_set = Arc::new(
                    sets.iter()
                        .flat_map(|&value| {
                            [
                                (SmolStr::new_static("c0"), value),
                                (SmolStr::new_static("list-item"), value),
                            ]
                        })
                        .collect(),
                );
                let mut expected = vec![ReversedStartTally::default(); 3];
                for _ in 0..2 {
                    for (index, name) in names.iter().enumerate() {
                        let (increment, set) = counter_directives(&cv, implicit.as_ref(), name);
                        expected[index].record(increment, set);
                    }
                    scopes.record_reversed_directives(&cv, implicit.as_ref());
                    let ReversedStarts::Collect(actual) = &scopes.reversed else {
                        panic!("collect")
                    };
                    let states = |tallies: &[ReversedStartTally]| {
                        tallies
                            .iter()
                            .map(|tally| {
                                (tally.num, tally.last_non_zero_increment_negated, tally.done)
                            })
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(
                        states(actual),
                        states(&expected),
                        "deltas={deltas:?} sets={sets:?} implicit={implicit:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn non_calculated_top_levels_hide_outer_calculated_tallies() {
    use std::sync::Arc;
    let (doc, cascade, leaf) = many_names(2, true);
    let reset = &cascade.computed[doc.parent_of(leaf).unwrap()];
    for shadow in [
        CounterResetValue::Integer(5),
        CounterResetValue::Reversed(Some(9)),
    ] {
        let mut scopes = CounterScopes::default();
        scopes.apply(reset, false, None);
        let mut cv = cascade.computed[leaf].clone();
        cv.counter_reset = Arc::new(vec![(SmolStr::new_static("c0"), shadow)]);
        cv.counter_increment = Arc::new(vec![
            (SmolStr::new_static("c0"), 1),
            (SmolStr::new_static("c1"), 1),
        ]);
        scopes.apply(&cv, false, None);
        let ReversedStarts::Collect(tallies) = &scopes.reversed else {
            panic!("collect")
        };
        assert_eq!(tallies[0].starting_value(), 0);
        assert_eq!(tallies[1].starting_value(), -2);
        scopes.pop_one(&Symbol::new("c0"));
        cv.counter_reset = Arc::new(Vec::new());
        cv.counter_increment = Arc::new(vec![(SmolStr::new_static("c0"), 1)]);
        scopes.apply(&cv, false, None);
        let ReversedStarts::Collect(tallies) = &scopes.reversed else {
            panic!("collect")
        };
        assert_eq!(tallies[0].starting_value(), -2);
        assert_eq!(tallies[1].starting_value(), -2);
    }
}

#[test]
fn scope_bucket_promotes_once_even_for_a_missing_reset_name() {
    eprintln!(
        "scope-bucket-inline bytes={}",
        std::mem::size_of::<CounterScopeBucket>()
    );
    let mut bucket = CounterScopeBucket::default();
    assert!(!bucket.remove(&Symbol::new("missing")));
    assert!(matches!(&bucket, CounterScopeBucket::Unindexed(names) if names.capacity() == 0));
    bucket.register(Symbol::new("a"));
    bucket.register(Symbol::new("b"));
    assert!(matches!(&bucket, CounterScopeBucket::Unindexed(names) if names.len() == 2));
    assert!(!bucket.remove(&Symbol::new("missing")));
    assert!(matches!(&bucket, CounterScopeBucket::Indexed(names) if names.len() == 2));
    assert!(bucket.remove(&Symbol::new("a")));
    bucket.register(Symbol::new("a"));
    assert!(bucket.remove(&Symbol::new("a")));
    assert!(bucket.remove(&Symbol::new("b")));
    assert!(matches!(&bucket, CounterScopeBucket::Indexed(names) if names.is_empty()));
    bucket.register(Symbol::new("c"));
    assert!(bucket.remove(&Symbol::new("c")));
}

#[test]
fn resets_after_auto_instantiation_promote_and_restore_outer_scopes() {
    for n in [32, 64, 128] {
        for overlap in [false, true] {
            let increments = (0..n)
                .map(|i| format!("c{i} 1"))
                .collect::<Vec<_>>()
                .join(" ");
            let resets = (0..n)
                .map(|i| format!("{}{i} 10", if overlap { "c" } else { "d" }))
                .collect::<Vec<_>>()
                .join(" ");
            let mut doc = Document::new();
            let outer = doc.append_element(Some(0), "section", Some("counter-reset:outer 100"));
            let group = doc.append_element(Some(outer), "section", None::<&str>);
            let first = doc.append_element(
                Some(group),
                "div",
                Some(&format!("counter-increment:{increments}")),
            );
            let reset = doc.append_element(
                Some(group),
                "div",
                Some(&format!("counter-reset:{resets} outer 5")),
            );
            let last = doc.append_element(
                Some(group),
                "div",
                Some(&format!("counter-increment:{increments}")),
            );
            let after = doc.append_element(
                Some(outer),
                "p",
                Some(&format!("counter-increment:{increments} outer 1")),
            );
            doc.mark_in_document_flags();
            let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
            COUNTER_NAME_WORK.with(|work| work.set([0; 3]));
            let snapshots = counter_snapshots(&doc, &cascade).unwrap();
            let work = COUNTER_NAME_WORK.with(std::cell::Cell::get);
            assert!(work[0] <= 4 * n, "n={n} overlap={overlap} work={work:?}");
            assert_eq!(snapshots[reset][&Symbol::new("outer")], vec![100, 5]);
            assert_eq!(snapshots[after][&Symbol::new("outer")], vec![101]);
            for i in 0..n {
                let c = Symbol::new(format!("c{i}"));
                let name = Symbol::new(format!("{}{i}", if overlap { "c" } else { "d" }));
                assert_eq!(snapshots[first][&c], vec![1]);
                assert_eq!(snapshots[reset][&name], vec![10]);
                assert_eq!(snapshots[last][&c], vec![if overlap { 11 } else { 2 }]);
                assert_eq!(snapshots[after][&c], vec![1]);
            }
        }
    }
}

#[test]
fn unrelated_set_does_not_feed_a_calculated_reversed_counter() {
    let mut doc = Document::new();
    let scope = doc.append_element(Some(0), "section", Some("counter-reset:reversed(r)"));
    let first = doc.append_element(
        Some(scope),
        "p",
        Some("counter-increment:r 1;counter-set:unrelated 7"),
    );
    let second = doc.append_element(
        Some(scope),
        "p",
        Some("counter-increment:r 2;counter-set:unrelated 9"),
    );
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let snapshots = counter_snapshots(&doc, &cascade).unwrap();
    let r = Symbol::new("r");
    let unrelated = Symbol::new("unrelated");
    assert_eq!(snapshots[scope][&r], vec![-5]);
    assert_eq!(snapshots[first][&r], vec![-4]);
    assert_eq!(snapshots[second][&r], vec![-2]);
    assert_eq!(snapshots[first][&unrelated], vec![7]);
    assert_eq!(snapshots[second][&unrelated], vec![9]);
}
