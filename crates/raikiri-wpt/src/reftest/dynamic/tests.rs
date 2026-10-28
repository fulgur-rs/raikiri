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
        ..ReftestConfig::default()
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
        ..ReftestConfig::default()
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
        ..ReftestConfig::default()
    };
    let prepared = prepare(html, &path, "", config).expect("reference prepare should succeed");
    assert!(
        prepared.html.contains("4000"),
        "reference scrollY should be recorded, got: {}",
        prepared.html
    );
}

#[test]
fn prepare_carries_sampled_style_sources_outside_serialized_html() {
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
    assert_eq!(prepared.style_sources.len(), 1);
    assert_eq!(prepared.style_sources[0].declarations, "font-size: 20px;");
}

#[test]
fn animation_style_sidecar_survives_adjacent_text_node_serialization() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("animated.html");
    std::fs::write(&path, "").unwrap();
    let html = "<!DOCTYPE html><html><head></head><body><div id=target style='font-size:16px'>WORD</div><script>const target = document.querySelector('#target'); target.parentNode.insertBefore(document.createTextNode('a'), target); target.parentNode.insertBefore(document.createTextNode('b'), target); const animation = target.animate({ fontSize: ['0px', '40px'] }, 40); animation.pause(); animation.currentTime = 20;</script></body></html>";
    let prepared = prepare(html, &path, "", crate::reftest::ReftestConfig::default())
        .expect("animated document should prepare");

    let parsed =
        raikiri_html::parse(prepared.html.as_bytes(), &raikiri::ParseOptions::new()).unwrap();
    let mut document = parsed.dom;
    super::super::restore_style_sources(&mut document, &prepared.style_sources).unwrap();
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

#[test]
fn focus_sidecar_survives_adjacent_text_node_serialization() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("focused.html");
    std::fs::write(&path, "").unwrap();
    let html = "<!DOCTYPE html><html><head></head><body><div id=target tabindex=0>WORD</div><div id=hidden tabindex=0 style='display:none'></div><script>const target = document.querySelector('#target'); target.parentNode.insertBefore(document.createTextNode('a'), target); target.parentNode.insertBefore(document.createTextNode('b'), target); document.querySelector('#hidden').focus(); const missed = document.activeElement === document.body; if (missed) target.focus({focusVisible: false});</script></body></html>";
    let prepared = prepare(html, &path, "", crate::reftest::ReftestConfig::default())
        .expect("focused document should prepare");
    // `<head>` and `<body>` under `<html>`, then the first element in body.
    let focused = prepared
        .focused_path
        .as_ref()
        .expect("focus should be recorded");
    assert_eq!(focused.path, [0, 1, 0]);
    assert!(!focused.visible);

    let parsed =
        raikiri_html::parse(prepared.html.as_bytes(), &raikiri::ParseOptions::new()).unwrap();
    let mut document = parsed.dom;
    super::super::restore_focus(&mut document, focused);
    let target = (0..document.node_count())
        .find(|id| document.element_attribute(*id, "id") == Some("target"))
        .expect("target element should survive serialization");

    assert_eq!(document.focused_element(), Some(target));
    assert_eq!(document.focus_visible_element(), None);
}

#[test]
fn focus_sidecar_is_dropped_when_reparsing_restructures_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("focused-row.html");
    std::fs::write(&path, "").unwrap();
    // A script-built `<tr>` directly under `<table>` reparses inside an
    // implied `<tbody>`, so its element-child path reaches the `<tbody>`.
    let html = "<!DOCTYPE html><html><head></head><body><table></table><script>const row = document.createElement('tr'); row.setAttribute('tabindex', '0'); document.querySelector('table').appendChild(row); row.focus();</script></body></html>";
    let prepared = prepare(html, &path, "", crate::reftest::ReftestConfig::default())
        .expect("focused document should prepare");
    let focused = prepared
        .focused_path
        .as_ref()
        .expect("focus should be recorded");
    assert_eq!(focused.path, [0, 1, 0, 0]);
    let names = focused.tags.iter().map(|(_, tag)| tag.as_str());
    assert!(names.eq(["html", "body", "table", "tr"]));

    let parsed =
        raikiri_html::parse(prepared.html.as_bytes(), &raikiri::ParseOptions::new()).unwrap();
    let mut document = parsed.dom;
    super::super::restore_focus(&mut document, focused);

    assert_eq!(document.focused_element(), None);
}

#[test]
fn focus_sidecar_is_dropped_when_reparsing_changes_a_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("focused-svg.html");
    std::fs::write(&path, "").unwrap();
    // `createElement('svg')` makes an HTML element; its markup reparses
    // as an SVG element at the same path.
    let html = "<!DOCTYPE html><html><head></head><body><script>const svg = document.createElement('svg'); svg.setAttribute('tabindex', '0'); document.body.insertBefore(svg, document.body.firstChild); svg.focus();</script></body></html>";
    let prepared = prepare(html, &path, "", crate::reftest::ReftestConfig::default())
        .expect("focused document should prepare");
    let focused = prepared
        .focused_path
        .as_ref()
        .expect("focus should be recorded");
    assert_eq!(focused.path, [0, 1, 0]);

    let parsed =
        raikiri_html::parse(prepared.html.as_bytes(), &raikiri::ParseOptions::new()).unwrap();
    let mut document = parsed.dom;
    super::super::restore_focus(&mut document, focused);

    assert_eq!(document.focused_element(), None);
}

#[test]
fn cssom_sources_survive_preparation_without_entering_dom_markup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cssom.html");
    std::fs::write(&path, "").unwrap();
    let html = r#"<!DOCTYPE html><html><head><style id=s>p { color: red }</style></head><body>
        <script>var el=document.querySelector('#s'), text=el.firstChild;
        el.sheet.insertRule('p::before { content: "</style><div>" }', 1);
        if(text!==el.firstChild || text.data!=='p { color: red }') throw Error('DOM changed');
        </script></body></html>"#;
    let prepared = prepare(html, &path, "", ReftestConfig::default()).unwrap();
    assert!(
        prepared
            .html
            .contains("<style id=\"s\">p { color: red }</style>")
    );
    let source = prepared
        .style_sources
        .iter()
        .find(|style| style.kind == StyleSourceKind::Stylesheet)
        .unwrap();
    assert!(source.declarations.contains("</style><div>"));
    let options = raikiri::ParseOptions::new();
    let mut parsed = raikiri_html::parse(prepared.html.as_bytes(), &options).unwrap();
    super::super::restore_style_sources(&mut parsed.dom, &prepared.style_sources).unwrap();
    raikiri_html::refresh_stylesheets(&mut parsed, &options);
    assert!(parsed.stylesheet_sources.iter().any(|sheet| {
        sheet
            .parts
            .iter()
            .any(|part| part.source.contains("</style><div>"))
    }));
    assert_eq!(
        parsed.dom.element_text_content(3).as_deref(),
        Some("p { color: red }")
    );
}

#[test]
fn cssom_page_rule_metadata_controls_reference_margin_mirroring() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("page-margin.html");
    std::fs::write(&path, "").unwrap();
    for (margin, edit, mirror) in [
        (
            "margin:0",
            "s.cssRules[0].style.removeProperty('margin')",
            true,
        ),
        ("", "s.cssRules[0].style.margin='0px'", false),
        ("", "s.deleteRule(0)", false),
    ] {
        let html = format!(
            "<!DOCTYPE html><head><style>@page{{size:300px 300px;{margin}}}</style></head><body><script>var s=document.querySelector('style').sheet;{edit};</script></body>"
        );
        let prepared = prepare(&html, &path, "", ReftestConfig::default()).unwrap();
        assert_eq!(
            super::super::should_mirror_default_page_margin(&prepared.page_rules, &[]),
            mirror
        );
    }
}
