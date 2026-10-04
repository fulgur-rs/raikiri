//! Public API regressions for selector matching during initial SVG parsing.

use raikiri_svg::{SvgDocument, SvgRootStyle, SvgViewport};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

fn nested_groups(count: usize, leaf: &str) -> String {
    let mut body = String::new();
    for _ in 0..count {
        body.push_str("<g>");
    }
    body.push_str(leaf);
    for _ in 0..count {
        body.push_str("</g>");
    }
    body
}

fn rasterize_pixel(source: &str) -> [u8; 4] {
    let svg = SvgDocument::parse(source.as_bytes()).expect("ordinary SVG parses");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle::default(),
            Some(4),
        )
        .expect("ordinary SVG rasterizes");
    image.rgba[..4].try_into().expect("one RGBA pixel")
}

#[test]
fn initial_parse_rejects_expensive_descendant_selector_backtracking() {
    let selector = std::iter::once("missing")
        .chain(std::iter::repeat_n("g", 8))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!(
        "<svg xmlns=\"{SVG_NAMESPACE}\"><style>{selector} {{ fill:red }}</style>{}</svg>",
        nested_groups(12, "<rect width=\"1\" height=\"1\"/>")
    );

    let error = match SvgDocument::parse(source.as_bytes()) {
        Ok(_) => panic!("initial parsing must stop bounded selector backtracking"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("selector matching resource limit")
    );
}

#[test]
fn initial_parse_shares_selector_budget_across_repeated_use_expansion() {
    let stylesheets = "g { fill:red }".repeat(768);
    let template = nested_groups(12, "<rect width=\"1\" height=\"1\"/>");
    let uses = "<use href=\"#template\"/>".repeat(512);
    let source = format!(
        "<svg xmlns=\"{SVG_NAMESPACE}\"><style>{stylesheets}</style><defs><g id=\"template\">{template}</g></defs>{uses}</svg>"
    );

    let error = match SvgDocument::parse(source.as_bytes()) {
        Ok(_) => panic!("repeated use expansion must share the initial parse budget"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("selector matching resource limit")
    );
}

#[test]
fn initial_parse_rejects_excessive_selector_rule_count() {
    let maximum_stylesheets = "g { fill:red }".repeat(4_096);
    let maximum_source =
        format!("<svg xmlns=\"{SVG_NAMESPACE}\"><style>{maximum_stylesheets}</style><g/></svg>");
    assert!(
        SvgDocument::parse(maximum_source.as_bytes()).is_ok(),
        "the configured selector count limit remains supported"
    );

    let stylesheets = "g { fill:red }".repeat(4_097);
    let source = format!("<svg xmlns=\"{SVG_NAMESPACE}\"><style>{stylesheets}</style><g/></svg>");

    let error = match SvgDocument::parse(source.as_bytes()) {
        Ok(_) => panic!("initial parsing must cap retained selector rules"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("selector matching resource limit")
    );
}

#[test]
fn selector_capacity_ignores_commas_in_css_comments() {
    let stylesheet = format!("/*{}*/ rect {{ fill:red }}", ",".repeat(4_097));
    let source = format!("<svg xmlns=\"{SVG_NAMESPACE}\"><style>{stylesheet}</style><rect/></svg>");

    assert!(
        SvgDocument::parse(source.as_bytes()).is_ok(),
        "comment punctuation must not consume selector capacity"
    );
}

#[test]
fn initial_parse_rejects_selector_declaration_clone_amplification() {
    let selectors = std::iter::repeat_n("g", 4_096)
        .collect::<Vec<_>>()
        .join(",");
    let declarations = "fill:red;".repeat(128);
    let source = format!(
        "<svg xmlns=\"{SVG_NAMESPACE}\"><style>{selectors} {{{declarations}}}</style><g/></svg>"
    );

    let error = match SvgDocument::parse(source.as_bytes()) {
        Ok(_) => panic!("initial parsing must bound selector declaration expansion"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("selector matching resource limit")
    );
}

#[test]
fn initial_parse_bounds_simplecss_recovery_after_invalid_declarations() {
    let selectors = std::iter::repeat_n("g", 2_048)
        .collect::<Vec<_>>()
        .join(",");
    let declarations = "fill:red;".repeat(128);
    let stylesheet = format!("g {{ fill: @ /* }},{selectors} {{{declarations}}} */ }}");
    let source = format!("<svg xmlns=\"{SVG_NAMESPACE}\"><style>{stylesheet}</style><g/></svg>");

    let error = match SvgDocument::parse(source.as_bytes()) {
        Ok(_) => panic!("initial parsing must bound CSS parser recovery work"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("selector matching resource limit")
    );
}

#[test]
fn ordinary_css_and_use_rendering_remain_supported() {
    let source = format!(
        "<svg xmlns=\"{SVG_NAMESPACE}\" width=\"1\" height=\"1\"><style>rect {{ fill:red }}</style><defs><g id=\"shape\"><rect width=\"1\" height=\"1\"/></g></defs><use href=\"#shape\"/></svg>"
    );

    assert_eq!(rasterize_pixel(&source), [255, 0, 0, 255]);
}
