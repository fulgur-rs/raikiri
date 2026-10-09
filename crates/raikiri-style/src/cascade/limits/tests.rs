use super::*;

fn limit_kind(result: Result<(), CascadeError>) -> Option<(CascadeLimitKind, u64, u64)> {
    match result {
        Ok(()) => None,
        Err(CascadeError::LimitExceeded {
            kind,
            limit,
            actual,
        }) => Some((kind, limit, actual)),
        Err(other) => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn a_counter_reaches_its_limit_and_fails_past_it() {
    let kind = CascadeLimitKind::SelectorTests;
    let mut counter = Counter::new(Some(5), kind);
    assert_eq!(limit_kind(counter.add(3)), None);
    assert_eq!(limit_kind(counter.add(2)), None);
    assert_eq!(counter.count(), 5);
    assert_eq!(limit_kind(counter.add(1)), Some((kind, 5, 6)));
    // A failed add adds nothing.
    assert_eq!(counter.count(), 5);
    counter.reset();
    assert_eq!(counter.count(), 0);
    assert_eq!(limit_kind(counter.add(5)), None);
}

#[test]
fn a_counter_without_a_limit_saturates() {
    let mut counter = Counter::new(None, CascadeLimitKind::DeclarationsVisited);
    assert_eq!(limit_kind(counter.add(u64::MAX - 1)), None);
    assert_eq!(limit_kind(counter.add(5)), None);
    assert_eq!(counter.count(), u64::MAX);
}

#[test]
fn the_element_count_restarts_and_the_total_does_not() {
    let limits = CascadeLimits {
        max_candidates_per_element: Some(4),
        max_declarations_visited: Some(10),
        ..CascadeLimits::default()
    };
    let mut budget = CandidateBudget::new(&limits);
    budget.start_element();
    assert_eq!(limit_kind(budget.charge(4)), None);
    assert_eq!(
        limit_kind(budget.charge(1)),
        Some((CascadeLimitKind::CandidatesPerElement, 4, 5))
    );
    assert_eq!(limit_kind(budget.finish_element()), None);
    budget.start_element();
    assert_eq!(limit_kind(budget.charge(4)), None);
    assert_eq!(limit_kind(budget.finish_element()), None);
    budget.start_element();
    assert_eq!(limit_kind(budget.charge(3)), None);
    assert_eq!(
        limit_kind(budget.finish_element()),
        Some((CascadeLimitKind::DeclarationsVisited, 10, 11))
    );
    assert_eq!((budget.max_element(), budget.visited()), (4, 8));
}

#[test]
fn an_unlimited_budget_never_fails() {
    let mut budget = CandidateBudget::unlimited();
    assert_eq!(limit_kind(budget.charge(usize::MAX)), None);
    assert_eq!(limit_kind(budget.charge(usize::MAX)), None);
}

#[test]
fn kept_bytes_count_against_both_result_limits() {
    let limits = CascadeLimits {
        max_retained_bytes: Some(100),
        max_output_bytes: Some(150),
        ..CascadeLimits::default()
    };
    let mut budget = ResultBudget::new(&limits);
    assert_eq!(limit_kind(budget.retained(100)), None);
    assert_eq!(
        limit_kind(budget.retained(1)),
        Some((CascadeLimitKind::RetainedBytes, 100, 101))
    );
    assert_eq!(limit_kind(budget.output(50)), None);
    assert_eq!(
        limit_kind(budget.output(1)),
        Some((CascadeLimitKind::OutputBytes, 150, 151))
    );
}

#[test]
fn byte_counts_saturate() {
    assert_eq!(bytes_of::<u64>(3), 24);
    assert_eq!(bytes_of::<u64>(usize::MAX), u64::MAX);
}

#[test]
fn try_filled_fills_every_slot() {
    assert_eq!(try_filled(3, 7u8).expect("a tiny buffer"), [7, 7, 7]);
}

#[test]
fn every_default_limit_is_finite() {
    let limits = CascadeLimits::default();
    assert!(limits.max_candidates_per_element.is_some());
    assert!(limits.max_declarations_visited.is_some());
    assert!(limits.max_selector_tests.is_some());
    assert!(limits.max_retained_bytes.is_some());
    assert!(limits.max_output_bytes.is_some());
    assert_eq!(CascadeOptions::default().limits, limits);
}
