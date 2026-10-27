use super::*;
use crate::cascade::inherit::{WalkOptions, walk};

fn indexed_workload(elements: usize, selectors: usize, grouped: bool) -> (TestDoc, RuleTree) {
    let mut doc = TestDoc::new();
    for n in 0..elements {
        doc.push_element_with_attrs(0, "p", None, &[("class", &format!("c{}", n % selectors))]);
    }
    let css = if grouped {
        format!(
            "{} {{ color: red }}",
            (0..selectors)
                .map(|s| format!(".c{s}"))
                .collect::<Vec<_>>()
                .join(",")
        )
    } else {
        (0..selectors)
            .map(|s| format!(".c{s} {{ color: red }}"))
            .collect::<String>()
    };
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&css, Origin::Author);
    (doc, tree)
}

#[test]
fn indexed_selector_positions_bound_actual_cascade_matching_work() {
    for (n, s) in [(8, 32), (32, 32), (8, 128), (32, 128)] {
        let (doc, tree) = indexed_workload(n, s, true);
        let grouped = walk(
            &doc,
            &tree,
            &MediaContext::default(),
            WalkOptions::default(),
        )
        .unwrap();
        let (_, single_tree) = indexed_workload(n, s, false);
        let separate = walk(
            &doc,
            &single_tree,
            &MediaContext::default(),
            WalkOptions::default(),
        )
        .unwrap();
        assert_eq!(grouped.computed, separate.computed);
        for cv in &grouped.computed[1..] {
            assert_eq!(cv.color, RED);
        }
        assert_eq!(grouped.counts.selector_tests, (n * s) as u64);
        assert_eq!(separate.counts.selector_tests, n as u64);
        assert_eq!(grouped.counts.selector_work, separate.counts.selector_work);
    }
}

#[test]
#[ignore = "explicit release selector-index measurement"]
fn selector_index_scaling_probe() {
    let env_usize = |name: &str, default| {
        std::env::var(name)
            .ok()
            .map(|v| v.parse::<usize>().unwrap())
            .unwrap_or(default)
    };
    let n = env_usize("SELECTOR_INDEX_N", 64);
    let s = env_usize("SELECTOR_INDEX_S", 64);
    let loops = env_usize("SELECTOR_INDEX_LOOPS", 5);
    let grouped = std::env::var("SELECTOR_INDEX_SHAPE").as_deref() != Ok("separate");
    let (doc, tree) = indexed_workload(n, s, grouped);
    let mut last = None;
    let started = std::time::Instant::now();
    for _ in 0..loops {
        let output = walk(
            std::hint::black_box(&doc),
            std::hint::black_box(&tree),
            &MediaContext::default(),
            WalkOptions::default(),
        )
        .unwrap();
        assert!(output.computed[1..].iter().all(|cv| cv.color == RED));
        last = Some(std::hint::black_box(output));
    }
    let elapsed = started.elapsed();
    let output = last.unwrap();
    println!(
        "SELECTOR_INDEX n={n} s={s} grouped={grouped} loops={loops} ns={} work={} tests={} nodes={}",
        elapsed.as_nanos(),
        output.counts.selector_work,
        output.counts.selector_tests,
        output.computed.len()
    );
}

#[test]
fn selector_position_filter_keeps_legacy_outer_budget_boundary() {
    let (doc, tree) = indexed_workload(8, 32, true);
    for (limit, succeeds) in [(255, false), (256, true)] {
        let mut options = WalkOptions::default();
        options.limits.max_selector_tests = Some(limit);
        let result = walk(&doc, &tree, &MediaContext::default(), options);
        assert_eq!(result.is_ok(), succeeds);
        if let Err(error) = result {
            assert!(matches!(
                error,
                crate::CascadeError::LimitExceeded {
                    kind: crate::CascadeLimitKind::SelectorTests,
                    ..
                }
            ));
        }
    }
}
