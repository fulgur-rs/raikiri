use super::*;
use crate::{CascadeError, CascadeLimitKind, SelectorWorkBudget};
use std::cell::Cell;

struct CountingDom<'a> {
    doc: &'a TestDoc,
    reads: Cell<usize>,
}

impl StyleDom for CountingDom<'_> {
    type NodeRef<'a>
        = <TestDoc as StyleDom>::NodeRef<'a>
    where
        Self: 'a;
    type ChildIter<'a>
        = <TestDoc as StyleDom>::ChildIter<'a>
    where
        Self: 'a;
    fn root_id(&self) -> StyleNodeId {
        self.doc.root_id()
    }
    fn node_count(&self) -> usize {
        self.doc.node_count()
    }
    fn parent_id(&self, id: StyleNodeId) -> Option<StyleNodeId> {
        self.doc.parent_id(id)
    }
    fn node(&self, id: StyleNodeId) -> Option<Self::NodeRef<'_>> {
        self.reads.set(self.reads.get() + 1);
        self.doc.node(id)
    }
    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_> {
        self.doc.child_ids(id)
    }
}

fn failure(result: Result<bool, CascadeError>) -> (u64, u64) {
    match result {
        Err(CascadeError::LimitExceeded {
            kind: CascadeLimitKind::SelectorWork,
            limit,
            actual,
        }) => (limit, actual),
        other => panic!("expected selector work failure: {other:?}"),
    }
}

#[test]
fn bounded_queries_preserve_output_at_exact_and_unlimited_work() {
    let (dom, div, p) = doc();
    for source in [
        "p",
        ".a > #x",
        "p:not(:has(.missing))",
        ":scope",
        ":lang(en)",
        ":empty",
    ] {
        let query = SelectorQuery::parse(source).unwrap();
        let expected = query.matches_scoped(&dom, p, &[div], Some(p));
        let unlimited = SelectorWorkBudget::new(None);
        assert_eq!(
            query
                .try_matches_scoped(&dom, p, &[div], Some(p), &unlimited)
                .unwrap(),
            expected
        );
        let needed = unlimited.count();
        let exact = SelectorWorkBudget::new(Some(needed));
        assert_eq!(
            query
                .try_matches_scoped(&dom, p, &[div], Some(p), &exact)
                .unwrap(),
            expected
        );
        assert_eq!(exact.count(), needed);
        let short = SelectorWorkBudget::new(Some(needed - 1));
        assert!(
            query
                .try_matches_scoped(&dom, p, &[div], Some(p), &short)
                .is_err()
        );
        let counted = CountingDom {
            doc: &dom,
            reads: Cell::new(0),
        };
        let zero = SelectorWorkBudget::new(Some(0));
        assert_eq!(
            failure(
                query
                    .bounded_matcher(&counted, None, &zero)
                    .try_matches(p, &[div])
            ),
            (0, 1)
        );
        assert_eq!(counted.reads.get(), 0);
    }
}

#[test]
fn internal_bulk_failure_is_terminal_with_truthful_unused_remaining() {
    let mut raw = TestDoc::new();
    let mut current = 0;
    let mut ancestors = Vec::new();
    for _ in 0..8 {
        current = raw.push_element(current, "div", None);
        ancestors.push(StyleNodeId(current as u64));
    }
    let subject = StyleNodeId(raw.push_element(current, "p", None) as u64);
    let counted = CountingDom {
        doc: &raw,
        reads: Cell::new(0),
    };
    let budget = SelectorWorkBudget::new(Some(5));
    let query = SelectorQuery::parse(":has(.absent)").unwrap();
    let matcher = query.bounded_matcher(&counted, None, &budget);
    let first = failure(matcher.try_matches(subject, &ancestors));
    assert_eq!(first, (5, 10));
    assert_eq!(budget.count(), 2);
    let reads = counted.reads.get();
    for _ in 0..2 {
        assert_eq!(failure(matcher.try_matches(subject, &ancestors)), first);
    }
    let cheap = SelectorQuery::parse("*").unwrap();
    assert_eq!(
        failure(cheap.try_matches_scoped(&counted, subject, &ancestors, None, &budget)),
        first
    );
    assert!(budget.charge(0).is_err());
    assert_eq!(counted.reads.get(), reads);
    assert_eq!(budget.count(), 2);
}

#[test]
fn explicit_bulk_rejection_cannot_be_reused_by_another_query() {
    let (dom, div, p) = doc();
    let counted = CountingDom {
        doc: &dom,
        reads: Cell::new(0),
    };
    let budget = SelectorWorkBudget::new(Some(5));
    let first = budget.charge(9).unwrap_err().to_string();
    assert_eq!(budget.count(), 0);
    assert_eq!(budget.charge(1).unwrap_err().to_string(), first);
    let query = SelectorQuery::parse("*").unwrap();
    assert_eq!(
        query
            .bounded_matcher(&counted, None, &budget)
            .try_matches(p, &[div])
            .unwrap_err()
            .to_string(),
        first
    );
    assert_eq!(counted.reads.get(), 0);
    assert_eq!(budget.count(), 0);
}

#[test]
fn dropping_caches_preserves_work_and_sees_dom_mutation() {
    let (mut dom, div, p) = doc();
    let query = SelectorQuery::parse(".changed").unwrap();
    let budget = SelectorWorkBudget::new(None);
    assert!(
        !query
            .bounded_matcher(&dom, None, &budget)
            .try_matches(p, &[div])
            .unwrap()
    );
    let first = budget.count();
    dom.set_attr(p.0 as usize, "class", "changed");
    assert!(
        query
            .bounded_matcher(&dom, None, &budget)
            .try_matches(p, &[div])
            .unwrap()
    );
    assert_eq!(budget.count(), first * 2);
}
