use std::collections::BTreeMap;

use super::*;
use crate::dom::{DomNodeId, DomRect, ElementGeometry};

#[derive(Default)]
struct TestDom {
    by_id: BTreeMap<String, (DomNodeId, ElementGeometry)>,
    body_html: String,
    attributes: BTreeMap<(DomNodeId, String), String>,
    styles: BTreeMap<(DomNodeId, String), String>,
    fail_geometry: bool,
}

impl TestDom {
    fn with_element(mut self, id: &str, node: DomNodeId, geometry: ElementGeometry) -> Self {
        self.by_id.insert(id.to_owned(), (node, geometry));
        self
    }

    fn with_attribute(mut self, node: DomNodeId, name: &str, value: &str) -> Self {
        self.attributes
            .insert((node, name.to_owned()), value.to_owned());
        self
    }

    fn with_failing_geometry(mut self) -> Self {
        self.fail_geometry = true;
        self
    }

    fn geometry(&self, node: DomNodeId) -> Option<ElementGeometry> {
        self.by_id
            .values()
            .find_map(|(candidate, geometry)| (*candidate == node).then_some(*geometry))
    }
}

fn test_geometry(
    offset_height: f64,
    left: f64,
    top: f64,
    width: f64,
    height: f64,
) -> ElementGeometry {
    ElementGeometry {
        offset_height,
        bounding_client_rect: DomRect {
            left,
            top,
            right: left + width,
            bottom: top + height,
            width,
            height,
        },
    }
}

impl DomBackend for TestDom {
    fn get_element_by_id(&mut self, id: &str) -> Result<Option<DomNodeId>, String> {
        Ok(self.by_id.get(id).map(|(node, _)| *node))
    }

    fn query_selector(&mut self, selector: &str) -> Result<Option<DomNodeId>, String> {
        Ok((selector.eq_ignore_ascii_case("body")).then_some(0))
    }

    fn parent_node(&mut self, node: DomNodeId) -> Result<Option<DomNodeId>, String> {
        Ok((node != 0).then_some(0))
    }

    fn offset_height(&mut self, node: DomNodeId) -> Result<f64, String> {
        if self.fail_geometry {
            return Err("test geometry backend failure".into());
        }
        self.geometry(node)
            .map(|geometry| geometry.offset_height)
            .ok_or_else(|| format!("no layout for node {node}"))
    }

    fn bounding_client_rect(&mut self, node: DomNodeId) -> Result<DomRect, String> {
        if self.fail_geometry {
            return Err("test geometry backend failure".into());
        }
        let geometry = self
            .geometry(node)
            .ok_or_else(|| format!("no layout for node {node}"))?;
        Ok(geometry.bounding_client_rect)
    }

    fn inner_html(&mut self, _node: DomNodeId) -> Result<String, String> {
        Ok(self.body_html.clone())
    }

    fn set_inner_html(&mut self, _node: DomNodeId, value: &str) -> Result<(), String> {
        self.body_html = value.to_owned();
        Ok(())
    }

    fn get_attribute(&mut self, node: DomNodeId, name: &str) -> Result<Option<String>, String> {
        Ok(self.attributes.get(&(node, name.to_owned())).cloned())
    }

    fn has_attribute(&mut self, node: DomNodeId, name: &str) -> Result<bool, String> {
        Ok(self.attributes.contains_key(&(node, name.to_owned())))
    }

    fn set_attribute(&mut self, node: DomNodeId, name: &str, value: &str) -> Result<(), String> {
        self.attributes
            .insert((node, name.to_owned()), value.to_owned());
        Ok(())
    }

    fn remove_attribute(&mut self, node: DomNodeId, name: &str) -> Result<(), String> {
        self.attributes.remove(&(node, name.to_owned()));
        Ok(())
    }

    fn style_property(&mut self, node: DomNodeId, property: &str) -> Result<String, String> {
        Ok(self
            .styles
            .get(&(node, property.to_owned()))
            .cloned()
            .unwrap_or_default())
    }

    fn set_style_property(
        &mut self,
        node: DomNodeId,
        property: &str,
        value: &str,
    ) -> Result<(), String> {
        self.styles
            .insert((node, property.to_owned()), value.to_owned());
        Ok(())
    }
}

#[test]
fn general_script_runtime_preserves_global_scope_across_evaluations() {
    let mut runtime = JsRuntime::new(TestDom::default()).unwrap();
    runtime.evaluate("var shared = 9;").unwrap();
    runtime
        .evaluate("if (shared !== 9) throw new Error('global scope was reset');")
        .unwrap();
}

#[test]
fn raw_script_uses_the_dom_facade_without_testharness_globals() {
    let large_node_id = (1_u64 << 53) + 1;
    let backend = TestDom::default().with_element(
        "line",
        large_node_id,
        test_geometry(60.0, 12.0, 4.0, 50.0, 60.0),
    );
    crate::dom::run_script(
            "var line = document.getElementById('line'); var rect = line.getBoundingClientRect(); if (document.body === null || line !== document.getElementById('line') || line.offsetHeight !== 60 || rect.left !== 12 || rect.top !== 4 || rect.right !== 62 || rect.bottom !== 64 || rect.width !== 50 || rect.height !== 60) throw new Error('DOM binding lost identity or geometry'); line.style.display = 'none'; if (line.style.display !== 'none') throw new Error('style mutation did not round-trip');",
            backend,
        )
        .unwrap();
}

#[test]
fn element_attribute_reads_distinguish_missing_and_empty_values() {
    let backend = TestDom::default()
        .with_element("line", 1, test_geometry(0.0, 0.0, 0.0, 0.0, 0.0))
        .with_attribute(1, "data-empty", "")
        .with_attribute(1, "title", "initial");
    crate::dom::run_script(
        r#"
                var line = document.getElementById('line');
                if (line.getAttribute('missing') !== null || line.hasAttribute('missing'))
                    throw new Error('missing attribute was reported present');
                if (line.getAttribute('data-empty') !== '' || !line.hasAttribute('data-empty'))
                    throw new Error('empty attribute was reported missing');
                if (line.getAttribute('title') !== 'initial' || !line.hasAttribute('title'))
                    throw new Error('existing attribute read failed');
            "#,
        backend,
    )
    .unwrap();
}

#[test]
fn element_attribute_mutations_round_trip_through_the_dom_backend() {
    let backend =
        TestDom::default().with_element("line", 1, test_geometry(0.0, 0.0, 0.0, 0.0, 0.0));
    crate::dom::run_script(
        r#"
                var line = document.getElementById('line');
                line.setAttribute('data-count', 42);
                if (line.getAttribute('data-count') !== '42' || !line.hasAttribute('data-count'))
                    throw new Error('setAttribute did not round-trip');
                line.removeAttribute('data-count');
                if (line.getAttribute('data-count') !== null || line.hasAttribute('data-count'))
                    throw new Error('removeAttribute did not remove the attribute');
            "#,
        backend,
    )
    .unwrap();
}

#[test]
fn runs_assertions_against_the_dom_backend() {
    let backend =
        TestDom::default().with_element("line", 1, test_geometry(60.0, 12.0, 0.0, 20.0, 60.0));
    let outcomes = run_testharness_script(
            "test(function() { assert_true(document.getElementById('line').offsetHeight > 35); }, 'height');",
            backend,
        )
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].passed, "{:?}", outcomes[0]);
}

#[test]
fn exposes_window_as_the_dom_global_object() {
    let outcomes = run_testharness_script(
        "test(function() { assert_true(window === globalThis); assert_true(window.document === document); assert_true(typeof window.getComputedStyle === 'function'); }, 'window global');",
        TestDom::default(),
    )
    .unwrap();

    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].passed, "{:?}", outcomes[0]);
}

#[test]
fn assert_not_equals_checks_inequality_and_reports_equal_values() {
    let passed = run_testharness_script(
        "test(function() { assert_not_equals('a', 'b'); }, 'different values');",
        TestDom::default(),
    )
    .unwrap();
    assert_eq!(passed.len(), 1);
    assert!(passed[0].passed, "{:?}", passed[0]);

    let failed = run_testharness_script(
        "test(function() { assert_not_equals('a', 'a'); }, 'equal values');",
        TestDom::default(),
    )
    .unwrap();
    assert_eq!(failed.len(), 1);
    assert!(!failed[0].passed);
    assert!(failed[0].message.contains("assert_not_equals"));
}

#[test]
fn assert_in_array_checks_membership_and_reports_non_members() {
    let passed = run_testharness_script(
        "test(function() { assert_in_array('b', ['a', 'b', 'c']); }, 'member');",
        TestDom::default(),
    )
    .unwrap();
    assert_eq!(passed.len(), 1);
    assert!(passed[0].passed, "{:?}", passed[0]);

    let failed = run_testharness_script(
        "test(function() { assert_in_array('z', ['a', 'b', 'c']); }, 'non-member');",
        TestDom::default(),
    )
    .unwrap();
    assert_eq!(failed.len(), 1);
    assert!(!failed[0].passed);
    assert!(failed[0].message.contains("assert_in_array"));
}

#[test]
fn empty_script_is_reported_as_no_tests() {
    let result = run_testharness_script("", TestDom::default());
    assert!(matches!(result, Err(TestHarnessError::NoTests)));
}

#[test]
fn uncaught_javascript_errors_are_not_assertion_failures() {
    let result = run_testharness_script("throw new Error('uncaught');", TestDom::default());
    assert!(matches!(result, Err(TestHarnessError::JavaScript(_))));
}

#[test]
fn missing_element_is_an_assertion_failure_not_a_pass() {
    let result = run_testharness_script(
            "test(function() { assert_true(document.getElementById('missing').offsetHeight > 35); }, 'missing');",
            TestDom::default(),
        )
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(!result[0].passed);
    assert!(result[0].message.contains("null"));
}

#[test]
fn inner_html_mutation_is_visible_to_deferred_font_callback() {
    let backend =
        TestDom::default().with_element("span", 2, test_geometry(30.0, 42.0, 0.0, 20.0, 30.0));
    let result = run_testharness_script(
            r#"
                document.querySelector('body').innerHTML = '<span id="span">text</span>';
                setup({explicit_done: true});
                document.fonts.ready.then(function() {
                    test(function() {
                        assert_approx_equals(document.getElementById('span').getBoundingClientRect().left, 42, 1);
                        document.getElementById('span').parentNode.style.display = 'none';
                    }, 'left');
                    done();
                });
            "#,
            backend,
        )
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].passed, "{:?}", result[0]);
}

#[test]
fn backend_failures_are_reported_as_dom_errors() {
    let backend = TestDom::default()
        .with_element("broken", 3, test_geometry(0.0, 0.0, 0.0, 0.0, 0.0))
        .with_failing_geometry();
    // The test harness catches the JS exception. The adapter still reports
    // the underlying failed geometry lookup as a DOM error, not an assertion.
    let result = run_testharness_script(
        "test(function() { document.getElementById('broken').offsetHeight; }, 'broken geometry');",
        backend,
    );
    assert!(matches!(result, Err(TestHarnessError::Dom(_))));
}

#[test]
fn callbacks_that_requeue_themselves_hit_the_event_loop_limit() {
    let result = run_testharness_script(
        "document.fonts.ready.then(function repeat() { document.fonts.ready.then(repeat); }); test(function() { assert_true(true); }, 'initial');",
        TestDom::default(),
    );
    assert!(matches!(result, Err(TestHarnessError::EventLoopLimit)));
}

#[test]
fn font_load_callbacks_run_through_the_testharness_event_loop() {
    let result = run_testharness_script(
        r#"document.fonts.load("20px Ahem").then(function(fonts) {
            test(function() { assert_equals(fonts.length, 0); }, "font load callback");
        });"#,
        TestDom::default(),
    )
    .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].passed, "{:?}", result[0]);
}

#[test]
fn run_testharness_on_host_reports_outcomes_from_native_dom() {
    let (host, ..) = crate::runtime::test_host::StubHost::page();
    let outcomes = crate::testharness::run_testharness_on_host(
        "test(function () { assert_equals(document.body.tagName, 'BODY'); }, 'body');
         test(function () { assert_true(document.body instanceof HTMLElement); }, 'proto');",
        host,
    )
    .unwrap();
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|o| o.passed), "{outcomes:?}");
}

#[test]
fn run_testharness_on_host_maps_host_failures_to_dom_errors() {
    let (mut host, ..) = crate::runtime::test_host::StubHost::page();
    host.fail_flush = true;
    let result = crate::testharness::run_testharness_on_host(
        "test(function () { document.body.offsetHeight; }, 'geometry');",
        host,
    );
    assert!(
        matches!(result, Err(crate::testharness::TestHarnessError::Dom(ref m)) if m == "stub flush failure"),
        "{result:?}"
    );
}
