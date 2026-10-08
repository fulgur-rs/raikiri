use super::*;
use raikiri_style::{StyleDom, StyleElement, StyleNode, StyleNodeId};

#[test]
fn missing_script_does_not_prevent_later_scripts_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let html = r#"<!DOCTYPE html><body id=probe><script>
      window.addEventListener('load', () => document.body.setAttribute('data-load', 'yes'));
    </script><script src=missing.js></script><script>
      document.body.setAttribute('data-after', 'yes');
    </script>"#;
    let prepared = prepare(
        html,
        &dir.path().join("test.html"),
        "",
        ReftestConfig::default(),
    )
    .expect("an ordinary script fetch failure must allow later scripts and load");
    assert!(prepared.html.contains("data-after=\"yes\""));
    assert!(prepared.html.contains("data-load=\"yes\""));
}

#[test]
fn missing_script_error_event_can_release_wait_and_paint_exact_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let test = dir.path().join("test.html");
    let reference = dir.path().join("ref.html");
    let html = r#"<!DOCTYPE html><html class=reftest-wait><body style="margin:0;background:red">
      <script>document.addEventListener('error', event => {
        if (event.target.tagName === 'SCRIPT' && event.isTrusted) {
          document.body.style.background = 'green';
          document.documentElement.removeAttribute('class');
        }
      }, true);</script><script src=missing.js></script>
    "#;
    std::fs::write(&test, html).unwrap();
    std::fs::write(
        &reference,
        "<!DOCTYPE html><body style='margin:0;background:green'>",
    )
    .unwrap();
    let config = ReftestConfig {
        width: 40,
        height: 40,
        tolerance: Tolerance::EXACT,
        require_inline_fonts: false,
    };
    let prepared = prepare(html, &test, "", config).unwrap();
    assert!(!prepared.html.contains("reftest-wait"));
    let pair = ReftestPair {
        test,
        reference,
        kind: ReftestKind::Match,
        reference_suffix: String::new(),
    };
    let result = run_pair(&pair, config).unwrap();
    assert_eq!(result.mismatched_pixels, 0);
    assert!(matches!(result.outcome, TestOutcome::Pass));
}

#[test]
fn missing_script_does_not_release_wait_or_hide_runtime_errors() {
    let dir = tempfile::tempdir().unwrap();
    for (html, expected) in [
        (
            "<!DOCTYPE html><html class=reftest-wait><script src=missing.js></script>",
            "still has reftest-wait",
        ),
        (
            "<!DOCTYPE html><script src=missing.js></script><script>throw new Error('after-fetch');</script>",
            "after-fetch",
        ),
    ] {
        let error = prepare(
            html,
            &dir.path().join("test.html"),
            "",
            ReftestConfig::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error:?}");
    }
}

#[test]
fn script_fetch_failure_does_not_hide_abort_host_or_uncaught_failures() {
    use raikiri_js::runtime::{Abort, RunReport};
    for (mut report, expected) in [
        (
            RunReport {
                aborted: Some(Abort::Tasks),
                ..Default::default()
            },
            "Tasks",
        ),
        (
            RunReport {
                host_failures: vec!["layout failure".into()],
                ..Default::default()
            },
            "layout failure",
        ),
        (
            RunReport {
                uncaught_errors: vec!["uncaught boom".into()],
                ..Default::default()
            },
            "uncaught boom",
        ),
    ] {
        report.fetch_errors.push("missing script".into());
        let error = serialize(&raikiri_dom::Document::new(), &report).unwrap_err();
        assert!(error.to_string().contains(expected), "{error:?}");
    }
}

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

#[test]
fn prepare_carries_sampled_animation_styles_outside_serialized_html() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("animated.html");
    std::fs::write(&path, "").unwrap();
    let html = "<!DOCTYPE html><html><head></head><body>\
        <div id=target style='font-size:16px'>WORD WORD</div>\
        <script>const target = document.querySelector('#target');\
        const animation = target.animate({ fontSize: ['0px', '40px'] }, 40);\
        animation.pause(); animation.currentTime = 20;</script>\
        </body></html>";
    let prepared = prepare(html, &path, "", crate::reftest::ReftestConfig::default())
        .expect("animated document should prepare");

    assert!(prepared.html.contains("font-size:16px"));
    assert!(
        !prepared.html.contains("font-size: 20px"),
        "sampled declarations must not be serialized as authored style"
    );
    assert_eq!(prepared.animation_styles.len(), 1);
    assert_eq!(
        prepared.animation_styles[0].declarations,
        "font-size: 20px;"
    );
}

#[test]
fn animation_style_sidecar_survives_adjacent_text_node_serialization() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("animated.html");
    std::fs::write(&path, "").unwrap();
    let html = "<!DOCTYPE html><html><head></head><body><div id=target style='font-size:16px'>WORD</div><script>const target = document.querySelector('#target'); target.parentNode.insertBefore(document.createTextNode('a'), target); target.parentNode.insertBefore(document.createTextNode('b'), target); const animation = target.animate({ fontSize: ['0px', '40px'] }, 40); animation.pause(); animation.currentTime = 20;</script></body></html>";
    let prepared = prepare(html, &path, "", crate::reftest::ReftestConfig::default())
        .expect("animated document should prepare");

    let parsed = raikiri_html::parse(
        prepared.html.as_bytes(),
        &raikiri::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let mut document = parsed.dom;
    super::super::restore_animation_styles(&mut document, &prepared.animation_styles).unwrap();
    let target = (0..document.node_count())
        .find(|id| document.element_attribute(*id, "id") == Some("target"))
        .expect("target element should survive serialization");
    let animation_style = document
        .node(StyleNodeId::new(target as u64))
        .and_then(|node| {
            node.as_element()
                .and_then(|element| element.animation_style_source().map(str::to_owned))
        });

    assert_eq!(animation_style.as_deref(), Some("font-size: 20px;"));
}
