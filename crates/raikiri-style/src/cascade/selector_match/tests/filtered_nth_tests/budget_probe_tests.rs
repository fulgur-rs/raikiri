use super::*;

#[test]
#[ignore = "manual finite/unlimited work budget timing and peak RSS probe"]
fn selector_budget_work_probe() {
    let (doc, groups, source, queries) = work_probe_tests::inputs();
    let list = crate::parse_selector_list(&source).unwrap();
    let limit = std::env::var("SELECTOR_PROBE_LIMIT")
        .ok()
        .map(|s| s.parse().unwrap());
    let caches = MatchCaches::with_work_limit(limit);
    let counted = ChildCountingDoc {
        doc: &doc,
        child_reads: Cell::new(0),
    };
    let start = Instant::now();
    let mut completed = 0;
    let mut stopped = false;
    'parents: for (parent, children) in &groups {
        for q in 0..queries {
            let index = q * children.len() / queries;
            match limited_match(&counted, &list, &caches, children[index], &[*parent]) {
                Ok(hit) => {
                    assert_eq!(hit.is_some(), !source.contains("missing") && index % 3 == 0);
                    completed += 1;
                }
                Err(CascadeError::LimitExceeded {
                    kind: CascadeLimitKind::SelectorWork,
                    ..
                }) => {
                    stopped = true;
                    break 'parents;
                }
                Err(other) => panic!("unexpected error: {other:?}"),
            }
        }
    }
    eprintln!(
        "SELECTOR_BUDGET n={} parents={} keys={} queries={queries} limit={limit:?} elapsed_ns={} child_reads={} class_reads={} work={} completed={completed} stopped={stopped} cached_keys={} cached_slots={}",
        groups[0].1.len(),
        groups.len(),
        source.matches(":nth").count(),
        start.elapsed().as_nanos(),
        counted.child_reads.get(),
        doc.class_reads.get(),
        caches.work_count(),
        caches.filtered_nth_entries(),
        caches.filtered_nth_slots()
    );
}
