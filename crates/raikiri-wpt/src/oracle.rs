//! Blitz baseline oracle (spec §12.4, T3 informational).
//!
//! In M1 the module provides type stubs plus a smoke test that exercises
//! the blitz-html / blitz-dom dev-deps end-to-end (parse `<p>hello</p>`).
//! Actual raikiri-vs-blitz diff aggregation lands in M3.

/// Type alias so downstream code (M3) can name the oracle's document type
/// without a direct `blitz_dom::*` import.
pub type BlitzDocument = blitz_dom::BaseDocument;

/// Runs blitz-html / blitz-dom parses side-by-side with raikiri to record
/// pass/fail deltas. **M1 stub**: no live diffing.
#[non_exhaustive]
pub struct BlitzOracle;

/// The recorded delta between one raikiri outcome and the corresponding
/// blitz outcome for a single test. **M1 stub**: field set is
/// intentionally empty until M3 populates it.
#[non_exhaustive]
pub struct OracleDiff {}

impl BlitzOracle {
    /// Parse `html` via blitz-html and return the underlying document.
    ///
    /// **M1 stub**: panics via `todo!()`. The smoke test below drives the
    /// blitz API directly to confirm the dev-dep links.
    pub fn parse_html(_html: &str) -> BlitzDocument {
        todo!("M3: wire blitz_html::HtmlDocument::from_html and return into_inner()")
    }

    /// Compute the informational delta between a raikiri outcome and a
    /// blitz outcome. **M1 stub**.
    pub fn diff(_raikiri: &(), _blitz: &()) -> OracleDiff {
        todo!("M3: compare TestOutcome tuples and record blitz-only-pass entries")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Confirms the blitz-html / blitz-dom dev-deps actually link in M1.
    /// Execution semantics land in M3.
    #[test]
    fn blitz_dev_dep_links_and_parses_trivial_html() {
        use blitz_dom::DocumentConfig;
        use blitz_html::HtmlDocument;

        let doc = HtmlDocument::from_html("<p>hello</p>", DocumentConfig::default());
        let _base: BlitzDocument = doc.into_inner();
    }
}
