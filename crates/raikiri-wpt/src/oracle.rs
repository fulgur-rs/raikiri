//! Blitz baseline oracle (spec §12.4, T3 informational).
//!
//! Renders HTML via blitz and computes the delta against raikiri outcomes.
//! `BlitzDocument` alias and `BlitzOracle::parse_html` are retained for
//! backwards compat; new `render` and `diff_outcomes` helpers power the
//! reftest runner's oracle comparison.

/// Type alias so downstream code can name the oracle's document type
/// without a direct `blitz_dom::*` import.
pub type BlitzDocument = blitz_dom::BaseDocument;

use crate::reftest::{ReftestKind, RenderedImage, compare_images, render_blitz, render_raikiri};
use crate::runner::{TestOutcome, Tolerance};

/// Runs blitz-html / blitz-dom parses side-by-side with raikiri to record
/// pass/fail deltas.
#[non_exhaustive]
pub struct BlitzOracle;

/// The recorded delta between one raikiri outcome and the corresponding
/// blitz outcome for a single test.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OracleDiff {
    /// Test identifier (path or WPT test id).
    pub test_id: String,
    /// Outcome under raikiri.
    pub raikiri: TestOutcome,
    /// Outcome under blitz.
    pub blitz: TestOutcome,
    /// True when blitz Passes but raikiri does not (blitz-only-pass, T3 signal).
    pub blitz_only_pass: bool,
    /// True when raikiri Passes but blitz does not (raikiri-ahead).
    pub raikiri_only_pass: bool,
}

impl OracleDiff {
    /// Whether the two oracles agree.
    pub fn agrees(&self) -> bool {
        self.raikiri == self.blitz
    }
}

impl BlitzOracle {
    /// Parse `html` via blitz-html and return the underlying document.
    ///
    /// Resolves viewport to 800×600 and calls `resolve(0.0)` so the
    /// returned document is style-and-layout resolved.
    pub fn parse_html(html: &str) -> BlitzDocument {
        use blitz_dom::DocumentConfig;
        use blitz_traits::shell::Viewport;
        use blitz_html::HtmlDocument;
        use blitz_traits::shell::ColorScheme;

        let viewport = Viewport {
            window_size: (crate::reftest::DEFAULT_REFTTEST_WIDTH, crate::reftest::DEFAULT_REFTTEST_HEIGHT),
            hidpi_scale: 1.0,
            zoom: 1.0,
            color_scheme: ColorScheme::Light,
        };
        let config = DocumentConfig {
            viewport: Some(viewport),
            ..Default::default()
        };
        let mut doc = HtmlDocument::from_html(html, config);
        doc.resolve(0.0);
        doc.into_inner()
    }

    /// Compute the informational delta between a raikiri outcome and a
    /// blitz outcome.
    pub fn diff(raikiri: &TestOutcome, blitz: &TestOutcome, test_id: &str) -> OracleDiff {
        let blitz_only_pass = matches!(blitz, TestOutcome::Pass) && !matches!(raikiri, TestOutcome::Pass);
        let raikiri_only_pass = matches!(raikiri, TestOutcome::Pass) && !matches!(blitz, TestOutcome::Pass);
        OracleDiff {
            test_id: test_id.to_owned(),
            raikiri: raikiri.clone(),
            blitz: blitz.clone(),
            blitz_only_pass,
            raikiri_only_pass,
        }
    }

    /// Render `html` via blitz at `width`×`height` and return the image.
    ///
    /// Convenience wrapper around `crate::reftest::render_blitz`.
    pub fn render_image(html: &str, width: u32, height: u32) -> Result<RenderedImage, String> {
        render_blitz(html, width, height).map_err(|e| e.to_string())
    }

    /// Render `html` via raikiri at `width`×`height` and return the image.
    pub fn render_raikiri_image(html: &str, width: u32, height: u32) -> Result<RenderedImage, String> {
        render_raikiri(html, width, height).map_err(|e| e.to_string())
    }

    /// Compare blitz vs raikiri renderings of the same `html` under `tolerance`.
    ///
    /// Returns `(blitz_image, raikiri_image, diff)` where `diff.matched`
    /// indicates whether the two engines agree pixel-wise. This is the
    /// per-file oracle pixel comparison (distinct from per-pair reftest
    /// match/mismatch semantics).
    pub fn compare_engines_for_same_html(
        html: &str,
        width: u32,
        height: u32,
        tolerance: Tolerance,
    ) -> Result<(RenderedImage, RenderedImage, crate::reftest::ImageDiff), String> {
        let blitz_img = Self::render_image(html, width, height)?;
        let raikiri_img = Self::render_raikiri_image(html, width, height)?;
        let diff = compare_images(&blitz_img, &raikiri_img, tolerance);
        Ok((blitz_img, raikiri_img, diff))
    }

    /// Compute reftest outcomes for the pair `(test_html, ref_html, kind)` under
    /// both engines and return the delta.
    ///
    /// This is the reftest-aware oracle: each engine's pass/fail is
    /// determined by `kind` (Match => images must match, Mismatch => must differ).
    pub fn diff_reftest_pair(
        test_html: &str,
        ref_html: &str,
        kind: ReftestKind,
        width: u32,
        height: u32,
        tolerance: Tolerance,
        test_id: &str,
    ) -> Result<OracleDiff, String> {
        let raikiri_test = Self::render_raikiri_image(test_html, width, height)?;
        let raikiri_ref = Self::render_raikiri_image(ref_html, width, height)?;
        let blitz_test = Self::render_image(test_html, width, height)?;
        let blitz_ref = Self::render_image(ref_html, width, height)?;

        let raikiri_diff = compare_images(&raikiri_test, &raikiri_ref, tolerance);
        let blitz_diff = compare_images(&blitz_test, &blitz_ref, tolerance);

        let raikiri_pass = match kind {
            ReftestKind::Match => raikiri_diff.matched,
            ReftestKind::Mismatch => !raikiri_diff.matched,
        };
        let blitz_pass = match kind {
            ReftestKind::Match => blitz_diff.matched,
            ReftestKind::Mismatch => !blitz_diff.matched,
        };
        let raikiri_outcome = if raikiri_pass { TestOutcome::Pass } else { TestOutcome::Fail(format!("raikiri reftest {:?} failed ({} mismatched)", kind, raikiri_diff.mismatched_pixels)) };
        let blitz_outcome = if blitz_pass { TestOutcome::Pass } else { TestOutcome::Fail(format!("blitz reftest {:?} failed ({} mismatched)", kind, blitz_diff.mismatched_pixels)) };
        Ok(Self::diff(&raikiri_outcome, &blitz_outcome, test_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::TestOutcome;

    /// Confirms the blitz-html / blitz-dom dev-deps actually link.
    #[test]
    fn blitz_dev_dep_links_and_parses_trivial_html() {
        use blitz_dom::DocumentConfig;
        use blitz_html::HtmlDocument;

        let doc = HtmlDocument::from_html("<p>hello</p>", DocumentConfig::default());
        let _base: BlitzDocument = doc.into_inner();
    }

    #[test]
    fn parse_html_via_oracle_does_not_panic() {
        let _doc = BlitzOracle::parse_html("<p>hello</p>");
    }

    #[test]
    fn diff_marks_blitz_only_pass() {
        let d = BlitzOracle::diff(&TestOutcome::Fail("x".into()), &TestOutcome::Pass, "foo.html");
        assert!(d.blitz_only_pass);
        assert!(!d.raikiri_only_pass);
        assert!(!d.agrees());
    }

    #[test]
    fn diff_agrees_on_both_pass() {
        let d = BlitzOracle::diff(&TestOutcome::Pass, &TestOutcome::Pass, "foo.html");
        assert!(d.agrees());
        assert!(!d.blitz_only_pass);
    }

    #[test]
    fn compare_engines_same_html_runs() {
        let html = "<html><body><p>hi</p></body></html>";
        let res = BlitzOracle::compare_engines_for_same_html(html, 200, 100, Tolerance::EXACT);
        assert!(res.is_ok(), "compare_engines failed: {:?}", res.err());
        let (_, _, diff) = res.unwrap();
        // engines may differ; just ensure diff is computed
        let _ = diff.mismatched_pixels;
    }

    #[test]
    fn diff_reftest_pair_both_engines_agree_on_identical_match() {
        let html = "<html><body><p>same</p></body></html>";
        let d = BlitzOracle::diff_reftest_pair(html, html, crate::reftest::ReftestKind::Match, 200, 100, Tolerance::EXACT, "t.html").unwrap();
        // identical => both Pass => agree
        assert_eq!(d.raikiri, TestOutcome::Pass);
        assert_eq!(d.blitz, TestOutcome::Pass);
        assert!(d.agrees());
    }
}
