use super::*;

#[test]
fn every_interrupted_combinator_search_propagates_budget_errors() {
    let mut doc = TestDoc::new();
    let main = doc.push_element(0, "main", None);
    doc.push_element(main, "aside", None);
    let section = doc.push_element(main, "section", None);
    let before = doc.push_element(section, "i", None);
    doc.set_attr(before, "class", "before");
    for _ in 0..12 {
        doc.push_element(section, "b", None);
    }
    let subject = doc.push_element(section, "p", None);
    let leaf = doc.push_element(subject, "em", None);
    doc.set_attr(leaf, "class", "leaf");
    let subject_id = StyleNodeId::new(subject as u64);
    let ancestors = [
        StyleNodeId::new(main as u64),
        StyleNodeId::new(section as u64),
    ];
    let node = doc.node(subject_id).unwrap();
    let elem = node.as_element().unwrap();
    for source in [
        "p:has(> .leaf)",
        "main > section > p",
        "aside + section > p",
        "b + p",
        "main > section > .before ~ p",
        "main > section > .before ~ p::before",
        "main > section > .missing ~ p::before",
    ] {
        let list = crate::parse_selector_list(source).unwrap();
        let selector = &list.slice()[0];
        let run = |caches: &MatchCaches| {
            let ctx = MatchContext::new(&doc, StyleQuirksMode::NoQuirks, None, false, caches);
            if selector.pseudo_element().is_some() {
                selector_matches_pseudo_element(ctx, selector, &elem, subject_id, &ancestors)
                    .map(|matched| matched.is_some())
            } else {
                match_complex_selector_list(&list, ctx, &elem, subject_id, &ancestors)
                    .map(|matched| matched.is_some())
            }
        };
        let unlimited = MatchCaches::default();
        let expected = !source.contains("missing");
        assert_eq!(run(&unlimited).unwrap(), expected, "{source}");
        let work = unlimited.work_count();
        for limit in 0..work {
            let caches = MatchCaches::with_work_limit(Some(limit));
            assert!(
                matches!(run(&caches), Err(CascadeError::LimitExceeded {
                    kind: CascadeLimitKind::SelectorWork, limit: actual_limit, ..
                }) if actual_limit == limit),
                "{source}, limit {limit}"
            );
            assert!(caches.work_count() <= limit, "{source}, limit {limit}");
        }
        let exact = MatchCaches::with_work_limit(Some(work));
        assert_eq!(run(&exact).unwrap(), expected, "{source}");
        assert_eq!(exact.work_count(), work, "{source}");
    }
}
