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
