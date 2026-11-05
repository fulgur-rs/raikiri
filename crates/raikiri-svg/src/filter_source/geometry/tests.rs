use super::*;

fn attribute(text: &str) -> Parsed {
    Parsed::Value(Declared::Attribute(text.to_owned()))
}

const OPAQUE: Parsed = Parsed::Value(Declared::Opaque);

#[test]
fn a_property_name_followed_by_a_colon_may_declare_geometry() {
    for source in [
        "<rect style=\"width:10px\"/>",
        "<rect style='height : 10px'/>",
        "<style>circle{r:5}</style>",
        "<style>path { D\n:\npath('M0 0') }</style>",
        "<style>rect{x:1;y:2}</style>",
        "<style>ellipse{cx:1;cy:2;rx:3;ry:4}</style>",
        // A comment between the name and the colon.
        "<rect style=\"width/**/:20px\"/>",
        "<rect style=\"height /* c */ : 10px\"/>",
        "<style>circle{r/* a *//* b */:5}</style>",
        "<style>rect{x/* /* */:1}</style>",
    ] {
        assert!(may_declare(source), "{source}");
    }
    for source in [
        "<rect width=\"10\"/>",
        "<use xlink:href=\"#a\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"/>",
        "<rect style=\"min-width:1px;stroke-width:2;font-size:3px\"/>",
        "<rect style=\"fill:red\"/>",
        "<style>rect{rx-x:1}</style>",
        // A name needs a colon after it, and not the other way around.
        "<style>.d{fill:red}</style>",
        // CSS is read from `style` elements and attributes only.
        "<text>d: 1, width: 2</text>",
        "<rect><title>x: y</title></rect>",
        "",
    ] {
        assert!(!may_declare(source), "{source}");
    }
}

#[test]
fn an_element_has_the_geometry_properties_that_apply_to_it() {
    assert_eq!(
        properties("rect"),
        ["x", "y", "width", "height", "rx", "ry"]
    );
    assert_eq!(properties("circle"), ["cx", "cy", "r"]);
    assert_eq!(properties("ellipse"), ["cx", "cy", "rx", "ry"]);
    assert_eq!(properties("path"), ["d"]);
    assert_eq!(properties("image"), ["x", "y", "width", "height"]);
    assert_eq!(properties("use"), ["x", "y"]);
    // The position and size of these come from attributes only.
    for tag in [
        "svg",
        "line",
        "polygon",
        "text",
        "g",
        "foreignObject",
        "symbol",
    ] {
        assert!(properties(tag).is_empty(), "{tag}");
    }
}

#[test]
fn a_mask_tells_the_properties_a_rule_declares() {
    assert_eq!(declared_mask(&declarations("fill:red")), 0);
    assert_eq!(declared_mask(&declarations("FILL:red;min-width:1px")), 0);
    assert_eq!(declared_mask(&declarations("x:1")), 1);
    assert_eq!(declared_mask(&declarations("D:none")), 1 << 9);
    assert_eq!(declared_mask(&declarations("Filter:none")), FILTER);
    assert_eq!(
        declared_mask(&declarations("width:1;r:1")),
        mask(&["width", "r"])
    );
    assert_eq!(mask(&["x", "y"]), 0b11);
    assert_eq!(mask(&[]), 0);
    assert_eq!(mask(&["fill"]), 0);
}

#[test]
fn what_the_cascade_reads_is_the_length_of_the_values_of_its_properties() {
    let found = declarations("width:10px;fill:rgb(1,2,3);Height : 5% ;filter:url(#f);D:none");
    assert_eq!(declared_bytes(&found, mask(&["width", "height"])), 4 + 2);
    assert_eq!(declared_bytes(&found, mask(&["d"])), 4);
    // The filter is not read as a value, and a property that is not asked
    // for is not counted.
    assert_eq!(declared_bytes(&found, mask(&["width", "filter"])), 4);
    assert_eq!(declared_bytes(&found, FILTER), 0);
    assert_eq!(declared_bytes(&found, 0), 0);
}

#[test]
fn a_length_is_written_as_the_attribute_reads_it() {
    for (property, value, expected) in [
        ("width", "20px", "20px"),
        ("width", " 20PX ", "20px"),
        ("width", "20", "20"),
        ("width", "50%", "50%"),
        ("width", "+.5e2px", "+.5e2px"),
        ("width", "2e1", "2e1"),
        ("width", "1.5em", "1.5em"),
        ("width", "1EX", "1ex"),
        ("width", "10mm", "10mm"),
        ("width", "0", "0"),
        ("width", "-0px", "-0px"),
        ("x", "-5px", "-5px"),
        ("cy", "-5%", "-5%"),
        ("r", "10%", "10%"),
        ("rx", "2pt", "2pt"),
        ("height", "1in", "1in"),
        ("height", "1cm", "1cm"),
        ("ry", "1pc", "1pc"),
        ("width", "AUTO", "auto"),
        ("height", "auto", "auto"),
        ("rx", "Auto", "auto"),
        ("ry", "auto", "auto"),
    ] {
        assert_eq!(
            parse(property, value),
            attribute(expected),
            "{property}: {value}"
        );
    }
}

#[test]
fn a_value_that_is_not_one_of_the_property_is_invalid() {
    for (property, value) in [
        ("width", "abc"),
        ("width", "20px 30px"),
        ("width", "#fff"),
        ("width", "-5px"),
        ("width", "-1%"),
        ("height", "-0.1"),
        ("r", "-3px"),
        ("rx", "-3px"),
        ("ry", "-3px"),
        // Only a size and a radius of a rounded corner take `auto`.
        ("x", "auto"),
        ("cx", "auto"),
        ("r", "auto"),
        ("d", "auto"),
        ("d", "M0 0L1 1"),
        ("d", "'M0 0'"),
        ("d", "path(evenodd, 'M0 0')"),
        ("d", "path(M0 0)"),
        ("d", "path('M0 0' 'L1 1')"),
        ("d", "path('M0 0)"),
        ("d", "path()"),
        ("d", "path('M0 0\nL1 1')"),
        ("d", "path(\"M0 0\rL1 1\")"),
        ("d", "url(#p)"),
        ("d", "path ('M0 0')"),
        // Functions that no length is made of.
        ("width", "bogus(1)"),
        ("width", "url(a)"),
        ("width", "rgb(1, 2, 3)"),
        ("width", "calcs(1px)"),
        ("width", "xcalc(1px)"),
        ("width", "10px + calc(1px)"),
    ] {
        assert_eq!(
            parse(property, value),
            Parsed::Invalid,
            "{property}: {value}"
        );
    }
}

#[test]
fn a_value_that_is_not_computed_here_is_opaque() {
    for (property, value) in [
        ("width", "calc(10px + 20px)"),
        ("width", "var(--w)"),
        ("width", "min(10px, 20px)"),
        ("width", "CALC(1px + 2px)"),
        ("width", "-webkit-calc(1px + 2px)"),
        ("width", "clamp(1px, 2px, 3px)"),
        ("width", "2px var(--x)"),
        ("width", "env(safe-area-inset-left)"),
        ("width", "attr(data-w px)"),
        ("r", "sqrt(4) * 1px"),
        ("width", "2rem"),
        ("width", "10vw"),
        ("width", "1Q"),
        // A unit that nothing here knows may still be one.
        ("width", "20pxx"),
        // Too large for a float, which a browser takes for the largest one.
        ("width", "1e999px"),
        ("width", "inherit"),
        ("width", "REVERT"),
        ("width", "revert-layer"),
        ("width", "/* comment */20px"),
        ("width", "2\\30 px"),
        ("d", "var(--p)"),
        ("d", "inherit"),
        ("d", "path('M0 \\31 ')"),
    ] {
        assert_eq!(parse(property, value), OPAQUE, "{property}: {value}");
    }
}

#[test]
fn a_keyword_that_resets_a_property_gives_its_initial_value() {
    for (property, initial) in [
        ("width", "auto"),
        ("height", "auto"),
        ("rx", "auto"),
        ("ry", "auto"),
        ("x", "0"),
        ("y", "0"),
        ("cx", "0"),
        ("cy", "0"),
        ("r", "0"),
        ("d", ""),
    ] {
        for keyword in ["initial", "UNSET", " Initial "] {
            assert_eq!(parse(property, keyword), attribute(initial), "{property}");
        }
    }
}

#[test]
fn a_path_is_the_string_of_its_function() {
    for (value, data) in [
        ("path('M10 10L50 10')", "M10 10L50 10"),
        ("path(\"M10 10L50 10\")", "M10 10L50 10"),
        ("PATH( 'M0 0' )", "M0 0"),
        ("path('')", ""),
        ("path('a\"b')", "a\"b"),
        ("path(\"a'b\")", "a'b"),
        ("none", ""),
        ("NONE", ""),
    ] {
        assert_eq!(parse("d", value), attribute(data), "{value}");
    }
}

#[test]
fn a_number_with_a_unit_of_letters_is_told_from_the_rest() {
    for value in ["2rem", "-1.5vw", "+.5Q", "1e3ch", "1e-3vh", "10e", "5E"] {
        assert!(has_other_unit(value), "{value}");
    }
    for value in [
        "", "rem", "-", ".", "+.", "1 rem", "1-rem", "1%", "1px2", "1.2.3ch",
    ] {
        assert!(!has_other_unit(value), "{value}");
    }
}

fn declarations(css: &str) -> Vec<simplecss::Declaration<'_>> {
    simplecss::DeclarationTokenizer::from(css).collect()
}

fn winners(css: &str, properties: &[&'static str]) -> Vec<(&'static str, Declared)> {
    cascade(declarations(css), properties)
}

fn attributes(winners: &[(&str, Declared)]) -> Vec<(String, String)> {
    winners
        .iter()
        .map(|(property, declared)| {
            let Declared::Attribute(text) = declared else {
                panic!("{property} is opaque");
            };
            ((*property).to_owned(), text.clone())
        })
        .collect()
}

#[test]
fn the_last_declaration_of_a_property_wins() {
    let rect = properties("rect");
    assert_eq!(
        attributes(&winners("width:10px;height:20px;width:30px", rect)),
        [
            ("width".to_owned(), "30px".to_owned()),
            ("height".to_owned(), "20px".to_owned())
        ]
    );
    // A property that does not apply to the element is not looked at.
    assert!(winners("r:10px;cx:1px;d:none", rect).is_empty());
    assert!(winners("fill:red", rect).is_empty());
    // The case of a name does not matter.
    assert_eq!(
        attributes(&winners("WIDTH:10px;Width:5px", rect)),
        [("width".to_owned(), "5px".to_owned())]
    );
}

#[test]
fn an_important_declaration_beats_the_ones_that_are_not() {
    let rect = properties("rect");
    for (css, expected) in [
        ("width:10px!important;width:30px", "10px"),
        ("width:10px ! important;width:30px", "10px"),
        ("width:30px;width:10px!important", "10px"),
        // Among the important ones the last wins.
        ("width:10px!important;width:30px!important", "30px"),
    ] {
        assert_eq!(
            attributes(&winners(css, rect)),
            [("width".to_owned(), expected.to_owned())],
            "{css}"
        );
    }
}

#[test]
fn an_invalid_declaration_does_not_take_part() {
    let rect = properties("rect");
    for css in [
        "width:10px;width:abc",
        "width:10px;width:-5px!important",
        "width:10px!important;width:20px 30px!important",
        "width:10px;width:auto auto",
    ] {
        assert_eq!(
            attributes(&winners(css, rect)),
            [("width".to_owned(), "10px".to_owned())],
            "{css}"
        );
    }
    assert!(winners("width:abc", rect).is_empty());
}

#[test]
fn an_opaque_declaration_hides_the_ones_below_it() {
    let rect = properties("rect");
    assert_eq!(
        winners("width:10px;width:calc(1px + 2px)", rect),
        [("width", Declared::Opaque)]
    );
    // And a valid one above takes its place.
    assert_eq!(
        attributes(&winners("width:calc(1px + 2px);width:10px", rect)),
        [("width".to_owned(), "10px".to_owned())]
    );
    assert_eq!(
        winners("width:calc(1px + 2px)!important;width:10px", rect),
        [("width", Declared::Opaque)]
    );
}
