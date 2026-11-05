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
        // CDATA sections that cut a name up, or the colon off its name.
        "<style><![CDATA[rect{wid]]><![CDATA[th:1px}]]></style>",
        "<style><![CDATA[rect{width]]><![CDATA[:1px}]]></style>",
        "<style>rect{wid<![CDATA[th:1px}]]></style>",
        "<style>rect{width]]>:1px}</style>",
        // A character reference that the XML parser decodes.
        "<rect style=\"width&#58;20px\"/>",
        "<rect style=\"&#x77;idth:20px\"/>",
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
        // A style in a CDATA section is read as it stands.
        "<style><![CDATA[rect{fill:red;stroke-width:2}]]></style>",
        // The entities that XML predefines spell no name and no colon.
        "<rect style=\"fill:red\" id=\"a&amp;b&lt;&quot;\"/>",
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
        // The number of CSS: a dot has digits after it, and an exponent may
        // have a sign.
        ("width", "20.0px", "20.0px"),
        ("width", ".5px", ".5px"),
        ("width", "2.5E1PX", "2.5e1px"),
        ("width", "2e+1px", "2e+1px"),
        ("width", "20.0%", "20.0%"),
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
        // Chromium drops the declaration for an error anywhere in the data.
        ("d", "path('garbage')"),
        ("d", "path('M0 0H20V20H0Z garbage')"),
        ("d", "path('M0 0 L')"),
        ("d", "path('L10 10 L20 20')"),
        ("d", "path('M0 0 A5 5 0 2 0 10 10')"),
        ("d", "path('M0 0 L10 x')"),
        ("d", "path('M0 0H20V20H0Z,')"),
        ("d", "path('a\"b')"),
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
        ("r", "sqrt(4) * 1px"),
        ("width", "-moz-calc(1px)"),
        // A math function with an error, or of a type that is no length.
        ("width", "calc(garbage)"),
        ("height", "calc()"),
        ("x", "calc(1px + 39)"),
        ("cx", "min(1px, 2deg)"),
        ("width", "calc(30)"),
        ("height", "min(40, 50)"),
        ("width", "calc(1px) calc(2px)"),
        ("rx", "calc(10px+20px)"),
        ("ry", "clamp(1px, 2px)"),
        // A math function is no value of `d`.
        ("d", "calc(1px)"),
        ("d", "min(1px)"),
        ("d", "calc(garbage)"),
        // `svgtypes` reads the number of SVG, which CSS does not: a dot needs
        // a digit after it, and the unit is one word.
        ("width", "1.px"),
        ("width", "20."),
        ("width", "20.%"),
        ("height", "2.e1px"),
        ("cx", "1.5.px"),
        ("width", "20 px"),
        ("r", "5.e0"),
        // A suffix that is not a unit of length.
        ("width", "20pxx"),
        ("width", "1bogus"),
        ("width", "10e"),
        ("x", "20fr"),
        ("width", "20deg"),
        ("width", "20s"),
        // A unit that `svgtypes` lacks does not let a negative size through.
        ("width", "-1rem"),
        ("height", "-1vw"),
        ("r", "-2VMIN"),
        ("rx", "-0.5lh"),
        ("ry", "-1e3dvh"),
        ("rx", "-1e999px"),
        ("ry", "-1e999%"),
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
        ("r", "calc(sqrt(4) * 1px)"),
        ("width", "calc(garbage var(--x))"),
        ("width", "min(1px, var(--y))"),
        // A math function that a browser reads, with the types that fit.
        ("width", "calc(30px * 1px / 1px)"),
        ("height", "round(up, 21px, 10px)"),
        ("rx", "clamp(none, 3px, 4%)"),
        ("cy", "calc(-infinity * 1px)"),
        // A sizing function, which is not looked into.
        ("width", "calc-size(auto, size)"),
        ("height", "CALC-SIZE(30px, size)"),
        // A number that a function gives is a length to the position and the
        // radii, but not to a size.
        ("x", "calc(40)"),
        ("y", "min(40, 50)"),
        ("cx", "calc(20 + 20)"),
        ("r", "abs(40)"),
        ("rx", "calc(sign(5) * 40)"),
        ("width", "2rem"),
        ("width", "10vw"),
        ("width", "1Q"),
        // The newer units of length.
        ("width", "2VMIN"),
        ("width", "1dvh"),
        ("width", "3lh"),
        ("width", "5cqw"),
        ("height", "2rex"),
        // A negative offset is valid, and so is a size of negative zero.
        ("x", "-1rem"),
        ("cy", "-2VMIN"),
        ("width", "-0rem"),
        ("r", "-0e5vw"),
        // Too large for a float, which a browser takes for the largest one.
        ("width", "1e999px"),
        ("width", "1e999%"),
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
        // The data is checked for errors as a whole, and an empty path is none.
        ("path('M0,0 20,0 20,20 0,20z')", "M0,0 20,0 20,20 0,20z"),
        ("path('m0 0 h20 v20 h-20 z')", "m0 0 h20 v20 h-20 z"),
        ("path('  M0 0H20V20H0Z  ')", "  M0 0H20V20H0Z  "),
        ("path('M0 0H20V20H0ZZ')", "M0 0H20V20H0ZZ"),
        ("path('M0 0')", "M0 0"),
        ("path('M0 0H20.5.5V20H0Z')", "M0 0H20.5.5V20H0Z"),
        ("none", ""),
        ("NONE", ""),
    ] {
        assert_eq!(parse("d", value), attribute(data), "{value}");
    }
}

#[test]
fn a_number_with_a_unit_of_length_is_told_from_the_rest() {
    for value in [
        "2rem",
        "-1.5vw",
        "+.5Q",
        "1e3ch",
        "1e-3vh",
        "1.5E+2rem",
        "5DVH",
        "1e999px",
        "1%",
    ] {
        assert!(has_length_unit(value), "{value}");
    }
    for value in [
        "", "rem", "-", ".", "+.", "1 rem", "1-rem", "1px2", "1.2.3ch", "1.rem", "1.e1ch", "10e",
        "5E", "1bogus", "20pxx", "1fr", "1deg",
    ] {
        assert!(!has_length_unit(value), "{value}");
    }
}

#[test]
fn a_number_below_zero_is_told_from_the_rest() {
    for value in ["-1rem", "-.5vw", "-1e3ch", "-1e999px", "-0.0001em"] {
        assert!(is_negative(value), "{value}");
    }
    for value in [
        "1rem", "+1rem", "0", "-0rem", "-0.0vw", "-0e5ch", "", "-", "rem",
    ] {
        assert!(!is_negative(value), "{value}");
    }
}

#[test]
fn a_number_is_read_the_way_css_reads_it() {
    for (text, len) in [
        ("1", 1),
        ("+1", 2),
        ("-.5px", 3),
        ("20.0%", 4),
        ("1e3", 3),
        ("1E+3px", 4),
        ("1.5e-2em", 6),
        // The dot and the exponent that have no digits are not of the number.
        ("1.", 1),
        ("1.px", 1),
        ("1.e1", 1),
        ("1e", 1),
        ("1e+", 1),
        ("1em", 1),
        ("1.5.2", 3),
    ] {
        assert_eq!(number_len(text), Some(len), "{text}");
    }
    for text in ["", "+", "-", ".", "+.", ".e1", "px", "e1", "-px"] {
        assert_eq!(number_len(text), None, "{text}");
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
        "width:10px;width:20.px",
        "width:10px!important;width:1.!important",
        "width:10px;width:-1rem",
        "width:10px!important;width:-2vmin!important",
        "width:10px;width:calc(garbage)",
        "width:10px;width:calc(30)",
        "width:10px!important;width:calc(1px + 2)!important",
        "width:10px;width:min(1px, 2deg)",
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

/// The declarations of a `style` attribute as name, value and importance.
fn read(style: &str) -> Vec<(&str, &str, bool)> {
    style_declarations(style)
        .into_iter()
        .map(|declaration| (declaration.name, declaration.value, declaration.important))
        .collect()
}

#[test]
fn the_declarations_of_a_style_attribute_are_split_at_semicolons() {
    assert_eq!(
        read("width:10px;height:20px"),
        [("width", "10px", false), ("height", "20px", false)]
    );
    // Names keep their spelling, and white space and comments around them go.
    assert_eq!(
        read(" /* a */ WIDTH /* b */ : /* c */ 5px /* d */ ; Height:\n6px\t;"),
        [("WIDTH", "5px", false), ("Height", "6px", false)]
    );
    // Semicolons that stand alone, and a missing last one.
    assert_eq!(
        read(";;a:1;;;b:2;;"),
        [("a", "1", false), ("b", "2", false)]
    );
    assert!(read("").is_empty());
    assert!(read(" ;; /* a */ ").is_empty());
}

#[test]
fn a_nested_function_does_not_end_the_declaration_early() {
    // `simplecss` ends a function at its first closing parenthesis and loses
    // everything after it.
    assert_eq!(
        read("width:0;x:calc(min(1px, 2px));width:20px"),
        [
            ("width", "0", false),
            ("x", "calc(min(1px, 2px))", false),
            ("width", "20px", false),
        ]
    );
    assert_eq!(
        read("width:calc(min(1px,2px) + 20px);height:30px"),
        [
            ("width", "calc(min(1px,2px) + 20px)", false),
            ("height", "30px", false),
        ]
    );
    assert_eq!(
        read("x:a(b(c;d));width:20px"),
        [("x", "a(b(c;d))", false), ("width", "20px", false)]
    );
    assert_eq!(
        read("x:calc((1px + 2px) * 2)!important;y:1px"),
        [("x", "calc((1px + 2px) * 2)", true), ("y", "1px", false)]
    );
}

#[test]
fn a_semicolon_in_a_string_or_a_block_does_not_end_the_declaration() {
    for value in [
        "\"a;b\"",
        "'a;b'",
        "url(a;b)",
        "url(\"a;b\")",
        "u\\72l(a;b)",
        "{a;b}",
        "[a;b]",
        "(a;b)",
    ] {
        let style = format!("width:20px;x:{value};height:30px");
        assert_eq!(
            read(&style),
            [
                ("width", "20px", false),
                ("x", value, false),
                ("height", "30px", false)
            ],
            "{style}"
        );
    }
}

#[test]
fn a_block_that_is_not_closed_takes_the_rest_of_the_text() {
    assert_eq!(
        read("width:calc(20px;height:30px"),
        [("width", "calc(20px;height:30px", false)]
    );
    assert_eq!(
        read("width:20px;x:a(b(c;d);height:30px"),
        [
            ("width", "20px", false),
            ("x", "a(b(c;d);height:30px", false)
        ]
    );
    // A string that is not closed ends with its line, or with the text.
    assert_eq!(
        read("width:20px;font-family:\"a\n;height:30px"),
        [
            ("width", "20px", false),
            ("font-family", "\"a", false),
            ("height", "30px", false),
        ]
    );
    assert_eq!(
        read("width:20px;font-family:\"a;height:30px"),
        [
            ("width", "20px", false),
            ("font-family", "\"a;height:30px", false),
        ]
    );
}

#[test]
fn a_closing_mark_with_no_opening_one_stays_in_its_value() {
    // The value is not one of the property, so it is dropped there, and the
    // declarations after it are read.
    assert_eq!(
        read("width:20px);height:30px"),
        [("width", "20px)", false), ("height", "30px", false)]
    );
    assert_eq!(
        read("width:20px};height:30px"),
        [("width", "20px}", false), ("height", "30px", false)]
    );
    assert_eq!(
        read("width:20px;x:a(b(c;d)));height:30px"),
        [
            ("width", "20px", false),
            ("x", "a(b(c;d)))", false),
            ("height", "30px", false),
        ]
    );
}

#[test]
fn what_is_not_a_declaration_is_skipped_up_to_the_next_semicolon() {
    for style in [
        // No colon.
        "width;height:30px",
        // No name.
        ":width;height:30px",
        // Not a name.
        "123:x;height:30px",
        "*width:20px;height:30px",
        "\"width\":20px;height:30px",
        "#a:b;height:30px",
        // A function or a block at the place of the name.
        "foo(1;2);height:30px",
        "{x:1;y:2};height:30px",
        "[a;b];height:30px",
        // At-rules.
        "@foo;height:30px",
        "@media x{a:b};height:30px",
        // A name and a word where the colon is.
        "width 20px;height:30px",
        "width 20px {a;b};height:30px",
        // A name with a semicolon after it.
        "width;height:30px;",
        // No value.
        "width:;height:30px",
        "width: /* a */ ;height:30px",
        "width:\n;height:30px",
    ] {
        assert_eq!(read(style), [("height", "30px", false)], "{style}");
    }
    // The end of the text ends a declaration that is not one.
    assert_eq!(read("width:20px;height"), [("width", "20px", false)]);
    assert_eq!(read("width:20px;height:"), [("width", "20px", false)]);
    assert_eq!(read("width:20px;height:;"), [("width", "20px", false)]);
    assert!(read("width").is_empty());
    assert!(read("{a;b").is_empty());
}

#[test]
fn a_comment_at_the_end_of_a_value_is_not_part_of_it() {
    assert_eq!(
        read("width:20px /* c */;height:30px/* unterminated"),
        [("width", "20px", false), ("height", "30px", false)]
    );
    assert_eq!(
        read("/* c */ width:20px;/* d */height:30px"),
        [("width", "20px", false), ("height", "30px", false)]
    );
    // One inside the value stays, and the value is not read any further.
    assert_eq!(read("x:1/**/2"), [("x", "1/**/2", false)]);
}

#[test]
fn an_important_mark_is_read_in_any_case() {
    for (style, value) in [
        ("width:10px !important", "10px"),
        ("width:10px !IMPORTANT", "10px"),
        ("width:10px!Important", "10px"),
        ("width:10px ! important ", "10px"),
        ("width:10px !\n\tIMPORTANT", "10px"),
        ("width:10px !/* c */IMPORTANT", "10px"),
        ("width:10px !/* a *//* b */ Important /* c */", "10px"),
        ("width:10px /* a */ ! /* b */ important", "10px"),
        ("width:calc(1px + 2px)!IMPORTANT", "calc(1px + 2px)"),
        ("width:\"x\" !IMPORTANT", "\"x\""),
        ("width:url(x) !IMPORTANT", "url(x)"),
        ("width:1px!important", "1px"),
    ] {
        assert_eq!(read(style), [("width", value, true)], "{style}");
    }
    // The mark is for the declaration that it ends.
    assert_eq!(
        read("a:b!Important;c:d !IMPORTANT;e:f"),
        [("a", "b", true), ("c", "d", true), ("e", "f", false)]
    );
    assert_eq!(read("\u{e9}:1 !IMPORTANT"), [("\u{e9}", "1", true)]);
}

#[test]
fn text_that_is_not_an_important_mark_stays_in_the_value() {
    for (style, value, important) in [
        ("width:10px !important x", "10px !important x", false),
        ("width:10px !IMPORTANTX", "10px !IMPORTANTX", false),
        ("width:10px !IMPORTANT-", "10px !IMPORTANT-", false),
        ("width:10px !IMPORTANT_1", "10px !IMPORTANT_1", false),
        (
            "width:10px !IMPORTANT\u{e9}",
            "10px !IMPORTANT\u{e9}",
            false,
        ),
        ("width:10px !IMPORTAN", "10px !IMPORTAN", false),
        ("width:10px ! x IMPORTANT", "10px ! x IMPORTANT", false),
        ("width:10px ! 1 important", "10px ! 1 important", false),
        ("width:10px important", "10px important", false),
        ("width:10px !important !", "10px !important !", false),
        // Only the last mark is one, and the one before it stays.
        ("width:10px !important !important", "10px !important", true),
        ("width:10px !! important", "10px !", true),
        ("width:!", "!", false),
        ("width:\"!important\"", "\"!important\"", false),
        ("width:'!IMPORTANT'", "'!IMPORTANT'", false),
        ("width:url(a!IMPORTANT)", "url(a!IMPORTANT)", false),
        ("width:(a !important)", "(a !important)", false),
    ] {
        assert_eq!(read(style), [("width", value, important)], "{style}");
    }
}

/// The declarations of the rules of a sheet, as name, value and importance.
fn declared(sheet: &simplecss::StyleSheet<'_>) -> Vec<Vec<(String, String, bool)>> {
    sheet
        .rules
        .iter()
        .map(|rule| {
            rule.declarations
                .iter()
                .map(|d| (d.name.to_owned(), d.value.to_owned(), d.important))
                .collect()
        })
        .collect()
}

/// A declaration, to compare with.
fn d(name: &str, value: &str, important: bool) -> (String, String, bool) {
    (name.to_owned(), value.to_owned(), important)
}

/// The sheet of `css`, with the geometry of its rules read again.
fn sheet_again(css: &str) -> simplecss::StyleSheet<'_> {
    let mut sheet = simplecss::StyleSheet::new();
    sheet.parse_more(css);
    read_blocks_again(&mut sheet.rules, css);
    sheet
}

#[test]
fn a_nested_function_does_not_end_the_geometry_of_a_rule() {
    assert_eq!(
        declared(&sheet_again(
            "rect{width:0;x:calc(min(1px, 2px));width:20px}"
        )),
        [[
            d("width", "0", false),
            d("x", "calc(min(1px, 2px))", false),
            d("width", "20px", false),
        ]]
    );
    // The mark of an important declaration is read in any case.
    assert_eq!(
        declared(&sheet_again("rect{width:30px !IMPORTANT;height:20px}")),
        [[d("width", "30px", true), d("height", "20px", false)]]
    );
    // And the end of the text ends a block that is not closed.
    assert_eq!(
        declared(&sheet_again("rect{x:calc(min(1px, 2px));width:20px")),
        [[
            d("x", "calc(min(1px, 2px))", false),
            d("width", "20px", false)
        ]]
    );
}

#[test]
fn a_rule_that_was_read_whole_is_left_as_it_is() {
    let css = "rect{width:10px;fill:red;height:20px}circle{r:5}";
    let mut before = simplecss::StyleSheet::new();
    before.parse_more(css);
    assert_eq!(declared(&sheet_again(css)), declared(&before));
}

#[test]
fn only_the_geometry_of_a_rule_is_read_again() {
    // `simplecss` stops at the function, so `stroke` is not one of its
    // declarations, and it stays out. The geometry follows the others.
    assert_eq!(
        declared(&sheet_again(
            "rect{fill:red;x:calc(min(1px, 2px));width:20px;stroke:blue}"
        )),
        [[
            d("fill", "red", false),
            d("x", "calc(min(1px, 2px))", false),
            d("width", "20px", false),
        ]]
    );
}

#[test]
fn the_rules_of_a_selector_list_share_their_block() {
    let rule = vec![
        d("x", "calc(min(1px, 2px))", false),
        d("width", "20px", false),
    ];
    assert_eq!(
        declared(&sheet_again(
            "rect, circle, ellipse{x:calc(min(1px, 2px));width:20px}"
        )),
        vec![rule; 3]
    );
}

#[test]
fn a_block_ends_at_its_closing_brace() {
    assert_eq!(
        declared(&sheet_again(
            "a{x:calc(min(1px, 2px));width:1px}b{height:2px}"
        )),
        [
            vec![
                d("x", "calc(min(1px, 2px))", false),
                d("width", "1px", false)
            ],
            vec![d("height", "2px", false)],
        ]
    );
    // A brace that a block opens is closed by its own, and the declarations
    // go on after it.
    assert_eq!(
        declared(&sheet_again(
            "rect{x:calc(1px);fill:{a;b};width:20px}a{r:1}"
        )),
        [
            vec![d("x", "calc(1px)", false), d("width", "20px", false)],
            vec![d("r", "1", false)],
        ]
    );
}

#[test]
fn a_rule_is_read_again_in_the_text_that_it_came_from() {
    let both = "a{x:calc(min(1px, 2px));width:1px}b{y:calc(min(1px, 2px));height:2px}";
    let (first, second) = both.split_at(both.find('b').unwrap());
    let mut sheet = simplecss::StyleSheet::new();
    sheet.parse_more(first);
    sheet.parse_more(second);
    // The rule of the second text is not one of the first, and the other way
    // round, so a text only changes its own rules.
    read_blocks_again(&mut sheet.rules, first);
    assert_eq!(
        declared(&sheet),
        [
            vec![
                d("x", "calc(min(1px, 2px))", false),
                d("width", "1px", false)
            ],
            vec![d("y", "calc(min(1px, 2px)", false)],
        ]
    );
    read_blocks_again(&mut sheet.rules, second);
    assert_eq!(
        declared(&sheet),
        [
            vec![
                d("x", "calc(min(1px, 2px))", false),
                d("width", "1px", false)
            ],
            vec![
                d("y", "calc(min(1px, 2px))", false),
                d("height", "2px", false)
            ],
        ]
    );
}

#[test]
fn a_rule_without_declarations_has_no_block_to_read() {
    let mut rules = vec![simplecss::Rule {
        selector: simplecss::Selector::parse("rect").unwrap(),
        declarations: Vec::new(),
    }];
    read_blocks_again(&mut rules, "rect{width:20px}");
    assert!(rules[0].declarations.is_empty());
}
