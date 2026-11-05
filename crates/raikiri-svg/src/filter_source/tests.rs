use super::{Edit, attribute_bytes, escaped_len, like_browsers, set_attribute};
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

/// Whether the filter of `body`, which is the content of [`flooded`], stays.
fn keeps_filter(body: &str) -> bool {
    !rewritten(body).contains("filter:none")
}

#[test]
fn a_dimension_that_a_style_declaration_sets_counts_as_rendered() {
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
        // Whatever a value that is not computed here comes to.
        "<rect style=\"width:calc(5px + 5px);height:var(--h)\" filter=\"url(#f)\"/>",
        "<circle style=\"r:2rem\" filter=\"url(#f)\"/>",
        "<circle r=\"0\" style=\"r:inherit\" filter=\"url(#f)\"/>",
        "<path style=\"d:var(--p)\" filter=\"url(#f)\"/>",
    ] {
        assert!(keeps_filter(body), "{body}");
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
fn a_dimension_that_a_style_declaration_takes_away_counts_as_empty() {
    for body in [
        "<rect style=\"width:0\" width=\"10\" height=\"10\" filter=\"url(#f)\"/>",
        "<rect style=\"height:auto\" width=\"10\" height=\"10\" filter=\"url(#f)\"/>",
        "<rect style=\"height:initial\" width=\"10\" height=\"10\" filter=\"url(#f)\"/>",
        "<circle style=\"r:0px\" r=\"5\" filter=\"url(#f)\"/>",
        "<ellipse style=\"rx:auto;ry:auto\" rx=\"5\" ry=\"5\" filter=\"url(#f)\"/>",
        "<path style=\"d:none\" d=\"M0 0L1 1\" filter=\"url(#f)\"/>",
        "<path style=\"d:path('')\" d=\"M0 0L1 1\" filter=\"url(#f)\"/>",
        "<image href=\"a.png\" style=\"width:0\" width=\"10\" filter=\"url(#f)\"/>",
        "<style>rect{width:0}</style><rect width=\"10\" height=\"10\" filter=\"url(#f)\"/>",
        // The one with the higher priority is the one that counts.
        "<style>rect{width:0px!important}</style><rect style=\"width:10px\" height=\"10\" filter=\"url(#f)\"/>",
    ] {
        assert!(!keeps_filter(body), "{body}");
    }
    // A declaration that is not valid, or one that a stronger declaration
    // beats, takes nothing away.
    for body in [
        "<rect style=\"width:abc\" width=\"10\" height=\"10\" filter=\"url(#f)\"/>",
        "<rect style=\"width:-5px\" width=\"10\" height=\"10\" filter=\"url(#f)\"/>",
        "<circle style=\"r:-1px\" r=\"5\" filter=\"url(#f)\"/>",
        "<style>rect{width:10px!important}</style><rect style=\"width:0\" height=\"10\" filter=\"url(#f)\"/>",
    ] {
        assert!(keeps_filter(body), "{body}");
    }
}

#[test]
fn a_declaration_that_is_not_written_only_says_the_geometry_is_there() {
    // A selector tests an attribute of the geometry, so the attributes stay as
    // they are, and a declaration is not known to take anything away.
    let sheet = "<style>rect[width]{fill:blue}</style>";
    for body in [
        "<rect width=\"10\" height=\"10\" style=\"width:0\" filter=\"url(#f)\"/>",
        "<rect style=\"width:10px;height:10px\" filter=\"url(#f)\"/>",
    ] {
        let body = format!("{sheet}{body}");
        assert_eq!(rewritten(&body), flooded(&body), "{body}");
    }
    // The shape that the attributes leave empty loses its filter as before.
    let body = format!("{sheet}<rect width=\"0\" height=\"10\" filter=\"url(#f)\"/>");
    assert!(!keeps_filter(&body));
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
fn an_attribute_that_a_selector_tests_is_left_alone() {
    // A new `style` attribute would make `circle[style] + rect` match.
    for test in ["[style]", "[ STYLE ^= \"x\"]", "[style|=\"x\"]"] {
        let source = flooded(&format!(
            "<style>circle{test} + rect{{fill:blue}}</style>\
             <circle r=\"0\" filter=\"url(#f)\"/><rect width=\"1\" height=\"1\"/>"
        ));
        assert!(matches!(like_browsers(&source), Cow::Borrowed(_)), "{test}");
    }
    // Nor does a zero divisor go where a selector tests it.
    let source = flooded(
        "<style>feConvolveMatrix[divisor]{x:y}</style>\
         <filter id=\"g\"><feConvolveMatrix divisor=\"0\"/></filter>",
    );
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
    // Another attribute, or one that only starts with the name, says nothing.
    for test in ["[class]", "[styled]", "[style-x=\"1\"]", "[x=\"style\"]"] {
        let source = flooded(&format!(
            "<style>circle{test}{{fill:blue}}</style><circle r=\"0\" filter=\"url(#f)\"/>"
        ));
        assert!(matches!(like_browsers(&source), Cow::Owned(_)), "{test}");
    }
}

#[test]
fn a_stylesheet_that_takes_too_much_to_match_leaves_the_shape_to_usvg() {
    // Matching the attribute costs as much as its value is long.
    let long = "a".repeat(1_100_000);
    let source = flooded(&format!(
        "<style>rect[class=\"b\"]{{filter:none}}</style><rect class=\"{long}\" filter=\"url(#f)\"/>"
    ));
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
    // A rule that says nothing about a filter or a geometry property is not
    // matched, so it takes nothing.
    let source = flooded(&format!(
        "<style>rect[class=\"b\"]{{fill:red}}</style><rect class=\"{long}\" filter=\"url(#f)\"/>"
    ));
    assert_eq!(
        like_browsers(&source),
        flooded(&format!(
            "<style>rect[class=\"b\"]{{fill:red}}</style><rect{NO_FILTER} class=\"{long}\" filter=\"url(#f)\"/>"
        ))
    );
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
fn a_filter_follows_the_geometry_that_a_declaration_gives() {
    // A declaration that takes the shape away takes its filter with it, and
    // the attribute and the style are both edited.
    for shape in [
        "<rect width=\"10\" height=\"10\" style=\"width:0\"",
        "<circle cx=\"50\" cy=\"50\" r=\"10\" style=\"r:0\"",
        "<path d=\"M10 10H50V50Z\" style=\"d:none\"",
        "<style>rect{width:0}</style><rect width=\"10\" height=\"10\"",
        "<style>rect{width:0}</style><rect height=\"10\"",
        "<style>rect{width:0}</style><rect",
    ] {
        let drawn = rasterized(&flooded(&format!("{shape} filter=\"url(#f)\"/>")));
        assert!(drawn.iter().all(|&byte| byte == 0), "{shape}");
    }
    // And one that gives it back brings its filter back.
    for shape in [
        "<rect style=\"width:10px;height:10px\"",
        "<style>rect{width:10px;height:10px}</style><rect",
        "<circle style=\"r:5px\"",
        "<path style=\"d:path('M0 0L10 10')\"",
    ] {
        let drawn = rasterized(&flooded(&format!("{shape} filter=\"url(#f)\"/>")));
        assert_eq!(
            &drawn[(50 * 100 + 50) * 4..][..4],
            &[255, 0, 0, 255],
            "{shape}"
        );
    }
    let both = rewritten("<style>rect{width:0}</style><rect height=\"10\" filter=\"url(#f)\"/>");
    assert_eq!(
        both,
        flooded(&format!(
            "<style>rect{{width:0}}</style><rect width=\"0\"{NO_FILTER} height=\"10\" filter=\"url(#f)\"/>"
        ))
    );
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

/// An SVG of `body`.
fn plain(body: &str) -> String {
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\">{body}</svg>")
}

/// What `body` becomes when it is the content of [`plain`].
fn geometry_of(body: &str) -> String {
    like_browsers(&plain(body)).into_owned()
}

#[test]
fn a_document_that_declares_no_geometry_is_not_read() {
    for body in [
        "<rect x=\"1\" y=\"2\" width=\"3\" height=\"4\"/>",
        "<rect style=\"fill:red\"/>",
        "<style>rect{fill:red;stroke-width:3}</style><rect/>",
        // A name that only ends the same, or a colon that is not a declaration.
        "<rect style=\"min-width:3px;font-size:1px\" id=\"x:y\"/>",
        "<use xlink:href=\"#a\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"/>",
    ] {
        assert!(
            matches!(like_browsers(&plain(body)), Cow::Borrowed(_)),
            "{body}"
        );
    }
    // Nor one that is not XML.
    assert!(matches!(
        like_browsers("<svg><rect style=\"width:1px\""),
        Cow::Borrowed(_)
    ));
    // Nor one that declares an entity, which can spell a name.
    assert!(matches!(
        like_browsers(
            "<!DOCTYPE svg [<!ENTITY w \"width\">]>\
             <svg xmlns=\"http://www.w3.org/2000/svg\"><rect style=\"&w;:1px\"/></svg>"
        ),
        Cow::Borrowed(_)
    ));
}

#[test]
fn the_geometry_of_a_declaration_goes_into_the_attribute() {
    for (body, expected) in [
        // Added after the name, in the order of the properties.
        (
            "<rect style=\"height:30px;width:20px\"/>",
            "<rect width=\"20px\" height=\"30px\" style=\"height:30px;width:20px\"/>",
        ),
        // A comment between a name and its colon is read past.
        (
            "<rect style=\"width/**/:20px;height /* c */ : 10px\"/>",
            "<rect width=\"20px\" height=\"10px\" style=\"width/**/:20px;height /* c */ : 10px\"/>",
        ),
        // A name that CDATA sections cut up is one text for the XML parser.
        (
            "<style><![CDATA[rect{wid]]><![CDATA[th:10px;height]]><![CDATA[:5px}]]></style><rect/>",
            "<style><![CDATA[rect{wid]]><![CDATA[th:10px;height]]><![CDATA[:5px}]]></style><rect width=\"10px\" height=\"5px\"/>",
        ),
        // And so is a character reference that spells a name or the colon.
        (
            "<rect style=\"width&#58;20px;&#104;eight&#x3a;10px\"/>",
            "<rect width=\"20px\" height=\"10px\" style=\"width&#58;20px;&#104;eight&#x3a;10px\"/>",
        ),
        (
            "<rect style=\"ry:4px;rx:3px;y:2px;x:1px\"/>",
            "<rect x=\"1px\" y=\"2px\" rx=\"3px\" ry=\"4px\" style=\"ry:4px;rx:3px;y:2px;x:1px\"/>",
        ),
        (
            "<circle style=\"r:5px;cx:1px;cy:2px\"/>",
            "<circle cx=\"1px\" cy=\"2px\" r=\"5px\" style=\"r:5px;cx:1px;cy:2px\"/>",
        ),
        (
            "<ellipse style=\"rx:5;ry:6\"></ellipse>",
            "<ellipse rx=\"5\" ry=\"6\" style=\"rx:5;ry:6\"></ellipse>",
        ),
        (
            "<path style=\"d:path('M0 0L1 1')\"/>",
            "<path d=\"M0 0L1 1\" style=\"d:path('M0 0L1 1')\"/>",
        ),
        (
            "<image style=\"width:3px;height:4px\"/>",
            "<image width=\"3px\" height=\"4px\" style=\"width:3px;height:4px\"/>",
        ),
        (
            "<use href=\"#a\" style=\"x:3px;y:4px\"/>",
            "<use x=\"3px\" y=\"4px\" href=\"#a\" style=\"x:3px;y:4px\"/>",
        ),
        // An attribute that is there gets the value in its place, in the
        // quotes it has.
        (
            "<rect width=\"50\" height='60' style=\"width:20px;height:30px\"/>",
            "<rect width=\"20px\" height='30px' style=\"width:20px;height:30px\"/>",
        ),
        (
            "<rect height=\"\" style=\"height:30px\"/>",
            "<rect height=\"30px\" style=\"height:30px\"/>",
        ),
        // The units are written in lowercase, which is how `usvg` reads them.
        (
            "<rect style=\"width:2EM;height:50%\"/>",
            "<rect width=\"2em\" height=\"50%\" style=\"width:2EM;height:50%\"/>",
        ),
        // A keyword comes out as the value it stands for.
        (
            "<rect width=\"5\" rx=\"5\" style=\"width:auto;rx:initial\"/>",
            "<rect width=\"auto\" rx=\"auto\" style=\"width:auto;rx:initial\"/>",
        ),
        (
            "<circle cx=\"5\" r=\"5\" style=\"cx:unset;r:initial\"/>",
            "<circle cx=\"0\" r=\"0\" style=\"cx:unset;r:initial\"/>",
        ),
        (
            "<path d=\"M0 0L1 1\" style=\"d:none\"/>",
            "<path d=\"\" style=\"d:none\"/>",
        ),
        // A name with a prefix, and a name in capitals.
        (
            "<s:rect xmlns:s=\"http://www.w3.org/2000/svg\" STYLE=\"width:1px\" style=\"WIDTH:2px\"/>",
            "<s:rect width=\"2px\" xmlns:s=\"http://www.w3.org/2000/svg\" STYLE=\"width:1px\" style=\"WIDTH:2px\"/>",
        ),
    ] {
        assert_eq!(geometry_of(body), plain(expected), "{body}");
    }
}

#[test]
fn the_declaration_that_the_cascade_picks_is_the_one_written() {
    let width_of = |body: &str| {
        let rewritten = geometry_of(body);
        // The root has a width of its own.
        let body = &rewritten[rewritten.find('>').unwrap()..];
        let at = body.find(" width=\"").unwrap() + " width=\"".len();
        body[at..at + body[at..].find('"').unwrap()].to_owned()
    };
    for (body, width) in [
        // A declaration beats the attribute, in a stylesheet and in `style`.
        (
            "<style>rect{width:20px}</style><rect width=\"50\"/>",
            "20px",
        ),
        ("<rect width=\"50\" style=\"width:20px\"/>", "20px"),
        // The `style` attribute beats a rule, and an important rule beats it.
        (
            "<style>rect{width:40px}</style><rect style=\"width:20px\"/>",
            "20px",
        ),
        (
            "<style>rect{width:40px!important}</style><rect style=\"width:20px\"/>",
            "40px",
        ),
        (
            "<style>rect{width:40px!important}</style><rect style=\"width:20px!important\"/>",
            "20px",
        ),
        // A rule that is more specific beats the other, and among the ones
        // that are as specific the later one wins, in the stylesheets too.
        (
            "<style>#a{width:30px}rect{width:40px}</style><rect id=\"a\"/>",
            "30px",
        ),
        (
            "<style>.b{width:30px}rect{width:40px}</style><rect class=\"b\"/>",
            "30px",
        ),
        (
            "<style>rect{width:30px}rect{width:40px}</style><rect/>",
            "40px",
        ),
        (
            "<style>rect{width:30px}</style><style>rect{width:40px}</style><rect/>",
            "40px",
        ),
        (
            "<style>rect{width:30px}svg rect{width:40px}rect{width:50px}</style><rect/>",
            "40px",
        ),
        // A declaration that is not valid is dropped.
        (
            "<style>rect{width:30px}</style><rect style=\"width:-5px\"/>",
            "30px",
        ),
        ("<rect style=\"width:20px;width:bogus(1)\"/>", "20px"),
        (
            "<style>rect{width:30px}</style><rect style=\"width:abc!important\"/>",
            "30px",
        ),
    ] {
        assert_eq!(width_of(body), width, "{body}");
    }
    // Nothing is written for a declaration that cannot be told, nor for one
    // that is not valid and has nothing below it.
    for body in [
        "<rect width=\"5\" style=\"width:calc(1px + 2px)\"/>",
        "<rect width=\"5\" style=\"width:var(--w)\"/>",
        "<rect width=\"5\" style=\"width:inherit\"/>",
        "<rect width=\"5\" style=\"width:2rem\"/>",
        "<rect width=\"5\" style=\"width:abc\"/>",
        "<rect width=\"5\" style=\"width:-5px\"/>",
        "<style>rect{width:calc(1px + 2px)}</style><rect width=\"5\"/>",
        // The one that is told is under one that cannot be.
        "<style>rect{width:30px}</style><rect width=\"5\" style=\"width:calc(1px + 2px)\"/>",
    ] {
        assert!(
            matches!(like_browsers(&plain(body)), Cow::Borrowed(_)),
            "{body}"
        );
    }
}

#[test]
fn only_the_properties_of_an_element_are_written() {
    for body in [
        "<circle r=\"5\" style=\"width:10px;height:10px;rx:3px;x:1px;y:1px;d:none\"/>",
        "<rect width=\"5\" height=\"5\" style=\"r:10px;cx:1px;d:none\"/>",
        "<ellipse rx=\"5\" ry=\"5\" style=\"r:10px;width:1px;x:3px\"/>",
        "<path d=\"M0 0\" style=\"x:1px;width:2px;r:3px;rx:4px\"/>",
        // The geometry of these is attributes only.
        "<line x1=\"0\" style=\"x1:10px;x:1px;width:5px\"/>",
        "<polygon points=\"0,0\" style=\"x:1px;d:path('M0 0')\"/>",
        "<g style=\"x:1px;width:5px\"/>",
        "<text style=\"x:1px;y:1px\">a</text>",
        "<svg x=\"1\" style=\"x:5px;y:5px;width:50px;height:50px\"/>",
        "<symbol style=\"width:50px\"/>",
        "<use href=\"#a\" style=\"width:50px;height:50px\"/>",
        "<foreignObject style=\"width:5px;height:5px\"/>",
        "<pattern style=\"x:5px;width:5px\"/>",
        "<feFlood style=\"x:5px;width:5px\"/>",
    ] {
        assert!(
            matches!(like_browsers(&plain(body)), Cow::Borrowed(_)),
            "{body}"
        );
    }
    // Only the elements of SVG, as everywhere here.
    let source = "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:x=\"urn:x\">\
                  <x:rect style=\"width:1px\"/><rect xmlns=\"urn:x\" style=\"width:1px\"/></svg>";
    assert!(matches!(like_browsers(source), Cow::Borrowed(_)));
}

#[test]
fn an_attribute_that_a_selector_tests_keeps_the_geometry_as_it_is() {
    for test in [
        "[width]",
        "[ WIDTH = \"10px\"]",
        "[r|=\"1\"]",
        "[d^=\"M\"]",
        "[x]",
        "[cx]",
    ] {
        let body = format!(
            "<style>rect{test}{{fill:blue}}</style>\
             <rect style=\"width:10px;height:10px;r:1px;d:none;x:1px;cx:1px\"/>"
        );
        assert!(
            matches!(like_browsers(&plain(&body)), Cow::Borrowed(_)),
            "{test}"
        );
    }
    // Another attribute, or one that only starts with the name, says nothing.
    for test in [
        "[class]",
        "[widths]",
        "[width-x=\"1\"]",
        "[x-y]",
        "[title=\"width\"]",
    ] {
        let body = format!(
            "<style>rect{test}{{fill:blue}}</style><rect style=\"width:10px;height:10px\"/>"
        );
        assert!(
            matches!(like_browsers(&plain(&body)), Cow::Owned(_)),
            "{test}"
        );
    }
}

#[test]
fn a_stylesheet_that_takes_too_much_to_match_leaves_the_geometry_to_usvg() {
    let long = "a".repeat(1_100_000);
    let body = format!(
        "<style>rect[class=\"b\"]{{width:10px}}</style><rect class=\"{long}\" style=\"height:5px\"/>"
    );
    assert!(matches!(like_browsers(&plain(&body)), Cow::Borrowed(_)));
}

#[test]
fn a_length_that_css_does_not_read_leaves_the_attribute_alone() {
    // `1.px` is a length for `svgtypes` and not for a browser, which drops the
    // declaration and keeps the width of the attribute.
    for style in ["width:1.px", "width:20.", "width:1.5.px", "width:20 px"] {
        let body = format!("<rect width=\"30\" height=\"10\" style=\"{style}\"/>");
        assert_eq!(geometry_of(&body), plain(&body), "{style}");
    }
    let body = "<rect width=\"30\" height=\"10\" style=\"width:1.px;width:20.0px\"/>";
    assert_eq!(
        geometry_of(body),
        plain("<rect width=\"20.0px\" height=\"10\" style=\"width:1.px;width:20.0px\"/>")
    );
}

#[test]
fn path_data_with_an_error_leaves_the_attribute_alone() {
    // A browser drops `d:path()` of data with an error anywhere in it, where
    // the attribute is drawn up to the error. The data is never a string of
    // characters that an attribute has to escape.
    for data in [
        "garbage",
        "M0 0H20V20H0Z garbage",
        "M0 0 L",
        "L10 10 L20 20",
        "M0 0 A5 5 0 2 0 10 10",
        "M0 0H20V20H0Z,",
        "M &amp; &lt; ' &gt; Z",
    ] {
        let body = format!("<path d=\"M10 10H50V50H10Z\" style=\"d:path(&quot;{data}&quot;)\"/>");
        assert_eq!(geometry_of(&body), plain(&body), "{data}");
    }
    let body = "<path d=\"M10 10H50V50H10Z\" style=\"d:path('M0 0H20V20H0Z garbage');d:path('M0,0 20,0 20,20z')\"/>";
    assert_eq!(
        geometry_of(body),
        plain(
            "<path d=\"M0,0 20,0 20,20z\" style=\"d:path('M0 0H20V20H0Z garbage');d:path('M0,0 20,0 20,20z')\"/>"
        )
    );
}

#[test]
fn a_suffix_that_is_not_a_unit_does_not_hide_the_declaration_below_it() {
    // A browser drops `1bogus` as it does `1.px`, so the declaration before it
    // stands. A real unit that is not computed here is another matter: it wins,
    // and the attribute is left as it is.
    let body = "<rect width=\"50\" height=\"10\" style=\"width:20px;width:1bogus\"/>";
    assert_eq!(
        geometry_of(body),
        plain("<rect width=\"20px\" height=\"10\" style=\"width:20px;width:1bogus\"/>")
    );
    let body = "<rect width=\"50\" height=\"10\" style=\"width:20px;width:1vw\"/>";
    assert_eq!(geometry_of(body), plain(body));
    // A negative size is invalid whatever its unit, so it hides nothing either.
    let body = "<rect width=\"50\" height=\"10\" style=\"width:20px;width:-1rem\"/>";
    assert_eq!(
        geometry_of(body),
        plain("<rect width=\"20px\" height=\"10\" style=\"width:20px;width:-1rem\"/>")
    );
    // An offset may be negative, so it wins and the attribute is left alone.
    let body = "<rect x=\"5\" width=\"50\" height=\"10\" style=\"x:20px;x:-1rem\"/>";
    assert_eq!(geometry_of(body), plain(body));
}

#[test]
fn a_value_that_many_shapes_share_is_written_to_a_limited_number_of_them() {
    // A rule gives its value to every shape it selects, and each one holds a
    // copy of it in the source.
    let data = format!("M0 0{}", " L1 1".repeat(200_000));
    let source = plain(&format!(
        "<style>path{{d:path('{data}')}}</style>{}",
        "<path/>".repeat(40)
    ));
    let out = like_browsers(&source);
    let written = out.matches(" d=\"").count();
    assert!(0 < written && written < 40, "{written}");
    // The shapes are written in turn, and the others are left as they are.
    assert!(out.ends_with(&format!("{}</svg>", "<path/>".repeat(40 - written))));
    assert!(out.len() < 20 * 1024 * 1024, "{}", out.len());
}

#[test]
fn reading_a_long_value_is_charged_to_each_shape_that_it_applies_to() {
    // A value that is not valid writes nothing, but is read for every shape.
    let long = "a".repeat(1_000_000);
    let source = plain(&format!(
        "<style>rect{{width:10px;height:foo({long})}}</style>{}",
        "<rect/>".repeat(60)
    ));
    let out = like_browsers(&source);
    let written = out.matches(" width=\"10px\"").count();
    assert!(0 < written && written < 60, "{written}");
    assert!(out.ends_with(&format!("{}</svg>", "<rect/>".repeat(60 - written))));
}

#[test]
fn the_bytes_of_an_attribute_are_counted_as_they_are_written() {
    let value = "M0 0 & <é\"'>";
    let source = "<rect/>";
    let xml = roxmltree::Document::parse(source).unwrap();
    let (_, text) = set_attribute(source, xml.root_element(), "d", value);
    assert_eq!(
        text, " d=\"M0 0 &amp; &lt;é&quot;&apos;>\"",
        "the value is escaped"
    );
    assert_eq!(text.len(), " d=\"\"".len() + escaped_len(value));
    assert_eq!(
        attribute_bytes("d", value),
        2 * escaped_len(value) + " d=\"\"".len() + std::mem::size_of::<Edit>()
    );
}

/// What `fragment` draws in black, as the box of the pixels that are drawn.
fn drawn_box(fragment: &str) -> Option<[usize; 4]> {
    let drawn = rasterized(&plain(fragment));
    let mut found: Option<[usize; 4]> = None;
    for y in 0..100 {
        for x in 0..100 {
            if drawn[(y * 100 + x) * 4 + 3] > 127 {
                let [x0, y0, x1, y1] = found.get_or_insert([x, y, x, y]);
                (*x0, *y0, *x1, *y1) = ((*x0).min(x), (*y0).min(y), (*x1).max(x), (*y1).max(y));
            }
        }
    }
    found
}

/// The cases that `drawn_box` checks have the boxes that Chromium draws for
/// them (measured in a 100 x 100 SVG in a page).
#[test]
fn a_shape_is_drawn_where_its_declarations_put_it() {
    for (fragment, expected) in [
        (
            "<rect style=\"width:20px;height:30px\"/>",
            Some([0, 0, 19, 29]),
        ),
        (
            "<rect style=\"x:10px;y:10px;width:20px;height:30px\"/>",
            Some([10, 10, 29, 39]),
        ),
        (
            "<style>rect{width:20px;height:30px}</style><rect x=\"5\" y=\"5\"/>",
            Some([5, 5, 24, 34]),
        ),
        (
            "<rect x=\"10\" y=\"10\" width=\"50\" height=\"50\" style=\"width:20px;height:30px\"/>",
            Some([10, 10, 29, 39]),
        ),
        (
            "<style>rect{width:20px}</style><rect x=\"10\" y=\"10\" width=\"50\" height=\"10\"/>",
            Some([10, 10, 29, 19]),
        ),
        (
            "<style>.a{width:20px}</style><rect class=\"b\" x=\"10\" y=\"10\" width=\"50\" height=\"10\"/>",
            Some([10, 10, 59, 19]),
        ),
        (
            "<rect x=\"10\" y=\"10\" style=\"width:50%;height:25%\"/>",
            Some([10, 10, 59, 34]),
        ),
        (
            "<rect x=\"10\" y=\"10\" height=\"10\" style=\"width:2em;font-size:10px\"/>",
            Some([10, 10, 29, 19]),
        ),
        (
            "<rect x=\"10\" y=\"10\" height=\"10\" style=\"width:20\"/>",
            Some([10, 10, 29, 19]),
        ),
        (
            "<rect x=\"-10\" y=\"10\" width=\"30\" height=\"10\" style=\"x:-5px\"/>",
            Some([0, 10, 24, 19]),
        ),
        // The mark of an important declaration is read in any case.
        (
            "<rect y=\"10\" width=\"50\" height=\"10\" style=\"width:30px !IMPORTANT;height:20px\"/>",
            Some([0, 10, 29, 29]),
        ),
        (
            "<rect y=\"10\" width=\"50\" height=\"10\" style=\"width:30px!important;width:20px !IMPORTANT;height:20px\"/>",
            Some([0, 10, 19, 29]),
        ),
        (
            "<rect y=\"10\" width=\"50\" height=\"10\" style=\"width:30px!IMPORTANT;width:20px;height:20px\"/>",
            Some([0, 10, 29, 29]),
        ),
        // A math function with an error is dropped.
        (
            "<rect y=\"10\" width=\"50\" height=\"10\" style=\"width:20px;width:calc(garbage)\"/>",
            Some([0, 10, 19, 19]),
        ),
        (
            "<rect y=\"10\" width=\"50\" height=\"10\" style=\"width:20px;width:calc(1px + 2)\"/>",
            Some([0, 10, 19, 19]),
        ),
        // The cascade.
        (
            "<style>rect{width:40px!important}</style><rect x=\"10\" y=\"10\" height=\"10\" style=\"width:20px\"/>",
            Some([10, 10, 49, 19]),
        ),
        (
            "<style>rect{width:40px!important}</style><rect x=\"10\" y=\"10\" height=\"10\" style=\"width:20px!important\"/>",
            Some([10, 10, 29, 19]),
        ),
        (
            "<style>#a{width:30px}rect{width:40px}</style><rect id=\"a\" x=\"10\" y=\"10\" height=\"10\"/>",
            Some([10, 10, 39, 19]),
        ),
        (
            "<style>rect{width:30px}rect{width:40px}</style><rect x=\"10\" y=\"10\" height=\"10\"/>",
            Some([10, 10, 49, 19]),
        ),
        // A declaration that is not valid does not count.
        (
            "<rect x=\"10\" y=\"10\" width=\"30\" height=\"10\" style=\"width:-5px\"/>",
            Some([10, 10, 39, 19]),
        ),
        (
            "<rect x=\"10\" y=\"10\" width=\"30\" height=\"10\" style=\"width:abc\"/>",
            Some([10, 10, 39, 19]),
        ),
        (
            "<rect x=\"10\" y=\"10\" width=\"50\" height=\"10\" style=\"width:20px 30px\"/>",
            Some([10, 10, 59, 19]),
        ),
        // A size of nothing draws nothing.
        (
            "<rect x=\"10\" y=\"10\" width=\"50\" height=\"50\" style=\"width:0\"/>",
            None,
        ),
        (
            "<rect x=\"10\" y=\"10\" width=\"50\" height=\"50\" style=\"width:auto\"/>",
            None,
        ),
        (
            "<rect x=\"10\" y=\"10\" width=\"50\" height=\"50\" style=\"width:initial\"/>",
            None,
        ),
        (
            "<rect x=\"10\" y=\"10\" width=\"50\" height=\"50\" style=\"width:unset\"/>",
            None,
        ),
        // Circles and ellipses.
        (
            "<circle cx=\"50\" cy=\"50\" style=\"r:20px\"/>",
            Some([30, 30, 69, 69]),
        ),
        (
            "<circle r=\"10\" style=\"cx:30px;cy:60px\"/>",
            Some([20, 50, 39, 69]),
        ),
        (
            "<circle cx=\"50\" cy=\"50\" r=\"40\" style=\"r:10px\"/>",
            Some([40, 40, 59, 59]),
        ),
        (
            "<circle cx=\"50\" cy=\"50\" style=\"r:10%\"/>",
            Some([40, 40, 59, 59]),
        ),
        (
            "<circle cx=\"50\" cy=\"50\" r=\"10\" style=\"r:-3px\"/>",
            Some([40, 40, 59, 59]),
        ),
        ("<circle cx=\"50\" cy=\"50\" r=\"10\" style=\"r:0\"/>", None),
        (
            "<circle cx=\"50\" cy=\"50\" r=\"10\" style=\"r:initial\"/>",
            None,
        ),
        (
            "<circle cx=\"50\" cy=\"50\" r=\"10\" style=\"cx:initial\"/>",
            Some([0, 40, 9, 59]),
        ),
        (
            "<ellipse style=\"cx:50px;cy:50px;rx:30px;ry:10px\"/>",
            Some([20, 40, 79, 59]),
        ),
        // A radius that is not given takes the other one.
        (
            "<ellipse cx=\"50\" cy=\"50\" style=\"rx:20px\"/>",
            Some([30, 30, 69, 69]),
        ),
        (
            "<ellipse cx=\"50\" cy=\"50\" style=\"rx:auto;ry:10px\"/>",
            Some([40, 40, 59, 59]),
        ),
        (
            "<ellipse cx=\"50\" cy=\"50\" rx=\"10\" ry=\"20\" style=\"rx:auto;ry:auto\"/>",
            None,
        ),
        // Paths.
        (
            "<path style=\"d:path('M10 10H50V50H10Z')\"/>",
            Some([10, 10, 49, 49]),
        ),
        (
            "<path d=\"M0 0H90V90H0Z\" style=\"d:path('M10 10H50V50H10Z')\"/>",
            Some([10, 10, 49, 49]),
        ),
        ("<path d=\"M10 10H50V50H10Z\" style=\"d:none\"/>", None),
        ("<path d=\"M10 10H50V50H10Z\" style=\"d:path('')\"/>", None),
        ("<path d=\"M10 10H50V50H10Z\" style=\"d:initial\"/>", None),
        // `path()` with a fill rule is not a value of `d`, nor is a string.
        (
            "<path d=\"M0 0H20V20H0Z\" style=\"d:path(evenodd,'M10 10H90V90H10Z')\"/>",
            Some([0, 0, 19, 19]),
        ),
        (
            "<path d=\"M10 10H50V50H10Z\" style=\"d:'M0 0H90V90H0Z'\"/>",
            Some([10, 10, 49, 49]),
        ),
        // A `use` is put where its `x` and `y` say.
        (
            "<defs><rect id=\"r\" width=\"10\" height=\"10\"/></defs><use href=\"#r\" style=\"x:30px;y:40px\"/>",
            Some([30, 40, 39, 49]),
        ),
        // These are not properties of the elements.
        (
            "<circle cx=\"50\" cy=\"50\" r=\"20\" style=\"rx:5px;width:5px;x:5px\"/>",
            Some([30, 30, 69, 69]),
        ),
        (
            "<line x1=\"10\" y1=\"10\" x2=\"50\" y2=\"10\" stroke=\"black\" stroke-width=\"4\" style=\"x1:20px\"/>",
            Some([10, 8, 49, 11]),
        ),
    ] {
        assert_eq!(drawn_box(fragment), expected, "{fragment}");
    }
}

#[test]
fn an_important_mark_in_any_case_is_read() {
    // Both declarations are read, and the mark is one.
    assert_eq!(
        geometry_of(
            "<rect width=\"50\" height=\"50\" style=\"width:10px !IMPORTANT;height:20px\"/>"
        ),
        plain("<rect width=\"10px\" height=\"20px\" style=\"width:10px !IMPORTANT;height:20px\"/>")
    );
    // It makes a declaration important, which a later one does not beat.
    assert_eq!(
        geometry_of("<rect width=\"50\" height=\"10\" style=\"width:10px!Important;width:20px\"/>"),
        plain("<rect width=\"10px\" height=\"10\" style=\"width:10px!Important;width:20px\"/>")
    );
    // The mark of a rule, which beats the `style` attribute.
    assert_eq!(
        geometry_of(
            "<style>rect{width:30px !IMPORTANT;height:20px}</style><rect width=\"50\" height=\"10\" style=\"width:10px\"/>"
        ),
        plain(
            "<style>rect{width:30px !IMPORTANT;height:20px}</style><rect width=\"30px\" height=\"20px\" style=\"width:10px\"/>"
        )
    );
    // A string that holds the mark is not a mark, and the attribute and the
    // text of the document keep their spelling.
    let body = "<rect width=\"50\" height=\"10\" style=\"width:10px;font-family:'!IMPORTANT'\"/><text>a !IMPORTANT</text>";
    assert_eq!(
        geometry_of(body),
        plain(&body.replace("width=\"50\"", "width=\"10px\""))
    );
}

#[test]
fn a_math_function_with_an_error_does_not_hide_the_declaration_below_it() {
    for (style, width) in [
        ("width:20px;width:calc(garbage)", Some("20px")),
        ("width:20px;width:calc(1px + 2)", Some("20px")),
        ("width:20px;width:calc(30)", Some("20px")),
        ("width:20px;width:min(1px, 2deg)", Some("20px")),
        ("width:20px;width:calc(1px) calc(2px)", Some("20px")),
        ("width:20px;width:-moz-calc(1px)", Some("20px")),
        // A function that a browser reads wins, and the attribute is left.
        ("width:20px;width:calc(1px + 2px)", None),
        ("width:20px;width:min(30px, 40px)", None),
        ("width:20px;width:calc(30px * 2px / 1px)", None),
        ("width:20px;width:calc(garbage var(--w))", None),
    ] {
        let body = format!("<rect width=\"50\" height=\"10\" style=\"{style}\"/>");
        let expected = match width {
            Some(width) => plain(&format!(
                "<rect width=\"{width}\" height=\"10\" style=\"{style}\"/>"
            )),
            None => plain(&body),
        };
        assert_eq!(geometry_of(&body), expected, "{style}");
    }
    // A number that a function gives is a length to a position, not to a size.
    let body = "<rect x=\"5\" width=\"50\" height=\"10\" style=\"x:20px;x:calc(40)\"/>";
    assert_eq!(geometry_of(body), plain(body));
    let body = "<rect x=\"5\" width=\"50\" height=\"10\" style=\"x:20px;x:calc(1px + 39)\"/>";
    assert_eq!(
        geometry_of(body),
        plain("<rect x=\"20px\" width=\"50\" height=\"10\" style=\"x:20px;x:calc(1px + 39)\"/>")
    );
    // Nor is a math function a path.
    let body = "<path d=\"M0 0H9\" style=\"d:path('M0 0H5');d:calc(1px)\"/>";
    assert_eq!(
        geometry_of(body),
        plain("<path d=\"M0 0H5\" style=\"d:path('M0 0H5');d:calc(1px)\"/>")
    );
}
