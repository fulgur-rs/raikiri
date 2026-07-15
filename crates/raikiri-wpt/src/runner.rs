//! WPT test runner dispatch (spec §12.6). Type stubs; run logic lands in M3.

use crate::expectations::ExpectationSet;

/// Dispatches WPT tests and records outcomes.
///
/// In M1 the constructor stores the loaded `ExpectationSet`; `run_test`
/// is a `todo!()` populated in M3 alongside the tree-walker and reftest
/// runner (currently stubs in [`crate::reftest`]).
#[non_exhaustive]
pub struct WptRunner {
    #[allow(dead_code)] // field held for M3 populate; silence M1 warning.
    expectations: ExpectationSet,
}

impl WptRunner {
    /// Construct a runner from a loaded [`ExpectationSet`].
    pub fn new(expectations: ExpectationSet) -> Self {
        Self { expectations }
    }

    /// Run one WPT test by id. **M1 stub**: panics via `todo!()`.
    pub fn run_test(&self, _test_id: &str) -> TestExecution {
        todo!("M3: walk the WPT submodule tree and dispatch to reftest::run_pair")
    }
}

/// The record produced by executing a single WPT test.
#[non_exhaustive]
pub struct TestExecution {
    /// Identifier of the executed test.
    pub test_id: String,
    /// Result of executing the test.
    pub outcome: TestOutcome,
}

/// Outcome of a single WPT test execution.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TestOutcome {
    /// The test passed.
    Pass,
    /// The test failed, with a reason.
    Fail(String),
    /// The test was skipped, with a reason.
    Skip(String),
    /// The test is quarantined (known-flaky, treated as informational per spec §12.10).
    Quarantined,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outcome_variants_are_constructible() {
        assert!(matches!(TestOutcome::Pass, TestOutcome::Pass));
        assert!(matches!(TestOutcome::Fail("boom".to_owned()), TestOutcome::Fail(_)));
        assert!(matches!(TestOutcome::Skip("non-goal".to_owned()), TestOutcome::Skip(_)));
        assert!(matches!(TestOutcome::Quarantined, TestOutcome::Quarantined));
    }
}
