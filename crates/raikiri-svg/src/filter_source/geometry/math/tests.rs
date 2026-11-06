use super::*;

// The tables hold what Chromium does with the same text as the value of
// `width` of a `rect` that has a declaration of `20px` before it: the ones in
// `VALID` give the width that they come to, and the ones in `INVALID` are
// dropped, so that the width stays 20.

const VALID: &[&str] = &[
    "calc(30px)",
    "calc(10px + 20px)",
    "CALC(10PX + 20PX)",
    "calc( 30px )",
    "calc((30px))",
    "calc(((10px + 20px)))",
    "calc(10px + 20%)",
    "calc(2 * 15px)",
    "calc(15px * 2)",
    "calc(60px / 2)",
    "calc(1px * 2 * 15)",
    "calc(10px - -20px)",
    "calc(10px + 5px * 4)",
    "calc(30%)",
    "calc(30%/1)",
    "calc(+30px)",
    "calc(-10px)",
    "calc(30px * -1 * -1)",
    "calc(1e1px * 3)",
    "calc(3e1px)",
    "calc(.5 * 60px)",
    "calc(1em * 2)",
    "calc(100vw - 70px)",
    "calc(1px + 1Q)",
    "-webkit-calc(30px)",
    "-WEBKIT-CALC(30px)",
    "calc(calc(30px))",
    // The white space of every kind.
    "calc(30px)\t",
    "calc(\n30px\n)",
    "calc(30px +\n0px)",
    "calc(30px\t+\t0px)",
    "calc(30px\x0c+\x0c0px)",
    "calc(30px*1)",
    "calc(1*30px)",
    "calc(90px/3)",
    // An input that ends closes every block that is open.
    "calc(30px",
    "calc(10px - 20px",
    "calc(((10px))",
    "calc(1px + (2px)",
    // The lists of arguments.
    "min(30px, 40px)",
    "max(10px, 30px)",
    "min(30px)",
    "min(30px,40px)",
    "min( 30px,40px )",
    "min(30px , 40px)",
    "min(30px,\n40px)",
    "min(30px, 40px, 50px)",
    "min(10px + 20px, 40px, 50px)",
    "min((30px), 40px)",
    "min(calc(30px), 40px)",
    "max(min(30px, 40px), 10px)",
    "min(100%, 30px)",
    "clamp(10px, 30px, 40px)",
    "clamp(10px, 2 * 15px, 40px)",
    "clamp(none, 30px, none)",
    "clamp(10px, 30px, none)",
    "clamp(none, 30px, 40px)",
    "clamp(NONE, 30px, none)",
    "hypot(30px)",
    "hypot(18px, 24px)",
    "abs(-30px)",
    "calc(abs(-30px))",
    "calc(abs(-30) * 1px)",
    "mod(30px, 100px)",
    "rem(100px, 70px)",
    "rem(1px, 1%)",
    "calc(mod(100, 70) * 1px)",
    "round(33px, 10px)",
    "round(30px, 0px)",
    "round(nearest, 30px, 1px)",
    "round(to-zero, 30px, 1px)",
    "round(UP, 30px, 1px)",
    "round(up, 21px, 10px)",
    "calc(round(30.4) * 1px)",
    // Functions of numbers and angles.
    "calc(sin(90deg) * 30px)",
    "calc(sin(30deg) * 60px)",
    "calc(sin(1turn) * 10px + 30px)",
    "calc(cos(0) * 30px)",
    "calc(cos(100grad) * 1px + 30px)",
    "calc(tan(0deg) * 1px + 30px)",
    "calc(30px + acos(1) / 1deg * 1px)",
    "calc(atan(1) / 1deg * 0.6667px)",
    "calc(atan2(30px, 30px) / 1deg * 0.6667px)",
    "calc(pow(2, 3) * 5px)",
    "calc(30px * pow(1, 5))",
    "calc(sqrt(900) * 1px)",
    "calc(30px * sqrt(1))",
    "calc(log(100) * 15px)",
    "calc(30px * log(1, 2))",
    "calc(exp(0) * 30px)",
    "calc(sign(-3px) * -30px)",
    "calc(sign(1px) * 30px)",
    "calc(hypot(3px, 4px) * 6)",
    // The constants.
    "calc(pi * 10px)",
    "calc(e * 10px)",
    "calc(PI * 10px)",
    "calc(E * 10px)",
    "calc(infinity * 1px)",
    "calc(INFINITY * 1px)",
    "calc(30px * -infinity)",
    "calc(-infinity * 1px)",
    "calc(NaN * 1px)",
    // A quotient by zero, which a browser reads and does not refuse.
    "calc(1px / 0)",
    "calc(1 / 0 * 1px)",
    "calc(30px / (2 - 2))",
    // Typed arithmetic: the units of the factors make the type.
    "calc(30px * 1px / 1px)",
    "calc(60px / 2px * 1px)",
    "calc(10% / 10% * 1px)",
    "calc(1deg / 1deg * 30px)",
    "calc(1s * 1px / 1s)",
    "calc(30px * 1Hz / 1hz)",
    "calc(30px * 1x / 1dppx)",
    "calc(30px + 1deg / 1deg * 0px)",
    "calc(30px + 0 * 1px)",
    "calc(30px * (1 + 0))",
    "calc(30px * (2 / 2))",
    "calc(30px * 1e0)",
    "calc(30px * 1E0)",
    "calc(30px * 1e+0)",
    "calc(30px * 1e-0)",
    "calc(1 / 3 * 90px)",
];

const INVALID: &[&str] = &[
    // Not a number, or not an expression.
    "calc(garbage)",
    "calc()",
    "calc( )",
    "calc(foo)",
    "calc(--x)",
    "calc(auto)",
    "calc(none)",
    "calc(10px + \"a\")",
    "calc(10px + foo)",
    "calc(10px + \u{e9})",
    "calc(10px + foo(1))",
    "calc(10px + bogus(1px))",
    "calc({30px})",
    "calc([30px])",
    "calc(30px;)",
    "calc(1px !important)",
    // The operators.
    "calc(1px +)",
    "calc(+)",
    "calc(1px + )",
    "calc(1px *)",
    "calc(* 10px)",
    "calc(10px ** 2)",
    "calc(10px / )",
    "calc(1px+)",
    "calc(10px+20px)",
    "calc(10px +20px)",
    "calc(10px+ 20px)",
    "calc(10px -20px)",
    "calc(10px- 20px)",
    "calc(- 10px)",
    "calc(+ 10px)",
    "calc(-(10px))",
    "calc(+(30px))",
    "calc(-(-30px))",
    "calc(1px - - 1px)",
    "calc(1px + + 1px)",
    "calc(1px * + 2)",
    "calc(1px * - 2)",
    // Two terms and no operator, or a list in a function of one term.
    "calc(1px 2px)",
    "calc(1px 2)",
    "calc(1px,2px)",
    "calc(30px, )",
    "calc(10 px)",
    "calc(10px + 10 %)",
    // What follows the function, or what stands before it.
    "calc(1px) calc(2px)",
    "calc(1px)x",
    "calc(30px) 1px",
    "calc(30px) !important x",
    "x calc(1px)",
    "calc(10px))",
    "calc (30px)",
    "calcs(1px)",
    "-moz-calc(30px)",
    // The types.
    "calc(30)",
    "calc(0)",
    "calc(1e3)",
    "calc(1px + 2)",
    "calc(2 + 1px)",
    "calc(30px + 0)",
    "calc(1 + 2 * 10px)",
    "calc(10px + 1deg)",
    "calc(10px + 1s)",
    "calc(30px + 1deg * 0)",
    "calc(1deg * 1px + 30px)",
    "calc(10px * 2px)",
    "calc(10px / 2px)",
    "calc(30px / 1px)",
    "calc(10px / 2px / 1px)",
    "calc(infinity)",
    "calc(pi)",
    "calc(30px + pi)",
    "calc(sin(90deg))",
    "sin(90deg)",
    "sign(30px)",
    "atan2(30px, 30px)",
    "calc(round(30.4))",
    "calc(sign(30px) * 30)",
    "calc(asin(1) * 0 + 30px)",
    "calc(atan2(1px, 1px) * 0 + 30px)",
    "calc(atan2(1px, 1) * 0 + 30px)",
    "calc(tan(0) + 30px)",
    "calc(30px + acos(1deg) * 1px)",
    "calc(30px * 1fr / 1fr)",
    "calc(30px * 1e)",
    "calc(30.px)",
    "calc(30px + 20.px)",
    // The arguments of the functions.
    "min()",
    "min(1px,)",
    "min(,1px)",
    "min(1px 2px)",
    "min(1px, 2deg)",
    "min(1px, 2)",
    "min(30px, none)",
    "min(1px,2px,)",
    "max(1px;2px)",
    "clamp(1px)",
    "clamp(1px, 2px)",
    "clamp(1px, 2px, 3px, 4px)",
    "clamp(1px,,3px)",
    "clamp(1px, 2, 3px)",
    "clamp(none, none, none)",
    "clamp(none, 2, 3px)",
    "clamp(1px, 2px, 3)",
    "round()",
    "round(1px)",
    "round(1px,)",
    "round(up)",
    "round(bogus, 30px, 1px)",
    "round(up, 33px)",
    "round(up 21px, 10px)",
    "round(, 21px, 10px)",
    "round(up, , 10px)",
    "round(33px, 10)",
    "round(33, 10px)",
    "calc(round(30.4,) * 1px)",
    "mod(1px)",
    "mod(1px, 2)",
    "mod(100px, 70px, 1px)",
    "mod(100px, 70)",
    "mod(100, 70px)",
    "rem(1px, 1deg)",
    "abs()",
    "abs(1px, 2px)",
    "calc(abs(30px, 1))",
    "calc(abs(30))",
    "sign()",
    "calc(sign() * 30px)",
    "hypot()",
    "hypot(3px, 4)",
    "calc(sin(1px))",
    "calc(sin())",
    "calc(sin(1s) * 10px + 30px)",
    "calc(pow(1px, 2))",
    "calc(pow(2) * 1px)",
    "calc(pow(2, 2, 2) * 1px)",
    "calc(pow(2px, 2) * 1px)",
    "calc(sqrt(4px))",
    "calc(sqrt(1, 2) * 30px)",
    "calc(log(1px))",
    "calc(30px * log())",
    "calc(30px * log(1, 2, 3))",
    "calc(exp(1px))",
    "calc(30px * exp(0, 1))",
    "calc(hypot())",
];

#[test]
fn a_math_function_that_a_browser_reads_is_a_length() {
    for value in VALID {
        assert!(is_length(value, false), "{value:?}");
        assert!(is_length(value, true), "{value:?}");
    }
}

#[test]
fn a_math_function_that_a_browser_drops_is_not_a_length() {
    for value in INVALID {
        assert!(!is_length(value, false), "{value:?}");
    }
}

#[test]
fn a_number_is_a_length_to_the_properties_that_take_it() {
    // Chromium reads `x:calc(40)` and `r:min(40, 50)` as lengths, and refuses
    // them for a width or a height.
    for value in [
        "calc(40)",
        "calc(20 + 20)",
        "min(40, 50)",
        "calc(2 * 20)",
        "calc(80 / 2)",
        "max(40)",
        "clamp(1, 40, 50)",
        "abs(40)",
        "calc(round(40.2))",
        "calc(sign(5) * 40)",
    ] {
        assert!(!is_length(value, false), "{value:?}");
        assert!(is_length(value, true), "{value:?}");
    }
    // A sum of a number and a length is no type at all, and an angle is no
    // length whatever the property.
    for value in [
        "calc(1px + 39)",
        "calc(1deg)",
        "calc(garbage)",
        "calc(10px + 1deg)",
        "min(1px, 2)",
        "calc(40 * 1deg)",
    ] {
        assert!(!is_length(value, true), "{value:?}");
    }
}

#[test]
fn a_value_that_is_not_one_function_is_not_a_math_function() {
    for value in [
        "",
        "30px",
        "(30px)",
        "bogus(30px)",
        "var(--x)",
        "calc",
        "-calc(30px)",
        "30px calc(1px)",
    ] {
        assert!(!is_length(value, true), "{value:?}");
    }
}

#[test]
fn blocks_nest_to_a_limited_depth() {
    // Chromium reads a hundred blocks, and drops a hundred and one.
    let functions = |depth: usize| format!("{}30px{}", "calc(".repeat(depth), ")".repeat(depth));
    assert!(is_length(&functions(100), false));
    assert!(!is_length(&functions(101), false));
    assert!(!is_length(&functions(100_000), false));
    let parentheses =
        |depth: usize| format!("calc({}30px{})", "(".repeat(depth), ")".repeat(depth));
    assert!(is_length(&parentheses(99), false));
    assert!(!is_length(&parentheses(100), false));
    assert!(!is_length(&parentheses(100_000), false));
    let lists = |depth: usize| format!("{}30px{}", "min(".repeat(depth), ")".repeat(depth));
    assert!(is_length(&lists(100), false));
    assert!(!is_length(&lists(101), false));
}

#[test]
fn a_token_is_told_by_its_first_characters() {
    fn kinds(text: &str) -> Vec<(Kind<'_>, bool)> {
        let mut parser = Parser {
            text,
            at: 0,
            depth: 0,
        };
        let mut kinds = Vec::new();
        loop {
            let token = parser.take();
            if token.kind == Kind::End {
                return kinds;
            }
            kinds.push((token.kind, token.space));
        }
    }
    // A number that ends the text has nothing after it.
    assert_eq!(kinds("12"), [(Kind::Value(NUMBER), false)]);
    assert_eq!(
        kinds("-1.5e2px"),
        [(Kind::Value(LENGTH), false)],
        "a signed number with a unit"
    );
    assert_eq!(
        kinds(" 1 -1 - 1 + -x"),
        [
            (Kind::Value(NUMBER), true),
            (Kind::Value(NUMBER), true),
            (Kind::Minus, true),
            (Kind::Value(NUMBER), true),
            (Kind::Plus, true),
            (Kind::Ident("-x"), true),
        ]
    );
    assert_eq!(
        kinds("50% 2.5deg 1e3-.5s 1px-2px"),
        [
            (Kind::Value(LENGTH), false),
            (Kind::Value(ANGLE), true),
            (Kind::Value(NUMBER), true),
            (Kind::Value(Dim([0, 0, 1, 0, 0])), false),
            // The unit takes the hyphen and the digits after it, as a name does.
            (Kind::Other, true),
        ]
    );
    assert_eq!(
        kinds("min(a,b)*/ ."),
        [
            (Kind::Function("min"), false),
            (Kind::Ident("a"), false),
            (Kind::Comma, false),
            (Kind::Ident("b"), false),
            (Kind::Close, false),
            (Kind::Star, false),
            (Kind::Slash, false),
            (Kind::Other, true),
        ]
    );
    assert_eq!(
        kinds("--x _y \u{e9}"),
        [
            (Kind::Ident("--x"), false),
            (Kind::Ident("_y"), true),
            (Kind::Ident("\u{e9}"), true),
        ]
    );
    assert_eq!(kinds("1\u{2192}"), [(Kind::Other, false)]);
}

// What Chromium makes of each text as the width of a `rect`: the number of
// pixels, to the single precision that it keeps.
const WORKED_OUT: &[(&str, f32)] = &[
    ("calc(5)", 5.0),
    ("calc(5px)", 5.0),
    ("calc(2 * 3px)", 6.0),
    ("calc(1in)", 96.0),
    ("calc(1cm)", 37.795277),
    ("calc(1mm)", 3.7795277),
    ("calc(1Q)", 0.9448819),
    ("calc(1pt)", 1.3333334),
    ("calc(1pc)", 16.0),
    ("calc(10px / 4)", 2.5),
    ("calc(10px + 5px * 2)", 20.0),
    ("calc((10px + 5px) * 2)", 30.0),
    ("calc(-5px + 20px)", 15.0),
    ("calc(1e1px)", 10.0),
    ("calc(.5px)", 0.5),
    ("calc(+5px)", 5.0),
    ("min(5px, 3px)", 3.0),
    ("max(5px, 3px, 9px)", 9.0),
    ("clamp(1px, 5px, 3px)", 3.0),
    ("clamp(10px, 5px, 30px)", 10.0),
    ("clamp(none, 5px, 3px)", 3.0),
    ("clamp(1px, 5px, none)", 5.0),
    ("clamp(10px, 5px, 3px)", 10.0),
    ("round(2.6px, 1px)", 3.0),
    ("round(2.5px, 1px)", 3.0),
    ("round(-2.5px, 1px)", -2.0),
    ("round(up, 2.1px, 1px)", 3.0),
    ("round(down, 2.9px, 1px)", 2.0),
    ("round(to-zero, -2.9px, 1px)", -2.0),
    ("round(nearest, 7px, 4px)", 8.0),
    ("round(up, 7px, 4px)", 8.0),
    ("round(2.6)", 3.0),
    ("round(up, 2.1)", 3.0),
    ("round(up, 5.5px, -2px)", 6.0),
    ("round(down, 5.5px, -2px)", 4.0),
    ("mod(7px, 3px)", 1.0),
    ("mod(-7px, 3px)", 2.0),
    ("mod(7px, -3px)", -2.0),
    ("rem(7px, 3px)", 1.0),
    ("rem(-7px, 3px)", -1.0),
    ("rem(7px, -3px)", 1.0),
    ("abs(-5px)", 5.0),
    ("calc(sign(-3px) * 4px)", -4.0),
    ("calc(sign(0) * 4px)", 0.0),
    ("calc(sin(90deg) * 10px)", 10.0),
    ("calc(cos(0) * 10px)", 10.0),
    ("calc(tan(45deg) * 10px)", 10.0),
    ("calc(sin(1) * 10px)", 8.41471),
    ("calc(asin(1) / 1deg * 1px)", 90.0),
    ("calc(acos(0.5) / 1deg * 1px)", 60.0),
    ("calc(atan(1) / 1deg * 1px)", 45.0),
    ("calc(atan2(1px, 1px) / 1deg * 1px)", 45.0),
    ("calc(atan2(-1px, -1px) / 1deg * 1px)", -135.0),
    ("calc(sqrt(16) * 1px)", 4.0),
    ("calc(pow(2, 3) * 1px)", 8.0),
    ("calc(exp(0) * 1px)", 1.0),
    ("calc(log(100, 10) * 1px)", 2.0),
    ("calc(log(e) * 1px)", 1.0),
    ("calc(hypot(3px, 4px))", 5.0),
    ("calc(pi * 1px)", std::f32::consts::PI),
    ("calc(e * 1px)", std::f32::consts::E),
    ("calc(1turn / 1deg * 1px)", 360.0),
    ("calc(1rad / 1deg * 1px)", 57.29578),
    ("calc(100grad / 1deg * 1px)", 90.0),
    ("calc(10px * 3)", 30.0),
    ("calc(10px / 3)", 3.3333333),
    ("calc(1px * 1px / 1px)", 1.0),
    ("calc(2 * (3 + 4) * 1px)", 14.0),
    ("calc(1px - -1px)", 2.0),
    ("calc(calc(1px + 2px) * 2)", 6.0),
    ("min(1px, 2px, max(3px, 4px))", 1.0),
    ("-webkit-calc(5px)", 5.0),
    ("CALC(5px)", 5.0),
    ("calc(5px/ 2)", 2.5),
    ("min(5, 3)", 3.0),
    ("max(1, 2)", 2.0),
    ("calc(2 * 3)", 6.0),
    ("round(2.5)", 3.0),
    ("calc(1.5)", 1.5),
    ("calc(1e-7px)", 1e-7),
    ("calc(1e10px)", 1e10),
    ("calc(0px * -1)", 0.0),
    ("calc(100px / 3)", 33.333332),
    ("calc(1px * 1000000 * 1000000)", 1e12),
    ("calc(2.5px * 2.5)", 6.25),
];

#[test]
fn a_math_function_comes_to_what_a_browser_makes_of_it() {
    for (value, expected) in WORKED_OUT {
        let Some(Length::Pixels(pixels)) = length(value, true) else {
            panic!("{value:?} is not worked out");
        };
        assert!(
            (pixels - expected).abs() <= expected.abs() * 1e-6,
            "{value:?}: {pixels} against {expected}"
        );
    }
}

#[test]
fn the_sign_of_the_step_of_round_does_not_matter() {
    for (value, expected) in [
        ("round(up, 5.5px, 2px)", 6.0),
        ("round(up, 5.5px, -2px)", 6.0),
        ("round(down, 5.5px, -2px)", 4.0),
        ("round(to-zero, -5.5px, -2px)", -4.0),
        ("round(nearest, 5px, -2px)", 6.0),
        // The quotient just under one half is rounded down, and one half up.
        ("round(0.49999999999999994px, 1px)", 0.0),
        ("round(nearest, 0.5px, 1px)", 1.0),
        ("round(nearest, -0.5px, 1px)", 0.0),
        ("round(nearest, 1.5px, 1px)", 2.0),
    ] {
        assert_eq!(
            length(value, true),
            Some(Length::Pixels(expected)),
            "{value}"
        );
    }
}

#[test]
fn a_length_that_depends_on_where_it_is_used_has_no_value() {
    for value in [
        "calc(50%)",
        "calc(10px + 20%)",
        "calc(1em)",
        "calc(2rem + 1px)",
        "calc(1vw)",
        "min(1px, 50%)",
        "max(1px, 1dvh)",
        "clamp(1px, 2em, 3px)",
        "clamp(1em, 2px, 3px)",
        "clamp(1px, 2px, 3%)",
        "round(up, 1em, 1px)",
        "round(up, 1px, 1em)",
        "abs(1em)",
        "calc(2 * 1ch)",
        // The units that are worked out in no case here.
        "calc(2s / 1s * 3px)",
        "calc(2hz / 1hz * 3px)",
        "calc(2dppx / 1dppx * 3px)",
        // Nor a number that is not finite, which a browser clamps.
        "calc(1px / 0)",
        "calc(0 * 1px / 0)",
        "calc(sqrt(-1) * 1px)",
        "calc(1px * infinity)",
        "calc(-infinity * 1px)",
        "calc(nan * 1px)",
        "calc(1px * 1e300 * 1e300)",
        "mod(1px, 0px)",
        "rem(1px, 0px)",
        "round(1px, 0px)",
        // Or one that a single precision number cannot hold.
        "calc(1px * 1e300)",
    ] {
        assert_eq!(length(value, true), Some(Length::Unresolved), "{value}");
    }
}

#[test]
fn the_type_of_a_term_is_checked_whether_or_not_it_has_a_value() {
    for value in [
        "calc(1em + 1)",
        "calc(50% + 1deg)",
        "min(1em, 1deg)",
        "clamp(1em, 2px, 3deg)",
        "round(up, 1em, 1deg)",
        "calc(2s * 3px)",
    ] {
        assert_eq!(length(value, true), None, "{value}");
    }
}

#[test]
fn a_number_is_a_length_only_where_it_is_asked_for() {
    assert_eq!(length("calc(4)", true), Some(Length::Pixels(4.0)));
    assert_eq!(length("calc(4)", false), None);
    assert_eq!(length("calc(4px)", false), Some(Length::Pixels(4.0)));
    // An angle, a time and the like are not lengths.
    assert_eq!(length("calc(4deg)", true), None);
    assert_eq!(length("calc(4s)", true), None);
}

#[test]
fn the_pixels_of_an_attribute_are_a_short_number() {
    for (pixels, text) in [
        (0.0, "0"),
        (-0.0, "0"),
        (5.0, "5"),
        (-5.0, "-5"),
        (1.5, "1.5"),
        (17.333334, "17.333334"),
        (0.000_001, "0.000001"),
        (999_999_000.0, "999999000"),
        (1e9, "1e9"),
        (-2.5e9, "-2.5e9"),
        (1e-7, "1e-7"),
        (f32::MAX, "3.4028235e38"),
    ] {
        assert_eq!(pixels_text(pixels), text);
    }
}

#[test]
fn a_source_may_hold_a_math_function_where_a_name_of_one_has_a_parenthesis() {
    for source in [
        "<rect width=\"calc(1px)\"/>",
        "<rect width=\"CALC(1px)\"/>",
        "<rect width=\"-webkit-calc(1px)\"/>",
        "<rect width=\"min(1,2)\" height=\" max(1,2)\"/>",
        "<rect width=\"sin(1) \"/>",
        "<rect transform=\"translate(1)\" width=\"clamp(1,2,3)\"/>",
        // A character reference may spell the parenthesis.
        "<rect width=\"calc&#40;1px)\"/>",
    ] {
        assert!(may_hold(source), "{source}");
    }
    for source in [
        "",
        "<rect width=\"5\"/>",
        "<rect transform=\"translate(1) rotate(2)\" fill=\"url(#a)\"/>",
        "<rect fill=\"rgb(1,2,3)\" width=\"(1)\"/>",
        "<rect width=\"-calc(1)\"/>",
        "<rect width=\"xmin(1)\"/>",
        "<rect width=\"calc (1)\"/>",
        "<style>calc{fill:red}</style>",
    ] {
        assert!(!may_hold(source), "{source}");
    }
}
