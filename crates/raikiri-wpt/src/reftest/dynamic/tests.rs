use super::*;

#[test]
fn waiting_returns_false_without_html_element() {
    let document = raikiri_dom::Document::new();
    assert!(
        !waiting(&document),
        "a fresh Document has no html element, so no reftest-wait can be present"
    );
}

#[test]
fn prepare_serves_shared_helper_and_records_genuine_scroll() {
    // Mirrors `fixed-z-index-blend.html`'s live shape without copying its
    // fixture: a `reftest-wait` document loads the shared helper as a
    // root-relative script, scrolls the viewport, records the observable
    // offset into the DOM, and releases the wait. Success proves the helper
    // is genuinely available (not a fetch error) and scrolling is genuine
    // (a no-op stub would leave `scrollY` at zero, failing the content
    // assertion below).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.html");
    std::fs::write(&path, "").unwrap();
    let html = "<!DOCTYPE html><html class=reftest-wait><head></head>\
        <body><div id=probe></div>\
        <script src=\"/common/reftest-wait.js\"></script>\
        <script>window.scrollBy(0, 4000); \
        document.getElementById('probe').textContent = String(window.scrollY); \
        takeScreenshot();</script></body></html>";
    let config = crate::reftest::ReftestConfig {
        width: 800,
        height: 600,
        tolerance: crate::runner::Tolerance::EXACT,
        require_inline_fonts: false,
    };
    let prepared = prepare(html, &path, "", config).expect("live prepare should succeed");
    assert!(
        prepared.html.contains("4000"),
        "scrollY should be recorded into the DOM, got: {}",
        prepared.html
    );
    assert!(
        !prepared.html.contains("class=\"reftest-wait\"")
            && !prepared.html.contains("class=reftest-wait"),
        "helper should have released the wait class, got: {}",
        prepared.html
    );
}

#[test]
fn prepare_runs_reference_side_scroll_without_wait() {
    // The reference half of the motivating pair also calls `window.scrollBy`
    // (no `reftest-wait`): it must run without script errors through the same
    // live path, keeping viewport/print coherence (both sides scroll the live
    // viewport and both ignore scroll in print).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ref.html");
    std::fs::write(&path, "").unwrap();
    let html = "<!DOCTYPE html><html><head></head>\
        <body><div id=probe></div>\
        <script>window.scrollBy(0, 4000); \
        document.getElementById('probe').textContent = String(window.scrollY);</script></body></html>";
    let config = crate::reftest::ReftestConfig {
        width: 800,
        height: 600,
        tolerance: crate::runner::Tolerance::EXACT,
        require_inline_fonts: false,
    };
    let prepared = prepare(html, &path, "", config).expect("reference prepare should succeed");
    assert!(
        prepared.html.contains("4000"),
        "reference scrollY should be recorded, got: {}",
        prepared.html
    );
}
