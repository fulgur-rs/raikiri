//! Focused CSS Selectors visual checks at the exact project viewport.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// A representative structural-selector slice matches its references exactly.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn selector_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let dir = root.join("css/selectors");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for name in [
        "case-insensitive-parent.html",
        "child-indexed-no-parent.html",
        "dir-selector-ltr-001.html",
        "dir-selector-ltr-002.html",
        "dir-selector-ltr-003.html",
        "dir-selector-rtl-001.html",
        "nth-child-and-nth-last-child.html",
        "nth-child-of-attribute.html",
        "nth-child-of-classname-002.html",
        "nth-child-of-classname.html",
        "nth-child-of-complex-selector-many-children-2.html",
        "nth-child-of-complex-selector-many-children.html",
        "nth-child-of-complex-selector.html",
        "nth-child-of-compound-selector.html",
        "nth-child-of-has.html",
        "nth-child-of-no-space-after-of.html",
        "nth-child-of-not.html",
        "nth-child-of-tagname.html",
        "nth-child-of-universal-selector.html",
        "nth-child-specificity-1.html",
        "nth-child-specificity-2.html",
        "nth-child-specificity-3.html",
        "nth-child-specificity-4.html",
    ] {
        let test = dir.join(name);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {}: {error}", test.display()));
        assert_eq!(pairs.len(), 1, "expected one pair for {}", test.display());
        let result = run_pair(&pairs[0], config)
            .unwrap_or_else(|error| panic!("run {}: {error}", test.display()));
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{}: {:?} ({} mismatched pixels)",
            test.display(),
            result.outcome,
            result.mismatched_pixels
        );
    }
}
