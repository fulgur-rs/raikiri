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
