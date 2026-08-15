//! WPT reftest pair + result types (spec §12.9). Stubs; execution is future work.

use std::path::PathBuf;

use crate::runner::TestOutcome;

/// Whether a reftest requires visual match or explicit mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReftestKind {
    /// Test and reference must rasterize to the same pixels.
    Match,
    /// Test and reference must rasterize to *different* pixels.
    Mismatch,
}

/// One (test, reference) pair discovered under the WPT submodule.
#[non_exhaustive]
pub struct ReftestPair {
    /// Path to the test file.
    pub test: PathBuf,
    /// Path to the reference file.
    pub reference: PathBuf,
    /// Whether the pair is a match or mismatch reftest.
    pub kind: ReftestKind,
}

/// Result of executing a `ReftestPair`.
#[non_exhaustive]
pub struct ReftestResult {
    /// Identifier of the test half of the pair.
    pub pair_test_id: String,
    /// Outcome of executing the pair.
    pub outcome: TestOutcome,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reftest_kind_roundtrip() {
        let m = ReftestKind::Match;
        let mm = ReftestKind::Mismatch;
        assert_ne!(m, mm);
    }
}
