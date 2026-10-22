use super::*;

#[test]
fn namespace_selectors_match_real_html_svg_and_mathml_elements() {
    for (css, expected) in [
        (
            "@namespace svg 'http://www.w3.org/2000/svg'; svg|rect {display:block}",
            [false, true, false],
        ),
        (
            "@namespace 'http://www.w3.org/2000/svg'; rect {display:block}",
            [false, true, false],
        ),
        (
            "@namespace html 'http://www.w3.org/1999/xhtml'; html|rect {display:block}",
            [true, false, false],
        ),
        ("|rect {display:block}", [false; 3]),
        ("*|rect {display:block}", [true; 3]),
    ] {
        let html = format!(
            "<style>{css}</style><rect id=html></rect><svg><rect id=svg></rect></svg><math><rect id=math></rect></math>"
        );
        let doc = crate::parse(
            html.as_bytes(),
            &crate::ParseOptions {
                extra_stylesheets: &[],
                network: None,
                base_url: None,
            },
        )
        .unwrap();
        let result = build_cascaded(&doc).expect("the cascade succeeds");
        for (name, matches) in ["html", "svg", "math"].into_iter().zip(expected) {
            let index = (0..doc.dom.node_count())
                .find(|&index| {
                    doc.dom
                        .get_node(index)
                        .and_then(|node| node.attribute("id"))
                        == Some(name)
                })
                .unwrap();
            assert_eq!(
                result.computed[index].display,
                if matches {
                    raikiri_style::DisplayValue::Block
                } else {
                    raikiri_style::DisplayValue::Inline
                },
                "{name} element, stylesheet {css}"
            );
        }
    }
}

#[test]
fn stylesheet_kind_to_origin_matches_spec() {
    assert_eq!(
        stylesheet_kind_to_origin(StylesheetKind::UserAgent),
        Origin::UserAgent,
    );
    assert_eq!(
        stylesheet_kind_to_origin(StylesheetKind::User),
        Origin::User,
    );
    assert_eq!(
        stylesheet_kind_to_origin(StylesheetKind::Author),
        Origin::Author,
    );
}

fn parsed_extra_stylesheet(source: &str) -> UncascadedDocument {
    crate::parse(
        &b"<p>x</p>"[..],
        &crate::ParseOptions {
            extra_stylesheets: &[source],
            network: None,
            base_url: None,
        },
    )
    .unwrap()
}

fn paragraph_display(doc: &UncascadedDocument) -> raikiri_style::DisplayValue {
    let index = (0..doc.dom.node_count())
        .find(|&index| doc.dom.get_node(index).and_then(|node| node.tag_name()) == Some("p"))
        .unwrap();
    build_cascaded(doc).expect("the cascade succeeds").computed[index].display
}

#[test]
fn later_dom_user_stylesheets_follow_parsed_extra_stylesheets() {
    let mut doc = parsed_extra_stylesheet("p {display:block}");
    doc.dom
        .add_stylesheet("p {display:none}", StylesheetKind::User);
    assert_eq!(paragraph_display(&doc), raikiri_style::DisplayValue::None);
}

#[test]
fn parsed_extra_layer_order_precedes_later_dom_user_stylesheets() {
    let mut doc = parsed_extra_stylesheet("@layer a,b;");
    doc.dom.add_stylesheet(
        "@layer b,a; @layer a {p {display:none}} @layer b {p {display:inline}}",
        StylesheetKind::User,
    );
    assert_eq!(paragraph_display(&doc), raikiri_style::DisplayValue::Inline);
}

#[test]
fn build_rule_tree_with_limits_reports_a_passed_limit() {
    let doc = parsed_extra_stylesheet("p {display:block}");
    let mut limits = raikiri_style::RuleTreeLimits::default();
    limits.max_selectors = Some(0);
    let Err(error) = build_rule_tree_with_limits(&doc, &[], &limits) else {
        panic!("the user agent's rules hold selectors");
    };
    assert!(
        matches!(
            error,
            raikiri_style::CascadeError::LimitExceeded {
                kind: raikiri_style::CascadeLimitKind::StyleSelectors,
                limit: 0,
                ..
            }
        ),
        "{error:?}"
    );
    // The tree built within the default limits holds the same rules.
    let tree = build_rule_tree_with_limits(&doc, &[], &raikiri_style::RuleTreeLimits::default())
        .expect("the default limits admit the document");
    assert_eq!(
        tree.style_rules().len(),
        build_rule_tree(&doc).style_rules().len()
    );
}

#[test]
fn build_cascaded_with_options_reports_a_passed_limit() {
    let doc = parsed_extra_stylesheet("p {display:block}");
    let mut options = raikiri_style::CascadeOptions::default();
    options.limits.max_selector_tests = Some(0);
    let error = build_cascaded_with_options(
        &doc,
        &MediaContext::default(),
        &PageContextQuery::default(),
        &[],
        &options,
    )
    .expect_err("every element is tested against the user agent's selectors");
    assert!(
        matches!(
            error,
            raikiri_style::CascadeError::LimitExceeded {
                kind: raikiri_style::CascadeLimitKind::SelectorTests,
                limit: 0,
                ..
            }
        ),
        "{error:?}"
    );
    options.limits.max_selector_tests = None;
    let result = build_cascaded_with_options(
        &doc,
        &MediaContext::default(),
        &PageContextQuery::default(),
        &[],
        &options,
    )
    .expect("no selector test limit");
    assert_eq!(result.computed.len(), doc.dom.node_count());
}

fn uncached_rule_tree(
    doc: &UncascadedDocument,
    consumer: &[ConsumerPropertyRegistration],
    limits: RuleTreeLimits,
) -> RuleTree {
    let mut tree = RuleTree::empty_with_consumer_properties(consumer).with_limits(limits);
    let mut sheets = doc.dom.stylesheets();
    for (source, kind) in sheets.by_ref().take(doc.user_stylesheet_insertion_index) {
        tree.add_stylesheet(source, stylesheet_kind_to_origin(kind));
    }
    for sheet in &doc.user_stylesheet_sources {
        add_sheet(&mut tree, sheet, Origin::User);
    }
    for (source, kind) in sheets {
        tree.add_stylesheet(source, stylesheet_kind_to_origin(kind));
    }
    for sheet in &doc.stylesheet_sources {
        add_sheet(&mut tree, sheet, Origin::Author);
    }
    tree
}

fn assert_same_cascade(doc: &UncascadedDocument, actual: &RuleTree, expected: &RuleTree) {
    use cssparser::ToCss;
    assert_eq!(actual.rules(), expected.rules());
    assert_eq!(actual.style_rules().len(), expected.style_rules().len());
    for (a, b) in actual.style_rules().iter().zip(expected.style_rules()) {
        assert_eq!(a.selectors.to_css_string(), b.selectors.to_css_string());
        assert_eq!(a.declarations(), b.declarations());
        assert_eq!((a.source_order, a.origin), (b.source_order, b.origin));
    }
    for media in [MediaContext::print(), MediaContext::default()] {
        let a = raikiri_style::cascade_with_media_context(&doc.dom, actual, &media).unwrap();
        let b = raikiri_style::cascade_with_media_context(&doc.dom, expected, &media).unwrap();
        assert_eq!(a.computed, b.computed);
        assert_eq!(a.page_values, b.page_values);
    }
}

#[test]
fn prepared_ua_preserves_author_namespace_media_layer_and_user_precedence() {
    let doc = crate::parse(
        b"<!doctype html><style>@namespace svg 'http://www.w3.org/2000/svg'; svg|rect {display:block}</style><style>@layer a,b; @layer b {p {color:red}} @layer a {p {color:blue}} @media print {p {font-size:24px}}</style><p>x</p><svg><rect/></svg>".as_slice(),
        &crate::ParseOptions {extra_stylesheets: &["p {color:green !important}"], network: None, base_url: None},
    ).unwrap();
    let expected = uncached_rule_tree(&doc, &[], RuleTreeLimits::default());
    for _ in 0..2 {
        assert_same_cascade(&doc, &build_rule_tree(&doc), &expected);
    }
}

#[test]
fn prepared_ua_does_not_mix_consumer_registrations() {
    let doc = parsed_extra_stylesheet("p {display:none}");
    for registrations in [
        vec![ConsumerPropertyRegistration::keyword(
            "display",
            &["block", "none"],
        )],
        vec![ConsumerPropertyRegistration::integer("display")],
        vec![],
    ] {
        let actual = build_rule_tree_with_consumer_properties(&doc, &registrations);
        let expected = uncached_rule_tree(&doc, &registrations, RuleTreeLimits::default());
        assert_same_cascade(&doc, &actual, &expected);
        assert_eq!(actual.consumer_property_registrations(), registrations);
    }
}

#[test]
fn prepared_ua_preserves_low_and_nondefault_limit_errors() {
    let doc = parsed_extra_stylesheet("p {color:green; margin:2px}");
    for limit in [0, 1, 86, 87, 88, 100, 200] {
        for field in 0..4 {
            let mut limits = RuleTreeLimits::default();
            match field {
                0 => limits.max_rules = Some(limit),
                1 => limits.max_selectors = Some(limit),
                2 => limits.max_declarations = Some(limit),
                _ => {
                    limits.max_rules = Some(limit);
                    limits.max_selectors = Some(limit);
                    limits.max_declarations = Some(limit);
                }
            }
            let expected = uncached_rule_tree(&doc, &[], limits);
            match (
                build_rule_tree_with_limits(&doc, &[], &limits),
                expected.limit_exceeded(),
            ) {
                (Ok(actual), None) => assert_same_cascade(&doc, &actual, &expected),
                (Err(a), Some(b)) => assert_eq!(format!("{a:?}"), format!("{b:?}")),
                _ => panic!("limit outcome changed"),
            }
        }
    }
}

#[test]
fn prepared_ua_trees_are_independent_and_preceding_sources_keep_their_order() {
    let doc = parsed_extra_stylesheet("p {color:green}");
    let mut a = build_rule_tree(&doc);
    let b = build_rule_tree(&doc);
    a.add_stylesheet("p {color:blue}", Origin::Author);
    let expected = uncached_rule_tree(&doc, &[], RuleTreeLimits::default());
    assert_same_cascade(&doc, &b, &expected);
    assert!(a.style_rules().len() > b.style_rules().len());

    for (sources, before_user) in [
        (
            vec![
                ("p {color:red}", StylesheetKind::UserAgent),
                (crate::MINIMAL_UA_CSS, StylesheetKind::UserAgent),
            ],
            2,
        ),
        (vec![(crate::MINIMAL_UA_CSS, StylesheetKind::Author)], 1),
        (vec![(crate::MINIMAL_UA_CSS, StylesheetKind::UserAgent)], 0),
        (vec![], 1),
    ] {
        let mut doc = parsed_extra_stylesheet("p {color:green}");
        let mut dom = raikiri_dom::Document::new();
        let p = dom.create_detached_element("p").unwrap();
        dom.append_child(dom.root_index(), p).unwrap();
        for (source, kind) in sources {
            dom.add_stylesheet(source, kind);
        }
        doc.dom = dom;
        doc.user_stylesheet_insertion_index = before_user;
        assert_same_cascade(
            &doc,
            &build_rule_tree(&doc),
            &uncached_rule_tree(&doc, &[], RuleTreeLimits::default()),
        );
    }
}
