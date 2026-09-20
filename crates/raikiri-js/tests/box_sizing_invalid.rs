//! Feasibility check: run a real, unmodified WPT parsing test file (fetched
//! by `scripts/wpt/fetch.sh`) through the spike harness end to end.

use std::path::Path;

fn read_fixture(rel: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&root)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", root.display()))
}

/// `box-sizing-invalid.html` has exactly one `<script>` block with no `src`
/// attribute; every other `<script>` tag loads `testharness.js` et al.
fn extract_inline_script(html: &str) -> String {
    let start_tag = "<script>\n";
    let start = html.find(start_tag).expect("inline <script> tag") + start_tag.len();
    let end = html[start..]
        .find("</script>")
        .expect("closing </script> tag");
    html[start..start + end].to_string()
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn box_sizing_invalid_all_seven_reject() {
    let parsing_testcommon = read_fixture("target/wpt/css/support/parsing-testcommon.js");
    let fixture_html = read_fixture("target/wpt/css/css-sizing/parsing/box-sizing-invalid.html");
    let script = extract_inline_script(&fixture_html);

    let outcomes = raikiri_js::run_invalid_value_script(&parsing_testcommon, &script)
        .expect("harness should produce a trustworthy result");

    assert_eq!(
        outcomes.len(),
        7,
        "box-sizing-invalid.html declares 7 test_invalid_value calls, got: {outcomes:?}"
    );
    for outcome in &outcomes {
        assert!(
            outcome.passed,
            "expected PASS (value correctly rejected) for {:?}, got FAIL: {}",
            outcome.name, outcome.message
        );
    }
}
