use super::*;
use std::cell::Cell;
use std::time::Instant;

struct CountingDoc {
    doc: TestDoc,
    class_reads: Cell<usize>,
}

struct CountingNode<'a> {
    node: <TestDoc as StyleDom>::NodeRef<'a>,
    reads: &'a Cell<usize>,
}

struct CountingElement<'a> {
    element: crate::test_dom::TestElementRef<'a>,
    reads: &'a Cell<usize>,
}

struct ChildCountingDoc<'a> {
    doc: &'a CountingDoc,
    child_reads: Cell<usize>,
}

struct CountingChildren<'a> {
    inner: <CountingDoc as StyleDom>::ChildIter<'a>,
    reads: &'a Cell<usize>,
}

impl Iterator for CountingChildren<'_> {
    type Item = StyleNodeId;
    fn next(&mut self) -> Option<StyleNodeId> {
        let child = self.inner.next()?;
        self.reads.set(self.reads.get() + 1);
        Some(child)
    }
}

impl StyleDom for ChildCountingDoc<'_> {
    type NodeRef<'a>
        = CountingNode<'a>
    where
        Self: 'a;
    type ChildIter<'a>
        = CountingChildren<'a>
    where
        Self: 'a;
    fn root_id(&self) -> StyleNodeId {
        self.doc.root_id()
    }
    fn node(&self, id: StyleNodeId) -> Option<Self::NodeRef<'_>> {
        self.doc.node(id)
    }
    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_> {
        CountingChildren {
            inner: self.doc.child_ids(id),
            reads: &self.child_reads,
        }
    }
    fn node_count(&self) -> usize {
        self.doc.node_count()
    }
    fn quirks_mode(&self) -> StyleQuirksMode {
        self.doc.quirks_mode()
    }
    fn parent_id(&self, id: StyleNodeId) -> Option<StyleNodeId> {
        self.doc.parent_id(id)
    }
}

impl StyleDom for CountingDoc {
    type NodeRef<'a> = CountingNode<'a>;
    type ChildIter<'a> = <TestDoc as StyleDom>::ChildIter<'a>;

    fn root_id(&self) -> StyleNodeId {
        self.doc.root_id()
    }
    fn node(&self, id: StyleNodeId) -> Option<CountingNode<'_>> {
        self.doc.node(id).map(|node| CountingNode {
            node,
            reads: &self.class_reads,
        })
    }
    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_> {
        self.doc.child_ids(id)
    }
    fn node_count(&self) -> usize {
        self.doc.node_count()
    }
    fn parent_id(&self, id: StyleNodeId) -> Option<StyleNodeId> {
        self.doc.parent_id(id)
    }
    fn quirks_mode(&self) -> StyleQuirksMode {
        self.doc.quirks_mode()
    }
}

impl StyleNode for CountingNode<'_> {
    type Element<'b>
        = CountingElement<'b>
    where
        Self: 'b;
    fn kind(&self) -> StyleNodeKind {
        self.node.kind()
    }
    fn as_element(&self) -> Option<CountingElement<'_>> {
        self.node.as_element().map(|element| CountingElement {
            element,
            reads: self.reads,
        })
    }
    fn text_content(&self) -> Option<&str> {
        self.node.text_content()
    }
    fn is_in_document(&self) -> bool {
        self.node.is_in_document()
    }
}

impl StyleElement for CountingElement<'_> {
    fn tag_name(&self) -> &str {
        self.element.tag_name()
    }
    fn inline_style_source(&self) -> Option<&str> {
        self.element.inline_style_source()
    }
    fn attr(&self, name: &str) -> Option<&str> {
        if name == "class" {
            self.reads.set(self.reads.get() + 1);
        }
        self.element.attr(name)
    }
    fn namespace_uri(&self) -> Option<&str> {
        self.element.namespace_uri()
    }
}

fn fixture(n: usize) -> (CountingDoc, StyleNodeId, Vec<StyleNodeId>) {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "ul", None);
    let children = (0..n)
        .map(|i| {
            let child = doc.push_element(parent, "li", None);
            doc.set_attr(child, "class", if i % 3 == 0 { "visible" } else { "other" });
            StyleNodeId::new(child as u64)
        })
        .collect();
    (
        CountingDoc {
            doc,
            class_reads: Cell::new(0),
        },
        StyleNodeId::new(parent as u64),
        children,
    )
}

#[test]
fn filtered_nth_matching_evaluates_each_sibling_once_per_filter() {
    for n in [16, 128, 1024] {
        let (doc, parent, children) = fixture(n);
        let query = crate::SelectorQuery::parse(":nth-child(2n of .visible)").unwrap();
        let matcher = query.matcher(&doc, None);
        for _ in 0..2 {
            for (i, &child) in children.iter().enumerate() {
                assert_eq!(
                    matcher.matches(child, &[parent]),
                    i % 3 == 0 && (i / 3 + 1) % 2 == 0
                );
            }
        }
        assert_eq!(doc.class_reads.get(), n, "n={n}");
    }
}

#[test]
fn one_shot_filtered_query_scans_siblings_only_once() {
    for n in [8, 128, 1024] {
        let (doc, parent, children) = fixture(n);
        let counted = ChildCountingDoc {
            doc: &doc,
            child_reads: Cell::new(0),
        };
        let query = crate::SelectorQuery::parse(":nth-child(1 of :scope > .visible)").unwrap();
        assert!(query.matches_scoped(&counted, children[0], &[parent], Some(parent)));
        assert_eq!(counted.child_reads.get(), n);
        assert_eq!(doc.class_reads.get(), n);
    }
}

#[test]
fn over_budget_parents_skip_repeated_admission_counts() {
    let (doc, parent, children) = fixture(16);
    let counted = ChildCountingDoc {
        doc: &doc,
        child_reads: Cell::new(0),
    };
    let caches = MatchCaches::default();
    *caches.filtered_nth.borrow_mut() = FilteredNthCache::with_limits(4, 7);
    let ctx = MatchContext::new(&counted, StyleQuirksMode::NoQuirks, None, false, &caches);
    let lists = [
        crate::parse_selector_list(":nth-child(n of .visible)").unwrap(),
        crate::parse_selector_list(":nth-last-child(n of .visible)").unwrap(),
    ];
    for list in &lists {
        for (i, &child) in children.iter().enumerate() {
            let node = counted.node(child).unwrap();
            let element = node.as_element().unwrap();
            assert_eq!(
                match_complex_selector_list(list, ctx, &element, child, &[parent])
                    .unwrap()
                    .is_some(),
                i % 3 == 0
            );
        }
    }
    assert_eq!(counted.child_reads.get(), 8 + 2 * 16 * 16);
}

#[test]
fn filtered_nth_cache_agrees_with_uncached_matching_across_contexts() {
    let (mut doc, parent, children) = fixture(32);
    doc.doc.set_in_document(children[6].0 as usize, false);
    doc.doc.push_text(parent.0 as usize, "ignored text");
    let other_parent = doc.doc.push_element(0, "section", None);
    let others: Vec<_> = (0..16)
        .map(|i| {
            let id = doc.doc.push_element(other_parent, "div", None);
            doc.doc
                .set_attr(id, "class", if i % 2 == 0 { "visible" } else { "OTHER" });
            StyleNodeId::new(id as u64)
        })
        .collect();
    let caches = MatchCaches::default();
    let direct = MatchCaches::default();
    *direct.filtered_nth.borrow_mut() = FilteredNthCache::with_limits(0, 0);
    let lists: Vec<_> = [
        ":nth-child(2n of .visible)",
        ":nth-last-child(3 of .visible, .other)",
        ":nth-child(0 of .visible)",
        ":nth-child(n of .absent)",
        ":nth-child(1 of :scope > .visible)",
        ":nth-child(2n of :is(.visible, .other):not(.OTHER))",
        ":nth-child(1 of :has(+ .visible))",
        ":is(:nth-child(2n of .visible), :nth-last-child(1 of .other))",
        // Public entry points still reject nested nth filters. Exercise
        // recursive cache construction directly without changing that policy.
        ":nth-child(1 of :nth-child(2n of .visible))",
    ]
    .into_iter()
    .map(|s| crate::parse_selector_list(s).unwrap())
    .collect();
    for scope in [
        None,
        Some(parent),
        Some(StyleNodeId::new(other_parent as u64)),
    ] {
        for allow_detached in [false, true] {
            for mode in [StyleQuirksMode::NoQuirks, StyleQuirksMode::Quirks] {
                let cached_ctx = MatchContext::new(&doc, mode, scope, allow_detached, &caches);
                let direct_ctx = MatchContext::new(&doc, mode, scope, allow_detached, &direct);
                for (parent, children) in [
                    (parent, &children),
                    (StyleNodeId::new(other_parent as u64), &others),
                ] {
                    for list in &lists {
                        for &child in children {
                            let node = doc.node(child).unwrap();
                            let element = node.as_element().unwrap();
                            assert_eq!(
                                match_complex_selector_list(
                                    list,
                                    cached_ctx,
                                    &element,
                                    child,
                                    &[parent]
                                )
                                .unwrap(),
                                match_complex_selector_list(
                                    list,
                                    direct_ctx,
                                    &element,
                                    child,
                                    &[parent]
                                )
                                .unwrap(),
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn filtered_nth_budget_fallback_preserves_existing_entries() {
    for (entries, slots) in [(1, 64), (8, 16), (8, 15), (0, 64)] {
        let (doc, parent, children) = fixture(16);
        let caches = MatchCaches::default();
        *caches.filtered_nth.borrow_mut() = FilteredNthCache::with_limits(entries, slots);
        let ctx = MatchContext::new(&doc, StyleQuirksMode::NoQuirks, None, false, &caches);
        let first = crate::parse_selector_list(":nth-child(n of .visible)").unwrap();
        let second = crate::parse_selector_list(":nth-child(n of .other)").unwrap();
        for list in [&first, &second, &first] {
            for &child in &children {
                let node = doc.node(child).unwrap();
                let element = node.as_element().unwrap();
                let expected = if std::ptr::eq(list, &first) {
                    element.attr("class") == Some("visible")
                } else {
                    element.attr("class") == Some("other")
                };
                assert_eq!(
                    match_complex_selector_list(list, ctx, &element, child, &[parent])
                        .unwrap()
                        .is_some(),
                    expected
                );
            }
        }
        // Remove the 48 explicit class reads used by the oracle above.
        let filter_reads = doc.class_reads.get() - 48;
        assert_eq!(
            filter_reads,
            if entries > 0 && slots >= 16 {
                16 + 256
            } else {
                3 * 256
            }
        );
    }
}

#[test]
fn cascade_shares_filtered_ranks_and_recomputes_after_mutation() {
    let n = 256;
    let (mut doc, _, children) = fixture(n);
    let style = doc.doc.push_element(0, "style", None);
    doc.doc
        .push_text(style, "li:nth-child(2n of .visible) { color: red }");
    for pass in 0..2 {
        let tree = build_rule_tree(&doc);
        doc.class_reads.set(0);
        let result = cascade(&doc, &tree).unwrap();
        for (i, child) in children.iter().enumerate() {
            let expected = i >= 3 * pass && i % 3 == 0 && (i / 3 + 1 - pass) % 2 == 0;
            assert_eq!(result.computed[child.0 as usize].color == RED, expected);
        }
        assert!(doc.class_reads.get() < 16 * n);
        doc.doc.nodes[children[0].0 as usize].attrs.clear();
        doc.doc.set_attr(children[0].0 as usize, "class", "other");
    }
}

#[test]
fn filtered_combinators_use_full_sibling_ancestor_chain() {
    let mut doc = TestDoc::new();
    let outer = doc.push_element(0, "section", None);
    doc.set_attr(outer, "class", "outer");
    let parent = doc.push_element(outer, "ul", None);
    let children: Vec<_> = (0..32)
        .map(|i| {
            let child = doc.push_element(parent, "li", None);
            doc.set_attr(child, "class", if i % 2 == 0 { "visible" } else { "other" });
            StyleNodeId::new(child as u64)
        })
        .collect();
    let ancestors = [
        StyleNodeId::new(outer as u64),
        StyleNodeId::new(parent as u64),
    ];
    let query = crate::SelectorQuery::parse(
        ":nth-last-child(2n of .outer > ul > :is(.visible):not(.other))",
    )
    .unwrap();
    let matcher = query.matcher(&doc, None);
    for (i, &child) in children.iter().enumerate() {
        assert_eq!(
            matcher.matches(child, &ancestors),
            i % 2 == 0 && (16 - i / 2) % 2 == 0
        );
    }
}

#[test]
fn filtered_nth_cache_does_not_survive_dom_changes() {
    let (mut doc, parent, children) = fixture(16);
    let query = crate::SelectorQuery::parse(":nth-child(1 of .visible)").unwrap();
    assert!(query.matcher(&doc, None).matches(children[0], &[parent]));
    doc.doc.nodes[children[0].0 as usize].attrs.clear();
    doc.doc.set_attr(children[0].0 as usize, "class", "other");
    let matcher = query.matcher(&doc, None);
    assert!(!matcher.matches(children[0], &[parent]));
    assert!(matcher.matches(children[3], &[parent]));
}

#[test]
fn small_filtered_nth_parents_keep_direct_scan() {
    for n in [1, 4, 7] {
        let (doc, parent, children) = fixture(n);
        let query = crate::SelectorQuery::parse(":nth-child(n of .visible)").unwrap();
        let matcher = query.matcher(&doc, None);
        for &child in &children {
            matcher.matches(child, &[parent]);
        }
        assert_eq!(doc.class_reads.get(), n * n);
    }
}

#[test]
#[ignore = "manual filtered nth performance probe"]
fn filtered_nth_scaling_probe() {
    let n = std::env::var("FILTERED_NTH_N")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(1024);
    let filters = std::env::var("FILTERED_NTH_FILTERS")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(1);
    let repeats: usize = std::env::var("FILTERED_NTH_REPEATS")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(1);
    let (doc, parent, children) = fixture(n);
    let source = (1..=filters)
        .map(|i| format!(":nth-child({i}n of .visible)"))
        .collect::<Vec<_>>()
        .join(", ");
    let query = crate::SelectorQuery::parse(&source).unwrap();
    let start = Instant::now();
    let mut hits = 0;
    for _ in 0..repeats {
        let matcher = query.matcher(&doc, None);
        hits += children
            .iter()
            .filter(|&&child| matcher.matches(child, &[parent]))
            .count();
    }
    let elapsed = start.elapsed();
    assert_eq!(hits, n.div_ceil(3) * repeats);
    eprintln!(
        "FILTERED_NTH n={n} filters={filters} repeats={repeats} elapsed_ns={} class_reads={} hits={hits}",
        elapsed.as_nanos(),
        doc.class_reads.get()
    );
}

#[test]
#[ignore = "manual filtered nth edge-path performance probe"]
fn filtered_nth_edge_scaling_probe() {
    let n: usize = std::env::var("FILTERED_NTH_N")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(1024);
    let queries: usize = std::env::var("FILTERED_NTH_QUERIES")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(1);
    let repeats: usize = std::env::var("FILTERED_NTH_REPEATS")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(1);
    let mode = std::env::var("FILTERED_NTH_MODE").unwrap_or_else(|_| "single".into());
    assert!(mode == "single" || mode == "over-budget");
    let (doc, parent, children) = fixture(n);
    let query = crate::SelectorQuery::parse(":nth-child(n of .visible)").unwrap();
    let expected: usize = (0..queries)
        .filter(|&q| (q * n / queries).is_multiple_of(3))
        .count();
    let start = Instant::now();
    let mut hits = 0;
    for _ in 0..repeats {
        if mode == "single" {
            for q in 0..queries {
                hits +=
                    query.matches_scoped(&doc, children[q * n / queries], &[parent], Some(parent))
                        as usize;
            }
        } else {
            let matcher = query.matcher(&doc, None);
            for q in 0..queries {
                hits += matcher.matches(children[q * n / queries], &[parent]) as usize;
            }
        }
    }
    let elapsed = start.elapsed();
    assert_eq!(hits, expected * repeats);
    eprintln!(
        "FILTERED_NTH_EDGE mode={mode} n={n} queries={queries} repeats={repeats} elapsed_ns={} class_reads={} hits={hits}",
        elapsed.as_nanos(),
        doc.class_reads.get()
    );
}

#[test]
fn a_dom_that_reports_no_focus_matches_no_focus_state() {
    let (mut doc, parent, children) = fixture(1);
    doc.doc.focused = Some(children[0].0 as usize);
    let focus = crate::SelectorQuery::parse(":focus").unwrap();
    // `CountingDoc` keeps the trait's default `focused_element`, so the
    // focus recorded on the wrapped document stays invisible to matching.
    assert!(!focus.matches(&doc, children[0], &[parent]));
    assert!(focus.matches(&doc.doc, children[0], &[parent]));
    let visible = crate::SelectorQuery::parse(":focus-visible").unwrap();
    assert!(!visible.matches(&doc, children[0], &[parent]));
}

fn limited_match<D: StyleDom>(
    doc: &D,
    list: &SelectorList<RaikiriSelectorImpl>,
    caches: &MatchCaches,
    subject: StyleNodeId,
    ancestors: &[StyleNodeId],
) -> Result<Option<Specificity>, CascadeError> {
    let node = doc.node(subject).unwrap();
    let element = node.as_element().unwrap();
    match_complex_selector_list(
        list,
        MatchContext::new(doc, StyleQuirksMode::NoQuirks, None, false, caches),
        &element,
        subject,
        ancestors,
    )
}

#[test]
fn selector_work_exact_limits_cover_internal_searches_and_negation() {
    for (source, on_parent) in [
        (".visible", false),
        (":has(.absent)", true),
        (":has(> .visible)", true),
        (":has(~ .absent)", false),
        (":has(+ .absent)", false),
        (":not(:has(.absent))", true),
        (":is(:has(.absent), *)", true),
        (".absent ~ .visible", false),
        (".absent + .visible", false),
        ("aside section .visible", false),
        (":nth-child(2n of .visible)", false),
        (":nth-child(2n)", false),
        (":empty", true),
        (":lang(en)", false),
        (":dir(rtl)", false),
    ] {
        let (doc, parent, children) = fixture(64);
        let counted = ChildCountingDoc {
            doc: &doc,
            child_reads: Cell::new(0),
        };
        let list = crate::parse_selector_list(source).unwrap();
        let (subject, ancestors) = if on_parent {
            (parent, vec![])
        } else {
            (children[63], vec![parent])
        };
        let unlimited = MatchCaches::default();
        let expected = limited_match(&counted, &list, &unlimited, subject, &ancestors).unwrap();
        let work = unlimited.work_count();
        assert!(work > 0, "{source}");
        let exact = MatchCaches::with_work_limit(Some(work));
        assert_eq!(
            limited_match(&counted, &list, &exact, subject, &ancestors).unwrap(),
            expected,
            "{source}"
        );
        assert_eq!(exact.work_count(), work, "{source}");
        for limit in [0, work - 1] {
            let caches = MatchCaches::with_work_limit(Some(limit));
            counted.child_reads.set(0);
            let result = limited_match(&counted, &list, &caches, subject, &ancestors);
            assert!(
                matches!(result, Err(CascadeError::LimitExceeded {kind: CascadeLimitKind::SelectorWork, limit: actual_limit, actual}) if actual_limit == limit && actual == limit + 1),
                "{source} {limit}: {result:?}"
            );
            assert!(counted.child_reads.get() <= limit as usize + 1, "{source}");
            let stopped_reads = counted.child_reads.get();
            let stopped_classes = doc.class_reads.get();
            assert!(limited_match(&counted, &list, &caches, subject, &ancestors).is_err());
            assert_eq!(counted.child_reads.get(), stopped_reads, "{source}");
            assert_eq!(doc.class_reads.get(), stopped_classes, "{source}");
        }
    }
}

#[test]
fn interrupted_cache_builds_are_never_committed_or_replaced_by_fallback() {
    for source in [
        ":nth-child(n of .visible)",
        ":nth-child(n)",
        ":has(.absent)",
    ] {
        let (doc, parent, children) = fixture(64);
        let counted = ChildCountingDoc {
            doc: &doc,
            child_reads: Cell::new(0),
        };
        let list = crate::parse_selector_list(source).unwrap();
        let caches = MatchCaches::with_work_limit(Some(70));
        let (subject, ancestors) = if source.starts_with(":has") {
            (parent, vec![])
        } else {
            (children[0], vec![parent])
        };
        assert!(limited_match(&counted, &list, &caches, subject, &ancestors).is_err());
        assert!(caches.filtered_nth_entries() == 0);
        assert_eq!(caches.cached_sibling_parents(), 0);
        assert_eq!(caches.has_entries(), 0);
        assert!(counted.child_reads.get() <= 71);
    }
}

#[test]
fn filtered_nth_real_cache_slot_boundaries_preserve_ranks_and_bound_fallback() {
    for n in [65_535, 65_536, 65_537] {
        let (doc, parent, children) = fixture(n);
        let list = crate::parse_selector_list(":nth-child(2n of .visible)").unwrap();
        let caches = MatchCaches::default();
        for index in [0, 3, n - 1] {
            let matched = limited_match(&doc, &list, &caches, children[index], &[parent])
                .unwrap()
                .is_some();
            assert_eq!(
                matched,
                index % 3 == 0 && (index / 3 + 1) % 2 == 0,
                "n={n} index={index}"
            );
        }
        assert_eq!(caches.filtered_nth_entries(), usize::from(n <= 65_536));
        assert!(caches.filtered_nth_slots() <= 65_536);
        let budgeted = MatchCaches::with_work_limit(Some(250_000));
        let mut stopped = false;
        for &child in children.iter().take(64) {
            if limited_match(&doc, &list, &budgeted, child, &[parent]).is_err() {
                stopped = true;
                break;
            }
        }
        assert_eq!(stopped, n > 65_536);
        assert!(budgeted.work_count() <= 250_000);
    }
}

#[test]
fn filtered_nth_256_and_257_filter_keys_keep_memory_bounded() {
    let (doc, parent, children) = fixture(8);
    let lists: Vec<_> = (1..=257)
        .map(|i| crate::parse_selector_list(&format!(":nth-child({i}n of .visible)")).unwrap())
        .collect();
    let caches = MatchCaches::default();
    for (i, list) in lists.iter().enumerate() {
        let matched = limited_match(&doc, list, &caches, children[3], &[parent])
            .unwrap()
            .is_some();
        assert_eq!(matched, 2 % (i + 1) == 0);
        assert_eq!(caches.filtered_nth_entries(), (i + 1).min(256));
    }
    assert_eq!(caches.filtered_nth_slots(), 256 * 8);
    let full = caches.work_count();
    for _ in 0..8 {
        limited_match(&doc, &lists[256], &caches, children[3], &[parent]).unwrap();
    }
    assert_eq!(caches.work_count() - full, 8 * (1 + 8 * 2));
}

#[test]
fn filtered_nth_multiple_parents_exhaust_aggregate_slots() {
    let (mut doc, first, children) = fixture(32_768);
    let mut parents = vec![(first, children[3])];
    for _ in 0..2 {
        let parent = doc.doc.push_element(0, "section", None);
        let mut subject = 0;
        for index in 0..32_768 {
            let child = doc.doc.push_element(parent, "li", None);
            doc.doc.set_attr(
                child,
                "class",
                if index % 3 == 0 { "visible" } else { "other" },
            );
            if index == 3 {
                subject = child;
            }
        }
        parents.push((
            StyleNodeId::new(parent as u64),
            StyleNodeId::new(subject as u64),
        ));
    }
    let list = crate::parse_selector_list(":nth-child(2 of .visible)").unwrap();
    let caches = MatchCaches::default();
    for &(parent, subject) in &parents {
        assert!(
            limited_match(&doc, &list, &caches, subject, &[parent])
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(caches.filtered_nth_entries(), 2);
    assert_eq!(caches.filtered_nth_slots(), 65_536);
    let before = caches.work_count();
    let (parent, subject) = parents[2];
    limited_match(&doc, &list, &caches, subject, &[parent]).unwrap();
    assert_eq!(caches.work_count() - before, 1 + 2 * 32_768);
    let bounded = MatchCaches::with_work_limit(Some(250_000));
    let mut stopped = false;
    for &(parent, subject) in &parents {
        if limited_match(&doc, &list, &bounded, subject, &[parent]).is_err() {
            stopped = true;
            break;
        }
    }
    assert!(stopped);
}

#[test]
fn cached_cursors_and_has_list_validation_consume_work() {
    for source in [
        ":has(+ .absent)",
        ":nth-child(n of .visible)",
        ".absent + .visible",
        ".absent ~ .visible",
        ":has(+ .visible, + .other, ~ .absent)",
    ] {
        let (doc, parent, children) = fixture(32);
        let list = crate::parse_selector_list(source).unwrap();
        let caches = MatchCaches::default();
        let subject = children[30];
        limited_match(&doc, &list, &caches, subject, &[parent]).unwrap();
        caches.work.reset(None);
        let expected = limited_match(&doc, &list, &caches, subject, &[parent]).unwrap();
        let hot = caches.work_count();
        assert!(hot > 0);
        caches.work.reset(Some(hot));
        assert_eq!(
            limited_match(&doc, &list, &caches, subject, &[parent]).unwrap(),
            expected
        );
        caches.work.reset(Some(hot - 1));
        assert!(
            limited_match(&doc, &list, &caches, subject, &[parent]).is_err(),
            "{source}"
        );
    }
}

#[test]
fn has_deep_subtree_and_combinator_backtracking_stop_promptly() {
    let mut raw = TestDoc::new();
    let root = raw.push_element(0, "section", None);
    let mut parent = root;
    let mut ancestors = vec![];
    for _ in 0..64 {
        ancestors.push(StyleNodeId::new(parent as u64));
        parent = raw.push_element(parent, "div", None);
        raw.set_attr(parent, "class", "visible");
        raw.push_element(parent, "span", None);
    }
    let doc = CountingDoc {
        doc: raw,
        class_reads: Cell::new(0),
    };
    for (source, subject, path) in [
        (":not(:has(.absent))", StyleNodeId::new(root as u64), vec![]),
        (
            "aside div div .visible",
            StyleNodeId::new(parent as u64),
            ancestors.clone(),
        ),
    ] {
        let counted = ChildCountingDoc {
            doc: &doc,
            child_reads: Cell::new(0),
        };
        let list = crate::parse_selector_list(source).unwrap();
        let unlimited = MatchCaches::default();
        limited_match(&counted, &list, &unlimited, subject, &path).unwrap();
        assert!(unlimited.work_count() > 100);
        let limited = MatchCaches::with_work_limit(Some(40));
        counted.child_reads.set(0);
        assert!(limited_match(&counted, &list, &limited, subject, &path).is_err());
        assert!(counted.child_reads.get() <= 41);
        assert!(limited.work_count() <= 40);
    }
}

mod budget_probe_tests;
mod work_probe_tests;

#[test]
fn directionality_auto_and_empty_child_walks_propagate_budget_errors() {
    for source in [":dir(ltr)", ":empty"] {
        let mut raw = TestDoc::new();
        let parent = raw.push_element(0, "section", None);
        raw.set_attr(parent, "dir", "auto");
        for _ in 0..64 {
            raw.push_comment(parent, "ignored");
        }
        let doc = CountingDoc {
            doc: raw,
            class_reads: Cell::new(0),
        };
        let counted = ChildCountingDoc {
            doc: &doc,
            child_reads: Cell::new(0),
        };
        let list = crate::parse_selector_list(source).unwrap();
        let subject = StyleNodeId::new(parent as u64);
        assert!(
            limited_match(&counted, &list, &MatchCaches::default(), subject, &[])
                .unwrap()
                .is_some()
        );
        counted.child_reads.set(0);
        let caches = MatchCaches::with_work_limit(Some(12));
        assert!(limited_match(&counted, &list, &caches, subject, &[]).is_err());
        assert!(counted.child_reads.get() <= 13);
        assert!(caches.directions.borrow().is_empty());
    }
}

#[test]
fn early_probe_gate_limits_filtered_fallback_class_and_child_work() {
    let n = 128;
    let (doc, parent, children) = fixture(n);
    let counted = ChildCountingDoc {
        doc: &doc,
        child_reads: Cell::new(0),
    };
    let caches = MatchCaches::default();
    *caches.filtered_nth.borrow_mut() = FilteredNthCache::with_limits(0, 0);
    let source = "li:nth-child(-n+64):nth-child(n of .visible)";
    let list = crate::parse_selector_list(source).unwrap();
    let mut hits = 0;
    for &child in &children {
        hits += usize::from(
            limited_match(&counted, &list, &caches, child, &[parent])
                .unwrap()
                .is_some(),
        );
    }
    eprintln!(
        "SELECTOR_GATE source={source} n={n} class_reads={} child_reads={} work={} hits={hits}",
        doc.class_reads.get(),
        counted.child_reads.get(),
        caches.work_count()
    );
    assert_eq!(hits, 22);
    assert_eq!(doc.class_reads.get(), n * 64);
    assert_eq!(counted.child_reads.get(), n * 64 + n + 8);
    assert_eq!(caches.work_count(), 16_840);
}
