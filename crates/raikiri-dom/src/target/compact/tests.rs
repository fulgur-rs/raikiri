use super::*;
use crate::generated_content::{CounterSnapshotLookup, generated_text, generated_text_compact};
use raikiri_style::{build_rule_tree, cascade};

fn compare(document: &Document, cascade: &CascadeResult) {
    let owned = counter_snapshots(document, cascade).expect("owned snapshots");
    let compact = compact_counter_snapshots(document, cascade).expect("compact snapshots");
    let names: Vec<_> = compact.names.keys().collect();
    for node in 0..document.node_count() {
        let view = compact.get(node).expect("node snapshot");
        for name in &names {
            assert_eq!(
                view.values_for(name.as_str()).iter().collect::<Vec<_>>(),
                owned[node].get(*name).cloned().unwrap_or_default(),
                "node {node}, name {}",
                name.as_str()
            );
        }
        for pseudo in [PseudoElem::Before, PseudoElem::After, PseudoElem::Marker] {
            assert_eq!(
                compact.quote_depth(node, pseudo),
                owned.quote_depth(node, pseudo)
            );
            assert_eq!(
                compact.pseudo(node, pseudo).is_some(),
                owned.pseudo(node, pseudo).is_some()
            );
            if let (Some(compact), Some(owned)) =
                (compact.pseudo(node, pseudo), owned.pseudo(node, pseudo))
            {
                for name in &names {
                    assert_eq!(
                        compact.values_for(name.as_str()).iter().collect::<Vec<_>>(),
                        owned.get(*name).cloned().unwrap_or_default()
                    );
                }
            }
            assert_eq!(
                generated_text_compact(document, cascade, node, pseudo, &compact)
                    .map(|(_, text)| text),
                generated_text(document, cascade, node, pseudo, &owned).map(|(_, text)| text)
            );
        }
    }
}

#[test]
fn compact_matches_owned_scopes_pseudos_quotes_and_reversed_counters() {
    let mut document = Document::new();
    let style = document.append_element(Some(0), "style", None::<&str>);
    document.append_text(style, "div::before { content: open-quote counters(c, '.') ':' counter(r); counter-increment: c 2 } div::after { content: counter(c) close-quote; counter-set: c 9 } li::marker { content: counter(list-item) '.' }");
    let root = document.append_element(
        Some(0),
        "section",
        Some("counter-reset: c 1 reversed(r) reversed(list-item)"),
    );
    let first = document.append_element(
        Some(root),
        "div",
        Some("counter-increment: c r; counter-set: c 3"),
    );
    let nested = document.append_element(
        Some(first),
        "div",
        Some("counter-reset: c 4 other 7; counter-increment: c r"),
    );
    document.append_element(Some(nested), "li", Some("counter-increment: r"));
    document.append_element(
        Some(first),
        "span",
        Some("counter-set: other 8; counter-increment: c"),
    );
    document.append_element(
        Some(root),
        "div",
        Some("counter-reset: c 10; counter-increment: r"),
    );
    document.append_element(
        Some(root),
        "div",
        Some("counter-reset: c 20; counter-set: c 2147483647; counter-increment: c"),
    );
    let hidden = document.append_element(
        Some(root),
        "div",
        Some("display:none; counter-reset: hidden 20"),
    );
    document.append_element(Some(hidden), "div", Some("counter-reset: omitted"));
    document.append_element(
        Some(root),
        "div",
        Some("display:contents; counter-reset: skipped"),
    );
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let computed = cascade(&document, &rules).expect("cascade");
    compare(&document, &computed);
}

#[test]
fn child_exit_preserves_changes_to_ancestor_and_old_snapshots() {
    let mut document = Document::new();
    let root = document.append_element(Some(0), "section", Some("counter-reset: c 1"));
    let first = document.append_element(Some(root), "div", None::<&str>);
    let nested = document.append_element(
        Some(first),
        "span",
        Some("counter-increment: c 4; counter-reset: temporary"),
    );
    let next = document.append_element(Some(root), "div", Some("counter-increment: c 2"));
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let computed = cascade(&document, &rules).expect("cascade");
    let snapshots = compact_counter_snapshots(&document, &computed).expect("compact snapshots");
    assert_eq!(
        snapshots.get(first).unwrap().values_for("c").last(),
        Some(1)
    );
    assert_eq!(
        snapshots.get(nested).unwrap().values_for("c").last(),
        Some(5)
    );
    assert_eq!(snapshots.get(next).unwrap().values_for("c").last(), Some(7));
    assert!(
        snapshots
            .get(next)
            .unwrap()
            .values_for("temporary")
            .is_empty()
    );
    compare(&document, &computed);
}

#[test]
fn compact_cache_tracks_generations_and_mutations() {
    let mut document = Document::new();
    let node = document.append_element(Some(0), "div", Some("counter-reset: c 1"));
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let computed = cascade(&document, &rules).expect("cascade");
    let first = document
        .shared_compact_counter_snapshots(&computed)
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &first,
        &document
            .shared_compact_counter_snapshots(&computed)
            .unwrap()
    ));
    let other = cascade(&document, &rules).expect("other cascade");
    assert!(!std::sync::Arc::ptr_eq(
        &first,
        &document.shared_compact_counter_snapshots(&other).unwrap()
    ));
    assert!(std::sync::Arc::ptr_eq(
        &first,
        &document
            .shared_compact_counter_snapshots(&computed)
            .unwrap()
    ));
    crate::layout_single_page(&mut document, &other, raikiri_traits::PageBox::A4).ok();
    let relaid = document.shared_compact_counter_snapshots(&other).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &relaid,
        &document.shared_compact_counter_snapshots(&other).unwrap()
    ));
    assert!(!std::sync::Arc::ptr_eq(&first, &relaid));
    document.detach_from_parent(node);
    document.mark_in_document_flags();
    let detached = document
        .shared_compact_counter_snapshots(&computed)
        .unwrap();
    assert!(detached.get(node).unwrap().values_for("c").is_empty());
    let appended = document.append_element(Some(0), "span", None::<&str>);
    let rebuilt = document
        .shared_compact_counter_snapshots(&computed)
        .unwrap();
    assert!(!std::sync::Arc::ptr_eq(&detached, &rebuilt));
    assert!(rebuilt.get(appended).is_some());
}

#[test]
fn retained_capacity_is_charged_once_and_limits_precede_growth() {
    let mut builder = CompactBuilder::new(20, &CounterSnapshotBudget::default()).unwrap();
    let name = Symbol::new("long-counter-name-that-uses-heap-storage");
    builder.change(&name, Some(1), true);
    let after_change = builder.budget.estimated_bytes;
    for node in 0..20 {
        builder.snapshot(CounterTarget::Element(node)).unwrap();
    }
    assert_eq!(after_change, builder.budget.estimated_bytes);
    for _ in 0..20 {
        builder.change(&name, Some(2), false);
    }
    assert!(builder.budget.estimated_bytes > after_change);
    let bytes = builder.budget.estimated_bytes;
    let mut limited = CompactBuilder::new(20, &CounterSnapshotBudget::new(bytes - 1)).unwrap();
    limited.change(&name, Some(1), true);
    for _ in 0..20 {
        limited.change(&name, Some(2), false);
    }
    assert!(limited.snapshot(CounterTarget::Element(0)).is_err());
    let mut budget = CounterSnapshotBudget::new(u64::MAX);
    budget.estimated_bytes = u64::MAX;
    assert!(CompactBuilder::new(1, &budget).is_err());
}

#[test]
fn deeply_nested_prefix_arena_drops_without_recursive_ownership() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let mut builder = CompactBuilder::new(1, &CounterSnapshotBudget::default()).unwrap();
            let name = Symbol::new("nested");
            for value in 0..100_000 {
                builder.change(&name, Some(value), true);
            }
            builder.snapshot(CounterTarget::Element(0)).unwrap();
            assert_eq!(
                builder
                    .snapshots
                    .get(0)
                    .unwrap()
                    .values_for("nested")
                    .last(),
                Some(99_999)
            );
            drop(builder);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn persistent_name_map_keeps_all_dense_ids_and_past_roots() {
    let mut builder = CompactBuilder::new(3, &CounterSnapshotBudget::default()).unwrap();
    for index in 0..2048 {
        builder.change(&Symbol::new(format!("name{index}")), Some(index), true);
    }
    builder.snapshot(CounterTarget::Element(0)).unwrap();
    for index in (0..2048).rev() {
        let name = Symbol::new(format!("name{index}"));
        builder.change(&name, Some(-index), false);
        if index % 2 == 0 {
            builder.change(&name, Some(99), true);
        }
    }
    builder.snapshot(CounterTarget::Element(1)).unwrap();
    for index in (0..2048).step_by(2) {
        builder.change(&Symbol::new(format!("name{index}")), None, false);
    }
    builder.snapshot(CounterTarget::Element(2)).unwrap();
    for index in 0..2048 {
        let name = format!("name{index}");
        assert_eq!(
            builder
                .snapshots
                .get(0)
                .unwrap()
                .values_for(&name)
                .iter()
                .collect::<Vec<_>>(),
            [index]
        );
        let expected = if index % 2 == 0 {
            vec![-index, 99]
        } else {
            vec![-index]
        };
        assert_eq!(
            builder
                .snapshots
                .get(1)
                .unwrap()
                .values_for(&name)
                .iter()
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            builder
                .snapshots
                .get(2)
                .unwrap()
                .values_for(&name)
                .iter()
                .collect::<Vec<_>>(),
            [-index]
        );
    }
}

#[test]
fn compact_cache_preserves_a_cached_failure_until_mutation() {
    let mut document = Document::new();
    let rules = build_rule_tree(&document);
    let computed = cascade(&document, &rules).unwrap();
    let error = CounterSnapshotLimitExceeded {
        limit: 1,
        actual: 2,
    };
    document
        .compact_counter_snapshot_cache
        .set((computed.element_generation(), Err(error)))
        .unwrap();
    assert_eq!(
        document
            .shared_compact_counter_snapshots(&computed)
            .unwrap_err(),
        error
    );
    assert_eq!(
        document
            .shared_compact_counter_snapshots(&computed)
            .unwrap_err(),
        error
    );
    document.append_element(Some(0), "div", None::<&str>);
    assert!(document.shared_compact_counter_snapshots(&computed).is_ok());
}
