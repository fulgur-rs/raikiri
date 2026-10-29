use super::*;

pub(super) fn inputs() -> (
    CountingDoc,
    Vec<(StyleNodeId, Vec<StyleNodeId>)>,
    String,
    usize,
) {
    let number = |key: &str, default| {
        std::env::var(key)
            .ok()
            .map(|s| s.parse().unwrap())
            .unwrap_or(default)
    };
    let n = number("SELECTOR_PROBE_N", 65_537);
    let parents = number("SELECTOR_PROBE_PARENTS", 1);
    let keys = number("SELECTOR_PROBE_KEYS", 1);
    let queries = number("SELECTOR_PROBE_QUERIES", 64);
    let (mut doc, parent, children) = fixture(n);
    let mut groups = vec![(parent, children)];
    for _ in 1..parents {
        let parent = doc.doc.push_element(0, "section", None);
        let children = (0..n)
            .map(|i| {
                let child = doc.doc.push_element(parent, "li", None);
                doc.doc
                    .set_attr(child, "class", if i % 3 == 0 { "visible" } else { "other" });
                StyleNodeId::new(child as u64)
            })
            .collect();
        groups.push((StyleNodeId::new(parent as u64), children));
    }
    let source = (0..keys)
        .map(|key| {
            if keys == 1 {
                ":nth-child(n of .visible)".to_owned()
            } else {
                format!(":nth-child(n of .missing{key})")
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    (doc, groups, source, queries)
}

#[test]
#[ignore = "manual cache-threshold timing, operation-count and peak RSS probe"]
fn selector_query_work_probe() {
    let (doc, groups, source, queries) = inputs();
    let query = crate::SelectorQuery::parse(&source).unwrap();
    let counted = ChildCountingDoc {
        doc: &doc,
        child_reads: Cell::new(0),
    };
    let start = Instant::now();
    let matcher = query.matcher(&counted, None);
    let mut hits = 0;
    for (parent, children) in &groups {
        for q in 0..queries {
            let index = q * children.len() / queries;
            let hit = matcher.matches(children[index], &[*parent]);
            assert_eq!(hit, !source.contains("missing") && index % 3 == 0);
            hits += usize::from(hit);
        }
    }
    eprintln!(
        "SELECTOR_QUERY n={} parents={} keys={} queries={queries} elapsed_ns={} child_reads={} class_reads={} hits={hits}",
        groups[0].1.len(),
        groups.len(),
        source.matches(":nth").count(),
        start.elapsed().as_nanos(),
        counted.child_reads.get(),
        doc.class_reads.get()
    );
}
