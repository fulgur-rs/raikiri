use super::like_browsers;
use crate::{SvgDocument, SvgRootStyle, SvgViewport};
use std::borrow::Cow;

/// The attribute that turns a filter off, which a shape without a `style`
/// attribute is given.
const NO_FILTER: &str = " style=\"filter:none!important\"";

/// An SVG with a flood filter and `body`.
fn flooded(body: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\">\
         <defs><filter id=\"f\" filterUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"100\" height=\"100\">\
         <feFlood flood-color=\"red\"/></filter></defs>{body}</svg>"
    )
}

/// What `body` becomes when it is the content of [`flooded`].
fn rewritten(body: &str) -> String {
    like_browsers(&flooded(body)).into_owned()
}

/// `element` with [`NO_FILTER`] after its name, which a space ends.
fn unfiltered(element: &str) -> String {
    let at = element.find(' ').unwrap();
    format!("{}{NO_FILTER}{}", &element[..at], &element[at..])
}

#[test]
fn a_document_without_a_filter_is_not_read() {
    for source in [
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>",
        // A filter function draws nothing from an empty shape.
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect filter=\"blur(2px)\"/></svg>",
    ] {
        assert!(
            matches!(like_browsers(source), Cow::Borrowed(_)),
            "{source}"
        );
    }
    // Nor is one that is not XML: the parser reports that.
    assert!(matches!(
        like_browsers("<svg><filter/><rect"),
        Cow::Borrowed(_)
    ));
    // A filter element with a prefix is read.
    let prefixed = "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:s=\"http://www.w3.org/2000/svg\">\
                    <s:filter id=\"f\"/><rect filter=\"url(#f)\"/></svg>";
    assert!(matches!(like_browsers(prefixed), Cow::Owned(_)));
}

#[test]
fn shapes_with_nothing_to_render_lose_their_filter() {
    for shape in [
        "<rect width=\"0\" height=\"10\"",
        "<rect width=\"10\"",
        "<rect height=\"10\" width=\"abc\"",
        "<circle r=\"0\"",
        "<circle",
        "<ellipse rx=\"0\" ry=\"20\"",
        "<ellipse rx=\"10\" ry=\"-1\"",
        "<ellipse",
        "<path d=\"\"",
        "<path",
        "<path d=\"M\"",
        "<polygon points=\"\"",
        "<polyline",
        "<image href=\"a.png\" width=\"10\" height=\"0\"",
        "<image href=\"a.png\" width=\"-5\"",
        "<image width=\"10\" height=\"10\"",
        "<image href=\" \" width=\"10\" height=\"10\"",
        "<image",
    ] {
        let element = format!("{shape} filter=\"url(#f)\"/>");
        assert_eq!(
            rewritten(&element),
            flooded(&unfiltered(&element)),
            "{shape}"
        );
    }
}

#[test]
fn shapes_with_something_to_render_keep_their_filter() {
    for shape in [
        "<rect width=\"10\" height=\"10\"",
        "<rect width=\"50%\" height=\"1e1\"",
        "<circle r=\"5\"",
        "<ellipse rx=\"10\"",
        "<ellipse ry=\"10\" rx=\"auto\"",
        "<path d=\"M 0 0 L 10 10\"",
        "<polygon points=\"1,2 3,4\"",
        "<polyline points=\"1,2\"",
        "<path d=\"M 10 10\"",
        "<image href=\"a.png\"",
        "<image href=\"a.png\" width=\"10\" height=\"10\"",
        "<image xlink:href=\"a.png\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
        "<line",
        "<g",
        "<use",
    ] {
        let body = format!("{shape} filter=\"url(#f)\"/>");
        assert_eq!(rewritten(&body), flooded(&body), "{shape}");
    }
}

#[test]
fn a_filter_from_a_style_declaration_or_a_stylesheet_counts() {
    assert_eq!(
        rewritten("<rect style=\"filter:url(#f)\"/>"),
        flooded("<rect style=\"filter:none!important;filter:url(#f)\"/>")
    );
    assert_eq!(
        rewritten("<style>rect{filter:url(#f)}</style><rect/>"),
        flooded(&format!(
            "<style>rect{{filter:url(#f)}}</style><rect{NO_FILTER}/>"
        ))
    );
    // The names of CSS properties do not depend on the case.
    assert_eq!(
        rewritten("<rect style=\"FILTER:url(#f)\"/>"),
        flooded("<rect style=\"filter:none!important;FILTER:url(#f)\"/>")
    );
    // A shape that no filter applies to is left to `usvg`.
    for body in [
        "<rect/>",
        "<rect filter=\"none\"/>",
        "<rect style=\"fill:red\"/>",
        "<rect style=\"backdrop-filter:blur(1px)\"/>",
        "<style>rect{backdrop-filter:blur(1px)}</style><rect/>",
    ] {
        assert_eq!(rewritten(body), flooded(body), "{body}");
    }
}

#[test]
fn only_a_stylesheet_of_css_is_read() {
    let rule = "rect{filter:url(#f)}";
    for kind in ["", " type=\"\"", " type=\"text/css\""] {
        let body = format!("<style{kind}>{rule}</style><rect/>");
        assert_eq!(
            rewritten(&body),
            flooded(&format!("<style{kind}>{rule}</style><rect{NO_FILTER}/>")),
            "{kind}"
        );
    }
    let body = format!("<style type=\"text/xml\">{rule}</style><rect/>");
    assert_eq!(rewritten(&body), flooded(&body));
    // Text after a comment belongs to the stylesheet as well.
    let body = format!("<style><!-- c -->{rule}</style><rect/>");
    assert_eq!(
        rewritten(&body),
        flooded(&format!(
            "<style><!-- c -->{rule}</style><rect{NO_FILTER}/>"
        ))
    );
}

#[test]
fn a_dimension_that_a_style_declaration_may_set_counts_as_rendered() {
    for body in [
        "<rect style=\"width:10px;height:10px\" filter=\"url(#f)\"/>",
        "<rect width=\"0\" style=\"width : 10px\" height=\"10\" filter=\"url(#f)\"/>",
        "<circle style=\"r:5px\" filter=\"url(#f)\"/>",
        "<ellipse style=\"rx:5px;ry:5px\" filter=\"url(#f)\"/>",
        "<path style=\"d:path('M0 0L1 1')\" filter=\"url(#f)\"/>",
        "<image href=\"a.png\" style=\"height:5px\" height=\"0\" filter=\"url(#f)\"/>",
        "<style>rect{width:10px;height:10px}</style><rect filter=\"url(#f)\"/>",
        "<style>.big{width:10px;height:10px}</style><rect class=\"big\" filter=\"url(#f)\"/>",
        "<style>svg rect{width:10px;height:10px}</style><rect filter=\"url(#f)\"/>",
        "<rect width=\"0\" height=\"0\" style=\"WIDTH:10px;Height:10px\" filter=\"url(#f)\"/>",
    ] {
        assert_eq!(rewritten(body), flooded(body), "{body}");
    }
    // A rule that does not select the shape says nothing about it.
    let sheet = "<style>.big{width:10px;height:10px}svg{width:100%}</style>";
    assert_eq!(
        rewritten(&format!(
            "{sheet}<rect width=\"0\" height=\"1\" filter=\"url(#f)\"/>"
        )),
        flooded(&format!(
            "{sheet}<rect{NO_FILTER} width=\"0\" height=\"1\" filter=\"url(#f)\"/>"
        ))
    );
    // Another property whose name ends in the same letters says nothing.
    assert_eq!(
        rewritten("<rect style=\"min-width:1px\" width=\"0\" height=\"1\" filter=\"url(#f)\"/>"),
        flooded(
            "<rect style=\"filter:none!important;min-width:1px\" width=\"0\" height=\"1\" filter=\"url(#f)\"/>"
        )
    );
}

#[test]
fn shapes_inside_one_another_are_each_read() {
    let inner = "<rect filter=\"url(#f)\"/>";
    assert_eq!(
        rewritten(&format!(
            "<rect filter=\"url(#f)\">{inner}</rect><circle r=\"0\" filter=\"url(#f)\"/>"
        )),
        flooded(&format!(
            "<rect{NO_FILTER} filter=\"url(#f)\">{}</rect><circle{NO_FILTER} r=\"0\" filter=\"url(#f)\"/>",
            unfiltered(inner)
        ))
    );
}

#[test]
fn the_element_of_a_shape_is_left_as_it_is_but_for_its_style() {
    for (body, expected) in [
        (
            "<rect id=\"r\" systemLanguage=\"xx\" width=\"0\" filter=\"url(#f)\"/>",
            "<rect style=\"filter:none!important\" id=\"r\" systemLanguage=\"xx\" width=\"0\" filter=\"url(#f)\"/>",
        ),
        (
            "<rect\n width=\"0\" filter=\"url(#f)\"/>",
            "<rect style=\"filter:none!important\"\n width=\"0\" filter=\"url(#f)\"/>",
        ),
        (
            "<style>rect{filter:url(#f)}</style><rect></rect>",
            "<style>rect{filter:url(#f)}</style><rect style=\"filter:none!important\"></rect>",
        ),
        (
            "<rect style='fill:red' width=\"0\" filter=\"url(#f)\"/>",
            "<rect style='filter:none!important;fill:red' width=\"0\" filter=\"url(#f)\"/>",
        ),
        (
            "<rect style=\"\" width=\"0\" filter=\"url(#f)\"/>",
            "<rect style=\"filter:none!important;\" width=\"0\" filter=\"url(#f)\"/>",
        ),
    ] {
        assert_eq!(rewritten(body), flooded(expected), "{body}");
    }
    let prefixed = |element: &str| {
        format!("<svg xmlns:s=\"http://www.w3.org/2000/svg\"><s:filter id=\"f\"/>{element}</svg>")
    };
    assert_eq!(
        like_browsers(&prefixed("<s:rect width=\"0\" filter=\"url(#f)\"/>")),
        prefixed("<s:rect style=\"filter:none!important\" width=\"0\" filter=\"url(#f)\"/>")
    );
}

#[test]
fn a_stylesheet_that_takes_too_much_to_match_leaves_the_shape_to_usvg() {
    // Matching the attribute costs as much as its value is long.
    let long = "a".repeat(1_100_000);
    let source = flooded(&format!(
        "<style>rect[class=\"b\"]{{fill:red}}</style><rect class=\"{long}\" filter=\"url(#f)\"/>"
    ));
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
}

#[test]
fn only_svg_elements_are_rewritten() {
    let source = "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:x=\"urn:x\" filter=\"f\">\
                  <filter/><x:rect filter=\"url(#f)\"/><x:feConvolveMatrix divisor=\"0\"/></svg>";
    assert!(matches!(like_browsers(source), Cow::Borrowed(_)));
}

#[test]
fn a_zero_divisor_of_a_convolution_is_left_out() {
    let convolve = |divisor: &str| {
        flooded(&format!(
            "<filter id=\"g\"><feConvolveMatrix kernelMatrix=\"1 1 1 1 1 1 1 1 1\"{divisor}/></filter>"
        ))
    };
    for divisor in [
        " divisor=\"0\"",
        " divisor=\"0.0\"",
        " divisor=\"-0e1\"",
        // Smaller than a single precision float holds.
        " divisor=\"1e-50\"",
        " divisor=\"-1e-45\"",
    ] {
        assert_eq!(like_browsers(&convolve(divisor)), convolve(""), "{divisor}");
    }
    for divisor in [
        " divisor=\"2\"",
        " divisor=\"1e-30\"",
        " divisor=\"x\"",
        " divisor=\"\"",
        "",
    ] {
        assert!(
            matches!(like_browsers(&convolve(divisor)), Cow::Borrowed(_)),
            "{divisor}"
        );
    }
}

fn rasterized(source: &str) -> Vec<u8> {
    let viewport = SvgViewport {
        width: 100.0,
        height: 100.0,
    };
    SvgDocument::parse(source.as_bytes())
        .unwrap()
        .rasterize(viewport, SvgRootStyle::default(), None)
        .unwrap()
        .rgba
}

#[test]
fn a_filter_on_an_empty_shape_draws_nothing() {
    let drawn = rasterized(&flooded(
        "<rect filter=\"url(#f)\" width=\"10\" height=\"10\"/>",
    ));
    assert_eq!(&drawn[(50 * 100 + 50) * 4..][..4], &[255, 0, 0, 255]);
    for shape in [
        "<rect width=\"0\" height=\"0\"",
        "<circle cx=\"40\" cy=\"40\" r=\"0\"",
        "<path d=\"\"",
        "<polygon points=\"\"",
        "<image width=\"10\" height=\"10\"",
        // The declaration goes first, so it holds against an important one
        // of the shape and against the rest of the attribute.
        "<rect width=\"0\" style=\"filter:url(#f)!important\"",
        "<rect width=\"0\" style=\"fill:red;/* a comment that goes on\"",
    ] {
        let drawn = rasterized(&flooded(&format!("{shape} filter=\"url(#f)\"/>")));
        assert!(drawn.iter().all(|&byte| byte == 0), "{shape}");
    }
    // The shape stays where it is, so a `switch` still passes it over.
    let drawn = rasterized(&flooded(
        "<switch><rect systemLanguage=\"xx\" filter=\"url(#f)\"/>\
         <circle cx=\"50\" cy=\"50\" r=\"20\" fill=\"blue\"/></switch>",
    ));
    assert_eq!(&drawn[(50 * 100 + 50) * 4..][..4], &[0, 0, 255, 255]);
    assert!(drawn[..4].iter().all(|&byte| byte == 0));
    // The filter of an empty group does draw, and so does that of a `use` of a
    // shape that is not rendered.
    for body in [
        "<g filter=\"url(#f)\"/>",
        "<style>use{filter:url(#f)}</style><defs><rect id=\"r\" height=\"10\"/></defs><use href=\"#r\"/>",
    ] {
        let drawn = rasterized(&flooded(body));
        assert_eq!(
            &drawn[(50 * 100 + 50) * 4..][..4],
            &[255, 0, 0, 255],
            "{body}"
        );
    }
}

#[test]
fn a_zero_divisor_draws_like_the_default_one() {
    let convolved = |kernel: &str, divisor: &str| {
        rasterized(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\">\
             <filter id=\"f\" filterUnits=\"userSpaceOnUse\" x=\"10\" y=\"10\" width=\"50\" height=\"50\">\
             <feConvolveMatrix kernelMatrix=\"{kernel}\" bias=\".5\" preserveAlpha=\"false\"{divisor}/></filter>\
             <rect x=\"10\" y=\"10\" width=\"50\" height=\"50\" fill=\"lime\" filter=\"url(#f)\"/></svg>"
        ))
    };
    let sum_of_nine = "1 1 1 1 1 1 1 1 1";
    let sum_of_zero = "1 0 0 0 -1 0 0 0 0";
    for kernel in [sum_of_nine, sum_of_zero] {
        let default = convolved(kernel, "");
        assert_eq!(convolved(kernel, " divisor=\"0\""), default, "{kernel}");
        // The primitive is not left out.
        assert!(default.iter().any(|&byte| byte != 0), "{kernel}");
    }
    // A divisor that is not zero is still used.
    assert_ne!(
        convolved(sum_of_nine, " divisor=\"2\""),
        convolved(sum_of_nine, "")
    );
}
