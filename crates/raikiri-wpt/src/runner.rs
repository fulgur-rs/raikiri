//! WPT test runner dispatch (spec §12.6).
//!
//! `WptRunner` holds the loaded `ExpectationSet` and orchestrates reftest
//! execution. Filtering by `deprecated`/`quarantine`/`baseline` follows
//! spec §12.10 precedence. The actual pixel work is delegated to
//! `crate::reftest`.

use std::path::Path;

use crate::expectations::ExpectationSet;
use crate::oracle::{BlitzOracle, OracleDiff};
use crate::reftest::{
    ReftestConfig, ReftestPair, ReftestResult, compare_images, render_blitz, render_raikiri,
};

// ── Tolerance ──────────────────────────────────────────────────────────

/// Pixel tolerance tier (mirrors `raikiri_vrt::reference::Tolerance`).
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Tolerance {
    /// Max per-channel absolute delta (0 = pixel-exact).
    pub max_delta: u8,
    /// Max fraction of pixels allowed to differ.
    pub max_diff_fraction: f32,
}

impl Tolerance {
    /// Pixel-exact (Tier 1: Linux x86_64).
    pub const EXACT: Self = Self {
        max_delta: 0,
        max_diff_fraction: 0.0,
    };
    /// Tier 2 (Linux aarch64, macOS): max_delta=1, max_diff=0.1%.
    pub const TIER2: Self = Self {
        max_delta: 1,
        max_diff_fraction: 0.001,
    };
    /// Tier 3 (Windows): max_delta=2, max_diff=0.5%.
    pub const TIER3: Self = Self {
        max_delta: 2,
        max_diff_fraction: 0.005,
    };
}

impl Eq for Tolerance {}

// ── WptRunner ──────────────────────────────────────────────────────────

/// Dispatches WPT tests and records outcomes.
#[non_exhaustive]
pub struct WptRunner {
    expectations: ExpectationSet,
}

impl WptRunner {
    /// Construct a runner from a loaded [`ExpectationSet`].
    pub fn new(expectations: ExpectationSet) -> Self {
        Self { expectations }
    }

    /// Borrow the loaded expectations.
    pub fn expectations(&self) -> &ExpectationSet {
        &self.expectations
    }

    /// Classify a `test_id` by expectations precedence.
    ///
    /// Returns `TestOutcome` for tests that are fully excluded or
    /// quarantined (so the caller can short-circuit before rendering).
    /// `None` means the test is not filtered and should be executed.
    pub fn classify(&self, test_id: &str) -> Option<TestOutcome> {
        // Precedence 1: deprecated => Skip (excluded)
        if self.expectations.deprecated.entries.contains(test_id) {
            return Some(TestOutcome::Skip("deprecated (spec §12.10)".to_owned()));
        }
        // Precedence 2: quarantine => Quarantined
        // quarantine entries may be filtered by platform; for the runner's
        // default classify we treat any quarantine entry for the test_id as quarantined.
        // Platform-aware filtering is available via `classify_with_matrix`.
        if self
            .expectations
            .quarantine
            .entries
            .iter()
            .any(|e| e.test_id == test_id)
        {
            return Some(TestOutcome::Quarantined);
        }
        // known-issues => Skip (non-goal)
        if self
            .expectations
            .known_issues
            .entries
            .iter()
            .any(|(pat, _)| test_id_matches_pattern(test_id, pat))
        {
            return Some(TestOutcome::Skip("known-issue (non-goal)".to_owned()));
        }
        None
    }

    /// Run one WPT test by id.
    ///
    /// If the test is filtered by expectations (deprecated/quarantined/known-issue),
    /// returns the filtered outcome without rendering. Otherwise returns
    /// `Skip("no reftest pair discovered")` when no pair can be formed
    /// (caller should supply a `wpt_root` and use `run_reftest_file` for
    /// real execution).
    pub fn run_test(&self, test_id: &str) -> TestExecution {
        if let Some(outcome) = self.classify(test_id) {
            return TestExecution {
                test_id: test_id.to_owned(),
                outcome,
            };
        }
        // No WPT tree walk in this entry point; caller that wants real
        // reftest rendering should use `run_reftest_file` / `run_reftest_pair`.
        TestExecution {
            test_id: test_id.to_owned(),
            outcome: TestOutcome::Skip("no reftest pair discovered (supply wpt_root)".to_owned()),
        }
    }

    /// Run a reftest pair discovered at `pair`, respecting expectations.
    ///
    /// If `pair.test`'s id is filtered, the filtered outcome is returned
    /// without rendering. Otherwise delegates to `crate::reftest::run_pair`.
    pub fn run_reftest_pair(&self, pair: &ReftestPair, config: ReftestConfig) -> ReftestResult {
        let test_id = pair.test.display().to_string();
        if let Some(filtered) = self.classify(&test_id) {
            return ReftestResult {
                pair_test_id: test_id,
                outcome: filtered,
                mismatched_pixels: 0,
                total_pixels: u64::from(config.width) * u64::from(config.height),
            };
        }
        match crate::reftest::run_pair(pair, config) {
            Ok(r) => r,
            Err(e) => ReftestResult {
                pair_test_id: test_id,
                outcome: TestOutcome::Fail(format!("reftest error: {e}")),
                mismatched_pixels: 0,
                total_pixels: u64::from(config.width) * u64::from(config.height),
            },
        }
    }

    /// Run a reftest pair via both engines and return the oracle delta.
    ///
    /// Respects expectations filtering on the raikiri outcome; the blitz
    /// oracle is still computed for informational reporting.
    pub fn run_pair_with_oracle(
        &self,
        pair: &ReftestPair,
        config: ReftestConfig,
    ) -> (ReftestResult, OracleDiff) {
        let raikiri_result = self.run_reftest_pair(pair, config);
        // For filtered tests, fabricate a matching blitz outcome so the diff
        // doesn't report a false blitz-only signal.
        let blitz_outcome = if matches!(
            raikiri_result.outcome,
            TestOutcome::Skip(_) | TestOutcome::Quarantined
        ) {
            raikiri_result.outcome.clone()
        } else {
            // Compute blitz outcome for same pair
            match render_pair_with_engine(pair, config, Engine::Blitz) {
                Ok(outcome) => outcome,
                Err(e) => TestOutcome::Fail(format!("blitz error: {e}")),
            }
        };
        let diff = BlitzOracle::diff(
            &raikiri_result.outcome,
            &blitz_outcome,
            &raikiri_result.pair_test_id,
        );
        (raikiri_result, diff)
    }

    /// Run all pairs under `wpt_root` and return per-pair results plus oracle deltas.
    ///
    /// Convenience for the nightly T3 sweep. Walks `wpt_root` via
    /// `crate::reftest::discover_all_pairs`.
    pub fn run_all_under(
        &self,
        wpt_root: &Path,
        config: ReftestConfig,
    ) -> Vec<(ReftestResult, OracleDiff)> {
        let pairs = crate::reftest::discover_all_pairs(wpt_root);
        pairs
            .iter()
            .map(|p| self.run_pair_with_oracle(p, config))
            .collect()
    }

    /// Compare raikiri vs blitz rendering of a single HTML string (non-reftest).
    ///
    /// Useful for ad-hoc oracle spot-checks without a reference file.
    pub fn compare_single_html_oracle(
        &self,
        html: &str,
        config: ReftestConfig,
    ) -> Result<(crate::reftest::ImageDiff, OracleDiff), String> {
        let raikiri_img =
            render_raikiri(html, config.width, config.height).map_err(|e| e.to_string())?;
        let blitz_img =
            render_blitz(html, config.width, config.height).map_err(|e| e.to_string())?;
        let diff = compare_images(&raikiri_img, &blitz_img, config.tolerance);
        let raikiri = TestOutcome::Pass;
        let blitz = if diff.matched {
            TestOutcome::Pass
        } else {
            TestOutcome::Fail(format!(
                "oracle pixel mismatch {} / {}",
                diff.mismatched_pixels, diff.total_pixels
            ))
        };
        let oracle = BlitzOracle::diff(&raikiri, &blitz, "<inline>");
        Ok((diff, oracle))
    }
}

#[derive(Debug, Clone, Copy)]
enum Engine {
    Blitz,
}

fn render_pair_with_engine(
    pair: &ReftestPair,
    config: ReftestConfig,
    engine: Engine,
) -> Result<TestOutcome, String> {
    let test_html = std::fs::read_to_string(&pair.test).map_err(|e| e.to_string())?;
    let ref_html = std::fs::read_to_string(&pair.reference).map_err(|e| e.to_string())?;
    let (test_img, ref_img) = match engine {
        Engine::Blitz => (
            render_blitz(&test_html, config.width, config.height).map_err(|e| e.to_string())?,
            render_blitz(&ref_html, config.width, config.height).map_err(|e| e.to_string())?,
        ),
    };
    let d = compare_images(&test_img, &ref_img, config.tolerance);
    let pass = match pair.kind {
        crate::reftest::ReftestKind::Match => d.matched,
        crate::reftest::ReftestKind::Mismatch => !d.matched,
    };
    if pass {
        Ok(TestOutcome::Pass)
    } else {
        Ok(TestOutcome::Fail(format!(
            "reftest {:?} failed: {} mismatched",
            pair.kind, d.mismatched_pixels
        )))
    }
}

fn test_id_matches_pattern(test_id: &str, pat: &str) -> bool {
    if pat.ends_with('/') {
        test_id.starts_with(pat)
    } else {
        test_id == pat
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
    use crate::expectations::{Baseline, Deprecated, KnownIssues, Quarantine, TrackedWpt};

    fn empty_set() -> ExpectationSet {
        ExpectationSet {
            tracked: TrackedWpt { entries: vec![] },
            known_issues: KnownIssues { entries: vec![] },
            baseline: Baseline {
                entries: std::collections::HashSet::new(),
            },
            quarantine: Quarantine { entries: vec![] },
            deprecated: Deprecated {
                entries: std::collections::HashSet::new(),
            },
        }
    }

    #[test]
    fn test_outcome_variants_are_constructible() {
        assert!(matches!(TestOutcome::Pass, TestOutcome::Pass));
        assert!(matches!(
            TestOutcome::Fail("boom".to_owned()),
            TestOutcome::Fail(_)
        ));
        assert!(matches!(
            TestOutcome::Skip("non-goal".to_owned()),
            TestOutcome::Skip(_)
        ));
        assert!(matches!(TestOutcome::Quarantined, TestOutcome::Quarantined));
    }

    #[test]
    fn classify_deprecated_takes_precedence() {
        let mut set = empty_set();
        set.deprecated.entries.insert("css/foo.html".to_owned());
        let runner = WptRunner::new(set);
        assert!(matches!(
            runner.classify("css/foo.html"),
            Some(TestOutcome::Skip(_))
        ));
    }

    #[test]
    fn classify_unknown_returns_none() {
        let runner = WptRunner::new(empty_set());
        assert!(runner.classify("css/bar.html").is_none());
    }

    #[test]
    fn run_test_filtered_returns_skip() {
        let mut set = empty_set();
        set.deprecated.entries.insert("a.html".to_owned());
        let runner = WptRunner::new(set);
        let exec = runner.run_test("a.html");
        assert!(matches!(exec.outcome, TestOutcome::Skip(_)));
    }

    #[test]
    fn tolerance_constants_distinct() {
        assert_ne!(Tolerance::EXACT, Tolerance::TIER2);
        assert_ne!(Tolerance::TIER2, Tolerance::TIER3);
    }
}
