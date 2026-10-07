use super::*;
use bytes::Bytes;
use raikiri_traits::{
    FetchOutcome, FetchedResource, NetworkError, PolicyViolation, ResourceKind, ViolationType,
};
use std::collections::HashMap;
use std::sync::Mutex;

thread_local! {
    static IDENTIFIER_SCAN_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_identifier_scan_step() {
    IDENTIFIER_SCAN_STEPS.with(|steps| steps.set(steps.get() + 1));
}

#[test]
fn unicode_root_and_live_sources_are_preserved_without_fetches() {
    let provider = MapProvider::new(&[]);
    let base = Url::parse("https://example.test/root.css").unwrap();
    let source = "p { --名: 値; --é: 🦀; }";
    let mut warnings = Vec::new();
    assert_eq!(
        expand_stylesheet_imports(source, Some(&base), None, Some(&provider), &mut warnings),
        source
    );
    assert_eq!(
        expand_live_stylesheet_imports(vec![source.to_owned()], Some(&base), Some(&provider)),
        vec![StylesheetSource::new(source.to_owned(), None)]
    );
    assert!(provider.requests.lock().unwrap().is_empty());
    assert!(warnings.is_empty());
}

#[test]
fn unicode_import_responses_pass_the_balance_scanner() {
    let child = "p { --名: 値; --é: 🦀; }";
    let provider = MapProvider::new(&[("https://example.test/child.css", child)]);
    let base = Url::parse("https://example.test/root.css").unwrap();
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        "@import 'child.css';",
        Some(&base),
        None,
        Some(&provider),
        &mut warnings,
    );
    assert!(expanded.contains(child));
    assert!(!expanded.contains("@import"));
    assert!(warnings.is_empty());
}

#[test]
fn unicode_import_media_and_unknown_blocks_preserve_import_order() {
    let imports = scan_leading_imports(
        "@unknown { --名: 値; } @import '子.css' screen and (名:値); p {color:red}",
    );
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].url, "子.css");
    assert_eq!(imports[0].media.as_deref(), Some("screen and (名:値)"));
}

#[test]
fn unicode_import_media_without_a_block_does_not_split_a_character() {
    let imports = scan_leading_imports("@import '子.css' screen and (名:値);");
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].media.as_deref(), Some("screen and (名:値)"));
}

fn assert_linear_identifier_work(malformed: bool) {
    for size in [128, 4096] {
        let source = if malformed {
            format!("{{{}\\", "a".repeat(size))
        } else {
            format!("{{{}:red}}", "a".repeat(size))
        };
        for balance in [false, true] {
            IDENTIFIER_SCAN_STEPS.with(|steps| steps.set(0));
            let accepted = if balance {
                stylesheet_source_is_balanced(&source)
            } else {
                scan_block_end(&source, 0).is_some()
            };
            assert_eq!(accepted, !malformed);
            let steps = IDENTIFIER_SCAN_STEPS.with(std::cell::Cell::get);
            assert!(
                steps <= 2 * source.len(),
                "identifier work {steps} for {} bytes",
                source.len()
            );
        }
    }
}

#[test]
fn identifier_scanning_work_is_linear_for_long_names() {
    assert_linear_identifier_work(false);
}

#[test]
fn identifier_scanning_work_is_linear_on_a_failed_escape() {
    assert_linear_identifier_work(true);
}

#[test]
fn component_scanners_preserve_complete_and_escaped_url_names() {
    for name in ["url", "URL", r"u\72l", r"\75rl"] {
        let source = format!("{{background:{name}(data:,}})}}");
        assert!(stylesheet_source_is_balanced(&source), "{source}");
        assert_eq!(scan_block_end(&source, 0), Some(source.len()), "{source}");
    }
    assert!(!stylesheet_source_is_balanced("{x:myurl(data:,})}"));
    assert!(!stylesheet_source_is_balanced("{x:url('unterminated)}"));
}

#[test]
fn component_scanners_preserve_strings_comments_and_escaped_delimiters() {
    for source in [
        "{x:'名}'; /* 🦀 } */ y:値}",
        r"{x:a\}b}",
        "{x:a\\\nb}",
        "{x:é; y:名; z:🦀}",
    ] {
        assert!(stylesheet_source_is_balanced(source), "{source}");
        assert_eq!(scan_block_end(source, 0), Some(source.len()), "{source}");
    }
}

struct MapProvider {
    responses: Mutex<HashMap<String, String>>,
    requests: Mutex<Vec<String>>,
    max_depth: Option<u32>,
    content_type: Option<String>,
}

impl MapProvider {
    fn new(responses: &[(&str, &str)]) -> Self {
        Self {
            responses: Mutex::new(
                responses
                    .iter()
                    .map(|(url, css)| ((*url).to_owned(), (*css).to_owned()))
                    .collect(),
            ),
            requests: Mutex::new(Vec::new()),
            max_depth: None,
            content_type: Some("text/css".to_owned()),
        }
    }

    fn with_content_type(mut self, content_type: Option<&str>) -> Self {
        self.content_type = content_type.map(str::to_owned);
        self
    }

    fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = Some(max_depth);
        self
    }
}

impl NetworkProvider for MapProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.requests.lock().unwrap().push(request.url.to_string());
        let Some(css) = self
            .responses
            .lock()
            .unwrap()
            .get(request.url.as_str())
            .cloned()
        else {
            return Err(NetworkError::Other("not found".to_owned()));
        };
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: Bytes::from(css),
            content_type: self.content_type.clone(),
            final_url: request.url,
            encoding: None,
        }))
    }

    fn max_import_depth(&self) -> Option<u32> {
        self.max_depth
    }
}

#[test]
fn scanner_preserves_only_leading_imports_and_decodes_urls() {
    let source = r#"/* c */ @import "a.css"; @import url('b.css') screen; p { color: red } @import "late.css";"#;
    let imports = scan_leading_imports(source);
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].url, "a.css");
    assert_eq!(imports[0].media, None);
    assert_eq!(imports[1].url, "b.css");
    assert_eq!(imports[1].media.as_deref(), Some("screen"));
}

#[test]
fn a_stylesheet_bom_does_not_hide_a_leading_import() {
    let imports = scan_leading_imports("\u{feff}@import \"a.css\";");
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].url, "a.css");
}

#[test]
fn unknown_rules_and_empty_layer_statements_do_not_block_imports() {
    let source = r#"@unknown { ignored: true } @layer foo; @import "a.css";"#;
    let imports = scan_leading_imports(source);
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].url, "a.css");

    let blocked = scan_leading_imports(r#"@media print {} @import "a.css";"#);
    assert!(blocked.is_empty());

    let with_url_brace = scan_leading_imports(
        r#"@unknown { value: url(foo}@import-nested.css;); } @import "top.css";"#,
    );
    assert_eq!(with_url_brace.len(), 1);
    assert_eq!(with_url_brace[0].url, "top.css");
}

#[test]
fn layered_and_supports_imports_remain_opaque_for_now() {
    assert!(scan_leading_imports(r#"@import "a.css" layer(foo);"#).is_empty());
    assert!(scan_leading_imports(r#"@import "a.css" supports(display: grid);"#).is_empty());
}

#[test]
fn scanner_handles_escaped_url_and_braces_inside_url() {
    let source = r#"@import url("dir/\7b theme\7d .css");"#;
    let imports = scan_leading_imports(source);
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].url, "dir/{theme}.css");
}

#[test]
fn malformed_import_and_unterminated_prefix_are_not_rewritten() {
    assert!(scan_leading_imports(r#"@import url("bad); @import "later.css";"#).is_empty());
    assert!(scan_leading_imports(r#"@import "a.css"; /* unterminated"#).len() == 1);
}

#[test]
fn expansion_preserves_import_position_and_media_conditions() {
    let provider = MapProvider::new(&[
        ("https://example.test/a.css", "a { color: red }"),
        ("https://example.test/b.css", "b { color: blue }"),
    ]);
    let source = r#"@import "a.css"; @import url("b.css") screen; c { color: green }"#;
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        source,
        Some(&Url::parse("https://example.test/main.css").unwrap()),
        None,
        Some(&provider),
        &mut warnings,
    );
    assert_eq!(
        expanded,
        "a { color: red } b { color: blue } c { color: green }"
    );
    assert_eq!(
        *provider.requests.lock().unwrap(),
        vec![
            "https://example.test/a.css".to_owned(),
            "https://example.test/b.css".to_owned()
        ]
    );
}

#[test]
fn failed_import_remains_opaque_and_cycles_stop() {
    let provider = MapProvider::new(&[(
        "https://example.test/a.css",
        "@import \"main.css\"; a { color: red }",
    )]);
    let source = r#"@import "missing.css"; @import "a.css";"#;
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        source,
        Some(&Url::parse("https://example.test/main.css").unwrap()),
        Some(&Url::parse("https://example.test/main.css").unwrap()),
        Some(&provider),
        &mut warnings,
    );
    assert!(expanded.contains(r#"@import "missing.css";"#));
    assert!(expanded.contains(r#"@import "main.css";"#));
    assert!(expanded.contains("a { color: red }"));
    assert_eq!(
        *provider.requests.lock().unwrap(),
        vec![
            "https://example.test/missing.css".to_owned(),
            "https://example.test/a.css".to_owned(),
        ]
    );
    assert!(
        warnings[0]
            .details
            .contains("provider returned a network error")
    );
    assert!(!warnings[0].details.contains("not found"));
}

#[test]
fn non_css_import_response_remains_opaque() {
    let provider = MapProvider::new(&[("https://example.test/a.css", "a { color: red }")])
        .with_content_type(Some("text/html"));
    let source = r#"@import "a.css"; p { color: blue }"#;
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        source,
        Some(&Url::parse("https://example.test/main.css").unwrap()),
        None,
        Some(&provider),
        &mut warnings,
    );
    assert_eq!(expanded, source);
    assert!(matches!(
        &warnings[0].kind,
        WarningKind::NetworkFallback { .. }
    ));
}

#[test]
fn policy_failures_remain_opaque_and_are_reported_as_policy_warnings() {
    struct PolicyProvider;

    impl NetworkProvider for PolicyProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            Err(NetworkError::PolicyViolation(Box::new(PolicyViolation {
                kind: ResourceKind::StylesheetImport,
                url: request.url,
                violation_type: ViolationType::HostNotAllowed,
                details: "blocked https://user:secret@example.test/token".to_owned(),
            })))
        }
    }

    let provider = PolicyProvider;
    let source = r#"@import "a.css"; p { color: blue }"#;
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        source,
        Some(&Url::parse("https://example.test/main.css").unwrap()),
        None,
        Some(&provider),
        &mut warnings,
    );
    assert_eq!(expanded, source);
    assert!(matches!(
        &warnings[0].kind,
        WarningKind::PolicyWarning { .. }
    ));
    assert_eq!(
        warnings[0].details,
        "stylesheet @import fetch violated network policy for https://example.test/a.css"
    );
    let WarningKind::PolicyWarning { violation } = &warnings[0].kind else {
        unreachable!("matched above");
    };
    assert_eq!(violation.details, "network policy denied the request");
}

#[test]
fn fragments_do_not_bypass_active_import_cycle_detection() {
    let provider = MapProvider::new(&[(
        "https://example.test/a.css",
        r#"@import "main.css#fragment"; a { color: red }"#,
    )]);
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        r#"@import "a.css#fragment";"#,
        Some(&Url::parse("https://example.test/main.css#root").unwrap()),
        Some(&Url::parse("https://example.test/main.css#root").unwrap()),
        Some(&provider),
        &mut warnings,
    );
    assert!(expanded.contains("a { color: red }"));
    assert!(expanded.contains(r#"@import "main.css#fragment";"#));
    assert_eq!(
        *provider.requests.lock().unwrap(),
        vec!["https://example.test/a.css".to_owned()]
    );
}

#[test]
fn malformed_child_is_not_allowed_to_consume_parent_source() {
    let provider = MapProvider::new(&[("https://example.test/a.css", "a { color: red")]);
    let source = r#"@import "a.css"; p { color: blue }"#;
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        source,
        Some(&Url::parse("https://example.test/main.css").unwrap()),
        None,
        Some(&provider),
        &mut warnings,
    );
    assert_eq!(expanded, source);
    assert!(matches!(
        &warnings[0].kind,
        WarningKind::NetworkFallback { .. }
    ));
}

#[test]
fn malformed_url_tokens_are_not_fetched() {
    assert!(scan_leading_imports(r#"@import url(foo bar);"#).is_empty());
    assert!(scan_leading_imports(r#"@import url(foo(bar));"#).is_empty());
    assert!(scan_leading_imports(r#"@import url(foo) screen,,print;"#).is_empty());
    assert_eq!(
        scan_leading_imports(r#"@import url(foo\ bar);"#)[0].url,
        "foo bar"
    );
    assert_eq!(
        scan_leading_imports(r#"@import url(/**/foo.css);"#)[0].url,
        "/**/foo.css"
    );
    assert_eq!(
        scan_leading_imports(r#"@import url( /*x*/foo.css);"#)[0].url,
        "/*x*/foo.css"
    );
}

#[test]
fn nested_imports_use_redirected_final_url_as_their_base() {
    struct RedirectProvider {
        requests: Mutex<Vec<String>>,
    }

    impl NetworkProvider for RedirectProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            let requested = request.url.to_string();
            self.requests.lock().unwrap().push(requested.clone());
            match requested.as_str() {
                "https://origin.test/styles/nested.css" => {
                    Ok(FetchOutcome::Body(FetchedResource {
                        bytes: Bytes::from_static(b"@import \"grand.css\"; .nested { color: red }"),
                        content_type: Some("text/css".to_owned()),
                        final_url: Url::parse("https://cdn.test/assets/nested.css").unwrap(),
                        encoding: None,
                    }))
                }
                "https://cdn.test/assets/grand.css" => Ok(FetchOutcome::Body(FetchedResource {
                    bytes: Bytes::from_static(b".grand { color: blue }"),
                    content_type: Some("text/css".to_owned()),
                    final_url: Url::parse("https://cdn.test/assets/grand.css").unwrap(),
                    encoding: None,
                })),
                _ => Err(NetworkError::Other("not found".to_owned())),
            }
        }
    }

    let provider = RedirectProvider {
        requests: Mutex::new(Vec::new()),
    };
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        r#"@import "nested.css";"#,
        Some(&Url::parse("https://origin.test/styles/main.css").unwrap()),
        Some(&Url::parse("https://origin.test/styles/main.css").unwrap()),
        Some(&provider),
        &mut warnings,
    );
    assert!(expanded.contains(".grand { color: blue }"));
    assert!(expanded.contains(".nested { color: red }"));
    assert_eq!(
        *provider.requests.lock().unwrap(),
        vec![
            "https://origin.test/styles/nested.css".to_owned(),
            "https://cdn.test/assets/grand.css".to_owned(),
        ]
    );
}

#[test]
fn child_late_import_does_not_block_a_following_parent_import() {
    let provider = MapProvider::new(&[
        (
            "https://example.test/a.css",
            r#"a { color: red } @import "late.css";"#,
        ),
        ("https://example.test/b.css", "b { color: blue }"),
    ]);
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        r#"@import "a.css"; @import "b.css";"#,
        Some(&Url::parse("https://example.test/main.css").unwrap()),
        None,
        Some(&provider),
        &mut warnings,
    );
    assert!(expanded.contains("a { color: red }"));
    assert!(expanded.contains("b { color: blue }"));
    assert!(expanded.contains(r#"@import "late.css";"#));
    assert_eq!(
        *provider.requests.lock().unwrap(),
        vec![
            "https://example.test/a.css".to_owned(),
            "https://example.test/b.css".to_owned(),
        ]
    );
}

#[test]
fn provider_import_depth_limit_overrides_the_fallback_bound() {
    let provider = MapProvider::new(&[(
        "https://example.test/a.css",
        r#"@import "b.css"; a { color: red }"#,
    )])
    .with_max_depth(1);
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        r#"@import "a.css";"#,
        Some(&Url::parse("https://example.test/main.css").unwrap()),
        Some(&Url::parse("https://example.test/main.css").unwrap()),
        Some(&provider),
        &mut warnings,
    );
    assert!(expanded.contains(r#"@import "b.css";"#));
    assert!(expanded.contains("a { color: red }"));
    assert_eq!(
        *provider.requests.lock().unwrap(),
        vec!["https://example.test/a.css".to_owned()]
    );
}

#[test]
fn shared_budget_caps_imports_across_independent_roots() {
    let provider = MapProvider::new(&[("https://example.test/a.css", ".a { color: red }")]);
    let mut warnings = Vec::new();
    let mut budget = ImportBudget::default();
    let source = r#"@import "a.css";"#;
    let base = Url::parse("https://example.test/root.css").unwrap();

    for _ in 0..(MAX_IMPORT_FETCHES + 8) {
        let _ = expand_stylesheet_imports_with_budget(
            source,
            Some(&base),
            None,
            Some(&provider),
            &mut warnings,
            &mut budget,
        );
    }

    assert_eq!(
        provider.requests.lock().unwrap().len(),
        MAX_IMPORT_FETCHES,
        "independent stylesheet roots must share the document import budget"
    );
}

#[test]
fn shared_budget_stops_fetching_after_cumulative_response_limit() {
    struct LargeProvider {
        response: Bytes,
        requests: Mutex<usize>,
    }

    impl NetworkProvider for LargeProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            *self.requests.lock().unwrap() += 1;
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: self.response.clone(),
                content_type: Some("text/css".to_owned()),
                final_url: request.url,
                encoding: None,
            }))
        }
    }

    let provider = LargeProvider {
        response: Bytes::from(vec![b' '; MAX_IMPORT_RESPONSE_BYTES]),
        requests: Mutex::new(0),
    };
    let mut warnings = Vec::new();
    let mut budget = ImportBudget::default();
    let source = r#"@import "a.css";"#;
    let base = Url::parse("https://example.test/root.css").unwrap();

    for _ in 0..3 {
        let _ = expand_stylesheet_imports_with_budget(
            source,
            Some(&base),
            None,
            Some(&provider),
            &mut warnings,
            &mut budget,
        );
    }

    assert_eq!(*provider.requests.lock().unwrap(), 2);
    assert_eq!(budget.response_bytes, MAX_IMPORT_RESPONSE_BYTES_TOTAL);
    assert_eq!(budget.expansion_bytes, MAX_IMPORT_EXPANSION_BYTES);
}

#[test]
fn depth_limit_leaves_the_next_import_untouched() {
    let urls: Vec<String> = (0..=MAX_IMPORT_DEPTH + 1)
        .map(|depth| format!("https://example.test/{depth}.css"))
        .collect();
    let responses: Vec<(&str, &str)> = urls
        .windows(2)
        .map(|pair| {
            // Leak only test fixture strings; the process owns them for the
            // duration of this test.
            let css = format!("@import url(\"{}\");", pair[1]);
            let url: &'static str = Box::leak(pair[0].clone().into_boxed_str());
            let css: &'static str = Box::leak(css.into_boxed_str());
            (url, css)
        })
        .collect();
    let provider = MapProvider::new(&responses);
    let source = format!("@import url(\"{}\");", urls[0]);
    let mut warnings = Vec::new();
    let expanded = expand_stylesheet_imports(
        &source,
        Some(&Url::parse("https://example.test/root.css").unwrap()),
        Some(&Url::parse("https://example.test/root.css").unwrap()),
        Some(&provider),
        &mut warnings,
    );
    assert!(expanded.contains("@import"));
    assert_eq!(
        provider.requests.lock().unwrap().len(),
        MAX_IMPORT_DEPTH as usize
    );
}

#[test]
fn media_import_preserves_child_namespace_for_supports() {
    let provider = MapProvider::new(&[(
        "https://example.test/child.css",
        "@namespace svg 'http://www.w3.org/2000/svg'; @supports selector(svg|rect) {p {display:none}}",
    )]);
    let document = imported_document(
        &provider,
        "<style>@import 'child.css' print;</style><p>x</p>",
        &[],
    );
    let tree = crate::build_rule_tree(&document);
    assert_eq!(
        element_display(&document, &tree, &raikiri_style::MediaContext::print(), "p"),
        raikiri_style::DisplayValue::None
    );
    assert_eq!(
        element_display(
            &document,
            &tree,
            &raikiri_style::MediaContext::screen(),
            "p"
        ),
        raikiri_style::DisplayValue::Block
    );
}

fn imported_document(
    provider: &MapProvider,
    html: &str,
    extra: &[&str],
) -> crate::UncascadedDocument {
    crate::parse(
        html.as_bytes(),
        &crate::ParseOptions {
            extra_stylesheets: extra,
            network: Some(provider),
            base_url: Some(Url::parse("https://example.test/root.html").unwrap()),
        },
    )
    .unwrap()
}

fn element_display(
    document: &crate::UncascadedDocument,
    tree: &raikiri_style::RuleTree,
    context: &raikiri_style::MediaContext,
    tag: &str,
) -> raikiri_style::DisplayValue {
    let index = (0..document.dom.node_count())
        .find(|&index| {
            document
                .dom
                .get_node(index)
                .and_then(|node| node.tag_name())
                == Some(tag)
        })
        .unwrap();
    raikiri_style::cascade_with_media_context(&document.dom, tree, context)
        .unwrap()
        .computed[index]
        .display
}

#[test]
fn nested_import_media_lists_keep_conjunctions_and_descriptor_collections() {
    let provider = MapProvider::new(&[
        (
            "https://example.test/child.css",
            "@import 'grand.css' (min-width:500px), screen;",
        ),
        (
            "https://example.test/grand.css",
            "@font-face {font-family:Imported;src:url(font.woff)} \
         @counter-style imported {system:cyclic;symbols:'*'} \
         @page {margin:1in} @layer imported {@supports (display:block) {p {display:none}}}",
        ),
    ]);
    let document = imported_document(
        &provider,
        "<style media='(max-width:800px)'>@import 'child.css' print, (min-width:900px);</style><p>x</p>",
        &[],
    );
    let tree = crate::build_rule_tree(&document);
    use raikiri_style::{DisplayValue, MediaContext, MediaType};
    for (kind, width, active) in [
        (MediaType::Print, 600, true),
        (MediaType::Print, 400, false),
        (MediaType::Print, 850, false),
        (MediaType::Screen, 600, false),
        (MediaType::Screen, 1000, false),
    ] {
        let context = MediaContext::with_viewport(kind, width, 1000);
        assert_eq!(
            tree.font_faces_for(&context).get("Imported").is_some(),
            active
        );
        assert_eq!(
            tree.counter_styles_for(&context).get("imported").is_some(),
            active
        );
        assert_eq!(
            element_display(&document, &tree, &context, "p"),
            if active {
                DisplayValue::None
            } else {
                DisplayValue::Block
            }
        );
        let page = raikiri_style::cascade_page_with_media_context(
            &tree,
            &raikiri_style::PageContextQuery::default(),
            raikiri_style::PageInheritance::LegacyInitialValues,
            &context,
        );
        assert_eq!(
            page.declarations()
                .contains_key(&raikiri_style::PropertyKey::MarginTop),
            active
        );
    }
    assert_eq!(tree.page_rules.len(), 1);
}

#[test]
fn false_import_media_does_not_change_layer_order() {
    let provider = MapProvider::new(&[("https://example.test/child.css", "@layer b,a;")]);
    let document = imported_document(
        &provider,
        "<style>@import 'child.css' screen; @layer a,b; @layer a {p {display:none}} @layer b {p {display:inline}}</style><p>x</p>",
        &[],
    );
    let tree = crate::build_rule_tree(&document);
    assert_eq!(
        element_display(&document, &tree, &raikiri_style::MediaContext::print(), "p"),
        raikiri_style::DisplayValue::Inline
    );
    assert_eq!(
        element_display(
            &document,
            &tree,
            &raikiri_style::MediaContext::screen(),
            "p"
        ),
        raikiri_style::DisplayValue::None
    );
}

#[test]
fn malformed_and_impossible_import_conditions_drop_all_rule_kinds() {
    let provider = MapProvider::new(&[(
        "https://example.test/child.css",
        "@font-face {font-family:Hidden;src:url(font.woff)} @counter-style hidden {system:cyclic;symbols:'*'} \
         @page {margin:1in} @layer b,a; @supports (display:block) {p {display:none}}",
    )]);
    for media in ["not all", "print,", "(hover:hover)"] {
        let document = imported_document(
            &provider,
            &format!(
                "<style>@import 'child.css' {media}; @layer a,b; @layer a {{p {{display:none}}}} @layer b {{p {{display:inline}}}}</style><p>x</p>"
            ),
            &[],
        );
        let tree = crate::build_rule_tree(&document);
        assert!(tree.page_rules.is_empty());
        for context in [
            raikiri_style::MediaContext::print(),
            raikiri_style::MediaContext::screen(),
        ] {
            assert!(tree.font_faces_for(&context).get("Hidden").is_none());
            assert!(tree.counter_styles_for(&context).get("hidden").is_none());
            assert_eq!(
                element_display(&document, &tree, &context, "p"),
                raikiri_style::DisplayValue::Inline
            );
        }
    }
}

#[test]
fn imports_preserve_root_order_and_user_origin_with_empty_element_media() {
    let provider = MapProvider::new(&[("https://example.test/child.css", "p {display:none}")]);
    let document = imported_document(
        &provider,
        "<style media='  '>@import 'child.css' print; p {display:inline}</style><p>x</p>",
        &["@import 'child.css' print;"],
    );
    let tree = crate::build_rule_tree(&document);
    for context in [
        raikiri_style::MediaContext::print(),
        raikiri_style::MediaContext::screen(),
    ] {
        assert_eq!(
            element_display(&document, &tree, &context, "p"),
            raikiri_style::DisplayValue::Inline
        );
    }
}

#[test]
fn imported_namespaces_do_not_leak_between_child_and_parent_sheets() {
    let provider = MapProvider::new(&[(
        "https://example.test/child.css",
        "@namespace x 'urn:child'; @supports selector(x|p) {p {display:none}}",
    )]);
    let document = imported_document(
        &provider,
        "<style>@import 'child.css'; @supports selector(x|p) {p {display:inline}}</style><p>x</p>",
        &[],
    );
    let tree = crate::build_rule_tree(&document);
    assert_eq!(
        element_display(&document, &tree, &raikiri_style::MediaContext::print(), "p"),
        raikiri_style::DisplayValue::None
    );
}

#[test]
fn live_import_expansion_preserves_sheet_namespace_and_media() {
    let provider = MapProvider::new(&[(
        "https://example.test/child.css",
        "@namespace h 'http://www.w3.org/1999/xhtml'; @supports selector(h|p) {p {display:none}}",
    )]);
    let mut document = imported_document(&provider, "<p>x</p>", &[]);
    document.stylesheet_sources = expand_live_stylesheet_imports(
        vec!["@import 'child.css' print;".to_owned()],
        Some(&Url::parse("https://example.test/root.html").unwrap()),
        Some(&provider),
    );
    let tree = crate::build_rule_tree(&document);
    assert_eq!(
        element_display(&document, &tree, &raikiri_style::MediaContext::print(), "p"),
        raikiri_style::DisplayValue::None
    );
    assert_eq!(
        element_display(
            &document,
            &tree,
            &raikiri_style::MediaContext::screen(),
            "p"
        ),
        raikiri_style::DisplayValue::Block
    );
}

#[test]
fn imported_user_important_declarations_outrank_author_important_declarations() {
    let provider = MapProvider::new(&[(
        "https://example.test/child.css",
        "p {display:none!important}",
    )]);
    let document = imported_document(
        &provider,
        "<style>p {display:inline!important}</style><p>x</p>",
        &["@import 'child.css' print;"],
    );
    let tree = crate::build_rule_tree(&document);
    assert_eq!(
        element_display(&document, &tree, &raikiri_style::MediaContext::print(), "p"),
        raikiri_style::DisplayValue::None
    );
    assert_eq!(
        element_display(
            &document,
            &tree,
            &raikiri_style::MediaContext::screen(),
            "p"
        ),
        raikiri_style::DisplayValue::Inline
    );
}

#[test]
fn import_media_bytes_count_toward_the_expansion_budget() {
    let provider = MapProvider::new(&[("https://example.test/child.css", "p{display:none}")]);
    let base = Url::parse("https://example.test/root.css").unwrap();
    let mut warnings = Vec::new();
    let mut budget = ImportBudget {
        expansion_bytes: MAX_IMPORT_EXPANSION_BYTES - "p{display:none}".len(),
        ..ImportBudget::default()
    };
    let source = "@import 'child.css' print;";
    let parts = expand_stylesheet_imports_with_budget(
        source,
        Some(&base),
        None,
        Some(&provider),
        &mut warnings,
        &mut budget,
    );
    assert_eq!(
        parts,
        vec![StylesheetPart {
            source: source.to_owned(),
            media: Vec::new()
        }]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn parent_import_media_budget_prevents_allocation_amplification() {
    const CHILD_MARKER: &str = "RAIKIRI_IMPORT_MEDIA_MEMORY_CHILD";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "import::tests::parent_import_media_budget_prevents_allocation_amplification",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_MARKER, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated memory regression failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    fn peak_resident_bytes() -> usize {
        std::fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse::<usize>()
            .unwrap()
            * 1024
    }

    // Interstitial whitespace creates separate source-ordered parts. The
    // input stays small while repeated parent media would exceed 16 MiB.
    let child = "@import 'grand.css'; ".repeat(255);
    let provider = MapProvider::new(&[
        ("https://example.test/child.css", &child),
        ("https://example.test/grand.css", "p{display:none}"),
    ]);
    let source = format!("@import 'child.css' print /*{}*/;", "x".repeat(64 * 1024));
    let base = Url::parse("https://example.test/root.css").unwrap();
    let mut warnings = Vec::new();
    let mut budget = ImportBudget::default();
    let before = peak_resident_bytes();
    let parts = expand_stylesheet_imports_with_budget(
        &source,
        Some(&base),
        None,
        Some(&provider),
        &mut warnings,
        &mut budget,
    );
    let growth = peak_resident_bytes().saturating_sub(before);
    println!("rejected import peak resident memory growth: {growth} bytes");
    assert_eq!(
        parts,
        vec![StylesheetPart {
            source,
            media: Vec::new()
        }]
    );
    assert!(
        growth <= 16 * 1024 * 1024,
        "rejected import grew peak resident memory by {growth} bytes"
    );
}

#[test]
fn credential_bearing_root_stylesheet_does_not_resolve_even_safe_child_urls() {
    let provider = MapProvider::new(&[("https://example.test/child.css", "p {display:none}")]);
    let base = Url::parse("https://example.test/root.css").unwrap();
    let root = Url::parse("https://user:secret@example.test/root.css").unwrap();
    let source = "@import 'child.css'; p {display:inline}";
    let mut warnings = Vec::new();
    let mut budget = ImportBudget::default();
    let parts = expand_stylesheet_imports_with_budget(
        source,
        Some(&base),
        Some(&root),
        Some(&provider),
        &mut warnings,
        &mut budget,
    );
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].source, source);
    assert!(provider.requests.lock().unwrap().is_empty());
    assert!(warnings.is_empty());
}

#[test]
fn live_sheets_without_a_provider_keep_unresolved_imports_and_cascade_in_order() {
    let mut document = crate::parse(
        &b"<p>x</p>"[..],
        &crate::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let first = "@import 'missing.css' print; p {display:none}";
    let second = "p {display:inline}";
    document.stylesheet_sources =
        expand_live_stylesheet_imports(vec![first.to_owned(), second.to_owned()], None, None);
    assert_eq!(document.stylesheet_sources.len(), 2);
    assert_eq!(document.stylesheet_sources[0].parts[0].source, first);
    assert_eq!(document.stylesheet_sources[1].parts[0].source, second);
    let tree = crate::build_rule_tree(&document);
    assert!(
        tree.opaque_at_rules()
            .iter()
            .any(|rule| rule.name == "import")
    );
    for context in [
        raikiri_style::MediaContext::print(),
        raikiri_style::MediaContext::screen(),
    ] {
        assert_eq!(
            element_display(&document, &tree, &context, "p"),
            raikiri_style::DisplayValue::Inline
        );
    }
}
