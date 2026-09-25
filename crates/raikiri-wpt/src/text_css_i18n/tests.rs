use super::*;

const TWO_BY_THREE_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 3, 8, 6, 0,
    0, 0, 185, 234, 222, 129, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31,
    10, 209, 24, 0, 158, 124, 11, 245, 228, 127, 198, 52, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
    130,
];

fn live_backend(html: &str, root: &Path) -> LiveDocumentBackend {
    let setup = crate::reftest::prepare_wpt_live_document(html, 800, 600, root, root)
        .expect("valid test HTML should configure the live document");
    LiveDocumentBackend::new(setup, root)
}

#[test]
fn create_element_rejects_invalid_xml_names() {
    let root = tempfile::tempdir().unwrap();
    let html = r#"<!doctype html><html><body></body></html>"#;
    let mut backend = live_backend(html, root.path());
    assert!(backend.create_element("a>b").is_err());
}

#[test]
fn live_dom_script_creates_appends_and_mutates_elements() {
    let root = tempfile::tempdir().unwrap();
    let html = r#"<!doctype html><html><head>
            <style>.dynamic { width: 22px; height: 7px; }</style>
        </head><body></body></html>"#;
    let backend = live_backend(html, root.path());
    let outcomes = run_testharness_script(
        r#"
                test(function() {
                    var body = document.body;
                    var child = document.createElement('DIV');
                    assert_equals(child.parentNode, null);
                    child.textContent = 'first';
                    assert_equals(child.textContent, 'first');
                    child.classList.add('dynamic');
                    assert_equals(body.appendChild(child), child);
                    assert_equals(child.parentNode, body);
                    assert_equals(document.querySelector('.dynamic'), child);
                    assert_equals(child.offsetHeight, 7);
                    child.style.height = '13px';
                    assert_equals(child.offsetHeight, 13);
                    child.textContent = 'updated';
                    assert_equals(child.textContent, 'updated');
                    child.textContent = null;
                    assert_equals(child.textContent, '');
                    child.textContent = 'restored';
                    assert_equals(child.offsetHeight, 13);
                }, 'dynamic element creation, mutation, and geometry');
            "#,
        backend,
    )
    .unwrap();

    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].passed);
}

#[test]
fn dynamic_style_text_and_connection_updates_the_cascade() {
    let root = tempfile::tempdir().unwrap();
    let html = r#"<!doctype html><html><head></head><body></body></html>"#;
    let backend = live_backend(html, root.path());
    let outcomes = run_testharness_script(
        r#"
                test(function() {
                    var box = document.createElement('div');
                    box.classList.add('from-style');
                    document.body.appendChild(box);
                    var sheet = document.createElement('style');
                    sheet.textContent = '.from-style { width: 10px; height: 9px; }';
                    assert_equals(box.offsetHeight, 0);
                    document.head.appendChild(sheet);
                    assert_equals(box.offsetHeight, 9);
                    sheet.textContent = '.from-style { width: 10px; height: 14px; }';
                    assert_equals(box.offsetHeight, 14);
                    var detached = document.createElement('div');
                    detached.appendChild(sheet);
                    assert_equals(box.offsetHeight, 0);
                    document.head.appendChild(sheet);
                    assert_equals(box.offsetHeight, 14);
                }, 'style text and connection changes update the cascade');
            "#,
        backend,
    )
    .unwrap();

    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].passed, "{outcome:?}", outcome = outcomes[0]);
}

#[test]
fn live_dom_mutations_coalesce_into_lazy_full_layout_flushes() {
    let root = tempfile::tempdir().unwrap();
    let html = r#"<!doctype html><html><body>
            <div id="probe" style="width: 10px; height: 5px"></div>
        </body></html>"#;
    let mut backend = live_backend(html, root.path());
    let probe = backend.get_element_by_id("probe").unwrap().unwrap();

    let initial = backend.bounding_client_rect(probe).unwrap();
    assert_eq!((initial.width, initial.height), (10.0, 5.0));
    assert_eq!(backend.layout_flush_count, 1);

    backend.set_style_property(probe, "width", "20px").unwrap();
    backend.set_style_property(probe, "height", "8px").unwrap();
    backend.set_attribute(probe, "data-mutated", "yes").unwrap();
    assert_eq!(backend.layout_flush_count, 1);

    let updated = backend.bounding_client_rect(probe).unwrap();
    assert_eq!((updated.width, updated.height), (20.0, 8.0));
    assert_eq!(backend.layout_flush_count, 2);
    backend.bounding_client_rect(probe).unwrap();
    assert_eq!(backend.layout_flush_count, 2);

    backend.set_style_property(probe, "height", "11px").unwrap();
    let updated_again = backend.bounding_client_rect(probe).unwrap();
    assert_eq!(updated_again.height, 11.0);
    assert_eq!(backend.layout_flush_count, 3);
}

#[test]
fn changing_image_source_updates_intrinsic_geometry_after_one_lazy_flush() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("small.png"), TWO_BY_THREE_PNG).unwrap();
    let html = r#"<!doctype html><html><body><img id="probe"></body></html>"#;
    let mut backend = live_backend(html, root.path());
    let image = backend.get_element_by_id("probe").unwrap().unwrap();

    let initial = backend.bounding_client_rect(image).unwrap();
    assert_eq!((initial.width, initial.height), (0.0, 0.0));
    assert_eq!(backend.layout_flush_count, 1);

    backend.set_attribute(image, "src", "small.png").unwrap();
    assert_eq!(backend.layout_flush_count, 1);
    let loaded = backend.bounding_client_rect(image).unwrap();
    assert_eq!((loaded.width, loaded.height), (2.0, 3.0));
    assert_eq!(backend.layout_flush_count, 2);

    backend.remove_attribute(image, "src").unwrap();
    let missing = backend.bounding_client_rect(image).unwrap();
    assert_eq!((missing.width, missing.height), (0.0, 0.0));
    assert_eq!(backend.layout_flush_count, 3);
}

#[test]
fn live_selectors_and_inline_style_access_cover_dom_facade_queries() {
    let root = tempfile::tempdir().unwrap();
    let html = r#"<!doctype html><html><body>
            <div id="target" class="first second" data-key="value"
                 style="color: red; --token: first"></div>
            <span id="unstyled"></span>
        </body></html>"#;
    let mut backend = live_backend(html, root.path());
    let target = backend.get_element_by_id("target").unwrap().unwrap();
    let unstyled = backend.get_element_by_id("unstyled").unwrap().unwrap();
    let body = backend.query_selector("body").unwrap().unwrap();
    let html_element = backend.query_selector("html").unwrap().unwrap();

    assert!(backend.get_element_by_id("").unwrap().is_none());
    assert!(backend.query_selector("*").unwrap().is_some());
    assert_eq!(backend.query_selector("#target").unwrap(), Some(target));
    assert_eq!(backend.query_selector(".second").unwrap(), Some(target));
    assert_eq!(
        backend.query_selector("[data-key='value']").unwrap(),
        Some(target)
    );
    assert_eq!(backend.query_selector("[data-key]").unwrap(), Some(target));
    assert_eq!(backend.query_selector("DIV").unwrap(), Some(target));
    assert_eq!(backend.parent_node(target).unwrap(), Some(body));
    assert_eq!(backend.parent_node(html_element).unwrap(), None);

    assert_eq!(backend.style_property(target, "COLOR").unwrap(), "red");
    assert_eq!(backend.style_property(target, "--token").unwrap(), "first");
    assert_eq!(backend.style_property(unstyled, "color").unwrap(), "");
    backend.set_style_property(target, "", "ignored").unwrap();
    backend
        .set_style_property(target, "--token", "second")
        .unwrap();
    assert_eq!(backend.style_property(target, "--token").unwrap(), "second");
    backend.set_style_property(target, "color", "").unwrap();
    assert_eq!(backend.style_property(target, "color").unwrap(), "");
}

#[test]
fn setting_style_element_inner_html_reloads_stylesheet_and_removal() {
    let root = tempfile::tempdir().unwrap();
    let html = r#"<!doctype html><html><head><style id="dynamic"></style></head>
            <body><div id="probe"></div></body></html>"#;
    let mut backend = live_backend(html, root.path());
    let style = backend.get_element_by_id("dynamic").unwrap().unwrap();
    let probe = backend.get_element_by_id("probe").unwrap().unwrap();

    backend
        .set_inner_html(style, "#probe { width: 27px; height: 19px; }")
        .unwrap();
    let styled = backend.bounding_client_rect(probe).unwrap();
    assert_eq!(styled.width, 27.0);
    assert_eq!(styled.height, 19.0);

    backend.set_inner_html(style, "").unwrap();
    let unstyled = backend.bounding_client_rect(probe).unwrap();
    assert_ne!(unstyled.width, 27.0);
}

#[test]
fn discovers_testharness_files_recursively_but_not_reference_documents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("css/css-text/i18n");
    fs::create_dir_all(root.join("zh/reference")).unwrap();
    fs::write(
        root.join("test.html"),
        "<script src='/resources/testharness.js'></script>",
    )
    .unwrap();
    fs::write(
        root.join("zh/locale.html"),
        "<script src='/resources/testharness.js'></script>",
    )
    .unwrap();
    fs::write(
        root.join("zh/reference/ref.html"),
        "<script src='/resources/testharness.js'></script>",
    )
    .unwrap();
    fs::write(
        root.join("visual.html"),
        "<link rel='match' href='ref.html'>",
    )
    .unwrap();

    let discovered = discover_testharness_files(&root).unwrap();
    let ids = discovered
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, ["test.html", "zh/locale.html"]);
}

#[test]
fn inline_script_extraction_ignores_html_comment_examples() {
    let html =
        "<!-- <script>not_a_test();</script> -->\n<script>test(function() {}, 'real');</script>";
    let scripts = inline_test_scripts(html);
    assert!(scripts.contains("test(function()"));
    assert!(!scripts.contains("not_a_test"));
}

fn write_test_page(wpt_root: &Path, name: &str, html: &str) {
    let test_dir = wpt_root.join(TEST_DIR);
    fs::create_dir_all(&test_dir).unwrap();
    fs::write(test_dir.join(name), html).unwrap();
}

#[test]
fn live_backend_updates_identity_attributes_style_and_current_geometry() {
    let wpt_root = tempfile::tempdir().unwrap();
    let html = r#"<!doctype html><html><head><style>
            @media screen { #media-box { width: 23px; height: 17px; } }
            @media print { #media-box { width: 230px; height: 170px; } }
            </style></head><body><p>initial</p></body></html>"#;
    let setup = prepare_wpt_live_document(
        html,
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        wpt_root.path(),
        wpt_root.path(),
    )
    .unwrap();
    let backend = LiveDocumentBackend::new(setup, wpt_root.path());

    raikiri_js::run_script(
            r#"
                var body = document.body;
                if (document.querySelector('body') !== body)
                    throw new Error('document/body wrappers are not stable');
                body.innerHTML = '<div id="box" style="width:10px;height:12px"></div>' +
                    '<div id="media-box"></div>' +
                    '<div id="container"><span id="child">old</span></div>';

                var mediaBox = document.getElementById('media-box');
                var mediaRect = mediaBox.getBoundingClientRect();
                if (mediaRect.width !== 23 || mediaRect.height !== 17)
                    throw new Error('live testharness layout did not use screen media');
                var box = document.getElementById('box');
                if (box !== document.querySelector('#box') || box.getAttribute('id') !== 'box')
                    throw new Error('repeated live element queries lost identity');
                box.setAttribute('data-empty', '');
                box.setAttribute('data-count', 7);
                if (!box.hasAttribute('data-empty') || box.getAttribute('data-empty') !== '' ||
                    box.getAttribute('data-count') !== '7')
                    throw new Error('attribute reads do not reflect mutations');
                box.setAttribute('DATA-Key', 'first');
                box.setAttribute('data-key', 'second');
                if (box.getAttribute('DATA-KEY') !== 'second')
                    throw new Error('HTML attribute names were not case-insensitive');
                box.removeAttribute('data-count');
                if (box.hasAttribute('data-count') || box.getAttribute('data-count') !== null)
                    throw new Error('removeAttribute did not update the element');
                if (!body.innerHTML.includes('data-empty=""'))
                    throw new Error('innerHTML did not serialize current attributes');

                var child = document.getElementById('child');
                var container = document.querySelector('#container');
                if (child.parentNode !== container)
                    throw new Error('parentNode does not follow the live tree');

                box.style.width = '20px';
                var before = box.getBoundingClientRect();
                box.style.width = '40px';
                box.style.height = '32px';
                var after = box.getBoundingClientRect();
                if (box.style.getPropertyValue('width') !== '40px')
                    throw new Error('CSSStyleDeclaration read missed the inline write');
                if (!(after.width > before.width && after.height > before.height &&
                    box.offsetHeight > 20))
                    throw new Error('geometry did not reflect current inline style');

                var oldChild = child;
                var oldContainer = oldChild.parentNode;
                body.innerHTML = '<div id="replacement"><span id="new-child">new</span></div>';
                if (document.body !== body || oldChild.parentNode !== oldContainer ||
                    oldContainer.parentNode !== null || document.getElementById('child') !== null)
                    throw new Error('innerHTML did not replace the actual body children');
                var newChild = document.getElementById('new-child');
                if (newChild === oldChild || newChild.parentNode !== document.getElementById('replacement'))
                    throw new Error('replacement nodes reused old identities or parent links');

                body.innerHTML = '<table><tbody id="rows"></tbody></table>' +
                    '<template id="template"></template><svg id="svg"></svg>';
                var rows = document.getElementById('rows');
                rows.innerHTML = '<tr><td id="cell">cell</td></tr>';
                var cell = document.getElementById('cell');
                if (cell === null || cell.parentNode === null ||
                    cell.parentNode.parentNode !== rows)
                    throw new Error('table-context innerHTML did not preserve fragment semantics');
                var template = document.getElementById('template');
                template.innerHTML = '<span id="inert-template-child">inside</span>';
                if (!template.innerHTML.includes('inert-template-child') ||
                    document.getElementById('inert-template-child') !== null)
                    throw new Error('template innerHTML did not target inert template contents');
                var svg = document.getElementById('svg');
                svg.innerHTML = '<g id="svg-child" viewBox="0 0 1 1"></g>';
                var svgChild = document.getElementById('svg-child');
                if (svgChild.getAttribute('viewBox') !== '0 0 1 1' ||
                    svgChild.getAttribute('viewbox') !== null)
                    throw new Error('SVG innerHTML or attribute-name case was incorrect');

                body.innerHTML = '<style>#from-style { display: none }</style>' +
                    '<div id="from-style">hidden</div>';
                if (document.getElementById('from-style').offsetHeight !== 0)
                    throw new Error('a style element inserted by innerHTML did not reach cascade');
                body.innerHTML = '<div id="from-style">shown</div>';
                if (document.getElementById('from-style').offsetHeight <= 0)
                    throw new Error('a removed style element still affected cascade');
            "#,
            backend,
        )
        .unwrap();
}

#[test]
fn live_document_preserves_hidden_geometry_parent_links_and_replacement_identity() {
    let wpt_root = tempfile::tempdir().unwrap();
    write_test_page(
        wpt_root.path(),
        "hidden.html",
        r#"<!doctype html><html><head>
                <style>#hidden { display: none }</style>
                <script src="/resources/testharness.js"></script>
            </head><body><div id="hidden">hidden</div><script>
                test(function() {
                    var hidden = document.getElementById('hidden');
                    assert_true(hidden !== null, 'hidden element remains in the DOM');
                    assert_approx_equals(hidden.offsetHeight, 0, 0);
                    var rect = hidden.getBoundingClientRect();
                    assert_approx_equals(rect.left, 0, 0);
                    assert_approx_equals(rect.top, 0, 0);
                    assert_approx_equals(rect.right, 0, 0);
                    assert_approx_equals(rect.bottom, 0, 0);
                    assert_approx_equals(rect.width, 0, 0);
                    assert_approx_equals(rect.height, 0, 0);
                    assert_true(document.body.getBoundingClientRect().width > 0,
                        'body geometry comes from layout');
                }, 'hidden geometry');
            </script></body></html>"#,
    );
    write_test_page(
        wpt_root.path(),
        "parent.html",
        r#"<!doctype html><html><head>
                <script src="/resources/testharness.js"></script>
            </head><body><div id="old-wrapper"><span id="old-target">old</span></div><script>
                var bodyBeforeReplacement = document.body;
                var oldTarget = document.getElementById('old-target');
                document.body.innerHTML = '<div id="wrapper"><span id="target">text</span></div>';
                setup({explicit_done: true});
                document.fonts.ready.then(function() {
                    test(function() {
                        var target = document.getElementById('target');
                        assert_true(bodyBeforeReplacement === document.body,
                            'body retains identity after innerHTML replacement');
                        assert_true(oldTarget !== target,
                            'a reparsed node gets a fresh wrapper even when source NodeIds repeat');
                        target.parentNode.style.display = 'none';
                        assert_true(document.getElementById('wrapper').style.display === 'none',
                            'parentNode refers to the real parent element');
                    }, 'parent links and live node identity');
                    done();
                });
            </script></body></html>"#,
    );

    let results = run_css_text_i18n(wpt_root.path()).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|result| result.error.is_none()));
    assert!(results.iter().all(TestHarnessFileResult::all_passed));
    assert_eq!(results[0].total(), 1);
    assert_eq!(results[1].total(), 1);
}

#[test]
fn missing_and_empty_test_roots_report_distinct_errors() {
    let wpt_root = tempfile::tempdir().unwrap();
    assert!(matches!(
        run_css_text_i18n(&wpt_root.path().join("missing")),
        Err(TestHarnessRunError::Io(_))
    ));
    fs::create_dir_all(wpt_root.path().join(TEST_DIR)).unwrap();
    assert!(matches!(
        run_css_text_i18n(wpt_root.path()),
        Err(TestHarnessRunError::NoTests)
    ));
}

#[test]
fn unreadable_test_file_is_reported_as_an_execution_error() {
    let wpt_root = tempfile::tempdir().unwrap();
    let result = run_testharness_file(&wpt_root.path().join("missing.html"), wpt_root.path());
    assert!(result.outcomes.is_empty());
    assert!(result.error.unwrap().starts_with("read test HTML:"));
}

#[test]
fn live_testharness_resolves_relative_stylesheets_and_images_from_the_page_directory() {
    let wpt_root = tempfile::tempdir().unwrap();
    let test_dir = wpt_root.path().join(TEST_DIR).join("nested");
    fs::create_dir_all(&test_dir).unwrap();
    fs::write(
        test_dir.join("relative.css"),
        "#relative { width: 27px; height: 19px; }",
    )
    .unwrap();
    fs::write(test_dir.join("small.png"), TWO_BY_THREE_PNG).unwrap();
    let html = r#"<!doctype html>
            <html><head>
              <link rel="stylesheet" href="relative.css">
              <script src="/resources/testharness.js"></script>
            </head><body>
              <div id="relative"></div>
              <img id="pixel" src="small.png">
              <script>
                test(function () {
                  assert_approx_equals(
                    document.getElementById('relative').getBoundingClientRect().width,
                    27, 0.1);
                  assert_approx_equals(document.getElementById('pixel').offsetHeight, 3, 0.1);
                }, 'relative stylesheets and images use the test page directory');
              </script>
            </body></html>"#;
    let test_file = test_dir.join("relative-resources.html");
    fs::write(&test_file, html).unwrap();

    let result = run_testharness_file(&test_file, wpt_root.path());
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 1);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_wrap_balance_wpt_runs_with_live_dynamic_dom_mutations() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/white-space/text-wrap-balance-001.html");
    let result = run_testharness_file(&test_file, &wpt_root);
    assert!(result.error.is_none());
    assert_eq!(result.total(), 3);
    assert!(result.all_passed());
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn static_and_dynamic_i18n_fixtures_report_assertion_outcomes() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    for (relative, expected_count) in [
        (
            "css/css-text/i18n/css3-text-line-break-baspglwj-001.html",
            4,
        ),
        (
            "css/css-text/i18n/zh/css-text-line-break-zh-pr-normal.html",
            8,
        ),
    ] {
        let result = run_testharness_file(&wpt_root.join(relative), &wpt_root);
        assert!(result.error.is_none(), "{}: {:?}", relative, result.error);
        assert_eq!(result.total(), expected_count, "{relative}");
        assert!(
            !result.outcomes.is_empty(),
            "{relative} produced no assertions"
        );
        if relative == "css/css-text/i18n/css3-text-line-break-baspglwj-001.html" {
            assert!(result.all_passed(), "{relative}: {:?}", result.outcomes);
        }
        // The dynamic fixture's remaining failures are engine-level line-break
        // gaps, not runner execution errors; this test verifies those outcomes
        // are returned without promoting them to a DOM-binding pass gate.
    }
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn white_space_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/white-space-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 6);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn white_space_collapse_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/white-space-collapse-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 4);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn line_break_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/line-break-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 5);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn hyphens_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/hyphens-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn overflow_wrap_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/overflow-wrap-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_break_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/word-break-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 5);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_transform_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-transform-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 10);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_align_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-align-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 7);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_justify_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-justify-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 4);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_justify_computed_legacy_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-justify-computed-legacy.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 1);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_indent_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-indent-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 10);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_align_last_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-align-last-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 8);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_wrap_mode_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-wrap-mode-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 2);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_autospace_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-autospace-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 32);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_wrap_style_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-wrap-style-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_wrap_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-wrap-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 17);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn hyphenate_character_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/hyphenate-character-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 5);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn hyphenate_limit_chars_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/hyphenate-limit-chars-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 11);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_spacing_trim_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-spacing-trim-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 7);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn letter_spacing_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/letter-spacing-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 9);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_spacing_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/word-spacing-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 9);
    let passed = result
        .outcomes
        .iter()
        .filter(|outcome| outcome.passed)
        .count();
    assert_eq!(passed, 8, "{:?}", result.outcomes);
    let failures: Vec<_> = result
        .outcomes
        .iter()
        .filter(|outcome| !outcome.passed)
        .collect();
    assert_eq!(failures.len(), 1, "{:?}", result.outcomes);
    assert_eq!(
        failures[0].name,
        "Property word-spacing value 'calc(10px - (5% + 10%)'"
    );
    assert_eq!(
        failures[0].message,
        "Error: assert_equals: expected calc(-15% + 10px), got 0px"
    );
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_shadow_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file =
        wpt_root.join("css/css-text-decor/text-shadow/parsing/text-shadow-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 7);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn letter_spacing_inherited_computed_wpt_case_uses_live_computed_style() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/letter-spacing-inherited-computed.html");
    let result = run_testharness_file(&test_file, &wpt_root);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_space_transform_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/word-space-transform-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 5);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_decoration_skip_ink_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file =
        wpt_root.join("css/css-text-decor/parsing/text-decoration-skip-ink-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_decoration_skip_spaces_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file =
        wpt_root.join("css/css-text-decor/parsing/text-decoration-skip-spaces-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 5);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_decoration_style_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text-decor/parsing/text-decoration-style-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 5);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_decoration_line_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text-decor/parsing/text-decoration-line-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 18);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_decoration_color_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text-decor/parsing/text-decoration-color-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_decoration_inset_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text-decor/parsing/text-decoration-inset-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 10);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_decoration_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text-decor/parsing/text-decoration-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 14);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_underline_offset_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text-decor/text-underline-offset-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 15);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn writing_mode_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-writing-modes/parsing/writing-mode-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn direction_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-writing-modes/parsing/direction-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 2);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn unicode_bidi_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-writing-modes/parsing/unicode-bidi-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 6);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_combine_upright_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file =
        wpt_root.join("css/css-writing-modes/parsing/text-combine-upright-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 2);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_orientation_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-writing-modes/parsing/text-orientation-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_underline_position_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file =
        wpt_root.join("css/css-text-decor/parsing/text-underline-position-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 7);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_emphasis_position_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file =
        wpt_root.join("css/css-text-decor/parsing/text-emphasis-position-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 7);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_emphasis_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text-decor/parsing/text-emphasis-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 7);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_emphasis_style_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text-decor/parsing/text-emphasis-style-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 9);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_emphasis_style_computed_vertical_lr_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file =
        wpt_root.join("css/css-text-decor/parsing/text-emphasis-style-computed-vertical-lr.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 9);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn tab_size_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/tab-size-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 10);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
// cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_wrap_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/word-wrap-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_spacing_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-text/parsing/text-spacing-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 16);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_kerning_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-kerning-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_variant_caps_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-variant-caps-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 7);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_optical_sizing_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-optical-sizing-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 2);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_variant_emoji_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-variant-emoji-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 4);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_language_override_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-language-override-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 5);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_variant_ligatures_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-variant-ligatures-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 10);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_synthesis_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-synthesis-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 21);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_variant_position_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-variant-position-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 3);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_palette_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-palette-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 4);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_variant_numeric_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-variant-numeric-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 11);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn font_variant_east_asian_computed_wpt_case_uses_the_pinned_computed_helper() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test_file = wpt_root.join("css/css-fonts/parsing/font-variant-east-asian-computed.html");
    let helper = fs::read_to_string(wpt_root.join("css/support/computed-testcommon.js"))
        .expect("read the pinned WPT computed-testcommon.js helper");
    let result = run_testharness_file_with_helper(&test_file, &wpt_root, &helper);

    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total(), 12);
    assert!(result.all_passed(), "{:?}", result.outcomes);
}

/// Reads `property` from one styled element per case and reports every mismatch at once.
fn assert_computed_cases(cases: &[(&str, &str, &str)]) {
    let root = tempfile::tempdir().unwrap();
    let body: String = cases
        .iter()
        .enumerate()
        .map(|(index, (property, value, _))| {
            format!(r#"<div id="c{index}" style="{property}: {value}">x</div>"#)
        })
        .collect();
    let html = format!("<!doctype html><html><body>{body}</body></html>");
    let mut backend = live_backend(&html, root.path());
    let mismatches: Vec<String> = cases
        .iter()
        .enumerate()
        .filter_map(|(index, (property, value, expected))| {
            let node = backend
                .get_element_by_id(&format!("c{index}"))
                .unwrap()
                .expect("case element exists");
            let actual = backend.computed_style_property(node, property).unwrap();
            (actual.as_deref() != Some(*expected))
                .then(|| format!("{property}: {value} => {actual:?}, expected {expected:?}"))
        })
        .collect();
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn computed_style_property_ignores_unsupported_names_without_flushing() {
    let root = tempfile::tempdir().unwrap();
    let mut backend = live_backend(
        r#"<!doctype html><html><body><div id="box"></div></body></html>"#,
        root.path(),
    );
    let node = backend.get_element_by_id("box").unwrap().unwrap();
    assert_eq!(
        backend.computed_style_property(node, "color").unwrap(),
        None
    );
    assert_eq!(backend.layout_flush_count, 0);
    assert_eq!(
        backend.computed_style_property(node, "WORD-WRAP").unwrap(),
        Some("normal".to_owned())
    );
    assert_eq!(backend.layout_flush_count, 1);
}

#[test]
fn computed_style_property_serializes_keyword_properties() {
    let mut cases = Vec::new();
    for (property, values) in [
        ("direction", &["ltr", "rtl"][..]),
        (
            "font-variant-caps",
            &[
                "normal",
                "small-caps",
                "all-small-caps",
                "petite-caps",
                "all-petite-caps",
                "unicase",
                "titling-caps",
            ],
        ),
        ("text-combine-upright", &["none", "all"]),
        ("text-orientation", &["mixed", "upright", "sideways"]),
        (
            "writing-mode",
            &[
                "horizontal-tb",
                "vertical-rl",
                "vertical-lr",
                "sideways-rl",
                "sideways-lr",
            ],
        ),
        (
            "unicode-bidi",
            &[
                "normal",
                "embed",
                "isolate",
                "bidi-override",
                "isolate-override",
                "plaintext",
            ],
        ),
        (
            "text-spacing-trim",
            &[
                "auto",
                "normal",
                "space-all",
                "trim-both",
                "trim-all",
                "trim-start",
                "space-first",
            ],
        ),
        (
            "word-space-transform",
            &[
                "none",
                "space",
                "ideographic-space",
                "space auto-phrase",
                "ideographic-space auto-phrase",
            ],
        ),
        (
            "white-space",
            &[
                "normal",
                "pre",
                "nowrap",
                "pre-wrap",
                "pre-line",
                "break-spaces",
            ],
        ),
        (
            "white-space-collapse",
            &[
                "collapse",
                "discard",
                "preserve",
                "preserve-breaks",
                "preserve-spaces",
                "break-spaces",
            ],
        ),
        (
            "line-break",
            &["auto", "loose", "normal", "strict", "anywhere"],
        ),
        ("hyphens", &["none", "manual", "auto"]),
        ("overflow-wrap", &["normal", "break-word", "anywhere"]),
        ("word-wrap", &["normal", "break-word", "anywhere"]),
        (
            "word-break",
            &[
                "normal",
                "keep-all",
                "break-all",
                "break-word",
                "auto-phrase",
            ],
        ),
        (
            "text-align",
            &["start", "end", "left", "right", "center", "justify"],
        ),
        (
            "text-align-last",
            &["auto", "start", "end", "left", "right", "center", "justify"],
        ),
        ("text-wrap-mode", &["wrap", "nowrap"]),
        ("text-wrap-style", &["auto", "balance", "pretty", "stable"]),
        (
            "text-justify",
            &["auto", "none", "inter-word", "inter-character"],
        ),
        (
            "text-transform",
            &[
                "none",
                "math-auto",
                "capitalize",
                "uppercase",
                "lowercase",
                "full-width",
                "full-size-kana",
                "capitalize full-width",
                "uppercase full-width",
                "lowercase full-width",
                "capitalize full-size-kana",
                "uppercase full-size-kana",
                "lowercase full-size-kana",
                "full-width full-size-kana",
                "capitalize full-width full-size-kana",
                "uppercase full-width full-size-kana",
                "lowercase full-width full-size-kana",
            ],
        ),
    ] {
        cases.extend(values.iter().map(|value| (property, *value, *value)));
    }
    assert_computed_cases(&cases);
}

#[test]
fn computed_style_property_serializes_structured_properties() {
    assert_computed_cases(&[
        ("font-kerning", "auto", "auto"),
        ("font-kerning", "normal", "normal"),
        ("font-kerning", "none", "none"),
        ("font-optical-sizing", "none", "none"),
        ("font-variant-emoji", "emoji", "emoji"),
        ("font-language-override", r#"'TRK'"#, r#""TRK""#),
        ("font-variant-ligatures", "no-contextual", "no-contextual"),
        ("font-synthesis", "weight style", "weight style"),
        ("font-variant-position", "super", "super"),
        ("font-palette", "--custom", "--custom"),
        (
            "font-variant-numeric",
            "tabular-nums slashed-zero",
            "tabular-nums slashed-zero",
        ),
        (
            "font-variant-east-asian",
            "jis04 full-width ruby",
            "jis04 full-width ruby",
        ),
        ("text-justify", "distribute", "inter-character"),
        ("text-align", "match-parent", "left"),
        ("text-wrap", "wrap", "wrap"),
        ("text-wrap", "balance", "balance"),
        ("text-wrap", "nowrap", "nowrap"),
        ("text-wrap", "nowrap pretty", "nowrap pretty"),
        ("hyphenate-limit-chars", "auto", "auto"),
        ("hyphenate-limit-chars", "5 2", "5 2"),
        ("hyphenate-limit-chars", "5 2 3", "5 2 3"),
        ("hyphenate-character", "auto", "auto"),
        ("hyphenate-character", r#"'-'"#, r#""-""#),
        ("text-spacing", "normal", "normal"),
        ("text-spacing", "none", "none"),
        ("text-spacing", "auto", "auto"),
        ("text-spacing", "trim-both", "trim-both"),
        (
            "text-spacing",
            "trim-both no-autospace",
            "trim-both no-autospace",
        ),
        (
            "text-autospace",
            "ideograph-alpha insert",
            "ideograph-alpha insert",
        ),
        (
            "text-autospace",
            "ideograph-numeric punctuation replace",
            "ideograph-numeric punctuation replace",
        ),
        ("text-decoration-skip-ink", "none", "none"),
        ("text-decoration-skip-spaces", "none", "none"),
        ("text-decoration", "none", "none"),
        ("text-decoration", "underline currentcolor", "underline"),
        (
            "text-decoration",
            "underline dotted from-font rgba(0, 0, 255, 0.5)",
            "underline dotted from-font rgba(0, 0, 255, 0.5)",
        ),
        (
            "text-decoration",
            "overline 2px red",
            "overline 2px rgb(255, 0, 0)",
        ),
        ("text-decoration-style", "wavy", "wavy"),
        ("text-decoration-line", "line-through", "line-through"),
        ("text-decoration-inset", "auto", "auto"),
        ("text-decoration-inset", "3px", "3px"),
        ("text-decoration-inset", "1px 2px", "1px 2px"),
        ("text-emphasis-position", "under left", "under left"),
        ("text-shadow", "none", "none"),
        (
            "text-shadow",
            "1px 2px 3px red, 4px 5px rgba(0, 0, 0, 0.5)",
            "rgb(255, 0, 0) 1px 2px 3px, rgba(0, 0, 0, 0.5) 4px 5px 0px",
        ),
        ("text-shadow", "1px 1px", "rgb(0, 0, 0) 1px 1px 0px"),
        ("text-emphasis-style", "filled sesame", "sesame"),
        (
            "text-emphasis",
            "open circle red",
            "open circle rgb(255, 0, 0)",
        ),
        (
            "text-emphasis",
            "dot rgba(0, 0, 0, 0.5)",
            "dot rgba(0, 0, 0, 0.5)",
        ),
        ("text-underline-position", "under", "under"),
        ("text-underline-offset", "auto", "auto"),
        ("text-underline-offset", "3px", "3px"),
        ("text-underline-offset", "10%", "10%"),
        (
            "text-underline-offset",
            "calc(10% + 2px)",
            "calc(10% + 2px)",
        ),
        ("text-decoration-color", "currentcolor", "rgb(0, 0, 0)"),
        (
            "text-decoration-color",
            "rgba(255, 0, 0, 0.5)",
            "rgba(255, 0, 0, 0.5)",
        ),
        ("letter-spacing", "normal", "normal"),
        ("letter-spacing", "1.2345678px", "1.23457px"),
        ("tab-size", "2.3456789", "2.34568"),
        ("text-indent", "12.345678%", "12.3457%"),
        ("letter-spacing", "2px", "2px"),
        ("letter-spacing", "10%", "10%"),
        ("letter-spacing", "calc(10% + 2px)", "calc(10% + 2px)"),
        ("letter-spacing", "calc(10% - 2px)", "calc(10% - 2px)"),
        ("word-spacing", "2px", "2px"),
        ("word-spacing", "10%", "10%"),
        ("word-spacing", "calc(10% + 2px)", "calc(10% + 2px)"),
        ("text-indent", "5px", "5px"),
        ("text-indent", "calc(10% + 2px)", "calc(10% + 2px)"),
        ("text-indent", "10%", "10%"),
        (
            "text-indent",
            "calc(10% - 2px) hanging each-line",
            "calc(10% - 2px) hanging each-line",
        ),
        ("tab-size", "4", "4"),
        ("tab-size", "12px", "12px"),
    ]);
}
