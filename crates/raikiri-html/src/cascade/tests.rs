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
