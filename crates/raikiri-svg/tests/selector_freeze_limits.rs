//! Public API regression coverage for SVG selector rewrite limits.

use raikiri_svg::{SvgDocument, SvgRootStyle, SvgViewport};

#[test]
fn excessive_selector_matches_are_rejected_through_the_public_api() {
    let stylesheets = "rect { fill:red }".repeat(257);
    let elements = "<rect width=\"1\" height=\"1\"/>".repeat(256);
    let source = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><style>{stylesheets}</style>{elements}</svg>"
    );
    let svg = SvgDocument::parse(source.as_bytes()).expect("supported SVG");

    let result = svg.rasterize(
        SvgViewport {
            width: 1.0,
            height: 1.0,
        },
        SvgRootStyle::default(),
        Some(4),
    );

    let error = result.expect_err("the selector match budget must be enforced");
    assert!(
        error
            .to_string()
            .contains("selector freezing resource limit")
    );
}

#[test]
fn root_style_rewrite_budget_is_enforced_through_the_public_api() {
    // One selector-freeze match leaves one fewer unit than root-style needs:
    // its 65,536 scope insertions fit a fresh budget but exceed the shared one.
    let descendants = format!(
        "<rect id=\"target\" width=\"1\" height=\"1\"/>{}",
        "<rect width=\"1\" height=\"1\"/>".repeat(65_534)
    );
    let source = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><style>#target {{ background:red }}</style>{descendants}</svg>"
    );
    let svg = SvgDocument::parse(source.as_bytes()).expect("supported SVG");

    let error = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                host_controls_root_background: true,
                ..SvgRootStyle::default()
            },
            Some(4),
        )
        .expect_err("root-style descendant rewrites must share the selector budget");

    assert!(
        error
            .to_string()
            .contains("selector freezing resource limit")
    );
}
