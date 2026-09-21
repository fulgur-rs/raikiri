use std::sync::Arc;

use super::*;
use crate::Atom;
use cssparser::{ParseError, Parser, ParserInput};
use smol_str::SmolStr;

fn parse(source: &str, name: &str) -> Option<PropertyValue> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_value(name, &mut parser)
}

fn parse_entire(source: &str, name: &str) -> Option<PropertyValue> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|i| -> Result<PropertyValue, ParseError<'_, ()>> {
            parse_value(name, i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok()
}

fn parse_color_entire(source: &str) -> Option<CssColor> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|i| -> Result<CssColor, ParseError<'_, ()>> {
            parse_color(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok()
}

fn parse_parsed_color_entire(source: &str) -> Option<ParsedColor> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|i| -> Result<ParsedColor, ParseError<'_, ()>> {
            parse_color_float(i, 0).ok_or_else(|| i.new_custom_error(()))
        })
        .ok()
}

fn red() -> CssColor {
    CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    }
}

#[test]
fn color_parse_hex() {
    assert_eq!(
        parse("#ff0000", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }))
    );
}

#[test]
fn color_parse_hex_3digit_duplicates_nibbles() {
    // CSS Color 4 §5.2 verbatim: "This syntax is often explained by saying
    // that it’s identical to a 6-digit notation obtained by "duplicating"
    // all of the digits." `#f00` == `#ff0000`.
    assert_eq!(
        parse("#f00", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }))
    );
}

#[test]
fn color_parse_hex_4digit_duplicates_alpha_nibble() {
    // CSS Color 4 §5.2: `#rgba` becomes `#rrggbbaa`。alpha nibble `8`
    // → `0x88` = 136 (8 * 17)。
    assert_eq!(
        parse("#f008", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 136
        }))
    );
}

#[test]
fn color_parse_hex_8digit_alpha_byte() {
    // CSS Color 4 §5.2 8-digit form: 末尾 byte が alpha (0..=255)。
    // `#ff000080` → alpha = 0x80 = 128。
    assert_eq!(
        parse("#ff000080", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 128
        }))
    );
}

#[test]
fn color_parse_hex_case_insensitive() {
    // CSS Color 4 §5.2 verbatim: "the case of the letters doesn’t matter -
    // #00ff00 is identical to #00FF00" — `#FF0000` == `#ff0000`。
    assert_eq!(
        parse("#FF0000", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }))
    );
}

#[test]
fn color_parse_hex_invalid_char_returns_none() {
    // spec-invalid: `g` は hex digit ではない (→ drop)。
    assert_eq!(parse("#gggggg", "color"), None);
}

#[test]
fn color_parse_hex_invalid_length_returns_none() {
    // spec-invalid: hex-notation grammar は 3/4/6/8 digit のみ。
    // 5-digit は spec に無い (→ drop)。
    assert_eq!(parse("#12345", "color"), None);
    // 7-digit も同様に spec-invalid。
    assert_eq!(parse("#1234567", "color"), None);
}

#[test]
fn color_parse_named() {
    assert_eq!(
        parse("red", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }))
    );
}

#[test]
fn color_parse_rgb() {
    assert_eq!(
        parse("rgb(255, 0, 0)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }))
    );
}

#[test]
fn color_parse_color_srgb_and_linear_srgb() {
    assert_eq!(
        parse("color(srgb 1 0 0 / 50%)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 128,
        }))
    );
    assert_eq!(
        parse("color(srgb-linear 1 0 0)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }))
    );
}

#[test]
fn color_mix_retains_out_of_range_authored_srgb_endpoint() {
    let source = "color-mix(in srgb-linear, color(srgb 2 0 0), black)";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::SrgbLinear);
    assert!(parsed.coordinates[0] > 1.0);
    assert_eq!(
        parsed.to_css_color(),
        CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }
    );
}

#[test]
fn color_parse_srgb_linear_retains_out_of_range_components() {
    let source = "color(srgb-linear 2 0 0)";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::SrgbLinear);
    assert_eq!(parsed.coordinates, [2.0, 0.0, 0.0]);
    assert_eq!(
        parsed.to_css_color(),
        CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }
    );
}

#[test]
fn color_mix_nested_srgb_linear_retains_out_of_range_endpoint() {
    let source = "color-mix(in srgb-linear, color(srgb-linear 2 0 0), color-mix(in srgb-linear, color(srgb-linear 2 0 0), black))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::SrgbLinear);
    assert!(parsed.coordinates[0] > 1.0);
    assert_eq!(
        parsed.to_css_color(),
        CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }
    );
}

#[test]
fn color_mix_effective_boundary_does_not_cross_srgb_conversion() {
    for (endpoint, expected) in [
        (
            "lab(0% 125 125)",
            CssColor {
                r: 126,
                g: 0,
                b: 0,
                a: 128,
            },
        ),
        (
            "lab(100% 125 125)",
            CssColor {
                r: 255,
                g: 77,
                b: 0,
                a: 128,
            },
        ),
    ] {
        let source = format!("color-mix(in srgb, lab(50% 50 0 / 0%), {endpoint})");
        assert_eq!(
            parse(&source, "color"),
            Some(PropertyValue::Color(expected))
        );
        let source = format!("color-mix(in srgb, {endpoint}, lab(50% 50 0 / 0%))");
        assert_eq!(
            parse(&source, "color"),
            Some(PropertyValue::Color(expected))
        );
    }
}

#[test]
fn color_mix_effective_boundary_maps_in_native_space() {
    for (endpoint, expected) in [
        (
            "lab(0% 125 125)",
            CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 128,
            },
        ),
        (
            "lab(100% 125 125)",
            CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 128,
            },
        ),
    ] {
        let source = format!("color-mix(in lab, lab(50% 50 0 / 0%), {endpoint})");
        assert_eq!(
            parse(&source, "color"),
            Some(PropertyValue::Color(expected))
        );
        let source = format!("color-mix(in lab, {endpoint}, lab(50% 50 0 / 0%))");
        assert_eq!(
            parse(&source, "color"),
            Some(PropertyValue::Color(expected))
        );
    }
}

#[test]
fn color_mix_exact_native_lab_family_boundaries_map_to_black_or_white() {
    let cases = [
        ("lab", "lab(0% 125 125)", "lab(100% 125 125)", "lab"),
        ("lch", "lch(0% 150 45deg)", "lch(100% 150 45deg)", "lch"),
        ("oklab", "oklab(0% 0.4 0.4)", "oklab(100% 0.4 0.4)", "oklab"),
        (
            "oklch",
            "oklch(0% 0.4 45deg)",
            "oklch(100% 0.4 45deg)",
            "oklch",
        ),
    ];
    for (_, lower, upper, native_space) in cases {
        for (target, endpoint, white) in [(native_space, lower, false), (native_space, upper, true)]
        {
            let source = format!("color-mix(in {target}, {endpoint} 100%, black 0%)");
            let expected = if white {
                CssColor {
                    r: 255,
                    g: 255,
                    b: 255,
                    a: 255,
                }
            } else {
                CssColor::BLACK
            };
            assert_eq!(
                parse(&source, "color"),
                Some(PropertyValue::Color(expected))
            );
            let source = format!("color-mix(in {target}, black 0%, {endpoint} 100%)");
            assert_eq!(
                parse(&source, "color"),
                Some(PropertyValue::Color(expected))
            );
        }
    }
}

#[test]
fn color_mix_exact_lab_boundaries_convert_without_srgb_mapping() {
    let cases = [
        (
            "srgb",
            "lab(0% 125 125)",
            [0.49444056, -0.26553026, -0.33006898],
            CssColor {
                r: 126,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
        (
            "srgb",
            "lab(100% 125 125)",
            [1.875541, 0.30204597, -0.1974194],
            CssColor {
                r: 255,
                g: 77,
                b: 0,
                a: 255,
            },
        ),
        (
            "srgb-linear",
            "lab(0% 125 125)",
            [0.20893149, -0.057316516, -0.089019805],
            CssColor {
                r: 126,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
        (
            "srgb-linear",
            "lab(100% 125 125)",
            [4.264065, 0.074256085, -0.03230641],
            CssColor {
                r: 255,
                g: 77,
                b: 0,
                a: 255,
            },
        ),
    ];
    for (space, endpoint, expected_coordinates, expected_color) in cases {
        let source = format!("color-mix(in {space}, {endpoint} 100%, black 0%)");
        let parsed = parse_parsed_color_entire(&source).expect(&source);
        assert!(parsed.lightness_boundary.is_none());
        assert_coordinates_close(parsed.coordinates, expected_coordinates);
        assert_eq!(parsed.to_css_color(), expected_color);
    }
}

#[test]
fn color_mix_cross_lab_family_boundaries_follow_converted_lightness() {
    let cases = [
        (
            "color-mix(in oklab, lab(0% 125 125) 100%, black 0%)",
            CssColor::BLACK,
        ),
        (
            "color-mix(in oklab, lab(100% 125 125) 100%, black 0%)",
            CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        ),
        (
            "color-mix(in lab, oklab(0% 0.4 0.4) 100%, black 0%)",
            CssColor::BLACK,
        ),
        (
            "color-mix(in lab, oklab(100% 0.4 0.4) 100%, black 0%)",
            CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        ),
        (
            "color-mix(in oklab, lab(0% 125 125), lab(50% 50 0 / 0%))",
            CssColor::BLACK,
        ),
        (
            "color-mix(in lab, oklab(0% 0.4 0.4), oklab(0.5 0.1 0.1 / 0%))",
            CssColor::BLACK,
        ),
    ];
    for (source, incorrectly_mapped) in cases {
        let parsed = parse_parsed_color_entire(source).expect(source);
        assert!(parsed.lightness_boundary.is_none());
        assert_ne!(parsed.to_css_color(), incorrectly_mapped);
    }
}

#[test]
fn color_mix_shared_native_lab_boundaries_keep_provenance() {
    let cases = [
        (
            "lab",
            "lab(0% 125 125)",
            "lab(0% -125 -125)",
            LightnessBoundary::Lower,
            CssColor::BLACK,
        ),
        (
            "lab",
            "lab(100% 125 125)",
            "lab(100% -125 -125)",
            LightnessBoundary::Upper,
            CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        ),
        (
            "lch",
            "lch(0% 150 45deg)",
            "lch(0% 150 225deg)",
            LightnessBoundary::Lower,
            CssColor::BLACK,
        ),
        (
            "lch",
            "lch(100% 150 45deg)",
            "lch(100% 150 225deg)",
            LightnessBoundary::Upper,
            CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        ),
        (
            "oklab",
            "oklab(0% 0.4 0.4)",
            "oklab(0% -0.4 -0.4)",
            LightnessBoundary::Lower,
            CssColor::BLACK,
        ),
        (
            "oklab",
            "oklab(100% 0.4 0.4)",
            "oklab(100% -0.4 -0.4)",
            LightnessBoundary::Upper,
            CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        ),
        (
            "oklch",
            "oklch(0% 0.4 45deg)",
            "oklch(0% 0.4 225deg)",
            LightnessBoundary::Lower,
            CssColor::BLACK,
        ),
        (
            "oklch",
            "oklch(100% 0.4 45deg)",
            "oklch(100% 0.4 225deg)",
            LightnessBoundary::Upper,
            CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        ),
    ];
    for (space, first, second, boundary, expected) in cases {
        let source = format!("color-mix(in {space}, {first}, {second})");
        let parsed = parse_parsed_color_entire(&source).expect(&source);
        assert!(parsed.lightness_boundary == Some(boundary));
        assert_eq!(parsed.to_css_color(), expected, "{source}");
    }
}

#[test]
fn color_mix_shared_upper_boundary_with_different_alpha_keeps_provenance() {
    let cases = [
        (
            "lab",
            "lab(100% 125 125 / 20%)",
            "lab(100% -125 -125 / 40%)",
        ),
        (
            "oklab",
            "oklab(100% 0.4 0.4 / 20%)",
            "oklab(100% -0.4 -0.4 / 40%)",
        ),
    ];
    for (space, first, second) in cases {
        let source = format!("color-mix(in {space}, {first}, {second})");
        let parsed = parse_parsed_color_entire(&source).expect(&source);
        assert!(parsed.lightness_boundary == Some(LightnessBoundary::Upper));
        assert_eq!(
            parsed.to_css_color(),
            CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 77,
            }
        );
    }
}

#[test]
fn color_mix_srgb_does_not_propagate_shared_lab_boundary() {
    let source = "color-mix(in srgb, lab(0% 125 125), lab(0% -125 -125))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Srgb);
    assert_coordinates_close(parsed.coordinates, [-0.03416577, -0.018700615, 0.20680122]);
    assert!(parsed.lightness_boundary.is_none());
    assert_ne!(parsed.to_css_color(), CssColor::BLACK);
    assert!(!lightness_boundary_matches(
        MixColorSpace::Srgb,
        LightnessBoundary::Lower,
        0.0,
    ));
}

#[test]
fn color_mix_one_authored_boundary_maps_at_native_result_boundary() {
    let cases = [
        (
            "lab",
            "lab(0% 125 125)",
            "black",
            LightnessBoundary::Lower,
            CssColor::BLACK,
        ),
        (
            "lab",
            "lab(100% 125 125)",
            "white",
            LightnessBoundary::Upper,
            CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        ),
        (
            "oklab",
            "oklab(0% 0.4 0.4)",
            "black",
            LightnessBoundary::Lower,
            CssColor::BLACK,
        ),
        (
            "oklab",
            "oklab(100% 0.4 0.4)",
            "white",
            LightnessBoundary::Upper,
            CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        ),
    ];
    for (space, first, second, boundary, expected) in cases {
        let source = format!("color-mix(in {space}, {first}, {second})");
        let parsed = parse_parsed_color_entire(&source).expect(&source);
        assert!(parsed.lightness_boundary == Some(boundary));
        assert_eq!(parsed.to_css_color(), expected);
    }
}

#[test]
fn color_mix_one_authored_boundary_maps_when_second_endpoint_is_boundary() {
    let source = "color-mix(in lab, black, lab(0% 125 125))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert!(parsed.lightness_boundary == Some(LightnessBoundary::Lower));
    assert_eq!(parsed.to_css_color(), CssColor::BLACK);
}

#[test]
fn color_mix_nested_generated_neutral_boundaries_keep_provenance() {
    let white = CssColor {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    let cases = [
        (
            "lab",
            "color-mix(in lab, white, white)",
            "lab(100% 125 125)",
            LightnessBoundary::Upper,
            white,
        ),
        (
            "lab",
            "color-mix(in lab, black, black)",
            "lab(0% 125 125)",
            LightnessBoundary::Lower,
            CssColor::BLACK,
        ),
        (
            "lch",
            "color-mix(in lch, white, white)",
            "lch(100% 150 225deg)",
            LightnessBoundary::Upper,
            white,
        ),
        (
            "lch",
            "color-mix(in lch, black, black)",
            "lch(0% 150 225deg)",
            LightnessBoundary::Lower,
            CssColor::BLACK,
        ),
        (
            "oklab",
            "color-mix(in oklab, white, white)",
            "oklab(100% 0.4 0.4)",
            LightnessBoundary::Upper,
            white,
        ),
        (
            "oklab",
            "color-mix(in oklab, black, black)",
            "oklab(0% 0.4 0.4)",
            LightnessBoundary::Lower,
            CssColor::BLACK,
        ),
        (
            "oklch",
            "color-mix(in oklch, white, white)",
            "oklch(100% 0.4 225deg)",
            LightnessBoundary::Upper,
            white,
        ),
        (
            "oklch",
            "color-mix(in oklch, black, black)",
            "oklch(0% 0.4 225deg)",
            LightnessBoundary::Lower,
            CssColor::BLACK,
        ),
    ];
    for (space, generated, authored, boundary, expected) in cases {
        let source = format!("color-mix(in {space}, {generated}, {authored})");
        let parsed = parse_parsed_color_entire(&source).expect(&source);
        assert!(parsed.lightness_boundary == Some(boundary), "{source}");
        assert_eq!(parsed.to_css_color(), expected, "{source}");
    }
}

#[test]
fn color_mix_nested_cross_lab_family_neutral_does_not_map() {
    for (space, generated, authored) in [
        (
            "oklab",
            "color-mix(in lab, white, white)",
            "oklab(100% 0.4 0.4)",
        ),
        (
            "lab",
            "color-mix(in oklab, white, white)",
            "lab(100% 125 125)",
        ),
    ] {
        let source = format!("color-mix(in {space}, {generated}, {authored})");
        let parsed = parse_parsed_color_entire(&source).expect(&source);
        assert!(parsed.lightness_boundary.is_none(), "{source}");
    }
}

#[test]
fn color_mix_near_boundary_does_not_inherit_provenance() {
    let source = "color-mix(in lab, lab(0% 125 125), lab(0.0000004 0 0))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert!(parsed.lightness_boundary.is_none());
    assert!(parsed.coordinates[0] > 0.0);
}

#[test]
fn color_mix_nonzero_lab_boundary_keeps_raw_coordinates() {
    let source = "color-mix(in lab, lab(0% 125 125), white)";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Lab);
    assert_coordinates_close(parsed.coordinates, [50.0, 62.5, 62.5]);
    assert!(parsed.lightness_boundary.is_none());
}

#[test]
fn color_mix_exact_lab_boundary_nested_result_retains_raw_coordinates() {
    let source = "color-mix(in srgb, color-mix(in lab, lab(0% 125 125) 100%, black 0%), white)";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Srgb);
    assert_coordinates_close(parsed.coordinates, [0.7472203, 0.3672349, 0.33496553]);
    assert_eq!(
        parsed.to_css_color(),
        CssColor {
            r: 191,
            g: 94,
            b: 85,
            a: 255,
        }
    );
}

#[test]
fn color_mix_nested_identical_lab_boundaries_are_raw_after_srgb_conversion() {
    for (endpoint, expected) in [
        (
            "lab(0% 125 125)",
            CssColor {
                r: 126,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
        (
            "lab(100% 125 125)",
            CssColor {
                r: 255,
                g: 77,
                b: 0,
                a: 255,
            },
        ),
    ] {
        let source =
            format!("color-mix(in srgb, color-mix(in lab, {endpoint}, {endpoint}), white 0%)");
        assert_eq!(
            parse(&source, "color"),
            Some(PropertyValue::Color(expected))
        );
    }
}

#[test]
fn color_parse_lab_family_extremes() {
    for source in [
        "lab(0% 0 0)",
        "lch(0% 0 0)",
        "oklab(0% 0 0)",
        "oklch(0% 0 0)",
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parse(source, "color"),
            Some(PropertyValue::Color(CssColor::BLACK)),
            "{source}"
        );
    }
    for source in [
        "lab(100% 0 0)",
        "lch(100% 0 0)",
        "oklab(100% 0 0)",
        "oklch(100% 0 0)",
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parse(source, "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            })),
            "{source}"
        );
    }
}

#[test]
fn color_parse_lab_family_converts_neutral_lightness() {
    assert_eq!(
        parse("lab(50% 0 0)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 119,
            g: 119,
            b: 119,
            a: 255,
        }))
    );
    assert_eq!(
        parse("lch(50% 0 120deg)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 119,
            g: 119,
            b: 119,
            a: 255,
        }))
    );
    assert_eq!(
        parse("oklab(50% 0 0)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 99,
            g: 99,
            b: 99,
            a: 255,
        }))
    );
    assert_eq!(
        parse("oklch(50% 0 120deg)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 99,
            g: 99,
            b: 99,
            a: 255,
        }))
    );
}

#[test]
fn color_parse_lab_family_clamps_lightness_and_chroma() {
    assert_eq!(
        parse("lab(120% 0 0)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }))
    );
    assert_eq!(
        parse("oklab(-1 0 0)", "color"),
        Some(PropertyValue::Color(CssColor::BLACK))
    );
    assert_eq!(
        parse("lch(50% -10 120deg)", "color"),
        parse("lab(50% 0 0)", "color")
    );
    assert_eq!(
        parse("oklch(50% -10 120deg)", "color"),
        parse("oklab(50% 0 0)", "color")
    );
}

#[test]
fn color_parse_lab_family_rejects_none_components() {
    for source in [
        "lab(50% none none)",
        "lch(50% none none)",
        "oklab(50% none none)",
        "oklch(50% none none)",
    ] {
        assert!(parse(source, "color").is_some(), "{source}");
    }
}

#[test]
fn color_parse_lab_family_accepts_percentage_components() {
    assert_eq!(
        parse("lab(50% 10% -10%)", "color"),
        parse("lab(50% 12.5 -12.5)", "color")
    );
    assert_eq!(
        parse("lch(50% 20% 30deg)", "color"),
        parse("lch(50% 30 30deg)", "color")
    );
    assert_eq!(
        parse("oklab(50% 20% -20%)", "color"),
        parse("oklab(50% 0.08 -0.08)", "color")
    );
    assert_eq!(
        parse("oklch(50% 20% 30deg)", "color"),
        parse("oklch(50% 0.08 30deg)", "color")
    );
}

fn nested_color_mix_in_first_endpoint(depth: usize) -> String {
    let mut source = String::new();
    for _ in 0..depth {
        source.push_str("color-mix(in srgb, ");
    }
    source.push_str("red");
    for _ in 0..depth {
        source.push_str(", blue)");
    }
    source
}

fn nested_color_mix_in_second_endpoint(depth: usize) -> String {
    let mut source = String::new();
    for _ in 0..depth {
        source.push_str("color-mix(in srgb, red, ");
    }
    source.push_str("blue");
    for _ in 0..depth {
        source.push(')');
    }
    source
}

fn nested_color_mix_in_both_endpoints(depth: usize) -> String {
    format!(
        "color-mix(in srgb, {}, {})",
        nested_color_mix_in_first_endpoint(depth),
        nested_color_mix_in_second_endpoint(depth)
    )
}

#[test]
fn color_parse_color_mix_accepts_boundary_depth_on_both_endpoints() {
    let boundary = MAX_COLOR_MIX_NESTING_DEPTH;
    for source in [
        nested_color_mix_in_first_endpoint(boundary),
        nested_color_mix_in_second_endpoint(boundary),
        nested_color_mix_in_both_endpoints(boundary.saturating_sub(1)),
    ] {
        assert!(parse_color_entire(&source).is_some());
    }
}

#[test]
fn color_parse_color_mix_rejects_first_depth_beyond_boundary_on_both_endpoints() {
    let too_deep = MAX_COLOR_MIX_NESTING_DEPTH + 1;
    for source in [
        nested_color_mix_in_first_endpoint(too_deep),
        nested_color_mix_in_second_endpoint(too_deep),
        nested_color_mix_in_both_endpoints(MAX_COLOR_MIX_NESTING_DEPTH),
    ] {
        assert_eq!(parse_color_entire(&source), None);
    }
}

#[test]
fn color_parse_color_mix_rejects_adversarially_deep_nesting() {
    let deep = MAX_COLOR_MIX_NESTING_DEPTH.saturating_mul(16);
    assert_eq!(
        parse_color_entire(&nested_color_mix_in_first_endpoint(deep)),
        None
    );
    assert_eq!(
        parse_color_entire(&nested_color_mix_in_second_endpoint(deep)),
        None
    );
}

#[test]
fn color_parse_color_mix_in_srgb_normalizes_percentages() {
    assert_eq!(
        parse("color-mix(in srgb, red 25%, blue 75%)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 64,
            g: 0,
            b: 191,
            a: 255,
        }))
    );
    assert_eq!(
        parse("color-mix(in srgb, red 80%, blue 80%)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 128,
            g: 0,
            b: 128,
            a: 255,
        }))
    );
}

#[test]
fn color_parse_color_mix_preserves_partial_percentage_alpha() {
    assert_eq!(
        parse_color_entire("color-mix(in srgb, red 20%, blue 20%)"),
        Some(CssColor {
            r: 128,
            g: 0,
            b: 128,
            a: 102,
        })
    );
}

#[test]
fn color_parse_color_mix_premultiplies_alpha() {
    assert_eq!(
        parse(
            "color-mix(in srgb, rgb(255, 0, 0, 0.5) 50%, blue 50%)",
            "color"
        ),
        Some(PropertyValue::Color(CssColor {
            r: 85,
            g: 0,
            b: 170,
            a: 191,
        }))
    );
}

#[test]
fn color_parse_color_mix_omitted_percentage_gets_leftover() {
    assert_eq!(
        parse("color-mix(in srgb, red 25%, blue)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 64,
            g: 0,
            b: 191,
            a: 255,
        }))
    );
}

#[test]
fn color_parse_color_mix_second_percentage_gets_leftover_for_first() {
    assert_eq!(
        parse("color-mix(in srgb, red, blue 25%)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 191,
            g: 0,
            b: 64,
            a: 255,
        }))
    );
}

#[test]
fn color_parse_color_mix_accepts_percentage_before_color() {
    let cases = [
        (
            "color-mix(in srgb, 25% red, blue)",
            "color-mix(in srgb, red 25%, blue)",
            CssColor {
                r: 64,
                g: 0,
                b: 191,
                a: 255,
            },
        ),
        (
            "color-mix(in srgb, red, 25% blue)",
            "color-mix(in srgb, red, blue 25%)",
            CssColor {
                r: 191,
                g: 0,
                b: 64,
                a: 255,
            },
        ),
        (
            "color-mix(in srgb, 25% red, 75% blue)",
            "color-mix(in srgb, red 25%, blue 75%)",
            CssColor {
                r: 64,
                g: 0,
                b: 191,
                a: 255,
            },
        ),
        (
            "color-mix(in srgb, 25% red, blue 75%)",
            "color-mix(in srgb, red 25%, blue 75%)",
            CssColor {
                r: 64,
                g: 0,
                b: 191,
                a: 255,
            },
        ),
        (
            "color-mix(in srgb, red 25%, 75% blue)",
            "color-mix(in srgb, red 25%, blue 75%)",
            CssColor {
                r: 64,
                g: 0,
                b: 191,
                a: 255,
            },
        ),
    ];
    for (percentage_before, percentage_after, expected) in cases {
        assert_eq!(parse_color_entire(percentage_before), Some(expected));
        assert_eq!(
            parse_color_entire(percentage_before),
            parse_color_entire(percentage_after)
        );
    }
}

#[test]
fn color_parse_color_mix_rejects_multiple_stop_percentages() {
    for source in [
        "color-mix(in srgb, 25% red 25%, blue)",
        "color-mix(in srgb, red 25% 25%, blue)",
        "color-mix(in srgb, 101% red, blue)",
        "color-mix(in srgb, red, 101% blue)",
    ] {
        assert_eq!(parse_color_entire(source), None);
    }
}

#[test]
fn color_parse_color_mix_rejects_percentage_above_100() {
    assert_eq!(parse("color-mix(in srgb, red 101%, blue)", "color"), None);
    assert_eq!(parse("color-mix(in srgb, red, blue 101%)", "color"), None);
}

#[test]
fn color_parse_color_mix_supports_linear_srgb() {
    assert_eq!(
        parse("color-mix(in srgb-linear, black, white)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 188,
            g: 188,
            b: 188,
            a: 255,
        }))
    );
}

#[test]
fn color_parse_color_mix_supports_lab_family_spaces() {
    for space in ["lab", "lch"] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parse(&format!("color-mix(in {space}, black, white)"), "color"),
            Some(PropertyValue::Color(CssColor {
                r: 119,
                g: 119,
                b: 119,
                a: 255,
            })),
            "{space}"
        );
    }
    for space in ["oklab", "oklch"] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parse(&format!("color-mix(in {space}, black, white)"), "color"),
            Some(PropertyValue::Color(CssColor {
                r: 99,
                g: 99,
                b: 99,
                a: 255,
            })),
            "{space}"
        );
    }
}

#[test]
fn color_parse_hue_rejects_none_and_accepts_css_angle_units() {
    assert_eq!(
        parse("lch(50% 20 none)", "color"),
        parse("lch(50% 20 0deg)", "color")
    );
    assert_eq!(
        parse("lch(50% 20 100grad)", "color"),
        parse("lch(50% 20 90deg)", "color")
    );
    assert_eq!(
        parse("lch(50% 20 1.5707964rad)", "color"),
        parse("lch(50% 20 90deg)", "color")
    );
    assert_eq!(
        parse("lch(50% 20 0.25turn)", "color"),
        parse("lch(50% 20 90deg)", "color")
    );
}

#[test]
fn color_parse_huge_powerless_hue_reduces_before_conversion() {
    let cases = [
        ("1e38", 1e38_f32.rem_euclid(360.0).to_radians()),
        ("1e38deg", 1e38_f32.rem_euclid(360.0).to_radians()),
        ("1e38grad", (1e38_f32.rem_euclid(400.0) * 0.9).to_radians()),
        ("1e38rad", 1e38_f32.rem_euclid(std::f32::consts::TAU)),
        ("1e38turn", (1e38_f32.rem_euclid(1.0) * 360.0).to_radians()),
    ];
    for (angle, expected_hue) in cases {
        let source = format!("lch(50% 0 {angle})");
        let parsed = parse_parsed_color_entire(&source).expect(&source);
        assert!(parsed.coordinates[2].is_finite(), "{source}");
        assert!((parsed.coordinates[2] - expected_hue).abs() < 0.000001);
    }

    let source = "color-mix(in lch, lch(50% 0 1e38turn), lch(50% 0 120deg))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert!(parsed.coordinates[2].is_finite());
    assert!((parsed.coordinates[2] - 60.0_f32.to_radians()).abs() < 0.000001);
    assert_eq!(parse("lch(50% 0 1e39)", "color"), None);
    assert_eq!(parse("lch(50% 0 1e39turn)", "color"), None);
}

#[test]
fn color_parse_hue_rejects_unknown_units_and_tokens() {
    assert_eq!(parse("lch(50% 20 1foo)", "color"), None);
    assert_eq!(parse("lch(50% 20 \"not-a-hue\")", "color"), None);
}

#[test]
fn color_parse_color_modern_alpha_rejects_none_and_accepts_number() {
    assert_eq!(
        parse("color(srgb 1 0 0 / none)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 0,
        }))
    );
    assert_eq!(
        parse("color(srgb 1 0 0 / 0.25)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 64,
        }))
    );
}

#[test]
fn color_parse_color_mix_zero_percentages_is_invalid() {
    assert_eq!(parse("color-mix(in srgb, red 0%, blue 0%)", "color"), None);
}

#[test]
fn color_parse_color_mix_nonzero_weights_with_zero_alpha_is_transparent() {
    assert_eq!(
        parse("color-mix(in srgb, transparent, transparent)", "color"),
        Some(PropertyValue::Color(CssColor::TRANSPARENT))
    );
    assert_eq!(
        parse("color-mix(in lch, transparent, transparent)", "color"),
        Some(PropertyValue::Color(CssColor::TRANSPARENT))
    );
}

#[test]
fn color_parse_color_mix_lch_handles_one_achromatic_color() {
    assert!(matches!(
        parse("color-mix(in lch, black, red)", "color"),
        Some(PropertyValue::Color(_))
    ));
    assert!(matches!(
        parse("color-mix(in lch, red, black)", "color"),
        Some(PropertyValue::Color(_))
    ));
}

#[test]
fn color_parse_color_mix_lch_interpolates_two_hues() {
    assert!(matches!(
        parse("color-mix(in lch, red, blue)", "color"),
        Some(PropertyValue::Color(_))
    ));
}

#[test]
fn color_parse_color_mix_preserves_float_color_endpoints() {
    assert_eq!(
        parse(
            "color-mix(in srgb, color(srgb 0.002 0 0) 50%, black 50%)",
            "color"
        ),
        Some(PropertyValue::Color(CssColor::BLACK))
    );
}

#[test]
fn color_mix_lch_keeps_hue_from_transparent_chromatic_endpoint() {
    let source = "color-mix(in lch, lch(50% 150 0deg / 0%), lch(50% 150 120deg))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Lch);
    assert_coordinates_close(parsed.coordinates, [50.0, 150.0, 60.0_f32.to_radians()]);
}

#[test]
fn color_mix_nested_zero_weight_authored_hue_carries_into_followup_mix() {
    let source = "color-mix(in lch, color-mix(in lch, lch(50% 150 120deg) 0%, rgb(128, 128, 128) 100%) 50%, rgb(128, 128, 128) 50%)";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Lch);
    assert!((parsed.coordinates[2] - 120.0_f32.to_radians()).abs() < 0.0001);
    assert!(!parsed.polar_hue_missing);
}

#[test]
fn color_mix_zero_weight_keeps_hue_selection_for_equal_missingness() {
    for polar_hue_missing in [false, true] {
        for (weight_one, weight_two, expected_hue) in [(0.0, 1.0, 2.0), (1.0, 0.0, 1.0)] {
            let mixed = mix_coordinates(
                ColorCoordinates {
                    first: 50.0,
                    second: 40.0,
                    third: 1.0,
                    alpha: 1.0,
                    polar_hue_missing,
                },
                ColorCoordinates {
                    first: 50.0,
                    second: 40.0,
                    third: 2.0,
                    alpha: 1.0,
                    polar_hue_missing,
                },
                weight_one,
                weight_two,
                MixColorSpace::Lch,
            );
            assert_eq!(mixed.third, expected_hue);
            assert_eq!(mixed.polar_hue_missing, polar_hue_missing);
        }
    }
}

#[test]
fn color_mix_lch_hue_uses_specified_weight_with_different_alpha() {
    let mixed = mix_coordinates(
        ColorCoordinates {
            first: 50.0,
            second: 40.0,
            third: 0.0,
            alpha: 1.0,
            polar_hue_missing: false,
        },
        ColorCoordinates {
            first: 50.0,
            second: 40.0,
            third: std::f32::consts::FRAC_PI_2,
            alpha: 0.25,
            polar_hue_missing: false,
        },
        0.25,
        0.75,
        MixColorSpace::Lch,
    );
    assert!((mixed.third - std::f32::consts::FRAC_PI_2 * 0.75).abs() < 0.000001);
}

#[test]
fn color_parse_color_mix_lch_handles_different_alpha_endpoints() {
    assert_eq!(
        parse(
            "color-mix(in lch, red 25%, rgb(0, 0, 255, 0.25) 75%)",
            "color"
        ),
        Some(PropertyValue::Color(CssColor {
            r: 206,
            g: 0,
            b: 215,
            a: 112,
        }))
    );
}

#[test]
fn color_mix_authored_powerless_hue_is_not_missing() {
    let source = "color-mix(in lch, lch(50% 0 120deg), lch(50% 0 240deg))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Lch);
    assert_coordinates_close(parsed.coordinates, [50.0, 0.0, 180.0_f32.to_radians()]);
    assert!(!parsed.polar_hue_missing);
}

#[test]
fn color_mix_authored_tiny_powerless_chroma_is_preserved() {
    let source = "color-mix(in lch, lch(50% 0.001 120deg), lch(50% 0.001 240deg))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Lch);
    assert_coordinates_close(parsed.coordinates, [50.0, 0.001, 180.0_f32.to_radians()]);
    assert!(!parsed.polar_hue_missing);
}

#[test]
fn color_mix_converted_powerless_hue_carries_authored_hue() {
    let source = "color-mix(in lch, rgb(128, 128, 128), lch(50% 150 120deg))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Lch);
    assert!((parsed.coordinates[2] - 120.0_f32.to_radians()).abs() < 0.0001);
    assert!(!parsed.polar_hue_missing);
}

#[test]
fn color_mix_uses_space_specific_powerless_hue_thresholds() {
    let near_neutral =
        ParsedColor::from_coordinates(ParsedColorSpace::Srgb, [0.5, 0.5, 0.50001], 1.0);
    assert_eq!(
        css_color_to_coordinates(near_neutral, MixColorSpace::Lch).third,
        0.0
    );
    let near_neutral_lch = css_color_to_coordinates(near_neutral, MixColorSpace::Lch);
    assert_eq!(near_neutral_lch.second, 0.0);
    assert_eq!(near_neutral_lch.third, 0.0);
    assert!(near_neutral_lch.polar_hue_missing);
    let near_neutral_oklch = css_color_to_coordinates(near_neutral, MixColorSpace::Oklch);
    assert_eq!(near_neutral_oklch.second, 0.0);
    assert_eq!(near_neutral_oklch.third, 0.0);
    assert!(near_neutral_oklch.polar_hue_missing);

    let converted_mix =
        parse_parsed_color_entire("color-mix(in lch, rgb(128, 128, 128), rgb(128, 128, 128))")
            .expect("converted mix");
    let normalized_converted_mix = css_color_to_coordinates(converted_mix, MixColorSpace::Lch);
    assert!(normalized_converted_mix.polar_hue_missing);
    assert_eq!(normalized_converted_mix.third, 0.0);
}

#[test]
fn color_mix_lch_shorter_hue_preserves_positive_half_turn() {
    let mixed = mix_coordinates(
        ColorCoordinates {
            first: 50.0,
            second: 40.0,
            third: 0.0,
            alpha: 1.0,
            polar_hue_missing: false,
        },
        ColorCoordinates {
            first: 50.0,
            second: 40.0,
            third: std::f32::consts::PI,
            alpha: 1.0,
            polar_hue_missing: false,
        },
        0.25,
        0.75,
        MixColorSpace::Lch,
    );
    assert!((mixed.third - std::f32::consts::PI * 0.75).abs() < 0.000001);
}

#[test]
fn color_mix_lch_shorter_hue_wraps_negative_delta() {
    let mixed = mix_coordinates(
        ColorCoordinates {
            first: 50.0,
            second: 40.0,
            third: std::f32::consts::PI * 1.5,
            alpha: 1.0,
            polar_hue_missing: false,
        },
        ColorCoordinates {
            first: 50.0,
            second: 40.0,
            third: 0.0,
            alpha: 1.0,
            polar_hue_missing: false,
        },
        0.5,
        0.5,
        MixColorSpace::Lch,
    );
    assert!((mixed.third - std::f32::consts::PI * 1.75).abs() < 0.000001);
}

#[test]
fn color_mix_lch_shorter_hue_wrap_preserves_zero_representative() {
    let source = "color-mix(in lch shorter hue, lch(50% 40 10deg), lch(50% 40 350deg))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Lch);
    assert!(parsed.coordinates[2].abs() < 0.0001);
}

#[test]
fn color_mix_nested_shorter_hue_wrap_preserves_rectangular_conversion() {
    let source = "color-mix(in lab, color-mix(in lch shorter hue, lch(50% 40 10deg), lch(50% 40 350deg)), lab(50% 0 0))";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Lab);
    assert!((parsed.coordinates[2] + 4.172325e-6).abs() < 0.0000001);
}

#[test]
fn color_parse_color_mix_accepts_all_polar_hue_interpolation_methods() {
    let methods = [
        ("shorter", 0.0),
        ("longer", 180.0),
        ("increasing", 180.0),
        ("decreasing", 360.0),
    ];
    for space in ["lch", "oklch"] {
        for (method, expected_hue) in methods {
            let source = format!(
                "color-mix(in {space} {method} hue, {space}(50% 40 10deg), {space}(50% 40 350deg))"
            );
            assert_parsed_hue_close(&source, expected_hue);
        }
        let omitted =
            format!("color-mix(in {space}, {space}(50% 40 10deg), {space}(50% 40 350deg))");
        let explicit_shorter = format!(
            "color-mix(in {space} shorter hue, {space}(50% 40 10deg), {space}(50% 40 350deg))"
        );
        assert_parsed_hue_close(&omitted, 0.0);
        assert_parsed_hue_close(&explicit_shorter, 0.0);
    }
    assert_parsed_hue_close(
        "color-mix(in lch longer hue, lch(50% 40 10deg), lch(50% 40 100deg))",
        235.0,
    );
    assert_parsed_hue_close(
        "color-mix(in lch longer hue, lch(50% 40 100deg), lch(50% 40 10deg))",
        235.0,
    );
}

#[test]
fn color_parse_color_mix_hue_methods_preserve_half_turn_direction() {
    let methods = ["shorter", "longer", "increasing", "decreasing"];
    let expected_forward = [90.0, 90.0, 90.0, 270.0];
    let expected_reverse = [90.0, 90.0, 270.0, 90.0];
    for space in ["lch", "oklch"] {
        for (first, second, expected) in [
            ("0deg", "180deg", expected_forward),
            ("180deg", "0deg", expected_reverse),
        ] {
            for (method, expected_hue) in methods.iter().zip(expected) {
                let source = format!(
                    "color-mix(in {space} {method} hue, {space}(50% 40 {first}), {space}(50% 40 {second}))"
                );
                assert_parsed_hue_close(&source, expected_hue);
            }
        }
    }
}

#[test]
fn color_parse_color_mix_rejects_polar_hue_methods_for_rectangular_spaces() {
    for space in ["srgb", "srgb-linear", "lab", "oklab"] {
        for method in ["shorter", "longer", "increasing", "decreasing"] {
            let source = format!("color-mix(in {space} {method} hue, red, blue)");
            assert_eq!(parse_color_entire(&source), None, "{source}");
        }
    }
}

#[test]
fn color_parse_color_mix_accepts_one_or_more_stops() {
    assert!(parse_color_entire("color-mix(in srgb, red)").is_some());
    assert!(parse_color_entire("color-mix(in srgb, red, blue, white)").is_some());
    assert!(parse_color_entire("color-mix(in srgb, red, blue)").is_some());
    assert_eq!(parse_color_entire("color-mix(in srgb, red, )"), None);
}

#[test]
fn color_parse_color_mix_rejects_malformed_hue_interpolation_methods() {
    for source in [
        "color-mix(in lch hue, red, blue)",
        "color-mix(in lch longer, red, blue)",
        "color-mix(in lch sideways hue, red, blue)",
        "color-mix(in lch longer hue increasing hue, red, blue)",
    ] {
        assert_eq!(parse_color_entire(source), None, "{source}");
    }
}

#[test]
fn color_parse_lab_family_maps_lightness_boundaries() {
    for source in ["lab(0% 125 125)", "lch(0% 150 45deg)"] {
        assert_eq!(
            parse(source, "color"),
            Some(PropertyValue::Color(CssColor::BLACK))
        );
    }
    for source in ["lab(100% -125 -125)", "lch(100% 150 45deg)"] {
        assert_eq!(
            parse(source, "color"),
            Some(PropertyValue::Color(CssColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            }))
        );
    }
    assert_eq!(
        parse("oklab(0 0.4 0.4)", "color"),
        Some(PropertyValue::Color(CssColor::BLACK))
    );
    assert_eq!(
        parse("oklab(1 -0.4 -0.4)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }))
    );
    assert_eq!(
        parse("oklch(0 0.4 45deg)", "color"),
        Some(PropertyValue::Color(CssColor::BLACK))
    );
    assert_eq!(
        parse("oklch(1 0.4 45deg)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }))
    );
}

#[test]
fn color_parse_relative_color_syntax() {
    for source in [
        "color(from red srgb 1 0 0)",
        "lab(from red 50% 0 0)",
        "lch(from red 50% 0 0)",
        "oklab(from red 0.5 0 0)",
        "oklch(from red 50% 0 0)",
    ] {
        assert!(parse(source, "color").is_some(), "{source}");
    }
}

#[test]
fn color_parse_relative_math_is_type_checked_before_deferral() {
    for source in [
        "rgb(from red calc(r + 1) g b)",
        "rgb(from var(--color) calc(r + 1) g b)",
        "rgb(from red min(r, 10) g b)",
        "color-mix(in srgb, red calc(10%), blue)",
    ] {
        assert!(
            matches!(parse(source, "color"), Some(PropertyValue::Deferred(_))),
            "{source}"
        );
    }
    for source in [
        "rgb(calc(1px) 0 0)",
        "rgb(0 0 0 / calc(1px))",
        "rgb(from red calc(r + 1%) g b)",
        "rgb(from red calc(r,g) g b)",
        "rgb(from var(--color) red g b)",
        "rgb(from var(--color) r 1deg b)",
    ] {
        assert_eq!(parse(source, "color"), None, "{source}");
    }
}

#[test]
fn color_parse_color_functions_reject_invalid_syntax() {
    assert_eq!(parse("color(xyz-unknown 1 0 0)", "color"), None);
    assert_eq!(parse("lab(50%, 0, 0)", "color"), None);
    assert_eq!(parse("color-mix(in unsupported, red, blue)", "color"), None);
}

// ── rgb() / rgba() function form ──
//
// CSS Color 4 §5.1 legacy comma syntax の追加 form covers。
// 1 sample あたり CssColor 値まで check (loose `Some(_)` は mix reject 系
// regression が silent pass するため避ける、既存 background_color assert
// pattern に揃える)。

#[test]
fn color_parse_rgb_percentage_form() {
    // §5.1: `<percentage>` 0%/100% は `<number>` 0/255 と等価。
    // 100% → 1.0 unit_value → final `rgb_f32_to_css_color` で round(255) = 255。
    assert_eq!(
        parse("rgb(100%, 0%, 0%)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }))
    );
}

#[test]
fn color_parse_rgba_number_alpha() {
    // §5.1 alpha-value = <number> 0..=1。0.5 → final u8 conversion の
    // round(127.5) = 128。
    assert_eq!(
        parse("rgba(255, 0, 0, 0.5)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 128,
        }))
    );
}

#[test]
fn color_parse_rgba_percentage_alpha() {
    // §5.1 alpha-value = <percentage> 0%..=100% は <number> 0..=1 と
    // 同じ mapping (50% → unit_value 0.5 → 128)。
    assert_eq!(
        parse("rgba(255, 0, 0, 50%)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 128,
        }))
    );
}

#[test]
fn color_parse_rgb_clamps_overflow() {
    // §5.1: "Values outside these ranges are not invalid, but are
    // clamped to the ranges defined here at parsed-value time"。300 → 255。
    assert_eq!(
        parse("rgb(300, 0, 0)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }))
    );
}

#[test]
fn color_parse_rgb_clamps_negative() {
    // §5.1 同上、負値も spec-valid で clamp のみ。-10 → 0。
    assert_eq!(
        parse("rgb(-10, 0, 0)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        }))
    );
}

#[test]
fn color_parse_rgb_mix_number_percentage_returns_none() {
    // §5.1: legacy form は "all-number or all-percentage"、mix は禁止。
    // 1 番目 = <number> 255 → is_pct=false 固定、2 番目 `50%` は
    // expect_integer が Percentage token を reject → Err → None。
    assert_eq!(parse("rgb(255, 50%, 0)", "color"), None);
}

#[test]
fn color_parse_rgb_mix_percentage_number_returns_none() {
    // 逆方向 mix (percentage → number): 1 番目 = <pct> 50% → is_pct=true
    // 固定、2 番目 `255` は expect_percentage が Number token を reject。
    assert_eq!(parse("rgb(50%, 255, 0)", "color"), None);
}

#[test]
fn color_parse_rgb_modern_syntax_returns_none() {
    // §5.1 modern (space + slash) syntax `rgb(R G B / A)` は本 task
    // で対応。legacy comma からの移行を check.
    assert_eq!(
        parse("rgb(255 0 0)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }))
    );
    assert_eq!(
        parse("rgb(255 0 0 / 0.5)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 128
        }))
    );
}

#[test]
fn color_parse_rgb_too_few_args_returns_none() {
    // §5.1 legacy grammar は 3 channel 必須。2 個 (`rgb(255, 0)`) は
    // 3 番目 channel 手前で `)` (block 終端) に達し、expect_integer が
    // Err → None。
    assert_eq!(parse("rgb(255, 0)", "color"), None);
}

#[test]
fn color_parse_rgb_too_many_args_returns_none() {
    // §5.1 legacy grammar は最大 4 slot (3 channel + optional alpha)。
    // 5 個目は `parse_nested_block` 内部の `parse_entirely` (cssparser
    // 0.37 parser.rs:1149) が exhaustion check で Err → None。
    assert_eq!(parse("rgb(255, 0, 0, 0.5, 99)", "color"), None);
}

#[test]
fn color_parse_rgb_name_accepts_alpha() {
    // §5.1 alias 規定 cross-cover: rgb() name でも alpha を受理。
    // parse_rgb_function は function name に依存せず、4 番目 comma の有無
    // だけで alpha slot を判定するため、`rgb(R, G, B, A)` は valid。
    // name-based branching が retro で入った場合の regression guard。
    assert_eq!(
        parse("rgb(255, 0, 0, 0.5)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 128,
        }))
    );
}

#[test]
fn color_parse_rgba_name_accepts_no_alpha() {
    // §5.1 alias 規定 cross-cover: rgba() name でも alpha を省略できる
    // (opaque と等価)。`rgba(R, G, B)` は spec grammar 上 valid で、
    // parse_rgb_function は name に依存せず 4 番目 comma 無し → a=255。
    assert_eq!(
        parse("rgba(255, 0, 0)", "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }))
    );
}

#[test]
fn color_parse_invalid_returns_none() {
    assert_eq!(parse("bogus", "color"), None);
    assert_eq!(parse("", "color"), None);
}

#[test]
fn color_parse_transparent_keyword_returns_zero_alpha() {
    // CSS Color 4 §6.3 "The transparent keyword": `transparent`
    // = rgba(0, 0, 0, 0)。Ident arm hardcodes a=255、明示 branch が無ければ
    // transparent が到達しても opaque black (`{0,0,0,255}`) になる bug の
    // regression check。
    assert_eq!(
        parse("transparent", "color"),
        Some(PropertyValue::Color(CssColor::TRANSPARENT))
    );
}

// ── CssColor::from_hex direct helper contract ──
//
// parse_color 経由の integration test は上で網羅済み。以下は helper 自体の
// API contract を check する direct call test — rgb() function form
// や future property (border-*-color 等) が同じ primitive を消費するため、
// 内部形状の regression を早く捕まえる目的。

#[test]
fn css_color_from_hex_6digit_returns_channels() {
    // 6-digit form: `rrggbb` は各 2 桁を byte として解釈、alpha = 255。
    assert_eq!(
        CssColor::from_hex("336699"),
        Some(CssColor {
            r: 0x33,
            g: 0x66,
            b: 0x99,
            a: 255,
        })
    );
}

#[test]
fn css_color_from_hex_3digit_expands_by_duplication() {
    // 3-digit form: 各 nibble を duplicate。`#369` == `#336699`。
    assert_eq!(CssColor::from_hex("369"), CssColor::from_hex("336699"));
}

#[test]
fn css_color_from_hex_4digit_expands_alpha_nibble() {
    // 4-digit form: `#369c` == `#336699cc`。alpha nibble `c` (12) →
    // `0xcc` = 204。
    assert_eq!(CssColor::from_hex("369c"), CssColor::from_hex("336699cc"));
}

#[test]
fn css_color_from_hex_8digit_carries_alpha_byte() {
    // 8-digit form: 末尾 byte がそのまま alpha (0..=255)。
    assert_eq!(
        CssColor::from_hex("336699cc"),
        Some(CssColor {
            r: 0x33,
            g: 0x66,
            b: 0x99,
            a: 0xcc,
        })
    );
}

#[test]
fn css_color_from_hex_mixed_case_accepted() {
    // §5.2 case-insensitive: `#aBcDeF` == `#abcdef`。
    assert_eq!(CssColor::from_hex("aBcDeF"), CssColor::from_hex("abcdef"));
}

#[test]
fn css_color_from_hex_invalid_length_returns_none() {
    // hex-notation grammar 外の length は spec-invalid → None。
    assert_eq!(CssColor::from_hex(""), None);
    assert_eq!(CssColor::from_hex("1"), None);
    assert_eq!(CssColor::from_hex("12"), None);
    assert_eq!(CssColor::from_hex("12345"), None);
    assert_eq!(CssColor::from_hex("1234567"), None);
    assert_eq!(CssColor::from_hex("123456789"), None);
}

#[test]
fn css_color_from_hex_non_hex_char_returns_none() {
    // non-hex byte → None (nibble parse で早期 fail)。
    assert_eq!(CssColor::from_hex("gggggg"), None);
    assert_eq!(CssColor::from_hex("12x456"), None);
    // 3-digit 内の non-hex も同様。
    assert_eq!(CssColor::from_hex("f0z"), None);
}

// ── background-color (CSS Backgrounds 3 §2.2) ──
//
// 5-sample accept check (task description Verification #4):
// named / hex / rgb() / rgba() / transparent が
// `Some(PropertyValue::BackgroundColor(<exact RGBA>))` を返す。
//
// exact RGBA assert が必要な理由: `Some(_)` の loose form だと
// `parse_color` の Ident arm が transparent に a=255 を返す regression
// (opaque black に落ちる bug) を silent pass してしまうため、
// 5 sample 全て CssColor 値まで check する。

#[test]
fn background_color_parse_named() {
    assert_eq!(
        parse("red", "background-color"),
        Some(PropertyValue::BackgroundColor(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }))
    );
}

#[test]
fn background_color_parse_hex() {
    assert_eq!(
        parse("#ff0000", "background-color"),
        Some(PropertyValue::BackgroundColor(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }))
    );
}

#[test]
fn background_color_parse_rgb() {
    assert_eq!(
        parse("rgb(255, 0, 0)", "background-color"),
        Some(PropertyValue::BackgroundColor(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }))
    );
}

#[test]
fn background_color_parse_rgba() {
    // rgba() alpha は number literal (0.0..=1.0)、final u8 conversion で
    // 0..=255 に mapping。0.5 → 128 (rounding は CSS Color 4 準拠)。
    assert_eq!(
        parse("rgba(0, 0, 0, 0.5)", "background-color"),
        Some(PropertyValue::BackgroundColor(CssColor {
            r: 0,
            g: 0,
            b: 0,
            a: 128
        }))
    );
}

#[test]
fn background_color_parse_transparent() {
    // CSS Color 4 §6.3 "The transparent keyword": shorthand for
    // rgba(0, 0, 0, 0)。spec initial value と一致 (CSS Backgrounds 3 §2.2)。
    assert_eq!(
        parse("transparent", "background-color"),
        Some(PropertyValue::BackgroundColor(CssColor::TRANSPARENT))
    );
}

#[test]
fn background_color_parse_invalid_returns_none() {
    // `none` は <color> grammar に含まれない spec-invalid keyword (task
    // Non-goals: spec-invalid → drop)。
    assert_eq!(parse("none", "background-color"), None);
    // Legacy comma-separated hsl() is a CSS Color 4-valid spelling.
    assert_eq!(
        parse("hsl(0, 100%, 50%)", "background-color"),
        Some(PropertyValue::BackgroundColor(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }))
    );
}

#[test]
fn background_color_key_returns_background_color() {
    // PropertyValue::BackgroundColor → PropertyKey::BackgroundColor (cascade
    // winner 選択の discriminant 導線、sibling `Color` key() と対称)。
    let v = PropertyValue::BackgroundColor(CssColor::TRANSPARENT);
    assert_eq!(v.key(), PropertyKey::BackgroundColor);
}

#[test]
fn font_size_parse_px() {
    assert_eq!(
        parse("16px", "font-size"),
        Some(PropertyValue::FontSize(Length::Px(16.0)))
    );
}

/// CSS Fonts 4 §2.5 の grammar `<length-percentage [0,∞]>` は font-relative
/// unit と percentage を含む。cascade の phase 2 (絶対化) が入ったので、
/// これらを parse 段で drop しなくなった。
#[test]
fn font_size_accepts_font_relative_and_percentage() {
    assert_eq!(
        parse("1.5em", "font-size"),
        Some(PropertyValue::FontSize(Length::Em(1.5)))
    );
    assert_eq!(
        parse("2rem", "font-size"),
        Some(PropertyValue::FontSize(Length::Rem(2.0)))
    );
    assert_eq!(
        parse("12pt", "font-size"),
        Some(PropertyValue::FontSize(Length::Pt(12.0)))
    );
    assert_eq!(
        parse("150%", "font-size"),
        Some(PropertyValue::FontSize(Length::Percent(150.0)))
    );
}

/// `math` は spec-valid だが未実装
/// (MathML scaling algorithm 未対応) として drop。
/// `<absolute-size>` / `<relative-size>` は受理済み —
/// 別 test (`font_size_accepts_absolute_size_keywords` /
/// `font_size_accepts_relative_size_keywords`) 参照。
#[test]
fn font_size_rejects_math_keyword() {
    assert_eq!(parse("math", "font-size"), None);
}

/// CSS Fonts 4 §2.5.1 <https://www.w3.org/TR/css-fonts-4/#absolute-size-mapping>
/// の scaling-factor table 全 8 keyword。`medium` = raikiri の固定基準
/// (16px) そのもの、他は table の分数を掛けたもの
/// (`resolve_relative_weight` 前例に倣い浮動小数 literal ではなく分数式で
/// 期待値を書く — 丸め誤差の議論を spec 引用だけで閉じるため)。
#[test]
fn font_size_accepts_absolute_size_keywords() {
    const MEDIUM: f32 = 16.0;
    let cases: &[(&str, f32)] = &[
        ("xx-small", MEDIUM * (3.0 / 5.0)),
        ("x-small", MEDIUM * (3.0 / 4.0)),
        ("small", MEDIUM * (8.0 / 9.0)),
        ("medium", MEDIUM),
        ("large", MEDIUM * (6.0 / 5.0)),
        ("x-large", MEDIUM * (3.0 / 2.0)),
        ("xx-large", MEDIUM * (2.0 / 1.0)),
        ("xxx-large", MEDIUM * (3.0 / 1.0)),
    ];
    for (keyword, px) in cases {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parse(keyword, "font-size"),
            Some(PropertyValue::FontSize(Length::Px(*px))),
            "keyword = {keyword}"
        );
    }
}

/// CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII
/// case-insensitive。sibling `font_weight_keyword_case_insensitive` と同 pattern。
#[test]
fn font_size_absolute_size_keyword_case_insensitive() {
    assert_eq!(
        parse("MEDIUM", "font-size"),
        Some(PropertyValue::FontSize(Length::Px(16.0)))
    );
    assert_eq!(
        parse("Large", "font-size"),
        Some(PropertyValue::FontSize(Length::Px(16.0 * (6.0 / 5.0))))
    );
}

/// `<relative-size>` (`larger` / `smaller`) は parse 段では解決せず
/// `PropertyValue::FontSizeRelative` をそのまま返す — 解決 (親の
/// computed font-size に対する read-modify-write) は
/// `crate::cascade` の責務 (`bolder` / `lighter` と同型)。 // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
#[test]
fn font_size_accepts_relative_size_keywords() {
    assert_eq!(
        parse("larger", "font-size"),
        Some(PropertyValue::FontSizeRelative(RelativeFontSize::Larger))
    );
    assert_eq!(
        parse("smaller", "font-size"),
        Some(PropertyValue::FontSizeRelative(RelativeFontSize::Smaller))
    );
    assert_eq!(
        parse("LARGER", "font-size"),
        Some(PropertyValue::FontSizeRelative(RelativeFontSize::Larger))
    );
}

/// `font-size: 12px` と `font-size: larger` は同じ property を競合する
/// (`PropertyValue::FontSizeRelative` doc 参照) — 別 key だと両方が
/// cascade で「勝つ」事態が起き spec (1 property = 1 winner) と食い違う。
#[test]
fn font_size_relative_shares_property_key_with_font_size() {
    assert_eq!(
        PropertyValue::FontSize(Length::Px(12.0)).key(),
        PropertyKey::FontSize
    );
    assert_eq!(
        PropertyValue::FontSizeRelative(RelativeFontSize::Larger).key(),
        PropertyKey::FontSize
    );
}

#[test]
fn font_size_rejects_negative() {
    // spec grammar `[0,∞]`: 負値は全 unit で drop (px だけではない)。
    assert_eq!(parse("-10px", "font-size"), None);
    assert_eq!(parse("-0.5px", "font-size"), None);
    assert_eq!(parse("-1em", "font-size"), None);
    assert_eq!(parse("-2rem", "font-size"), None);
    assert_eq!(parse("-12pt", "font-size"), None);
    assert_eq!(parse("-50%", "font-size"), None);
    // 追加した unit も `length_payload` 経由で同じ
    // non-negative check を通ることを pin。
    assert_eq!(parse("-1ex", "font-size"), None);
    assert_eq!(parse("-1cm", "font-size"), None);
}

#[test]
fn font_size_accepts_additional_units() {
    // CSS Fonts 4 §2.5 `<length-percentage [0,∞]>` —
    // 追加した font-relative / absolute unit も `font-size` 上で受理される
    // (`parse_length_value` の dispatch に mode 差は無い)。
    assert_eq!(
        parse("2ex", "font-size"),
        Some(PropertyValue::FontSize(Length::Ex(2.0)))
    );
    assert_eq!(
        parse("1cm", "font-size"),
        Some(PropertyValue::FontSize(Length::Cm(1.0)))
    );
}

#[test]
fn font_size_accepts_lh_and_rlh() {
    // CSS Fonts 4's `font-size` grammar
    // (`<absolute-size> | <relative-size> | <length-percentage [0,∞]>`)
    // has no carve-out excluding `lh`/`rlh` from `<length-percentage>`'s
    // `<length>` component (CSS Values 4 §6.1.1) — `font-size: 1lh` /
    // `font-size: 1rlh` are spec-valid and must survive parsing so the
    // cascade can pick them as a winner (dropping at parse time, as this
    // crate previously did, can change *which
    // declaration wins* the cascade — a stronger effect than an
    // incorrectly-resolved value). Resolution against the parent's used
    // line-height is `crate::resolve::resolve_font_size`'s concern, not
    // this parser's — pinned by that module's tests, not here.
    assert_eq!(
        parse("1lh", "font-size"),
        Some(PropertyValue::FontSize(Length::Lh(1.0)))
    );
    assert_eq!(
        parse("1rlh", "font-size"),
        Some(PropertyValue::FontSize(Length::Rlh(1.0)))
    );
}

#[test]
fn font_size_rejects_negative_lh_and_rlh() {
    // The grammar's `[0,∞]` non-negative constraint (`parse_font_size`
    // doc "Non-negative constraint" 節) applies to `lh`/`rlh` the same as
    // every other `Length` variant — `length_payload` reads their inner
    // `f32` generically, so this falls out of the existing post-filter
    // without a dedicated branch.
    assert_eq!(parse("-1lh", "font-size"), None);
    assert_eq!(parse("-1rlh", "font-size"), None);
}

#[test]
fn font_size_accepts_zero() {
    // spec `[0,∞]` の閉区間下端。`0px` は Dimension arm、bare `0` は
    // CSS Values 3 §5 unitless-zero clause の Number arm を通し、
    // parse_font_size の非負 Px post-filter を pass。
    assert_eq!(
        parse("0px", "font-size"),
        Some(PropertyValue::FontSize(Length::Px(0.0)))
    );
    assert_eq!(
        parse("0", "font-size"),
        Some(PropertyValue::FontSize(Length::Px(0.0)))
    );
}

#[test]
fn font_family_parse_comma_list() {
    let got = parse(r#"Arial, "Times New Roman", serif"#, "font-family");
    let expected = Some(PropertyValue::FontFamily(Arc::new(vec![
        Atom::from("Arial"),
        Atom::from("Times New Roman"),
        Atom::from("serif"),
    ])));
    assert_eq!(got, expected);
}

#[test]
fn font_family_unquoted_multi_word_single_family() {
    // CSS4: unquoted multi-word family name = ident sequence joined by space。
    let got = parse("Times New Roman", "font-family");
    let expected = Some(PropertyValue::FontFamily(Arc::new(vec![Atom::from(
        "Times New Roman",
    )])));
    assert_eq!(got, expected);
}

/// `initial_font_family()` は呼び出しごとに独立した call site でも
/// **同一** underlying `Vec` allocation を指す (`Arc::ptr_eq` = true) —
/// `OnceLock` 経由の shared slot であることの直接 pin。
///
/// この check は cascade level の test (`mod@crate::cascade` の
/// `initial_font_family_shares_arc_slot_across_independent_cascade_runs`
/// 等) では**代替できない** — `font-family` は inherited なので、単一
/// document 内の兄弟 element は `SpecifiedValues::inherit_from` の
/// 「親の Arc を bump」経路で共有される。これは同 document 内で
/// `initial_font_family()` が実質 1 回しか呼ばれないことを意味し、
/// ここで `OnceLock` を外して per-call `Arc::new(..)` に戻す regression を
/// 混入させても、その cascade level test は green のままになる
/// (実際に perturbation で確認済み)。
/// 本 test は `initial_font_family()` を直接 2 回呼ぶことで、この
/// inheritance-sharing の死角を回避する。
#[test]
fn initial_font_family_shares_arc_slot_across_calls() {
    assert!(Arc::ptr_eq(&initial_font_family(), &initial_font_family()));
}

/// `font-weight` の parse 期待値を組み立てる test-local helper。
fn fw(w: f32) -> Option<PropertyValue> {
    Some(PropertyValue::FontWeight(FontWeightValue::Absolute(w)))
}

#[test]
fn font_weight_parse_integer() {
    assert_eq!(parse("400", "font-weight"), fw(400.0));
    assert_eq!(parse("700", "font-weight"), fw(700.0));
}

#[test]
fn font_weight_parse_keyword_normal() {
    // CSS Fonts 4 §2.2: normal = 400。
    assert_eq!(parse("normal", "font-weight"), fw(400.0));
}

#[test]
fn font_weight_parse_keyword_bold() {
    // CSS Fonts 4 §2.2: bold = 700。
    assert_eq!(parse("bold", "font-weight"), fw(700.0));
}

#[test]
fn font_weight_keyword_case_insensitive() {
    // CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII
    // case-insensitive で照合する。
    assert_eq!(parse("NORMAL", "font-weight"), fw(400.0));
    assert_eq!(parse("Bold", "font-weight"), fw(700.0));
}

#[test]
fn font_weight_accepts_full_spec_range() {
    // CSS Fonts 4 §2.2 `<font-weight-absolute> = [ normal | bold |
    // <number [1,1000]> ]`。旧実装は `[100, 900]` に絞っていたが spec は
    // `[1, 1000]`。
    assert_eq!(parse("1", "font-weight"), fw(1.0));
    assert_eq!(parse("1000", "font-weight"), fw(1000.0));
    assert_eq!(parse("50", "font-weight"), fw(50.0));
    // 旧 range の両端も当然 valid のまま (regression guard)。
    assert_eq!(parse("100", "font-weight"), fw(100.0));
    assert_eq!(parse("900", "font-weight"), fw(900.0));
}

#[test]
fn font_weight_rejects_out_of_range_number() {
    // spec-invalid — spec grammar。§2.2 "Only values greater than or
    // equal to 1, and less than or equal to 1000, are valid, and all other
    // values are invalid"。
    assert_eq!(parse("0", "font-weight"), None);
    assert_eq!(parse("1001", "font-weight"), None);
    assert_eq!(parse("-100", "font-weight"), None);
    // 範囲判定は **丸める前の指定値** に対して行う: 丸めれば範囲内に入る
    // 値でも spec 上は invalid。
    assert_eq!(parse("0.6", "font-weight"), None);
    assert_eq!(parse("1000.4", "font-weight"), None);
    // 非有限値。`1e400` は f32 に収まらず ±inf に overflow するため、
    // `value <= 1000.0` (または `>= 1.0`) が成立せず reject される
    // (§2.2 "all other values are invalid" と一致) — これは genuine
    // magnitude overflow であり、`next_numeric_stable` が訂正する
    // zero-mantissa/huge-mantissa 由来の `NaN` collapse とは別の hazard
    // class (`parse_font_weight` doc参照)。`nan` / `inf` は `<number>`
    // production ではなく Ident token なので keyword arm にも該当せず
    // reject される — cssparser tokenizer の `NaN` artifact とは無関係。
    assert_eq!(parse("1e400", "font-weight"), None);
    assert_eq!(parse("-1e400", "font-weight"), None);
    assert_eq!(parse("nan", "font-weight"), None);
    assert_eq!(parse("inf", "font-weight"), None);
}

#[test]
fn font_weight_mirror_huge_mantissa_tiny_exponent_resolves_inside_valid_range() {
    // Same mirror-collapse class as
    // `parse_length_value_mirror_huge_mantissa_tiny_exponent_resolves_correctly`
    // — a mantissa long enough to itself overflow to `+Infinity` during
    // cssparser's digit-by-digit accumulation (`5` followed by 400
    // zeros), multiplied by a sufficiently negative exponent's
    // `10^exponent` (which underflows to `0.0`), collapses to
    // `Infinity * 0.0` = `NaN` in the tokenizer's own two-step
    // computation — even though the true value, `5 * 10^400 * 10^-399`
    // = `50`, lands squarely inside `<font-weight-absolute>`'s valid
    // `[1,1000]` range (CSS Fonts 4 §2.2). `next_numeric_stable`
    // (which `parse_font_weight` acquires its token through) recovers
    // this before the range guard ever runs, so this legitimate value
    // resolves to `FontWeightValue::Absolute(50.0)` instead of being
    // silently dropped.
    let digits = format!("5{}", "0".repeat(400)); // 5 * 10^400, overflows f64 digit accumulation to +Infinity
    let source = format!("{digits}e-399");
    assert_eq!(
        parse(&source, "font-weight"),
        Some(PropertyValue::FontWeight(FontWeightValue::Absolute(50.0)))
    );
}

#[test]
fn font_weight_computed_preserves_fractional_precision() {
    // **spec 準拠 pin。** §2.2 の computed value は "a number" であり、
    // §2.2.2 "Missing weights" <https://www.w3.org/TR/css-fonts-4/#missing-weights>
    // は "Fractional weights are valid" と明言する。旧実装 (computed side が
    // `u16`) は parse 時に round-half-away-from-zero で整数化しており、
    // これは spec 沈黙点の選択ではなく表現上の制約による既知 divergence
    // だった。payload / `ComputedValues.font_weight`
    // を `f32` に格上げしたことで丸め自体が不要になり、本 test はその
    // 解消を check する — もはや丸めていないことの regression guard。
    // 全て 2 進数で厳密表現可能な小数 (`.5` / `.25`) — parse 側と期待値の
    // 独立な文字列→f32 変換が bit-for-bit 一致することを保証でき、
    // 丸め誤差を懸念せず `assert_eq!` で直接比較できる。
    assert_eq!(parse("100.5", "font-weight"), fw(100.5));
    assert_eq!(parse("250.75", "font-weight"), fw(250.75));
    assert_eq!(parse("399.5", "font-weight"), fw(399.5));
    assert_eq!(parse("999.5", "font-weight"), fw(999.5));
}

#[test]
fn font_weight_wpt_font_weight_computed_150_25() {
    // WPT css/css-fonts/parsing/font-weight-computed.html:
    // `test_computed_value('font-weight', '150.25')` — 2-arg 形は
    // computed === specified を check する。parse 結果 (specified-equivalent
    // な `PropertyValue`) がそのまま `150.25` を保持することを確認する。
    // cascade を経由した computed 側の同値 check は
    // `crate::cascade::tests::font_weight_wpt_font_weight_computed_150_25`。
    assert_eq!(parse("150.25", "font-weight"), fw(150.25));
}

#[test]
fn font_weight_accepts_scientific_notation_number() {
    // `int_value` matcher から `value` (f32) 参照に変えた副次効果。
    // `1e3` は CSS Values 3 の `<number>` production として spec-valid
    // なので受理が正しい。
    assert_eq!(parse("1e3", "font-weight"), fw(1000.0));
}

#[test]
fn font_weight_parses_relative_keywords_as_sentinels() {
    // CSS Fonts 4 §2.2: `bolder` / `lighter` は継承値依存の relative
    // weight。parse 段では解けないので sentinel variant を返し、cascade が
    // 親の computed weight から解決する。
    assert_eq!(
        parse("bolder", "font-weight"),
        Some(PropertyValue::FontWeight(FontWeightValue::Bolder))
    );
    assert_eq!(
        parse("lighter", "font-weight"),
        Some(PropertyValue::FontWeight(FontWeightValue::Lighter))
    );
    // CSS Values 3 §3.1: relative keyword も ASCII case-insensitive。
    assert_eq!(
        parse("BOLDER", "font-weight"),
        Some(PropertyValue::FontWeight(FontWeightValue::Bolder))
    );
    assert_eq!(
        parse("Lighter", "font-weight"),
        Some(PropertyValue::FontWeight(FontWeightValue::Lighter))
    );
}

#[test]
fn font_weight_rejects_unknown_ident() {
    // spec-invalid keyword → declaration drop。
    assert_eq!(parse("normal-ish", "font-weight"), None);
    assert_eq!(parse("super-bold", "font-weight"), None);
}

#[test]
fn unknown_property_returns_none() {
    // `background-color` / `padding` / `margin` / `width` / `height` /
    // `float` が順次実装済 = ここから除外。
    // `cursor` (CSS Basic User Interface Module Level 3
    // <https://www.w3.org/TR/css-ui-3/#cursor>) は現時点で
    // parse_value dispatch に未登録 → fall-through で None が返る
    // canonical unknown-property canary。実装され次第、別の未実装
    // property 名へ再び移設すること。
    assert_eq!(parse("pointer", "cursor"), None);
}

// ── Display (CSS Display 3 §2) ─────────────

#[test]
fn display_parse_block() {
    assert_eq!(
        parse("block", "display"),
        Some(PropertyValue::Display(DisplayValue::Block))
    );
}

#[test]
fn display_parse_inline() {
    assert_eq!(
        parse("inline", "display"),
        Some(PropertyValue::Display(DisplayValue::Inline))
    );
}

#[test]
fn display_parse_inline_block() {
    // CSS Display 3 §2 <display-legacy>
    assert_eq!(
        parse("inline-block", "display"),
        Some(PropertyValue::Display(DisplayValue::InlineBlock))
    );
}

#[test]
fn display_parse_flow_root() {
    assert_eq!(
        parse("flow-root", "display"),
        Some(PropertyValue::Display(DisplayValue::FlowRoot))
    );
    assert_eq!(
        parse("FLOW-ROOT", "display"),
        Some(PropertyValue::Display(DisplayValue::FlowRoot))
    );
}

#[test]
fn display_parse_none() {
    // CSS Display 3 §2 <display-box>
    assert_eq!(
        parse("none", "display"),
        Some(PropertyValue::Display(DisplayValue::None))
    );
}

#[test]
fn display_parse_flex() {
    // CSS Display 3 §2.2 "Inner Display Layout Models" — `<display-inside>`
    // keyword, outer-defaulting rule makes it equivalent to `block flex`.
    assert_eq!(
        parse("flex", "display"),
        Some(PropertyValue::Display(DisplayValue::Flex))
    );
}

#[test]
fn display_parse_grid() {
    // CSS Display 3 §2.2 "Inner Display Layout Models" — `<display-inside>`
    // keyword, outer-defaulting rule makes it equivalent to `block grid`.
    assert_eq!(
        parse("grid", "display"),
        Some(PropertyValue::Display(DisplayValue::Grid))
    );
}

#[test]
fn display_parse_list_item() {
    // CSS Display 3 §2 <display-listitem>, outer-defaulting rule makes
    // it equivalent to `block flow list-item`. HTML Living Standard's
    // default UA stylesheet uses this for `li`
    // <https://html.spec.whatwg.org/multipage/rendering.html#lists>.
    assert_eq!(
        parse("list-item", "display"),
        Some(PropertyValue::Display(DisplayValue::ListItem))
    );
}

#[test]
fn display_parse_contents() {
    // CSS Display 3 §2.5 <display-box> keyword — element generates no
    // box of its own, children/pseudo-elements still generate boxes.
    assert_eq!(
        parse("contents", "display"),
        Some(PropertyValue::Display(DisplayValue::Contents))
    );
}

// ── List styling (CSS Lists 3 §3) ─────────────

#[test]
fn list_style_values_have_css_initial_defaults() {
    assert_eq!(ListStyleType::default(), ListStyleType::Disc);
    assert_eq!(ListStylePosition::default(), ListStylePosition::Outside);
}

#[test]
fn list_style_type_parses_builtin_custom_and_string_values() {
    assert_eq!(
        parse_entire("disc", "list-style-type"),
        Some(PropertyValue::ListStyleType(ListStyleType::Disc))
    );
    assert_eq!(
        parse_entire("none", "list-style-type"),
        Some(PropertyValue::ListStyleType(ListStyleType::None))
    );
    assert_eq!(
        parse_entire("upper-roman", "list-style-type"),
        Some(PropertyValue::ListStyleType(ListStyleType::Named(
            "upper-roman".into()
        )))
    );
    assert_eq!(
        parse_entire("\"→\"", "list-style-type"),
        Some(PropertyValue::ListStyleType(ListStyleType::String(
            "→".into()
        )))
    );
}

#[test]
fn list_style_image_parses_none_and_url() {
    assert_eq!(
        parse_entire("none", "list-style-image"),
        Some(PropertyValue::ListStyleImage(BackgroundImage::None))
    );
    assert_eq!(
        parse_entire("url(marker.png)", "list-style-image"),
        Some(PropertyValue::ListStyleImage(BackgroundImage::Url(
            "marker.png".to_string()
        )))
    );
    assert_eq!(
        parse_entire("url(marker.png) none", "list-style-image"),
        None
    );
}

#[test]
fn list_style_type_rejects_reserved_or_trailing_values() {
    for value in [
        "inherit",
        "initial",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ] {
        assert_eq!(parse_entire(value, "list-style-type"), None, "{value}");
    }
    assert_eq!(parse_entire("disc none", "list-style-type"), None);
    assert_eq!(parse_entire("url(marker.svg)", "list-style-type"), None);
}

#[test]
fn list_style_position_parses_case_insensitive_keywords_only() {
    assert_eq!(
        parse_entire("INSIDE", "list-style-position"),
        Some(PropertyValue::ListStylePosition(ListStylePosition::Inside))
    );
    assert_eq!(
        parse_entire("Outside", "list-style-position"),
        Some(PropertyValue::ListStylePosition(ListStylePosition::Outside))
    );
    assert_eq!(parse_entire("middle", "list-style-position"), None);
    assert_eq!(parse_entire("inside outside", "list-style-position"), None);
}

#[test]
fn display_rejects_unknown_ident() {
    // inline-flex / inline-grid / flow-root are accepted as their
    // corresponding formatting contexts.
    assert_eq!(
        parse("inline-flex", "display"),
        Some(PropertyValue::Display(DisplayValue::InlineFlex))
    );
    assert_eq!(
        parse("inline-grid", "display"),
        Some(PropertyValue::Display(DisplayValue::InlineGrid))
    );
    assert_eq!(
        parse_entire("flow-root extra", "display"),
        None,
        // cov:ignore: panic-message literal only executes on assertion failure.
        "display accepts one standalone keyword in this slice"
    );
}

#[test]
fn display_rejects_non_ident() {
    assert_eq!(parse("16px", "display"), None);
    assert_eq!(parse("100", "display"), None);
}

#[test]
fn display_is_case_insensitive() {
    // CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII case-insensitive
    assert_eq!(
        parse("BLOCK", "display"),
        Some(PropertyValue::Display(DisplayValue::Block))
    );
    assert_eq!(
        parse("Inline", "display"),
        Some(PropertyValue::Display(DisplayValue::Inline))
    );
    assert_eq!(
        parse("INLINE-BLOCK", "display"),
        Some(PropertyValue::Display(DisplayValue::InlineBlock))
    );
    assert_eq!(
        parse("Inline-Block", "display"),
        Some(PropertyValue::Display(DisplayValue::InlineBlock))
    );
    assert_eq!(
        parse("NONE", "display"),
        Some(PropertyValue::Display(DisplayValue::None))
    );
    assert_eq!(
        parse("None", "display"),
        Some(PropertyValue::Display(DisplayValue::None))
    );
    assert_eq!(
        parse("FLEX", "display"),
        Some(PropertyValue::Display(DisplayValue::Flex))
    );
    assert_eq!(
        parse("Grid", "display"),
        Some(PropertyValue::Display(DisplayValue::Grid))
    );
    assert_eq!(
        parse("LIST-ITEM", "display"),
        Some(PropertyValue::Display(DisplayValue::ListItem))
    );
    assert_eq!(
        parse("List-Item", "display"),
        Some(PropertyValue::Display(DisplayValue::ListItem))
    );
    assert_eq!(
        parse("CONTENTS", "display"),
        Some(PropertyValue::Display(DisplayValue::Contents))
    );
    assert_eq!(
        parse("Contents", "display"),
        Some(PropertyValue::Display(DisplayValue::Contents))
    );
}

// ── box-sizing (CSS Sizing 3 §3.3) ────────
//
// Verification anchors:
//   #1 content-box → Some(BoxSizing::ContentBox)
//   #2 border-box  → Some(BoxSizing::BorderBox)
//   #3 padding-box → None (spec 外、CSS UI 3 draft の削除済 keyword)
//   #4 initial + #5 non-inheritance test は crate::computed 側
//
// sibling: `display_*` / `text_align_*` の keyword parser test 群と同構造。

#[test]
fn box_sizing_parse_content_box() {
    assert_eq!(
        parse("content-box", "box-sizing"),
        Some(PropertyValue::BoxSizing(BoxSizing::ContentBox))
    );
}

#[test]
fn box_sizing_parse_border_box() {
    assert_eq!(
        parse("border-box", "box-sizing"),
        Some(PropertyValue::BoxSizing(BoxSizing::BorderBox))
    );
}

#[test]
fn box_sizing_rejects_unknown_ident() {
    // spec-invalid (→ drop):
    // - `padding-box` は CSS-UI 3 draft 相当だが css-sizing-3 では削除済み
    //   (spec note "supersedes the one in `[CSS-UI-3]`")、
    // - `margin-box` は grammar 外の任意 ident。
    assert_eq!(parse("padding-box", "box-sizing"), None);
    assert_eq!(parse("margin-box", "box-sizing"), None);
    assert_eq!(parse("bogus", "box-sizing"), None);
}

#[test]
fn box_sizing_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
    // canonical: PropertyValue doc「CSS-wide keyword」節。
    assert_eq!(parse("inherit", "box-sizing"), None);
    assert_eq!(parse("initial", "box-sizing"), None);
    assert_eq!(parse("unset", "box-sizing"), None);
    assert_eq!(parse("revert", "box-sizing"), None);
    assert_eq!(parse("revert-layer", "box-sizing"), None);
}

#[test]
fn box_sizing_rejects_non_ident() {
    assert_eq!(parse("16px", "box-sizing"), None);
    assert_eq!(parse("100", "box-sizing"), None);
}

#[test]
fn box_sizing_is_case_insensitive() {
    // CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII case-insensitive
    // (sibling `display_is_case_insensitive` と同 flavor)。
    assert_eq!(
        parse("CONTENT-BOX", "box-sizing"),
        Some(PropertyValue::BoxSizing(BoxSizing::ContentBox))
    );
    assert_eq!(
        parse("Border-Box", "box-sizing"),
        Some(PropertyValue::BoxSizing(BoxSizing::BorderBox))
    );
}

#[test]
fn box_sizing_key_returns_box_sizing() {
    // PropertyValue::BoxSizing → PropertyKey::BoxSizing (cascade winner
    // 選択の discriminant 導線、sibling `Display` / `TextAlign` key() と対称)。
    let v = PropertyValue::BoxSizing(BoxSizing::BorderBox);
    assert_eq!(v.key(), PropertyKey::BoxSizing);
}

// ── counter-* (CSS Lists 3 §4) ──

// `PropertyValue::Counter*(Arc<Vec<..>>)` に wrap したため、
// literal test 比較用に Arc<Vec<..>> を返す helper に切り替え
// (content/string_set helper と同 pattern)。
fn counter_pairs(pairs: &[(&str, i32)]) -> Arc<Vec<(SmolStr, i32)>> {
    Arc::new(
        pairs
            .iter()
            .map(|(name, value)| (SmolStr::new(name), *value))
            .collect(),
    )
}

#[test]
fn counter_reset_single_name_defaults_to_zero() {
    // spec: reset の default は 0
    assert_eq!(
        parse("chapter", "counter-reset"),
        Some(PropertyValue::CounterReset(counter_pairs(&[(
            "chapter", 0
        )])))
    );
}

#[test]
fn counter_reset_multiple_names_with_mixed_ints() {
    // 2 番目に integer が付く → 1 番目は default 0、2 番目は 3
    assert_eq!(
        parse("chapter section 3", "counter-reset"),
        Some(PropertyValue::CounterReset(counter_pairs(&[
            ("chapter", 0),
            ("section", 3)
        ])))
    );
}

#[test]
fn counter_reset_none_returns_empty_vec() {
    // spec: `none` は空リストと同等 (top-level alternative)
    // empty case は shared Arc slot (`empty_counter_entries`) を使う。
    assert_eq!(
        parse("none", "counter-reset"),
        Some(PropertyValue::CounterReset(empty_counter_entries()))
    );
}

#[test]
fn counter_reset_rejects_number_first() {
    // 先頭が number → ident が来るまで peel できず empty → None (drop)
    // spec §4: `<counter-name> = <custom-ident>` (数値は counter-name ではない)
    assert_eq!(parse("123 abc", "counter-reset"), None);
}

#[test]
fn counter_increment_single_name_defaults_to_one() {
    // spec: increment の default は 1
    assert_eq!(
        parse("chapter", "counter-increment"),
        Some(PropertyValue::CounterIncrement(counter_pairs(&[(
            "chapter", 1
        )])))
    );
}

#[test]
fn counter_increment_mixed_int_and_default() {
    // `chapter 2 section` → chapter=2、section=default(1)
    assert_eq!(
        parse("chapter 2 section", "counter-increment"),
        Some(PropertyValue::CounterIncrement(counter_pairs(&[
            ("chapter", 2),
            ("section", 1)
        ])))
    );
}

#[test]
fn counter_increment_accepts_negative_integer() {
    // spec §4: <integer> — negative も valid (counter を decrement する用途)
    assert_eq!(
        parse("chapter -1", "counter-increment"),
        Some(PropertyValue::CounterIncrement(counter_pairs(&[(
            "chapter", -1
        )])))
    );
}

#[test]
fn counter_increment_none_returns_empty_vec() {
    // empty case は shared Arc slot を使う。
    assert_eq!(
        parse("none", "counter-increment"),
        Some(PropertyValue::CounterIncrement(empty_counter_entries()))
    );
}

#[test]
fn counter_set_defaults_to_zero() {
    // spec: set の default は 0
    assert_eq!(
        parse("page 5 note", "counter-set"),
        Some(PropertyValue::CounterSet(counter_pairs(&[
            ("page", 5),
            ("note", 0)
        ])))
    );
}

#[test]
fn counter_set_none_returns_empty_vec() {
    // empty case は shared Arc slot を使う。
    assert_eq!(
        parse("none", "counter-set"),
        Some(PropertyValue::CounterSet(empty_counter_entries()))
    );
}

#[test]
fn counter_reset_is_case_insensitive_on_none() {
    // CSS spec: keyword `none` は ASCII case-insensitive
    // empty case は shared Arc slot を使う。
    assert_eq!(
        parse("NONE", "counter-reset"),
        Some(PropertyValue::CounterReset(empty_counter_entries()))
    );
}

#[test]
fn counter_reset_accepts_inherit_marker_and_rejects_other_reserved_names() {
    // `counter-reset: inherit` is retained as a page-context marker;
    // the other CSS-wide keywords are not valid counter names.
    assert_eq!(
        parse("inherit", "counter-reset"),
        Some(PropertyValue::CounterResetInherit)
    );
    assert_eq!(parse("initial", "counter-reset"), None);
    assert_eq!(parse("unset", "counter-reset"), None);
    assert_eq!(parse("revert", "counter-reset"), None);
    assert_eq!(parse("default", "counter-reset"), None);
}

#[test]
fn counter_reset_accepts_negative_integer() {
    // CSS Values 3 §4.2 "Integers: the <integer> type"
    // (https://www.w3.org/TR/css-values-3/#integers): <integer> は負値を含む。
    // increment だけでなく reset / set も同一 grammar。
    assert_eq!(
        parse("chapter -5", "counter-reset"),
        Some(PropertyValue::CounterReset(counter_pairs(&[(
            "chapter", -5
        )])))
    );
}

#[test]
fn counter_set_accepts_negative_integer() {
    // 同上 (parity with reset/increment negative-integer coverage)。
    assert_eq!(
        parse("page -3", "counter-set"),
        Some(PropertyValue::CounterSet(counter_pairs(&[("page", -3)])))
    );
}

// ── content property (CSS Content 3 §2) ──
//
// task の verification items は spec-derived。task 記述の
// `raikiri_traits::ContentValueItem` は下流 (raikiri-dom) mapping 先。
// raikiri-style は raikiri-traits に依存しない leaf crate
// のため、counter-* precedent に倣い local `ContentComponent` を emit
// する (原則 1: 前例主義)。Symbol → SmolStr、Url → String へ substitution。

fn content_items(source: &str) -> Vec<ContentComponent> {
    match parse(source, "content") {
        // PropertyValue::Content(Arc<Vec<..>>) を expose するため
        // (*v).clone() で Vec を deref-clone。tests は既存 shape のまま検証。
        Some(PropertyValue::Content(v)) => (*v).clone(),
        other => panic!("expected PropertyValue::Content, got {other:?}"),
    }
}

#[test]
fn content_parse_string_function() {
    // Verification 1: content: string(my_str)
    // → ContentComponent::String { name: "my_str", fetch: default (First) }
    let items = content_items("string(my_str)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::String {
            name: SmolStr::new("my_str"),
            fetch: StringFetchMode::First,
        }
    );
}

#[test]
fn content_parse_counter_function() {
    // Verification 2: content: counter(chapter)
    // → ContentComponent::Counter { name: "chapter", style: default (Decimal) }
    let items = content_items("counter(chapter)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_counters_function() {
    // Verification 3: content: counters(section, ".")
    // → ContentComponent::Counters { name, separator: ".", style: default }
    let items = content_items(r#"counters(section, ".")"#);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Counters {
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_target_counter_function() {
    // Verification 4: content: target-counter(url("#anchor"), page)
    // → ContentComponent::TargetCounter { url: "#anchor", name: "page", style: default }
    let items = content_items(r##"target-counter(url("#anchor"), page)"##);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::TargetCounter {
            url: String::from("#anchor"),
            name: SmolStr::new("page"),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_target_counters_function() {
    // Verification 5: content: target-counters(url("#anchor"), section, ".")
    // → ContentComponent::TargetCounters { url, name, separator, style: default }
    let items = content_items(r##"target-counters(url("#anchor"), section, ".")"##);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::TargetCounters {
            url: String::from("#anchor"),
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_target_text_first_letter() {
    // Verification 6: content: target-text(url("#anchor"), first-letter)
    // → ContentComponent::TargetText { url, part: ContentPart::FirstLetter }
    //
    // NB: task description の "content-first-letter" は spec (§2.6.3
    // `[ content | before | after | first-letter ]?`) と食い違うため、
    // spec-correct な `first-letter` を採用 (task 側の記述が誤りと
    // 判明したための訂正)。
    let items = content_items(r##"target-text(url("#anchor"), first-letter)"##);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::TargetText {
            url: String::from("#anchor"),
            part: ContentPart::FirstLetter,
        }
    );
}

#[test]
fn content_parse_attr_function() {
    // Verification 7: content: attr(href) → ContentComponent::Attr { name: "href" }
    let items = content_items("attr(href)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Attr {
            name: SmolStr::new("href"),
        }
    );
}

#[test]
fn content_parse_attr_untyped_fallbacks() {
    let items = content_items(r#"attr(missing, "Fallback value") attr(missing, invalid)"#);
    assert_eq!(
        items,
        vec![
            ContentComponent::AttrFallback {
                name: SmolStr::new("missing"),
                fallback: Some(SmolStr::new("Fallback value")),
            },
            ContentComponent::AttrFallback {
                name: SmolStr::new("missing"),
                fallback: None,
            },
        ]
    );
}

#[test]
fn content_parse_literal_string() {
    // Verification 8: content: "hello" → ContentComponent::Literal("hello")
    let items = content_items(r#""hello""#);
    assert_eq!(
        items,
        vec![ContentComponent::Literal(SmolStr::new("hello"))]
    );
}

#[test]
fn content_parse_mixed_sequence_preserves_order() {
    // Verification 9: content: "Chapter " counter(chapter) ": " string(chapter_title)
    // → 4-item Vec in order
    let items = content_items(r#""Chapter " counter(chapter) ": " string(chapter_title)"#);
    assert_eq!(items.len(), 4, "expected 4 items, got {items:?}");
    assert_eq!(
        items[0],
        ContentComponent::Literal(SmolStr::new("Chapter "))
    );
    assert_eq!(
        items[1],
        ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
    assert_eq!(items[2], ContentComponent::Literal(SmolStr::new(": ")));
    assert_eq!(
        items[3],
        ContentComponent::String {
            name: SmolStr::new("chapter_title"),
            fetch: StringFetchMode::First,
        }
    );
}

// ── content property: image / contents / <quote> / leader() (CSS Content 3
// §2.2 / §2.3 / §2.4.2 / §2.5.1 — under-accept fix、CssContent3 mode arm) ──

#[test]
fn content_parse_image_url_quoted_form() {
    // `<image>` の `<url>` alternative、`url("...")` (quoted) form。
    let items = content_items(r#"url("cat.png")"#);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Image {
            url: String::from("cat.png"),
        }
    );
}

#[test]
fn content_parse_image_url_unquoted_form() {
    // `<image>` の `<url>` alternative、`url(...)` (unquoted url-token) form。
    let items = content_items("url(cat.png)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Image {
            url: String::from("cat.png"),
        }
    );
}

#[test]
fn content_bare_string_is_still_literal_not_image() {
    // Regression check: `<image>` production は `<url> | <gradient>` のみで
    // bare `<string>` を含まない (target-* の `[<string>|<url>]` とは別
    // grammar)。`expect_url` は quoted string 単体を受理しないため
    // `content: "cat.png"` は Literal のまま — Image への誤変換防止。
    let items = content_items(r#""cat.png""#);
    assert_eq!(
        items,
        vec![ContentComponent::Literal(SmolStr::new("cat.png"))]
    );
}

#[test]
fn content_parse_contents_keyword() {
    // CSS Content 3 §2.3 "Elemental Content: the contents keyword"。
    let items = content_items("contents");
    assert_eq!(items, vec![ContentComponent::Contents]);
}

#[test]
fn content_contents_keyword_is_case_insensitive() {
    let items = content_items("CoNtEnTs");
    assert_eq!(items, vec![ContentComponent::Contents]);
}

#[test]
fn content_parse_quote_keywords() {
    // CSS Content 3 §2.4.2 `<quote> = open-quote | close-quote |
    // no-open-quote | no-close-quote` の 4 keyword 全数検証。
    assert_eq!(
        content_items("open-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::OpenQuote)]
    );
    assert_eq!(
        content_items("close-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::CloseQuote)]
    );
    assert_eq!(
        content_items("no-open-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::NoOpenQuote)]
    );
    assert_eq!(
        content_items("no-close-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::NoCloseQuote)]
    );
}

#[test]
fn content_quote_keyword_is_case_insensitive() {
    let items = content_items("OPEN-QUOTE");
    assert_eq!(
        items,
        vec![ContentComponent::Quote(QuoteKeyword::OpenQuote)]
    );
}

#[test]
fn content_parse_leader_dotted_solid_space_keywords() {
    // CSS Content 3 §2.5.1 `<leader-type> = dotted | solid | space | <string>`。
    assert_eq!(
        content_items("leader(dotted)"),
        vec![ContentComponent::Leader(LeaderType::Dotted)]
    );
    assert_eq!(
        content_items("leader(solid)"),
        vec![ContentComponent::Leader(LeaderType::Solid)]
    );
    assert_eq!(
        content_items("leader(space)"),
        vec![ContentComponent::Leader(LeaderType::Space)]
    );
}

#[test]
fn content_parse_leader_custom_string() {
    let items = content_items(r#"leader(".~.")"#);
    assert_eq!(
        items,
        vec![ContentComponent::Leader(LeaderType::String(SmolStr::new(
            ".~."
        )))]
    );
}

#[test]
fn content_leader_is_case_insensitive() {
    let items = content_items("LEADER(DOTTED)");
    assert_eq!(items, vec![ContentComponent::Leader(LeaderType::Dotted)]);
}

#[test]
fn content_leader_rejects_missing_argument() {
    // spec production `leader( <leader-type> )` に `?` が無いため引数必須。
    // bare `leader()` は spec-invalid → declaration drop。
    assert_eq!(parse("leader()", "content"), None);
}

#[test]
fn content_leader_rejects_unknown_keyword() {
    assert_eq!(parse("leader(bogus)", "content"), None);
}

#[test]
fn content_rejects_unknown_bare_keyword() {
    // `parse_content_bare_keyword` の 5 keyword (`contents` / 4 `<quote>`)
    // いずれにも一致しない ident は catch-all `_ => None` に落ちる —
    // items 0 → declaration drop (単独 token の場合)。
    assert_eq!(parse("bogus", "content"), None);
}

#[test]
fn content_unknown_bare_keyword_mid_list_stops_items_and_leaves_leftover() {
    // 認識済み item (`counter(chapter)`) の後に未知 ident が来た場合、
    // items+ loop は unknown token で break する (catch-all の break 経路)。
    // caller (rule.rs) の `expect_exhausted` 相当は `parse` helper では
    // 経由しないため、本 helper 経由では 1-item 到達で観測できる — leftover
    // 自体の drop 挙動は既存 `content_rejects_unknown_function` /
    // `string_set_accepts_missing_comma_single_leftover_entry` と同じ
    // break-then-leftover pattern の non-regression check。
    let items = content_items("counter(chapter) bogus");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }]
    );
}

#[test]
fn content_parse_mixed_sequence_with_new_alternatives() {
    // image / contents / quote / leader を既存 alternative と混在させ、
    // 順序が保持されることを検証。
    let items =
        content_items(r#"open-quote "term" close-quote leader(dotted) url("icon.png") contents"#);
    assert_eq!(
        items,
        vec![
            ContentComponent::Quote(QuoteKeyword::OpenQuote),
            ContentComponent::Literal(SmolStr::new("term")),
            ContentComponent::Quote(QuoteKeyword::CloseQuote),
            ContentComponent::Leader(LeaderType::Dotted),
            ContentComponent::Image {
                url: String::from("icon.png"),
            },
            ContentComponent::Contents,
        ]
    );
}

// ── string-set narrow <content-list> gate: image / contents / quote /
// leader() (CSS GCPM 3 §1.1.1 L82) ──
//
// GCPM 3 §1.1.1 narrow list には `<image>` / `contents` / `<quote>` /
// `leader()` のいずれも含まれない (既存の string_set_rejects_* group と
// 同じ rationale — sibling test 群と揃えて 1 declaration = 1 rejection の
// check にする)。

#[test]
fn string_set_rejects_image_url() {
    assert_eq!(parse(r#"title url("a.png")"#, "string-set"), None);
}

#[test]
fn string_set_rejects_contents_keyword() {
    assert_eq!(parse("title contents", "string-set"), None);
}

#[test]
fn string_set_rejects_quote_keyword() {
    assert_eq!(parse("title open-quote", "string-set"), None);
}

#[test]
fn string_set_rejects_leader_fn() {
    assert_eq!(parse("title leader(dotted)", "string-set"), None);
}

// ── content property edge cases (spec-derived、guard rails) ──

#[test]
fn content_normal_returns_empty_list() {
    // spec §1: `normal` は「content が明示されない場合と同じ」= 空 list として保持。
    // pseudo-element generation 判断は下流で行う。
    assert_eq!(
        parse("normal", "content"),
        Some(PropertyValue::Content(empty_content_list()))
    );
}

#[test]
fn content_none_keeps_a_suppression_sentinel() {
    assert_eq!(
        parse("none", "content"),
        Some(PropertyValue::Content(Arc::new(vec![
            ContentComponent::None
        ])))
    );
}

#[test]
fn content_string_with_fetch_last_keyword() {
    // spec §2.7.2 の string() 第 2 引数 keyword を全て受理することを smoke で pin。
    let items = content_items("string(head, last)");
    assert_eq!(
        items,
        vec![ContentComponent::String {
            name: SmolStr::new("head"),
            fetch: StringFetchMode::Last,
        }]
    );
}

#[test]
fn content_target_text_default_part_is_content() {
    // target-text() の第 2 引数省略時、raikiri は ContentPart::Content を
    // フォールバック値として使う (根拠は spec の "default" 宣言ではない —
    // CSS Content 3 §2.6.3 は第 2 引数省略時の値を規定していない)。
    let items = content_items(r##"target-text(url("#a"))"##);
    assert_eq!(
        items,
        vec![ContentComponent::TargetText {
            url: String::from("#a"),
            part: ContentPart::Content,
        }]
    );
}

#[test]
fn content_counter_rejects_none_name() {
    // spec CSS Lists 3 §4 <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>:
    // "A <counter-name> name cannot match the keyword `none`; such an identifier
    // is invalid as a <counter-name>". §4.7 counter() の first argument が
    // <counter-name> production のため `counter(none)` は declaration drop。
    // counter-reset/increment/set (property.rs 既存) と一貫、Chrome/FF と一致。
    assert_eq!(parse("counter(none)", "content"), None);
}

#[test]
fn content_counters_rejects_none_name() {
    // spec CSS Lists 3 §4 / §4.7: counters() の first argument も
    // <counter-name> production、`none` は invalid。
    assert_eq!(parse(r#"counters(none, ".")"#, "content"), None);
}

#[test]
fn content_counter_with_named_style_preserves_ident() {
    // spec CSS Lists 3 §4.7: 第 2 引数 `<counter-style>` は decimal 以外の
    // named style も受ける。下流 (raikiri-dom) が解釈するため raw ident 保持。
    let items = content_items("counter(chapter, upper-alpha)");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Named(SmolStr::new("upper-alpha")),
        }]
    );
}

#[test]
fn content_rejects_unknown_function() {
    // 未知 function は認識できず、items 開始 token として peel 失敗。
    // 先頭 token が unknown function だと empty items → None (drop)。
    assert_eq!(parse("bogus(x)", "content"), None);
}

#[test]
fn content_case_insensitive_function_name() {
    // spec: function name は ASCII case-insensitive。
    let items = content_items("COUNTER(chapter)");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }]
    );
}

#[test]
fn content_key_maps_to_content_property_key() {
    // PropertyValue::Content → PropertyKey::Content (cascade winner 選択の
    // discriminant integrity、既存 sibling counter-* と同じ pattern)。
    let cv = PropertyValue::Content(empty_content_list());
    assert_eq!(cv.key(), PropertyKey::Content);
}

// ── parse_optional_counter_style trailing-comma strict reject ──
//
// CSS Lists 3 §4.7 `counter(<counter-name>, <counter-style>?)` /
// CSS Content 3 §2.6.1-2 `target-counter()` / `target-counters()` は
// `<counter-style>?` — `,` を先行させる時は ident 必須。trailing-comma
// (`counter(chapter,)` 等) は spec-invalid → declaration ごと drop すべき。
// sibling `parse_string_fetch` / `parse_content_part` は既に strict `?`
// propagation、`parse_optional_counter_style` のみ silent Decimal fallback
// していた regression を check する。

#[test]
fn content_counter_rejects_trailing_comma() {
    // `counter(chapter,)` — comma 消費後に ident 不在。spec-invalid、
    // declaration drop = None (Chrome/Firefox と同挙動)。
    assert_eq!(parse("counter(chapter,)", "content"), None);
}

#[test]
fn content_counters_rejects_trailing_comma() {
    // `counters(chapter, ".",)` — separator string 後の trailing comma。
    assert_eq!(parse(r#"counters(chapter, ".",)"#, "content"), None);
}

#[test]
fn content_target_counter_rejects_trailing_comma() {
    // `target-counter(url("#a"), page,)` — name 後の trailing comma。
    // target-counter/target-counters は parse_optional_counter_style を
    // 経由 (parse_target_counter_fn / parse_target_counters_fn) するため同じ pattern で drop。
    assert_eq!(
        parse(r##"target-counter(url("#a"), page,)"##, "content"),
        None
    );
}

#[test]
fn content_target_counters_rejects_trailing_comma() {
    // `target-counters(url("#a"), section, ".",)` — separator 後の trailing。
    assert_eq!(
        parse(r##"target-counters(url("#a"), section, ".",)"##, "content"),
        None
    );
}

#[test]
fn content_string_rejects_trailing_comma() {
    // 対照実験 (現行 strict の維持確認): `string(foo,)` は
    // `parse_string_fetch` が `?` 経由で伝播、既に None。
    assert_eq!(parse("string(foo,)", "content"), None);
}

#[test]
fn content_target_text_rejects_trailing_comma() {
    // 対照実験: `target-text(url("#a"),)` は `parse_content_part` が
    // `?` 経由で伝播、既に None。
    assert_eq!(parse(r##"target-text(url("#a"),)"##, "content"), None);
}

#[test]
fn content_counter_accepts_bare_default() {
    // `counter(chapter)` — trailing comma 無しの正常 case、Decimal default
    // で Some を返す (silent fallback を strict にしても正常 path は変えない)。
    let items = content_items("counter(chapter)");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }]
    );
}

// ── string-set (CSS GCPM 3 §1.1.1) ──
//
// grammar: `none | [ <custom-ident> <content-list> ]#` — 各 entry は
// (name, content-list) pair、`ContentComponent` + `parse_content_list_items`
// を reuse。task description の "4-item Vec" は entry name の分を content 側に
// 誤って含めた結果、実態は 3-item (name は tuple の第 1 要素)。

fn string_set_entries(source: &str) -> Vec<(SmolStr, Vec<ContentComponent>)> {
    match parse(source, "string-set") {
        // PropertyValue::StringSet(Arc<Vec<..>>)、content_items と同 pattern。
        Some(PropertyValue::StringSet(v)) => (*v).clone(),
        other => panic!("expected PropertyValue::StringSet, got {other:?}"),
    }
}

#[test]
fn string_set_single_entry_with_literal() {
    // Verification 1: string-set: my_str "hello"
    // → `[(SmolStr("my_str"), [Literal("hello")])]`
    let entries = string_set_entries(r#"my_str "hello""#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("my_str"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Literal(SmolStr::new("hello"))]
    );
}

#[test]
fn string_set_mixed_content_list_preserves_order() {
    // Verification 2:
    // string-set: chapter_title counter(chapter) ": " attr(title)
    //
    // 先頭 `chapter_title` は entry name (tuple 第 1 要素)。content-list は
    // 残りの `counter(chapter) ": " attr(title)` = 3 items。
    //
    // NB: 原 test は末尾に `string(chapter_title)` を置いていたが、
    // GCPM 3 §1.1.1 narrow list は `string()` function を含まないため
    // `attr()` (GCPM narrow list の 5 alt の 1 つ) に
    // swap。テストの主意 (mixed content-list の order 保持) は保つ。
    let entries = string_set_entries(r#"chapter_title counter(chapter) ": " attr(title)"#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("chapter_title"));
    assert_eq!(entries[0].1.len(), 3);
    assert_eq!(
        entries[0].1[0],
        ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
    assert_eq!(
        entries[0].1[1],
        ContentComponent::Literal(SmolStr::new(": "))
    );
    assert_eq!(
        entries[0].1[2],
        ContentComponent::Attr {
            name: SmolStr::new("title"),
        }
    );
}

#[test]
fn string_set_comma_separated_multi_entry() {
    // Verification 3: string-set: a "x", b "y" → 2 entries
    let entries = string_set_entries(r#"a "x", b "y""#);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, SmolStr::new("a"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Literal(SmolStr::new("x"))]
    );
    assert_eq!(entries[1].0, SmolStr::new("b"));
    assert_eq!(
        entries[1].1,
        vec![ContentComponent::Literal(SmolStr::new("y"))]
    );
}

#[test]
fn string_set_none_returns_empty_vec() {
    // spec §1.1.1: top-level `none` = empty list
    assert_eq!(
        parse("none", "string-set"),
        Some(PropertyValue::StringSet(empty_string_set_entries()))
    );
}

#[test]
fn string_set_rejects_reserved_css_wide_keyword_as_name() {
    // spec §1.1.1 + CSS Values 4 §4.2
    // <https://www.w3.org/TR/css-values-4/#custom-idents>:
    // `<custom-ident>` は CSS-wide keyword 除外。
    // 先頭 ident が `inherit` → try_parse rewind で entries 空 → None。
    //
    // NB: 先頭が `none` の場合は top-level alternative の branch を先に
    // 通って `Some(empty)` を返し、leftover は下流 `expect_exhausted` で
    // declaration drop (rule.rs level)。この case は parse_value 単体では
    // 検証しない。
    assert_eq!(parse("inherit \"x\"", "string-set"), None);
    assert_eq!(parse("initial \"x\"", "string-set"), None);
    assert_eq!(parse("unset \"x\"", "string-set"), None);
    assert_eq!(parse("revert \"x\"", "string-set"), None);
    assert_eq!(parse("default \"x\"", "string-set"), None);
}

#[test]
fn string_set_rejects_name_without_content_list() {
    // spec §1.1.1 + Content 3 §2: `<content-list>` は 1+ items 必須。
    // name だけで items 0 → declaration drop (None)。
    assert_eq!(parse("my_str", "string-set"), None);
}

#[test]
fn string_set_is_case_insensitive_on_none() {
    // CSS spec: keyword `none` は ASCII case-insensitive
    assert_eq!(
        parse("NONE", "string-set"),
        Some(PropertyValue::StringSet(empty_string_set_entries()))
    );
}

#[test]
fn string_set_key_maps_to_string_set_property_key() {
    // PropertyValue::StringSet → PropertyKey::StringSet (cascade winner 選択の
    // discriminant integrity、既存 sibling counter-* / content と同じ pattern)。
    let v = PropertyValue::StringSet(empty_string_set_entries());
    assert_eq!(v.key(), PropertyKey::StringSet);
}

// ── string-set trailing-comma strict reject ──
//
// `#` (comma-separated multiplier、CSS Values 4 §2.3
// <https://www.w3.org/TR/css-values-4/#mult-comma>) は trailing comma を
// 許容しない。GCPM 3 §1.1.1 <string-set-value> = `[ <custom-ident>
// <content-list> ]#` は entry 間 comma 必須 + trailing comma 禁止。
//
// 初期実装は separator loop で `try_parse(expect_comma).is_err() {
// break }` していたため、trailing comma を silently 受理していた (comma を
// consume 後 next iteration で name parse fail → break → 既存 entries を
// Some で返す)。同じ principle の `.ok()?` propagation で strict 化。

#[test]
fn string_set_rejects_trailing_comma_single_entry() {
    // `string-set: a "x",` → trailing comma → declaration drop。
    // pre-fix は Some(`[(a, [Literal("x")])]`) を silently 返していた。
    assert_eq!(parse(r#"a "x","#, "string-set"), None);
}

#[test]
fn string_set_rejects_trailing_comma_two_entries() {
    // `string-set: a "x", b "y",` → trailing comma → declaration drop。
    // 内部 comma 1 個は valid separator、末尾 comma のみが `#` 違反。
    assert_eq!(parse(r#"a "x", b "y","#, "string-set"), None);
}

#[test]
fn string_set_rejects_trailing_comma_three_entries() {
    // 3 entries + trailing comma — chain 越しの一貫 strict reject を pin。
    assert_eq!(parse(r#"a "x", b "y", c "z","#, "string-set"), None);
}

#[test]
fn string_set_rejects_missing_entry_after_comma() {
    // `string-set: a "x", b` → comma 後 name は取れるが `<content-list>`
    // が 0 items (`parse_content_list_items` empty) → declaration drop。
    // trailing-comma 系とは reject 経路が異なる (items-empty) 独立 pin。
    assert_eq!(parse(r#"a "x", b"#, "string-set"), None);
}

#[test]
fn string_set_accepts_missing_comma_single_leftover_entry() {
    // `string-set: a "x" b "y"` は separator comma 欠如。iter 1 で
    // (a, `["x"]`) push 後、bottom expect_comma fail → break、leftover
    // `b "y"` は本 helper (parse_value 直呼び、caller expect_exhausted
    // 経由なし) では drop されず 1 entry の Some として観測される。
    // 実 caller (rule.rs) は expect_exhausted で declaration drop する
    // — 本 test は parse_string_set の break exit が Some (`.ok()?`
    // 経路と混同しない) であることを check する目的、trailing-comma fix の
    // non-regression coverage。
    let entries = string_set_entries(r#"a "x" b "y""#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("a"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Literal(SmolStr::new("x"))]
    );
}

// ── string-set narrow <content-list> gate (CSS GCPM 3 §1.1.1) ──
//
// GCPM 3 §1.1.1 L82 verbatim: <content-list> = [ <string> | <counter()> |
// <counters()> | <content()> | <attr()> ]+ — CSS Content 3 §2 broad list を
// string-set 用に narrower 再定義。`string()` (function、bare literal とは別)
// および `target-counter()` / `target-counters()` / `target-text()` は
// spec grammar に含まれず、`ContentListMode::GcpmStringSet` mode dispatch で
// reject する (parse_content_list_items が 0 items → parse_string_set →
// None → declaration drop、cascade shadow 例
// `p.hi { string-set: title target-counter(url("#x"), page); }` の spec 準拠
// 挙動 = .hi rule drop → parser layer で確認)。
//
// 一方 content property (CssContent3 mode) はこれら全てを引き続き受理する
// (下の content_parse_* 系 check test 群で non-regression 検証)。

#[test]
fn string_set_rejects_string_fn() {
    // GCPM 3 §1.1.1 L82 は `string()` function を narrow list から除外。
    // bare `<string>` literal (`"..."`) と混同しないよう function 側のみ reject。
    assert_eq!(parse("title string(x)", "string-set"), None);
}

#[test]
fn string_set_rejects_target_counter_fn() {
    // GCPM 3 §1.1.1 L82 は `target-counter()` を narrow list から除外。
    // cascade shadow の主要例、declaration drop → cascade で
    // 先行の spec-valid rule が winner になる shape。
    assert_eq!(
        parse(r##"title target-counter(url("#a"), page)"##, "string-set"),
        None
    );
}

#[test]
fn string_set_rejects_target_counters_fn() {
    // GCPM 3 §1.1.1 L82 は `target-counters()` を narrow list から除外。
    assert_eq!(
        parse(
            r##"title target-counters(url("#a"), section, ".")"##,
            "string-set"
        ),
        None
    );
}

#[test]
fn string_set_rejects_target_text_fn() {
    // GCPM 3 §1.1.1 L82 は `target-text()` を narrow list から除外。
    assert_eq!(
        parse(r##"title target-text(url("#a"))"##, "string-set"),
        None
    );
}

// ── content() function (CSS GCPM 3 §1.1.1.1) ──
//
// grammar (spec verbatim, line 758 of TR/css-gcpm-3/, string-set/GCPM3側の
// grammar):
//   content() = content(`[text | before | after | first-letter]`)
// 4 keyword。keyword 省略時は `text` をフォールバック値として使う (根拠は
// GCPM 3 側の spec "default" 宣言ではない — grammar に `?` が無く、"default
// をどう定義するか" 自体が未解決の WG issue として残っている)。GCPM 3
// §1.1.1 の narrow `<content-list>` と CSS Content 3 §2 の broad
// `<content-list>` の両方に対し unconditional に受理されるため、
// string-set および content property 双方の content-list 内で受理される
// (`ContentListMode` mode dispatch 導入後も `content()` arm は両
// mode で unconditional accept)。
//
// CSS Content 3 §2.7.3 は content() を `?` 付き 5 keyword (`marker` 含む)
// で別途定義しており、GCPM 3 §1.1.1.1 と keyword 集合が食い違う。この
// 実装は keyword 集合について GCPM 3 §1.1.1.1 に従うと決めており、content
// property 側でも `marker` は意図的に reject する (詳細・根拠は
// `parse_content_fn` の doc comment 参照)。CSS Content 3 §2.7.3 の
// `marker` keyword は既知の feature gap として残る。
//
// pre-fix reproduction: `string-set: title content(text)` は
// silent drop していた (parse_content_function match arm 欠如 →
// parse_content_list_items break → 0 items → parse_string_set None →
// declaration drop)。arm 追加で Some を返すことを check する。

#[test]
fn string_set_content_text_reproduces_pre_fix_drop() {
    // description の主要 repro case:
    // pre-fix では declaration drop = None、post-fix では
    // (title, `[Content{keyword: Text}]`) を含む Some を返す。
    let entries = string_set_entries("title content(text)");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("title"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_explicit_text_keyword() {
    // §1.1.1.1: `content(text)` は element の string value (bare `content()`
    // のフォールバック値と同じ keyword だが、明示的 keyword 保持で
    // downstream の分岐余地を残す)。
    let items = content_items("content(text)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_default_keyword_on_empty_parens() {
    // §1.1.1.1 の spec 例 `h2 { string-set: heading content() }` (string-set
    // /GCPM3側の文脈) — bare `content()` は `text` をフォールバック値として
    // 使う (根拠は GCPM 3 側の spec "default" 宣言ではない。
    // content property側でのgrammar相反は上記 parse_content_fn doc 参照)。
    let items = content_items("content()");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_before_keyword() {
    // §1.1.1.1 の spec 例 `h1 { string-set: header content(before) ':' content(text); }`
    // で使われる `before` keyword。
    let items = content_items("content(before)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Before,
        }]
    );
}

#[test]
fn content_content_fn_after_keyword() {
    let items = content_items("content(after)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::After,
        }]
    );
}

#[test]
fn content_content_fn_first_letter_keyword() {
    let items = content_items("content(first-letter)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::FirstLetter,
        }]
    );
}

#[test]
fn content_content_fn_rejects_unknown_keyword() {
    // GCPM 3 §1.1.1.1 の grammar は `[text | before | after | first-letter]`
    // の 4 alternative のみ (string-set/GCPM3側の文脈)。それ以外の ident は
    // parse_content_text_keyword が None を返し、上位伝播で
    // parse_content_list_items が break、declaration drop = None。`marker`
    // はこの GCPM3 grammar には無い。CSS Content 3 §2.7.3 は独自に
    // content() を `marker` 含む 5 keyword で定義しているが、この実装は
    // keyword 集合について GCPM 3 §1.1.1.1 に従うと決めており
    // (parse_content_fn の doc comment 参照)、`marker` reject は意図した
    // 挙動であって未解決の問題ではない。
    // 本 test は現状の GCPM3-scoped 実装の挙動を
    // check するものであり、`marker` が spec に一切存在しないという主張では
    // ない。
    assert_eq!(parse("content(marker)", "content"), None);
    assert_eq!(parse("content(bogus)", "content"), None);
}

#[test]
fn content_content_fn_rejects_target_text_keyword() {
    // §1.1.1.1 は `text` alternative を持つ (target-text() §2.6.3 は `content`)。
    // spec spelling divergence — `content(content)` は spec-invalid、reject。
    // 混同 (sibling ContentPart 再利用) を防ぐ regression check。
    assert_eq!(parse("content(content)", "content"), None);
}

#[test]
fn content_content_fn_case_insensitive_keyword() {
    // spec 慣行: keyword は ASCII case-insensitive。
    let items = content_items("content(TEXT)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_case_insensitive_function_name() {
    // parse_content_function は既存 arm と同じく ASCII case-insensitive dispatch。
    let items = content_items("CONTENT(before)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Before,
        }]
    );
}

#[test]
fn content_content_fn_rejects_extra_argument() {
    // grammar は single-argument。余剰 token は
    // parse_nested_block 内 parse_entirely が拒否し declaration drop。
    assert_eq!(parse("content(text, extra)", "content"), None);
    assert_eq!(parse("content(text before)", "content"), None);
}

#[test]
fn string_set_content_fn_mixed_with_other_items() {
    // §1.1.1.1 の spec 例:
    //   h1 { string-set: header content(before) ':' content(text); }
    // → (header, `[Content{Before}, Literal(":"), Content{Text}]`) 3 items。
    let entries = string_set_entries(r#"header content(before) ":" content(text)"#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("header"));
    assert_eq!(
        entries[0].1,
        vec![
            ContentComponent::Content {
                keyword: ContentTextKeyword::Before,
            },
            ContentComponent::Literal(SmolStr::new(":")),
            ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            },
        ]
    );
}

// ── position: running() (CSS GCPM 3 §1.2.1) ──
//
// Verification items 1-6 は task description 由来、
// canonical shape は後に amended。sibling は counter-* /
// content / string-set の SmolStr wire-through pattern。

#[test]
fn position_parse_running_header() {
    // Verification 1: position: running(header)
    // → PropertyValue::Position(PositionValue::Running("header"))
    assert_eq!(
        parse("running(header)", "position"),
        Some(PropertyValue::Position(PositionValue::Running(
            SmolStr::new("header")
        )))
    );
}

#[test]
fn position_parse_running_footer() {
    // Verification 2: 別 name の smoke — SmolStr::new が生きていることを pin。
    assert_eq!(
        parse("running(footer)", "position"),
        Some(PropertyValue::Position(PositionValue::Running(
            SmolStr::new("footer")
        )))
    );
}

#[test]
fn position_parse_static() {
    // Verification 5 baseline: position: static → PositionValue::Static。
    // apply_value は no-op、running_templates は inherit_from の initial
    // (空 Vec) が残る = cascade winner が earlier running(...) を suppress する
    // ID 用途 (cascade.rs 側の `static_position_wins_over_running` で検証)。
    assert_eq!(
        parse("static", "position"),
        Some(PropertyValue::Position(PositionValue::Static))
    );
}

#[test]
fn position_running_case_insensitive_function_name() {
    // Verification 4: function name は ASCII case-insensitive (CSS spec 慣行)、
    // custom-ident は case-preserving。
    assert_eq!(
        parse("RUNNING(header)", "position"),
        Some(PropertyValue::Position(PositionValue::Running(
            SmolStr::new("header")
        )))
    );
}

#[test]
fn position_running_rejects_none_custom_ident() {
    // Verification 6: `running(none)` reject。`none` は position property
    // spec-defined keyword ではないが、runtime resolve で `element(none)` 参照が
    // silent match するのを避けるため custom-ident としても弾く (string-set
    // と同じ規約)。
    assert_eq!(parse("running(none)", "position"), None);
}

#[test]
fn position_running_rejects_reserved_css_wide_keyword() {
    // spec CSS Values 4 §4.2 <https://www.w3.org/TR/css-values-4/#custom-idents>:
    // <custom-ident> は CSS-wide keyword + `default` 除外。
    // position: running(inherit) 等は declaration drop。
    assert_eq!(parse("running(inherit)", "position"), None);
    assert_eq!(parse("running(initial)", "position"), None);
    assert_eq!(parse("running(unset)", "position"), None);
    assert_eq!(parse("running(revert)", "position"), None);
    assert_eq!(parse("running(default)", "position"), None);
}

#[test]
fn position_rejects_missing_custom_ident() {
    // spec §1.2.1: `running() = running( <custom-ident> )` — argument 必須。
    // 空 argument は malformed、declaration drop。
    assert_eq!(parse("running()", "position"), None);
}

#[test]
fn position_parse_sticky() {
    // CSS Positioned Layout Module Level 3 §3 sticky positioning
    // <https://www.w3.org/TR/css-position-3/#sticky-pos>:
    // `position: sticky` は有効な position 値。本 crate では parse 段階で
    // PositionValue::Sticky として保持する (layout 連携は将来)。
    assert_eq!(
        parse("sticky", "position"),
        Some(PropertyValue::Position(PositionValue::Sticky))
    );
    // ident は ASCII case-insensitive (cssparser の expect_ident_matching 準拠、
    // `static` と同様)。
    assert_eq!(
        parse("Sticky", "position"),
        Some(PropertyValue::Position(PositionValue::Sticky))
    );
    assert_eq!(
        parse("STICKY", "position"),
        Some(PropertyValue::Position(PositionValue::Sticky))
    );
}

#[test]
fn position_rejects_out_of_scope_keywords() {
    // relative / absolute / fixed は受理する (position:relative offset 実装)。
    // `sticky` も受理。
    assert_eq!(
        parse("relative", "position"),
        Some(PropertyValue::Position(PositionValue::Relative))
    );
    assert_eq!(
        parse("absolute", "position"),
        Some(PropertyValue::Position(PositionValue::Absolute))
    );
    assert_eq!(
        parse("fixed", "position"),
        Some(PropertyValue::Position(PositionValue::Fixed))
    );
    // それ以外の keyword は drop
    assert_eq!(parse("inherit", "position"), None);
    assert_eq!(parse("initial", "position"), None);
}

#[test]
fn position_rejects_running_with_extra_arg() {
    // `running(a, b)` — parse_nested_block が parse_entirely 経由で
    // 余剰 token を検知し、declaration drop になる。
    assert_eq!(parse("running(a, b)", "position"), None);
}

// ── parse_length_value helper ────────────────────
//
// helper 単体を叩く共通 fixture — property dispatcher (`parse_value`) を経由せず
// 5 unit sample (`px` / `em` / `rem` / `%` / `pt`) の parse を直接 verify する。
// `parse_font_size` 経由 test は上流に既存 (`font_size_parse_px` 等)、そちらは
// px-only post-filter を verify するので分離する。

fn parse_length(source: &str, allow_percentage: bool) -> Option<Length> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_length_value(&mut parser, allow_percentage)
}

#[test]
fn parse_length_value_accepts_px() {
    assert_eq!(parse_length("10px", false), Some(Length::Px(10.0)));
    // length-percentage mode でも px 受理 (mode 非依存)。
    assert_eq!(parse_length("10px", true), Some(Length::Px(10.0)));
}

#[test]
fn parse_length_value_accepts_em() {
    // CSS Values 4 §6.1.1 em (https://www.w3.org/TR/css-values-4/#em):
    // authored `1.2em` を Length::Em(1.2) にそのまま保持 (resolve は下流責務)。
    assert_eq!(parse_length("1.2em", false), Some(Length::Em(1.2)));
}

#[test]
fn parse_length_value_accepts_rem() {
    // CSS Values 4 §6.1.1 rem (https://www.w3.org/TR/css-values-4/#rem):
    // root element の font-size 基準、authored value を Length::Rem に格納。
    assert_eq!(parse_length("1rem", false), Some(Length::Rem(1.0)));
}

#[test]
fn parse_length_value_accepts_pt() {
    // CSS Values 4 §6.2 absolute lengths (https://www.w3.org/TR/css-values-4/#absolute-lengths):
    // 1pt = 1/72 in, 1in = 96px、resolve 側で 12pt → 16px 相当に変換。
    assert_eq!(parse_length("12pt", false), Some(Length::Pt(12.0)));
}

#[test]
fn parse_length_value_accepts_percentage_when_allowed() {
    // CSS Values 4 §5.5 (https://www.w3.org/TR/css-values-4/#percentages):
    // `<length-percentage>` mode でのみ受理。cssparser `unit_value = 0.5` を
    // × 100.0 で authored `50` に戻して Length::Percent(50.0) に格納。
    assert_eq!(parse_length("50%", true), Some(Length::Percent(50.0)));
}

#[test]
fn parse_length_value_extreme_percentage_saturates_to_f32_max_not_inf() {
    // `1e40%` は cssparser tokenizer 側で
    // `unit_value = 1e40 / 100.0 = 1e38` (f32 有限範囲 `3.4028235e38` 内)
    // になるが、authored number へ戻す本 helper の `× 100.0` 自体が
    // f32 overflow を起こし +Inf を作っていた (fix 前)。
    //
    // CSS Values 4 §5 "Range Checking and Precision for Numeric Types"
    // <https://www.w3.org/TR/css-values-4/#numeric-types>:
    // "When a value cannot be explicitly supported due to
    // range/precision limitations, it must be converted to the closest
    // value supported by the implementation" — 非有限は許容されないため、
    // 符号を保持しつつ f32::MAX に寄った有限値を check する。
    assert_eq!(parse_length("1e40%", true), Some(Length::Percent(f32::MAX)));
    // 符号保持も合わせて check (負の overflow は -f32::MAX へ)。
    assert_eq!(
        parse_length("-1e40%", true),
        Some(Length::Percent(-f32::MAX))
    );
}

#[test]
fn parse_length_value_zero_mantissa_huge_exponent_percentage_resolves_to_zero() {
    // `0e999%` is a zero-mantissa, huge-exponent literal — cssparser's
    // tokenizer computes its pre-`/100` magnitude as `0.0 *
    // f64::powf(10., 999.)`, and `10f64.powf(999.0)` is `+Inf`, so the
    // product collapses to `NaN` per IEEE 754, even though CSS Syntax
    // Level 3 §4.3.13's own formula gives an exact `0` for this input
    // (`property.rs` module doc's "Numeric-token NaN stabilization"
    // section is canonical for the mechanism). `next_numeric_stable`
    // (which `parse_length_value` acquires its token through) corrects
    // this before `parse_length_value` ever sees the `Token::Percentage`,
    // so `0e999%` resolves to the spec-correct `Length::Percent(0.0)`
    // directly — it no longer reaches the `is_infinite()`-only
    // saturation guard as `NaN` at all (that guard remains, for the
    // genuinely-different `1e40%` magnitude-overflow case pinned above).
    assert_eq!(parse_length("0e999%", true), Some(Length::Percent(0.0)));
    // Sign is preserved through the recovery (`-0.0 == 0.0` under IEEE
    // 754, so this also confirms the negative-mantissa path resolves,
    // not just that it doesn't panic).
    assert_eq!(parse_length("-0e999%", true), Some(Length::Percent(0.0)));
}

#[test]
fn parse_length_value_zero_mantissa_with_fractional_part_huge_exponent_resolves_to_zero() {
    // Same collapse as `0e999`, but with a written fractional part
    // (`numeric_token_prefix`'s `.`-branch, not exercised by the
    // integer-only `0e999`/`0e999%` cases above) — `0.0e999px` still
    // has zero mantissa (`0 + 0 * 10^-1 == 0`), so it resolves to
    // `Length::Px(0.0)` the same way.
    assert_eq!(parse_length("0.0e999px", false), Some(Length::Px(0.0)));
}

#[test]
fn parse_length_value_mirror_huge_mantissa_tiny_exponent_resolves_correctly() {
    // The `0 * Infinity` collapse this crate works around
    // (`0e999`-shaped literals, zero mantissa / huge exponent) has a
    // mirror case: a mantissa long enough to itself overflow to
    // `+Infinity` while being accumulated digit-by-digit (cssparser
    // `tokenizer.rs`'s `consume_numeric`, no exponent needed for this
    // half), multiplied by a sufficiently negative exponent's
    // `10^exponent` (which underflows to `0.0`), hits `Infinity * 0.0`
    // = `NaN` the same way — for a literal whose true value is small
    // but nonzero. `1` followed by 400 zeros, `e-400`, has true value
    // `1` (`1eN * 10^-N == 1` for any `N`); this crate's recovery does
    // not special-case which operand collapsed to `0`/`Infinity` (it
    // simply re-parses the raw token text), so it resolves this
    // mirror case for free, not just the zero-mantissa one.
    let digits = format!("1{}", "0".repeat(400)); // 10^400, overflows f64 digit accumulation to +Infinity
    let source = format!("{digits}e-400px");
    assert_eq!(parse_length(&source, false), Some(Length::Px(1.0)));
}

#[test]
fn stabilize_nan_numeric_value_is_a_no_op_for_non_nan_input() {
    // Direct unit test of the private recovery primitive's documented
    // "no-op" contract for the common case — every call site
    // (`expect_number_stable`/`next_numeric_stable`) already gates on
    // `value.is_nan()` before calling this function, so this branch
    // is otherwise never exercised through the public parsing paths
    // above (they only ever pass a `NaN` `value` in practice). The
    // `raw_number_text` argument is deliberately garbage here (it
    // would never actually be parsed, since the `!value.is_nan()`
    // early return fires first) to prove the early return, not the
    // reparse path, is what's under test.
    assert_eq!(stabilize_nan_numeric_value("not a number", 5.0), 5.0);
    assert_eq!(
        stabilize_nan_numeric_value("not a number", f32::INFINITY),
        f32::INFINITY
    );
}

#[test]
fn stabilize_nan_percentage_value_is_a_no_op_for_non_nan_input() {
    // Same contract, `Token::Percentage`'s `unit_value` counterpart —
    // see `stabilize_nan_numeric_value_is_a_no_op_for_non_nan_input`.
    assert_eq!(stabilize_nan_percentage_value("not a number", 0.5), 0.5);
    assert_eq!(
        stabilize_nan_percentage_value("not a number", f32::NEG_INFINITY),
        f32::NEG_INFINITY
    );
}

#[test]
fn parse_length_value_rejects_percentage_in_length_only_mode() {
    // `<length>` mode (font-size 等) では `%` は grammar 外、None を返す。
    assert_eq!(parse_length("50%", false), None);
}

#[test]
fn parse_length_value_rejects_unsupported_unit() {
    // (b) 非対応 — viewport-relative unit / `cap` / `rcap` は本 helper で
    // 引き続き silent drop。`lh` / `rlh` は受理側へ移った
    // (下記 `parse_length_value_accepts_lh` / `_rlh` を参照)。
    assert_eq!(parse_length("10vw", false), None);
    assert_eq!(parse_length("1cap", true), None);
    // container-query unit (CSS Contain 3 §6) — `_` arm 直前 comment が
    // 挙げる `cq*` 一覧をこの assertion で check する。comment のみで
    // test 未網羅だと、将来 `cq*` 対応 arm が誤って追加されても
    // どの test も落ちず canonical comment が silent に stale 化する
    // (spec-lens follow-up として追加)。
    assert_eq!(parse_length("10cqw", false), None);
}

#[test]
fn parse_length_value_accepts_lh() {
    // https://www.w3.org/TR/css-values-4/#lh — authored value をそのまま保持。
    assert_eq!(parse_length("1.5lh", false), Some(Length::Lh(1.5)));
}

#[test]
fn parse_length_value_accepts_rlh() {
    // https://www.w3.org/TR/css-values-4/#rlh
    assert_eq!(parse_length("2rlh", false), Some(Length::Rlh(2.0)));
}

// ── 追加 font-relative unit (CSS Values 4 §6.1.1) ──

#[test]
fn parse_length_value_accepts_ex() {
    // https://www.w3.org/TR/css-values-4/#ex — authored value をそのまま保持。
    assert_eq!(parse_length("2ex", false), Some(Length::Ex(2.0)));
}

#[test]
fn parse_length_value_accepts_rex() {
    // https://www.w3.org/TR/css-values-4/#rex
    assert_eq!(parse_length("2rex", false), Some(Length::Rex(2.0)));
}

#[test]
fn parse_length_value_accepts_ch() {
    // https://www.w3.org/TR/css-values-4/#ch
    assert_eq!(parse_length("3ch", false), Some(Length::Ch(3.0)));
}

#[test]
fn parse_length_value_accepts_rch() {
    // https://www.w3.org/TR/css-values-4/#rch
    assert_eq!(parse_length("3rch", false), Some(Length::Rch(3.0)));
}

#[test]
fn parse_length_value_accepts_ic() {
    // https://www.w3.org/TR/css-values-4/#ic
    assert_eq!(parse_length("1.5ic", false), Some(Length::Ic(1.5)));
}

#[test]
fn parse_length_value_accepts_ric() {
    // https://www.w3.org/TR/css-values-4/#ric
    assert_eq!(parse_length("1.5ric", false), Some(Length::Ric(1.5)));
}

// ── 追加 absolute unit (CSS Values 4 §6.2) ──

#[test]
fn parse_length_value_accepts_cm() {
    assert_eq!(parse_length("2cm", false), Some(Length::Cm(2.0)));
}

#[test]
fn parse_length_value_accepts_mm() {
    assert_eq!(parse_length("5mm", false), Some(Length::Mm(5.0)));
}

#[test]
fn parse_length_value_accepts_q() {
    // `Q` — unit token は `to_ascii_lowercase()` を経て `"q"` として dispatch
    // される。case-insensitivity test でも uppercase `Q` を確認する。
    assert_eq!(parse_length("40Q", false), Some(Length::Q(40.0)));
}

#[test]
fn parse_length_value_accepts_in() {
    assert_eq!(parse_length("1in", false), Some(Length::In(1.0)));
}

#[test]
fn parse_length_value_accepts_pc() {
    assert_eq!(parse_length("6pc", false), Some(Length::Pc(6.0)));
}

#[test]
fn parse_length_value_accepts_unitless_zero_only() {
    // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths>:
    // "For zero lengths the unit identifier is optional (i.e. can be
    // syntactically represented as the `<number>` 0)." — bare `0` は
    // mode 非依存で Length::Px(0.0) 受理 (両 mode 網羅で mode-independence pin)。
    assert_eq!(parse_length("0", false), Some(Length::Px(0.0)));
    assert_eq!(parse_length("0", true), Some(Length::Px(0.0)));
    // 非零 unitless number は grammar 上 length ではない — `== 0.0` guard で
    // 分岐して下段 `_ => None` fallthrough で drop。drop 経路は mode 非依存
    // (guard を通らず fallthrough する path が両 mode 共通) のため 1 mode で pin。
    assert_eq!(parse_length("5", false), None);
    assert_eq!(parse_length("-1", false), None);
}

#[test]
fn parse_length_value_rejects_non_numeric_token() {
    assert_eq!(parse_length("medium", false), None);
    assert_eq!(parse_length("", false), None);
}

#[test]
fn parse_length_value_preserves_negative_sign() {
    // helper は sign check を行わない — property ごとに要件が異なるため
    // (font-size は non-negative post-filter、margin は negative 許容)。
    assert_eq!(parse_length("-5px", false), Some(Length::Px(-5.0)));
    assert_eq!(parse_length("-1em", false), Some(Length::Em(-1.0)));
}

#[test]
fn parse_length_value_unit_dispatch_case_insensitive() {
    // CSS spec: unit identifier は ASCII case-insensitive。
    assert_eq!(parse_length("10PX", false), Some(Length::Px(10.0)));
    assert_eq!(parse_length("1.5EM", false), Some(Length::Em(1.5)));
    assert_eq!(parse_length("2Rem", false), Some(Length::Rem(2.0)));
    assert_eq!(parse_length("14Pt", false), Some(Length::Pt(14.0)));
    // `unit.to_ascii_lowercase()` の dispatch key はすべて lowercase
    // (`"q"` / `"in"` 等) — uppercase 単位が正しく畳み込まれることを
    // 個別に確認する (`Q` は特に取り違えやすい)。
    assert_eq!(parse_length("10IN", false), Some(Length::In(10.0)));
    assert_eq!(parse_length("40Q", false), Some(Length::Q(40.0)));
    assert_eq!(parse_length("2CM", false), Some(Length::Cm(2.0)));
    assert_eq!(parse_length("2EX", false), Some(Length::Ex(2.0)));
    assert_eq!(parse_length("2CH", false), Some(Length::Ch(2.0)));
    assert_eq!(parse_length("2IC", false), Some(Length::Ic(2.0)));
}

// ── padding (CSS Box 3 §4.1 physical + §4.2 shorthand) ──
//
// Primary sources:
// - https://www.w3.org/TR/css-box-3/#padding-physical
//   "Negative values for padding properties are invalid." — non-negative
//   constraint を parse-time enforce (parse_padding_side が全 Length variant
//   で >= 0.0 check、負値 = declaration drop)。
// - https://www.w3.org/TR/css-box-3/#padding-shorthand
//   `<'padding-top'>{1,4}` — 1-4 value expansion (top/right/bottom/left)。

fn padding_sides(top: Length, right: Length, bottom: Length, left: Length) -> Sides<Length> {
    Sides {
        top,
        right,
        bottom,
        left,
    }
}

// Verification #3 — longhand parse 4 arm (px / % / em / pt の 5 unit)。
#[test]
fn padding_top_parses_px() {
    assert_eq!(
        parse("10px", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Px(10.0)))
    );
}

#[test]
fn padding_right_parses_percentage() {
    // spec grammar `<length-percentage>` — % 受理。
    assert_eq!(
        parse("5%", "padding-right"),
        Some(PropertyValue::PaddingRight(Length::Percent(5.0)))
    );
}

#[test]
fn padding_bottom_parses_em() {
    assert_eq!(
        parse("1em", "padding-bottom"),
        Some(PropertyValue::PaddingBottom(Length::Em(1.0)))
    );
}

#[test]
fn padding_left_parses_pt() {
    assert_eq!(
        parse("12pt", "padding-left"),
        Some(PropertyValue::PaddingLeft(Length::Pt(12.0)))
    );
}

// Verification #4 — shorthand 1-4 value expansion (CSS Box 3 §4.2)。
#[test]
fn padding_shorthand_one_value_all_sides() {
    // 1 value → 4 sides = value
    let px10 = Length::Px(10.0);
    assert_eq!(
        parse("10px", "padding"),
        Some(PropertyValue::Padding(Sides::all(px10)))
    );
}

#[test]
fn padding_shorthand_two_values_top_bottom_left_right() {
    // 2 values → top/bottom = 1st, left/right = 2nd
    let px10 = Length::Px(10.0);
    let px20 = Length::Px(20.0);
    assert_eq!(
        parse("10px 20px", "padding"),
        Some(PropertyValue::Padding(padding_sides(
            px10, px20, px10, px20
        )))
    );
}

#[test]
fn padding_shorthand_three_values_top_horizontal_bottom() {
    // 3 values → top = 1st, left/right = 2nd, bottom = 3rd
    let px10 = Length::Px(10.0);
    let px20 = Length::Px(20.0);
    let px30 = Length::Px(30.0);
    assert_eq!(
        parse("10px 20px 30px", "padding"),
        Some(PropertyValue::Padding(padding_sides(
            px10, px20, px30, px20
        )))
    );
}

#[test]
fn padding_shorthand_four_values_clockwise() {
    // 4 values → top / right / bottom / left (clockwise from top)
    assert_eq!(
        parse("10px 20px 30px 40px", "padding"),
        Some(PropertyValue::Padding(padding_sides(
            Length::Px(10.0),
            Length::Px(20.0),
            Length::Px(30.0),
            Length::Px(40.0),
        )))
    );
}

#[test]
fn padding_shorthand_mixed_units() {
    // spec (CSS Box 3) §4.2 は per-value `<'padding-top'>` = `<length-percentage>` を許容 —
    // 混合 unit も spec-valid (padding: 10px 5% 1em 12pt)。
    assert_eq!(
        parse("10px 5% 1em 12pt", "padding"),
        Some(PropertyValue::Padding(padding_sides(
            Length::Px(10.0),
            Length::Percent(5.0),
            Length::Em(1.0),
            Length::Pt(12.0),
        )))
    );
}

// Verification #5 — non-negative constraint (spec-literal claim)。
#[test]
fn padding_top_rejects_negative_px() {
    // spec (CSS Box 3) §4.1: "Negative values for padding properties are invalid."。
    assert_eq!(parse("-5px", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_percentage() {
    // 負 percentage も同様に spec-invalid。
    assert_eq!(parse("-10%", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_em() {
    // 負 em (font-relative) も spec-invalid。
    assert_eq!(parse("-1em", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_rem() {
    // 全 Length variant 経路の non-negative check check (rem)。
    assert_eq!(parse("-0.5rem", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_pt() {
    // 全 Length variant 経路の non-negative check check (pt)。
    assert_eq!(parse("-3pt", "padding-top"), None);
}

#[test]
fn padding_top_accepts_zero() {
    // zero (bound の下端) は spec grammar `[0,∞]` の閉区間で有効。
    // `0px` は Dimension arm、bare `0` は CSS Values 3 §5 unitless-zero clause
    // の Number arm を通し、非負 filter を pass。
    assert_eq!(
        parse("0px", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Px(0.0)))
    );
    assert_eq!(
        parse("0", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Px(0.0)))
    );
}

#[test]
fn padding_shorthand_rejects_any_negative_value() {
    // `padding: 10px -5px` — spec (CSS Box 3) §4.2 の {1,4} multiplier は各 iteration が
    // 有効 `<'padding-top'>` であることを要求。2 番目 `-5px` は spec (CSS Box 3) §4.1
    // `[0,∞]` 制約違反で fail、try_parse rewind で 1-value form の Some を
    // parse_padding_shorthand が返す。ここで DeclParser の expect_exhausted
    // が leftover `-5px` を検知して declaration ごと drop する — 実 caller
    // 経路として rule.rs 経由で drop を check (parse_value 単体では
    // Some(all(10px)) が観測されるが、それは leftover 込みで invalid)。
    let decls_2 = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding: 10px -5px;",
    )));
    assert!(
        decls_2.is_empty(),
        "`padding: 10px -5px` must drop via expect_exhausted leftover"
    );
    // 4 value form 内の 4 番目が負値 case — 同様 leftover 経由 drop。
    let decls_4 = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding: 10px 20px 30px -40px;",
    )));
    assert!(
        decls_4.is_empty(),
        "`padding: 10px 20px 30px -40px` must drop via expect_exhausted leftover"
    );
}

// Verification #6 — `auto` keyword reject (spec grammar に無い)。
#[test]
fn padding_top_rejects_auto_keyword() {
    // spec (CSS Box 3) §4.1 grammar = `<length-percentage>` のみ、`auto` は margin 側の
    // extension で padding には無い。parse_length_value の Dimension /
    // Percentage arm fall-through で自然 reject。
    assert_eq!(parse("auto", "padding-top"), None);
}

#[test]
fn padding_shorthand_rejects_auto_keyword() {
    // shorthand も同様 auto reject (1st value で fail、全体 drop)。
    assert_eq!(parse("auto", "padding"), None);
}

#[test]
fn padding_shorthand_mixed_with_auto_drops_via_leftover() {
    // `padding: 10px auto` — 1st 成功 (10px)、2nd で auto → try_parse rewind、
    // 1-value form の Some を parse_padding_shorthand が返す。ここまでは
    // parse_value 単体で観測可能だが、DeclParser の expect_exhausted が
    // leftover `auto` を検知して declaration drop する — 実 caller 経路の
    // check として rule.rs 経由でも drop することを確認。
    let decls = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding: 10px auto;",
    )));
    assert!(
        decls.is_empty(),
        "`padding: 10px auto` must drop via expect_exhausted leftover"
    );
}

// Verification #6 — spec grammar 外 unit の drop (vw / cap 等、非対応)。
#[test]
fn padding_top_rejects_unsupported_unit() {
    // (b) 非対応 — vw / cap 等は spec-valid だが
    // 未対応、parse_length_value 側で drop、`None`
    // propagate → declaration drop。`ch` / `lh` / `rlh` はそれぞれ受理側へ移った
    // (`padding_top_accepts_ch` / `padding_top_accepts_lh` 参照)。
    assert_eq!(parse("10vw", "padding-top"), None);
    assert_eq!(parse("5cap", "padding-top"), None);
}

#[test]
fn padding_top_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`。
    assert_eq!(
        parse("5lh", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Lh(5.0)))
    );
    assert_eq!(
        parse("1rlh", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Rlh(1.0)))
    );
}

#[test]
fn padding_top_accepts_ch() {
    assert_eq!(
        parse("2ch", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Ch(2.0)))
    );
}

#[test]
fn padding_top_accepts_cm() {
    assert_eq!(
        parse("2cm", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Cm(2.0)))
    );
}

#[test]
fn padding_top_rejects_negative_cm() {
    // 全 Length variant 経路の non-negative check check (cm、新規 absolute unit)。
    assert_eq!(parse("-1cm", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_ex() {
    // 全 Length variant 経路の non-negative check check (ex、新規 font-relative unit)。
    assert_eq!(parse("-1ex", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_lh() {
    // 全 Length variant 経路の non-negative check check (`lh`/`rlh`、
    // `length_payload` の OR-pattern に `Lh`/`Rlh`
    // を足し忘れていないことの直接 pin)。
    assert_eq!(parse("-1lh", "padding-top"), None);
    assert_eq!(parse("-1rlh", "padding-top"), None);
}

/// 追加した残り unit (`rex` / `rch` / `ic` / `ric` /
/// `mm` / `Q`) を `length_payload` 経由で直接 exercise する — 他 call site
/// (font-size / width / height / margin / border-width / line-height) の
/// テストは Ex / Ch / Cm / In / Pc しか通さないため、`length_payload` の
/// OR-pattern 全 arm の patch coverage には本 test が要る。
#[test]
fn padding_top_accepts_remaining_additional_units() {
    assert_eq!(
        parse("1rex", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Rex(1.0)))
    );
    assert_eq!(
        parse("1rch", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Rch(1.0)))
    );
    assert_eq!(
        parse("1ic", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Ic(1.0)))
    );
    assert_eq!(
        parse("1ric", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Ric(1.0)))
    );
    assert_eq!(
        parse("1mm", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Mm(1.0)))
    );
    assert_eq!(
        parse("40Q", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Q(40.0)))
    );
}

// Verification — Sides::all constructor + PropertyKey mapping smoke。
#[test]
fn padding_key_maps_to_padding_property_keys() {
    // 5 discriminant (4 longhand + 1 shorthand) が個別 PropertyKey を返すこと。
    // cascade winner selection の discriminant integrity 確認。
    assert_eq!(
        PropertyValue::PaddingTop(Length::Px(0.0)).key(),
        PropertyKey::PaddingTop
    );
    assert_eq!(
        PropertyValue::PaddingRight(Length::Px(0.0)).key(),
        PropertyKey::PaddingRight
    );
    assert_eq!(
        PropertyValue::PaddingBottom(Length::Px(0.0)).key(),
        PropertyKey::PaddingBottom
    );
    assert_eq!(
        PropertyValue::PaddingLeft(Length::Px(0.0)).key(),
        PropertyKey::PaddingLeft
    );
    assert_eq!(
        PropertyValue::Padding(Sides::all(Length::Px(0.0))).key(),
        PropertyKey::Padding
    );
}

// ── line-height (CSS Inline 3 §5.1) ────────────────
//
// Verification 5/6/7 の spec-derived: grammar `normal |
// <number [0,∞]> | <length-percentage [0,∞]>` — 4 accept branch + negative
// reject + Number vs Length variant distinction を check する。
//
// sibling: parse_display (keyword accept)、parse_font_size (Length
// post-filter for non-negative)、parse_length_value (unit dispatch)。

#[test]
fn line_height_parse_normal_keyword() {
    // Verification 5.1: `line-height: normal` → LineHeight::Normal
    assert_eq!(
        parse("normal", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Normal))
    );
}

#[test]
fn line_height_parse_bare_number() {
    // Verification 5.2 + 6: `line-height: 1.5` (bare number, no unit) →
    // LineHeight::Number(1.5)。Token::Number arm を通り Length branch には
    // 落ちない (Number vs Length distinction load-bearing、下流 special
    // behavior "specified value inherit" のための variant tag)。
    assert_eq!(
        parse("1.5", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Number(1.5)))
    );
}

#[test]
fn line_height_parse_length_px() {
    // Verification 5.3: `line-height: 24px` → LineHeight::Length(Px(24.0))
    assert_eq!(
        parse("24px", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Px(
            24.0
        ))))
    );
}

#[test]
fn line_height_parse_length_percentage() {
    // Verification 5.4: `line-height: 150%` → LineHeight::Length(Percent(150.0))
    // parse_length_value(allow_percentage=true) が Percent branch を有効化。
    assert_eq!(
        parse("150%", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(
            Length::Percent(150.0)
        )))
    );
}

#[test]
fn line_height_number_vs_em_are_distinct_variants() {
    // Verification 6: `1.5` (unitless) と `1.5em` (dimensioned) は同じ scalar
    // でも別 variant に mapping (Token::Number vs Token::Dimension で分岐)。
    // spec §5.1 unitless number は child が specified value を inherit する
    // special behavior、Length variant は通常 resolve — 下流が区別する必要。
    let number = parse("1.5", "line-height");
    let length_em = parse("1.5em", "line-height");
    assert_eq!(
        number,
        Some(PropertyValue::LineHeight(LineHeight::Number(1.5)))
    );
    assert_eq!(
        length_em,
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Em(
            1.5
        ))))
    );
    assert_ne!(number, length_em, "Number and Length must be distinct");
}

#[test]
fn line_height_accepts_length_em_rem_pt() {
    // 5 unit sample の length-percentage branch smoke — parse_length_value
    // helper との integration を check (em/rem/pt helper 経由)。
    assert_eq!(
        parse("1.2em", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Em(
            1.2
        ))))
    );
    assert_eq!(
        parse("1rem", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Rem(
            1.0
        ))))
    );
    assert_eq!(
        parse("12pt", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Pt(
            12.0
        ))))
    );
}

#[test]
fn sides_all_constructor_replicates_value() {
    // Sides::all(v) は 4 field を全て v で埋める。
    let sides = Sides::all(Length::Px(7.5));
    assert_eq!(sides.top, Length::Px(7.5));
    assert_eq!(sides.right, Length::Px(7.5));
    assert_eq!(sides.bottom, Length::Px(7.5));
    assert_eq!(sides.left, Length::Px(7.5));
}

#[test]
fn padding_shorthand_five_values_dropped_by_leftover() {
    // 5 個目以降は本 helper が consume せず leftover として残す。
    // parse_value 単体では 4-value form の Some を返すが、caller (rule.rs)
    // の expect_exhausted が leftover を検知して declaration drop するので、
    // rule.rs 経由で drop 確認。
    let source = "padding: 10px 20px 30px 40px 50px;";
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let decls = crate::rule::parse_declaration_block(&mut parser);
    assert!(
        decls.is_empty(),
        "5-value form must be dropped by expect_exhausted"
    );
}

#[test]
fn line_height_accepts_zero_number_and_length() {
    // spec `[0,∞]`: 0 は境界の valid value。
    // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths> clause 2:
    // "if a 0 could be parsed as either a `<number>` or a `<length>` in a
    // property (such as line-height), it must parse as a `<number>`" —
    // parse_line_height は expect_number branch を parse_length_value より
    // 先に試すため、bare `0` は LineHeight::Number(0.0) として確定 (unitless-zero
    // clause の Length 経路が導入した Px(0.0) route ではない)。
    assert_eq!(
        parse("0", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Number(0.0)))
    );
    assert_eq!(
        parse("0px", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Px(
            0.0
        ))))
    );
}

#[test]
fn padding_case_insensitive_unit() {
    // CSS spec: unit identifier は ASCII case-insensitive。
    assert_eq!(
        parse("10PX", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Px(10.0)))
    );
    assert_eq!(
        parse("2EM", "padding-bottom"),
        Some(PropertyValue::PaddingBottom(Length::Em(2.0)))
    );
}

#[test]
fn line_height_rejects_negative_number() {
    // Verification 7: `<number [0,∞]>` — 負値は spec grammar 違反 → drop。
    assert_eq!(parse("-1.5", "line-height"), None);
}

#[test]
fn line_height_rejects_negative_number_with_trailing_length() {
    // Regression: Number branch は
    // Token::Number を commit した後 fallthrough すべきでない。fallthrough
    // していた旧実装では `-0.5 20px` が Length branch で `20px` を拾い
    // silently accept されていた (spec-invalid → 本来 declaration drop)。
    // 現行: Number 到達 = 確定、`[0,∞]` 違反は declaration drop、
    // 後続 token は expect_exhausted なくとも parse_length_value 側で拾わない。
    assert_eq!(parse("-0.5 20px", "line-height"), None);
    // 対称: negative number + em / % も同じく drop。
    assert_eq!(parse("-0.5 1em", "line-height"), None);
    assert_eq!(parse("-1.0 50%", "line-height"), None);
}

#[test]
fn line_height_rejects_negative_length() {
    // Verification 7: `<length-percentage [0,∞]>` — 負 length は drop。
    assert_eq!(parse("-10px", "line-height"), None);
    assert_eq!(parse("-1em", "line-height"), None);
}

#[test]
fn line_height_rejects_negative_percentage() {
    // Verification 7: 負 percentage も spec `[0,∞]` 違反 → drop。
    assert_eq!(parse("-50%", "line-height"), None);
}

#[test]
fn line_height_rejects_unknown_keyword() {
    // spec grammar 外の ident (`auto` / `medium` 等) は (a) spec-invalid、
    // silent drop。CSS-wide keyword は別 test
    // (`line_height_rejects_css_wide_keyword`) — (a) ではなく (b) の
    // 非対応なので混同しないこと。
    assert_eq!(parse("auto", "line-height"), None);
    assert_eq!(parse("medium", "line-height"), None);
}

#[test]
fn line_height_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
    // canonical: PropertyValue doc「CSS-wide keyword」節。
    assert_eq!(parse("inherit", "line-height"), None);
    assert_eq!(parse("initial", "line-height"), None);
    assert_eq!(parse("unset", "line-height"), None);
    assert_eq!(parse("revert", "line-height"), None);
    assert_eq!(parse("revert-layer", "line-height"), None);
}

#[test]
fn line_height_rejects_unsupported_unit() {
    // parse_length_value が silent drop する unit (`vw` / `cap` 等、
    // 現状未対応) は helper 側で `None` →
    // line-height parse も declaration drop。`ch` / `lh` / `rlh` は
    // それぞれ受理側へ移った
    // (`line_height_accepts_ch` / `line_height_accepts_lh` 参照)。
    assert_eq!(parse("10vw", "line-height"), None);
    assert_eq!(parse("10cap", "line-height"), None);
}

#[test]
fn line_height_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`.
    // `line-height` itself is a valid context for `lh`/`rlh` at parse
    // time (unlike `font-size`, which `parse_font_size` post-filters —
    // see that function's doc for why) — the self-reference resolve
    // basis is handled downstream in `crate::resolve::resolve_line_height`.
    assert_eq!(
        parse("10lh", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Lh(
            10.0
        ))))
    );
    assert_eq!(
        parse("1rlh", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Rlh(
            1.0
        ))))
    );
}

#[test]
fn line_height_accepts_ch() {
    assert_eq!(
        parse("2ch", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Length(Length::Ch(
            2.0
        ))))
    );
}

#[test]
fn line_height_normal_is_case_insensitive() {
    // CSS spec: keyword ident は ASCII case-insensitive
    // (expect_ident_matching が case-insensitive)。
    assert_eq!(
        parse("NORMAL", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Normal))
    );
    assert_eq!(
        parse("Normal", "line-height"),
        Some(PropertyValue::LineHeight(LineHeight::Normal))
    );
}

#[test]
fn line_height_key_maps_to_line_height_property_key() {
    // PropertyValue::LineHeight → PropertyKey::LineHeight (cascade winner 選択の
    // discriminant integrity、既存 sibling font_size / display と同じ pattern)。
    let v = PropertyValue::LineHeight(LineHeight::Normal);
    assert_eq!(v.key(), PropertyKey::LineHeight);
    let v = PropertyValue::LineHeight(LineHeight::Number(1.5));
    assert_eq!(v.key(), PropertyKey::LineHeight);
    let v = PropertyValue::LineHeight(LineHeight::Length(Length::Px(24.0)));
    assert_eq!(v.key(), PropertyKey::LineHeight);
}

#[test]
fn position_key_maps_to_position_property_key() {
    // PropertyValue::Position → PropertyKey::Position (cascade winner 選択の
    // discriminant integrity、既存 sibling counter-* / content / string-set と
    // 同じ pattern)。
    let v = PropertyValue::Position(PositionValue::Static);
    assert_eq!(v.key(), PropertyKey::Position);
    let v = PropertyValue::Position(PositionValue::Running(SmolStr::new("hdr")));
    assert_eq!(v.key(), PropertyKey::Position);
}

// ── text-align (CSS Text 3 §6.1) ──
//
// Value grammar (§6.1 spec verbatim):
//   start | end | left | right | center | justify | match-parent | justify-all
// Initial: start / Inherited: yes / spec 上 shorthand (text-align-all +
// text-align-last、単一 field で保持 = (b)
// 非対応)。inheritance test は cascade.rs 側 (parent → child コピー、display
// non-inherited との対比)。

#[test]
fn text_align_parse_all_eight_keywords() {
    // Verification 5: 8 keyword が全て正しく TextAlign variant にマップされる。
    // 1 test で全 arm coverage (patch coverage 100% 目標)。
    assert_eq!(
        parse("start", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Start))
    );
    assert_eq!(
        parse("end", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::End))
    );
    assert_eq!(
        parse("left", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Left))
    );
    assert_eq!(
        parse("right", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Right))
    );
    assert_eq!(
        parse("center", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Center))
    );
    assert_eq!(
        parse("justify", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Justify))
    );
    assert_eq!(
        parse("match-parent", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::MatchParent))
    );
    assert_eq!(
        parse("justify-all", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::JustifyAll))
    );
}

#[test]
fn text_align_is_case_insensitive() {
    // Verification 6: CSS spec 慣行 — property value keyword は ASCII case-insensitive。
    assert_eq!(
        parse("CENTER", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::Center))
    );
    assert_eq!(
        parse("Justify-All", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::JustifyAll))
    );
    assert_eq!(
        parse("Match-Parent", "text-align"),
        Some(PropertyValue::TextAlign(TextAlign::MatchParent))
    );
}

#[test]
fn text_align_rejects_unknown_keyword() {
    // spec §6.1 grammar に含まれない keyword は silent drop (spec-invalid)。
    // `middle` は typo/俗称、`text-align` spec に存在しない。
    assert_eq!(parse("middle", "text-align"), None);
    assert_eq!(parse("baseline", "text-align"), None);
    assert_eq!(parse("top", "text-align"), None);
}

#[test]
fn text_align_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。5 keyword
    // の一覧・理由は `PropertyValue` doc の「CSS-wide keyword」節が canonical。
    assert_eq!(parse("inherit", "text-align"), None);
    assert_eq!(parse("initial", "text-align"), None);
    assert_eq!(parse("unset", "text-align"), None);
    assert_eq!(parse("revert", "text-align"), None);
    assert_eq!(parse("revert-layer", "text-align"), None);
}

#[test]
fn text_align_rejects_string_value() {
    // CSS Text 3 §6.1 grammar は 8 keyword のみ、`<string>` value は本 crate
    // が引用する level では未定義 → spec-invalid、silent drop。
    // (Text 4 draft では tabular-data character alignment 用に `<string>` が
    // 検討されているが本 crate は Text 3 pin。expect_ident が String token を
    // reject する経路で `None` を返す。)
    assert_eq!(parse(r#""." "#, "text-align"), None);
}

#[test]
fn text_align_rejects_non_ident() {
    // Number / dimension token は expect_ident で reject。
    assert_eq!(parse("16px", "text-align"), None);
    assert_eq!(parse("100", "text-align"), None);
}

#[test]
fn text_align_key_maps_to_text_align_property_key() {
    // PropertyValue::TextAlign → PropertyKey::TextAlign (cascade winner 選択の
    // discriminant integrity、既存 sibling counter-* / content / string-set /
    // position と同じ pattern)。
    let v = PropertyValue::TextAlign(TextAlign::Start);
    assert_eq!(v.key(), PropertyKey::TextAlign);
    let v = PropertyValue::TextAlign(TextAlign::Center);
    assert_eq!(v.key(), PropertyKey::TextAlign);
}

// ── text-indent (CSS Text 3 §8.1) ──
//
// Full value grammar: `<length-percentage> && hanging? && each-line?` —
// this crate implements only the `<length-percentage>` component.
// Initial: 0 / Applies to: block containers / Inherited: yes /
// Percentages: refers to block container's own inline-axis inner size /
// Computed value: computed <length-percentage> value, plus any
// specified keywords.

#[test]
fn text_indent_parse_px() {
    assert_eq!(
        parse("20px", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: Length::Px(20.0),
            hanging: false,
            each_line: false
        }))
    );
}

#[test]
fn text_indent_parse_percentage() {
    assert_eq!(
        parse("10%", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: Length::Percent(10.0),
            hanging: false,
            each_line: false
        }))
    );
}

#[test]
fn text_indent_parse_em() {
    assert_eq!(
        parse("2em", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: Length::Em(2.0),
            hanging: false,
            each_line: false
        }))
    );
}

#[test]
fn text_indent_accepts_negative_length() {
    // CSS Text 3 §8.1 places no `[0,∞]` restriction on this grammar
    // (unlike `padding-top` — `PropertyValue::TextIndent` doc).
    assert_eq!(
        parse("-2em", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: Length::Em(-2.0),
            hanging: false,
            each_line: false
        }))
    );
}

#[test]
fn text_indent_accepts_zero() {
    // CSS Values 3 §5 unitless-zero clause — bare `0` is a valid `<length>`.
    assert_eq!(
        parse("0", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: Length::Px(0.0),
            hanging: false,
            each_line: false
        }))
    );
}

#[test]
fn text_indent_parse_hanging() {
    // CSS Text 3 §8.1 `hanging` keyword is kept, not dropped.
    assert_eq!(
        parse("2em hanging", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: Length::Em(2.0),
            hanging: true,
            each_line: false,
        }))
    );
}

#[test]
fn text_indent_parse_each_line() {
    assert_eq!(
        parse("each-line 2em", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: Length::Em(2.0),
            hanging: false,
            each_line: true,
        }))
    );
}

#[test]
fn text_indent_parse_hanging_each_line_combined() {
    // Order-independent `&&`: flags may precede the length.
    assert_eq!(
        parse("hanging each-line 2em", "text-indent"),
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: Length::Em(2.0),
            hanging: true,
            each_line: true,
        }))
    );
}

#[test]
fn text_wrap_parse_nowrap() {
    assert_eq!(
        parse("nowrap", "text-wrap"),
        Some(PropertyValue::TextWrap(TextWrapMode::Nowrap))
    );
}

#[test]
fn text_wrap_parse_wrap() {
    assert_eq!(
        parse("wrap", "text-wrap"),
        Some(PropertyValue::TextWrap(TextWrapMode::Wrap))
    );
}

#[test]
fn text_wrap_rejects_balance() {
    // Full shorthand (wrap-style) is deferred — see the TextWrapMode doc.
    assert_eq!(parse("balance", "text-wrap"), None);
}

#[test]
fn text_indent_rejects_unsupported_unit() {
    // `cap` (CSS Values 4 §6.1.1) is not implemented — dropped by
    // `parse_length_value`'s `Token::Dimension` fall-through, same as the
    // `margin_side_rejects_unsupported_unit` sibling.
    assert_eq!(parse("1cap", "text-indent"), None);
}

#[test]
fn text_indent_rejects_auto() {
    // Unlike `margin` / `width`, `text-indent`'s grammar has no `auto`
    // alternative — the `hanging`/`each-line` keywords are the only
    // idents the full grammar accepts. `parse_length_value` reads one
    // token via `input.next()` and only has match arms for
    // `Token::Dimension` / `Token::Percentage` / a zero `Token::Number` —
    // an `Ident` token (`auto` included) matches none of them and falls
    // through to the trailing `_ => None`.
    assert_eq!(parse("auto", "text-indent"), None);
}

#[test]
fn text_indent_key_maps_to_text_indent_property_key() {
    let v = PropertyValue::TextIndent(TextIndentValue {
        length: Length::Px(20.0),
        hanging: false,
        each_line: false,
    });
    assert_eq!(v.key(), PropertyKey::TextIndent);
}

// ── direction (CSS Writing Modes 4 §2.1) ──
//
// Value grammar (§2.1 spec verbatim): ltr | rtl
// Initial: ltr / Inherited: yes / Computed value: specified value。

#[test]
fn direction_parse_both_keywords() {
    assert_eq!(
        parse("ltr", "direction"),
        Some(PropertyValue::Direction(Direction::Ltr))
    );
    assert_eq!(
        parse("rtl", "direction"),
        Some(PropertyValue::Direction(Direction::Rtl))
    );
}

#[test]
fn direction_is_case_insensitive() {
    assert_eq!(
        parse("LTR", "direction"),
        Some(PropertyValue::Direction(Direction::Ltr))
    );
    assert_eq!(
        parse("Rtl", "direction"),
        Some(PropertyValue::Direction(Direction::Rtl))
    );
}

#[test]
fn direction_rejects_unknown_keyword() {
    // 旧 draft 相当の `auto` は現行 §2.1 grammar に無い — 実 spec-invalid。
    assert_eq!(parse("auto", "direction"), None);
    assert_eq!(parse("horizontal-tb", "direction"), None);
}

#[test]
fn direction_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。5 keyword
    // の一覧・理由は `PropertyValue` doc の「CSS-wide keyword」節が canonical。
    assert_eq!(parse("inherit", "direction"), None);
    assert_eq!(parse("initial", "direction"), None);
    assert_eq!(parse("unset", "direction"), None);
    assert_eq!(parse("revert", "direction"), None);
    assert_eq!(parse("revert-layer", "direction"), None);
}

#[test]
fn direction_rejects_non_ident() {
    assert_eq!(parse("16px", "direction"), None);
    assert_eq!(parse(r#""ltr""#, "direction"), None);
}

#[test]
fn direction_key_maps_to_direction_property_key() {
    let v = PropertyValue::Direction(Direction::Ltr);
    assert_eq!(v.key(), PropertyKey::Direction);
    let v = PropertyValue::Direction(Direction::Rtl);
    assert_eq!(v.key(), PropertyKey::Direction);
}

// ── writing-mode (CSS Writing Modes 4 §3.2) ──
//
// Value grammar (§3.2 spec verbatim): horizontal-tb | vertical-rl |
// vertical-lr | sideways-rl | sideways-lr
// Initial: horizontal-tb / Inherited: yes / Computed value: specified
// value — this crate deliberately diverges from the last clause for the
// 4 non-`horizontal-tb` keywords (`WritingMode` doc's Non-goal section,
// `resolve_writing_mode` tests below).

#[test]
fn writing_mode_parse_all_five_keywords() {
    // All 5 keywords parse successfully (`Some`, not `None`) — this
    // crate accepts the full spec grammar even though the 4
    // non-`horizontal-tb` keywords later collapse at the computed layer
    // (`resolve_writing_mode`, not this parser).
    assert_eq!(
        parse("horizontal-tb", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::HorizontalTb))
    );
    assert_eq!(
        parse("vertical-rl", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::VerticalRl))
    );
    assert_eq!(
        parse("vertical-lr", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::VerticalLr))
    );
    assert_eq!(
        parse("sideways-rl", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::SidewaysRl))
    );
    assert_eq!(
        parse("sideways-lr", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::SidewaysLr))
    );
}

#[test]
fn writing_mode_is_case_insensitive() {
    assert_eq!(
        parse("HORIZONTAL-TB", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::HorizontalTb))
    );
    assert_eq!(
        parse("Vertical-Rl", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::VerticalRl))
    );
    assert_eq!(
        parse("SIDEWAYS-LR", "writing-mode"),
        Some(PropertyValue::WritingMode(WritingMode::SidewaysLr))
    );
}

#[test]
fn writing_mode_rejects_unknown_keyword() {
    assert_eq!(parse("auto", "writing-mode"), None);
    // `ltr`/`rtl` are `direction`'s keywords, not `writing-mode`'s.
    assert_eq!(parse("ltr", "writing-mode"), None);
}

#[test]
fn writing_mode_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。5
    // keyword の一覧・理由は `PropertyValue` doc の「CSS-wide keyword」節が
    // canonical。
    assert_eq!(parse("inherit", "writing-mode"), None);
    assert_eq!(parse("initial", "writing-mode"), None);
    assert_eq!(parse("unset", "writing-mode"), None);
    assert_eq!(parse("revert", "writing-mode"), None);
    assert_eq!(parse("revert-layer", "writing-mode"), None);
}

#[test]
fn writing_mode_rejects_non_ident() {
    assert_eq!(parse("16px", "writing-mode"), None);
    assert_eq!(parse(r#""vertical-rl""#, "writing-mode"), None);
}

#[test]
fn writing_mode_key_maps_to_writing_mode_property_key() {
    let v = PropertyValue::WritingMode(WritingMode::HorizontalTb);
    assert_eq!(v.key(), PropertyKey::WritingMode);
    let v = PropertyValue::WritingMode(WritingMode::VerticalRl);
    assert_eq!(v.key(), PropertyKey::WritingMode);
}

/// [`resolve_writing_mode`]'s whole reason to exist — every one of the 5
/// spec keywords collapses to [`WritingMode::HorizontalTb`], not just the
/// 4 non-horizontal ones (identity for `HorizontalTb` itself is also
/// pinned so a future refactor can't "fix" this into a no-op passthrough
/// without a test noticing).
///
/// Future work: vertical writing-mode 実装時に本 collapse を
/// 削除し、本 test を revert/rewrite すること。
#[test]
fn resolve_writing_mode_collapses_all_five_keywords_to_horizontal_tb() {
    for specified in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
        WritingMode::SidewaysRl,
        WritingMode::SidewaysLr,
    ] {
        assert_eq!(resolve_writing_mode(specified), WritingMode::HorizontalTb);
    }
}

// ── overflow-x / overflow-y / overflow (CSS Overflow 3 §3.1) ──
//
// Value grammar (§3.1 spec verbatim): visible | hidden | clip | scroll |
// auto. Initial: visible / Inherited: no. `overflow` shorthand grammar:
// `<'overflow-block'>{1,2}` (mapped to physical x/y — see `OverflowValue`
// doc's Non-goal note).

#[test]
fn overflow_x_parse_all_five_keywords() {
    assert_eq!(
        parse("visible", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Visible))
    );
    assert_eq!(
        parse("hidden", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Hidden))
    );
    assert_eq!(
        parse("clip", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Clip))
    );
    assert_eq!(
        parse("scroll", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Scroll))
    );
    assert_eq!(
        parse("auto", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Auto))
    );
}

#[test]
fn overflow_y_parse_all_five_keywords() {
    // Sibling of `overflow_x_parse_all_five_keywords` — same grammar,
    // separate `PropertyValue` variant / `PropertyKey`.
    assert_eq!(
        parse("visible", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Visible))
    );
    assert_eq!(
        parse("hidden", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Hidden))
    );
    assert_eq!(
        parse("clip", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Clip))
    );
    assert_eq!(
        parse("scroll", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Scroll))
    );
    assert_eq!(
        parse("auto", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Auto))
    );
}

#[test]
fn overflow_is_case_insensitive() {
    assert_eq!(
        parse("HIDDEN", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Hidden))
    );
    assert_eq!(
        parse("Auto", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Auto))
    );
}

#[test]
fn overflow_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "overflow-x"), None);
    assert_eq!(parse("collapse", "overflow-y"), None);
    // `padding-box` etc. are not part of this property's grammar.
    assert_eq!(parse("padding-box", "overflow-x"), None);
}

#[test]
fn overflow_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "overflow-x"), None);
        assert_eq!(parse(kw, "overflow-y"), None);
        assert_eq!(parse(kw, "overflow"), None);
    }
}

#[test]
fn overflow_rejects_non_ident() {
    assert_eq!(parse("16px", "overflow-x"), None);
    assert_eq!(parse(r#""hidden""#, "overflow-y"), None);
}

#[test]
fn overflow_x_key_maps_to_overflow_x_property_key() {
    let v = PropertyValue::OverflowX(OverflowValue::Hidden);
    assert_eq!(v.key(), PropertyKey::OverflowX);
}

#[test]
fn overflow_y_key_maps_to_overflow_y_property_key() {
    let v = PropertyValue::OverflowY(OverflowValue::Scroll);
    assert_eq!(v.key(), PropertyKey::OverflowY);
}

#[test]
fn overflow_key_maps_to_overflow_property_key() {
    let v = PropertyValue::Overflow(OverflowXY::both(OverflowValue::Auto));
    assert_eq!(v.key(), PropertyKey::Overflow);
}

#[test]
fn overflow_shorthand_one_value_spreads_to_both_axes() {
    // §3.1 "If there is only one component value, it applies to all
    // sides" (paraphrase of the shared `<'overflow-block'>{1,2}`
    // expansion rule this crate maps onto physical x/y).
    assert_eq!(
        parse("hidden", "overflow"),
        Some(PropertyValue::Overflow(OverflowXY::both(
            OverflowValue::Hidden
        )))
    );
}

#[test]
fn overflow_shorthand_two_values_set_x_then_y() {
    // §3.1 verbatim: "The overflow property is a shorthand property that
    // sets the specified values of overflow-x and overflow-y in that
    // order."
    assert_eq!(
        parse("hidden scroll", "overflow"),
        Some(PropertyValue::Overflow(OverflowXY {
            x: OverflowValue::Hidden,
            y: OverflowValue::Scroll,
        }))
    );
}

#[test]
fn overflow_shorthand_leaves_extra_values_for_caller_exhausted_check() {
    // Mirrors `margin_shorthand_leaves_extra_values_for_caller_exhausted_check`
    // — this helper consumes only 2 values; a 3rd is left unconsumed for
    // the `expect_exhausted` caller in `rule.rs` to reject the whole
    // declaration. `parse_value` itself does not call `expect_exhausted`,
    // so this direct call only demonstrates the helper's own consumption,
    // not the end-to-end drop (that is `rule.rs`'s job, pinned by
    // `rule::tests::overflow_shorthand_three_values_declaration_dropped`).
    assert_eq!(
        parse("hidden scroll auto", "overflow"),
        Some(PropertyValue::Overflow(OverflowXY {
            x: OverflowValue::Hidden,
            y: OverflowValue::Scroll,
        }))
    );
}

// ── text-decoration-line (CSS Text Decoration Module Level 3 §2.1) ──

#[test]
fn text_decoration_line_parses_none() {
    assert_eq!(
        parse("none", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine::NONE))
    );
}

#[test]
fn text_decoration_line_parses_each_single_keyword() {
    assert_eq!(
        parse("underline", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::UNDERLINE
        ))
    );
    assert_eq!(
        parse("overline", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::OVERLINE
        ))
    );
    assert_eq!(
        parse("line-through", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::LINE_THROUGH
        ))
    );
    assert_eq!(
        parse("blink", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine::BLINK))
    );
}

#[test]
fn text_decoration_line_parses_combination_in_any_order() {
    // `||` grammar: order-independent. Both orderings of the same pair
    // must produce the same flag set.
    assert_eq!(
        parse("underline overline", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine {
            underline: true,
            overline: true,
            line_through: false,
            blink: false,
            spelling_error: false,
            grammar_error: false,
        }))
    );
    assert_eq!(
        parse("overline underline", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine {
            underline: true,
            overline: true,
            line_through: false,
            blink: false,
            spelling_error: false,
            grammar_error: false,
        }))
    );
}

#[test]
fn text_decoration_line_parses_spelling_and_grammar_error_alone() {
    // CSS Text Decoration 4 §2.1: `spelling-error` / `grammar-error`
    // are top-level alternatives, each accepted only on its own.
    assert_eq!(
        parse("spelling-error", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::SPELLING_ERROR
        ))
    );
    assert_eq!(
        parse("grammar-error", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::GRAMMAR_ERROR
        ))
    );
}

#[test]
fn text_decoration_line_rejects_spelling_error_combined() {
    // Same grammar: neither combines with the `||` group nor with
    // each other — leftover makes the caller drop the declaration.
    assert_eq!(
        parse_entire("underline spelling-error", "text-decoration-line"),
        None
    );
    assert_eq!(
        parse_entire("spelling-error underline", "text-decoration-line"),
        None
    );
    assert_eq!(
        parse_entire("spelling-error grammar-error", "text-decoration-line"),
        None
    );
}

#[test]
fn text_decoration_line_parses_all_four_combined() {
    assert_eq!(
        parse(
            "underline overline line-through blink",
            "text-decoration-line"
        ),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine {
            underline: true,
            overline: true,
            line_through: true,
            blink: true,
            spelling_error: false,
            grammar_error: false,
        }))
    );
}

#[test]
fn text_decoration_line_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(TextDecorationLine::NONE))
    );
    assert_eq!(
        parse("Underline", "text-decoration-line"),
        Some(PropertyValue::TextDecorationLine(
            TextDecorationLine::UNDERLINE
        ))
    );
}

#[test]
fn text_decoration_line_two_underlines_leaves_leftover_for_caller_exhausted_check() {
    // Each `||` component at most once (CSS Values 4 §2.2). The 2nd
    // `underline` is left unconsumed by `parse_text_decoration_line`
    // (its flag is already set) — `parse_border_shorthand`'s sibling
    // `border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check`
    // test pattern: the helper itself still returns `Some` (1st token
    // consumed), and rejection is the caller's (`rule.rs`'s
    // `DeclParser::parse_value`'s `expect_exhausted`) responsibility —
    // pinned end-to-end by `rule.rs`'s
    // `text_decoration_line_duplicate_and_none_combination_declarations_dropped`
    // sibling test.
    let mut input = ParserInput::new("underline underline");
    let mut parser = Parser::new(&mut input);
    assert_eq!(
        parse_text_decoration_line(&mut parser),
        Some(TextDecorationLine::UNDERLINE)
    );
    assert!(!parser.is_exhausted());
}

#[test]
fn text_decoration_line_none_combined_with_a_keyword_leaves_leftover() {
    // `none | [ ... ]` — `none` is a separate top-level alternative, not
    // a member of the `||` combination, so it cannot co-occur with the
    // other keywords in either order. Same "helper returns `Some`,
    // leftover is the caller's `expect_exhausted` responsibility" shape
    // as the duplicate-keyword sibling test above.
    let mut input = ParserInput::new("none underline");
    let mut parser = Parser::new(&mut input);
    assert_eq!(
        parse_text_decoration_line(&mut parser),
        Some(TextDecorationLine::NONE)
    );
    assert!(!parser.is_exhausted());

    let mut input = ParserInput::new("underline none");
    let mut parser = Parser::new(&mut input);
    assert_eq!(
        parse_text_decoration_line(&mut parser),
        Some(TextDecorationLine::UNDERLINE)
    );
    assert!(!parser.is_exhausted());
}

#[test]
fn text_decoration_line_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "text-decoration-line"), None);
}

#[test]
fn text_decoration_line_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "text-decoration-line"), None);
    }
}

#[test]
fn text_decoration_line_rejects_non_ident() {
    assert_eq!(parse("16px", "text-decoration-line"), None);
    assert_eq!(parse(r#""underline""#, "text-decoration-line"), None);
}

// ── text-decoration-style (CSS Text Decoration Module Level 3 §2.2) ──

#[test]
fn text_decoration_style_parses_all_five_keywords() {
    for (kw, expected) in [
        ("solid", TextDecorationStyle::Solid),
        ("double", TextDecorationStyle::Double),
        ("dotted", TextDecorationStyle::Dotted),
        ("dashed", TextDecorationStyle::Dashed),
        ("wavy", TextDecorationStyle::Wavy),
    ] {
        assert_eq!(
            parse(kw, "text-decoration-style"),
            Some(PropertyValue::TextDecorationStyle(expected))
        );
    }
}

#[test]
fn text_decoration_style_is_case_insensitive() {
    assert_eq!(
        parse("WAVY", "text-decoration-style"),
        Some(PropertyValue::TextDecorationStyle(
            TextDecorationStyle::Wavy
        ))
    );
}

#[test]
fn text_decoration_style_rejects_unknown_keyword() {
    // `underline` is a `text-decoration-line` keyword, not a
    // `text-decoration-style` one — the two properties' keyword sets are
    // disjoint.
    assert_eq!(parse("underline", "text-decoration-style"), None);
    assert_eq!(parse("bogus", "text-decoration-style"), None);
}

// ── text-decoration-color (CSS Text Decoration Module Level 3 §2.3) ──

#[test]
fn text_decoration_color_parses_currentcolor() {
    assert_eq!(
        parse("currentcolor", "text-decoration-color"),
        Some(PropertyValue::TextDecorationColor(
            TextDecorationColor::CurrentColor
        ))
    );
    assert_eq!(
        parse("CurrentColor", "text-decoration-color"),
        Some(PropertyValue::TextDecorationColor(
            TextDecorationColor::CurrentColor
        ))
    );
}

#[test]
fn text_decoration_color_parses_resolved_color() {
    assert_eq!(
        parse("red", "text-decoration-color"),
        Some(PropertyValue::TextDecorationColor(
            TextDecorationColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            })
        ))
    );
    assert_eq!(
        parse("#00ff00", "text-decoration-color"),
        Some(PropertyValue::TextDecorationColor(
            TextDecorationColor::Resolved(CssColor {
                r: 0,
                g: 255,
                b: 0,
                a: 255,
            })
        ))
    );
}

#[test]
fn text_decoration_color_rejects_unknown_ident() {
    assert_eq!(parse("bogus", "text-decoration-color"), None);
}

// ── text-decoration shorthand (CSS Text Decoration Module Level 3
// §2.4) ──
//
// `<'text-decoration-line'> || <'text-decoration-style'> ||
// <'text-decoration-color'>`. Omitted components fill with their
// longhand's initial value (verbatim: "Omitted values are set to their
// initial values.").

#[test]
fn text_decoration_shorthand_line_only_fills_the_rest_with_initial() {
    assert_eq!(
        parse("underline", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::Auto,
        }))
    );
}

#[test]
fn text_decoration_shorthand_style_only_fills_the_rest_with_initial() {
    // A bare style keyword is a spec-valid shorthand value under `||`
    // (`text-decoration: wavy;`) — this was previously unreachable
    // (pre-longhand-decomposition `text-decoration` only accepted
    // `none`/`underline`).
    assert_eq!(
        parse("wavy", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::NONE,
            style: TextDecorationStyle::Wavy,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::Auto,
        }))
    );
}

#[test]
fn text_decoration_shorthand_color_only_fills_the_rest_with_initial() {
    // Likewise a bare color (`text-decoration: red;`) was rejected
    // wholesale pre-decomposition; `||` makes it valid on its own.
    assert_eq!(
        parse("red", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::NONE,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
            thickness: TextDecorationThickness::Auto,
        }))
    );
}

#[test]
fn text_decoration_shorthand_parses_all_four_in_any_order() {
    let expected = Some(PropertyValue::TextDecoration(TextDecorationShorthand {
        line: TextDecorationLine::UNDERLINE,
        style: TextDecorationStyle::Wavy,
        color: TextDecorationColor::Resolved(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
        thickness: TextDecorationThickness::Auto,
    }));
    assert_eq!(parse("underline wavy red", "text-decoration"), expected);
    assert_eq!(parse("red wavy underline", "text-decoration"), expected);
    assert_eq!(parse("wavy red underline", "text-decoration"), expected);
}

#[test]
fn text_decoration_shorthand_line_combination_plus_style_and_color() {
    assert_eq!(
        parse("underline overline wavy red", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine {
                underline: true,
                overline: true,
                line_through: false,
                blink: false,
                spelling_error: false,
                grammar_error: false,
            },
            style: TextDecorationStyle::Wavy,
            color: TextDecorationColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
            thickness: TextDecorationThickness::Auto,
        }))
    );
}

#[test]
fn text_decoration_shorthand_thickness_only_fills_the_rest_with_initial() {
    assert_eq!(
        parse("from-font", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::NONE,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::FromFont,
        }))
    );
}

#[test]
fn text_decoration_shorthand_thickness_combines_with_other_components() {
    // ED §2.6: thickness participates in the `||` loop like the other
    // 3 components (WPT `text-decoration-shorthand.html` maps
    // `overline from-font dotted green` to all 4 longhands).
    assert_eq!(
        parse("overline from-font dotted green", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::OVERLINE,
            style: TextDecorationStyle::Dotted,
            color: TextDecorationColor::Resolved(CssColor {
                r: 0,
                g: 128,
                b: 0,
                a: 255,
            }),
            thickness: TextDecorationThickness::FromFont,
        }))
    );
}

#[test]
fn text_decoration_shorthand_length_thickness_combines() {
    assert_eq!(
        parse("line-through 20px", "text-decoration"),
        Some(PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::LINE_THROUGH,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::Length(Length::Px(20.0)),
        }))
    );
}

// ── text-decoration-skip-ink (ED §2.10.4) ──

#[test]
fn text_decoration_skip_ink_parses_all_three_keywords() {
    assert_eq!(
        parse("auto", "text-decoration-skip-ink"),
        Some(PropertyValue::TextDecorationSkipInk(
            TextDecorationSkipInk::Auto
        ))
    );
    assert_eq!(
        parse("none", "text-decoration-skip-ink"),
        Some(PropertyValue::TextDecorationSkipInk(
            TextDecorationSkipInk::None
        ))
    );
    assert_eq!(
        parse("all", "text-decoration-skip-ink"),
        Some(PropertyValue::TextDecorationSkipInk(
            TextDecorationSkipInk::All
        ))
    );
    assert_eq!(parse_entire("auto none", "text-decoration-skip-ink"), None);
    assert_eq!(parse_entire("bogus", "text-decoration-skip-ink"), None);
}

#[test]
fn text_decoration_skip_ink_key_maps_to_property_key() {
    let v = PropertyValue::TextDecorationSkipInk(TextDecorationSkipInk::Auto);
    assert_eq!(v.key(), PropertyKey::TextDecorationSkipInk);
}

// ── text-decoration-skip-spaces (ED §2.10.3) ──

#[test]
fn text_decoration_skip_spaces_parses_all_forms() {
    assert_eq!(
        parse("none", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::None
        ))
    );
    assert_eq!(
        parse("all", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::All
        ))
    );
    assert_eq!(
        parse("start", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::Start
        ))
    );
    assert_eq!(
        parse("end", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::End
        ))
    );
    // `||` order-independence, both orders map to `StartEnd`.
    assert_eq!(
        parse("start end", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::StartEnd
        ))
    );
    assert_eq!(
        parse("end start", "text-decoration-skip-spaces"),
        Some(PropertyValue::TextDecorationSkipSpaces(
            TextDecorationSkipSpaces::StartEnd
        ))
    );
    assert_eq!(
        parse_entire("none start", "text-decoration-skip-spaces"),
        None
    );
    assert_eq!(
        parse_entire("start start", "text-decoration-skip-spaces"),
        None
    );
    assert_eq!(parse_entire("bogus", "text-decoration-skip-spaces"), None);
}

#[test]
fn text_decoration_skip_spaces_key_maps_to_property_key() {
    let v = PropertyValue::TextDecorationSkipSpaces(TextDecorationSkipSpaces::StartEnd);
    assert_eq!(v.key(), PropertyKey::TextDecorationSkipSpaces);
}

// ── text-decoration-thickness (ED §2.4.1) ──

#[test]
fn text_decoration_thickness_parses_all_forms() {
    assert_eq!(
        parse("auto", "text-decoration-thickness"),
        Some(PropertyValue::TextDecorationThickness(
            TextDecorationThickness::Auto
        ))
    );
    assert_eq!(
        parse("from-font", "text-decoration-thickness"),
        Some(PropertyValue::TextDecorationThickness(
            TextDecorationThickness::FromFont
        ))
    );
    assert_eq!(
        parse("3em", "text-decoration-thickness"),
        Some(PropertyValue::TextDecorationThickness(
            TextDecorationThickness::Length(Length::Em(3.0))
        ))
    );
    assert_eq!(
        parse("50%", "text-decoration-thickness"),
        Some(PropertyValue::TextDecorationThickness(
            TextDecorationThickness::Length(Length::Percent(50.0))
        ))
    );
    // `<line-width>` keywords are out of scope (dropped).
    assert_eq!(parse_entire("thin", "text-decoration-thickness"), None);
    assert_eq!(parse_entire("medium", "text-decoration-thickness"), None);
}

#[test]
fn text_decoration_thickness_key_maps_to_property_key() {
    let v = PropertyValue::TextDecorationThickness(TextDecorationThickness::Auto);
    assert_eq!(v.key(), PropertyKey::TextDecorationThickness);
}

// ── text-decoration-inset (ED §2.9.1) ──

#[test]
fn text_decoration_inset_parses_auto_and_one_or_two_lengths() {
    assert_eq!(
        parse("auto", "text-decoration-inset"),
        Some(PropertyValue::TextDecorationInset(
            TextDecorationInset::Auto
        ))
    );
    assert_eq!(
        parse("-1em", "text-decoration-inset"),
        Some(PropertyValue::TextDecorationInset(
            TextDecorationInset::Lengths {
                start: Length::Em(-1.0),
                end: Length::Em(-1.0),
            }
        ))
    );
    assert_eq!(
        parse("1px 2px", "text-decoration-inset"),
        Some(PropertyValue::TextDecorationInset(
            TextDecorationInset::Lengths {
                start: Length::Px(1.0),
                end: Length::Px(2.0),
            }
        ))
    );
    // `<percentage>` is rejected (WPT ground truth over ED grammar).
    assert_eq!(parse_entire("10%", "text-decoration-inset"), None);
    assert_eq!(parse_entire("none", "text-decoration-inset"), None);
    assert_eq!(parse_entire("auto auto", "text-decoration-inset"), None);
}

#[test]
fn text_decoration_inset_key_maps_to_property_key() {
    let v = PropertyValue::TextDecorationInset(TextDecorationInset::Auto);
    assert_eq!(v.key(), PropertyKey::TextDecorationInset);
}

// ── text-emphasis-position (ED §3.4) ──

#[test]
fn text_emphasis_position_parses_auto_and_axis_combinations() {
    assert_eq!(
        parse("auto", "text-emphasis-position"),
        Some(PropertyValue::TextEmphasisPosition(
            TextEmphasisPosition::Auto
        ))
    );
    assert_eq!(
        parse("over", "text-emphasis-position"),
        Some(PropertyValue::TextEmphasisPosition(
            TextEmphasisPosition::Position {
                vertical: TextEmphasisVEdge::Over,
                horizontal: None,
            }
        ))
    );
    assert_eq!(
        parse("right under", "text-emphasis-position"),
        Some(PropertyValue::TextEmphasisPosition(
            TextEmphasisPosition::Position {
                vertical: TextEmphasisVEdge::Under,
                horizontal: Some(TextEmphasisHEdge::Right),
            }
        ))
    );
    // vertical is required; doubled axes rejected.
    assert_eq!(parse_entire("left", "text-emphasis-position"), None);
    assert_eq!(
        parse_entire("left over right", "text-emphasis-position"),
        None
    );
    assert_eq!(
        parse_entire("under right over", "text-emphasis-position"),
        None
    );
}

#[test]
fn text_emphasis_position_key_maps_to_property_key() {
    let v = PropertyValue::TextEmphasisPosition(TextEmphasisPosition::Auto);
    assert_eq!(v.key(), PropertyKey::TextEmphasisPosition);
}

// ── text-underline-position (ED §2.7) ──

#[test]
fn text_underline_position_parses_auto_and_combinations() {
    assert_eq!(
        parse("auto", "text-underline-position"),
        Some(PropertyValue::TextUnderlinePosition(
            TextUnderlinePosition::AUTO
        ))
    );
    assert_eq!(
        parse("under", "text-underline-position"),
        Some(PropertyValue::TextUnderlinePosition(
            TextUnderlinePosition {
                under: true,
                ..TextUnderlinePosition::AUTO
            }
        ))
    );
    assert_eq!(
        parse("from-font left", "text-underline-position"),
        Some(PropertyValue::TextUnderlinePosition(
            TextUnderlinePosition {
                from_font: true,
                left: true,
                ..TextUnderlinePosition::AUTO
            }
        ))
    );
    assert_eq!(
        parse("right under", "text-underline-position"),
        Some(PropertyValue::TextUnderlinePosition(
            TextUnderlinePosition {
                under: true,
                right: true,
                ..TextUnderlinePosition::AUTO
            }
        ))
    );
    // `auto` is exclusive; `from-font`+`under` and `left`+`right`
    // never combine.
    assert_eq!(parse_entire("auto under", "text-underline-position"), None);
    assert_eq!(
        parse_entire("under from-font", "text-underline-position"),
        None
    );
    assert_eq!(parse_entire("left right", "text-underline-position"), None);
    assert_eq!(parse_entire("bogus", "text-underline-position"), None);
}

#[test]
fn text_underline_position_key_maps_to_property_key() {
    let v = PropertyValue::TextUnderlinePosition(TextUnderlinePosition::AUTO);
    assert_eq!(v.key(), PropertyKey::TextUnderlinePosition);
}

#[test]
fn text_decoration_shorthand_rejects_empty_value() {
    assert_eq!(parse("", "text-decoration"), None);
}

#[test]
fn text_decoration_shorthand_two_style_components_leaves_leftover_for_caller_exhausted_check() {
    // Each `||` component at most once — a 2nd style keyword ("wavy")
    // doesn't match any unfilled slot (style already filled by "solid";
    // it isn't a line keyword or a `<color>`) so it's left unconsumed.
    // Same "helper returns `Some`, caller's `expect_exhausted` drops the
    // whole declaration" shape as `parse_border_shorthand`'s
    // `border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check`
    // — end-to-end rejection is pinned by `rule.rs`'s
    // `text_decoration_shorthand_two_style_components_declaration_dropped`.
    let mut input = ParserInput::new("solid wavy");
    let mut parser = Parser::new(&mut input);
    assert_eq!(
        parse_text_decoration_shorthand(&mut parser),
        Some(TextDecorationShorthand {
            line: TextDecorationLine::NONE,
            style: TextDecorationStyle::Solid,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::Auto,
        })
    );
    assert!(!parser.is_exhausted());
}

#[test]
fn text_decoration_shorthand_rejects_css_wide_keyword() {
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "text-decoration"), None);
    }
}

#[test]
fn text_decoration_key_maps_to_text_decoration_property_key() {
    let line = PropertyValue::TextDecorationLine(TextDecorationLine::UNDERLINE);
    assert_eq!(line.key(), PropertyKey::TextDecorationLine);
    let style = PropertyValue::TextDecorationStyle(TextDecorationStyle::Wavy);
    assert_eq!(style.key(), PropertyKey::TextDecorationStyle);
    let color = PropertyValue::TextDecorationColor(TextDecorationColor::CurrentColor);
    assert_eq!(color.key(), PropertyKey::TextDecorationColor);
    let shorthand = PropertyValue::TextDecoration(TextDecorationShorthand {
        line: TextDecorationLine::NONE,
        style: TextDecorationStyle::Solid,
        color: TextDecorationColor::CurrentColor,
        thickness: TextDecorationThickness::Auto,
    });
    assert_eq!(shorthand.key(), PropertyKey::TextDecoration);
}

// ── page (CSS Paged Media 3 §8.1) ──

#[test]
fn page_value_parses_auto_and_custom_ident() {
    assert_eq!(
        parse("auto", "page"),
        Some(PropertyValue::Page(PageValue::Auto))
    );
    assert_eq!(
        parse("table", "page"),
        Some(PropertyValue::Page(PageValue::Named(Atom::from("table"))))
    );
    assert_eq!(
        parse("xyzabc", "page"),
        Some(PropertyValue::Page(PageValue::Named(Atom::from("xyzabc"))))
    );
    // CSS-wide keywords and `default` are not custom idents.
    for kw in [
        "inherit",
        "initial",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ] {
        assert_eq!(parse_entire(kw, "page"), None, "{kw}");
    }
    // two idents / dimension are grammar-outside (caller drops).
    assert_eq!(parse_entire("not valid", "page"), None);
    assert_eq!(parse_entire("123px", "page"), None);
}

#[test]
fn page_value_key_maps_to_page_property_key() {
    let v = PropertyValue::Page(PageValue::Auto);
    assert_eq!(v.key(), PropertyKey::Page);
}

// ── vertical-align (CSS 2.1 §10.8.1) ──
//
// Value grammar (`VerticalAlign` doc's "Scope carving" section):
// baseline | sub | super | middle | text-top | text-bottom | <length>.
// `top` / `bottom` / `<percentage>` remain out of scope. Initial:
// baseline / Inherited: no / Computed value: keyword as specified,
// `<length>` absolutized.

#[test]
fn vertical_align_parse_all_keywords() {
    assert_eq!(
        parse("baseline", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Baseline))
    );
    assert_eq!(
        parse("sub", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Sub))
    );
    assert_eq!(
        parse("super", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Super))
    );
    assert_eq!(
        parse("middle", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Middle))
    );
    assert_eq!(
        parse("text-top", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::TextTop))
    );
    assert_eq!(
        parse("text-bottom", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::TextBottom))
    );
    assert_eq!(
        parse("top", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Top))
    );
    assert_eq!(
        parse("bottom", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Bottom))
    );
}

#[test]
fn vertical_align_is_case_insensitive() {
    assert_eq!(
        parse("BASELINE", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Baseline))
    );
    assert_eq!(
        parse("Sub", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Sub))
    );
    assert_eq!(
        parse("SUPER", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Super))
    );
    assert_eq!(
        parse("Middle", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Middle))
    );
    assert_eq!(
        parse("TEXT-TOP", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::TextTop))
    );
    assert_eq!(
        parse("Text-Bottom", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::TextBottom))
    );
}

#[test]
fn vertical_align_parse_length() {
    assert_eq!(
        parse("10px", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Px(10.0)
        )))
    );
    assert_eq!(
        parse("1.5em", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Em(1.5)
        )))
    );
}

#[test]
fn vertical_align_parse_length_allows_negative() {
    // §10.8.1 spec verbatim: "Raise (positive value) or lower
    // (negative value) the box by this distance." — no non-negative
    // filter, same as `letter-spacing`/`margin-*`.
    assert_eq!(
        parse("-4px", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Px(-4.0)
        )))
    );
}

#[test]
fn vertical_align_rejects_unknown_keywords() {
    for kw in ["sideways", "unknown"] {
        assert_eq!(parse(kw, "vertical-align"), None);
    }
}

#[test]
fn vertical_align_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "vertical-align"), None);
}

#[test]
fn vertical_align_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "vertical-align"), None);
    }
}

#[test]
fn vertical_align_parse_percentage() {
    // `<percentage>` is now implemented — absolutization is
    // `resolve_vertical_align` responsibility (own line-height basis,
    // `normal` fallback pinned there).
    assert_eq!(
        parse("50%", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Percent(50.0)
        )))
    );
    assert_eq!(
        parse("-20%", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Percent(-20.0)
        )))
    );
    assert_eq!(
        parse("0%", "vertical-align"),
        Some(PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Percent(0.0)
        )))
    );
}

#[test]
fn vertical_align_key_maps_to_vertical_align_property_key() {
    for va in [
        VerticalAlign::Baseline,
        VerticalAlign::Sub,
        VerticalAlign::Super,
        VerticalAlign::Middle,
        VerticalAlign::TextTop,
        VerticalAlign::TextBottom,
        VerticalAlign::Top,
        VerticalAlign::Bottom,
        VerticalAlign::Length(Length::Px(3.0)),
    ] {
        assert_eq!(
            PropertyValue::VerticalAlign(va).key(),
            PropertyKey::VerticalAlign
        );
    }
}

// ── font-style (CSS Fonts 4 §2.4) ──
//
// Value grammar (§2.4 spec verbatim, full property grammar): `normal |
// italic | left | right | oblique <angle [-90deg,90deg]>?`. This crate
// implements normal / italic / bare oblique (`FontStyle` doc's "Scope
// carving" section — the `<angle>` argument to `oblique` is out of
// scope). Initial: normal / Inherited: yes / Computed value:
// specified keyword (angle-bearing branch unreachable at this scope).

#[test]
fn font_style_parse_all_implemented_keywords() {
    assert_eq!(
        parse("normal", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Normal))
    );
    assert_eq!(
        parse("italic", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Italic))
    );
    assert_eq!(
        parse("oblique", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Oblique))
    );
}

#[test]
fn font_style_is_case_insensitive() {
    assert_eq!(
        parse("NORMAL", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Normal))
    );
    assert_eq!(
        parse("Italic", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Italic))
    );
    assert_eq!(
        parse("OBLIQUE", "font-style"),
        Some(PropertyValue::FontStyle(FontStyle::Oblique))
    );
}

#[test]
fn font_style_rejects_unimplemented_keywords() {
    // (b) not supported — the `left` / `right` slant-direction keywords
    // are spec-valid but unimplemented (`FontStyle` doc's "Scope
    // carving" section), not (a) spec-invalid.
    assert_eq!(parse("left", "font-style"), None);
    assert_eq!(parse("right", "font-style"), None);
}

// `oblique <angle>` (e.g. `oblique 14deg`) is not exercised by this
// module's `parse()` helper — it calls `parse_value` directly, which
// only runs the property-specific parser and does not perform the
// exhaustive-consumption check that drops a declaration with unconsumed
// trailing tokens. `parse_font_style` alone happily returns
// `Some(FontStyle::Oblique)` after consuming just the `oblique` ident,
// leaving `14deg` unread — the rejection of `oblique <angle>` as a
// whole declaration only happens one layer up, in
// `crate::rule`'s `DeclParser`. See
// `rule::tests::rejects_font_style_oblique_with_angle` (mirrors
// `rejects_extra_length_after_font_size`) for that test.

#[test]
fn font_style_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "font-style"), None);
}

#[test]
fn font_style_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "font-style"), None);
    }
}

#[test]
fn font_style_rejects_non_ident() {
    assert_eq!(parse("16px", "font-style"), None);
    assert_eq!(parse(r#""italic""#, "font-style"), None);
}

#[test]
fn font_style_key_maps_to_font_style_property_key() {
    let v = PropertyValue::FontStyle(FontStyle::Normal);
    assert_eq!(v.key(), PropertyKey::FontStyle);
    let v = PropertyValue::FontStyle(FontStyle::Italic);
    assert_eq!(v.key(), PropertyKey::FontStyle);
    let v = PropertyValue::FontStyle(FontStyle::Oblique);
    assert_eq!(v.key(), PropertyKey::FontStyle);
}

// ── font-variant-caps (CSS Fonts Module Level 3 §6.6) ──
//
// Value grammar (§6.6 spec verbatim, full property grammar): `normal |
// small-caps | all-small-caps | petite-caps | all-petite-caps | unicase
// | titling-caps`. This crate implements all 7 keywords
// (`FontVariantCaps` doc's "7 keyword の意味" section). Initial: normal /
// Inherited: yes / Computed value: specified keyword.

#[test]
fn font_variant_caps_parse_all_seven_keywords() {
    assert_eq!(
        parse("normal", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::Normal))
    );
    assert_eq!(
        parse("small-caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::SmallCaps))
    );
    assert_eq!(
        parse("all-small-caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(
            FontVariantCaps::AllSmallCaps
        ))
    );
    assert_eq!(
        parse("petite-caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::PetiteCaps))
    );
    assert_eq!(
        parse("all-petite-caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(
            FontVariantCaps::AllPetiteCaps
        ))
    );
    assert_eq!(
        parse("unicase", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::Unicase))
    );
    assert_eq!(
        parse("titling-caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::TitlingCaps))
    );
}

#[test]
fn font_variant_caps_is_case_insensitive() {
    assert_eq!(
        parse("NORMAL", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::Normal))
    );
    assert_eq!(
        parse("Small-Caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::SmallCaps))
    );
    assert_eq!(
        parse("ALL-SMALL-CAPS", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(
            FontVariantCaps::AllSmallCaps
        ))
    );
    assert_eq!(
        parse("Titling-Caps", "font-variant-caps"),
        Some(PropertyValue::FontVariantCaps(FontVariantCaps::TitlingCaps))
    );
}

#[test]
fn font_variant_caps_rejects_unknown_keyword() {
    // (a) spec-invalid — not one of the 7 keywords in the property
    // grammar.
    assert_eq!(parse("bogus", "font-variant-caps"), None);
}

#[test]
fn font_variant_caps_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "font-variant-caps"), None);
    }
}

#[test]
fn font_variant_caps_rejects_non_ident() {
    assert_eq!(parse("16px", "font-variant-caps"), None);
    assert_eq!(parse(r#""small-caps""#, "font-variant-caps"), None);
}

#[test]
fn font_variant_caps_key_maps_to_font_variant_caps_property_key() {
    for value in [
        FontVariantCaps::Normal,
        FontVariantCaps::SmallCaps,
        FontVariantCaps::AllSmallCaps,
        FontVariantCaps::PetiteCaps,
        FontVariantCaps::AllPetiteCaps,
        FontVariantCaps::Unicase,
        FontVariantCaps::TitlingCaps,
    ] {
        let v = PropertyValue::FontVariantCaps(value);
        assert_eq!(v.key(), PropertyKey::FontVariantCaps);
    }
}

#[test]
fn font_variant_caps_shorthand_name_has_no_dispatch_arm() {
    // `font-variant` (the shorthand, not `font-variant-caps`) resets
    // longhands this crate doesn't have (`FontVariantCaps` doc's
    // "Scope carving" section) — it is unrecognized like any other
    // unknown property name, not silently mapped to `-caps`.
    assert_eq!(parse("small-caps", "font-variant"), None);
}

// ── text-transform (CSS Text Module Level 3 §2.1) ──
//
// Value grammar (§2.1 spec verbatim, full property grammar): `none |
// [capitalize | uppercase | lowercase] || full-width || full-size-kana`.
// This crate implements only none / capitalize / uppercase / lowercase
// (`TextTransform` doc's "Scope carving" section). Initial: none /
// Inherited: yes / Computed value: specified keyword.

#[test]
fn text_transform_parse_all_four_keywords() {
    assert_eq!(
        parse("none", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::None))
    );
    assert_eq!(
        parse("capitalize", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::Capitalize))
    );
    assert_eq!(
        parse("uppercase", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::Uppercase))
    );
    assert_eq!(
        parse("lowercase", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::Lowercase))
    );
}

#[test]
fn text_transform_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::None))
    );
    assert_eq!(
        parse("Uppercase", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::Uppercase))
    );
}

#[test]
fn text_transform_accepts_width_keywords_and_combinations() {
    assert_eq!(
        parse("full-width", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::FullWidth))
    );
    assert_eq!(
        parse("full-size-kana", "text-transform"),
        Some(PropertyValue::TextTransform(TextTransform::FullSizeKana))
    );
    assert_eq!(
        parse("full-width full-size-kana lowercase", "text-transform"),
        Some(PropertyValue::TextTransform(
            TextTransform::LowercaseFullWidthFullSizeKana
        ))
    );
    assert_eq!(
        parse("full-size-kana capitalize", "text-transform"),
        Some(PropertyValue::TextTransform(
            TextTransform::CapitalizeFullSizeKana
        ))
    );
    assert_eq!(
        parse("full-width uppercase", "text-transform"),
        Some(PropertyValue::TextTransform(
            TextTransform::UppercaseFullWidth
        ))
    );
    assert_eq!(parse("uppercase uppercase", "text-transform"), None);
    assert_eq!(parse("full-width full-width", "text-transform"), None);
}

#[test]
fn text_transform_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "text-transform"), None);
}

#[test]
fn text_transform_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "text-transform"), None);
    }
}

#[test]
fn text_transform_rejects_non_ident() {
    assert_eq!(parse("16px", "text-transform"), None);
    assert_eq!(parse(r#""uppercase""#, "text-transform"), None);
}

#[test]
fn text_transform_key_maps_to_text_transform_property_key() {
    let v = PropertyValue::TextTransform(TextTransform::None);
    assert_eq!(v.key(), PropertyKey::TextTransform);
    let v = PropertyValue::TextTransform(TextTransform::Capitalize);
    assert_eq!(v.key(), PropertyKey::TextTransform);
}

// ── visibility (CSS Display 3 §4) ──
//
// Value grammar (spec verbatim): `visible | hidden | collapse`. This
// crate implements all 3 keywords (`Visibility` doc's "Scope carving"
// section — `collapse`'s formatting-context-specific space-saving effect
// is unimplemented, but the keyword itself is fully accepted). Initial:
// visible / Inherited: yes / Computed value: as specified.

#[test]
fn visibility_parse_all_keywords() {
    assert_eq!(
        parse("visible", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Visible))
    );
    assert_eq!(
        parse("hidden", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Hidden))
    );
    assert_eq!(
        parse("collapse", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Collapse))
    );
}

#[test]
fn visibility_is_case_insensitive() {
    assert_eq!(
        parse("VISIBLE", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Visible))
    );
    assert_eq!(
        parse("Hidden", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Hidden))
    );
    assert_eq!(
        parse("Collapse", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Collapse))
    );
}

#[test]
fn visibility_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "visibility"), None);
}

#[test]
fn visibility_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "visibility"), None);
    }
}

#[test]
fn visibility_rejects_non_ident() {
    assert_eq!(parse("16px", "visibility"), None);
    assert_eq!(parse(r#""hidden""#, "visibility"), None);
}

#[test]
fn visibility_key_maps_to_visibility_property_key() {
    let v = PropertyValue::Visibility(Visibility::Visible);
    assert_eq!(v.key(), PropertyKey::Visibility);
    let v = PropertyValue::Visibility(Visibility::Hidden);
    assert_eq!(v.key(), PropertyKey::Visibility);
    let v = PropertyValue::Visibility(Visibility::Collapse);
    assert_eq!(v.key(), PropertyKey::Visibility);
}

#[test]
fn z_index_key_maps_to_z_index_property_key() {
    let v = PropertyValue::ZIndex(ZIndexValue::Auto);
    assert_eq!(v.key(), PropertyKey::ZIndex);
    let v = PropertyValue::ZIndex(ZIndexValue::Integer(-1));
    assert_eq!(v.key(), PropertyKey::ZIndex);
}

#[test]
fn z_index_parses_auto_and_integers() {
    assert_eq!(
        parse("auto", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Auto))
    );
    assert_eq!(
        parse("0", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(0)))
    );
    assert_eq!(
        parse("3", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(3)))
    );
    assert_eq!(
        parse("-1", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(-1)))
    );
    assert_eq!(
        parse("2147483647", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MAX)))
    );
    assert_eq!(
        parse("-2147483648", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MIN)))
    );
}

#[test]
fn z_index_clamps_out_of_i32_range() {
    // cssparser 0.37.0 tokenizer.rs:1084-1091 clamps out-of-i32-range integer
    // literals to i32::MAX/MIN rather than wrapping or erroring; parse_z_index
    // delegates to `expect_integer()` so the clamp propagates.
    assert_eq!(
        parse("99999999999", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MAX)))
    );
    assert_eq!(
        parse("-99999999999", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MIN)))
    );
    // Just beyond boundaries also clamp.
    assert_eq!(
        parse("2147483648", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MAX)))
    );
    assert_eq!(
        parse("-2147483649", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MIN)))
    );
    // Very large magnitude (far beyond i32) still clamps.
    assert_eq!(
        parse("99999999999999999999", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MAX)))
    );
    assert_eq!(
        parse("-99999999999999999999", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MIN)))
    );
}

// ── float (CSS2 §9.5.1) ──
//
// Value grammar (§9.5.1 spec verbatim): `left | right | none | inherit`.
// Initial: none / Inherited: no / Computed value: as specified.

#[test]
fn float_parse_all_keywords() {
    assert_eq!(
        parse("none", "float"),
        Some(PropertyValue::Float(FloatValue::None))
    );
    assert_eq!(
        parse("left", "float"),
        Some(PropertyValue::Float(FloatValue::Left))
    );
    assert_eq!(
        parse("right", "float"),
        Some(PropertyValue::Float(FloatValue::Right))
    );
}

#[test]
fn float_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "float"),
        Some(PropertyValue::Float(FloatValue::None))
    );
    assert_eq!(
        parse("Left", "float"),
        Some(PropertyValue::Float(FloatValue::Left))
    );
    assert_eq!(
        parse("RIGHT", "float"),
        Some(PropertyValue::Float(FloatValue::Right))
    );
}

#[test]
fn float_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "float"), None);
    // `clear`'s `both` keyword is not valid on `float`.
    assert_eq!(parse("both", "float"), None);
}

#[test]
fn float_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "float"), None);
    }
}

#[test]
fn float_rejects_non_ident() {
    assert_eq!(parse("16px", "float"), None);
    assert_eq!(parse(r#""left""#, "float"), None);
}

#[test]
fn float_key_maps_to_float_property_key() {
    let v = PropertyValue::Float(FloatValue::None);
    assert_eq!(v.key(), PropertyKey::Float);
    let v = PropertyValue::Float(FloatValue::Left);
    assert_eq!(v.key(), PropertyKey::Float);
    let v = PropertyValue::Float(FloatValue::Right);
    assert_eq!(v.key(), PropertyKey::Float);
}

// ── clear (CSS2 §9.5.2) ──
//
// Value grammar (§9.5.2 spec verbatim): `none | left | right | both |
// inherit`. Initial: none / Inherited: no / Computed value: as
// specified.

#[test]
fn clear_parse_all_keywords() {
    assert_eq!(
        parse("none", "clear"),
        Some(PropertyValue::Clear(ClearValue::None))
    );
    assert_eq!(
        parse("left", "clear"),
        Some(PropertyValue::Clear(ClearValue::Left))
    );
    assert_eq!(
        parse("right", "clear"),
        Some(PropertyValue::Clear(ClearValue::Right))
    );
    assert_eq!(
        parse("both", "clear"),
        Some(PropertyValue::Clear(ClearValue::Both))
    );
}

#[test]
fn clear_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "clear"),
        Some(PropertyValue::Clear(ClearValue::None))
    );
    assert_eq!(
        parse("Both", "clear"),
        Some(PropertyValue::Clear(ClearValue::Both))
    );
}

#[test]
fn clear_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "clear"), None);
}

#[test]
fn clear_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "clear"), None);
    }
}

#[test]
fn clear_rejects_non_ident() {
    assert_eq!(parse("16px", "clear"), None);
    assert_eq!(parse(r#""left""#, "clear"), None);
}

#[test]
fn clear_key_maps_to_clear_property_key() {
    let v = PropertyValue::Clear(ClearValue::None);
    assert_eq!(v.key(), PropertyKey::Clear);
    let v = PropertyValue::Clear(ClearValue::Both);
    assert_eq!(v.key(), PropertyKey::Clear);
}

// ── resolve_display_for_float (CSS2 §9.7) ──

#[test]
fn resolve_display_for_float_is_noop_when_float_is_none() {
    for display in [
        DisplayValue::Block,
        DisplayValue::Inline,
        DisplayValue::InlineBlock,
        DisplayValue::None,
        DisplayValue::Flex,
        DisplayValue::Grid,
        DisplayValue::ListItem,
        DisplayValue::FlowRoot,
        DisplayValue::Contents,
    ] {
        assert_eq!(
            resolve_display_for_float(display, FloatValue::None),
            display
        );
    }
}

#[test]
fn resolve_display_for_float_forces_inline_and_inline_block_to_block() {
    // §9.7 table: `inline` / `inline-block` → `block`.
    for float in [FloatValue::Left, FloatValue::Right] {
        assert_eq!(
            resolve_display_for_float(DisplayValue::Inline, float),
            DisplayValue::Block
        );
        assert_eq!(
            resolve_display_for_float(DisplayValue::InlineBlock, float),
            DisplayValue::Block
        );
    }
}

#[test]
fn resolve_display_for_float_leaves_block_flex_grid_list_item_unchanged() {
    // §9.7 table's "others" row — not in the forced-to-block list.
    for float in [FloatValue::Left, FloatValue::Right] {
        assert_eq!(
            resolve_display_for_float(DisplayValue::Block, float),
            DisplayValue::Block
        );
        assert_eq!(
            resolve_display_for_float(DisplayValue::Flex, float),
            DisplayValue::Flex
        );
        assert_eq!(
            resolve_display_for_float(DisplayValue::Grid, float),
            DisplayValue::Grid
        );
        assert_eq!(
            resolve_display_for_float(DisplayValue::ListItem, float),
            DisplayValue::ListItem
        );
        assert_eq!(
            resolve_display_for_float(DisplayValue::FlowRoot, float),
            DisplayValue::FlowRoot
        );
    }
}

#[test]
fn resolve_display_for_float_leaves_none_as_none() {
    // §9.7 leading clause: "If 'display' has the value 'none', then
    // 'position' and 'float' do not apply" — this precedes the table,
    // so `none` must not be affected even when `float` is not `none`.
    for float in [FloatValue::Left, FloatValue::Right] {
        assert_eq!(
            resolve_display_for_float(DisplayValue::None, float),
            DisplayValue::None
        );
    }
}

#[test]
fn resolve_display_for_float_leaves_contents_unchanged() {
    // CSS Display 3 §2.7 verbatim: blockification "has no effect on
    // display types that generate no box at all, such as display:
    // none or display: contents" — so floating a `contents` element
    // must not force it to `block` either.
    for float in [FloatValue::Left, FloatValue::Right] {
        assert_eq!(
            resolve_display_for_float(DisplayValue::Contents, float),
            DisplayValue::Contents
        );
    }
}

// ── word-break (CSS Text 3 §5.1) ──
//
// Value grammar (§5.1 spec verbatim, full property grammar): `normal |
// keep-all | break-all | break-word`. This crate implements only
// normal / keep-all / break-all (`WordBreak` doc's "Scope carving"
// section) — the 4th, deprecated `break-word` keyword is excluded.
// Initial: normal / Inherited: yes / Computed value: specified keyword.

#[test]
fn word_break_parse_all_implemented_keywords() {
    assert_eq!(
        parse("normal", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::Normal))
    );
    assert_eq!(
        parse("keep-all", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::KeepAll))
    );
    assert_eq!(
        parse("break-all", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::BreakAll))
    );
}

#[test]
fn word_break_is_case_insensitive() {
    assert_eq!(
        parse("NORMAL", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::Normal))
    );
    assert_eq!(
        parse("Keep-All", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::KeepAll))
    );
    assert_eq!(
        parse("BREAK-ALL", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::BreakAll))
    );
}

#[test]
fn word_break_accepts_break_word_and_level4_keywords() {
    assert_eq!(
        parse("break-word", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::BreakWord))
    );
    assert_eq!(
        parse("manual", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::Manual))
    );
    assert_eq!(
        parse("auto-phrase", "word-break"),
        Some(PropertyValue::WordBreak(WordBreak::AutoPhrase))
    );
}

#[test]
fn word_break_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "word-break"), None);
}

#[test]
fn word_break_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "word-break"), None);
    }
}

#[test]
fn word_break_rejects_non_ident() {
    assert_eq!(parse("16px", "word-break"), None);
    assert_eq!(parse(r#""normal""#, "word-break"), None);
}

#[test]
fn word_break_key_maps_to_word_break_property_key() {
    let v = PropertyValue::WordBreak(WordBreak::Normal);
    assert_eq!(v.key(), PropertyKey::WordBreak);
    let v = PropertyValue::WordBreak(WordBreak::KeepAll);
    assert_eq!(v.key(), PropertyKey::WordBreak);
    let v = PropertyValue::WordBreak(WordBreak::BreakAll);
    assert_eq!(v.key(), PropertyKey::WordBreak);
}

// ── overflow-wrap / word-wrap legacy alias (CSS Text 3 §5.4) ──
//
// Value grammar (§5.4 spec verbatim): `normal | break-word | anywhere`.
// All 3 keywords are implemented — unlike `word-break`'s deprecated
// `break-word`, `overflow-wrap: break-word` is not deprecated
// (`OverflowWrap` doc). `word-wrap` is the spec-mandated legacy name
// alias and must parse identically. Initial: normal / Inherited: yes /
// Computed value: specified keyword.

#[test]
fn overflow_wrap_parse_all_keywords() {
    assert_eq!(
        parse("normal", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::Normal))
    );
    assert_eq!(
        parse("break-word", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::BreakWord))
    );
    assert_eq!(
        parse("anywhere", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::Anywhere))
    );
}

#[test]
fn overflow_wrap_is_case_insensitive() {
    assert_eq!(
        parse("NORMAL", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::Normal))
    );
    assert_eq!(
        parse("Break-Word", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::BreakWord))
    );
    assert_eq!(
        parse("ANYWHERE", "overflow-wrap"),
        Some(PropertyValue::OverflowWrap(OverflowWrap::Anywhere))
    );
}

#[test]
fn word_wrap_legacy_alias_parses_identically_to_overflow_wrap() {
    // CSS Text 3 §5.4 verbatim: "For legacy reasons, UAs must treat
    // word-wrap as a legacy name alias of the overflow-wrap property."
    for (kw, expected) in [
        ("normal", OverflowWrap::Normal),
        ("break-word", OverflowWrap::BreakWord),
        ("anywhere", OverflowWrap::Anywhere),
    ] {
        assert_eq!(
            parse(kw, "word-wrap"),
            Some(PropertyValue::OverflowWrap(expected))
        );
        assert_eq!(parse(kw, "word-wrap"), parse(kw, "overflow-wrap"));
    }
}

#[test]
fn overflow_wrap_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "overflow-wrap"), None);
    assert_eq!(parse("bogus", "word-wrap"), None);
}

#[test]
fn overflow_wrap_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "overflow-wrap"), None);
        assert_eq!(parse(kw, "word-wrap"), None);
    }
}

#[test]
fn overflow_wrap_rejects_non_ident() {
    assert_eq!(parse("16px", "overflow-wrap"), None);
    assert_eq!(parse(r#""normal""#, "overflow-wrap"), None);
}

#[test]
fn overflow_wrap_key_maps_to_overflow_wrap_property_key() {
    // `word-wrap` and `overflow-wrap` share one `PropertyKey` — the
    // legacy alias cascades as one property, not two independently
    // winning ones (`OverflowWrap` doc's "legacy alias" section).
    let v = PropertyValue::OverflowWrap(OverflowWrap::Normal);
    assert_eq!(v.key(), PropertyKey::OverflowWrap);
    let v = PropertyValue::OverflowWrap(OverflowWrap::BreakWord);
    assert_eq!(v.key(), PropertyKey::OverflowWrap);
    let v = PropertyValue::OverflowWrap(OverflowWrap::Anywhere);
    assert_eq!(v.key(), PropertyKey::OverflowWrap);
}

// ── break-before / break-after (CSS Fragmentation Module Level 3
// §3.1) + page-break-before / page-break-after legacy shorthand (§3.4) ──
//
// Value grammar (this crate's scope, `BreakBetween` doc's "Scope
// carving" section): `auto | avoid | avoid-page | page`. Initial:
// `auto` / Inherited: no / Computed value: specified keyword.

#[test]
fn break_before_after_parse_all_implemented_keywords() {
    assert_eq!(
        parse("auto", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Auto))
    );
    assert_eq!(
        parse("auto", "break-after"),
        Some(PropertyValue::BreakAfter(BreakBetween::Auto))
    );
    assert_eq!(
        parse("avoid", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Avoid))
    );
    assert_eq!(
        parse("avoid-page", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::AvoidPage))
    );
    assert_eq!(
        parse("page", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Page))
    );
    assert_eq!(
        parse("avoid", "break-after"),
        Some(PropertyValue::BreakAfter(BreakBetween::Avoid))
    );
    assert_eq!(
        parse("avoid-page", "break-after"),
        Some(PropertyValue::BreakAfter(BreakBetween::AvoidPage))
    );
    assert_eq!(
        parse("page", "break-after"),
        Some(PropertyValue::BreakAfter(BreakBetween::Page))
    );
}

#[test]
fn break_before_is_case_insensitive() {
    assert_eq!(
        parse("AUTO", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Auto))
    );
    assert_eq!(
        parse("Avoid-Page", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::AvoidPage))
    );
    assert_eq!(
        parse("PAGE", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Page))
    );
}

#[test]
fn break_before_after_rejects_out_of_scope_column_and_region_values() {
    // (b) not supported — this crate has no multi-column or CSS
    // Regions fragmentation context (`BreakBetween` doc's "Scope
    // carving" section), not (a) spec-invalid.
    for kw in ["avoid-column", "column", "avoid-region", "region"] {
        assert_eq!(parse(kw, "break-before"), None);
        assert_eq!(parse(kw, "break-after"), None);
    }
}

#[test]
fn break_before_after_rejects_out_of_scope_page_spread_values() {
    // (b) not supported — no page-spread concept in this crate
    // (`BreakBetween` doc's "Scope carving" section).
    for kw in ["left", "right", "recto", "verso"] {
        assert_eq!(parse(kw, "break-before"), None);
        assert_eq!(parse(kw, "break-after"), None);
    }
}

#[test]
fn break_before_after_rejects_always_and_all() {
    // `always`/`all` are not part of the current break-before/
    // break-after grammar at all (`BreakBetween` doc's "Scope carving"
    // section — Level 3's change log only names `always`, not `all`;
    // both live in Level 4 instead, which this crate does not target).
    // `always` is valid only as the `page-break-before`/
    // `page-break-after` legacy shorthand's own keyword, never
    // directly on `break-before`/`break-after`; `all` has no path in
    // at all.
    for kw in ["always", "all"] {
        assert_eq!(parse(kw, "break-before"), None);
        assert_eq!(parse(kw, "break-after"), None);
    }
}

#[test]
fn break_before_after_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "break-before"), None);
    assert_eq!(parse("bogus", "break-after"), None);
}

#[test]
fn break_before_after_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "break-before"), None);
        assert_eq!(parse(kw, "break-after"), None);
    }
}

#[test]
fn break_before_after_rejects_non_ident() {
    assert_eq!(parse("16px", "break-before"), None);
    assert_eq!(parse(r#""auto""#, "break-after"), None);
}

#[test]
fn break_before_after_key_maps_to_distinct_property_keys() {
    // `break-before` / `break-after` are 2 independent cascade winners
    // (unlike the `word-wrap`/`overflow-wrap` name alias, which shares
    // one `PropertyKey` — `BreakBetween` doc's "legacy shorthand"
    // section explains why this pair does too, just each with its
    // *own* longhand).
    let v = PropertyValue::BreakBefore(BreakBetween::Page);
    assert_eq!(v.key(), PropertyKey::BreakBefore);
    let v = PropertyValue::BreakAfter(BreakBetween::Page);
    assert_eq!(v.key(), PropertyKey::BreakAfter);
}

// ── page-break-before / page-break-after legacy shorthand value remap
// (CSS Fragmentation Module Level 3 §3.4) ──

#[test]
fn page_break_before_after_legacy_shorthand_identity_values() {
    for prop in ["page-break-before", "page-break-after"] {
        let wrap = |v| {
            if prop == "page-break-before" {
                PropertyValue::BreakBefore(v)
            } else {
                PropertyValue::BreakAfter(v)
            }
        };
        assert_eq!(parse("auto", prop), Some(wrap(BreakBetween::Auto)));
        assert_eq!(parse("avoid", prop), Some(wrap(BreakBetween::Avoid)));
    }
}

#[test]
fn page_break_before_after_legacy_shorthand_remaps_always_to_page() {
    // CSS Fragmentation Module Level 3 §3.4 mapping table verbatim:
    // `always` (page-break-*) -> `page` (break-*). Non-identity remap —
    // check the exact equality with the longhand spelling, mirroring
    // `word_wrap_legacy_alias_parses_identically_to_overflow_wrap`'s
    // shape (there the two spellings are identical; here they are not,
    // which is exactly what this test must catch).
    assert_eq!(
        parse("always", "page-break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Page))
    );
    assert_eq!(
        parse("always", "page-break-before"),
        parse("page", "break-before")
    );
    assert_eq!(
        parse("always", "page-break-after"),
        Some(PropertyValue::BreakAfter(BreakBetween::Page))
    );
    assert_eq!(
        parse("always", "page-break-after"),
        parse("page", "break-after")
    );
}

#[test]
fn page_break_before_after_legacy_shorthand_rejects_new_property_only_values() {
    // `avoid-page` / `page` are valid on `break-before`/`break-after`
    // directly, but CSS2.1's own `page-break-before`/`page-break-after`
    // propdef grammar (`auto | always | avoid | left | right`) does not
    // have them — the legacy shorthand's grammar is CSS2.1's, not the
    // new property's (`BreakBetween` doc's "legacy shorthand" section).
    for kw in ["avoid-page", "page"] {
        assert_eq!(parse(kw, "page-break-before"), None);
        assert_eq!(parse(kw, "page-break-after"), None);
    }
}

#[test]
fn page_break_before_after_legacy_shorthand_rejects_left_and_right() {
    // CSS2.1's own grammar has `left`/`right`, but they remap to
    // `break-before`/`break-after` values this crate does not
    // implement (`BreakBetween` doc's "Scope carving" section) — the
    // scope carve applies transitively through the legacy shorthand.
    for kw in ["left", "right"] {
        assert_eq!(parse(kw, "page-break-before"), None);
        assert_eq!(parse(kw, "page-break-after"), None);
    }
}

#[test]
fn page_break_before_after_legacy_shorthand_key_maps_to_same_key_as_longhand() {
    // One cascade winner per property, whether declared via the new
    // name or the legacy shorthand name (`BreakBetween` doc's "legacy
    // shorthand" section — no `expand_shorthand_into` arm needed since
    // this is a 1:1, not a fan-out, shorthand).
    let v = parse("always", "page-break-before").unwrap();
    assert_eq!(v.key(), PropertyKey::BreakBefore);
    let v = parse("always", "page-break-after").unwrap();
    assert_eq!(v.key(), PropertyKey::BreakAfter);
}

// ── break-inside (CSS Fragmentation Module Level 3 §3.2) +
// page-break-inside legacy shorthand (§3.4) ──
//
// Value grammar (this crate's scope, `BreakInside` doc's "Scope
// carving" section): `auto | avoid | avoid-page` — a smaller, disjoint
// set from `break-before`/`break-after`'s `BreakBetween` (no `page`).
// Initial: `auto` / Inherited: no / Computed value: specified keyword.

#[test]
fn break_inside_parse_all_implemented_keywords() {
    assert_eq!(
        parse("auto", "break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::Auto))
    );
    assert_eq!(
        parse("avoid", "break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::Avoid))
    );
    assert_eq!(
        parse("avoid-page", "break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::AvoidPage))
    );
}

#[test]
fn break_inside_is_case_insensitive() {
    assert_eq!(
        parse("AUTO", "break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::Auto))
    );
    assert_eq!(
        parse("Avoid-Page", "break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::AvoidPage))
    );
}

#[test]
fn break_inside_rejects_forced_break_values() {
    // `break-inside` has no forced-break values at all — `page` /
    // `column` / `region` are valid on `break-before`/`break-after`
    // (or would be, absent this crate's scope carve) but have no
    // `break-inside` counterpart in the spec grammar at all, not even
    // an excluded one (`BreakInside` doc: "breaking within has no
    // start/end edge to force a break relative to").
    for kw in ["page", "column", "region"] {
        assert_eq!(parse(kw, "break-inside"), None);
    }
}

#[test]
fn break_inside_rejects_out_of_scope_avoid_values() {
    // Unlike `page`/`column`/`region` above, `avoid-column` and
    // `avoid-region` *are* in `break-inside`'s own propdef grammar
    // (`auto | avoid | avoid-page | avoid-column | avoid-region`,
    // `BreakInside` doc) — they are rejected here purely by this
    // crate's scope carve (no multi-column / CSS Regions fragmentation
    // context), not because the spec lacks them.
    for kw in ["avoid-column", "avoid-region"] {
        assert_eq!(parse(kw, "break-inside"), None);
    }
}

#[test]
fn break_inside_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "break-inside"), None);
}

#[test]
fn break_inside_rejects_css_wide_keyword() {
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "break-inside"), None);
    }
}

#[test]
fn break_inside_rejects_non_ident() {
    assert_eq!(parse("16px", "break-inside"), None);
    assert_eq!(parse(r#""auto""#, "break-inside"), None);
}

#[test]
fn break_inside_key_maps_to_break_inside_property_key() {
    let v = PropertyValue::BreakInside(BreakInside::AvoidPage);
    assert_eq!(v.key(), PropertyKey::BreakInside);
}

#[test]
fn page_break_inside_legacy_shorthand_identity_values() {
    assert_eq!(
        parse("auto", "page-break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::Auto))
    );
    assert_eq!(
        parse("avoid", "page-break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::Avoid))
    );
    assert_eq!(
        parse("auto", "page-break-inside"),
        parse("auto", "break-inside")
    );
    assert_eq!(
        parse("avoid", "page-break-inside"),
        parse("avoid", "break-inside")
    );
}

#[test]
fn page_break_inside_legacy_shorthand_rejects_new_property_only_value() {
    // `avoid-page` is valid on `break-inside` directly, but CSS2.1's
    // own `page-break-inside` propdef grammar is just `auto | avoid` —
    // the legacy shorthand's grammar is CSS2.1's, not the new
    // property's fuller one (`BreakInside` doc's "legacy shorthand"
    // section).
    assert_eq!(parse("avoid-page", "page-break-inside"), None);
}

#[test]
fn page_break_inside_legacy_shorthand_key_maps_to_same_key_as_longhand() {
    let v = parse("avoid", "page-break-inside").unwrap();
    assert_eq!(v.key(), PropertyKey::BreakInside);
}

// ── letter-spacing / word-spacing (CSS Text 3 §7.2 / §7.1) ──
//
// Value grammar (spec verbatim, identical for both): `normal | <length>`.
// Initial: `normal`. Inherited: yes. Percentages: N/A. Both share
// `parse_letter_or_word_spacing` — see that function's doc for the
// ordering / non-negative / percentage-rejection rationale.

#[test]
fn letter_spacing_parse_normal_keyword() {
    assert_eq!(
        parse("normal", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LengthOrNormal::Normal))
    );
}

#[test]
fn word_spacing_parse_normal_keyword() {
    assert_eq!(
        parse("normal", "word-spacing"),
        Some(PropertyValue::WordSpacing(LengthOrNormal::Normal))
    );
}

#[test]
fn letter_spacing_is_case_insensitive_normal() {
    assert_eq!(
        parse("NORMAL", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LengthOrNormal::Normal))
    );
    assert_eq!(
        parse("Normal", "word-spacing"),
        Some(PropertyValue::WordSpacing(LengthOrNormal::Normal))
    );
}

#[test]
fn letter_spacing_parse_length_px() {
    assert_eq!(
        parse("2px", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
            Length::Px(2.0)
        )))
    );
}

#[test]
fn word_spacing_parse_length_px() {
    assert_eq!(
        parse("4px", "word-spacing"),
        Some(PropertyValue::WordSpacing(LengthOrNormal::Length(
            Length::Px(4.0)
        )))
    );
}

#[test]
fn letter_spacing_accepts_length_em_rem_pt() {
    assert_eq!(
        parse("0.1em", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
            Length::Em(0.1)
        )))
    );
    assert_eq!(
        parse("1rem", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
            Length::Rem(1.0)
        )))
    );
    assert_eq!(
        parse("2pt", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
            Length::Pt(2.0)
        )))
    );
}

// Spec verbatim (both §7.1 and §7.2): "Values may be negative, but there
// may be implementation-dependent limits." — unlike `line-height` /
// `font-size` / `padding` etc., this property does NOT reject negative
// lengths at parse time (`parse_letter_or_word_spacing` doc's "Negative
// length は許容" section).
#[test]
fn letter_spacing_accepts_negative_length() {
    assert_eq!(
        parse("-2px", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
            Length::Px(-2.0)
        )))
    );
    assert_eq!(
        parse("-0.05em", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
            Length::Em(-0.05)
        )))
    );
}

#[test]
fn word_spacing_accepts_negative_length() {
    assert_eq!(
        parse("-1px", "word-spacing"),
        Some(PropertyValue::WordSpacing(LengthOrNormal::Length(
            Length::Px(-1.0)
        )))
    );
}

// Bare `0` has no `<number>` grammar alternative to disambiguate against
// here (unlike `line-height`) — it goes through `parse_length_value`'s
// unitless-zero clause and becomes `Length::Px(0.0)`, not `Normal`.
#[test]
fn letter_spacing_unitless_zero_is_length_not_normal() {
    assert_eq!(
        parse("0", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
            Length::Px(0.0)
        )))
    );
}

// Spec verbatim (both §7.1 and §7.2): "Percentages: N/A" / "Percentages:
// n/a" — percentage is rejected at parse time, unlike `line-height`'s
// `<length-percentage>`.
#[test]
fn letter_spacing_accepts_percentage() {
    assert_eq!(
        parse("5%", "letter-spacing"),
        Some(PropertyValue::LetterSpacing(LengthOrNormal::Length(
            Length::Percent(5.0)
        )))
    );
}

#[test]
fn word_spacing_accepts_percentage() {
    assert_eq!(
        parse("5%", "word-spacing"),
        Some(PropertyValue::WordSpacing(LengthOrNormal::Length(
            Length::Percent(5.0)
        )))
    );
}

#[test]
fn letter_spacing_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "letter-spacing"), None);
    assert_eq!(parse("auto", "letter-spacing"), None);
}

#[test]
fn word_spacing_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "word-spacing"), None);
}

#[test]
fn letter_spacing_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "letter-spacing"), None);
    }
}

#[test]
fn word_spacing_rejects_css_wide_keyword() {
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "word-spacing"), None);
    }
}

#[test]
fn letter_spacing_rejects_non_length_non_ident() {
    assert_eq!(parse(r#""2px""#, "letter-spacing"), None);
}

#[test]
fn letter_spacing_key_maps_to_letter_spacing_property_key() {
    let v = PropertyValue::LetterSpacing(LengthOrNormal::Normal);
    assert_eq!(v.key(), PropertyKey::LetterSpacing);
    let v = PropertyValue::LetterSpacing(LengthOrNormal::Length(Length::Px(2.0)));
    assert_eq!(v.key(), PropertyKey::LetterSpacing);
}

// ── tab-size (CSS Text Module Level 3 §4.2) ──
//
// Value grammar: `<number [0,∞]> | <length [0,∞]>`. Initial: `8`.
// Inherited: yes. Percentages: N/A. Shares `parse_line_height`'s
// Number-then-Length ordering (minus the `normal` branch tab-size's
// grammar doesn't have) — see `parse_tab_size` doc.

#[test]
fn tab_size_parse_bare_number() {
    assert_eq!(
        parse("4", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Number(4.0)))
    );
}

#[test]
fn tab_size_parse_length_px() {
    assert_eq!(
        parse("32px", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Px(32.0))))
    );
}

#[test]
fn tab_size_accepts_length_em_rem_pt() {
    assert_eq!(
        parse("2em", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Em(2.0))))
    );
    assert_eq!(
        parse("1rem", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Rem(1.0))))
    );
    assert_eq!(
        parse("6pt", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Pt(6.0))))
    );
}

#[test]
fn tab_size_number_vs_em_are_distinct_variants() {
    // Same load-bearing Number-vs-Length distinction as `line-height`
    // (`line_height_number_vs_em_are_distinct_variants`) — unitless `4`
    // and dimensioned `4em` map to different variants even though the
    // scalar matches.
    let number = parse("4", "tab-size");
    let length_em = parse("4em", "tab-size");
    assert_eq!(number, Some(PropertyValue::TabSize(TabSize::Number(4.0))));
    assert_eq!(
        length_em,
        Some(PropertyValue::TabSize(TabSize::Length(Length::Em(4.0))))
    );
    assert_ne!(number, length_em, "Number and Length must be distinct");
}

#[test]
fn tab_size_accepts_zero_number_and_length() {
    // spec `[0,∞]`: 0 is a valid boundary value. Same CSS Values 3 §5
    // "0 could be parsed as either a `<number>` or a `<length>`... must
    // parse as a `<number>`" clause `line-height` exercises
    // (`line_height_accepts_zero_number_and_length`) — bare `0` commits
    // to the Number branch before the Length branch is ever tried.
    assert_eq!(
        parse("0", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Number(0.0)))
    );
    assert_eq!(
        parse("0px", "tab-size"),
        Some(PropertyValue::TabSize(TabSize::Length(Length::Px(0.0))))
    );
}

#[test]
fn tab_size_rejects_negative_number() {
    // spec verbatim: "Negative values are not allowed."
    assert_eq!(parse("-4", "tab-size"), None);
}

#[test]
fn tab_size_rejects_negative_length() {
    assert_eq!(parse("-10px", "tab-size"), None);
    assert_eq!(parse("-1em", "tab-size"), None);
}

#[test]
fn tab_size_rejects_negative_number_with_trailing_length() {
    // Regression guard mirroring
    // `line_height_rejects_negative_number_with_trailing_length`: the
    // Number branch must not fall through to the Length branch once a
    // Token::Number has been consumed via `try_parse`'s `Ok` path
    // (which does not rewind). A fallthrough implementation would let
    // `tab-size: -1 20px` silently accept `20px`.
    assert_eq!(parse("-1 20px", "tab-size"), None);
    assert_eq!(parse("-1 2em", "tab-size"), None);
}

#[test]
fn tab_size_rejects_percentage() {
    // spec propdef: "Percentages: N/A".
    assert_eq!(parse("50%", "tab-size"), None);
}

#[test]
fn tab_size_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "tab-size"), None);
    assert_eq!(parse("auto", "tab-size"), None);
    assert_eq!(parse("normal", "tab-size"), None);
}

#[test]
fn tab_size_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future
    // work), silent drop (`PropertyValue` doc's "CSS-wide keyword"
    // section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "tab-size"), None);
    }
}

#[test]
fn tab_size_rejects_non_length_non_ident() {
    assert_eq!(parse(r#""4""#, "tab-size"), None);
}

#[test]
fn tab_size_key_maps_to_tab_size_property_key() {
    let v = PropertyValue::TabSize(TabSize::Number(4.0));
    assert_eq!(v.key(), PropertyKey::TabSize);
    let v = PropertyValue::TabSize(TabSize::Length(Length::Px(32.0)));
    assert_eq!(v.key(), PropertyKey::TabSize);
}

#[test]
fn word_spacing_key_maps_to_word_spacing_property_key() {
    let v = PropertyValue::WordSpacing(LengthOrNormal::Normal);
    assert_eq!(v.key(), PropertyKey::WordSpacing);
    let v = PropertyValue::WordSpacing(LengthOrNormal::Length(Length::Px(2.0)));
    assert_eq!(v.key(), PropertyKey::WordSpacing);
}

// ── white-space (CSS Text Module Level 3 §3) ──
//
// Value grammar (§3 spec verbatim, full property grammar): `normal | pre
// | nowrap | pre-wrap | break-spaces | pre-line`. This crate implements
// only normal / pre / nowrap / pre-wrap / pre-line (`WhiteSpace` doc's
// "Scope carving" section) — the 6th keyword `break-spaces` is excluded.
// Initial: normal / Inherited: yes / Computed value: specified keyword.

#[test]
fn white_space_parse_all_implemented_keywords() {
    assert_eq!(
        parse("normal", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::Normal))
    );
    assert_eq!(
        parse("pre", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::Pre))
    );
    assert_eq!(
        parse("nowrap", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::Nowrap))
    );
    assert_eq!(
        parse("pre-wrap", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::PreWrap))
    );
    assert_eq!(
        parse("pre-line", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::PreLine))
    );
}

#[test]
fn white_space_is_case_insensitive() {
    assert_eq!(
        parse("NORMAL", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::Normal))
    );
    assert_eq!(
        parse("Pre-Wrap", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::PreWrap))
    );
    assert_eq!(
        parse("PRE-LINE", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::PreLine))
    );
}

#[test]
fn white_space_accepts_break_spaces() {
    assert_eq!(
        parse("break-spaces", "white-space"),
        Some(PropertyValue::WhiteSpace(WhiteSpace::BreakSpaces))
    );
}

#[test]
fn white_space_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "white-space"), None);
}

#[test]
fn white_space_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "white-space"), None);
    }
}

#[test]
fn white_space_rejects_non_ident() {
    assert_eq!(parse("16px", "white-space"), None);
    assert_eq!(parse(r#""pre""#, "white-space"), None);
}

#[test]
fn white_space_key_maps_to_white_space_property_key() {
    let v = PropertyValue::WhiteSpace(WhiteSpace::Normal);
    assert_eq!(v.key(), PropertyKey::WhiteSpace);
    let v = PropertyValue::WhiteSpace(WhiteSpace::Pre);
    assert_eq!(v.key(), PropertyKey::WhiteSpace);
    let v = PropertyValue::WhiteSpace(WhiteSpace::Nowrap);
    assert_eq!(v.key(), PropertyKey::WhiteSpace);
    let v = PropertyValue::WhiteSpace(WhiteSpace::PreWrap);
    assert_eq!(v.key(), PropertyKey::WhiteSpace);
    let v = PropertyValue::WhiteSpace(WhiteSpace::PreLine);
    assert_eq!(v.key(), PropertyKey::WhiteSpace);
}

// ── hyphens (CSS Text 3 §5.3) ──
//
// grammar: `none | manual | auto` (`Hyphens` doc's Scope carving section
// — this crate implements all 3 spec-valid keywords, unlike `word-break`
// or `white-space`). Initial: manual / Inherited: yes / Computed value:
// specified keyword.

#[test]
fn hyphens_parse_all_implemented_keywords() {
    assert_eq!(
        parse("none", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::None))
    );
    assert_eq!(
        parse("manual", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::Manual))
    );
    // `auto` parses to its own distinct variant, not `Hyphens::Manual`
    // — the collapse to soft-hyphen-only splitting is a downstream
    // consumer decision (`Hyphens` doc's "Downstream handoff" section),
    // not something this parser (or the computed value) performs.
    assert_eq!(
        parse("auto", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::Auto))
    );
}

#[test]
fn hyphens_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::None))
    );
    assert_eq!(
        parse("Manual", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::Manual))
    );
    assert_eq!(
        parse("AUTO", "hyphens"),
        Some(PropertyValue::Hyphens(Hyphens::Auto))
    );
}

#[test]
fn hyphens_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "hyphens"), None);
}

#[test]
fn hyphens_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "hyphens"), None);
    }
}

#[test]
fn hyphens_rejects_non_ident() {
    assert_eq!(parse("16px", "hyphens"), None);
    assert_eq!(parse(r#""manual""#, "hyphens"), None);
}

#[test]
fn hyphens_key_maps_to_hyphens_property_key() {
    let v = PropertyValue::Hyphens(Hyphens::None);
    assert_eq!(v.key(), PropertyKey::Hyphens);
    let v = PropertyValue::Hyphens(Hyphens::Manual);
    assert_eq!(v.key(), PropertyKey::Hyphens);
    let v = PropertyValue::Hyphens(Hyphens::Auto);
    assert_eq!(v.key(), PropertyKey::Hyphens);
}

// ── resolve_overflow (CSS Overflow 3 §3.1 cross-axis computed-value
// coupling) ──
//
// Spec verbatim: "The visible/clip values of overflow compute to
// auto/hidden (respectively) if one of overflow-x or overflow-y is
// neither visible nor clip."

#[test]
fn resolve_overflow_both_visible_is_unaffected() {
    let pair = OverflowXY::both(OverflowValue::Visible);
    assert_eq!(resolve_overflow(pair), pair);
}

#[test]
fn resolve_overflow_visible_x_computes_to_auto_when_y_is_hidden() {
    let pair = OverflowXY {
        x: OverflowValue::Visible,
        y: OverflowValue::Hidden,
    };
    assert_eq!(
        resolve_overflow(pair),
        OverflowXY {
            x: OverflowValue::Auto,
            y: OverflowValue::Hidden,
        }
    );
}

#[test]
fn resolve_overflow_clip_x_computes_to_hidden_when_y_is_scroll() {
    let pair = OverflowXY {
        x: OverflowValue::Clip,
        y: OverflowValue::Scroll,
    };
    assert_eq!(
        resolve_overflow(pair),
        OverflowXY {
            x: OverflowValue::Hidden,
            y: OverflowValue::Scroll,
        }
    );
}

#[test]
fn resolve_overflow_visible_and_clip_do_not_gate_each_other() {
    // The gate condition is "the *other* axis is neither visible nor
    // clip" — `visible` and `clip` are each themselves one of the two
    // values the gate exempts, so pairing them together never satisfies
    // the condition for either axis. Both stay as specified.
    let pair = OverflowXY {
        x: OverflowValue::Visible,
        y: OverflowValue::Clip,
    };
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        resolve_overflow(pair),
        pair,
        "visible/clip do not gate each other"
    );
}

#[test]
fn resolve_overflow_non_visible_non_clip_values_pass_through_unchanged() {
    // `hidden`/`scroll`/`auto` are not rewritten by the coupling
    // regardless of the other axis's value (the rule only ever rewrites
    // `visible`/`clip`).
    for this in [
        OverflowValue::Hidden,
        OverflowValue::Scroll,
        OverflowValue::Auto,
    ] {
        for other in [
            OverflowValue::Visible,
            OverflowValue::Hidden,
            OverflowValue::Clip,
            OverflowValue::Scroll,
            OverflowValue::Auto,
        ] {
            let pair = OverflowXY { x: this, y: other };
            assert_eq!(resolve_overflow(pair).x, this);
        }
    }
}

#[test]
fn resolve_overflow_both_non_visible_non_clip_is_unaffected() {
    let pair = OverflowXY {
        x: OverflowValue::Scroll,
        y: OverflowValue::Auto,
    };
    assert_eq!(resolve_overflow(pair), pair);
}

// ── resolve_text_align_match_parent (CSS Text 3 §6.1
// `#valdef-text-align-match-parent`) ──
//
// This is the shared resolver both `SpecifiedValues::finalize` (element
// path) and `cascade::resolve_against_inherited` (page path) funnel into
// — see the function doc for why `apply_value` itself is deliberately
// *not* a caller (the same-node winner-order hazard between `direction`
// and `text-align`).

#[test]
fn match_parent_resolves_start_against_ltr_parent_to_left() {
    assert_eq!(
        resolve_text_align_match_parent(TextAlign::MatchParent, TextAlign::Start, Direction::Ltr),
        TextAlign::Left
    );
}

#[test]
fn match_parent_resolves_start_against_rtl_parent_to_right() {
    assert_eq!(
        resolve_text_align_match_parent(TextAlign::MatchParent, TextAlign::Start, Direction::Rtl),
        TextAlign::Right
    );
}

#[test]
fn match_parent_resolves_end_against_ltr_parent_to_right() {
    assert_eq!(
        resolve_text_align_match_parent(TextAlign::MatchParent, TextAlign::End, Direction::Ltr),
        TextAlign::Right
    );
}

#[test]
fn match_parent_resolves_end_against_rtl_parent_to_left() {
    assert_eq!(
        resolve_text_align_match_parent(TextAlign::MatchParent, TextAlign::End, Direction::Rtl),
        TextAlign::Left
    );
}

#[test]
fn match_parent_copies_non_start_end_parent_value_verbatim() {
    // "behaves the same as inherit" for the non-start/end half — direction
    // plays no role.
    for parent in [
        TextAlign::Left,
        TextAlign::Right,
        TextAlign::Center,
        TextAlign::Justify,
        TextAlign::JustifyAll,
    ] {
        assert_eq!(
            resolve_text_align_match_parent(TextAlign::MatchParent, parent, Direction::Ltr),
            parent
        );
        assert_eq!(
            resolve_text_align_match_parent(TextAlign::MatchParent, parent, Direction::Rtl),
            parent
        );
    }
}

#[test]
fn non_match_parent_specified_values_pass_through_unchanged() {
    // Every other keyword's computed value is "as specified" — the parent
    // args must be ignored entirely.
    for specified in [
        TextAlign::Start,
        TextAlign::End,
        TextAlign::Left,
        TextAlign::Right,
        TextAlign::Center,
        TextAlign::Justify,
        TextAlign::JustifyAll,
    ] {
        assert_eq!(
            resolve_text_align_match_parent(specified, TextAlign::Center, Direction::Rtl),
            specified
        );
    }
}

// ── margin longhand + shorthand (CSS Box 3 §3.1/§3.2) ──
//
// Primary source:
// - #margin-physical (§3.1): `<length-percentage> | auto`, initial 0, non-inherited.
// - #margin-shorthand (§3.2): `<'margin-top'>{1,4}` with 1/2/3/4 value expansion.

#[test]
fn margin_top_parse_px() {
    // Verification 3-a: `margin-top: 10px` → MarginTop(Length(Px(10)))。
    assert_eq!(
        parse("10px", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            10.0
        ))))
    );
}

#[test]
fn margin_right_parse_auto() {
    // Verification 3-b: `margin-right: auto` → MarginRight(Auto)。§3.1 の
    // `auto` alternative の受理を per-side longhand で pin。
    assert_eq!(
        parse("auto", "margin-right"),
        Some(PropertyValue::MarginRight(LengthOrAuto::Auto))
    );
}

#[test]
fn margin_bottom_parse_percentage() {
    // Verification 3-c: `margin-bottom: 50%` → MarginBottom(Length(Percent(50)))。
    assert_eq!(
        parse("50%", "margin-bottom"),
        Some(PropertyValue::MarginBottom(LengthOrAuto::Length(
            Length::Percent(50.0)
        )))
    );
}

#[test]
fn margin_left_parse_em() {
    // Verification 3-d: `margin-left: 2em` → MarginLeft(Length(Em(2)))。
    // `<length-percentage>` mode 経由で em 受理 (parse_length_value の mode
    // arg = true)。
    assert_eq!(
        parse("2em", "margin-left"),
        Some(PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Em(
            2.0
        ))))
    );
}

#[test]
fn margin_side_accepts_negative_length() {
    // Task Non-goals: negative margin は spec-valid (§3.1 "Negative values
    // for margin properties are allowed")。longhand も含めて受理を pin。
    assert_eq!(
        parse("-10px", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            -10.0
        ))))
    );
}

#[test]
fn margin_top_accepts_zero() {
    // spec `<length-percentage> | auto` — 0 は valid length。`0px` は Dimension arm、
    // bare `0` は CSS Values 3 §5 unitless-zero clause の Number arm を通す。
    // margin は non-negative filter を持たないため素通り。
    assert_eq!(
        parse("0px", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            0.0
        ))))
    );
    assert_eq!(
        parse("0", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            0.0
        ))))
    );
}

#[test]
fn margin_side_case_insensitive_auto() {
    // CSS spec: ident keyword は ASCII case-insensitive。`AUTO` 受理を check
    // (expect_ident_matching が case-insensitive の証拠、helper 変更で
    // regression した際の canary)。
    assert_eq!(
        parse("AUTO", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Auto))
    );
}

#[test]
fn margin_side_rejects_unsupported_unit() {
    // `cap` (§6.1.1 font-relative lengths) は現状
    // 未対応 (parse_length_value 側で drop)。`cm` / `lh` / `rlh` は
    // それぞれ受理側へ移った — margin-side helper に非依存で波及ドロップを pin。
    assert_eq!(parse("1cap", "margin-top"), None);
}

#[test]
fn margin_side_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`。
    assert_eq!(
        parse("1lh", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Lh(
            1.0
        ))))
    );
    assert_eq!(
        parse("2rlh", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Rlh(
            2.0
        ))))
    );
}

#[test]
fn margin_side_accepts_absolute_unit() {
    assert_eq!(
        parse("1cm", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Cm(
            1.0
        ))))
    );
}

#[test]
fn margin_side_rejects_bogus_ident() {
    // `<length-percentage> | auto` grammar 外 ident は declaration drop。
    assert_eq!(parse("fill-available", "margin-top"), None);
    assert_eq!(parse("initial", "margin-top"), None);
}

#[test]
fn margin_shorthand_one_value_spreads_all_sides() {
    // Verification 4-a (§3.2 "If there is only one component value, it
    // applies to all sides"): `margin: 10px` → 全 4 side = 10px。
    let want = Sides::all(LengthOrAuto::Length(Length::Px(10.0)));
    assert_eq!(parse("10px", "margin"), Some(PropertyValue::Margin(want)));
}

#[test]
fn margin_shorthand_two_values_top_bottom_and_right_left() {
    // Verification 4-b (§3.2 "If there are two values, the top and bottom
    // margins are set to the first value and the right and left margins
    // are set to the second"): top/bottom = 10px, right/left = 20px。
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(10.0)),
        right: LengthOrAuto::Length(Length::Px(20.0)),
        bottom: LengthOrAuto::Length(Length::Px(10.0)),
        left: LengthOrAuto::Length(Length::Px(20.0)),
    };
    assert_eq!(
        parse("10px 20px", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_shorthand_three_values_top_horiz_bottom() {
    // Verification 4-c (§3.2 "If there are three values, the top is set to
    // the first value, the left and right are set to the second, and the
    // bottom is set to the third"): top = 10px, right/left = 20px,
    // bottom = 30px。
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(10.0)),
        right: LengthOrAuto::Length(Length::Px(20.0)),
        bottom: LengthOrAuto::Length(Length::Px(30.0)),
        left: LengthOrAuto::Length(Length::Px(20.0)),
    };
    assert_eq!(
        parse("10px 20px 30px", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_shorthand_four_values_clockwise() {
    // Verification 4-d (§3.2 "If there are four values they apply to the
    // top, right, bottom, and left, respectively"): clockwise from top。
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(10.0)),
        right: LengthOrAuto::Length(Length::Px(20.0)),
        bottom: LengthOrAuto::Length(Length::Px(30.0)),
        left: LengthOrAuto::Length(Length::Px(40.0)),
    };
    assert_eq!(
        parse("10px 20px 30px 40px", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_shorthand_all_auto() {
    // Verification 5-a: `margin: auto` (1 value auto) → 全 4 side = Auto。
    // browser の "block-level centering" 慣用の parse pin。
    let want = Sides::all(LengthOrAuto::Auto);
    assert_eq!(parse("auto", "margin"), Some(PropertyValue::Margin(want)));
}

#[test]
fn margin_shorthand_zero_and_auto_horizontal_center() {
    // Verification 5-b: `margin: 0 auto` (2 value mixed) は block-level
    // horizontal centering の canonical form。top/bottom = 0px, right/left = auto。
    // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths> の
    // unitless-zero clause により bare `0` は Length::Px(0.0) 受理
    // (`parse_length_value_accepts_unitless_zero_only`
    // で pin)。
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(0.0)),
        right: LengthOrAuto::Auto,
        bottom: LengthOrAuto::Length(Length::Px(0.0)),
        left: LengthOrAuto::Auto,
    };
    assert_eq!(parse("0 auto", "margin"), Some(PropertyValue::Margin(want)));
}

#[test]
fn margin_shorthand_mixed_units() {
    // grammar coverage: 4-value shorthand で unit / auto を全て混在させる。
    // 32n `<length-percentage> | auto` の grammar 網羅を単一 assertion に集約。
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(10.0)),
        right: LengthOrAuto::Auto,
        bottom: LengthOrAuto::Length(Length::Percent(50.0)),
        left: LengthOrAuto::Length(Length::Em(2.0)),
    };
    assert_eq!(
        parse("10px auto 50% 2em", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_shorthand_rejects_empty_input() {
    // 空 value: parse_margin_side 1st fail → parse_margin_shorthand `?`
    // 上位伝播で None (declaration drop)。
    assert_eq!(parse("", "margin"), None);
}

#[test]
fn margin_shorthand_rejects_bogus_ident() {
    // 1st value 位置に grammar 外 ident → declaration drop。
    assert_eq!(parse("bogus", "margin"), None);
}

#[test]
fn margin_shorthand_leaves_extra_values_for_caller_exhausted_check() {
    // 5+ value shorthand: 本 helper は 4 value 消費、5th 以降は unconsumed で
    // return。DeclParser::parse_value の expect_exhausted で最終的に
    // declaration drop されるので、rule.rs 側 test
    // (`margin_shorthand_five_values_declaration_dropped`) で end-to-end
    // 挙動を check する。本 test は parse_value 単体 (caller expect_exhausted
    // 経由なし) では 4 value までは Some が返る shape の pin。
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(10.0)),
        right: LengthOrAuto::Length(Length::Px(20.0)),
        bottom: LengthOrAuto::Length(Length::Px(30.0)),
        left: LengthOrAuto::Length(Length::Px(40.0)),
    };
    assert_eq!(
        parse("10px 20px 30px 40px 50px", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_shorthand_case_insensitive_auto_and_units() {
    // shorthand path でも case-insensitive dispatch が生きている pin。
    let want = Sides {
        top: LengthOrAuto::Auto,
        right: LengthOrAuto::Length(Length::Px(20.0)),
        bottom: LengthOrAuto::Auto,
        left: LengthOrAuto::Length(Length::Px(20.0)),
    };
    assert_eq!(
        parse("AUTO 20PX", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_longhand_keys_map_correctly() {
    // 4 longhand + shorthand variant → 対応 key (cascade winner 選択の
    // discriminant integrity)。sibling `position_key_maps_to_position_property_key`
    // と同 pattern。shorthand `Margin` key も expansion 前の PropertyValue
    // 段で観測可能なため includes。
    let top = PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(1.0)));
    assert_eq!(top.key(), PropertyKey::MarginTop);
    let right = PropertyValue::MarginRight(LengthOrAuto::Length(Length::Px(1.0)));
    assert_eq!(right.key(), PropertyKey::MarginRight);
    let bottom = PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Px(1.0)));
    assert_eq!(bottom.key(), PropertyKey::MarginBottom);
    let left = PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(1.0)));
    assert_eq!(left.key(), PropertyKey::MarginLeft);
    let shorthand = PropertyValue::Margin(Sides::all(LengthOrAuto::Length(Length::Px(1.0))));
    assert_eq!(shorthand.key(), PropertyKey::Margin);
}

#[test]
fn sides_all_spreads_value_to_all_four() {
    // Sides::all helper ( reused by shorthand 1-value + initial value):
    // 1 value → top/right/bottom/left が全て同値、Clone 経路 (最後の side は
    // move 消費) が正しく動く pin。
    let s = Sides::all(LengthOrAuto::Length(Length::Px(3.5)));
    assert_eq!(s.top, LengthOrAuto::Length(Length::Px(3.5)));
    assert_eq!(s.right, LengthOrAuto::Length(Length::Px(3.5)));
    assert_eq!(s.bottom, LengthOrAuto::Length(Length::Px(3.5)));
    assert_eq!(s.left, LengthOrAuto::Length(Length::Px(3.5)));
}

// ── CSS Logical Properties and Values 1 §4.2/§4.4 margin-inline-*/
//    margin-block-*/padding-inline-*/padding-block-* longhand +
//    margin-inline/margin-block/padding-inline/padding-block shorthand ──
//
// Physical fixed-mapping rationale (writing-mode always HorizontalTb
// since raikiri does not implement a vertical-writing rendering
// pipeline, inline axis additionally assumes `direction: ltr`) is
// `PropertyValue::PaddingInline` doc's canonical record — not repeated
// per test here.

#[test]
fn margin_inline_start_parses_to_margin_left() {
    // §4.2 physical fixed-mapping: `margin-inline-start` produces the
    // exact same `PropertyValue` as `margin-left` (no dedicated variant
    // — `PropertyValue::PaddingInline` doc's "なぜ 8 longhand が専用
    // variant を持たないか" section).
    assert_eq!(
        parse("5px", "margin-inline-start"),
        Some(PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(
            5.0
        ))))
    );
}

#[test]
fn margin_inline_end_parses_to_margin_right() {
    assert_eq!(
        parse("auto", "margin-inline-end"),
        Some(PropertyValue::MarginRight(LengthOrAuto::Auto))
    );
}

#[test]
fn margin_block_start_parses_to_margin_top() {
    assert_eq!(
        parse("5px", "margin-block-start"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            5.0
        ))))
    );
}

#[test]
fn margin_block_end_parses_to_margin_bottom() {
    assert_eq!(
        parse("auto", "margin-block-end"),
        Some(PropertyValue::MarginBottom(LengthOrAuto::Auto))
    );
}

#[test]
fn padding_inline_start_parses_to_padding_left() {
    assert_eq!(
        parse("5px", "padding-inline-start"),
        Some(PropertyValue::PaddingLeft(Length::Px(5.0)))
    );
}

#[test]
fn padding_inline_end_parses_to_padding_right() {
    assert_eq!(
        parse("5%", "padding-inline-end"),
        Some(PropertyValue::PaddingRight(Length::Percent(5.0)))
    );
}

#[test]
fn padding_block_start_parses_to_padding_top() {
    assert_eq!(
        parse("5px", "padding-block-start"),
        Some(PropertyValue::PaddingTop(Length::Px(5.0)))
    );
}

#[test]
fn padding_block_end_parses_to_padding_bottom() {
    assert_eq!(
        parse("5px", "padding-block-end"),
        Some(PropertyValue::PaddingBottom(Length::Px(5.0)))
    );
}

#[test]
fn padding_inline_block_start_end_reject_negative() {
    // CSS Box 3 §4.1 "Negative values for padding properties are
    // invalid." applies identically here (`parse_padding_side` reuse).
    assert_eq!(parse("-5px", "padding-inline-start"), None);
    assert_eq!(parse("-5px", "padding-inline-end"), None);
    assert_eq!(parse("-5px", "padding-block-start"), None);
    assert_eq!(parse("-5px", "padding-block-end"), None);
}

#[test]
fn margin_inline_shorthand_one_value_spreads_to_start_and_end() {
    // CSS Logical Properties and Values 1 §4.2 "If only one value is
    // given, it applies to both the start and end edges."
    assert_eq!(
        parse("12px", "margin-inline"),
        Some(PropertyValue::MarginInline(StartEnd::both(
            LengthOrAuto::Length(Length::Px(12.0))
        )))
    );
}

#[test]
fn margin_inline_shorthand_two_value() {
    assert_eq!(
        parse("5px auto", "margin-inline"),
        Some(PropertyValue::MarginInline(StartEnd {
            start: LengthOrAuto::Length(Length::Px(5.0)),
            end: LengthOrAuto::Auto,
        }))
    );
}

#[test]
fn margin_block_shorthand_two_value() {
    assert_eq!(
        parse("auto 5px", "margin-block"),
        Some(PropertyValue::MarginBlock(StartEnd {
            start: LengthOrAuto::Auto,
            end: LengthOrAuto::Length(Length::Px(5.0)),
        }))
    );
}

#[test]
fn padding_inline_shorthand_one_value_spreads_to_start_and_end() {
    assert_eq!(
        parse("12px", "padding-inline"),
        Some(PropertyValue::PaddingInline(StartEnd::both(Length::Px(
            12.0
        ))))
    );
}

#[test]
fn padding_inline_shorthand_two_value() {
    assert_eq!(
        parse("5px 10px", "padding-inline"),
        Some(PropertyValue::PaddingInline(StartEnd {
            start: Length::Px(5.0),
            end: Length::Px(10.0),
        }))
    );
}

#[test]
fn padding_block_shorthand_two_value() {
    assert_eq!(
        parse("5px 10px", "padding-block"),
        Some(PropertyValue::PaddingBlock(StartEnd {
            start: Length::Px(5.0),
            end: Length::Px(10.0),
        }))
    );
}

#[test]
fn padding_inline_shorthand_rejects_negative_component() {
    // `padding-inline: 10px -5px` — same shape as
    // `padding_shorthand_rejects_any_negative_value`: the 2nd component
    // (`-5px`) fails §4.1's `[0,∞]` constraint, `try_parse` rewinds, and
    // `parse_padding_logical_shorthand` returns the 1-value form
    // (`Some(StartEnd::both(10px))`) with `-5px` left unconsumed — it is
    // the caller's `expect_exhausted` (`crate::rule::parse_declaration_block`)
    // that detects the leftover token and drops the whole declaration.
    // At the bare `parse_value` level (this file's `parse` test helper,
    // which never runs `expect_exhausted`), the 1-value form is the
    // *correct* observed value, not a bug — pinned directly below so a
    // reader doesn't mistake it for one.
    assert_eq!(
        parse("10px -5px", "padding-inline"),
        Some(PropertyValue::PaddingInline(StartEnd::both(Length::Px(
            10.0
        ))))
    );
    let decls = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding-inline: 10px -5px;",
    )));
    // cov:ignore: the failure-message branch of this `assert!` only
    // executes when the assertion fails; it passes here, so llvm-cov
    // reports the macro's condition-false region as an uncovered added
    // line even though the assertion itself runs and does its job.
    assert!(
        decls.is_empty(),
        "`padding-inline: 10px -5px` must drop via expect_exhausted leftover"
    );
    // 1st component negative — no rewind opportunity, `?` propagates
    // `None` directly from `parse_padding_logical_shorthand` itself.
    assert_eq!(parse("-5px 10px", "padding-inline"), None);
}

#[test]
fn margin_inline_shorthand_leaves_extra_values_for_caller_exhausted_check() {
    // 3rd+ value: helper consumes only 2, leftover is unconsumed (caller
    // `expect_exhausted` drops the whole declaration at the rule.rs
    // layer — same shape as `margin_shorthand_leaves_extra_values_for_caller_exhausted_check`).
    assert_eq!(
        parse("5px 10px 15px", "margin-inline"),
        Some(PropertyValue::MarginInline(StartEnd {
            start: LengthOrAuto::Length(Length::Px(5.0)),
            end: LengthOrAuto::Length(Length::Px(10.0)),
        }))
    );
}

#[test]
fn logical_margin_padding_keys_map_to_their_physical_counterparts() {
    // cascade winner selection の discriminant integrity — 8 longhand は
    // 専用 key を持たず物理 key へ写像 (`margin_longhand_keys_map_correctly`
    // と同 pattern)。4 shorthand は自分専用の key を持つ。
    assert_eq!(
        PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(1.0))).key(),
        PropertyKey::MarginLeft
    );
    assert_eq!(
        PropertyValue::MarginInline(StartEnd::both(LengthOrAuto::Length(Length::Px(1.0)))).key(),
        PropertyKey::MarginInline
    );
    assert_eq!(
        PropertyValue::MarginBlock(StartEnd::both(LengthOrAuto::Length(Length::Px(1.0)))).key(),
        PropertyKey::MarginBlock
    );
    assert_eq!(
        PropertyValue::PaddingInline(StartEnd::both(Length::Px(1.0))).key(),
        PropertyKey::PaddingInline
    );
    assert_eq!(
        PropertyValue::PaddingBlock(StartEnd::both(Length::Px(1.0))).key(),
        PropertyKey::PaddingBlock
    );
}

#[test]
fn logical_margin_padding_property_names_resolve_to_physical_property_keys() {
    // `property_key_for_name` — the deferred (`var()`) path's key
    // lookup (`parse_value`'s deferred-detection branch) must agree with
    // `parse_value`'s own non-deferred arm for every logical longhand,
    // or `resolve_deferred_value`'s `value.key() == key` optimized path
    // (`cascade::project_deferred_value` doc) silently breaks.
    assert_eq!(
        property_key_for_name("margin-inline-start"),
        Some(PropertyKey::MarginLeft)
    );
    assert_eq!(
        property_key_for_name("margin-inline-end"),
        Some(PropertyKey::MarginRight)
    );
    assert_eq!(
        property_key_for_name("margin-block-start"),
        Some(PropertyKey::MarginTop)
    );
    assert_eq!(
        property_key_for_name("margin-block-end"),
        Some(PropertyKey::MarginBottom)
    );
    assert_eq!(
        property_key_for_name("padding-inline-start"),
        Some(PropertyKey::PaddingLeft)
    );
    assert_eq!(
        property_key_for_name("padding-inline-end"),
        Some(PropertyKey::PaddingRight)
    );
    assert_eq!(
        property_key_for_name("padding-block-start"),
        Some(PropertyKey::PaddingTop)
    );
    assert_eq!(
        property_key_for_name("padding-block-end"),
        Some(PropertyKey::PaddingBottom)
    );
    assert_eq!(
        property_key_for_name("margin-inline"),
        Some(PropertyKey::MarginInline)
    );
    assert_eq!(
        property_key_for_name("margin-block"),
        Some(PropertyKey::MarginBlock)
    );
    assert_eq!(
        property_key_for_name("padding-inline"),
        Some(PropertyKey::PaddingInline)
    );
    assert_eq!(
        property_key_for_name("padding-block"),
        Some(PropertyKey::PaddingBlock)
    );
}

// ── border longhand + shorthand (CSS Backgrounds 3 §3) ──

#[test]
fn border_top_width_parse_px() {
    // Verification #1: parse("1px", "border-top-width") =
    // Some(PropertyValue::BorderTopWidth(Length::Px(1.0)))。
    assert_eq!(
        parse("1px", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Px(1.0)))
    );
}

// ── width (CSS Sizing 3 §3.1.1) ────────────────────────
//
// Primary source:
// https://www.w3.org/TR/css-sizing-3/#preferred-size-properties
// Value: `auto | <length-percentage [0,∞]> | min-content | max-content |
//         fit-content(<length-percentage>)`
// Initial: auto、Inherited: no。
//
// 本 task では `auto` + non-negative `<length-percentage>` のみ受理、
// min-content / max-content / fit-content() は (b) 非対応。

#[test]
fn width_parse_auto_keyword() {
    // Verification #1: `width: auto` → Width(Auto)。initial 値と同 shape で
    // grammar 上位優先分岐 (parse_width の try_parse ident branch) が生きて
    // いることを pin。
    assert_eq!(
        parse("auto", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn border_top_width_parse_medium_keyword() {
    // Verification #2: parse("medium", "border-top-width") =
    // Some(PropertyValue::BorderTopWidth(Length::Px(3.0)))。
    // spec §3.3 規定値 (normative equivalence) の 1/3/5 px mapping。
    assert_eq!(
        parse("medium", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Px(3.0)))
    );
}

#[test]
fn width_parse_length_px() {
    // Verification #2: `width: 100px` → Width(Length(Px(100)))。
    // 従来 `unknown_property_returns_none` canary で `None` だった箇所が
    // 実 variant を返すようになった transition check (canary はその後
    // `float` を経て `cursor` に移設済み)。
    assert_eq!(
        parse("100px", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(
            100.0
        ))))
    );
}

#[test]
fn border_width_thin_thick_keywords_map_to_1px_5px() {
    // spec §3.3 規定値: thin=1px、thick=5px。
    // 4 side 各 arm の smoke — arm cross-copy regression check (`top` arm を
    // `right` arm に誤 wire しても本 test で fail する)。
    assert_eq!(
        parse("thin", "border-right-width"),
        Some(PropertyValue::BorderRightWidth(Length::Px(1.0)))
    );
    assert_eq!(
        parse("thick", "border-left-width"),
        Some(PropertyValue::BorderLeftWidth(Length::Px(5.0)))
    );
}

#[test]
fn width_parse_length_percentage() {
    // Verification #3: `width: 50%` → Width(Length(Percent(50)))。
    // parse_length_value(allow_percentage=true) 経由の Percent branch。
    assert_eq!(
        parse("50%", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Percent(
            50.0
        ))))
    );
}

#[test]
fn border_width_rejects_negative() {
    // Verification #6: parse("-1px", "border-top-width") = None。
    // spec `<line-width>` = `<length [0,∞]>` — 負値は grammar 違反 → drop。
    assert_eq!(parse("-1px", "border-top-width"), None);
    // Em / Rem / Pt も同 constraint (unit-bearing variant 全て)。
    assert_eq!(parse("-1em", "border-bottom-width"), None);
}

#[test]
fn border_top_width_accepts_zero() {
    // spec `<line-width>` = `<length [0,∞]>` — 0 は閉区間下端。`0px` は
    // Dimension arm、bare `0` は CSS Values 3 §5 unitless-zero clause の
    // Number arm を通す。parse_border_width_side の
    // `>= 0.0` 非負 filter を pass。
    assert_eq!(
        parse("0px", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Px(0.0)))
    );
    assert_eq!(
        parse("0", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Px(0.0)))
    );
}

#[test]
fn border_shorthand_accepts_bare_zero_width() {
    // Follow-on coverage: `0 solid`
    // は shorthand の width slot を bare-zero で埋めた canonical form。
    // parse_border_shorthand の width slot が parse_border_width_side_res 経由で
    // parse_length_value Number arm を通して Length::Px(0.0) を取り、
    // style slot は Solid、color slot は省略で spec initial =
    // `BorderColor::CurrentColor` (CSS Backgrounds 3 §3.1)。
    let border = Border {
        width: Length::Px(0.0),
        style: BorderStyle::Solid,
        color: BorderColor::CurrentColor,
    };
    assert_eq!(
        parse("0 solid", "border"),
        Some(PropertyValue::Border(Sides::all(border)))
    );
}

#[test]
fn border_width_rejects_percentage() {
    // `<line-width>` grammar は `<percentage>` を含まない (padding とは
    // 違う点)。`parse_length_value(input, false)` の
    // `<length>` mode で Percentage token 自体が reject される。
    assert_eq!(parse("50%", "border-top-width"), None);
}

#[test]
fn border_width_rejects_unknown_keyword() {
    // spec §3.3 の keyword 集合外は drop (`auto` / `fat` / `bold` etc.)。
    assert_eq!(parse("auto", "border-top-width"), None);
    assert_eq!(parse("fat", "border-top-width"), None);
}

#[test]
fn border_width_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
    // canonical: PropertyValue doc「CSS-wide keyword」節。
    assert_eq!(parse("inherit", "border-top-width"), None);
    assert_eq!(parse("initial", "border-top-width"), None);
    assert_eq!(parse("unset", "border-top-width"), None);
    assert_eq!(parse("revert", "border-top-width"), None);
    assert_eq!(parse("revert-layer", "border-top-width"), None);
}

#[test]
fn border_width_accepts_absolute_unit() {
    // `<line-width>` の `<length [0,∞]>` half は `<percentage>` を含まないが
    // 他 absolute unit は含む — 追加した `pc` を
    // border-width 経路 (`allow_percentage=false`) でも check する。
    assert_eq!(
        parse("1pc", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Pc(1.0)))
    );
}

#[test]
fn border_width_rejects_negative_absolute_unit() {
    // 全 unit-bearing variant の non-negative check check (cm、新規 absolute unit)。
    assert_eq!(parse("-1cm", "border-top-width"), None);
}

#[test]
fn border_width_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`。`<line-width>`
    // grammar (`<length [0,∞]> | thin | medium | thick`) has no
    // self-reference concern the way `font-size` / `line-height` do
    // (`Length::Lh` doc), so `border-*-width` accepts them unfiltered.
    assert_eq!(
        parse("2lh", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Lh(2.0)))
    );
    assert_eq!(
        parse("1rlh", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Rlh(1.0)))
    );
}

#[test]
fn border_style_shorthand_expansion_1_to_4_values() {
    use PropertyValue::BorderStyle as BS;
    // 1 value → all sides.
    assert_eq!(
        parse("double", "border-style"),
        Some(BS(Sides::all(BorderStyle::Double)))
    );
    // 2 values → vertical / horizontal.
    assert_eq!(
        parse("solid dotted", "border-style"),
        Some(BS(Sides {
            top: BorderStyle::Solid,
            right: BorderStyle::Dotted,
            bottom: BorderStyle::Solid,
            left: BorderStyle::Dotted,
        }))
    );
    // 4 values → clockwise.
    assert_eq!(
        parse("solid dotted dashed double", "border-style"),
        Some(BS(Sides {
            top: BorderStyle::Solid,
            right: BorderStyle::Dotted,
            bottom: BorderStyle::Dashed,
            left: BorderStyle::Double,
        }))
    );
    // Invalid keyword drops the whole declaration (exhaustion
    // enforced by the caller — `parse_entire` mirrors DeclParser).
    assert_eq!(parse_entire("solid wavy", "border-style"), None);
    assert_eq!(parse("", "border-style"), None);
}

#[test]
fn border_width_shorthand_keywords_and_lengths() {
    use PropertyValue::BorderWidth as BW;
    assert_eq!(
        parse("medium", "border-width"),
        Some(BW(Sides::all(Length::Px(BORDER_WIDTH_MEDIUM_PX))))
    );
    assert_eq!(
        parse("1px 2px", "border-width"),
        Some(BW(Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(1.0),
            left: Length::Px(2.0),
        }))
    );
    // Negative lengths are grammar violations.
    assert_eq!(parse("-1px", "border-width"), None);
}

#[test]
fn border_color_shorthand_currentcolor_and_named() {
    use PropertyValue::BorderColor as BC;
    assert_eq!(
        parse("black", "border-color"),
        Some(BC(Sides::all(BorderColor::Resolved(CssColor::BLACK))))
    );
    assert_eq!(
        parse("currentcolor", "border-color"),
        Some(BC(Sides::all(BorderColor::CurrentColor)))
    );
}

#[test]
fn border_top_style_parse_solid() {
    // Verification #3: parse("solid", "border-top-style") =
    // Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))。
    assert_eq!(
        parse("solid", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))
    );
}

#[test]
fn width_parse_length_em() {
    // font-relative unit 経路 check — parse_length_value 経由で em を受理。
    assert_eq!(
        parse("2em", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Em(2.0))))
    );
}

#[test]
fn border_style_all_10_variants_accepted() {
    // spec §3.2 `<line-style>` の 10 alternative 全てを smoke (arm 削り
    // regression 検知)。
    assert_eq!(
        parse("none", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::None))
    );
    assert_eq!(
        parse("hidden", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Hidden))
    );
    assert_eq!(
        parse("dotted", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Dotted))
    );
    assert_eq!(
        parse("dashed", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Dashed))
    );
    assert_eq!(
        parse("double", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Double))
    );
    assert_eq!(
        parse("groove", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Groove))
    );
    assert_eq!(
        parse("ridge", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Ridge))
    );
    assert_eq!(
        parse("inset", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Inset))
    );
    assert_eq!(
        parse("outset", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Outset))
    );
}

#[test]
fn width_rejects_negative_px() {
    // Verification #4: `width: -10px` → None (spec grammar `[0,∞]` violation)。
    // parse_width の post-filter が enforce (padding と同 pattern)。
    assert_eq!(parse("-10px", "width"), None);
}

#[test]
fn width_rejects_negative_percentage() {
    // 全 Length variant OR-pattern check の check (Percent 分岐)。
    assert_eq!(parse("-50%", "width"), None);
}

#[test]
fn width_rejects_negative_em() {
    // 全 Length variant OR-pattern check の check (Em 分岐)。
    assert_eq!(parse("-2em", "width"), None);
}

#[test]
fn width_accepts_zero() {
    // spec `[0,∞]` の closed interval — 下端 0 は有効。
    assert_eq!(
        parse("0px", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(0.0))))
    );
    // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths>
    // unitless-zero clause 経由: bare `0` も同 Px(0.0)
    // として受理 (width は `<length-percentage [0,∞]>`、helper が Number arm で
    // 拾い parse_width の非負 filter を pass)。
    assert_eq!(
        parse("0", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(0.0))))
    );
}

#[test]
fn border_style_rejects_unknown_keyword() {
    // `<line-style>` grammar 外 (`wavy` は CSS Text Decoration 4 由来、
    // border-style では invalid) は drop。
    assert_eq!(parse("wavy", "border-top-style"), None);
}

#[test]
fn border_style_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
    // canonical: PropertyValue doc「CSS-wide keyword」節。
    assert_eq!(parse("inherit", "border-top-style"), None);
    assert_eq!(parse("initial", "border-top-style"), None);
    assert_eq!(parse("unset", "border-top-style"), None);
    assert_eq!(parse("revert", "border-top-style"), None);
    assert_eq!(parse("revert-layer", "border-top-style"), None);
}

#[test]
fn border_style_case_insensitive() {
    // CSS Values 3 §3.1: keyword は ASCII case-insensitive。
    assert_eq!(
        parse("SOLID", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))
    );
}

#[test]
fn width_rejects_min_content_keyword() {
    // Intrinsic sizing keyword — now accepted as valid parsing (placeholder Auto).
    assert_eq!(
        parse("min-content", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn width_rejects_max_content_keyword() {
    assert_eq!(
        parse("max-content", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn width_rejects_fit_content_function() {
    assert_eq!(
        parse("fit-content(50%)", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
    assert_eq!(
        parse("fit-content", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn width_rejects_unsupported_unit() {
    // (b) 非対応 — vw / cap 等は spec-valid だが
    // 未対応、parse_length_value 側で drop、None
    // propagate。`ch` / `lh` / `rlh` は
    // それぞれ受理側へ移った
    // (`width_accepts_absolute_unit` / `width_accepts_lh` 参照)。
    assert_eq!(parse("10vw", "width"), None);
    assert_eq!(parse("5cap", "width"), None);
}

#[test]
fn width_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`。
    assert_eq!(
        parse("5lh", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Lh(5.0))))
    );
    assert_eq!(
        parse("1rlh", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Rlh(1.0))))
    );
}

#[test]
fn width_accepts_absolute_unit() {
    // `1in` = 96px 相当 (specified 層は authored unit をそのまま保持、
    // 絶対化は resolve.rs の責務 — check: `resolve::tests::length_additional_absolute_units_convert_per_spec_table`)。
    assert_eq!(
        parse("1in", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::In(1.0))))
    );
}

#[test]
fn width_case_insensitive_auto() {
    // CSS spec: ident keyword は ASCII case-insensitive。`AUTO` 受理を check
    // (sibling `margin_side_case_insensitive_auto` と同 pattern)。
    assert_eq!(
        parse("AUTO", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn border_top_color_parse_hex() {
    // border-*-color の hex form は `parse_color` (background-color と同じ
    // helper) が hex/named/rgb(a)/transparent を受理し、`parse_border_color`
    // が `BorderColor::Resolved` で wrap して cascade static side に届く。
    assert_eq!(
        parse("#ff0000", "border-top-color"),
        Some(PropertyValue::BorderTopColor(BorderColor::Resolved(
            CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }
        )))
    );
}

#[test]
fn border_color_named_and_rgb() {
    // 4 side 各 arm の smoke + 3 color form (named / rgb / transparent) を
    // 分散して cross-arm regression 検知 (background-color test の pattern)。
    // `BorderColor::Resolved` wrap。
    assert_eq!(
        parse("red", "border-right-color"),
        Some(PropertyValue::BorderRightColor(BorderColor::Resolved(
            CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }
        )))
    );
    assert_eq!(
        parse("rgb(0, 0, 255)", "border-bottom-color"),
        Some(PropertyValue::BorderBottomColor(BorderColor::Resolved(
            CssColor {
                r: 0,
                g: 0,
                b: 255,
                a: 255
            }
        )))
    );
    assert_eq!(
        parse("transparent", "border-left-color"),
        Some(PropertyValue::BorderLeftColor(BorderColor::Resolved(
            CssColor::TRANSPARENT
        )))
    );
}

#[test]
fn border_top_color_parse_currentcolor() {
    // CSS Backgrounds 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color>
    // "Initial: currentcolor" — author 明示 `border-*-color: currentcolor` が
    // `BorderColor::CurrentColor` variant として保持されることを check する
    // (hazard case 1 の cascade-side coverage、used-value resolution は
    // paint scope 責務)。
    assert_eq!(
        parse("currentcolor", "border-top-color"),
        Some(PropertyValue::BorderTopColor(BorderColor::CurrentColor))
    );
    // CSS Color 3 §4.4 keyword は ASCII case-insensitive。
    assert_eq!(
        parse("CurrentColor", "border-right-color"),
        Some(PropertyValue::BorderRightColor(BorderColor::CurrentColor))
    );
    assert_eq!(
        parse("CURRENTCOLOR", "border-bottom-color"),
        Some(PropertyValue::BorderBottomColor(BorderColor::CurrentColor))
    );
}

#[test]
fn border_shorthand_all_three_components() {
    // Verification #5: parse("1px solid red", "border") = shorthand 経由で
    // 全 4 side の Border {width: 1px, style: Solid, color: red} を expand。
    // color slot は `BorderColor::Resolved` に wrap。
    let border = Border {
        width: Length::Px(1.0),
        style: BorderStyle::Solid,
        color: BorderColor::Resolved(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
    };
    assert_eq!(
        parse("1px solid red", "border"),
        Some(PropertyValue::Border(Sides::all(border)))
    );
}

#[test]
fn border_shorthand_any_order() {
    // spec §3.4 grammar は `||` (any-order)。全 6 permutation を check する
    // 代わりに 3 order (color-first / style-first / mixed) を smoke。
    let expected = Border {
        width: Length::Px(2.0),
        style: BorderStyle::Dashed,
        color: BorderColor::Resolved(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
    };
    // color first
    assert_eq!(
        parse("red 2px dashed", "border"),
        Some(PropertyValue::Border(Sides::all(expected)))
    );
    // style first
    assert_eq!(
        parse("dashed 2px red", "border"),
        Some(PropertyValue::Border(Sides::all(expected)))
    );
}

#[test]
fn border_shorthand_omitted_components_use_initial() {
    // spec §3.4 "Omitted values are set to their initial values" —
    // width 省略 → medium (3px)、style 省略 → None、color 省略 →
    // `currentcolor` keyword (`BorderColor::CurrentColor`、spec §3.1
    // initial)。
    // 1 component only (color) — width と style は initial:
    let with_only_color = Border {
        width: Length::Px(3.0), // medium initial
        style: BorderStyle::None,
        color: BorderColor::Resolved(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
    };
    assert_eq!(
        parse("red", "border"),
        Some(PropertyValue::Border(Sides::all(with_only_color)))
    );
    // 1 component only (style) — width と color は initial:
    let with_only_style = Border {
        width: Length::Px(3.0),
        style: BorderStyle::Solid,
        color: BorderColor::CurrentColor, // spec §3.1 initial
    };
    assert_eq!(
        parse("solid", "border"),
        Some(PropertyValue::Border(Sides::all(with_only_style)))
    );
}

#[test]
fn border_width_medium_is_consistent_across_its_independent_call_sites() {
    // Before this fix, `medium` = 3px was written
    // as 3 independent `Length::Px(3.0)` literals — the `medium` keyword
    // branch in `parse_border_width_side`, the border shorthand's
    // omitted-width default in `parse_border_shorthand`, and
    // `crate::specified::INITIAL_BORDER`'s `width` field — with no test
    // tying them together, so they could silently drift apart. All 3 now
    // derive from `BORDER_WIDTH_MEDIUM_PX`; this test exercises all 3
    // through real behavior (not literal-vs-literal) and pins that they
    // still agree with each other and with the const, so a future edit
    // that touches only one of them fails loudly here instead of
    // drifting silently. The sibling tests
    // `border_top_width_parse_medium_keyword` and
    // `border_shorthand_omitted_components_use_initial` independently
    // check the *absolute* value (`3.0`) as a literal — do not fold those
    // into a reference to the const, or nothing catches an accidental
    // edit to the const itself (see the const's doc).
    let via_keyword = parse("medium", "border-top-width");
    assert_eq!(
        via_keyword,
        Some(PropertyValue::BorderTopWidth(Length::Px(
            BORDER_WIDTH_MEDIUM_PX
        )))
    );

    let via_shorthand_omission = parse("solid", "border");
    // cov:ignore: this let-else panic branch is unreached as long as the
    // test passes — `parse("solid", "border")` always matches
    // `Some(PropertyValue::Border(_))`, so llvm-cov marks the panic-message
    // literal "uncovered" the same way it does for any other panic-only
    // branch (same false-positive class as r7r1).
    let Some(PropertyValue::Border(sides)) = via_shorthand_omission else {
        panic!("expected `border: solid` to parse to a Border shorthand value");
    };
    assert_eq!(sides.top.width, Length::Px(BORDER_WIDTH_MEDIUM_PX));

    assert_eq!(
        crate::specified::INITIAL_BORDER.width,
        Length::Px(BORDER_WIDTH_MEDIUM_PX)
    );
}

#[test]
fn border_default_matches_initial_border() {
    // `Border::default()` (public, umbrella-facing
    // constructor) and `crate::specified::INITIAL_BORDER` (`pub(crate)`,
    // cascade-internal optimized path) encode the same CSS Backgrounds 3
    // initial value. Precision on what this actually catches (the sibling
    // test just above, `border_width_medium_is_consistent_across_its_independent_call_sites`,
    // warns explicitly against a "vacuous pin" of this shape):
    //
    // - `style` / `color`: each side hardcodes `BorderStyle::None` /
    //   `BorderColor::CurrentColor` independently (no shared constant), so
    //   this assert is a real independent-literal drift check for those 2
    //   fields — same rationale as the sibling test.
    // - `width`: both sides already read `BORDER_WIDTH_MEDIUM_PX` (this
    //   fn's own body and `INITIAL_BORDER`'s definition), so an edit to
    //   that const moves both sides together and this assert alone would
    //   NOT catch it — that drift is what the sibling test's real,
    //   behavior-driven exercise of the const (plus
    //   `border_top_width_parse_medium_keyword`'s absolute-value literal
    //   pin) already covers. This test's width leg is a
    //   both-must-reference-the-same-const structural check, not an
    //   independent value check — do not treat it as one.
    assert_eq!(Border::default(), crate::specified::INITIAL_BORDER);
}

#[test]
fn border_new_is_default() {
    // `Border::new()` is documented as a thin
    // `Self::default()` wrapper (same shape as
    // `raikiri_traits::page::PageBox::new`) — check that the two stay
    // equivalent.
    assert_eq!(Border::new(), Border::default());
}

#[test]
fn border_shorthand_color_slot_accepts_currentcolor() {
    // sibling: border shorthand の color slot は 4 longhand と同じ
    // `parse_border_color` を経由するため、`currentcolor` keyword も
    // shorthand から受理される。
    let expected = Border {
        width: Length::Px(1.0),
        style: BorderStyle::Solid,
        color: BorderColor::CurrentColor,
    };
    assert_eq!(
        parse("1px solid currentcolor", "border"),
        Some(PropertyValue::Border(Sides::all(expected)))
    );
    // 引数 order は自由。style first。
    assert_eq!(
        parse("solid currentcolor 1px", "border"),
        Some(PropertyValue::Border(Sides::all(expected)))
    );
}

#[test]
fn border_shorthand_empty_returns_none() {
    // `||` grammar は at least 1 component 必須。0 component は None。
    assert_eq!(parse("", "border"), None);
}

#[test]
fn border_shorthand_unknown_keyword_only_returns_none() {
    // 未知 keyword (width/style/color いずれの slot にも match しない) は
    // 1st iteration で全 slot None、`matched=false` で break、0-component
    // guard で None (declaration drop)。
    assert_eq!(parse("garbage", "border"), None);
}

#[test]
fn border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check() {
    // `border: 1px 2px` — 1st iteration で width=1px、2nd iteration で
    // width slot 満了、`2px` は他 slot (style/color) に match しないため
    // fall-through break。leftover は caller の `expect_exhausted` 責務。
    // 本 helper 単体としては 1st を確保して Some を返す (parse_value 経路
    // では end-to-end で declaration drop する — rule.rs test で check 予定)。
    let expected = Border {
        width: Length::Px(1.0),
        style: BorderStyle::None,
        color: BorderColor::CurrentColor, // spec §3.1 initial
    };
    let mut input = ParserInput::new("1px 2px");
    let mut parser = Parser::new(&mut input);
    let result = parse_border_shorthand(&mut parser);
    assert_eq!(result, Some(Sides::all(expected)));
    // 2px は unconsumed のまま — parser cursor は "2px" の直前を指す。
    assert!(!parser.is_exhausted());
}

#[test]
fn border_longhand_keys_map_correctly() {
    // 12 longhand + 1 shorthand variant → 対応 key (cascade winner 選択の
    // discriminant integrity)。sibling `margin_longhand_keys_map_correctly`
    // と同 pattern。
    assert_eq!(
        PropertyValue::BorderTopWidth(Length::Px(1.0)).key(),
        PropertyKey::BorderTopWidth
    );
    assert_eq!(
        PropertyValue::BorderRightWidth(Length::Px(1.0)).key(),
        PropertyKey::BorderRightWidth
    );
    assert_eq!(
        PropertyValue::BorderBottomWidth(Length::Px(1.0)).key(),
        PropertyKey::BorderBottomWidth
    );
    assert_eq!(
        PropertyValue::BorderLeftWidth(Length::Px(1.0)).key(),
        PropertyKey::BorderLeftWidth
    );
    assert_eq!(
        PropertyValue::BorderTopStyle(BorderStyle::Solid).key(),
        PropertyKey::BorderTopStyle
    );
    assert_eq!(
        PropertyValue::BorderRightStyle(BorderStyle::Solid).key(),
        PropertyKey::BorderRightStyle
    );
    assert_eq!(
        PropertyValue::BorderBottomStyle(BorderStyle::Solid).key(),
        PropertyKey::BorderBottomStyle
    );
    assert_eq!(
        PropertyValue::BorderLeftStyle(BorderStyle::Solid).key(),
        PropertyKey::BorderLeftStyle
    );
    assert_eq!(
        PropertyValue::BorderTopColor(BorderColor::CurrentColor).key(),
        PropertyKey::BorderTopColor
    );
    assert_eq!(
        PropertyValue::BorderRightColor(BorderColor::CurrentColor).key(),
        PropertyKey::BorderRightColor
    );
    assert_eq!(
        PropertyValue::BorderBottomColor(BorderColor::CurrentColor).key(),
        PropertyKey::BorderBottomColor
    );
    assert_eq!(
        PropertyValue::BorderLeftColor(BorderColor::CurrentColor).key(),
        PropertyKey::BorderLeftColor
    );
    let default_border = Border {
        width: Length::Px(3.0),
        style: BorderStyle::None,
        color: BorderColor::CurrentColor, // spec §3.1 initial
    };
    assert_eq!(
        PropertyValue::Border(Sides::all(default_border)).key(),
        PropertyKey::Border
    );
}

#[test]
fn width_key_maps_to_width_property_key() {
    // PropertyValue::Width → PropertyKey::Width (cascade winner 選択の
    // discriminant integrity、既存 sibling padding/margin と同じ pattern)。
    assert_eq!(
        PropertyValue::Width(LengthOrAuto::Auto).key(),
        PropertyKey::Width
    );
    assert_eq!(
        PropertyValue::Width(LengthOrAuto::Length(Length::Px(100.0))).key(),
        PropertyKey::Width
    );
}

#[test]
fn logical_size_aliases_use_physical_horizontal_writing_mode_axes() {
    // CSS Sizing 3 logical preferred-size properties map to width/height
    // while this engine's supported writing mode is horizontal-tb.
    assert_eq!(
        property_key_for_name("inline-size"),
        Some(PropertyKey::Width)
    );
    assert_eq!(
        property_key_for_name("block-size"),
        Some(PropertyKey::Height)
    );
    assert_eq!(
        parse("120px", "inline-size"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(
            120.0
        ))))
    );
    assert_eq!(
        parse("80px", "block-size"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(
            80.0
        ))))
    );
}

// ── height (CSS Sizing 3 §3.1.1) ─────────────
//
// Primary source:
// - #preferred-size-properties: `auto | <length-percentage [0,∞]> |
//   min-content | max-content | fit-content(<length-percentage>)`,
//   initial `auto`, Inheritance `No`.
//
// 現状 scope は `auto` + 非負 `<length-percentage>` の 2 分岐のみ、
// 他 sizing keyword / global keyword / calc() / var() は silent drop
// (parse_height doc の Scope carving 節参照)。
//
// sibling: sibling `width` と同 shape の非負 `<length-percentage>` +
// `auto` grammar、payload 型は共通 `LengthOrAuto`。

#[test]
fn height_parse_auto() {
    // Verification 1 (task doc): `auto` ident は spec initial value でもある
    // (§3.1.1 "Initial: auto") — cascade winner として declaration が到達
    // した場合の受理 pattern を pin。
    assert_eq!(
        parse("auto", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
}

#[test]
fn height_parse_px() {
    // Verification 2 (task doc): 非負 px は spec-valid `<length-percentage>`
    // (§3.1.1)。sibling `width_parse_length_px` と同 shape。
    assert_eq!(
        parse("100px", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(
            100.0
        ))))
    );
}

#[test]
fn height_parse_percentage() {
    // Verification 3 (task doc): percentage 受理 (parse_length_value の
    // allow_percentage = true 経路)。resolve (containing block % → 実寸)
    // は下流責務。
    assert_eq!(
        parse("50%", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(
            Length::Percent(50.0)
        )))
    );
}

#[test]
fn height_rejects_negative_length() {
    // Verification 4 (task doc): `<length-percentage [0,∞]>` (§3.1.1) の
    // 非負制約により `-10px` は spec-invalid → drop。sibling
    // padding の非負フィルタ pattern と同 shape、margin の `-10px` 受理
    // (§3.1) との対称的な reject を pin。
    assert_eq!(parse("-10px", "height"), None);
}

#[test]
fn height_accepts_zero() {
    // spec `<length-percentage [0,∞]>` — 0 は閉区間下端。`0px` は Dimension arm、
    // bare `0` は CSS Values 3 §5 unitless-zero clause の Number arm を通す。
    // parse_height の `>= 0.0` 非負 filter を pass。
    assert_eq!(
        parse("0px", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(0.0))))
    );
    assert_eq!(
        parse("0", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(0.0))))
    );
}

#[test]
fn height_rejects_negative_percentage() {
    // 非負フィルタが Percent variant にも効く check (parse_padding_side の
    // 同 pattern、Verification 4 の姉妹)。
    assert_eq!(parse("-10%", "height"), None);
}

#[test]
fn height_rejects_unsupported_sizing_keyword() {
    assert_eq!(
        parse("min-content", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
    assert_eq!(
        parse("max-content", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
    assert_eq!(
        parse("fit-content(50%)", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
    assert_eq!(
        parse("fit-content", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
}

#[test]
fn height_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
    // canonical: PropertyValue doc「CSS-wide keyword」節。
    assert_eq!(parse("inherit", "height"), None);
    assert_eq!(parse("initial", "height"), None);
    assert_eq!(parse("unset", "height"), None);
    assert_eq!(parse("revert", "height"), None);
    assert_eq!(parse("revert-layer", "height"), None);
}

#[test]
fn height_case_insensitive_auto() {
    // CSS spec: ident keyword は ASCII case-insensitive
    // (`expect_ident_matching` の cssparser 慣行、sibling
    // `margin_side_case_insensitive_auto` と同 pattern)。
    assert_eq!(
        parse("AUTO", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
}

#[test]
fn height_rejects_unsupported_unit() {
    // `cap` (§6.1.1 font-relative lengths) は現状
    // 未対応 (parse_length_value 側で drop)。`cm` / `lh` / `rlh` は
    // それぞれ受理側へ移った (`height_accepts_absolute_unit` /
    // `height_accepts_lh` 参照)。sibling
    // `margin_side_rejects_unsupported_unit` と同 pattern。
    assert_eq!(parse("1cap", "height"), None);
}

#[test]
fn height_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`。
    assert_eq!(
        parse("1.5lh", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Lh(1.5))))
    );
    assert_eq!(
        parse("2rlh", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Rlh(
            2.0
        ))))
    );
}

#[test]
fn height_accepts_absolute_unit() {
    // CSS Values 4 §6.2 absolute lengths。
    assert_eq!(
        parse("1cm", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Cm(1.0))))
    );
}

#[test]
fn height_rejects_negative_absolute_unit() {
    assert_eq!(parse("-1cm", "height"), None);
}

#[test]
fn height_parse_em_and_rem() {
    // grammar coverage: font-relative units (`em` / `rem`) も
    // `<length-percentage>` mode で受理される。resolve は下流
    // (font-size context / root font-size context) 責務。
    assert_eq!(
        parse("1.2em", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Em(1.2))))
    );
    assert_eq!(
        parse("2rem", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Rem(
            2.0
        ))))
    );
}

#[test]
fn height_key_maps_to_height_property_key() {
    // sibling `margin_longhand_keys_map_correctly` と同 pattern — cascade
    // winner selection の discriminant integrity を pin。
    let v = PropertyValue::Height(LengthOrAuto::Auto);
    assert_eq!(v.key(), PropertyKey::Height);
    let v = PropertyValue::Height(LengthOrAuto::Length(Length::Px(100.0)));
    assert_eq!(v.key(), PropertyKey::Height);
}

// ── flex-direction (CSS Flexible Box Layout Module Level 1 §5.1) ────

#[test]
fn flex_direction_parse_all_keywords() {
    assert_eq!(
        parse("row", "flex-direction"),
        Some(PropertyValue::FlexDirection(FlexDirectionValue::Row))
    );
    assert_eq!(
        parse("row-reverse", "flex-direction"),
        Some(PropertyValue::FlexDirection(FlexDirectionValue::RowReverse))
    );
    assert_eq!(
        parse("column", "flex-direction"),
        Some(PropertyValue::FlexDirection(FlexDirectionValue::Column))
    );
    assert_eq!(
        parse("column-reverse", "flex-direction"),
        Some(PropertyValue::FlexDirection(
            FlexDirectionValue::ColumnReverse
        ))
    );
}

#[test]
fn flex_direction_case_insensitive() {
    assert_eq!(
        parse("ROW-REVERSE", "flex-direction"),
        Some(PropertyValue::FlexDirection(FlexDirectionValue::RowReverse))
    );
}

#[test]
fn flex_direction_rejects_unknown_ident() {
    assert_eq!(parse("diagonal", "flex-direction"), None);
}

// ── flex-wrap (CSS Flexible Box Layout Module Level 1 §5.2) ─────────

#[test]
fn flex_wrap_parse_all_keywords() {
    assert_eq!(
        parse("nowrap", "flex-wrap"),
        Some(PropertyValue::FlexWrap(FlexWrapValue::NoWrap))
    );
    assert_eq!(
        parse("wrap", "flex-wrap"),
        Some(PropertyValue::FlexWrap(FlexWrapValue::Wrap))
    );
    assert_eq!(
        parse("wrap-reverse", "flex-wrap"),
        Some(PropertyValue::FlexWrap(FlexWrapValue::WrapReverse))
    );
}

#[test]
fn flex_wrap_rejects_unknown_ident() {
    assert_eq!(parse("nowrap-ish", "flex-wrap"), None);
}

// ── flex-grow / flex-shrink (CSS Flexible Box Layout Module Level 1
//    §7.2.1 / §7.2.2) ──────────────────────────────────────────────

#[test]
fn flex_grow_parse_number() {
    assert_eq!(parse("2", "flex-grow"), Some(PropertyValue::FlexGrow(2.0)));
    assert_eq!(parse("0", "flex-grow"), Some(PropertyValue::FlexGrow(0.0)));
    assert_eq!(
        parse("1.5", "flex-grow"),
        Some(PropertyValue::FlexGrow(1.5))
    );
}

#[test]
fn flex_grow_rejects_negative() {
    // spec `<number [0,∞]>` — negative は grammar 違反。
    assert_eq!(parse("-1", "flex-grow"), None);
}

#[test]
fn flex_grow_rejects_non_finite_literal() {
    // f64 → f32 変換で `+Inf` になる巨大 literal
    // (`parse_nonneg_finite_number` doc の hazard 節参照)。
    assert_eq!(parse("1e40", "flex-grow"), None);
}

#[test]
fn flex_shrink_parse_number() {
    assert_eq!(
        parse("3", "flex-shrink"),
        Some(PropertyValue::FlexShrink(3.0))
    );
}

#[test]
fn flex_shrink_rejects_negative() {
    assert_eq!(parse("-2", "flex-shrink"), None);
}

// ── flex-basis (CSS Flexible Box Layout Module Level 1 §7.2.3) ──────

#[test]
fn flex_basis_parse_auto() {
    assert_eq!(
        parse("auto", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::Auto))
    );
}

#[test]
fn flex_basis_parse_content() {
    assert_eq!(
        parse("content", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::Content))
    );
}

#[test]
fn flex_basis_parse_min_max_fit_content_keywords() {
    // CSS Sizing 3 intrinsic keywords (WPT `flex-basis-valid.html`).
    // Bare `fit-content` only — the `fit-content(<length-percentage>)`
    // function form stays out of scope.
    assert_eq!(
        parse("min-content", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::MinContent))
    );
    assert_eq!(
        parse("max-content", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::MaxContent))
    );
    assert_eq!(
        parse("fit-content", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::FitContent))
    );
}

#[test]
fn flex_basis_parse_length_and_percentage() {
    assert_eq!(
        parse("200px", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::Length(
            Length::Px(200.0)
        )))
    );
    assert_eq!(
        parse("50%", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::Length(
            Length::Percent(50.0)
        )))
    );
}

#[test]
fn flex_basis_rejects_negative_length() {
    // `<'width'>` reuse — CSS Sizing 3 §3.1.1 の `[0,∞]` constraint。
    assert_eq!(parse("-10px", "flex-basis"), None);
}

// ── flex shorthand (CSS Flexible Box Layout Module Level 1 §7.1) ────

#[test]
fn flex_shorthand_none_expands_to_0_0_auto() {
    assert_eq!(
        parse("none", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 0.0,
            shrink: 0.0,
            basis: FlexBasisValue::Auto,
        }))
    );
}

#[test]
fn flex_shorthand_auto_is_1_1_auto() {
    // §7.1.1 informative summary: `flex: auto` == `flex: 1 1 auto`。
    assert_eq!(
        parse("auto", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 1.0,
            shrink: 1.0,
            basis: FlexBasisValue::Auto,
        }))
    );
}

#[test]
fn flex_shorthand_bare_number_defaults_shrink_1_basis_0() {
    // §7.1.1 informative summary: `flex: <number [1,∞]>` ==
    // `flex: <number> 1 0` — omitted-component default (grow=1/shrink=1
    // であって longhand 自身の initial ではない、`FlexShorthand` doc の
    // "Omitted-component defaults" 節)。
    assert_eq!(
        parse("2", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 2.0,
            shrink: 1.0,
            basis: FlexBasisValue::Length(Length::Px(0.0)),
        }))
    );
}

#[test]
fn flex_shorthand_unitless_zero_is_a_flex_factor_not_yet_preceded_by_two() {
    // spec §7.1 verbatim: "A unitless zero that is not already preceded
    // by two flex factors must be interpreted as a flex factor."
    assert_eq!(
        parse("0", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 0.0,
            shrink: 1.0,
            basis: FlexBasisValue::Length(Length::Px(0.0)),
        }))
    );
}

#[test]
fn flex_shorthand_unitless_zero_after_two_factors_is_basis() {
    // 同じ spec 文の逆方向 — 2 つの flex factor の**後**の unitless zero は
    // flex-basis として解釈される。
    assert_eq!(
        parse("2 3 0", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 2.0,
            shrink: 3.0,
            basis: FlexBasisValue::Length(Length::Px(0.0)),
        }))
    );
}

#[test]
fn flex_shorthand_basis_only_defaults_grow_1_shrink_1() {
    assert_eq!(
        parse("30px", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 1.0,
            shrink: 1.0,
            basis: FlexBasisValue::Length(Length::Px(30.0)),
        }))
    );
}

#[test]
fn flex_shorthand_basis_before_grow_shrink() {
    // `||` combinator — order-independent between the 2 groups.
    assert_eq!(
        parse("300px 2", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 2.0,
            shrink: 1.0,
            basis: FlexBasisValue::Length(Length::Px(300.0)),
        }))
    );
}

#[test]
fn flex_shorthand_grow_shrink_basis_full_form() {
    assert_eq!(
        parse("2 3 10%", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 2.0,
            shrink: 3.0,
            basis: FlexBasisValue::Length(Length::Percent(10.0)),
        }))
    );
}

#[test]
fn flex_shorthand_content_basis() {
    assert_eq!(
        parse("1 1 content", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 1.0,
            shrink: 1.0,
            basis: FlexBasisValue::Content,
        }))
    );
}

#[test]
fn flex_shorthand_empty_is_none() {
    assert_eq!(parse("", "flex"), None);
}

// ── flex-flow shorthand (CSS Flexible Box Layout Module Level 1 §5.3)

#[test]
fn flex_flow_direction_only() {
    assert_eq!(
        parse("column", "flex-flow"),
        Some(PropertyValue::FlexFlow(FlexFlow {
            direction: FlexDirectionValue::Column,
            wrap: FlexWrapValue::NoWrap,
        }))
    );
}

#[test]
fn flex_flow_wrap_only() {
    assert_eq!(
        parse("wrap", "flex-flow"),
        Some(PropertyValue::FlexFlow(FlexFlow {
            direction: FlexDirectionValue::Row,
            wrap: FlexWrapValue::Wrap,
        }))
    );
}

#[test]
fn flex_flow_both_components_either_order() {
    let both = || {
        PropertyValue::FlexFlow(FlexFlow {
            direction: FlexDirectionValue::RowReverse,
            wrap: FlexWrapValue::WrapReverse,
        })
    };
    assert_eq!(parse("row-reverse wrap-reverse", "flex-flow"), Some(both()));
    assert_eq!(parse("wrap-reverse row-reverse", "flex-flow"), Some(both()));
}

#[test]
fn flex_flow_rejects_empty_and_unknown() {
    assert_eq!(parse("", "flex-flow"), None);
    assert_eq!(parse("diagonal", "flex-flow"), None);
}

#[test]
fn flex_flow_rejects_duplicate_components() {
    // `parse_value` 契約では leftover token を consume せず caller
    // (`DeclParser` の `expect_exhausted`) が declaration ごと drop する
    // (`parse_flex_shorthand` と同じ contract) — ここでは
    // `parse_entire` で declaration-level の exhaustiveness を再現する。
    assert_eq!(parse_entire("row row", "flex-flow"), None);
    assert_eq!(parse_entire("wrap wrap", "flex-flow"), None);
    assert_eq!(parse_entire("row wrap nowrap", "flex-flow"), None);
}

#[test]
fn flex_flow_key_maps_to_flex_flow_property_key() {
    let v = PropertyValue::FlexFlow(FlexFlow {
        direction: FlexDirectionValue::Row,
        wrap: FlexWrapValue::NoWrap,
    });
    assert_eq!(v.key(), PropertyKey::FlexFlow);
}

// ── order (CSS Flexible Box Layout Module Level 1 §4.2) ────────────

#[test]
fn order_parses_integers() {
    assert_eq!(parse("0", "order"), Some(PropertyValue::Order(0)));
    assert_eq!(parse("3", "order"), Some(PropertyValue::Order(3)));
    assert_eq!(parse("-1", "order"), Some(PropertyValue::Order(-1)));
}

#[test]
fn order_rejects_non_integers() {
    assert_eq!(parse("auto", "order"), None);
    assert_eq!(parse("1.5", "order"), None);
    assert_eq!(parse("row", "order"), None);
}

#[test]
fn order_key_maps_to_order_property_key() {
    let v = PropertyValue::Order(2);
    assert_eq!(v.key(), PropertyKey::Order);
}

// ── justify-content / align-content (CSS Box Alignment Module Level 3
//    §5.1) — shared `ContentAlignmentValue` ─────────────────────────

#[test]
fn justify_content_parse_content_distribution_and_position() {
    assert_eq!(
        parse("space-between", "justify-content"),
        Some(PropertyValue::JustifyContent(
            ContentAlignmentValue::SpaceBetween
        ))
    );
    assert_eq!(
        parse("center", "justify-content"),
        Some(PropertyValue::JustifyContent(ContentAlignmentValue::Center))
    );
    assert_eq!(
        parse("flex-end", "justify-content"),
        Some(PropertyValue::JustifyContent(
            ContentAlignmentValue::FlexEnd
        ))
    );
    assert_eq!(
        parse("normal", "justify-content"),
        Some(PropertyValue::JustifyContent(ContentAlignmentValue::Normal))
    );
}

#[test]
fn align_content_parse_same_grammar_as_justify_content() {
    assert_eq!(
        parse("stretch", "align-content"),
        Some(PropertyValue::AlignContent(ContentAlignmentValue::Stretch))
    );
    assert_eq!(
        parse("space-evenly", "align-content"),
        Some(PropertyValue::AlignContent(
            ContentAlignmentValue::SpaceEvenly
        ))
    );
}

#[test]
fn content_alignment_rejects_left_right_and_baseline() {
    // scope carving: `left`/`right` (justify-content-specific extension)
    // と `<baseline-position>` は taffy に対応 variant が無いため未実装
    // (`ContentAlignmentValue` doc 参照)。
    assert_eq!(parse("left", "justify-content"), None);
    assert_eq!(parse("right", "justify-content"), None);
    assert_eq!(parse("baseline", "align-content"), None);
}

#[test]
fn content_alignment_rejects_overflow_position_prefix() {
    // scope carving: `safe`/`unsafe` prefix は未実装。
    assert_eq!(parse("safe center", "justify-content"), None);
}

// ── align-items (CSS Box Alignment Module Level 3 §7.2) ─────────────

#[test]
fn align_items_parse_self_position_and_baseline() {
    assert_eq!(
        parse("stretch", "align-items"),
        Some(PropertyValue::AlignItems(SelfAlignmentValue::Stretch))
    );
    assert_eq!(
        parse("baseline", "align-items"),
        Some(PropertyValue::AlignItems(SelfAlignmentValue::Baseline))
    );
    assert_eq!(
        parse("flex-start", "align-items"),
        Some(PropertyValue::AlignItems(SelfAlignmentValue::FlexStart))
    );
}

#[test]
fn align_items_rejects_auto() {
    // `auto` は align-self 専用 keyword — align-items の grammar には無い。
    assert_eq!(parse("auto", "align-items"), None);
}

#[test]
fn align_items_rejects_self_start_self_end() {
    // scope carving: writing-mode 相対 keyword は未実装
    // (`SelfAlignmentValue` doc 参照)。
    assert_eq!(
        parse("self-start", "align-items"),
        Some(PropertyValue::AlignItems(SelfAlignmentValue::Start))
    );
    assert_eq!(
        parse("self-end", "align-items"),
        Some(PropertyValue::AlignItems(SelfAlignmentValue::End))
    );
}

// ── align-self (CSS Box Alignment Module Level 3 §6.2) ──────────────

#[test]
fn align_self_parse_auto() {
    assert_eq!(
        parse("auto", "align-self"),
        Some(PropertyValue::AlignSelf(AlignSelfValue::Auto))
    );
}

#[test]
fn align_self_parse_explicit_reuses_self_alignment_grammar() {
    assert_eq!(
        parse("center", "align-self"),
        Some(PropertyValue::AlignSelf(AlignSelfValue::Value(
            SelfAlignmentValue::Center
        )))
    );
    assert_eq!(
        parse("baseline", "align-self"),
        Some(PropertyValue::AlignSelf(AlignSelfValue::Value(
            SelfAlignmentValue::Baseline
        )))
    );
}

// ── row-gap / column-gap (CSS Box Alignment Module Level 3 §8.1) ────

#[test]
fn row_gap_parse_normal() {
    assert_eq!(
        parse("normal", "row-gap"),
        Some(PropertyValue::RowGap(LengthOrNormal::Normal))
    );
}

#[test]
fn row_gap_parse_length_and_percentage() {
    assert_eq!(
        parse("10px", "row-gap"),
        Some(PropertyValue::RowGap(LengthOrNormal::Length(Length::Px(
            10.0
        ))))
    );
    assert_eq!(
        parse("5%", "row-gap"),
        Some(PropertyValue::RowGap(LengthOrNormal::Length(
            Length::Percent(5.0)
        )))
    );
}

#[test]
fn row_gap_rejects_negative() {
    assert_eq!(parse("-1px", "row-gap"), None);
}

#[test]
fn column_gap_parse_same_grammar_as_row_gap() {
    assert_eq!(
        parse("2em", "column-gap"),
        Some(PropertyValue::ColumnGap(LengthOrNormal::Length(
            Length::Em(2.0)
        )))
    );
}

// ── gap shorthand (CSS Box Alignment Module Level 3 §8.2) ───────────

#[test]
fn gap_shorthand_single_value_copies_to_both() {
    assert_eq!(
        parse("10px", "gap"),
        Some(PropertyValue::Gap(GapShorthand {
            row: LengthOrNormal::Length(Length::Px(10.0)),
            column: LengthOrNormal::Length(Length::Px(10.0)),
        }))
    );
}

#[test]
fn gap_shorthand_two_values() {
    assert_eq!(
        parse("10px 20px", "gap"),
        Some(PropertyValue::Gap(GapShorthand {
            row: LengthOrNormal::Length(Length::Px(10.0)),
            column: LengthOrNormal::Length(Length::Px(20.0)),
        }))
    );
}

#[test]
fn gap_shorthand_normal() {
    assert_eq!(
        parse("normal", "gap"),
        Some(PropertyValue::Gap(GapShorthand {
            row: LengthOrNormal::Normal,
            column: LengthOrNormal::Normal,
        }))
    );
}

// ── place-content shorthand (CSS Box Alignment Module Level 3 §5.2) ─

#[test]
fn place_content_shorthand_single_value_copies_to_both() {
    assert_eq!(
        parse("center", "place-content"),
        Some(PropertyValue::PlaceContent(PlaceContentShorthand {
            align: ContentAlignmentValue::Center,
            justify: ContentAlignmentValue::Center,
        }))
    );
}

#[test]
fn place_content_shorthand_two_values() {
    assert_eq!(
        parse("center space-between", "place-content"),
        Some(PropertyValue::PlaceContent(PlaceContentShorthand {
            align: ContentAlignmentValue::Center,
            justify: ContentAlignmentValue::SpaceBetween,
        }))
    );
}

// ── key() discriminant integrity (mirrors `height_key_maps_to_height_property_key`) ─

#[test]
fn flex_group_keys_map_correctly() {
    assert_eq!(
        PropertyValue::FlexDirection(FlexDirectionValue::Row).key(),
        PropertyKey::FlexDirection
    );
    assert_eq!(
        PropertyValue::FlexWrap(FlexWrapValue::NoWrap).key(),
        PropertyKey::FlexWrap
    );
    assert_eq!(PropertyValue::FlexGrow(1.0).key(), PropertyKey::FlexGrow);
    assert_eq!(
        PropertyValue::FlexShrink(1.0).key(),
        PropertyKey::FlexShrink
    );
    assert_eq!(
        PropertyValue::FlexBasis(FlexBasisValue::Auto).key(),
        PropertyKey::FlexBasis
    );
    assert_eq!(
        PropertyValue::Flex(FlexShorthand {
            grow: 1.0,
            shrink: 1.0,
            basis: FlexBasisValue::Auto,
        })
        .key(),
        PropertyKey::Flex
    );
    assert_eq!(
        PropertyValue::JustifyContent(ContentAlignmentValue::Normal).key(),
        PropertyKey::JustifyContent
    );
    assert_eq!(
        PropertyValue::AlignContent(ContentAlignmentValue::Normal).key(),
        PropertyKey::AlignContent
    );
    assert_eq!(
        PropertyValue::AlignItems(SelfAlignmentValue::Normal).key(),
        PropertyKey::AlignItems
    );
    assert_eq!(
        PropertyValue::AlignSelf(AlignSelfValue::Auto).key(),
        PropertyKey::AlignSelf
    );
    assert_eq!(
        PropertyValue::RowGap(LengthOrNormal::Normal).key(),
        PropertyKey::RowGap
    );
    assert_eq!(
        PropertyValue::ColumnGap(LengthOrNormal::Normal).key(),
        PropertyKey::ColumnGap
    );
    assert_eq!(
        PropertyValue::Gap(GapShorthand {
            row: LengthOrNormal::Normal,
            column: LengthOrNormal::Normal,
        })
        .key(),
        PropertyKey::Gap
    );
    assert_eq!(
        PropertyValue::PlaceContent(PlaceContentShorthand {
            align: ContentAlignmentValue::Normal,
            justify: ContentAlignmentValue::Normal,
        })
        .key(),
        PropertyKey::PlaceContent
    );
}

// ── quotes property (CSS Content Module Level 3 §2.4.1 / legacy CSS2
// §12.3.1 grammar subset `none | [ <string> <string> ]+`) ──

fn quotes_pairs(pairs: &[(&str, &str)]) -> Arc<Vec<(SmolStr, SmolStr)>> {
    Arc::new(
        pairs
            .iter()
            .map(|(open, close)| (SmolStr::new(open), SmolStr::new(close)))
            .collect(),
    )
}

#[test]
fn quotes_none_returns_empty_vec() {
    // spec: `none` は空リストと同等 (top-level alternative)。
    // empty case は shared Arc slot (`empty_quotes_entries`) を使う。
    assert_eq!(
        parse("none", "quotes"),
        Some(PropertyValue::Quotes(empty_quotes_entries()))
    );
}

#[test]
fn quotes_is_case_insensitive_on_none() {
    // CSS spec: keyword `none` は ASCII case-insensitive。
    assert_eq!(
        parse("NONE", "quotes"),
        Some(PropertyValue::Quotes(empty_quotes_entries()))
    );
}

#[test]
fn quotes_single_pair() {
    assert_eq!(
        parse(r#""«" "»""#, "quotes"),
        Some(PropertyValue::Quotes(quotes_pairs(&[("«", "»")])))
    );
}

#[test]
fn quotes_multiple_pairs_deeper_nesting_levels() {
    // 2 pair 目は 1 pair 目より深い nesting level (`PropertyValue::Quotes`
    // doc の「levels of nesting」節)。
    assert_eq!(
        parse(r#""«" "»" "‹" "›""#, "quotes"),
        Some(PropertyValue::Quotes(quotes_pairs(&[
            ("«", "»"),
            ("‹", "›"),
        ])))
    );
}

#[test]
fn quotes_rejects_empty_declaration() {
    // grammar は `[ <string> <string> ]+` — 0 pair (`none` でもなく
    // 何も書かれていない) は invalid、declaration drop。
    assert_eq!(parse("", "quotes"), None);
}

#[test]
fn quotes_rejects_odd_number_of_strings() {
    // trailing unpaired <string> は `[ <string> <string> ]+` に一致しない
    // (`parse_quotes_property` doc の "malformed pair" 節)。
    assert_eq!(parse(r#""«" "»" "‹""#, "quotes"), None);
}

#[test]
fn quotes_rejects_non_string_token() {
    // <string> でない token (bare ident) は grammar 違反。
    assert_eq!(parse("open close", "quotes"), None);
}

#[test]
fn quotes_key_maps_to_quotes_property_key() {
    assert_eq!(
        PropertyValue::Quotes(empty_quotes_entries()).key(),
        PropertyKey::Quotes
    );
}

// ── text-shadow (CSS Text Decoration Module Level 3 §4) ─────────

/// [`content_items`] と同じ shape の extraction helper —
/// `PropertyValue::TextShadow(Arc<Vec<..>>)` の payload を clone して返す。
fn text_shadow_items(source: &str) -> Vec<TextShadowItem> {
    match parse(source, "text-shadow") {
        Some(PropertyValue::TextShadow(v)) => (*v).clone(),
        // cov:ignore: this panic branch is unreached as long as every
        // caller passes a genuinely valid text-shadow declaration —
        // llvm-cov marks the panic-message literal "uncovered" the
        // same way it does for any other panic-only branch (same
        // false-positive class as the let-else panic branch above).
        other => panic!("expected PropertyValue::TextShadow, got {other:?}"),
    }
}

#[test]
fn text_shadow_parse_none_is_empty_list() {
    assert_eq!(text_shadow_items("none"), Vec::new());
}

#[test]
fn text_shadow_parse_single_offset_only_defaults_blur_and_color() {
    // `<color>` / blur-radius 省略 — `TextShadowItem` doc の「各成分の
    // 初期値埋め」節。
    assert_eq!(
        text_shadow_items("1px 2px"),
        vec![TextShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_parse_color_after_lengths() {
    assert_eq!(
        text_shadow_items("1px 2px 3px red"),
        vec![TextShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(3.0),
            color: TextShadowColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
        }]
    );
}

/// `<color>? && <length>{2,3}` の `&&` combinator — 順序は自由
/// ([`parse_text_shadow_item`] doc の「`&&` grammar semantics」節)。
/// color-before は color-after (直上 test) と同じ結果になる。
#[test]
fn text_shadow_parse_color_before_lengths_matches_color_after() {
    assert_eq!(
        text_shadow_items("red 1px 2px 3px"),
        text_shadow_items("1px 2px 3px red"),
    );
}

#[test]
fn text_shadow_parse_explicit_currentcolor_keyword() {
    assert_eq!(
        text_shadow_items("currentcolor 1px 1px"),
        vec![TextShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_parse_negative_offsets_allowed() {
    // offset-x / offset-y に non-negative 制約は無い (`TextShadowItem`
    // doc の「Non-negative blur-radius」節 — blur のみ制約対象)。
    assert_eq!(
        text_shadow_items("-1px -2px"),
        vec![TextShadowItem {
            offset_x: Length::Px(-1.0),
            offset_y: Length::Px(-2.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_parse_rejects_percentage() {
    // `<length>` のみ、percentage 不可 (CSS Text Decoration Module Level
    // 3 §4 "Percentages: N/A", `TextShadowItem` doc 参照)。
    // `parse_length_value(input, false)` (`allow_percentage=false`) が
    // parse-time で拒否する — sibling precedent
    // `page_size_percentage_rejected` と同じ shape。
    assert_eq!(parse("50% 50%", "text-shadow"), None);
}

#[test]
fn text_shadow_parse_rejects_negative_blur_radius() {
    // blur-radius (3rd length) は non-negative — CSS Backgrounds 3 §6.1
    // "Drop Shadows: the box-shadow property"
    // "Negative values are invalid" (box-shadow / text-shadow 共通の
    // `<shadow>` syntax)。負の 3rd length は blur slot にマッチせず
    // unconsumed のまま残り、`parse_comma_separated` の
    // `parse_until_before` → `parse_entirely` が leftover を検知して
    // declaration ごと drop する (`parse_text_shadow` doc 参照)。
    assert_eq!(parse("1px 1px -3px", "text-shadow"), None);
}

#[test]
fn text_shadow_zero_mantissa_huge_exponent_offset_resolves_to_zero_but_preserves_infinity() {
    // `0e999` is a zero-mantissa, huge-exponent literal that
    // cssparser's tokenizer collapses to `NaN` internally (module doc's
    // "Numeric-token NaN stabilization" section), but
    // `next_numeric_stable` (which `parse_length_value` — reached via
    // `parse_shadow_length_reject_nan` → `parse_length_allow_negative`
    // — acquires its token through) corrects that before this
    // property's `!is_nan()` guard ever runs. So `0e999px` resolves to
    // the spec-correct `Length::Px(0.0)` offset, and the whole
    // declaration parses successfully — it is no longer dropped.
    assert_eq!(
        text_shadow_items("0e999px 1px"),
        vec![TextShadowItem {
            offset_x: Length::Px(0.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        }]
    );
    assert_eq!(
        text_shadow_items("1px 0e999px"),
        vec![TextShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(0.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        }]
    );

    // `+Inf`/`-Inf` are a *different* hazard class — ordinary `<number>`
    // magnitude overflow, a legitimate (if extreme) `<length>` per CSS
    // Values 4 §5 — and must NOT be rejected here either. Both signs
    // are checked (not just `+Inf`) because an earlier iteration of the
    // sibling `opacity` guard used `is_finite()` and wrongly dropped
    // the negative-overflow case too (`opacity_zero_mantissa_huge_exponent_resolves_to_zero_but_preserves_infinity`
    // doc参照).
    assert_eq!(
        text_shadow_items("1e40px 1px"),
        vec![TextShadowItem {
            offset_x: Length::Px(f32::INFINITY),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        }]
    );
    assert_eq!(
        text_shadow_items("-1e40px 1px"),
        vec![TextShadowItem {
            offset_x: Length::Px(f32::NEG_INFINITY),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_blur_radius_zero_mantissa_huge_exponent_resolves_to_zero() {
    // Same recovery as the offset case above, for blur-radius (3rd
    // slot) — `0e999px` resolves to `Length::Px(0.0)`, which
    // `parse_non_negative_length`'s `>= 0.0` check accepts normally
    // (it is no longer `NaN`, so there is nothing for that check to
    // incidentally reject).
    assert_eq!(
        text_shadow_items("1px 1px 0e999px"),
        vec![TextShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        }]
    );
}

#[test]
fn text_shadow_parse_multiple_comma_separated() {
    assert_eq!(
        text_shadow_items("1px 1px red, 2px 2px 4px blue"),
        vec![
            TextShadowItem {
                offset_x: Length::Px(1.0),
                offset_y: Length::Px(1.0),
                blur_radius: Length::Px(0.0),
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            },
            TextShadowItem {
                offset_x: Length::Px(2.0),
                offset_y: Length::Px(2.0),
                blur_radius: Length::Px(4.0),
                color: TextShadowColor::Resolved(CssColor {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 255,
                }),
            },
        ]
    );
}

#[test]
fn text_shadow_parse_rejects_empty_value() {
    // length run は必須 — `<color>` 単体 (`text-shadow: red`) は
    // grammar 上 invalid。
    assert_eq!(parse("red", "text-shadow"), None);
}

#[test]
fn text_shadow_key_maps_to_text_shadow_property_key() {
    assert_eq!(
        PropertyValue::TextShadow(Arc::new(Vec::new())).key(),
        PropertyKey::TextShadow
    );
}

// ── grid-template-columns / grid-template-rows (CSS Grid Layout Module
//    Level 1 §7.2) ──────────────────────────────────────────────────

#[test]
fn grid_template_columns_none() {
    assert_eq!(
        parse("none", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::None))
    );
}

#[test]
fn grid_template_columns_single_track() {
    assert_eq!(
        parse("100px", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Size(GridTrackSize::Breadth(
                    GridTrackBreadth::Length(Length::Px(100.0))
                ))],
            }))
        ))
    );
}

#[test]
fn grid_template_columns_multiple_tracks_and_keywords() {
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) = parse(
        "100px auto 1fr min-content max-content",
        "grid-template-columns",
    ) else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(list.components.len(), 5);
    assert_eq!(
        list.components[0],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::Length(
            Length::Px(100.0)
        )))
    );
    assert_eq!(
        list.components[1],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::Auto))
    );
    assert_eq!(
        list.components[2],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::Flex(1.0)))
    );
    assert_eq!(
        list.components[3],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::MinContent))
    );
    assert_eq!(
        list.components[4],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::MaxContent))
    );
    // 6 line-name slots (5 components + 1 trailing), all empty.
    assert_eq!(list.line_names.len(), 6);
    assert!(list.line_names.iter().all(Vec::is_empty));
}

#[test]
fn grid_template_columns_minmax() {
    assert_eq!(
        parse("minmax(0, 1fr)", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Size(GridTrackSize::MinMax(
                    GridInflexibleBreadth::Length(Length::Px(0.0)),
                    GridTrackBreadth::Flex(1.0),
                ))],
            }))
        ))
    );
}

#[test]
fn grid_template_columns_minmax_rejects_flex_in_min_position() {
    // `<inflexible-breadth>` (minmax's first argument) excludes `<flex>`
    // — CSS Grid Layout Module Level 1 §7.2.1.
    assert_eq!(parse("minmax(1fr, 100px)", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_minmax_accepts_all_inflexible_breadth_keywords_as_min() {
    // `<inflexible-breadth>` (`minmax()`'s first argument) accepts
    // `auto` / `min-content` / `max-content` in addition to
    // `<length-percentage>` (already covered by `grid_template_columns_minmax`
    // above) — CSS Grid Layout Module Level 1 §7.2.1.
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) = parse(
        "minmax(auto, 100px) minmax(min-content, 1fr) minmax(max-content, 1fr)",
        "grid-template-columns",
    ) else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(
        list.components,
        vec![
            GridTrackListComponent::Size(GridTrackSize::MinMax(
                GridInflexibleBreadth::Auto,
                GridTrackBreadth::Length(Length::Px(100.0)),
            )),
            GridTrackListComponent::Size(GridTrackSize::MinMax(
                GridInflexibleBreadth::MinContent,
                GridTrackBreadth::Flex(1.0),
            )),
            GridTrackListComponent::Size(GridTrackSize::MinMax(
                GridInflexibleBreadth::MaxContent,
                GridTrackBreadth::Flex(1.0),
            )),
        ]
    );
}

#[test]
fn grid_template_columns_rejects_negative_fr() {
    // `<flex [0,∞]>` (CSS Grid Layout Module Level 1 §7.2.4) — a
    // negative `fr` value fails `parse_grid_flex_res`'s non-negative
    // check, and (unlike a valid `fr`) doesn't fall back to a
    // `<length-percentage>` either, since `fr` isn't a length unit.
    assert_eq!(parse("-1fr", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_zero_mantissa_huge_exponent_flex_resolves_to_zero() {
    // `0e999fr` is a zero-mantissa, huge-exponent literal that cssparser's
    // tokenizer collapses to `NaN` internally (module doc's
    // "Numeric-token NaN stabilization" section is canonical for the
    // mechanism), but `next_numeric_stable` (which `parse_grid_flex_res`
    // acquires its token through) corrects that before the `is_finite()`
    // guard ever runs. So `0e999fr` resolves to the spec-correct
    // `GridTrackBreadth::Flex(0.0)` directly, per CSS Syntax 3 §4.3.13
    // (`true value is 0`) and CSS Grid 1 §7.2.4 (`<flex [0,∞]>` allows 0).
    assert_eq!(
        parse("0e999fr", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Size(GridTrackSize::Breadth(
                    GridTrackBreadth::Flex(0.0)
                ))],
            }))
        ))
    );
}

#[test]
fn grid_template_columns_fit_content() {
    assert_eq!(
        parse("fit-content(40%)", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Size(GridTrackSize::FitContent(
                    Length::Percent(40.0)
                ))],
            }))
        ))
    );
}

#[test]
fn grid_template_columns_fit_content_rejects_negative_length() {
    // `fit-content( <length-percentage [0,∞]> )` (CSS Grid Layout
    // Module Level 1 §7.2.1) — `parse_grid_fit_content_res` parses the
    // length itself first (which does accept a negative sign), then
    // rejects it in a separate non-negative check.
    assert_eq!(parse("fit-content(-10px)", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_named_lines() {
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) = parse(
        "[full-start] 1fr [content-start] 2fr [content-end] 1fr [full-end]",
        "grid-template-columns",
    ) else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(list.components.len(), 3);
    assert_eq!(
        list.line_names,
        vec![
            vec![SmolStr::new("full-start")],
            vec![SmolStr::new("content-start")],
            vec![SmolStr::new("content-end")],
            vec![SmolStr::new("full-end")],
        ]
    );
}

#[test]
fn grid_template_columns_named_lines_reject_span_and_auto() {
    // CSS Grid Layout Module Level 1 §7.2.2 verbatim: "A line name
    // cannot be span or auto, i.e. the `<custom-ident>` in the
    // `<line-names>` production excludes the keywords span and auto."
    // `parse_line_names` must reject these the same way
    // `parse_grid_custom_ident` already does for `<grid-line>`
    // productions — a malformed `<line-names>` invalidates the whole
    // declaration (silent drop), same as any other grammar violation.
    assert_eq!(parse("[auto] 1fr", "grid-template-columns"), None);
    assert_eq!(parse("[span] 1fr", "grid-template-columns"), None);
    // Case-insensitive, same as `is_reserved_grid_line_name`.
    assert_eq!(parse("[AUTO] 1fr", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_repeat_integer() {
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) =
        parse("repeat(3, 1fr)", "grid-template-columns")
    else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(
        list.components,
        vec![GridTrackListComponent::Repeat(GridTrackRepeat {
            count: GridRepeatCount::Count(3),
            line_names: vec![vec![], vec![]],
            tracks: vec![GridTrackSize::Breadth(GridTrackBreadth::Flex(1.0))],
        })]
    );
}

#[test]
fn grid_template_columns_repeat_auto_fill() {
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) = parse(
        "repeat(auto-fill, minmax(100px, 1fr))",
        "grid-template-columns",
    ) else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(
        list.components,
        vec![GridTrackListComponent::Repeat(GridTrackRepeat {
            count: GridRepeatCount::AutoFill,
            line_names: vec![vec![], vec![]],
            tracks: vec![GridTrackSize::MinMax(
                GridInflexibleBreadth::Length(Length::Px(100.0)),
                GridTrackBreadth::Flex(1.0),
            )],
        })]
    );
}

#[test]
fn grid_template_columns_repeat_auto_fit() {
    assert!(matches!(
        parse("repeat(auto-fit, 100px)", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(_)
        ))
    ));
}

#[test]
fn grid_template_columns_repeat_auto_fill_rejects_flex() {
    // CSS Grid Layout Module Level 1 §7.2.3.1 verbatim: "Automatic
    // repetitions (auto-fill or auto-fit) cannot be combined with fully
    // intrinsic or flexible sizes" — `<auto-repeat>` requires
    // `<fixed-size>`, which excludes bare `fr`.
    assert_eq!(
        parse("repeat(auto-fill, 1fr)", "grid-template-columns"),
        None
    );
}

#[test]
fn grid_template_columns_repeat_auto_fill_rejects_bare_min_content() {
    // `<fixed-size>` also excludes bare `min-content`/`max-content`/
    // `auto` (only `<fixed-breadth>`, or `minmax()` with a
    // `<fixed-breadth>` side, qualify).
    assert_eq!(
        parse("repeat(auto-fill, min-content)", "grid-template-columns"),
        None
    );
}

#[test]
fn grid_template_columns_rejects_second_auto_repeat() {
    // §7.2.3.1 verbatim: "It can only appear once in the track list".
    assert_eq!(
        parse(
            "repeat(auto-fill, 100px) repeat(auto-fit, 100px)",
            "grid-template-columns"
        ),
        None
    );
}

#[test]
fn grid_template_columns_allows_auto_repeat_plus_fixed_repeat() {
    // §7.2.3.1 verbatim: "...but the same track list can also contain
    // `<fixed-repeat>`s." — a numeric `repeat()` alongside the one
    // `auto-fill`/`auto-fit` is valid provided its own tracks are also
    // `<fixed-size>`.
    assert!(matches!(
        parse(
            "repeat(2, 50px) repeat(auto-fill, 100px)",
            "grid-template-columns"
        ),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(_)
        ))
    ));
}

#[test]
fn grid_template_columns_rejects_zero_repeat_count() {
    assert_eq!(parse("repeat(0, 1fr)", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_rejects_repeat_with_no_tracks() {
    // `repeat( <count>, [ <line-names>? <track-size> ]+ <line-names>? )`
    // — the `+` requires at least 1 track; a bare trailing comma with
    // nothing after it parses the count and comma but finds no
    // `<track-size>`.
    assert_eq!(parse("repeat(3,)", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_auto_repeat_plus_bare_track_validates_together() {
    // `grid_track_list_obeys_auto_repeat_constraint`'s fixed-size walk
    // must check bare `Size` components too, not just the tracks
    // nested inside `repeat()` —
    // `grid_template_columns_allows_auto_repeat_plus_fixed_repeat` above
    // only combines 2 `repeat()`s, never a bare `Size` component
    // alongside an auto-repeat.
    assert!(matches!(
        parse("100px repeat(auto-fill, 50px)", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(_)
        ))
    ));
    // `fit-content()` has no `<fixed-size>` alternative
    // (`grid_track_size_is_fixed` doc) — a bare `fit-content()`
    // component alongside an auto-repeat makes the whole declaration
    // invalid.
    assert_eq!(
        parse(
            "fit-content(50%) repeat(auto-fill, 50px)",
            "grid-template-columns"
        ),
        None
    );
}

#[test]
fn grid_template_columns_rejects_unknown_ident() {
    assert_eq!(parse("bogus", "grid-template-columns"), None);
}

#[test]
fn grid_template_rows_shares_the_same_grammar() {
    assert_eq!(
        parse("50%", "grid-template-rows"),
        Some(PropertyValue::GridTemplateRows(GridTemplateTracks::List(
            Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Size(GridTrackSize::Breadth(
                    GridTrackBreadth::Length(Length::Percent(50.0))
                ))],
            })
        )))
    );
}

// ── grid-template-areas (CSS Grid Layout Module Level 1 §7.3) ───────

#[test]
fn grid_template_areas_none() {
    assert_eq!(
        parse("none", "grid-template-areas"),
        Some(PropertyValue::GridTemplateAreas(
            GridTemplateAreasValue::None
        ))
    );
}

#[test]
fn grid_template_areas_simple_single_cell() {
    assert_eq!(
        parse(r#""a""#, "grid-template-areas"),
        Some(PropertyValue::GridTemplateAreas(
            GridTemplateAreasValue::Areas(Arc::new(GridTemplateAreas {
                row_strings: vec![SmolStr::new("a")],
                areas: vec![GridTemplateAreaEntry {
                    name: SmolStr::new("a"),
                    row_start: 1,
                    row_end: 2,
                    column_start: 1,
                    column_end: 2,
                }],
                row_count: 1,
                column_count: 1,
            }))
        ))
    );
}

#[test]
fn grid_template_areas_multi_row_multi_col_with_null_cells() {
    let Some(PropertyValue::GridTemplateAreas(GridTemplateAreasValue::Areas(areas))) = parse(
        r#""header header" "nav main" "footer ...""#,
        "grid-template-areas",
    ) else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected parsed areas");
    };
    assert_eq!(areas.row_count, 3);
    assert_eq!(areas.column_count, 2);
    let mut names: Vec<&str> = areas.areas.iter().map(|a| a.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["footer", "header", "main", "nav"]);
    let header = areas.areas.iter().find(|a| a.name == "header").unwrap();
    assert_eq!(
        (
            header.row_start,
            header.row_end,
            header.column_start,
            header.column_end
        ),
        (1, 2, 1, 3)
    );
    let footer = areas.areas.iter().find(|a| a.name == "footer").unwrap();
    // `...` is a run of `.` null-cell tokens spanning the second
    // column — `footer` only occupies the first column of row 3.
    assert_eq!(
        (
            footer.row_start,
            footer.row_end,
            footer.column_start,
            footer.column_end
        ),
        (3, 4, 1, 2)
    );
}

#[test]
fn grid_template_areas_spanning_area_forms_rectangle() {
    let Some(PropertyValue::GridTemplateAreas(GridTemplateAreasValue::Areas(areas))) =
        parse(r#""a a" "a a""#, "grid-template-areas")
    else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected parsed areas");
    };
    assert_eq!(areas.areas.len(), 1);
    let a = &areas.areas[0];
    assert_eq!(
        (a.row_start, a.row_end, a.column_start, a.column_end),
        (1, 3, 1, 3)
    );
}

#[test]
fn grid_template_areas_rejects_uneven_columns() {
    // spec verbatim: "All strings must define the same number of cell
    // tokens ... or else the declaration is invalid."
    assert_eq!(parse(r#""a b" "c""#, "grid-template-areas"), None);
}

#[test]
fn grid_template_areas_rejects_non_rectangular_area() {
    // spec verbatim: "If a named grid area spans multiple grid cells,
    // but those cells do not form a single filled-in rectangle, the
    // declaration is invalid."
    assert_eq!(parse(r#""a b" "b a""#, "grid-template-areas"), None);
}

#[test]
fn grid_template_areas_rejects_trash_token() {
    // spec verbatim: "A trash token is a syntax error, and makes the
    // declaration invalid." `#` is neither an ident code point nor `.`.
    assert_eq!(parse(r#""a #""#, "grid-template-areas"), None);
}

#[test]
fn grid_template_areas_treats_non_ascii_space_as_an_ident_char_not_whitespace() {
    // CSS Syntax 3 whitespace is exactly {tab, newline, space}
    // (<https://www.w3.org/TR/css-syntax-3/#whitespace>) — U+3000
    // IDEOGRAPHIC SPACE is not whitespace under that definition, so it
    // must fall into the "ident code point" bucket (CSS Syntax 3's
    // ident-code-point production includes any non-ASCII code point),
    // becoming *part of* the named-cell token rather than a silently
    // skipped separator between two cells.
    let Some(PropertyValue::GridTemplateAreas(GridTemplateAreasValue::Areas(areas))) =
        parse("\"a\u{3000}b\"", "grid-template-areas")
    else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a GridTemplateAreas value");
    };
    // A single named cell "a\u{3000}b", not two cells "a" and "b".
    assert_eq!(areas.row_count, 1);
    assert_eq!(areas.column_count, 1);
}

#[test]
fn grid_template_areas_rejects_vertical_tab_as_trash_not_whitespace() {
    // U+000B LINE TABULATION is Unicode `White_Space` but not CSS
    // whitespace (only tab/newline/space qualify) and not an ASCII
    // ident code point either, so it must be a trash token — same
    // shape as `grid_template_areas_rejects_trash_token`.
    assert_eq!(parse("\"a\u{b}b\"", "grid-template-areas"), None);
}

#[test]
fn grid_template_areas_rejects_a_value_with_no_strings_at_all() {
    // `none | <string>+` — neither the `none` keyword nor any `<string>`
    // is present, so `parse_grid_template_areas`'s `rows.is_empty()`
    // guard rejects it before ever reaching `build_grid_template_areas`
    // (distinct from `grid_template_areas_rejects_trash_token` above,
    // where a string IS present but its content is invalid).
    assert_eq!(parse("5px", "grid-template-areas"), None);
}

#[test]
fn grid_template_areas_rejects_empty_string_list() {
    assert_eq!(parse(r#""""#, "grid-template-areas"), None);
}

// ── grid-auto-columns / grid-auto-rows (CSS Grid Layout Module Level 1
//    §7.6) ──────────────────────────────────────────────────────────

#[test]
fn grid_auto_columns_single_track() {
    assert_eq!(
        parse("200px", "grid-auto-columns"),
        Some(PropertyValue::GridAutoColumns(Arc::new(vec![
            GridTrackSize::Breadth(GridTrackBreadth::Length(Length::Px(200.0)))
        ])))
    );
}

#[test]
fn grid_auto_columns_multiple_tracks() {
    assert_eq!(
        parse("100px 1fr", "grid-auto-columns"),
        Some(PropertyValue::GridAutoColumns(Arc::new(vec![
            GridTrackSize::Breadth(GridTrackBreadth::Length(Length::Px(100.0))),
            GridTrackSize::Breadth(GridTrackBreadth::Flex(1.0)),
        ])))
    );
}

#[test]
fn grid_auto_rows_shares_the_same_grammar() {
    assert_eq!(
        parse("min-content", "grid-auto-rows"),
        Some(PropertyValue::GridAutoRows(Arc::new(vec![
            GridTrackSize::Breadth(GridTrackBreadth::MinContent)
        ])))
    );
}

#[test]
fn grid_auto_columns_rejects_repeat() {
    // `<track-size>+` — `repeat()` is not part of this grammar (unlike
    // `grid-template-columns`'s `<track-list>`).
    assert_eq!(parse("repeat(2, 10px)", "grid-auto-columns"), None);
}

// ── grid-auto-flow (CSS Grid Layout Module Level 1 §7.7) ────────────

#[test]
fn grid_auto_flow_row() {
    assert_eq!(
        parse("row", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::Row))
    );
}

#[test]
fn grid_auto_flow_column() {
    assert_eq!(
        parse("column", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::Column))
    );
}

#[test]
fn grid_auto_flow_dense_alone_defaults_to_row() {
    assert_eq!(
        parse("dense", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::RowDense))
    );
}

#[test]
fn grid_auto_flow_row_dense_either_order() {
    assert_eq!(
        parse("row dense", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::RowDense))
    );
    assert_eq!(
        parse("dense row", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::RowDense))
    );
}

#[test]
fn grid_auto_flow_column_dense() {
    assert_eq!(
        parse("column dense", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::ColumnDense))
    );
}

#[test]
fn grid_auto_flow_rejects_empty() {
    assert_eq!(parse("", "grid-auto-flow"), None);
}

// ── grid-row-start / grid-row-end / grid-column-start /
//    grid-column-end (CSS Grid Layout Module Level 1 §8.3) ──────────

#[test]
fn grid_line_auto() {
    assert_eq!(
        parse("auto", "grid-row-start"),
        Some(PropertyValue::GridRowStart(GridLineValue::Auto))
    );
}

#[test]
fn grid_line_positive_and_negative_integer() {
    assert_eq!(
        parse("3", "grid-row-start"),
        Some(PropertyValue::GridRowStart(GridLineValue::Line(3)))
    );
    assert_eq!(
        parse("-1", "grid-row-end"),
        Some(PropertyValue::GridRowEnd(GridLineValue::Line(-1)))
    );
}

#[test]
fn grid_line_rejects_zero() {
    // spec verbatim: "Negative integers or zero are invalid."
    assert_eq!(parse("0", "grid-column-start"), None);
}

#[test]
fn grid_line_bare_custom_ident() {
    assert_eq!(
        parse("content-start", "grid-column-start"),
        Some(PropertyValue::GridColumnStart(GridLineValue::Named(
            SmolStr::new("content-start")
        )))
    );
}

#[test]
fn grid_line_integer_and_name_either_order() {
    assert_eq!(
        parse("2 content-start", "grid-column-start"),
        Some(PropertyValue::GridColumnStart(GridLineValue::NamedLine(
            SmolStr::new("content-start"),
            2
        )))
    );
    assert_eq!(
        parse("content-start 2", "grid-column-start"),
        Some(PropertyValue::GridColumnStart(GridLineValue::NamedLine(
            SmolStr::new("content-start"),
            2
        )))
    );
}

#[test]
fn grid_line_span_integer() {
    assert_eq!(
        parse("span 3", "grid-column-end"),
        Some(PropertyValue::GridColumnEnd(GridLineValue::Span(3)))
    );
}

#[test]
fn grid_line_span_rejects_zero_or_negative() {
    assert_eq!(parse("span 0", "grid-column-end"), None);
    assert_eq!(parse("span -1", "grid-column-end"), None);
}

#[test]
fn grid_line_span_named() {
    assert_eq!(
        parse("span content-end", "grid-column-end"),
        Some(PropertyValue::GridColumnEnd(GridLineValue::SpanNamed(
            SmolStr::new("content-end"),
            1
        )))
    );
}

#[test]
fn grid_line_span_named_and_integer_either_order() {
    assert_eq!(
        parse("span 2 content-end", "grid-column-end"),
        Some(PropertyValue::GridColumnEnd(GridLineValue::SpanNamed(
            SmolStr::new("content-end"),
            2
        )))
    );
    assert_eq!(
        parse("span content-end 2", "grid-column-end"),
        Some(PropertyValue::GridColumnEnd(GridLineValue::SpanNamed(
            SmolStr::new("content-end"),
            2
        )))
    );
}

#[test]
fn grid_line_span_alone_is_invalid() {
    // grammar: `span && [ <integer> || <custom-ident> ]` — the bracketed
    // group is mandatory.
    assert_eq!(parse("span", "grid-row-start"), None);
}

#[test]
fn grid_line_rejects_span_and_auto_as_custom_ident() {
    // spec verbatim (§8.3): "the `<custom-ident>` additionally excludes
    // the keywords `span` and `auto`".
    assert_eq!(parse("span", "grid-column-start"), None);
}

#[test]
fn grid_line_integer_then_reserved_ident_leaves_leftover_for_caller_exhausted_check() {
    // Sibling of `grid_line_rejects_span_and_auto_as_custom_ident` above,
    // but exercised through the plain `<integer> <custom-ident>`
    // alternative instead of the `span` prefix: after the leading `2`
    // is consumed, `auto` fails the trailing `<custom-ident>`
    // alternative (same additional exclusion) and is left unconsumed.
    // Same "helper returns `Some`, rejection is the caller's job"
    // pattern as
    // `text_decoration_line_two_underlines_leaves_leftover_for_caller_exhausted_check`.
    let mut input = ParserInput::new("2 auto");
    let mut parser = Parser::new(&mut input);
    assert_eq!(parse_grid_line(&mut parser), Some(GridLineValue::Line(2)));
    assert!(!parser.is_exhausted());
}

#[test]
fn grid_line_rejects_a_value_that_is_neither_integer_nor_ident() {
    // `[ [ <integer> ] && <custom-ident>? ] | <custom-ident> | auto`
    // (`GridLineValue` doc) — a `<string>` token satisfies none of the
    // alternatives `parse_grid_line` tries (`auto`, `span`, `<integer>`,
    // `<custom-ident>`), so it's rejected outright rather than leaving
    // a leftover token.
    assert_eq!(parse(r#""foo""#, "grid-row-start"), None);
}

// ── grid-row / grid-column shorthand (CSS Grid Layout Module Level 1
//    §8.4) ───────────────────────────────────────────────────────────

#[test]
fn grid_row_shorthand_two_values() {
    assert_eq!(
        parse("2 / 5", "grid-row"),
        Some(PropertyValue::GridRow(GridLineShorthand {
            start: GridLineValue::Line(2),
            end: GridLineValue::Line(5),
        }))
    );
}

#[test]
fn grid_row_shorthand_omitted_second_copies_custom_ident() {
    // spec verbatim: "if the first value is a `<custom-ident>`, the
    // grid-row-end / grid-column-end longhand is also set to that
    // `<custom-ident>`".
    assert_eq!(
        parse("content", "grid-row"),
        Some(PropertyValue::GridRow(GridLineShorthand {
            start: GridLineValue::Named(SmolStr::new("content")),
            end: GridLineValue::Named(SmolStr::new("content")),
        }))
    );
}

#[test]
fn grid_row_shorthand_omitted_second_defaults_to_auto_for_non_ident() {
    // spec verbatim: "...otherwise, it is set to auto."
    assert_eq!(
        parse("3", "grid-row"),
        Some(PropertyValue::GridRow(GridLineShorthand {
            start: GridLineValue::Line(3),
            end: GridLineValue::Auto,
        }))
    );
    assert_eq!(
        parse("span 2", "grid-row"),
        Some(PropertyValue::GridRow(GridLineShorthand {
            start: GridLineValue::Span(2),
            end: GridLineValue::Auto,
        }))
    );
}

#[test]
fn grid_column_shorthand_two_values() {
    assert_eq!(
        parse("main-start / main-end", "grid-column"),
        Some(PropertyValue::GridColumn(GridLineShorthand {
            start: GridLineValue::Named(SmolStr::new("main-start")),
            end: GridLineValue::Named(SmolStr::new("main-end")),
        }))
    );
}

// ── justify-items / justify-self (CSS Box Alignment Module Level 3
//    §7.1 / §6.1) ────────────────────────────────────────────────────

#[test]
fn justify_items_parse_keywords() {
    assert_eq!(
        parse("center", "justify-items"),
        Some(PropertyValue::JustifyItems(SelfAlignmentValue::Center))
    );
    assert_eq!(
        parse("stretch", "justify-items"),
        Some(PropertyValue::JustifyItems(SelfAlignmentValue::Stretch))
    );
}

#[test]
fn justify_self_parse_auto_and_keywords() {
    assert_eq!(
        parse("auto", "justify-self"),
        Some(PropertyValue::JustifySelf(AlignSelfValue::Auto))
    );
    assert_eq!(
        parse("end", "justify-self"),
        Some(PropertyValue::JustifySelf(AlignSelfValue::Value(
            SelfAlignmentValue::End
        )))
    );
}

// ── place-items / place-self (CSS Box Alignment Module Level 3 §7.3 /
//    §6.3) ────────────────────────────────────────────────────────────

#[test]
fn place_items_shorthand_single_value_copies_to_both() {
    assert_eq!(
        parse("center", "place-items"),
        Some(PropertyValue::PlaceItems(PlaceItemsShorthand {
            align: SelfAlignmentValue::Center,
            justify: SelfAlignmentValue::Center,
        }))
    );
}

#[test]
fn place_items_shorthand_two_values() {
    assert_eq!(
        parse("start end", "place-items"),
        Some(PropertyValue::PlaceItems(PlaceItemsShorthand {
            align: SelfAlignmentValue::Start,
            justify: SelfAlignmentValue::End,
        }))
    );
}

#[test]
fn place_self_shorthand_single_value_copies_to_both() {
    assert_eq!(
        parse("auto", "place-self"),
        Some(PropertyValue::PlaceSelf(PlaceSelfShorthand {
            align: AlignSelfValue::Auto,
            justify: AlignSelfValue::Auto,
        }))
    );
}

#[test]
fn place_self_shorthand_two_values() {
    assert_eq!(
        parse("center stretch", "place-self"),
        Some(PropertyValue::PlaceSelf(PlaceSelfShorthand {
            align: AlignSelfValue::Value(SelfAlignmentValue::Center),
            justify: AlignSelfValue::Value(SelfAlignmentValue::Stretch),
        }))
    );
}

// ── key() discriminant integrity (mirrors `flex_group_keys_map_correctly`) ─

#[test]
fn grid_group_keys_map_correctly() {
    assert_eq!(
        PropertyValue::GridTemplateColumns(GridTemplateTracks::None).key(),
        PropertyKey::GridTemplateColumns
    );
    assert_eq!(
        PropertyValue::GridTemplateRows(GridTemplateTracks::None).key(),
        PropertyKey::GridTemplateRows
    );
    assert_eq!(
        PropertyValue::GridTemplateAreas(GridTemplateAreasValue::None).key(),
        PropertyKey::GridTemplateAreas
    );
    assert_eq!(
        PropertyValue::GridAutoColumns(Arc::new(vec![GridTrackSize::Breadth(
            GridTrackBreadth::Auto
        )]))
        .key(),
        PropertyKey::GridAutoColumns
    );
    assert_eq!(
        PropertyValue::GridAutoRows(Arc::new(vec![GridTrackSize::Breadth(
            GridTrackBreadth::Auto
        )]))
        .key(),
        PropertyKey::GridAutoRows
    );
    assert_eq!(
        PropertyValue::GridAutoFlow(GridAutoFlowValue::Row).key(),
        PropertyKey::GridAutoFlow
    );
    assert_eq!(
        PropertyValue::GridRowStart(GridLineValue::Auto).key(),
        PropertyKey::GridRowStart
    );
    assert_eq!(
        PropertyValue::GridRowEnd(GridLineValue::Auto).key(),
        PropertyKey::GridRowEnd
    );
    assert_eq!(
        PropertyValue::GridColumnStart(GridLineValue::Auto).key(),
        PropertyKey::GridColumnStart
    );
    assert_eq!(
        PropertyValue::GridColumnEnd(GridLineValue::Auto).key(),
        PropertyKey::GridColumnEnd
    );
    assert_eq!(
        PropertyValue::GridRow(GridLineShorthand {
            start: GridLineValue::Auto,
            end: GridLineValue::Auto,
        })
        .key(),
        PropertyKey::GridRow
    );
    assert_eq!(
        PropertyValue::GridColumn(GridLineShorthand {
            start: GridLineValue::Auto,
            end: GridLineValue::Auto,
        })
        .key(),
        PropertyKey::GridColumn
    );
    assert_eq!(
        PropertyValue::JustifyItems(SelfAlignmentValue::Normal).key(),
        PropertyKey::JustifyItems
    );
    assert_eq!(
        PropertyValue::JustifySelf(AlignSelfValue::Auto).key(),
        PropertyKey::JustifySelf
    );
    assert_eq!(
        PropertyValue::PlaceItems(PlaceItemsShorthand {
            align: SelfAlignmentValue::Normal,
            justify: SelfAlignmentValue::Normal,
        })
        .key(),
        PropertyKey::PlaceItems
    );
    assert_eq!(
        PropertyValue::PlaceSelf(PlaceSelfShorthand {
            align: AlignSelfValue::Auto,
            justify: AlignSelfValue::Auto,
        })
        .key(),
        PropertyKey::PlaceSelf
    );
}

#[test]
fn deferred_function_scanner_skips_literals_and_handles_bounds() {
    assert!(contains_deferred_function_in_source("foo(VAR(--x))"));
    assert!(!contains_deferred_function_in_source(
        r#""var(--x)" /* calc(1px) */"#
    ));
    assert!(!contains_deferred_function_in_source("#var(--x)"));
    assert!(!contains_deferred_function_in_source("@calc(1px)"));
    assert!(!contains_deferred_function_in_source("\"unterminated"));
    assert!(!contains_deferred_function_in_source("/* unterminated"));
    assert_eq!(skip_deferred_string(r#""a\"b""#, 0), Some(6));
    assert_eq!(skip_deferred_string("\"unterminated", 0), None);
    assert_eq!(skip_deferred_comment("/* comment */", 0), Some(13));
    assert_eq!(skip_deferred_comment("/* unterminated", 0), None);
    assert!(contains_deferred_function_in_source("[var(--x)]"));
    assert!(contains_function_in_source("[var(--x)]", "var"));
}

#[test]
fn deferred_value_capture_rejects_oversized_input() {
    let source = format!("calc(1px){}", "x".repeat(64 * 1024));
    assert!(parse(&source, "width").is_none());
    assert!(contains_deferred_function_in_source(&source));
    let mut parser_input = ParserInput::new(&source);
    let mut parser = Parser::new(&mut parser_input);
    assert!(contains_deferred_function(&mut parser));

    let oversized = "x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1);
    let mut oversized_input = ParserInput::new(&oversized);
    let mut oversized_parser = Parser::new(&mut oversized_input);
    assert!(contains_deferred_function(&mut oversized_parser));
}

#[test]
fn deferred_value_capture_rejects_oversized_trailing_comment() {
    let source = format!("var(--x)/*{}*/", "x".repeat(MAX_SUBSTITUTED_VALUE_BYTES));
    assert!(parse(&source, "width").is_none());
}

#[test]
fn deferred_value_capture_rejects_oversized_comment_before_important() {
    let source = format!(
        "var(--x)/*{}*/!important",
        "x".repeat(MAX_SUBSTITUTED_VALUE_BYTES)
    );
    assert!(parse(&source, "width").is_none());
}

#[test]
fn deferred_value_capture_rejects_bad_url_before_important() {
    assert!(parse(r#"var(--x) url(foo"bar)!important"#, "width").is_none());
}

#[test]
fn deferred_value_capture_rejects_excessive_component_nesting() {
    let depth = 129;
    let source = format!("{}var(--x){}", "[".repeat(depth), "]".repeat(depth));
    assert!(parse(&source, "width").is_none());
}

#[test]
fn deferred_value_capture_does_not_nest_unquoted_url_contents() {
    let depth = 129;
    let source = format!(
        "var(--image) url(data:image/svg+xml,{}{}x{}{})",
        "[".repeat(depth),
        "{".repeat(depth),
        "}".repeat(depth),
        "]".repeat(depth),
    );
    assert!(parse(&source, "width").is_some());
}

#[test]
fn math_without_var_is_validated_during_declaration_parsing() {
    assert!(parse("calc(foo)", "width").is_none());
    assert!(parse("min(10px, 20px)", "width").is_some());
}

#[test]
fn math_dummy_selection_is_type_aware() {
    // `math_source_has_dimension_or_percentage`: dimensions and
    // percentages count even through nesting and across comma-separated
    // arguments (no early exit — `parse_nested_block` runs its closure
    // via `parse_entirely`).
    assert!(math_source_has_dimension_or_percentage("calc(10px)"));
    assert!(math_source_has_dimension_or_percentage("min(20px, 10px)"));
    assert!(math_source_has_dimension_or_percentage("calc((1px))"));
    assert!(math_source_has_dimension_or_percentage("calc(10% + 1)"));
    assert!(!math_source_has_dimension_or_percentage("calc(0)"));
    assert!(!math_source_has_dimension_or_percentage("calc(3 - 3)"));

    // Percentage-bearing math is invalid in box-shadow length slots, even
    // when the percentage is nested in a math function or block token.
    assert!(math_source_has_percentage("calc(10%)"));
    assert!(math_source_has_percentage("min(10px, 20%)"));
    assert!(!math_source_has_percentage("foo(10%)"));
    assert!(math_source_has_percentage("calc((10%))"));
    assert!(math_source_has_percentage("calc([10%])"));
    assert!(math_source_has_percentage("calc({10%})"));
    assert!(math_source_has_percentage("calc(10% + 1px)"));
    assert_eq!(parse_entire("calc(10% + 1px) 2px", "box-shadow"), None);
    assert!(matches!(
        parse_entire("calc(1px + 2px) 2px", "box-shadow"),
        Some(PropertyValue::BoxShadow(_))
    ));

    // `deferred_dummy_is_valid_for_property`: pure-number math validates
    // only `<number>` positions (WPT `flex: 1 2 calc(0)` invalid), while
    // dimension-carrying math keeps the `1px` behavior.
    assert!(deferred_dummy_is_valid_for_property(
        "calc(-1)",
        "flex-grow"
    ));
    assert!(!deferred_dummy_is_valid_for_property(
        "calc(0)",
        "flex-basis"
    ));
    assert!(deferred_dummy_is_valid_for_property(
        "calc(2em + 3ex)",
        "width"
    ));
}

// ── orphans / widows (CSS Fragmentation Module Level 3 §3.3) ──
//
// Value grammar: `<integer>`, restricted to positive integers by spec
// prose ("Only positive integers are allowed as values of orphans and
// widows. Negative values and zero are invalid and must cause the
// declaration to be ignored."). Initial: 2 / Inherited: yes / Computed
// value: specified integer.

#[test]
fn orphans_widows_parse_valid_positive_integers() {
    assert_eq!(parse("1", "orphans"), Some(PropertyValue::Orphans(1)));
    assert_eq!(parse("2", "orphans"), Some(PropertyValue::Orphans(2)));
    assert_eq!(parse("100", "orphans"), Some(PropertyValue::Orphans(100)));
    assert_eq!(parse("1", "widows"), Some(PropertyValue::Widows(1)));
    assert_eq!(parse("3", "widows"), Some(PropertyValue::Widows(3)));
}

// ── border-radius / box-shadow / outline (CSS Backgrounds 3 / CSS UI 3 §4)
// ──

#[test]
fn border_radius_expands_one_to_four_lengths_in_clockwise_order() {
    assert_eq!(
        parse("1px", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0),
            top_right: Length::Px(1.0),
            bottom_right: Length::Px(1.0),
            bottom_left: Length::Px(1.0),
        }))
    );
    assert_eq!(
        parse("1px 2em", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0),
            top_right: Length::Em(2.0),
            bottom_right: Length::Px(1.0),
            bottom_left: Length::Em(2.0),
        }))
    );
    assert_eq!(
        parse("1px 2px 3px", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0),
            top_right: Length::Px(2.0),
            bottom_right: Length::Px(3.0),
            bottom_left: Length::Px(2.0),
        }))
    );
    assert_eq!(
        parse("1px 2px 3px 4px", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0),
            top_right: Length::Px(2.0),
            bottom_right: Length::Px(3.0),
            bottom_left: Length::Px(4.0),
        }))
    );
}

#[test]
fn border_radius_accepts_percentages_and_rejects_negative_lengths() {
    assert_eq!(
        parse_entire("50% 25%", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Percent(50.0),
            top_right: Length::Percent(25.0),
            bottom_right: Length::Percent(50.0),
            bottom_left: Length::Percent(25.0),
        }))
    );
    assert_eq!(
        parse_entire("25%", "border-top-left-radius"),
        Some(PropertyValue::BorderRadiusTopLeft(Length::Percent(25.0)))
    );
    assert_eq!(
        parse_entire("inherit", "border-radius"),
        Some(PropertyValue::BorderRadiusInherit)
    );
    assert_eq!(parse_entire("1px -2px", "border-radius"), None);
}

#[test]
fn box_shadow_parses_none_multiple_entries_and_optional_components() {
    assert_eq!(
        parse("none", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![])))
    );

    let value = parse("red 1px -2px 3px 4px, 2px 3px", "box-shadow");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    let Some(PropertyValue::BoxShadow(shadows)) = value else {
        panic!("box-shadow should parse to a shadow list");
    };
    assert_eq!(
        shadows.as_ref(),
        &[
            BoxShadowItem {
                offset_x: Length::Px(1.0),
                offset_y: Length::Px(-2.0),
                blur_radius: Length::Px(3.0),
                spread_radius: Length::Px(4.0),
                color: TextShadowColor::Resolved(red()),
                inset: false,
            },
            BoxShadowItem {
                offset_x: Length::Px(2.0),
                offset_y: Length::Px(3.0),
                blur_radius: Length::Px(0.0),
                spread_radius: Length::Px(0.0),
                color: TextShadowColor::CurrentColor,
                inset: false,
            },
        ]
    );
}

#[test]
fn box_shadow_parses_inset_any_order_and_rejects_invalid_components() {
    assert!(matches!(
        parse("inset 1px 2px", "box-shadow"),
        Some(PropertyValue::BoxShadow(shadows)) if shadows[0].inset
    ));
    assert!(matches!(
        parse("red 1px 2px 3px -4px inset", "box-shadow"),
        Some(PropertyValue::BoxShadow(shadows))
            if shadows[0].inset && shadows[0].color == TextShadowColor::Resolved(red())
    ));

    assert_eq!(parse_entire("1px 2px 3px 4px 5px", "box-shadow"), None);
    assert_eq!(parse("1px 2px -3px", "box-shadow"), None);
    assert_eq!(
        parse("1px 2px 3px -4px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(3.0),
            spread_radius: Length::Px(-4.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(parse("1px 2px 10%", "box-shadow"), None);
    assert_eq!(parse_entire("inset 1px 2px inset", "box-shadow"), None);
}

#[test]
fn box_shadow_zero_mantissa_huge_exponent_offset_or_spread_resolves_to_zero_but_preserves_infinity()
{
    // `0e999` is a zero-mantissa, huge-exponent literal that
    // cssparser's tokenizer collapses to `NaN` internally (module doc's
    // "Numeric-token NaN stabilization" section), but
    // `next_numeric_stable` corrects it before `parse_length_value`
    // (and so `parse_shadow_length_reject_nan`'s `!is_nan()` guard)
    // ever sees the token — for offset-x, offset-y, and spread-radius
    // alike (all three carry no sign restriction, unlike blur-radius's
    // `[0,∞]` incidental filter). Each resolves to the spec-correct
    // `Length::Px(0.0)`, and the whole declaration parses successfully.
    assert_eq!(
        parse("0e999px 1px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(0.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(
        parse("1px 0e999px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(0.0),
            blur_radius: Length::Px(0.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(
        parse("1px 1px 1px 0e999px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(1.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );

    // `+Inf`/`-Inf` are a *different* hazard class — ordinary `<number>`
    // magnitude overflow, a legitimate (if extreme) `<length>` per CSS
    // Values 4 §5 — and must NOT be rejected here. Both signs are
    // checked (not just `+Inf`) because an earlier iteration of the
    // sibling `opacity` guard used `is_finite()` and wrongly dropped
    // the negative-overflow case too (`opacity_zero_mantissa_huge_exponent_resolves_to_zero_but_preserves_infinity`
    // doc参照).
    assert_eq!(
        parse("1e40px 1px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(f32::INFINITY),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(
        parse("-1e40px 1px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(f32::NEG_INFINITY),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(
        parse("1px 1px 1px 1e40px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(1.0),
            spread_radius: Length::Px(f32::INFINITY),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(
        parse("1px 1px 1px -1e40px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(1.0),
            spread_radius: Length::Px(f32::NEG_INFINITY),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
}

#[test]
fn box_shadow_blur_radius_zero_mantissa_huge_exponent_resolves_to_zero() {
    // Same recovery as
    // `box_shadow_zero_mantissa_huge_exponent_offset_or_spread_resolves_to_zero`
    // above, for blur-radius (3rd slot) — `0e999px` resolves to
    // `Length::Px(0.0)` before `parse_box_shadow_lengths`'s
    // `length_payload(value) >= 0.0` check ever runs, so it is accepted
    // normally rather than incidentally rejected.
    assert_eq!(
        parse("1px 1px 0e999px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
}

#[test]
fn outline_parses_any_order_and_fills_initial_components() {
    assert_eq!(
        parse("solid 2px red", "outline"),
        Some(PropertyValue::Outline(Outline {
            width: Length::Px(2.0),
            style: OutlineStyle::Solid,
            color: OutlineColor::Resolved(red()),
        }))
    );
    assert_eq!(
        parse("red", "outline"),
        Some(PropertyValue::Outline(Outline {
            width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
            style: OutlineStyle::None,
            color: OutlineColor::Resolved(red()),
        }))
    );
    assert_eq!(
        parse("auto", "outline"),
        Some(PropertyValue::Outline(Outline {
            width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
            style: OutlineStyle::Auto,
            color: OutlineColor::Invert,
        }))
    );
    assert_eq!(
        parse("auto 2px red", "outline"),
        Some(PropertyValue::Outline(Outline {
            width: Length::Px(2.0),
            style: OutlineStyle::Auto,
            color: OutlineColor::Resolved(red()),
        }))
    );
    assert_eq!(
        parse("auto", "outline-style"),
        Some(PropertyValue::OutlineStyle(OutlineStyle::Auto))
    );
    assert_eq!(parse("hidden", "outline-style"), None);
    assert_eq!(parse("auto", "border-top-style"), None);
    assert_eq!(parse("hidden", "outline"), None);
    assert_eq!(
        PropertyValue::Outline(Outline {
            width: Length::Px(1.0),
            style: OutlineStyle::None,
            color: OutlineColor::Invert,
        })
        .key(),
        PropertyKey::Outline
    );
}

#[test]
fn outline_color_accepts_invert_currentcolor_and_resolved_colors() {
    assert_eq!(
        parse("invert", "outline-color"),
        Some(PropertyValue::OutlineColor(OutlineColor::Invert))
    );
    assert_eq!(
        parse("InVeRt", "outline-color"),
        Some(PropertyValue::OutlineColor(OutlineColor::Invert))
    );
    assert_eq!(
        parse("currentcolor", "outline-color"),
        Some(PropertyValue::OutlineColor(OutlineColor::CurrentColor))
    );
    assert_eq!(
        parse("red", "outline-color"),
        Some(PropertyValue::OutlineColor(OutlineColor::Resolved(red())))
    );
    assert_eq!(
        parse("solid invert", "outline"),
        Some(PropertyValue::Outline(Outline {
            width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
            style: OutlineStyle::Solid,
            color: OutlineColor::Invert,
        }))
    );
    // `invert` is outline-only; border-color parsing remains unchanged.
    assert_eq!(parse("invert", "border-top-color"), None);
}

#[test]
fn outline_offset_parses_length_and_rejects_non_length() {
    // CSS UI 3 §4.5 <https://www.w3.org/TR/css-ui-3/#outline-offset> — `<length>`,
    // initial `0`, non-inherited. Negative values are valid (inset).
    assert_eq!(
        parse("5px", "outline-offset"),
        Some(PropertyValue::OutlineOffset(Length::Px(5.0)))
    );
    assert_eq!(
        parse("0", "outline-offset"),
        Some(PropertyValue::OutlineOffset(Length::Px(0.0)))
    );
    assert_eq!(
        parse("-3px", "outline-offset"),
        Some(PropertyValue::OutlineOffset(Length::Px(-3.0)))
    );
    assert_eq!(
        parse("2em", "outline-offset"),
        Some(PropertyValue::OutlineOffset(Length::Em(2.0)))
    );
    assert_eq!(
        PropertyValue::OutlineOffset(Length::Px(4.0)).key(),
        PropertyKey::OutlineOffset
    );
    // `<percentage>` is not part of the grammar — reject.
    assert_eq!(parse("5%", "outline-offset"), None);
    // `auto` / `none` are not valid for this property.
    assert_eq!(parse("auto", "outline-offset"), None);
    assert_eq!(parse("none", "outline-offset"), None);
}

#[test]
fn orphans_widows_parse_leading_plus_sign() {
    // CSS Values and Units 3 §4.2 `<integer>`: a leading `+` sign is
    // part of the grammar (`[+-]? digit+`), not an error.
    assert_eq!(parse("+3", "orphans"), Some(PropertyValue::Orphans(3)));
    assert_eq!(parse("+3", "widows"), Some(PropertyValue::Widows(3)));
}

#[test]
fn orphans_widows_reject_zero() {
    // Spec verbatim: "Negative values and zero are invalid and must
    // cause the declaration to be ignored."
    assert_eq!(parse("0", "orphans"), None);
    assert_eq!(parse("0", "widows"), None);
}

#[test]
fn orphans_widows_reject_negative_integers() {
    assert_eq!(parse("-1", "orphans"), None);
    assert_eq!(parse("-100", "orphans"), None);
    assert_eq!(parse("-1", "widows"), None);
}

#[test]
fn orphans_widows_reject_non_integer_values() {
    // Fractional numbers, lengths, idents, and strings are all outside
    // the `<integer>` grammar.
    assert_eq!(parse("1.5", "orphans"), None);
    assert_eq!(parse("2px", "orphans"), None);
    assert_eq!(parse("auto", "orphans"), None);
    assert_eq!(parse(r#""2""#, "orphans"), None);
    assert_eq!(parse("1.5", "widows"), None);
    assert_eq!(parse("2px", "widows"), None);
}

#[test]
fn orphans_widows_reject_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future
    // work), silent drop (`PropertyValue` doc's "CSS-wide keyword"
    // section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "orphans"), None);
        assert_eq!(parse(kw, "widows"), None);
    }
}

#[test]
fn orphans_widows_key_maps_to_distinct_property_keys() {
    assert_eq!(PropertyValue::Orphans(2).key(), PropertyKey::Orphans);
    assert_eq!(PropertyValue::Widows(2).key(), PropertyKey::Widows);
}

fn assert_parsed_hue_close(source: &str, expected_degrees: f32) {
    let parsed = parse_parsed_color_entire(source).expect(source);
    let actual_degrees = parsed.coordinates[2].to_degrees();
    assert!(
        (actual_degrees - expected_degrees).abs() < 0.0001,
        "{source}: {actual_degrees} != {expected_degrees}" // cov:ignore: assertion failure message literal only executes when the assertion fails
    );
}

fn assert_coordinates_close(actual: [f32; 3], expected: [f32; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
    }
}

#[test]
fn color_parse_lab_family_preserves_out_of_gamut_endpoint_coordinates() {
    let cases = [
        (
            "lab(50% 100 100)",
            ParsedColorSpace::Lab,
            [50.0, 100.0, 100.0],
        ),
        (
            "lch(50% 150 30deg)",
            ParsedColorSpace::Lch,
            [50.0, 150.0, 30.0_f32.to_radians()],
        ),
        (
            "oklab(0.5 0.4 0.4)",
            ParsedColorSpace::Oklab,
            [0.5, 0.4, 0.4],
        ),
        (
            "oklch(0.5 0.4 30deg)",
            ParsedColorSpace::Oklch,
            [0.5, 0.4, 30.0_f32.to_radians()],
        ),
    ];
    for (source, expected_space, expected_coordinates) in cases {
        let parsed = parse_parsed_color_entire(source).expect(source);
        assert_eq!(parsed.space, expected_space, "{source}");
        assert_eq!(parsed.coordinates, expected_coordinates, "{source}");
        assert!(
            parsed
                .to_srgb()
                .iter()
                .any(|component| *component < 0.0 || *component > 1.0),
        );
    }
}

#[test]
fn color_mix_ignores_huge_zero_weight_lab_endpoint() {
    let source = "color-mix(in srgb, lab(50% 3e38 3e38) 0%, red 100%)";
    assert_eq!(
        parse(source, "color"),
        Some(PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }))
    );
}

#[test]
fn color_mix_ignores_huge_zero_weight_lab_endpoint_in_polar_space() {
    let source = "color-mix(in lch, lab(50% 3e38 3e38) 0%, lch(50% 50 20deg) 100%)";
    assert_eq!(parse(source, "color"), parse("lch(50% 50 20deg)", "color"));
}

#[test]
fn color_mix_zero_weight_extreme_polar_endpoint_does_not_poison_hue() {
    let source = "color-mix(in lch, oklab(0.5 3e38 3e38) 0%, lch(50% 50 20deg) 100%)";
    assert_eq!(parse(source, "color"), parse("lch(50% 50 20deg)", "color"));
}

#[test]
fn color_mix_large_rectangular_endpoint_has_finite_polar_coordinates() {
    let cases = [
        (
            "color-mix(in lch, lab(50% 1e38 1e38) 1%, lch(50% 50 20deg) 99%)",
            ParsedColorSpace::Lch,
        ),
        (
            "color-mix(in oklch, oklab(50% 1e38 1e38) 1%, oklch(50% 0.2 20deg) 99%)",
            ParsedColorSpace::Oklch,
        ),
    ];
    for (source, expected_space) in cases {
        let parsed = parse_parsed_color_entire(source).expect(source);
        assert_eq!(parsed.space, expected_space);
        assert!(
            parsed
                .coordinates
                .iter()
                .all(|component| component.is_finite())
        );
        assert!(!parsed.coordinates[1].is_nan());
    }
}

#[test]
fn color_mix_lab_family_keeps_unclipped_intermediate_coordinates() {
    let cases = [
        (
            "color-mix(in lab, lab(50% 100 100) 75%, lab(50% 0 0) 25%)",
            ParsedColorSpace::Lab,
            [50.0, 75.0, 75.0],
            CssColor {
                r: 234,
                g: 8,
                b: 0,
                a: 255,
            },
        ),
        (
            "color-mix(in lch, lch(50% 150 30deg) 75%, lch(50% 0 0) 25%)",
            ParsedColorSpace::Lch,
            [50.0, 112.5, 22.5_f32.to_radians()],
            CssColor {
                r: 255,
                g: 0,
                b: 58,
                a: 255,
            },
        ),
        (
            "color-mix(in oklab, oklab(0.5 0.4 0.4) 75%, oklab(0.5 0 0) 25%)",
            ParsedColorSpace::Oklab,
            [0.5, 0.3, 0.3],
            CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
        (
            "color-mix(in oklch, oklch(0.5 0.4 30deg) 75%, oklch(0.5 0 0) 25%)",
            ParsedColorSpace::Oklch,
            [0.5, 0.3, 22.5_f32.to_radians()],
            CssColor {
                r: 221,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
    ];
    for (source, expected_space, expected_coordinates, expected_color) in cases {
        let parsed = parse_parsed_color_entire(source).expect(source);
        assert_eq!(parsed.space, expected_space, "{source}");
        assert_coordinates_close(parsed.coordinates, expected_coordinates);
        assert_eq!(parsed.to_css_color(), expected_color, "{source}");
    }
}

#[test]
fn color_mix_preserves_out_of_gamut_lab_endpoint_in_each_interpolation_space() {
    let cases = [
        (
            "srgb",
            ParsedColorSpace::Srgb,
            [1.0343289, -0.30642307, -0.15550733],
        ),
        (
            "srgb-linear",
            ParsedColorSpace::SrgbLinear,
            [1.0798806, -0.07645964, -0.020894665],
        ),
        ("lab", ParsedColorSpace::Lab, [50.0, 100.0, 100.0]),
        (
            "lch",
            ParsedColorSpace::Lch,
            [50.0, 141.42136, 45.0_f32.to_radians()],
        ),
        (
            "oklab",
            ParsedColorSpace::Oklab,
            [0.5973762, 0.28092682, 0.13886046],
        ),
        (
            "oklch",
            ParsedColorSpace::Oklch,
            [0.5973762, 0.31337216, 0.45907247],
        ),
    ];
    for (space, expected_space, expected_coordinates) in cases {
        let source = format!("color-mix(in {space}, lab(50% 100 100) 100%, black 0%)");
        let parsed = parse_parsed_color_entire(&source).expect(&source);
        assert_eq!(parsed.space, expected_space, "{source}");
        assert_coordinates_close(parsed.coordinates, expected_coordinates);
        assert!(
            parsed
                .to_srgb()
                .iter()
                .any(|component| *component < 0.0 || *component > 1.0),
        );
    }
}

#[test]
fn srgb_transfer_functions_restore_negative_sign() {
    assert!((srgb_encode(-0.5) + srgb_encode(0.5)).abs() < 0.000001);
    assert!((srgb_decode(-0.5) + srgb_decode(0.5)).abs() < 0.000001);
}

#[test]
fn color_mix_preserves_negative_out_of_gamut_lab_interpolation() {
    let source = "color-mix(in srgb, lab(50% -100 -100) 50%, lab(50% 100 100) 50%)";
    let parsed = parse_parsed_color_entire(source).expect(source);
    assert_eq!(parsed.space, ParsedColorSpace::Srgb);
    assert_coordinates_close(parsed.coordinates, [0.10649943, 0.1554926, 0.4975962]);
    assert_eq!(
        parsed.to_css_color(),
        CssColor {
            r: 27,
            g: 40,
            b: 127,
            a: 255,
        }
    );
}

#[test]
fn color_mix_retains_generated_lightness_above_boundary() {
    for space in ["lab", "lch", "oklab", "oklch"] {
        let source = format!("color-mix(in {space}, color(srgb 2 0 0), color(srgb 2 0 0))");
        let parsed = parse_parsed_color_entire(&source).expect(&source);
        let lightness_limit = if matches!(space, "lab" | "lch") {
            100.0
        } else {
            1.0
        };
        assert!(parsed.coordinates[0] > lightness_limit);
        assert!(parsed.lightness_boundary.is_none());
        assert_eq!(
            parsed.to_css_color(),
            CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }
        );
    }
}

#[test]
fn color_mix_out_of_gamut_serialization_is_deterministic() {
    let source = "color-mix(in oklch, oklch(0.5 0.4 30deg) 75%, oklch(0.5 0 0) 25%)";
    let first = parse_color_entire(source);
    let second = parse_color_entire(source);
    assert_eq!(first, second);
    assert_eq!(
        first,
        Some(CssColor {
            r: 221,
            g: 0,
            b: 0,
            a: 255,
        })
    );
}

#[test]
fn color_mix_converts_each_lab_family_without_clipping() {
    let endpoints = [
        "lab(50% 100 100)",
        "lch(50% 150 30deg)",
        "oklab(0.5 0.4 0.4)",
        "oklch(0.5 0.4 30deg)",
    ];
    let spaces = [
        MixColorSpace::Srgb,
        MixColorSpace::SrgbLinear,
        MixColorSpace::Lab,
        MixColorSpace::Lch,
        MixColorSpace::Oklab,
        MixColorSpace::Oklch,
    ];
    for endpoint in endpoints {
        let parsed = parse_parsed_color_entire(endpoint).expect(endpoint);
        for space in spaces {
            let coordinates = css_color_to_coordinates(parsed, space);
            assert!(
                [coordinates.first, coordinates.second, coordinates.third]
                    .iter()
                    .all(|component| component.is_finite()),
            );
        }
    }

    let achromatic_lch =
        ParsedColor::from_coordinates(ParsedColorSpace::Lch, [50.0, 0.0, 1.0], 1.0);
    assert_eq!(
        css_color_to_coordinates(achromatic_lch, MixColorSpace::Lch).second,
        0.0
    );
    let achromatic_oklch =
        ParsedColor::from_coordinates(ParsedColorSpace::Oklch, [0.5, 0.0, 1.0], 1.0);
    assert_eq!(
        css_color_to_coordinates(achromatic_oklch, MixColorSpace::Oklch).second,
        0.0
    );

    let linear = ParsedColor::from_coordinates(ParsedColorSpace::SrgbLinear, [1.2, -0.1, 0.5], 1.0);
    assert_coordinates_close(linear.to_srgb_linear(), [1.2, -0.1, 0.5]);
    assert!(
        linear
            .to_lab()
            .iter()
            .all(|component| component.is_finite())
    );
    assert!(
        linear
            .to_oklab()
            .iter()
            .all(|component| component.is_finite())
    );
}

#[test]
fn color_mix_nested_out_of_gamut_intermediate_is_not_clipped() {
    let cases = [
        (
            "color-mix(in lab, color-mix(in lab, lab(50% 100 100) 75%, lab(50% 0 0) 25%) 50%, lab(50% 0 0) 50%)",
            CssColor {
                r: 185,
                g: 90,
                b: 57,
                a: 255,
            },
        ),
        (
            "color-mix(in lch, color-mix(in lch, lch(50% 150 30deg) 75%, lch(50% 0 0) 25%) 50%, lch(50% 0 0) 50%)",
            CssColor {
                r: 203,
                g: 70,
                b: 104,
                a: 255,
            },
        ),
        (
            "color-mix(in oklab, color-mix(in oklab, oklab(0.5 0.4 0.4) 75%, oklab(0.5 0 0) 25%) 50%, oklab(0.5 0 0) 50%)",
            CssColor {
                r: 187,
                g: 21,
                b: 0,
                a: 255,
            },
        ),
        (
            "color-mix(in oklch, color-mix(in oklch, oklch(0.5 0.4 30deg) 75%, oklch(0.5 0 0) 25%) 50%, oklch(0.5 0 0) 50%)",
            CssColor {
                r: 166,
                g: 52,
                b: 77,
                a: 255,
            },
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(parse_color_entire(source), Some(expected), "{source}");
    }
}

#[test]
fn color_mix_in_gamut_lab_coordinates_stay_native() {
    let cases = [
        (
            "color-mix(in lab, lab(50% 20 30) 50%, lab(50% 0 0) 50%)",
            ParsedColorSpace::Lab,
            [50.0, 10.0, 15.0],
        ),
        (
            "color-mix(in oklab, oklab(0.5 0.05 0.05) 50%, oklab(0.5 0 0) 50%)",
            ParsedColorSpace::Oklab,
            [0.5, 0.025, 0.025],
        ),
    ];
    for (source, expected_space, expected_coordinates) in cases {
        let parsed = parse_parsed_color_entire(source).expect(source);
        assert_eq!(parsed.space, expected_space, "{source}");
        assert_coordinates_close(parsed.coordinates, expected_coordinates);
    }
}

#[test]
fn color_mix_in_gamut_lab_family_results_stay_compatible() {
    let cases = [
        (
            "color-mix(in lab, lab(50% 20 30), lab(60% -20 -10))",
            CssColor {
                r: 137,
                g: 131,
                b: 114,
                a: 255,
            },
        ),
        (
            "color-mix(in lch, lch(50% 30 30deg), lch(60% 20 200deg))",
            CssColor {
                r: 124,
                g: 136,
                b: 91,
                a: 255,
            },
        ),
        (
            "color-mix(in oklab, oklab(0.5 0.05 0.05), oklab(0.6 -0.03 -0.02))",
            CssColor {
                r: 122,
                g: 111,
                b: 104,
                a: 255,
            },
        ),
        (
            "color-mix(in oklch, oklch(0.5 0.08 30deg), oklch(0.6 0.06 200deg))",
            CssColor {
                r: 112,
                g: 119,
                b: 70,
                a: 255,
            },
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(parse_color_entire(source), Some(expected), "{source}");
    }
}

// ── parse_url_value helper ────────────────────
//
// CSS Values 4 §4.4 <url> value type
// (<https://www.w3.org/TR/css-values-4/#urls>) の共通 helper を property
// dispatcher (`parse_value`) を経由せず直接叩く — `background-image` の
// 呼び出し経路とは独立に helper 自体の grammar 境界 (unquoted/quoted
// form、`<url-modifier>` reject 等) を check する
// (`parse_length_value` helper 単体 test と同じ fixture pattern)。

fn parse_url(source: &str) -> Option<String> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_url_value(&mut parser)
}

#[test]
fn url_value_accepts_unquoted_form() {
    // unquoted `<url-token>` form — see `parse_url_value` doc.
    assert_eq!(parse_url("url(foo.png)"), Some("foo.png".to_string()));
}

#[test]
fn url_value_accepts_quoted_form() {
    // quoted `url( <string> )` function-token form — see
    // `parse_url_value` doc.
    assert_eq!(parse_url("url(\"foo.png\")"), Some("foo.png".to_string()));
}

#[test]
fn url_value_accepts_quoted_form_with_single_quotes() {
    // CSS Syntax 3 `<string-token>` は `"` `'` どちらの quote 文字も
    // 受理する — `url()` 内の `<string>` も同様。
    assert_eq!(parse_url("url('foo.png')"), Some("foo.png".to_string()));
}

#[test]
fn url_value_rejects_css_wide_keywords() {
    // CSS-wide keyword は `<url>` grammar のどの alternative にも
    // 一致しない — see `parse_url_value` doc.
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse_url(keyword), None, "{keyword}");
    }
}

#[test]
fn url_value_rejects_bare_string_without_url_wrapper() {
    // bare `<string>` (no `url()` wrapper) — @import-only legacy
    // allowance, not part of the general `<url>` value type. See
    // `parse_url_value` doc.
    assert_eq!(parse_url("\"foo.png\""), None);
}

#[test]
fn url_value_rejects_non_url_ident() {
    assert_eq!(parse_url("foo"), None);
}

#[test]
fn url_value_rejects_empty_input() {
    assert_eq!(parse_url(""), None);
}

#[test]
fn url_value_rejects_number() {
    assert_eq!(parse_url("42"), None);
}

#[test]
fn url_value_rejects_url_with_modifier() {
    // `<url-modifier>` (`crossorigin()` 等) 付き `url()` — unsupported,
    // see `parse_url_value` doc for the block-exhaustion mechanism.
    assert_eq!(parse_url("url(\"foo.png\" crossorigin)"), None);
}

// ── background-repeat (CSS Backgrounds 3 §2.4) ──

#[test]
fn background_repeat_parse_single_keyword_applies_to_both_axes() {
    for (source, keyword) in [
        ("repeat", BackgroundRepeatKeyword::Repeat),
        ("space", BackgroundRepeatKeyword::Space),
        ("round", BackgroundRepeatKeyword::Round),
        ("no-repeat", BackgroundRepeatKeyword::NoRepeat),
    ] {
        // cov:ignore: the failure-message branch of this `assert_eq!`
        // only executes when the assertion fails; it passes here, so
        // llvm-cov reports the macro's condition-false region as an
        // uncovered added line even though the assertion itself runs
        // and does its job.
        assert_eq!(
            parse(source, "background-repeat"),
            Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
                x: keyword,
                y: keyword,
            })),
            "{source}"
        );
    }
}

#[test]
fn background_repeat_parse_repeat_x_and_repeat_y() {
    assert_eq!(
        parse("repeat-x", "background-repeat"),
        Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        }))
    );
    assert_eq!(
        parse("repeat-y", "background-repeat"),
        Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::Repeat,
        }))
    );
}

#[test]
fn background_repeat_parse_two_keyword_form() {
    assert_eq!(
        parse("space round", "background-repeat"),
        Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Space,
            y: BackgroundRepeatKeyword::Round,
        }))
    );
}

#[test]
fn background_repeat_is_case_insensitive() {
    assert_eq!(
        parse("REPEAT-X", "background-repeat"),
        Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        }))
    );
    assert_eq!(
        parse("No-Repeat", "background-repeat"),
        Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        }))
    );
}

#[test]
fn background_repeat_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-repeat"), None, "{keyword}");
    }
}

#[test]
fn background_repeat_rejects_unknown_keyword() {
    assert_eq!(parse("stretch", "background-repeat"), None);
    assert_eq!(parse("16px", "background-repeat"), None);
}

#[test]
fn background_repeat_key_maps_to_background_repeat_property_key() {
    let v = PropertyValue::BackgroundRepeat(BackgroundRepeat {
        x: BackgroundRepeatKeyword::Repeat,
        y: BackgroundRepeatKeyword::Repeat,
    });
    assert_eq!(v.key(), PropertyKey::BackgroundRepeat);
}

// ── background-attachment (CSS Backgrounds 3 §2.5) ──

#[test]
fn background_attachment_parse_all_three_keywords() {
    assert_eq!(
        parse("scroll", "background-attachment"),
        Some(PropertyValue::BackgroundAttachment(
            BackgroundAttachment::Scroll
        ))
    );
    assert_eq!(
        parse("fixed", "background-attachment"),
        Some(PropertyValue::BackgroundAttachment(
            BackgroundAttachment::Fixed
        ))
    );
    assert_eq!(
        parse("local", "background-attachment"),
        Some(PropertyValue::BackgroundAttachment(
            BackgroundAttachment::Local
        ))
    );
}

#[test]
fn background_attachment_is_case_insensitive() {
    assert_eq!(
        parse("FIXED", "background-attachment"),
        Some(PropertyValue::BackgroundAttachment(
            BackgroundAttachment::Fixed
        ))
    );
}

#[test]
fn background_attachment_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-attachment"), None, "{keyword}");
    }
}

#[test]
fn background_attachment_rejects_unknown_keyword() {
    assert_eq!(parse("static", "background-attachment"), None);
}

#[test]
fn background_attachment_key_maps_to_background_attachment_property_key() {
    let v = PropertyValue::BackgroundAttachment(BackgroundAttachment::Scroll);
    assert_eq!(v.key(), PropertyKey::BackgroundAttachment);
}

// ── background-clip (CSS Backgrounds 3 §2.7) / background-origin (§2.8) ──

#[test]
fn background_clip_parse_all_three_keywords() {
    assert_eq!(
        parse("border-box", "background-clip"),
        Some(PropertyValue::BackgroundClip(VisualBox::BorderBox))
    );
    assert_eq!(
        parse("padding-box", "background-clip"),
        Some(PropertyValue::BackgroundClip(VisualBox::PaddingBox))
    );
    assert_eq!(
        parse("content-box", "background-clip"),
        Some(PropertyValue::BackgroundClip(VisualBox::ContentBox))
    );
}

#[test]
fn background_origin_parse_all_three_keywords() {
    assert_eq!(
        parse("border-box", "background-origin"),
        Some(PropertyValue::BackgroundOrigin(VisualBox::BorderBox))
    );
    assert_eq!(
        parse("padding-box", "background-origin"),
        Some(PropertyValue::BackgroundOrigin(VisualBox::PaddingBox))
    );
    assert_eq!(
        parse("content-box", "background-origin"),
        Some(PropertyValue::BackgroundOrigin(VisualBox::ContentBox))
    );
}

#[test]
fn background_clip_and_origin_are_case_insensitive() {
    assert_eq!(
        parse("BORDER-BOX", "background-clip"),
        Some(PropertyValue::BackgroundClip(VisualBox::BorderBox))
    );
    assert_eq!(
        parse("Padding-Box", "background-origin"),
        Some(PropertyValue::BackgroundOrigin(VisualBox::PaddingBox))
    );
}

#[test]
fn background_clip_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-clip"), None, "{keyword}");
    }
}

#[test]
fn background_origin_rejects_unknown_keyword() {
    assert_eq!(parse("fill-box", "background-origin"), None);
    assert_eq!(parse("16px", "background-origin"), None);
}

#[test]
fn background_clip_key_maps_to_background_clip_property_key() {
    let v = PropertyValue::BackgroundClip(VisualBox::BorderBox);
    assert_eq!(v.key(), PropertyKey::BackgroundClip);
}

#[test]
fn background_origin_key_maps_to_background_origin_property_key() {
    let v = PropertyValue::BackgroundOrigin(VisualBox::PaddingBox);
    assert_eq!(v.key(), PropertyKey::BackgroundOrigin);
}

// ── background-size (CSS Backgrounds 3 §2.9) ──

#[test]
fn background_size_parse_cover_and_contain() {
    assert_eq!(
        parse("cover", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Cover))
    );
    assert_eq!(
        parse("contain", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Contain))
    );
}

#[test]
fn background_size_parse_single_value_fills_auto_for_second_axis() {
    // spec verbatim: "If only one value is given the second is assumed
    // to be auto." — NOT a duplicate of the first value (unlike
    // `border-radius`'s 1-4 value fill rule).
    assert_eq!(
        parse("50%", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Percent(50.0)),
            height: LengthOrAuto::Auto,
        }))
    );
}

#[test]
fn background_size_parse_two_values() {
    assert_eq!(
        parse("100px 50%", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Px(100.0)),
            height: LengthOrAuto::Length(Length::Percent(50.0)),
        }))
    );
}

#[test]
fn background_size_parse_auto_auto() {
    assert_eq!(
        parse("auto auto", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Explicit {
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
        }))
    );
    // single bare `auto` also fills the second axis with `auto`.
    assert_eq!(
        parse("auto", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Explicit {
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
        }))
    );
}

#[test]
fn background_size_rejects_negative_length() {
    // `<length-percentage [0,∞]>` — negative values are grammar-invalid.
    assert_eq!(parse("-10px", "background-size"), None);
    // A present-but-invalid 2nd axis is not the same as an *omitted*
    // 2nd axis: `parse_background_size_axis` rejects `-10px` and the
    // wrapping `try_parse` rewinds, so the token survives as leftover
    // for the caller's `expect_exhausted` (`rule.rs`) to drop the whole
    // declaration — `parse` alone (no exhaustion check) would otherwise
    // silently observe only the 1st axis and default the 2nd to `auto`.
    assert_eq!(parse_entire("10px -10px", "background-size"), None);
}

#[test]
fn background_size_rejects_negative_percentage() {
    // Sibling of `background_size_rejects_negative_length` — the
    // `[0,∞]` bound applies to the whole `<length-percentage>`, not
    // just its `<length>` alternative. `length_payload` extracts the
    // numeric payload uniformly across `Length` variants including
    // `Percent`, so `parse_background_size_axis`'s `>= 0.0` gate
    // covers this case identically.
    assert_eq!(parse("-10%", "background-size"), None);
}

#[test]
fn background_size_accepts_percentage() {
    assert_eq!(
        parse("10%", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Percent(10.0)),
            height: LengthOrAuto::Auto,
        }))
    );
}

#[test]
fn background_size_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-size"), None, "{keyword}");
    }
}

#[test]
fn background_size_rejects_unknown_unit() {
    assert_eq!(parse("10vw", "background-size"), None);
}

#[test]
fn background_size_key_maps_to_background_size_property_key() {
    let v = PropertyValue::BackgroundSize(BackgroundSize::Cover);
    assert_eq!(v.key(), PropertyKey::BackgroundSize);
}

// ── background-position / `<position>` (CSS Backgrounds 3 §2.6) ──
//
// `<position>` grammar has 3 overlapping alternatives (`CssPosition`
// doc) — the test names below reference which alternative/branch each
// input exercises so a future regression is easy to localize.

#[test]
fn background_position_parse_single_keyword() {
    // 1st alternative, bare keyword — the other axis defaults to
    // `center`.
    assert_eq!(
        parse("center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
    assert_eq!(
        parse("left", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
    assert_eq!(
        parse("right", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(100.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
    assert_eq!(
        parse("top", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
    assert_eq!(
        parse("bottom", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(100.0)),
        }))
    );
}

#[test]
fn background_position_parse_single_length_percentage() {
    // 1st alternative, bare `<length-percentage>` — always horizontal,
    // vertical defaults to `center`.
    assert_eq!(
        parse("25%", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(25.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
}

#[test]
fn background_position_parse_two_bare_length_percentages() {
    // 2nd alternative: `[left|center|right|<LP>] [top|center|bottom|<LP>]`
    // — strict horizontal-then-vertical order, no reordering.
    assert_eq!(
        parse("10px 20px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Px(20.0)),
        }))
    );
}

#[test]
fn background_position_parse_two_keywords_reordered() {
    // 3rd alternative (`&&`, either order) — `top left` and `left top`
    // must produce the identical result.
    let expected = PropertyValue::BackgroundPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
        vertical: CssPositionOffset::Start(Length::Percent(0.0)),
    });
    assert_eq!(
        parse("left top", "background-position"),
        Some(expected.clone())
    );
    assert_eq!(parse("top left", "background-position"), Some(expected));
}

#[test]
fn background_position_parse_center_with_single_edge_keyword_either_order() {
    // `center` is ambiguous until the other token disambiguates it
    // (`parse_position_branch3` doc) — both orders must agree.
    let expected = PropertyValue::BackgroundPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
        vertical: CssPositionOffset::Start(Length::Percent(50.0)),
    });
    assert_eq!(
        parse("center left", "background-position"),
        Some(expected.clone())
    );
    assert_eq!(parse("left center", "background-position"), Some(expected));
}

#[test]
fn background_position_parse_keyword_then_bare_length_percentage() {
    // "left 10px" — `left` fills the horizontal slot (0%, no attached
    // offset — the 3rd alternative's `left <length-percentage>?`
    // greedily tries to consume `10px` as an offset first, but then
    // has nothing left for the mandatory vertical group and fails as a
    // whole; the 2nd alternative matches instead, treating `10px` as
    // the bare vertical value). See `parse_bg_position` doc.
    assert_eq!(
        parse("left 10px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Px(10.0)),
        }))
    );
}

#[test]
fn background_position_parse_right_10px_is_not_an_edge_offset() {
    // The well-known 2-value gotcha: "right 10px" does NOT mean "10px
    // from the right edge" — with only 2 tokens the 3rd alternative
    // (edge-offset form) cannot satisfy its mandatory vertical group,
    // so the 2nd alternative wins: horizontal = `right` (100%),
    // vertical = the bare `10px`. The edge-offset reading requires a
    // 3rd token (see `background_position_parse_edge_offset_three_values`).
    assert_eq!(
        parse("right 10px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(100.0)),
            vertical: CssPositionOffset::Start(Length::Px(10.0)),
        }))
    );
}

#[test]
fn background_position_parse_edge_offset_three_values() {
    // 3rd alternative, 3 tokens: an offset attached to one edge, the
    // other axis a bare `center`.
    assert_eq!(
        parse("right 10px center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
}

#[test]
fn background_position_parse_edge_offset_four_values() {
    // 3rd alternative, 4 tokens, both axes carrying an explicit offset
    // — matches CSS Backgrounds 3 §2.6's own worked example verbatim
    // ("a 10px upward offset from the bottom and 20px leftward offset
    // from the right edge").
    assert_eq!(
        parse("bottom 10px right 20px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(20.0)),
            vertical: CssPositionOffset::End(Length::Px(10.0)),
        }))
    );
}

#[test]
fn background_position_parse_edge_offset_missing_offset_defaults_to_zero() {
    // 3rd alternative, 3 tokens: the edge with no attached offset
    // defaults to `0` — `right` alone normalizes to `Start(100%)`
    // (`normalize_css_position_offset` collapses a percentage `End`
    // back to `Start`).
    assert_eq!(
        parse("bottom 10px right", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(100.0)),
            vertical: CssPositionOffset::End(Length::Px(10.0)),
        }))
    );
}

#[test]
fn background_position_accepts_negative_length_offset() {
    // Unlike `background-size`, `<position>`'s `<length-percentage>`
    // has no `[0,∞]` restriction — negative offsets are spec-valid
    // ("outward" offsets, per the propdef's offset-computation prose).
    assert_eq!(
        parse("bottom -10px right -20px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(-20.0)),
            vertical: CssPositionOffset::End(Length::Px(-10.0)),
        }))
    );
}

#[test]
fn background_position_parse_edge_offset_percentage_normalizes_non_zero() {
    // `normalize_css_position_offset` folds `End(Percent(p))` back to
    // `Start(Percent(100.0 - p))` — the other tests above only exercise
    // this at `p = 0` (`right` alone, via
    // `background_position_parse_edge_offset_missing_offset_defaults_to_zero`).
    // A non-zero `p` proves the subtraction itself, not just the
    // identity case. `right 30%` measures 30% in from the right edge,
    // which is the same physical point as 70% in from the left edge —
    // the two are interchangeable because both `Start` and `End`
    // percentages share the same basis (CSS Backgrounds 3 §2.6:
    // "refer to size of background positioning area minus size of
    // background image"), whatever that basis resolves to, so
    // `100% - 30% = 70%` lands on the correct point without this
    // helper itself needing to know that basis.
    assert_eq!(
        parse("right 30% center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(70.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
}

#[test]
fn background_position_parse_branch2_vertical_keywords_with_bare_horizontal_length() {
    // "<LP> top|center|bottom" only reaches the 2nd alternative's
    // vertical keyword arms (`parse_position_branch2_vertical`) when
    // the horizontal side is a bare `<length-percentage>` — every
    // keyword-pair input elsewhere in this file (`"left top"` etc.) is
    // claimed by the 3rd (edge-offset) alternative first, since that
    // one is tried before the 2nd (`parse_bg_position` doc).
    assert_eq!(
        parse("10px top", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
    assert_eq!(
        parse("10px center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
    assert_eq!(
        parse("10px bottom", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Percent(100.0)),
        }))
    );
}

#[test]
fn background_position_parse_branch2_horizontal_center_with_bare_vertical_length() {
    // Sibling of the test above, horizontal side of the 2nd
    // alternative: "center <LP>" reaches `parse_position_branch2_horizontal`'s
    // `center` arm — every other `center`-with-keyword input in this
    // file is claimed by the 3rd alternative first.
    assert_eq!(
        parse("center 10px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Px(10.0)),
        }))
    );
}

#[test]
fn background_position_parse_edge_keyword_then_center() {
    // "top center" — the 3rd alternative's vertical-edge-first branch
    // (`top` matches `parse_position_vertical_edge`) leaves `center`
    // for the mandatory horizontal group, reaching
    // `parse_position_horizontal_group`'s own `center` arm (distinct
    // from the ambiguous-`center`-first branch exercised by
    // `background_position_parse_center_with_single_edge_keyword_either_order`,
    // which starts from `center` rather than ending on it).
    assert_eq!(
        parse("top center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
}

#[test]
fn background_position_parse_center_then_vertical_edge() {
    // "center top" — `parse_position_branch3`'s ambiguous-`center`
    // branch resolves by trying the vertical edge first; distinct from
    // `background_position_parse_center_with_single_edge_keyword_either_order`,
    // which only ever pairs `center` with a *horizontal* edge (`left`).
    assert_eq!(
        parse("center top", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
}

#[test]
fn background_position_parse_center_center() {
    // "center center" — `parse_position_branch3`'s ambiguous-`center`
    // branch falls through both the vertical-edge and horizontal-edge
    // attempts before matching the explicit trailing `center` ident.
    assert_eq!(
        parse("center center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
}

#[test]
fn background_position_rejects_two_horizontal_keywords() {
    // "left right" — after `left` fills the horizontal slot, `right`
    // has nowhere to go in any alternative and is left as unconsumed
    // trailing garbage.
    assert_eq!(parse_entire("left right", "background-position"), None);
}

#[test]
fn background_position_rejects_two_vertical_keywords() {
    assert_eq!(parse_entire("top bottom", "background-position"), None);
}

#[test]
fn background_position_rejects_trailing_garbage() {
    // "left center 20px" — 2 tokens fully satisfy the 3rd alternative
    // (`left`, `center`), leaving `20px` as leftover with no
    // preceding edge keyword to attach to.
    assert_eq!(
        parse_entire("left center 20px", "background-position"),
        None
    );
}

#[test]
fn background_position_is_case_insensitive() {
    assert_eq!(
        parse("TOP LEFT", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
}

#[test]
fn background_position_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-position"), None, "{keyword}");
    }
}

#[test]
fn background_position_key_maps_to_background_position_property_key() {
    let v = PropertyValue::BackgroundPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
        vertical: CssPositionOffset::Start(Length::Percent(0.0)),
    });
    assert_eq!(v.key(), PropertyKey::BackgroundPosition);
}

// ── background-image (CSS Backgrounds and Borders 3 §2.3) ──

#[test]
fn background_image_parse_none() {
    assert_eq!(
        parse("none", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::None))
    );
}

#[test]
fn background_image_parse_url_unquoted_form() {
    assert_eq!(
        parse("url(foo.png)", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Url(
            "foo.png".to_string()
        )))
    );
}

#[test]
fn background_image_parse_url_quoted_form() {
    assert_eq!(
        parse("url(\"foo.png\")", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Url(
            "foo.png".to_string()
        )))
    );
}

#[test]
fn background_image_is_case_insensitive() {
    // `none` keyword は ASCII case-insensitive (他の keyword-only property
    // と同じ扱い、`background-repeat` の `REPEAT-X`/`No-Repeat` test 参照)。
    assert_eq!(
        parse("NONE", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::None))
    );
}

// ── background-image: <gradient> (CSS Images 4 §3) ──

const RED: CssColor = CssColor {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};
const BLUE: CssColor = CssColor {
    r: 0,
    g: 0,
    b: 255,
    a: 255,
};

fn gradient_stop(color: CssColor, position: Option<Length>) -> GradientColorStop {
    GradientColorStop {
        color: GradientStopColor::Resolved(color),
        position,
    }
}

fn oklab_shorter() -> GradientColorInterpolation {
    GradientColorInterpolation {
        color_space: MixColorSpace::Oklab,
        hue_method: HueInterpolationMethod::Shorter,
    }
}

fn expect_linear_gradient(value: Option<PropertyValue>) -> LinearGradient {
    match value {
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(Gradient::Linear(g)))) => g,
        // cov:ignore: this branch only executes when a caller's `parse(...)`
        // unexpectedly fails to produce a linear gradient — every call site
        // below passes, so llvm-cov reports this panic arm as an uncovered
        // added line even though the successful branch above (and therefore
        // this helper itself) is exercised by every one of those call sites.
        other => panic!("expected a linear gradient BackgroundImage, got {other:?}"),
    }
}

fn expect_radial_gradient(value: Option<PropertyValue>) -> RadialGradient {
    match value {
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(Gradient::Radial(g)))) => g,
        // cov:ignore: same reasoning as `expect_linear_gradient`'s panic arm.
        other => panic!("expected a radial gradient BackgroundImage, got {other:?}"),
    }
}

fn expect_conic_gradient(value: Option<PropertyValue>) -> ConicGradient {
    match value {
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(Gradient::Conic(g)))) => g,
        // cov:ignore: same reasoning as `expect_linear_gradient`'s panic arm.
        other => panic!("expected a conic gradient BackgroundImage, got {other:?}"),
    }
}

#[test]
fn background_image_parses_linear_gradient_with_default_direction_and_interpolation() {
    // No direction, no `in ...` clause — both spec-mandated defaults
    // (`to bottom` / `Oklab`) are baked in.
    assert_eq!(
        parse("linear-gradient(red, blue)", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(
            Gradient::Linear(LinearGradient {
                repeating: false,
                direction: LinearGradientDirection::Side(SideOrCorner {
                    horizontal: None,
                    vertical: Some(VerticalSide::Bottom),
                }),
                interpolation: oklab_shorter(),
                stops: Arc::new(vec![gradient_stop(RED, None), gradient_stop(BLUE, None),]),
            })
        )))
    );
}

#[test]
fn background_image_parses_repeating_linear_gradient_sets_repeating_flag() {
    let g = expect_linear_gradient(parse(
        "repeating-linear-gradient(red, blue)",
        "background-image",
    ));
    assert!(g.repeating);
}

#[test]
fn background_image_parses_linear_gradient_angle_direction() {
    let g = expect_linear_gradient(parse(
        "linear-gradient(45deg, red, blue)",
        "background-image",
    ));
    assert_eq!(g.direction, LinearGradientDirection::Angle(Angle(45.0)));
}

#[test]
fn background_image_parses_linear_gradient_unitless_zero_angle() {
    // `<angle> | <zero>` — legacy bare `0` is valid (`parse_angle` doc).
    let g = expect_linear_gradient(parse("linear-gradient(0, red, blue)", "background-image"));
    assert_eq!(g.direction, LinearGradientDirection::Angle(Angle(0.0)));
}

#[test]
fn background_image_rejects_linear_gradient_bare_nonzero_number_as_angle() {
    // Unlike `<zero>`, a bare non-zero number is not a valid `<angle>`.
    assert_eq!(
        parse("linear-gradient(45, red, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_rejects_linear_gradient_unrecognized_angle_unit() {
    assert_eq!(
        parse("linear-gradient(45xyz, red, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_parses_linear_gradient_angle_overflow_saturates_to_f32_max() {
    // `1e40turn` overflows f32 only *after* the `* 360.0` unit
    // conversion — same "convert, then saturate" policy `parse_length_value`
    // uses for percentages (`parse_angle` doc's "overflow saturation"
    // note).
    let g = expect_linear_gradient(parse(
        "linear-gradient(1e40turn, red, blue)",
        "background-image",
    ));
    assert_eq!(g.direction, LinearGradientDirection::Angle(Angle(f32::MAX)));
}

#[test]
fn background_image_parses_conic_gradient_stop_percentage_overflow_saturates_to_f32_max() {
    let g = expect_conic_gradient(parse("conic-gradient(red 1e40%, blue)", "background-image"));
    assert_eq!(
        g.stops[0].position,
        Some(AnglePercentage::Percent(f32::MAX))
    );
}

#[test]
fn background_image_parses_linear_gradient_to_bottom_explicit() {
    // Explicit `to bottom` — exercises `parse_vertical_side`'s `bottom`
    // arm directly, distinct from the same *value* reached via the
    // omitted-direction default (`..._with_default_direction_and_interpolation`).
    let g = expect_linear_gradient(parse(
        "linear-gradient(to bottom, red, blue)",
        "background-image",
    ));
    assert_eq!(
        g.direction,
        LinearGradientDirection::Side(SideOrCorner {
            horizontal: None,
            vertical: Some(VerticalSide::Bottom),
        })
    );
}

#[test]
fn background_image_rejects_linear_gradient_bare_to_keyword() {
    // `to` with no side-or-corner keyword following it — `parse_side_or_corner`
    // rejects when neither axis matched.
    assert_eq!(
        parse("linear-gradient(to, red, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_rejects_linear_gradient_side_followed_by_non_side_keyword() {
    // `right` matches the horizontal axis; `center` matches neither axis
    // of `parse_vertical_side`, exercising its rejection arm before the
    // whole declaration fails on the missing comma.
    assert_eq!(
        parse(
            "linear-gradient(to right center, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_rejects_conic_gradient_single_stop() {
    // `<angular-color-stop-list>` requires 2+ stops, same as
    // `GradientColorStop`'s linear/radial sibling.
    assert_eq!(parse("conic-gradient(red)", "background-image"), None);
}

#[test]
fn background_image_parses_linear_gradient_side_and_corner_any_order() {
    // `[left | right] || [top | bottom]` — keyword order doesn't matter.
    let to_top_left = parse(
        "linear-gradient(to top left, red, blue)",
        "background-image",
    );
    let to_left_top = parse(
        "linear-gradient(to left top, red, blue)",
        "background-image",
    );
    let expected = Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(
        Gradient::Linear(LinearGradient {
            repeating: false,
            direction: LinearGradientDirection::Side(SideOrCorner {
                horizontal: Some(HorizontalSide::Left),
                vertical: Some(VerticalSide::Top),
            }),
            interpolation: oklab_shorter(),
            stops: Arc::new(vec![gradient_stop(RED, None), gradient_stop(BLUE, None)]),
        }),
    )));
    assert_eq!(to_top_left, expected);
    assert_eq!(to_left_top, expected);
}

#[test]
fn background_image_parses_linear_gradient_stop_positions() {
    let g = expect_linear_gradient(parse(
        "linear-gradient(red 10%, blue 90%)",
        "background-image",
    ));
    assert_eq!(
        *g.stops,
        vec![
            gradient_stop(RED, Some(Length::Percent(10.0))),
            gradient_stop(BLUE, Some(Length::Percent(90.0))),
        ]
    );
}

#[test]
fn background_image_gradient_stop_accepts_currentcolor() {
    let g = expect_linear_gradient(parse(
        "linear-gradient(currentcolor, blue)",
        "background-image",
    ));
    assert_eq!(g.stops[0].color, GradientStopColor::CurrentColor);
}

#[test]
fn background_image_rejects_linear_gradient_single_stop() {
    // CSS Images 3 baseline grammar requires 2+ stops (`BackgroundImage`
    // doc's scope-carving note — Level 4's single-stop relaxation is
    // deferred).
    assert_eq!(parse("linear-gradient(red)", "background-image"), None);
}

#[test]
fn background_image_rejects_linear_gradient_transition_hint() {
    // `<linear-color-hint>` between stops is unimplemented scope — the
    // bare `50%` token isn't a valid `<color>`, so the whole
    // comma-separated stop list fails to parse.
    assert_eq!(
        parse("linear-gradient(red, 50%, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_parses_linear_gradient_color_interpolation_method() {
    let g = expect_linear_gradient(parse(
        "linear-gradient(in oklch longer hue, red, blue)",
        "background-image",
    ));
    assert_eq!(
        g.interpolation,
        GradientColorInterpolation {
            color_space: MixColorSpace::Oklch,
            hue_method: HueInterpolationMethod::Longer,
        }
    );
}

#[test]
fn background_image_rejects_hue_method_on_non_polar_interpolation_space() {
    // `<hue-interpolation-method>` is only valid for a polar `<color-space>`
    // (`Lch`/`Oklch`) — same rule `color-mix()` already enforces.
    assert_eq!(
        parse(
            "linear-gradient(in srgb longer hue, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_rejects_unimplemented_interpolation_color_space() {
    // `hsl`/`hwb`/`xyz` family have no `<color>` function parser in this
    // crate (`MixColorSpace` doc's scope-carving note), so they aren't
    // offered as gradient interpolation spaces either.
    assert_eq!(
        parse("linear-gradient(in hsl, red, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_parses_linear_gradient_direction_and_interpolation_together_either_order() {
    // CSS Images 4 §3.1.1's own worked example, plus the reverse
    // ordering — the `||` combinator in `[ [ <angle> | <zero> | to
    // <side-or-corner> ] || <color-interpolation-method> ]?` permits
    // either order, exercising both branches of
    // `parse_linear_gradient_body`'s any-order loop in one gradient
    // rather than direction-only and interpolation-only separately.
    let direction_first = expect_linear_gradient(parse(
        "linear-gradient(in lab to right, #F01, #081)",
        "background-image",
    ));
    let interpolation_first = expect_linear_gradient(parse(
        "linear-gradient(to right in lab, #F01, #081)",
        "background-image",
    ));
    let expected_direction = LinearGradientDirection::Side(SideOrCorner {
        horizontal: Some(HorizontalSide::Right),
        vertical: None,
    });
    let expected_interpolation = GradientColorInterpolation {
        color_space: MixColorSpace::Lab,
        hue_method: HueInterpolationMethod::Shorter,
    };
    assert_eq!(direction_first.direction, expected_direction);
    assert_eq!(direction_first.interpolation, expected_interpolation);
    assert_eq!(interpolation_first.direction, expected_direction);
    assert_eq!(interpolation_first.interpolation, expected_interpolation);
}

#[test]
fn background_image_rejects_radial_gradient_bare_percentage_size() {
    // CSS Images 3 §3.2.1: "Percentages are not allowed here" for the
    // circle-radius `<length [0,∞]>` alternative — a bare `50%` (no
    // second value) doesn't match the 2-value ellipse form either, so
    // `parse_radial_size` has no alternative left to try.
    assert_eq!(
        parse("radial-gradient(50%, red, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_rejects_radial_gradient_position_before_shape() {
    // CSS Images 4 §3.2.1's `[ <radial-shape> || <radial-size> ]? [ at
    // <position> ]?` is a *sequence* of two groups — `at <position>`
    // may only follow the shape/size group, never precede it.
    // `parse_radial_shape_size_position_group` claims `at center` as a
    // position-only match, leaving `circle` as an unconsumed leftover
    // token that fails the subsequent `expect_comma()`.
    assert_eq!(
        parse(
            "radial-gradient(at center circle, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_rejects_conic_gradient_position_before_from_angle() {
    // Angular sibling of `..._rejects_radial_gradient_position_before_shape`
    // — CSS Images 4 §3.3.1's `[ from [...] ]? [ at <position> ]?` is
    // likewise a sequence, `from` before `at`.
    assert_eq!(
        parse(
            "conic-gradient(at center from 45deg, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_parses_radial_gradient_with_default_shape_size_and_position() {
    // No shape/size/position/interpolation — `ellipse farthest-corner at
    // center` / `Oklab` are all spec-mandated defaults.
    assert_eq!(
        parse("radial-gradient(red, blue)", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(
            Gradient::Radial(RadialGradient {
                repeating: false,
                shape: RadialShape::Ellipse,
                size: RadialSize::Extent(RadialExtent::FarthestCorner),
                position: CssPosition {
                    horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
                    vertical: CssPositionOffset::Start(Length::Percent(50.0)),
                },
                interpolation: oklab_shorter(),
                stops: Arc::new(vec![gradient_stop(RED, None), gradient_stop(BLUE, None)]),
            })
        )))
    );
}

#[test]
fn background_image_parses_repeating_radial_gradient_sets_repeating_flag() {
    let g = expect_radial_gradient(parse(
        "repeating-radial-gradient(red, blue)",
        "background-image",
    ));
    assert!(g.repeating);
}

#[test]
fn background_image_parses_radial_gradient_circle_with_explicit_length() {
    // Spec's own CSS Images 3 §3.2.1 example.
    let g = expect_radial_gradient(parse(
        "radial-gradient(5em circle at top left, yellow, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Circle);
    assert_eq!(g.size, RadialSize::Circle(Length::Em(5.0)));
    assert_eq!(
        g.position,
        CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }
    );
}

#[test]
fn background_image_parses_radial_gradient_ellipse_with_two_lengths() {
    let g = expect_radial_gradient(parse(
        "radial-gradient(20px 30px, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Ellipse);
    assert_eq!(
        g.size,
        RadialSize::Ellipse(Length::Px(20.0), Length::Px(30.0))
    );
}

#[test]
fn background_image_parses_radial_gradient_bare_length_with_no_shape_keyword_infers_circle() {
    // Shape omitted + a single bare `<length>` (no percentage) — defaults
    // to circle (CSS Images 3 §3.2.1's "a single `<length>`" rule),
    // distinct from `..._circle_with_explicit_length` above (which
    // spells `circle` explicitly and exercises a different
    // `resolve_radial_shape_and_size` arm).
    let g = expect_radial_gradient(parse(
        "radial-gradient(5px at center, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Circle);
    assert_eq!(g.size, RadialSize::Circle(Length::Px(5.0)));
}

#[test]
fn background_image_parses_radial_gradient_extent_keyword_infers_ellipse() {
    // Shape omitted + `<radial-extent>` keyword (not "a single <length>")
    // — defaults to ellipse (CSS Images 3 §3.2.1).
    let g = expect_radial_gradient(parse(
        "radial-gradient(closest-side, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Ellipse);
    assert_eq!(g.size, RadialSize::Extent(RadialExtent::ClosestSide));
}

#[test]
fn background_image_parses_radial_gradient_explicit_shape_with_extent_keyword() {
    // Explicit shape keyword *and* explicit extent keyword together —
    // distinct `resolve_radial_shape_and_size` arm from both the
    // shape-omitted case above and the shape-alone case below.
    let g = expect_radial_gradient(parse(
        "radial-gradient(circle closest-side, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Circle);
    assert_eq!(g.size, RadialSize::Extent(RadialExtent::ClosestSide));
}

#[test]
fn background_image_parses_radial_gradient_explicit_ellipse_shape_with_two_lengths() {
    // Explicit `ellipse` shape keyword *and* explicit 2-length size
    // together — the sibling combination to
    // `..._circle_with_explicit_length` (`circle` + single length).
    let g = expect_radial_gradient(parse(
        "radial-gradient(ellipse 20px 30px, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Ellipse);
    assert_eq!(
        g.size,
        RadialSize::Ellipse(Length::Px(20.0), Length::Px(30.0))
    );
}

#[test]
fn background_image_parses_radial_gradient_shape_keyword_alone_defaults_to_farthest_corner() {
    // Explicit shape keyword, no size at all — `farthest-corner` default
    // still applies (distinct from the fully-omitted default test above,
    // which never names a shape keyword).
    let g = expect_radial_gradient(parse(
        "radial-gradient(circle, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Circle);
    assert_eq!(g.size, RadialSize::Extent(RadialExtent::FarthestCorner));
}

#[test]
fn background_image_parses_radial_gradient_explicit_color_interpolation_method() {
    let g = expect_radial_gradient(parse(
        "radial-gradient(in oklch, red, blue)",
        "background-image",
    ));
    assert_eq!(
        g.interpolation,
        GradientColorInterpolation {
            color_space: MixColorSpace::Oklch,
            hue_method: HueInterpolationMethod::Shorter,
        }
    );
}

#[test]
fn background_image_rejects_radial_gradient_circle_shape_with_ellipse_size() {
    // `circle` + a 2-length-percentage (ellipse-only) size is an invalid
    // combination (CSS Images 3 §3.2.1's expanded grammar).
    assert_eq!(
        parse(
            "radial-gradient(circle 20px 30px, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_rejects_radial_gradient_negative_length() {
    assert_eq!(
        parse(
            "radial-gradient(circle -5px, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_rejects_linear_gradient_double_position_stop() {
    // Level 4's `<color-stop-length> = <length-percentage>{1,2}` (one
    // stop, two positions) is deferred scope (`GradientColorStop` doc)
    // — this pins that the second position is *rejected*, not silently
    // discarded: after `parse_gradient_color_stop` consumes one
    // `<length-percentage>`, the leftover `20%` token makes
    // `parse_comma_separated`'s per-segment parse fail, dropping the
    // whole declaration rather than producing a stop at `10%` alone.
    assert_eq!(
        parse("linear-gradient(red 10% 20%, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_rejects_conic_gradient_double_angle_stop() {
    // Angular sibling of `..._rejects_linear_gradient_double_position_stop`.
    assert_eq!(
        parse("conic-gradient(red 0deg 90deg, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_parses_conic_gradient_with_default_angle_and_position() {
    assert_eq!(
        parse("conic-gradient(red, blue)", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(
            Gradient::Conic(ConicGradient {
                repeating: false,
                angle: Angle(0.0),
                position: CssPosition {
                    horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
                    vertical: CssPositionOffset::Start(Length::Percent(50.0)),
                },
                interpolation: oklab_shorter(),
                stops: Arc::new(vec![
                    AngularColorStop {
                        color: GradientStopColor::Resolved(RED),
                        position: None,
                    },
                    AngularColorStop {
                        color: GradientStopColor::Resolved(BLUE),
                        position: None,
                    },
                ]),
            })
        )))
    );
}

#[test]
fn background_image_parses_repeating_conic_gradient_sets_repeating_flag() {
    let g = expect_conic_gradient(parse(
        "repeating-conic-gradient(gold, #f06 20deg)",
        "background-image",
    ));
    assert!(g.repeating);
}

#[test]
fn background_image_parses_conic_gradient_from_angle_and_position() {
    let g = expect_conic_gradient(parse(
        "conic-gradient(from 45deg at 25% 40%, white, black)",
        "background-image",
    ));
    assert_eq!(g.angle, Angle(45.0));
    assert_eq!(
        g.position,
        CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(25.0)),
            vertical: CssPositionOffset::Start(Length::Percent(40.0)),
        }
    );
}

#[test]
fn background_image_parses_conic_gradient_stop_with_percentage_position() {
    let g = expect_conic_gradient(parse(
        "conic-gradient(#f06 0%, gold 100%)",
        "background-image",
    ));
    assert_eq!(g.stops[0].position, Some(AnglePercentage::Percent(0.0)));
}

#[test]
fn background_image_parses_conic_gradient_stop_with_angle_position() {
    let g = expect_conic_gradient(parse(
        "conic-gradient(#f06 0deg, gold 1turn)",
        "background-image",
    ));
    assert_eq!(
        g.stops[1].position,
        Some(AnglePercentage::Angle(Angle(360.0)))
    );
}

#[test]
fn background_image_parses_conic_gradient_explicit_color_interpolation_method() {
    let g = expect_conic_gradient(parse(
        "conic-gradient(in oklch, red, blue)",
        "background-image",
    ));
    assert_eq!(
        g.interpolation,
        GradientColorInterpolation {
            color_space: MixColorSpace::Oklch,
            hue_method: HueInterpolationMethod::Shorter,
        }
    );
}

#[test]
fn background_image_rejects_bare_string_without_url_wrapper() {
    // `<image>` は `<url> | <gradient>` のみで bare `<string>` を含まない
    // — `parse_url_value` doc の同節参照 (`content_bare_string_is_still_literal_not_image`
    // と同型の regression check)。
    assert_eq!(parse("\"foo.png\"", "background-image"), None);
}

#[test]
fn background_image_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-image"), None, "{keyword}");
    }
}

#[test]
fn background_image_rejects_unknown_keyword() {
    assert_eq!(parse("foo", "background-image"), None);
}

#[test]
fn background_image_key_maps_to_background_image_property_key() {
    let v = PropertyValue::BackgroundImage(BackgroundImage::None);
    assert_eq!(v.key(), PropertyKey::BackgroundImage);
}

// ── background-* comma-list (multi-layer) rejection ──

#[test]
fn background_longhands_reject_comma_separated_multi_layer() {
    // All 7 background-* longhands are single-layer only; comma-separated
    // multi-layer (#-list) must be whole-declaration-drop via
    // `DeclParser::parse_value` + `expect_exhausted` (rule.rs:1094 etc.).
    // This pins that a future refactor never silently truncates to the
    // first layer ("first layer wins") — which would paint the wrong
    // background instead of falling through to the previous declaration
    // or the initial value.
    let cases: &[(&str, &str)] = &[
        ("background-repeat", "repeat, no-repeat"),
        ("background-attachment", "scroll, fixed"),
        ("background-clip", "border-box, padding-box"),
        ("background-origin", "padding-box, content-box"),
        ("background-size", "cover, contain"),
        ("background-position", "left top, right bottom"),
        ("background-image", "url(a.png), url(b.png)"),
    ];
    for (name, source) in cases {
        assert_eq!(
            parse_entire(source, name),
            None,
            "{name}: {source:?} should be whole-declaration-drop"
        );
        // The leading layer itself is valid — so the rejection is solely
        // due to the trailing `, <layer>` leftover caught by
        // `expect_exhausted`, not the parser rejecting the first token.
        assert!(
            parse(source, name).is_some(),
            "{name}: first layer of {source:?} should still parse without exhaustion check"
        );
    }
}

#[test]
fn background_image_rejects_comma_list_with_none_and_gradient_variants() {
    // `none` and `<gradient>` are also valid single layers; mixing them
    // with a comma must still be whole-declaration-drop.
    for source in [
        "none, url(b.png)",
        "url(a.png), none",
        "none, none",
        "linear-gradient(red, blue), url(b.png)",
        "url(a.png), linear-gradient(red, blue)",
    ] {
        assert_eq!(
            parse_entire(source, "background-image"),
            None,
            "background-image: {source:?} should be whole-declaration-drop"
        );
    }
}

// ── `background` shorthand (CSS Backgrounds and Borders 3 §2.10) ──

fn expect_background(value: Option<PropertyValue>) -> BackgroundShorthand {
    match value {
        Some(PropertyValue::Background(shorthand)) => shorthand,
        // cov:ignore: this branch only executes when a caller's
        // `parse(..., "background")` unexpectedly fails to parse or
        // parses to the wrong variant; every call site in this test
        // module passes valid `background` shorthand input, so the
        // panic never fires while the tests pass.
        other => panic!("expected PropertyValue::Background, got {other:?}"),
    }
}

/// Spec §2.10 verbatim first worked example: "In the first rule …, only
/// a value for background-color has been given and the other individual
/// properties are set to their initial values." — `body { background:
/// red }` is spec-equivalent to setting all 8 longhands, 7 of them to
/// their initial value.
#[test]
fn background_shorthand_color_only_fills_the_other_7_with_initial_values() {
    let got = expect_background(parse("red", "background"));
    assert_eq!(
        got,
        BackgroundShorthand {
            color: CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            },
            image: BackgroundImage::None,
            repeat: BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::Repeat,
            },
            attachment: BackgroundAttachment::Scroll,
            position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
                vertical: CssPositionOffset::Start(Length::Percent(0.0)),
            },
            size: BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            },
            // 0 `<visual-box>` occurrence: the 2 longhands fall back to
            // their own (different) initial values, not to each other.
            clip: VisualBox::BorderBox,
            origin: VisualBox::PaddingBox,
        }
    );
}

/// Spec §2.10 verbatim second worked example: `p { background:
/// url("chess.png") 40% / 10em gray round fixed border-box; }` is
/// spec-equivalent to `background-color: gray; background-position: 40%
/// 50%; background-size: 10em auto; background-repeat: round;
/// background-clip: border-box; background-origin: border-box;
/// background-attachment: fixed; background-image: url(chess.png)`.
/// Single `<visual-box>` occurrence (`border-box`) sets **both**
/// origin and clip to it.
#[test]
fn background_shorthand_spec_example_url_position_size_color_repeat_attachment_box() {
    let got = expect_background(parse(
        "url(\"chess.png\") 40% / 10em gray round fixed border-box",
        "background",
    ));
    assert_eq!(got.image, BackgroundImage::Url("chess.png".to_string()));
    assert_eq!(
        got.position,
        CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(40.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }
    );
    assert_eq!(
        got.size,
        BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Em(10.0)),
            height: LengthOrAuto::Auto,
        }
    );
    assert_eq!(
        got.color,
        CssColor {
            r: 128,
            g: 128,
            b: 128,
            a: 255
        }
    );
    assert_eq!(
        got.repeat,
        BackgroundRepeat {
            x: BackgroundRepeatKeyword::Round,
            y: BackgroundRepeatKeyword::Round,
        }
    );
    assert_eq!(got.attachment, BackgroundAttachment::Fixed);
    assert_eq!(got.clip, VisualBox::BorderBox);
    assert_eq!(got.origin, VisualBox::BorderBox);
}

/// Spec §2.10 verbatim third worked example: `div { background:
/// padding-box url(paper.jpg) white center }` is spec-equivalent to
/// `background-color: white; background-image: url(paper.jpg);
/// background-repeat: repeat; background-attachment: scroll;
/// background-position: center; background-clip: padding-box;
/// background-origin: padding-box; background-size: auto auto`. Also
/// exercises `||` reordering: the `<visual-box>` component appears
/// *before* the image/color components in this example.
#[test]
fn background_shorthand_spec_example_box_before_image_and_color() {
    let got = expect_background(parse(
        "padding-box url(paper.jpg) white center",
        "background",
    ));
    assert_eq!(got.clip, VisualBox::PaddingBox);
    assert_eq!(got.origin, VisualBox::PaddingBox);
    assert_eq!(got.image, BackgroundImage::Url("paper.jpg".to_string()));
    assert_eq!(
        got.color,
        CssColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255
        }
    );
    assert_eq!(
        got.position,
        CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }
    );
    assert_eq!(
        got.repeat,
        BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::Repeat,
        }
    );
    assert_eq!(got.attachment, BackgroundAttachment::Scroll);
    assert_eq!(
        got.size,
        BackgroundSize::Explicit {
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
        }
    );
}

/// Spec §2.10's 2nd worked example (`E { background: #CCC
/// url("metal.jpg") top left / 100% auto no-repeat}`), single-layer
/// portion only — this crate does not accept the spec's other example
/// (`background: url(a.png) top left no-repeat, …`, comma-separated
/// multi-layer, see `background_shorthand_rejects_comma_separated_multi_layer`
/// below). `top left` (keyword reordering, only reachable via
/// `parse_bg_position`'s `&&` branch — CSS Position 3 §2's
/// non-reordering 2-value form rejects `top` in the horizontal slot)
/// immediately followed by `/ 100% auto` exercises the atomic
/// position+size `||` component with a non-trivial position.
#[test]
fn background_shorthand_reordered_position_before_slash_size() {
    let got = expect_background(parse(
        "#CCC url(\"metal.jpg\") top left / 100% auto no-repeat",
        "background",
    ));
    assert_eq!(
        got.color,
        CssColor {
            r: 204,
            g: 204,
            b: 204,
            a: 255
        }
    );
    assert_eq!(got.image, BackgroundImage::Url("metal.jpg".to_string()));
    assert_eq!(
        got.position,
        CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }
    );
    assert_eq!(
        got.size,
        BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Percent(100.0)),
            height: LengthOrAuto::Auto,
        }
    );
    assert_eq!(
        got.repeat,
        BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        }
    );
}

/// 2 distinct `<visual-box>` occurrences: first sets `background-origin`,
/// second sets `background-clip` (spec §2.10 verbatim, [`BackgroundShorthand`]
/// doc). Neither of the spec's own worked examples exercises 2
/// *different* values (its only 2-occurrence-adjacent example still
/// repeats the same keyword), so this is a crate-authored regression check
/// for the origin-then-clip assignment order specifically.
#[test]
fn background_shorthand_two_distinct_visual_boxes_assign_origin_then_clip() {
    let got = expect_background(parse("content-box border-box", "background"));
    assert_eq!(got.origin, VisualBox::ContentBox);
    assert_eq!(got.clip, VisualBox::BorderBox);
}

/// `<bg-position>`'s 3-4 value edge-offset form (`CssPosition` doc's
/// `parse_bg_position` — branch3) immediately followed by another `||`
/// component. Unlike the longhand `background-position` parse path
/// (where leftover tokens always mean rejection via `expect_exhausted`),
/// the shorthand loop hands leftover tokens to the *next* component —
/// this pins that `bottom 10px right 20px` stops exactly at its own 4
/// tokens and does not swallow `no-repeat`.
#[test]
fn background_shorthand_edge_offset_position_stops_before_next_component() {
    let got = expect_background(parse("bottom 10px right 20px no-repeat", "background"));
    assert_eq!(
        got.position,
        CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(20.0)),
            vertical: CssPositionOffset::End(Length::Px(10.0)),
        }
    );
    assert_eq!(
        got.repeat,
        BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        }
    );
}

/// spec `||` grammar: "one or more of them must occur" — 0 component is
/// invalid.
#[test]
fn background_shorthand_rejects_empty_value() {
    assert_eq!(parse("", "background"), None);
}

/// An unrecognized ident matches no component's grammar at all (not
/// `none`/`url()`/gradient, not a `<repeat-style>`/`<attachment>`/
/// `<visual-box>` keyword, not a `<position>` keyword, not a named
/// color) — same "0 component" rejection as the empty-value case above.
#[test]
fn background_shorthand_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "background"), None);
}

/// spec §2.10's `<bg-layer>#? , <final-bg-layer>` grammar allows
/// comma-separated multi-layer values (see the spec's own 4th example,
/// `background: url(a.png) top left no-repeat, url(b.png) center /
/// 100% 100% no-repeat, url(c.png) white`). This crate's single-layer
/// scope carving ([`BackgroundShorthand`] doc's Non-goal section) does
/// not silently take the first layer — it rejects the whole declaration:
/// the parser stops at the first layer's end, and the leftover comma
/// (plus any further layers) fails the caller's `expect_exhausted`
/// check, dropping the declaration entirely.
#[test]
fn background_shorthand_rejects_comma_separated_multi_layer() {
    assert_eq!(
        parse_entire("url(a.png) top, url(b.png) bottom", "background"),
        None
    );
}

/// `||` semantics: each component at most once. A 2nd `<repeat-style>`
/// token has nowhere to go (the `repeat` slot is already filled by the
/// first `repeat`, and `repeat-x` matches no other slot) — leftover,
/// declaration dropped.
#[test]
fn background_shorthand_rejects_repeated_repeat_style_component() {
    assert_eq!(parse_entire("repeat repeat-x", "background"), None);
}

/// `<visual-box>` may occur at most **twice** (spec §2.10 verbatim, "If
/// two values are present…" — never 3). A 3rd occurrence is leftover.
#[test]
fn background_shorthand_rejects_a_third_visual_box_occurrence() {
    assert_eq!(
        parse_entire("border-box padding-box content-box", "background"),
        None
    );
}

/// A `/` component followed by an unparseable `<bg-size>` does not
/// silently drop just the size — `parse_background_position_and_size`'s
/// inner `try_parse` rewinds the whole `/ <bg-size>` attempt (position
/// succeeds alone, size stays absent), leaving `/ bogus` as leftover
/// that matches no other `||` component, so the whole declaration is
/// dropped rather than falling back to the shorthand's `auto auto` size
/// fill.
#[test]
fn background_shorthand_rejects_slash_with_invalid_size() {
    assert_eq!(parse_entire("center / bogus", "background"), None);
}

#[test]
fn background_shorthand_rejects_size_without_a_preceding_position() {
    // The `<bg-position> [ / <bg-size> ]?` slot is atomic — `<bg-size>`
    // is a "then"-clause of a leading `<bg-position>`, never a
    // standalone `||` component on its own (`parse_background_shorthand`
    // doc's grammar section). `/ 100% auto` has no position to attach
    // to, so the leading `/` matches no slot at all and is dropped as
    // leftover, same as `center / bogus` above but exercising the
    // "no position present" edge rather than "position present, size
    // invalid".
    assert_eq!(parse_entire("/ 100% auto", "background"), None);
}

#[test]
fn background_shorthand_key_maps_to_background_property_key() {
    let v = PropertyValue::Background(BackgroundShorthand {
        color: CssColor::TRANSPARENT,
        image: BackgroundImage::None,
        repeat: BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::Repeat,
        },
        attachment: BackgroundAttachment::Scroll,
        position: CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        },
        size: BackgroundSize::Explicit {
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
        },
        clip: VisualBox::BorderBox,
        origin: VisualBox::PaddingBox,
    });
    assert_eq!(v.key(), PropertyKey::Background);
}

// ── font shorthand (CSS Fonts 4 §2.1) ──

fn expect_font(value: Option<PropertyValue>) -> FontShorthand {
    match value {
        Some(PropertyValue::Font(shorthand)) => shorthand,
        // cov:ignore: this branch only executes when a caller's
        // `parse(..., "font")` unexpectedly fails to parse or parses to
        // the wrong variant; every call site in this test module passes
        // valid `font` shorthand input, so the panic never fires while
        // the tests pass.
        other => panic!("expected PropertyValue::Font, got {other:?}"),
    }
}

#[test]
fn font_shorthand_minimal_size_and_family_fill_the_rest_with_initial_values() {
    assert_eq!(
        expect_font(parse_entire("16px serif", "font")),
        FontShorthand {
            style: FontStyle::Normal,
            variant: FontVariantCaps::Normal,
            weight: FontWeightValue::Absolute(400.0),
            size: FontShorthandSize::Absolute(Length::Px(16.0)),
            line_height: LineHeight::Normal,
            family: Arc::new(vec![Atom::from("serif")]),
        }
    );
}

#[test]
fn font_shorthand_full_preface_with_slash_line_height_and_family_list() {
    assert_eq!(
        expect_font(parse_entire(
            "italic small-caps bold 16px/1.5 \"Times New Roman\", serif",
            "font"
        )),
        FontShorthand {
            style: FontStyle::Italic,
            variant: FontVariantCaps::SmallCaps,
            weight: FontWeightValue::Absolute(700.0),
            size: FontShorthandSize::Absolute(Length::Px(16.0)),
            line_height: LineHeight::Number(1.5),
            family: Arc::new(vec![Atom::from("Times New Roman"), Atom::from("serif")]),
        }
    );
}

#[test]
fn font_shorthand_preface_accepts_any_order() {
    let forward = expect_font(parse_entire("italic bold 16px serif", "font"));
    let backward = expect_font(parse_entire("bold italic 16px serif", "font"));
    assert_eq!(forward, backward);
    assert_eq!(forward.style, FontStyle::Italic);
    assert_eq!(forward.weight, FontWeightValue::Absolute(700.0));
}

#[test]
fn font_shorthand_numeric_weight_and_absolute_unit_size() {
    let shorthand = expect_font(parse_entire("600 14pt Georgia", "font"));
    assert_eq!(shorthand.weight, FontWeightValue::Absolute(600.0));
    assert_eq!(
        shorthand.size,
        FontShorthandSize::Absolute(Length::Pt(14.0))
    );
}

#[test]
fn font_shorthand_relative_size_keywords() {
    assert_eq!(
        expect_font(parse_entire("larger serif", "font")).size,
        FontShorthandSize::Relative(RelativeFontSize::Larger)
    );
    assert_eq!(
        expect_font(parse_entire("italic smaller serif", "font")).size,
        FontShorthandSize::Relative(RelativeFontSize::Smaller)
    );
}

#[test]
fn font_shorthand_absolute_size_keyword() {
    assert_eq!(
        expect_font(parse_entire("x-large serif", "font")).size,
        FontShorthandSize::Absolute(Length::Px(24.0))
    );
}

#[test]
fn font_shorthand_slash_line_height_length_and_percentage() {
    assert_eq!(
        expect_font(parse_entire("16px/24px serif", "font")).line_height,
        LineHeight::Length(Length::Px(24.0))
    );
    assert_eq!(
        expect_font(parse_entire("16px/150% serif", "font")).line_height,
        LineHeight::Length(Length::Percent(150.0))
    );
}

#[test]
fn font_shorthand_triple_normal_fills_three_preface_slots() {
    let shorthand = expect_font(parse_entire("normal normal normal 16px serif", "font"));
    assert_eq!(shorthand.style, FontStyle::Normal);
    assert_eq!(shorthand.variant, FontVariantCaps::Normal);
    assert_eq!(shorthand.weight, FontWeightValue::Absolute(400.0));
}

#[test]
fn font_shorthand_stretch_normal_consumed_after_full_preface() {
    // `normal` after style + variant + weight can only be the
    // `font-stretch` slot (`FontShorthand` doc's Scope carving section) —
    // consumed and dropped, longhands keep their fills.
    let shorthand = expect_font(parse_entire(
        "italic small-caps bold normal 16px serif",
        "font",
    ));
    assert_eq!(shorthand.style, FontStyle::Italic);
    assert_eq!(shorthand.variant, FontVariantCaps::SmallCaps);
    assert_eq!(shorthand.weight, FontWeightValue::Absolute(700.0));
}

#[test]
fn font_shorthand_oblique_style() {
    assert_eq!(
        expect_font(parse_entire("oblique 16px serif", "font")).style,
        FontStyle::Oblique
    );
}

#[test]
fn font_shorthand_rejects_system_font_keywords() {
    for keyword in [
        "caption",
        "icon",
        "menu",
        "message-box",
        "small-caption",
        "status-bar",
    ] {
        assert_eq!(parse_entire(keyword, "font"), None, "{keyword}");
    }
}

#[test]
fn font_shorthand_rejects_missing_size_or_family() {
    // preface only, no size/family
    assert_eq!(parse_entire("italic", "font"), None);
    // size without family
    assert_eq!(parse_entire("italic 16px", "font"), None);
    assert_eq!(parse_entire("16px", "font"), None);
    // family without size
    assert_eq!(parse_entire("serif", "font"), None);
    assert_eq!(parse_entire("italic serif", "font"), None);
    // empty
    assert_eq!(parse_entire("", "font"), None);
}

#[test]
fn font_shorthand_rejects_non_normal_font_stretch() {
    // `condensed` matches no preface slot, so the `font-size` parse runs
    // on it and fails — the whole declaration is dropped rather than
    // silently ignoring the stretch (`FontShorthand` doc's Scope
    // carving section).
    assert_eq!(parse_entire("condensed 16px serif", "font"), None);
}

#[test]
fn font_shorthand_rejects_non_css2_font_variant() {
    // `unicase` is a valid `font-variant-caps` keyword but not part of
    // CSS2's `font-variant-css2` (`normal` / `small-caps`) subset the
    // shorthand accepts — same drop shape as the stretch case above.
    assert_eq!(parse_entire("italic unicase 16px serif", "font"), None);
}

#[test]
fn font_shorthand_rejects_repeated_preface_component() {
    // `||` semantics: each preface component at most once. The 2nd
    // `italic` matches no remaining slot, so the `font-size` parse runs
    // on it and fails.
    assert_eq!(parse_entire("italic italic 16px serif", "font"), None);
}

#[test]
fn font_shorthand_rejects_slash_with_missing_or_invalid_line_height() {
    assert_eq!(parse_entire("16px/ serif", "font"), None);
    assert_eq!(parse_entire("16px/bogus serif", "font"), None);
}

#[test]
fn font_shorthand_rejects_unknown_keyword() {
    assert_eq!(parse_entire("bogus 16px serif", "font"), None);
}

#[test]
fn font_shorthand_key_maps_to_font_property_key() {
    let v = PropertyValue::Font(FontShorthand {
        style: FontStyle::Normal,
        variant: FontVariantCaps::Normal,
        weight: FontWeightValue::Absolute(400.0),
        size: FontShorthandSize::Absolute(Length::Px(16.0)),
        line_height: LineHeight::Normal,
        family: Arc::new(vec![Atom::from("serif")]),
    });
    assert_eq!(v.key(), PropertyKey::Font);
}

// ── object-fit (CSS Images Module Level 3 §5.1) ──

#[test]
fn object_fit_parse_all_five_keywords() {
    assert_eq!(
        parse("fill", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::Fill))
    );
    assert_eq!(
        parse("contain", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::Contain))
    );
    assert_eq!(
        parse("cover", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::Cover))
    );
    assert_eq!(
        parse("none", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::None))
    );
    assert_eq!(
        parse("scale-down", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::ScaleDown))
    );
}

#[test]
fn object_fit_is_case_insensitive() {
    assert_eq!(
        parse("SCALE-DOWN", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::ScaleDown))
    );
}

#[test]
fn object_fit_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "object-fit"), None, "{keyword}");
    }
}

#[test]
fn object_fit_rejects_unknown_keyword() {
    assert_eq!(parse("stretch", "object-fit"), None);
    assert_eq!(parse("16px", "object-fit"), None);
}

#[test]
fn object_fit_key_maps_to_object_fit_property_key() {
    let v = PropertyValue::ObjectFit(ObjectFit::Fill);
    assert_eq!(v.key(), PropertyKey::ObjectFit);
}

// ── object-position (CSS Images Module Level 3 §5.2) ──
//
// `object-position` uses `parse_position_strict`, not
// `parse_bg_position` (`CssPosition` doc's Grammar section) — its Value
// is plain `<position>` (CSS Values 4 §8.3), not `<bg-position>`
// (`<bg-position>` is `background-position`'s own extension, CSS
// Backgrounds 3 §2.6). The `background_position_*` tests above check
// `<bg-position>` coverage (they exercise `parse_bg_position`, which
// *does* accept the 3-value edge-offset form `<bg-position>` adds on top
// of `<position>`) — that is NOT full `<position>` coverage, since the
// 3-value form is exactly what plain `<position>` disallows. These tests
// confirm both: the `object-position` name wires into
// `parse_position_strict` and into its own distinct
// `PropertyValue`/`PropertyKey` (`object_position_parse_reuses_position_grammar`),
// and that the 3-value form specific to `<bg-position>` is rejected
// (`object_position_rejects_bg_position_only_3_value_edge_offset_forms`).

#[test]
fn object_position_parse_reuses_position_grammar() {
    // 1st alternative, bare keyword pair.
    assert_eq!(
        parse("top", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
    // 3rd alternative (`&&`, either order) — `top left` and `left top`
    // both parse.
    let expected = PropertyValue::ObjectPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
        vertical: CssPositionOffset::Start(Length::Percent(0.0)),
    });
    assert_eq!(parse("left top", "object-position"), Some(expected.clone()));
    assert_eq!(parse("top left", "object-position"), Some(expected));
    // 2nd alternative, bare `<length-percentage>` pair.
    assert_eq!(
        parse("10px 20%", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Percent(20.0)),
        }))
    );
    // 4-value edge-offset form — both axes carry an offset, so this is
    // valid for plain `<position>` too (`<position-four>`,
    // `parse_position_branch3_strict` doc's "Why" section), unlike the
    // 3-value forms `object_position_rejects_bg_position_only_3_value_edge_offset_forms`
    // pins as rejected. Both this and the next case start with a
    // vertical edge (`bottom`/`top`), so they exercise the
    // vertical-exclusive-first branch of `parse_position_branch3_strict`;
    // the horizontal-exclusive-first branch (symmetric offset case with
    // `h_offset == v_offset == true` accepted via horizontal-edge
    // priority) is pinned separately below.
    assert_eq!(
        parse("bottom 10px right 20px", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(20.0)),
            vertical: CssPositionOffset::End(Length::Px(10.0)),
        }))
    );
    // Same 4-value form with the other vertical edge — also
    // vertical-exclusive-first, not a distinct branch from the case
    // above (both start with `bottom`/`top`; `&&` allows either order
    // but both orders here are still vertical-first).
    assert_eq!(
        parse("top 10px left 20px", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(20.0)),
            vertical: CssPositionOffset::Start(Length::Px(10.0)),
        }))
    );
    // Horizontal-edge-first 4-value form — exercises the
    // horizontal-exclusive-first branch of `parse_position_branch3_strict`
    // (the symmetric `h_offset == v_offset == true` accept path where
    // horizontal edge wins). Distinct from the two vertical-first cases
    // above.
    assert_eq!(
        parse("right 20px bottom 10px", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(20.0)),
            vertical: CssPositionOffset::End(Length::Px(10.0)),
        }))
    );
    // Ambiguous `center` leading, paired with a bare (offset-less)
    // vertical keyword — exercises `parse_position_branch3_strict`'s
    // ambiguous-`center` branch success arm (`center` and the other
    // axis both carry no offset, so it's symmetric and accepted).
    assert_eq!(
        parse("center bottom", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(100.0)),
        }))
    );
    // Ambiguous `center` leading, paired with a bare (offset-less)
    // horizontal keyword — `parse_position_vertical_edge` fails on
    // `left` (not `top`/`bottom`), falling to the horizontal-edge-
    // after-center check; symmetric (both sides offset-less) so
    // accepted. Distinct code path from `center bottom` above (which
    // never reaches the horizontal-edge-after-center check at all) and
    // from `center right 10px`
    // (`object_position_rejects_bg_position_only_3_value_edge_offset_forms`,
    // which takes the same path but with an offset present).
    assert_eq!(
        parse("center left", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
}

/// `<bg-position>` (CSS Backgrounds 3 §2.6) extends plain `<position>`
/// (CSS Values 4 §8.3) with a 3-value edge-offset form — offset
/// authored on exactly one of the two edge-keyword groups, the other
/// axis a bare keyword/`center`. That spec explicitly calls this out:
/// "For 3-value productions (which are not valid in `<position>`)".
/// `object-position`'s Value is `<position>`, not `<bg-position>`
/// (CSS Images 3 §5.2), so this asymmetric form must be rejected —
/// `parse_position_branch3_strict` doc's "Why" section. All 5 inputs
/// here have offset on exactly one axis; contrast with
/// `object_position_parse_reuses_position_grammar`'s
/// `bottom 10px right 20px` (offset on both axes, still valid).
///
/// Uses `parse_entire`, not the bare `parse` helper: once
/// `parse_position_branch3_strict` rejects the asymmetric 3rd
/// alternative, `parse_position_strict`'s fallback to the 2nd/1st
/// alternative (shared with `parse_bg_position`, `CssPosition` doc's
/// Grammar section) greedily matches a *prefix* of these 3-token inputs
/// (e.g. `right 10px center` → 2nd alternative consumes `right 10px`,
/// leaving `center` over) — same "prefix match, caller enforces full
/// consumption" contract every multi-token value in this crate relies on
/// (`rule::DeclParser`'s `expect_exhausted`, mirrored here by
/// `parse_entirely`). The bare `parse` helper does not enforce that, so
/// it would see the prefix's `Some(..)` and miss the leftover.
#[test]
fn object_position_rejects_bg_position_only_3_value_edge_offset_forms() {
    for source in [
        "right 10px center",
        "right 10px top",
        "bottom 10px right",
        "left 5% center",
        "center bottom 10px",
        // Ambiguous `center` leading, followed by an offset-bearing
        // horizontal edge (`parse_position_vertical_edge` fails on
        // `right`/`left`, falling to the horizontal-edge-after-center
        // check) — same asymmetric shape, distinct code path from
        // `center bottom 10px` above (which hits the vertical-edge-
        // after-center check instead).
        "center right 10px",
    ] {
        assert_eq!(parse_entire(source, "object-position"), None, "{source}");
    }
}

#[test]
fn object_position_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "object-position"), None, "{keyword}");
    }
}

#[test]
fn object_position_key_maps_to_object_position_property_key() {
    let v = PropertyValue::ObjectPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
        vertical: CssPositionOffset::Start(Length::Percent(50.0)),
    });
    assert_eq!(v.key(), PropertyKey::ObjectPosition);
}

// ── opacity (CSS Color 4 §3.3) ──────────────────────────────────────

#[test]
fn opacity_parse_number() {
    assert_eq!(parse("0.5", "opacity"), Some(PropertyValue::Opacity(0.5)));
    assert_eq!(parse("1", "opacity"), Some(PropertyValue::Opacity(1.0)));
    assert_eq!(parse("0", "opacity"), Some(PropertyValue::Opacity(0.0)));
}

#[test]
fn opacity_parse_percentage() {
    assert_eq!(parse("50%", "opacity"), Some(PropertyValue::Opacity(0.5)));
    assert_eq!(parse("100%", "opacity"), Some(PropertyValue::Opacity(1.0)));
    assert_eq!(parse("0%", "opacity"), Some(PropertyValue::Opacity(0.0)));
}

#[test]
fn opacity_out_of_range_values_are_preserved_unclamped_at_parse_time() {
    // CSS Color 4 §3.3: "Opacity values outside the range `[0, 1]` are not
    // invalid, and are preserved in specified values, but are clamped to
    // the range `[0, 1]` in computed values." — the parser (specified
    // layer) must NOT clamp; that is `SpecifiedValues::absolutize_with`'s
    // job (phase 3), pinned separately in `specified.rs`/`cascade.rs`.
    assert_eq!(parse("2", "opacity"), Some(PropertyValue::Opacity(2.0)));
    assert_eq!(parse("-0.5", "opacity"), Some(PropertyValue::Opacity(-0.5)));
    assert_eq!(parse("150%", "opacity"), Some(PropertyValue::Opacity(1.5)));
}

#[test]
fn opacity_zero_mantissa_huge_exponent_resolves_to_zero_but_preserves_infinity() {
    // `0 * 10^999` collapses to `0.0 * f32::INFINITY` = NaN internally
    // during cssparser's tokenizer computation (module doc's
    // "Numeric-token NaN stabilization" section), but
    // `expect_number_stable`/`expect_percentage_stable` (which
    // `parse_opacity_value` acquires its value through) recover the
    // spec-correct `0.0` before `parse_opacity_value`'s `!is_nan()`
    // guard ever runs — so `opacity: 0e999` parses successfully to
    // `PropertyValue::Opacity(0.0)`, not `None` (this is the
    // most-visible regression case the fix targets: without it, this
    // declaration used to be dropped and fall back to the initial
    // `1.0`, making a fully-transparent `0e999` render fully opaque).
    assert_eq!(parse("0e999", "opacity"), Some(PropertyValue::Opacity(0.0)));
    assert_eq!(
        parse("0e999%", "opacity"),
        Some(PropertyValue::Opacity(0.0))
    );

    // `+Inf`/`-Inf` are a *different* hazard class — a spec-valid
    // `<number>` overflow (not a NaN collapse) that the phase-3 clamp
    // handles correctly (`f32::clamp` maps either to `1.0`/`0.0`), so
    // the parser must NOT reject them here — doing so would drop the
    // whole declaration and wrongly fall back to the initial `1.0`
    // instead of clamping to `0.0` for a huge negative literal (this
    // was a regression in an earlier iteration of this guard, which
    // used `is_finite()` and rejected these too).
    assert_eq!(
        parse("1e40", "opacity"),
        Some(PropertyValue::Opacity(f32::INFINITY))
    );
    assert_eq!(
        parse("-1e40", "opacity"),
        Some(PropertyValue::Opacity(f32::NEG_INFINITY))
    );

    // Percentage branch: cssparser 0.37.0 tokenizes `<percentage>` by
    // parsing the numeric part as `f64` then dividing by `100.0` *in
    // `f64`* before casting to `f32`, so `1e40%` becomes `1e38` as
    // `f32` (finite — `1e40 / 100 = 1e38 < f32::MAX ≈ 3.4e38`) and does
    // NOT overflow to `Infinity`. The first exponent that does overflow
    // through this path is `1e41%` (`1e41 / 100 = 1e39 > f32::MAX` →
    // `Infinity`). Pin both signs to cover `!is_nan()` passthrough for
    // the percentage branch specifically (the existing `0e999%` case
    // only exercises the `NaN` rejection path, not this `Infinity`
    // passthrough).
    assert_eq!(
        parse("1e41%", "opacity"),
        Some(PropertyValue::Opacity(f32::INFINITY))
    );
    assert_eq!(
        parse("-1e41%", "opacity"),
        Some(PropertyValue::Opacity(f32::NEG_INFINITY))
    );
}

#[test]
fn opacity_rejects_non_number_percentage_tokens() {
    for source in ["auto", "none", "opaque"] {
        assert_eq!(parse(source, "opacity"), None, "{source}");
    }
}

#[test]
fn opacity_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "opacity"), None, "{keyword}");
    }
}

#[test]
fn opacity_key_maps_to_opacity_property_key() {
    let v = PropertyValue::Opacity(0.5);
    assert_eq!(v.key(), PropertyKey::Opacity);
}

// ── isolation (CSS Compositing and Blending Level 1 §3.4.2) ─────────

#[test]
fn isolation_parse_keywords() {
    assert_eq!(
        parse("auto", "isolation"),
        Some(PropertyValue::Isolation(Isolation::Auto))
    );
    assert_eq!(
        parse("isolate", "isolation"),
        Some(PropertyValue::Isolation(Isolation::Isolate))
    );
}

#[test]
fn isolation_rejects_unknown_keyword() {
    for source in ["none", "isolated"] {
        assert_eq!(parse(source, "isolation"), None, "{source}");
    }
}

#[test]
fn isolation_rejects_trailing_garbage() {
    // `parse_isolation` itself only consumes one ident token — a second
    // keyword is leftover input the property parser doesn't reject on
    // its own (this crate's convention: the declaration-level
    // `expect_exhausted` check, exercised here via `parse_entire`,
    // drops the whole declaration instead — same shape as
    // `mix_blend_mode_rejects_trailing_garbage` below).
    assert_eq!(parse_entire("auto isolate", "isolation"), None);
}

#[test]
fn isolation_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "isolation"), None, "{keyword}");
    }
}

#[test]
fn isolation_key_maps_to_isolation_property_key() {
    let v = PropertyValue::Isolation(Isolation::Isolate);
    assert_eq!(v.key(), PropertyKey::Isolation);
}

// ── mix-blend-mode (CSS Compositing and Blending Level 1 §3.4.1) ────

#[test]
fn mix_blend_mode_parse_all_16_keywords() {
    let cases = [
        ("normal", MixBlendMode::Normal),
        ("multiply", MixBlendMode::Multiply),
        ("screen", MixBlendMode::Screen),
        ("overlay", MixBlendMode::Overlay),
        ("darken", MixBlendMode::Darken),
        ("lighten", MixBlendMode::Lighten),
        ("color-dodge", MixBlendMode::ColorDodge),
        ("color-burn", MixBlendMode::ColorBurn),
        ("hard-light", MixBlendMode::HardLight),
        ("soft-light", MixBlendMode::SoftLight),
        ("difference", MixBlendMode::Difference),
        ("exclusion", MixBlendMode::Exclusion),
        ("hue", MixBlendMode::Hue),
        ("saturation", MixBlendMode::Saturation),
        ("color", MixBlendMode::Color),
        ("luminosity", MixBlendMode::Luminosity),
    ];
    for (source, expected) in cases {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parse(source, "mix-blend-mode"),
            Some(PropertyValue::MixBlendMode(expected)),
            "{source}"
        );
    }
}

#[test]
fn mix_blend_mode_rejects_unknown_keyword() {
    for source in ["hsl", "blend"] {
        assert_eq!(parse(source, "mix-blend-mode"), None, "{source}");
    }
}

#[test]
fn mix_blend_mode_rejects_trailing_garbage() {
    // Same shape as `isolation_rejects_trailing_garbage` — a second
    // keyword is leftover input for the declaration-level
    // `expect_exhausted` check to drop, not something
    // `parse_mix_blend_mode` itself rejects.
    assert_eq!(parse_entire("normal multiply", "mix-blend-mode"), None);
}

#[test]
fn mix_blend_mode_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "mix-blend-mode"), None, "{keyword}");
    }
}

#[test]
fn mix_blend_mode_key_maps_to_mix_blend_mode_property_key() {
    let v = PropertyValue::MixBlendMode(MixBlendMode::Multiply);
    assert_eq!(v.key(), PropertyKey::MixBlendMode);
}

// ── mask-image (CSS Masking Level 1 §7.1) ────────────────────────────

#[test]
fn mask_image_parse_none() {
    assert_eq!(
        parse("none", "mask-image"),
        Some(PropertyValue::MaskImage(MaskImage::None))
    );
}

#[test]
fn mask_image_parse_url_unquoted_form() {
    assert_eq!(
        parse("url(mask.svg#m)", "mask-image"),
        Some(PropertyValue::MaskImage(MaskImage::Url(
            "mask.svg#m".to_string()
        )))
    );
}

#[test]
fn mask_image_parse_url_quoted_form() {
    assert_eq!(
        parse("url(\"mask.svg#m\")", "mask-image"),
        Some(PropertyValue::MaskImage(MaskImage::Url(
            "mask.svg#m".to_string()
        )))
    );
}

#[test]
fn mask_image_parse_gradient_reuses_the_shared_gradient_parser() {
    // grammar-shape reuse check (`parse_mask_image` doc) — the gradient
    // internals themselves are already exhaustively covered by
    // `background-image`'s own gradient tests, so this only confirms
    // the `<gradient>` alternative is reachable through `mask-image`.
    assert!(matches!(
        parse("linear-gradient(red, blue)", "mask-image"),
        Some(PropertyValue::MaskImage(MaskImage::Gradient(
            Gradient::Linear(_)
        )))
    ));
}

#[test]
fn mask_image_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "mask-image"),
        Some(PropertyValue::MaskImage(MaskImage::None))
    );
}

#[test]
fn mask_image_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "mask-image"), None, "{keyword}");
    }
}

#[test]
fn mask_image_key_maps_to_mask_image_property_key() {
    let v = PropertyValue::MaskImage(MaskImage::None);
    assert_eq!(v.key(), PropertyKey::MaskImage);
}

// ── clip-path (CSS Masking Level 1 §5.1) ─────────────────────────────

#[test]
fn clip_path_parse_none() {
    assert_eq!(
        parse("none", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::None))
    );
}

#[test]
fn clip_path_parse_url() {
    assert_eq!(
        parse("url(#my-clip)", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::Url(
            "#my-clip".to_string()
        )))
    );
}

#[test]
fn clip_path_parse_url_quoted_form() {
    assert_eq!(
        parse("url(\"#my-clip\")", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::Url(
            "#my-clip".to_string()
        )))
    );
}

#[test]
fn clip_path_parse_all_7_geometry_box_keywords() {
    let cases = [
        ("border-box", GeometryBox::BorderBox),
        ("padding-box", GeometryBox::PaddingBox),
        ("content-box", GeometryBox::ContentBox),
        ("margin-box", GeometryBox::MarginBox),
        ("fill-box", GeometryBox::FillBox),
        ("stroke-box", GeometryBox::StrokeBox),
        ("view-box", GeometryBox::ViewBox),
    ];
    for (source, expected) in cases {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parse(source, "clip-path"),
            Some(PropertyValue::ClipPath(ClipPath::GeometryBox(expected))),
            "{source}"
        );
    }
}

#[test]
fn clip_path_parses_basic_shapes() {
    // Basic shapes per CSS Shapes Module Level 1 §3.1 — spot check that
    // each of the 5 functions is accepted and lands as ClipPath::BasicShape.
    let cases = [
        "circle(50%)",
        "ellipse(50% 50%)",
        "inset(10px)",
        "polygon(0 0, 100% 0, 100% 100%)",
        "path('M0 0 L10 10')",
    ];
    for source in cases {
        let parsed = parse(source, "clip-path");
        assert!(
            matches!(
                parsed,
                Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
            ),
            "expected BasicShape for {source}, got {parsed:?}"
        );
    }
}

#[test]
fn clip_path_basic_shape_with_geometry_box() {
    // The basic-shape / geometry-box pair — either order.
    let a = parse("circle(50%) border-box", "clip-path");
    assert!(matches!(
        a,
        Some(PropertyValue::ClipPath(ClipPath::BasicShape {
            geometry_box: Some(GeometryBox::BorderBox),
            ..
        }))
    ));
    let b = parse("border-box circle(50%)", "clip-path");
    assert!(matches!(
        b,
        Some(PropertyValue::ClipPath(ClipPath::BasicShape {
            geometry_box: Some(GeometryBox::BorderBox),
            ..
        }))
    ));
    // Also check with padding-box + ellipse at center
    let c = parse("ellipse(50% 50% at center) padding-box", "clip-path");
    assert!(matches!(
        c,
        Some(PropertyValue::ClipPath(ClipPath::BasicShape {
            geometry_box: Some(GeometryBox::PaddingBox),
            ..
        }))
    ));
}

#[test]
fn clip_path_inset_round_and_polygon_fill_rule() {
    assert!(matches!(
        parse("inset(10px 20px round 5px)", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
    ));
    assert!(matches!(
        parse("polygon(evenodd, 0 0, 100% 0, 100% 100%)", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
    ));
    assert!(matches!(
        parse("path(evenodd, 'M0 0 L10 10 Z')", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
    ));
}

#[test]
fn clip_path_circle_ellipse_position_variants() {
    // circle with position, ellipse with at center, plain circle()
    for source in [
        "circle(at center)",
        "circle(closest-side at 10px 20%)",
        "ellipse(closest-side farthest-side at center)",
        "circle()",
        "ellipse()",
    ] {
        let parsed = parse(source, "clip-path");
        assert!(
            matches!(
                parsed,
                Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
            ),
            "{source}"
        );
    }
}

#[test]
fn clip_path_rejects_invalid_basic_shapes() {
    // Negative radius, empty inset, empty polygon should drop.
    for source in [
        "circle(-10px)",
        "inset()",
        "polygon()",
        "path()",
        "circle(50% 50% 50%)",
        "inset(10px unknown)",
    ] {
        assert_eq!(parse(source, "clip-path"), None, "{source}");
    }
}

#[test]
fn clip_path_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::None))
    );
    assert_eq!(
        parse("BORDER-BOX", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::GeometryBox(
            GeometryBox::BorderBox
        )))
    );
}

#[test]
fn clip_path_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "clip-path"), None, "{keyword}");
    }
}

#[test]
fn clip_path_key_maps_to_clip_path_property_key() {
    let v = PropertyValue::ClipPath(ClipPath::None);
    assert_eq!(v.key(), PropertyKey::ClipPath);
}

// ── transform (CSS Transforms Level 1 §4/§9.1) ───────────────────────

fn expect_transform(value: Option<PropertyValue>) -> Vec<TransformFunction> {
    match value {
        Some(PropertyValue::Transform(v)) => (*v).clone(),
        // cov:ignore: this branch only executes when a caller's
        // `parse(...)` unexpectedly fails to produce a Transform value
        // — every call site below passes.
        other => panic!("expected a Transform PropertyValue, got {other:?}"),
    }
}

#[test]
fn transform_parse_none() {
    assert_eq!(
        parse("none", "transform"),
        Some(PropertyValue::Transform(empty_transform_list()))
    );
}

#[test]
fn transform_parse_matrix() {
    assert_eq!(
        expect_transform(parse("matrix(1, 2, 3, 4, 5, 6)", "transform")),
        vec![TransformFunction::Matrix([1.0, 2.0, 3.0, 4.0, 5.0, 6.0])]
    );
}

#[test]
fn transform_matrix_requires_all_6_arguments() {
    for source in [
        "matrix()",
        "matrix(1, 2, 3, 4, 5)",
        "matrix(1, 2, 3, 4, 5, 6, 7)",
    ] {
        assert_eq!(parse(source, "transform"), None, "{source}");
    }
}

#[test]
fn transform_parse_translate_both_axes() {
    assert_eq!(
        expect_transform(parse("translate(10px, 20%)", "transform")),
        vec![TransformFunction::Translate(
            Length::Px(10.0),
            Length::Percent(20.0)
        )]
    );
}

#[test]
fn transform_translate_single_argument_defaults_ty_to_0() {
    // CSS Transforms Level 1 §9.1: the omitted 2nd argument of
    // `translate()` defaults to `0` (`parse_translate_args` doc).
    assert_eq!(
        expect_transform(parse("translate(10px)", "transform")),
        vec![TransformFunction::Translate(
            Length::Px(10.0),
            Length::Px(0.0)
        )]
    );
}

#[test]
fn transform_translate_requires_at_least_1_argument() {
    assert_eq!(parse("translate()", "transform"), None);
}

#[test]
fn transform_parse_translate_x_and_y() {
    assert_eq!(
        expect_transform(parse("translateX(5px)", "transform")),
        vec![TransformFunction::TranslateX(Length::Px(5.0))]
    );
    assert_eq!(
        expect_transform(parse("translateY(50%)", "transform")),
        vec![TransformFunction::TranslateY(Length::Percent(50.0))]
    );
}

#[test]
fn transform_translate_x_and_y_require_exactly_1_argument() {
    for source in ["translateX()", "translateY()"] {
        assert_eq!(parse(source, "transform"), None, "{source}");
    }
}

#[test]
fn transform_parse_scale_both_arguments() {
    assert_eq!(
        expect_transform(parse("scale(2, 3)", "transform")),
        vec![TransformFunction::Scale(2.0, 3.0)]
    );
}

#[test]
fn transform_scale_single_argument_copies_sx_into_sy() {
    // CSS Transforms Level 1 §9.1: the omitted 2nd argument of
    // `scale()` copies the 1st, unlike `translate()`/`skew()`'s
    // "defaults to 0" (`parse_scale_args` doc).
    assert_eq!(
        expect_transform(parse("scale(2)", "transform")),
        vec![TransformFunction::Scale(2.0, 2.0)]
    );
}

#[test]
fn transform_scale_requires_at_least_1_argument() {
    assert_eq!(parse("scale()", "transform"), None);
}

#[test]
fn transform_parse_scale_x_and_y() {
    assert_eq!(
        expect_transform(parse("scaleX(2)", "transform")),
        vec![TransformFunction::ScaleX(2.0)]
    );
    assert_eq!(
        expect_transform(parse("scaleY(3)", "transform")),
        vec![TransformFunction::ScaleY(3.0)]
    );
}

#[test]
fn transform_parse_rotate() {
    assert_eq!(
        expect_transform(parse("rotate(45deg)", "transform")),
        vec![TransformFunction::Rotate(Angle(45.0))]
    );
}

#[test]
fn transform_rotate_accepts_unitless_zero() {
    // CSS Values 4 §7.1's `<zero>` legacy allowance — `rotate(0)`.
    assert_eq!(
        expect_transform(parse("rotate(0)", "transform")),
        vec![TransformFunction::Rotate(Angle(0.0))]
    );
}

#[test]
fn transform_rotate_rejects_unitless_nonzero() {
    assert_eq!(parse("rotate(45)", "transform"), None);
}

#[test]
fn transform_parse_skew_both_arguments() {
    assert_eq!(
        expect_transform(parse("skew(10deg, 20deg)", "transform")),
        vec![TransformFunction::Skew(Angle(10.0), Angle(20.0))]
    );
}

#[test]
fn transform_skew_single_argument_defaults_second_to_0deg() {
    // Same "defaults to 0" shape as `translate()`, unlike `scale()`'s
    // "copies the 1st" (`parse_skew_args` doc).
    assert_eq!(
        expect_transform(parse("skew(30deg)", "transform")),
        vec![TransformFunction::Skew(Angle(30.0), Angle(0.0))]
    );
}

#[test]
fn transform_parse_skew_x_and_y() {
    assert_eq!(
        expect_transform(parse("skewX(10deg)", "transform")),
        vec![TransformFunction::SkewX(Angle(10.0))]
    );
    assert_eq!(
        expect_transform(parse("skewY(20deg)", "transform")),
        vec![TransformFunction::SkewY(Angle(20.0))]
    );
}

#[test]
fn transform_parse_multiple_functions_space_separated() {
    assert_eq!(
        expect_transform(parse("rotate(45deg) scale(2)", "transform")),
        vec![
            TransformFunction::Rotate(Angle(45.0)),
            TransformFunction::Scale(2.0, 2.0),
        ]
    );
}

#[test]
fn transform_rejects_comma_separated_functions() {
    // `<transform-list> = <transform-function>[+]` is whitespace-
    // separated, not comma-separated (`parse_transform` doc) — a
    // comma between functions is leftover input for the
    // declaration-level `expect_exhausted` check to drop.
    assert_eq!(parse_entire("rotate(1deg), scale(2)", "transform"), None);
}

#[test]
fn transform_rejects_3d_functions() {
    // V2 (3D transform, CSS Transforms Level 1 §10) is out of scope
    // (`TransformFunction` doc's Non-goal note) — these function names
    // are simply unrecognized.
    for source in [
        "translate3d(1px, 2px, 3px)",
        "rotate3d(1, 0, 0, 45deg)",
        "matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)",
        "perspective(100px)",
    ] {
        assert_eq!(parse(source, "transform"), None, "{source}");
    }
}

#[test]
fn transform_rejects_unknown_function() {
    assert_eq!(parse("frobnicate(1px)", "transform"), None);
}

#[test]
fn transform_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "transform"), None, "{keyword}");
    }
}

#[test]
fn transform_matrix_zero_mantissa_huge_exponent_argument_resolves_to_zero() {
    // `0e999` collapses to `NaN` internally during cssparser
    // tokenization (module doc's "Numeric-token NaN stabilization"
    // section), but `expect_number_stable` (which `parse_transform_number`
    // acquires its value through) recovers the spec-correct `0.0`
    // before `parse_transform_number`'s `!is_nan()` guard ever runs —
    // unlike ordinary magnitude overflow (`1e40`, preserved as `+Inf`
    // below), this is not a hazard `matrix()` rejects.
    assert_eq!(
        expect_transform(parse("matrix(0e999, 0, 0, 1, 0, 0)", "transform")),
        vec![TransformFunction::Matrix([0.0, 0.0, 0.0, 1.0, 0.0, 0.0])]
    );
}

#[test]
fn transform_matrix_preserves_infinity_argument() {
    // `1e40` overflows to `+Inf` — a spec-valid `<number>` this crate
    // preserves (no range restriction on `matrix()`'s arguments),
    // unlike the NaN case above (`parse_transform_number` doc).
    assert_eq!(
        expect_transform(parse("matrix(1e40, 0, 0, 1, 0, 0)", "transform")),
        vec![TransformFunction::Matrix([
            f32::INFINITY,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0
        ])]
    );
}

#[test]
fn transform_translate_zero_mantissa_huge_exponent_length_resolves_to_zero() {
    // Same recovery as the `matrix()` case above, through
    // `parse_transform_length_percentage` / `parse_length_value` /
    // `next_numeric_stable` — `translate(0e999px)`'s omitted 2nd
    // argument defaults to `Length::Px(0.0)` (`parse_translate_args`
    // doc), same as the (now also `0.0`, not rejected) 1st.
    assert_eq!(
        expect_transform(parse("translate(0e999px)", "transform")),
        vec![TransformFunction::Translate(
            Length::Px(0.0),
            Length::Px(0.0)
        )]
    );
}

#[test]
fn transform_rotate_zero_mantissa_huge_exponent_angle_resolves_to_zero() {
    // Same recovery as above, through `parse_angle_reject_nan` /
    // `parse_angle` / `next_numeric_stable`.
    assert_eq!(
        expect_transform(parse("rotate(0e999deg)", "transform")),
        vec![TransformFunction::Rotate(Angle(0.0))]
    );
}

#[test]
fn transform_key_maps_to_transform_property_key() {
    let v = PropertyValue::Transform(empty_transform_list());
    assert_eq!(v.key(), PropertyKey::Transform);
}

// ── filter (CSS Filter Effects Level 1 §5/§6) ────────────────────────

fn expect_filter(value: Option<PropertyValue>) -> Vec<FilterFunction> {
    match value {
        Some(PropertyValue::Filter(v)) => (*v).clone(),
        // cov:ignore: same reasoning as `expect_transform`'s panic arm.
        other => panic!("expected a Filter PropertyValue, got {other:?}"),
    }
}

#[test]
fn filter_parse_none() {
    assert_eq!(
        parse("none", "filter"),
        Some(PropertyValue::Filter(empty_filter_list()))
    );
}

#[test]
fn filter_parse_blur_with_argument() {
    assert_eq!(
        expect_filter(parse("blur(5px)", "filter")),
        vec![FilterFunction::Blur(Length::Px(5.0))]
    );
}

#[test]
fn filter_blur_omitted_argument_defaults_to_0px() {
    assert_eq!(
        expect_filter(parse("blur()", "filter")),
        vec![FilterFunction::Blur(Length::Px(0.0))]
    );
}

#[test]
fn filter_blur_rejects_negative_length() {
    assert_eq!(parse("blur(-5px)", "filter"), None);
}

#[test]
fn filter_blur_rejects_percentage() {
    // `blur(<length>?)` — no `<percentage>` alternative.
    assert_eq!(parse("blur(50%)", "filter"), None);
}

/// `filter_parse_amount_functions_with_argument`/
/// `filter_amount_functions_preserve_over_100_percent_unclamped` の
/// per-function name/constructor table 用 — clippy `type_complexity`
/// を避けるための alias (意味論的な新型ではない、`RadialShapeSizePositionGroup`
/// と同じ convention)。
type FilterAmountCtorCase = (&'static str, fn(f32) -> FilterFunction);

#[test]
fn filter_parse_amount_functions_with_argument() {
    let cases: [FilterAmountCtorCase; 6] = [
        ("brightness", FilterFunction::Brightness),
        ("contrast", FilterFunction::Contrast),
        ("grayscale", FilterFunction::Grayscale),
        ("invert", FilterFunction::Invert),
        ("saturate", FilterFunction::Saturate),
        ("sepia", FilterFunction::Sepia),
    ];
    for (name, ctor) in cases {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            expect_filter(parse(&format!("{name}(0.5)"), "filter")),
            vec![ctor(0.5)],
            "{name}"
        );
        // `<number-percentage>` — percentage alternative.
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            expect_filter(parse(&format!("{name}(50%)"), "filter")),
            vec![ctor(0.5)],
            "{name}%"
        );
    }
    // `opacity()` is tested separately (`filter_opacity_function_...`)
    // to avoid a same-name clash with the `PropertyValue::Opacity`
    // property in this table's `ctor` type.
}

#[test]
fn filter_opacity_function_parses_and_is_distinct_from_the_opacity_property() {
    assert_eq!(
        expect_filter(parse("opacity(0.5)", "filter")),
        vec![FilterFunction::Opacity(0.5)]
    );
}

#[test]
fn filter_amount_functions_omitted_argument_defaults_to_1() {
    for name in [
        "brightness",
        "contrast",
        "grayscale",
        "invert",
        "opacity",
        "saturate",
        "sepia",
    ] {
        let source = format!("{name}()");
        let functions = expect_filter(parse(&source, "filter"));
        assert_eq!(functions.len(), 1, "{name}");
        let amount = match functions[0] {
            FilterFunction::Brightness(v)
            | FilterFunction::Contrast(v)
            | FilterFunction::Grayscale(v)
            | FilterFunction::Invert(v)
            | FilterFunction::Opacity(v)
            | FilterFunction::Saturate(v)
            | FilterFunction::Sepia(v) => v,
            // cov:ignore: unreachable given the `name` list above only
            // dispatches to the 7 arms this match already covers.
            ref other => panic!("unexpected filter function {other:?}"),
        };
        assert_eq!(amount, 1.0, "{name}");
    }
}

#[test]
fn filter_amount_functions_reject_negative() {
    for name in [
        "brightness",
        "contrast",
        "grayscale",
        "invert",
        "opacity",
        "saturate",
        "sepia",
    ] {
        let source = format!("{name}(-0.5)");
        assert_eq!(parse(&source, "filter"), None, "{source}");
    }
}

#[test]
fn filter_amount_functions_preserve_over_100_percent_unclamped() {
    // CSS Filter Effects Level 1 §6.1: values over 100% are allowed
    // for every one of the 7 amount functions — for
    // `grayscale()`/`invert()`/`opacity()`/`sepia()` specifically, the
    // spec adds "but UAs must clamp the values to 1" as a
    // **rendering-time** obligation, not a specified/computed-value
    // transform (`filter`'s own Computed value is "as specified",
    // `FilterFunction` doc's "Range restriction is reject, not clamp"
    // section) — so this crate preserves the raw value unclamped,
    // deferring the clamp to a future paint-side consumer.
    let cases: [FilterAmountCtorCase; 7] = [
        ("brightness", FilterFunction::Brightness),
        ("contrast", FilterFunction::Contrast),
        ("grayscale", FilterFunction::Grayscale),
        ("invert", FilterFunction::Invert),
        ("opacity", FilterFunction::Opacity),
        ("saturate", FilterFunction::Saturate),
        ("sepia", FilterFunction::Sepia),
    ];
    for (name, ctor) in cases {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            expect_filter(parse(&format!("{name}(2)"), "filter")),
            vec![ctor(2.0)],
            "{name}"
        );
    }
}

#[test]
fn filter_amount_functions_zero_mantissa_huge_exponent_resolves_to_zero() {
    // `0e999` collapses to `NaN` internally (same cssparser tokenizer
    // hazard as `transform`'s numeric arguments), but
    // `expect_number_stable`/`expect_percentage_stable` (which
    // `parse_filter_amount` acquires its value through) recover the
    // spec-correct `0.0` before `parse_filter_amount`'s `v >= 0.0`
    // check ever runs — so it accepts `0.0` normally instead of
    // incidentally rejecting `NaN` (`parse_filter_amount` doc's "No
    // separate `!is_nan()` guard is needed" section).
    assert_eq!(
        expect_filter(parse("brightness(0e999)", "filter")),
        vec![FilterFunction::Brightness(0.0)]
    );
    assert_eq!(
        expect_filter(parse("brightness(0e999%)", "filter")),
        vec![FilterFunction::Brightness(0.0)]
    );
}

#[test]
fn filter_parse_hue_rotate_with_argument() {
    assert_eq!(
        expect_filter(parse("hue-rotate(90deg)", "filter")),
        vec![FilterFunction::HueRotate(Angle(90.0))]
    );
}

#[test]
fn filter_hue_rotate_omitted_argument_defaults_to_0deg() {
    assert_eq!(
        expect_filter(parse("hue-rotate()", "filter")),
        vec![FilterFunction::HueRotate(Angle(0.0))]
    );
}

#[test]
fn filter_hue_rotate_accepts_unitless_zero() {
    assert_eq!(
        expect_filter(parse("hue-rotate(0)", "filter")),
        vec![FilterFunction::HueRotate(Angle(0.0))]
    );
}

#[test]
fn filter_hue_rotate_is_not_normalized_beyond_360deg() {
    // CSS Filter Effects Level 1 §6.1: "Implementations must not
    // normalize this value" — `810deg` stays `810.0`, matching
    // `Angle` doc's own "no normalization" policy.
    assert_eq!(
        expect_filter(parse("hue-rotate(810deg)", "filter")),
        vec![FilterFunction::HueRotate(Angle(810.0))]
    );
}

#[test]
fn filter_hue_rotate_zero_mantissa_huge_exponent_angle_resolves_to_zero() {
    // Same recovery as `transform_rotate_zero_mantissa_huge_exponent_angle_resolves_to_zero`
    // — `hue-rotate()` shares `parse_angle_reject_nan` with `rotate()`.
    assert_eq!(
        expect_filter(parse("hue-rotate(0e999deg)", "filter")),
        vec![FilterFunction::HueRotate(Angle(0.0))]
    );
}

#[test]
fn filter_parse_drop_shadow_full_form() {
    assert_eq!(
        expect_filter(parse("drop-shadow(red 1px 2px 3px)", "filter")),
        vec![FilterFunction::DropShadow(TextShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(3.0),
            color: TextShadowColor::Resolved(RED),
        })]
    );
}

#[test]
fn filter_drop_shadow_omitted_color_defaults_to_currentcolor() {
    assert_eq!(
        expect_filter(parse("drop-shadow(1px 2px)", "filter")),
        vec![FilterFunction::DropShadow(TextShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        })]
    );
}

#[test]
fn filter_drop_shadow_requires_at_least_the_2_offset_lengths() {
    assert_eq!(parse("drop-shadow(red)", "filter"), None);
    assert_eq!(parse("drop-shadow()", "filter"), None);
}

#[test]
fn filter_drop_shadow_third_length_rejects_negative() {
    // Standard deviation (3rd length) is non-negative — "Values are
    // interpreted as for box-shadow" (`FilterFunction::DropShadow`
    // doc), same as box-shadow's own blur-radius restriction.
    assert_eq!(parse("drop-shadow(1px 2px -3px)", "filter"), None);
}

#[test]
fn filter_drop_shadow_zero_mantissa_huge_exponent_offset_resolves_to_zero() {
    // `0e999` collapses to `NaN` internally during tokenization, but
    // `next_numeric_stable` recovers the spec-correct `0.0` before
    // `parse_shadow_length_reject_nan`'s `!is_nan()` guard ever runs —
    // `parse_drop_shadow_args` inherits this through its verbatim
    // reuse of `parse_text_shadow_item`, same shape as
    // `transform_translate_zero_mantissa_huge_exponent_length_resolves_to_zero`.
    assert_eq!(
        expect_filter(parse("drop-shadow(0e999px 2px)", "filter")),
        vec![FilterFunction::DropShadow(TextShadowItem {
            offset_x: Length::Px(0.0),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        })]
    );
}

#[test]
fn filter_drop_shadow_offset_infinity_passes_through_unclamped() {
    // `+Inf`/`-Inf` are a *different* hazard class from `0e999`'s NaN
    // collapse above (ordinary `<number>` magnitude overflow, not a
    // `0 * Infinity` collapse) — legitimate, if extreme, `<length>`
    // values per CSS Values 4 §5, so unlike NaN they must NOT be
    // rejected here (`parse_shadow_length_reject_nan` doc参照).
    assert_eq!(
        expect_filter(parse("drop-shadow(1e40px 2px)", "filter")),
        vec![FilterFunction::DropShadow(TextShadowItem {
            offset_x: Length::Px(f32::INFINITY),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        })]
    );
    assert_eq!(
        expect_filter(parse("drop-shadow(-1e40px 2px)", "filter")),
        vec![FilterFunction::DropShadow(TextShadowItem {
            offset_x: Length::Px(f32::NEG_INFINITY),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        })]
    );
}

#[test]
fn filter_parse_url() {
    assert_eq!(
        expect_filter(parse("url(#my-filter)", "filter")),
        vec![FilterFunction::Url("#my-filter".to_string())]
    );
}

#[test]
fn filter_parse_url_quoted_form() {
    assert_eq!(
        expect_filter(parse("url(\"#my-filter\")", "filter")),
        vec![FilterFunction::Url("#my-filter".to_string())]
    );
}

#[test]
fn filter_parse_mixed_url_and_function_list() {
    assert_eq!(
        expect_filter(parse("url(#f) blur(2px)", "filter")),
        vec![
            FilterFunction::Url("#f".to_string()),
            FilterFunction::Blur(Length::Px(2.0)),
        ]
    );
}

#[test]
fn filter_parse_multiple_functions_space_separated() {
    assert_eq!(
        expect_filter(parse("blur(1px) blur(2px)", "filter")),
        vec![
            FilterFunction::Blur(Length::Px(1.0)),
            FilterFunction::Blur(Length::Px(2.0)),
        ]
    );
}

#[test]
fn filter_rejects_comma_separated_functions() {
    // Same whitespace-only shape as `transform` — see
    // `transform_rejects_comma_separated_functions`.
    assert_eq!(parse_entire("blur(1px), blur(2px)", "filter"), None);
}

#[test]
fn filter_rejects_unknown_function() {
    assert_eq!(parse("frobnicate(1px)", "filter"), None);
}

#[test]
fn filter_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "filter"), None, "{keyword}");
    }
}

#[test]
fn filter_key_maps_to_filter_property_key() {
    let v = PropertyValue::Filter(empty_filter_list());
    assert_eq!(v.key(), PropertyKey::Filter);
}

// ── table-layout (CSS Tables 3 §4) ────────────────────────────────────
//
// WPT css/css-tables/parsing/table-layout-{valid,invalid}.html の
// grammar (`auto | fixed`) を check する。invalid 側 2 case
// (`none` / `auto fixed`) は caller の `expect_exhausted`
// (rule.rs::DeclParser) が落とす — ここでは `parse_entire` で同条件を
// 再現する。

#[test]
fn table_layout_accepts_auto_and_fixed() {
    assert_eq!(
        parse("auto", "table-layout"),
        Some(PropertyValue::TableLayout(TableLayoutValue::Auto))
    );
    assert_eq!(
        parse("fixed", "table-layout"),
        Some(PropertyValue::TableLayout(TableLayoutValue::Fixed))
    );
    // ASCII case-insensitive (CSS Values 3 §3.1)。
    assert_eq!(
        parse("FIXED", "table-layout"),
        Some(PropertyValue::TableLayout(TableLayoutValue::Fixed))
    );
}

#[test]
fn table_layout_rejects_invalid() {
    assert_eq!(parse("none", "table-layout"), None);
    assert_eq!(parse_entire("auto fixed", "table-layout"), None);
    assert_eq!(parse("collapse", "table-layout"), None);
}

#[test]
fn table_layout_key_maps_to_table_layout_property_key() {
    let v = PropertyValue::TableLayout(TableLayoutValue::Auto);
    assert_eq!(v.key(), PropertyKey::TableLayout);
}

// ── border-collapse (CSS Tables 3 §6) ─────────────────────────────────
//
// WPT css/css-tables/parsing/border-collapse-{valid,invalid}.html の
// grammar (`collapse | separate`) を check する (同上の構成)。

#[test]
fn border_collapse_accepts_collapse_and_separate() {
    assert_eq!(
        parse("collapse", "border-collapse"),
        Some(PropertyValue::BorderCollapse(BorderCollapseValue::Collapse))
    );
    assert_eq!(
        parse("separate", "border-collapse"),
        Some(PropertyValue::BorderCollapse(BorderCollapseValue::Separate))
    );
    // ASCII case-insensitive (CSS Values 3 §3.1)。
    assert_eq!(
        parse("COLLAPSE", "border-collapse"),
        Some(PropertyValue::BorderCollapse(BorderCollapseValue::Collapse))
    );
}

#[test]
fn border_collapse_rejects_invalid() {
    assert_eq!(parse("none", "border-collapse"), None);
    assert_eq!(parse_entire("separate collapse", "border-collapse"), None);
    assert_eq!(parse("fixed", "border-collapse"), None);
}

#[test]
fn border_collapse_key_maps_to_border_collapse_property_key() {
    let v = PropertyValue::BorderCollapse(BorderCollapseValue::Separate);
    assert_eq!(v.key(), PropertyKey::BorderCollapse);
}

// ── border-spacing (CSS Tables 3 §6.1) ─────────────────────────────────
//
// WPT css/css-tables/parsing/border-spacing-{valid,invalid}.html の
// 全 case の pin。valid の `calc()` 混じり 2 件は上流 deferred path が
// `Deferred` に回す (`width: calc(..)` と同型) ため、ここでは受理
// (`Some`) のみ assert し payload の中身は assert しない。

#[test]
fn border_spacing_accepts_valid_values() {
    // 単一 standard length は両軸に double する (spec 本文 +
    // `GapShorthand` と同型)。
    assert_eq!(
        parse("0px", "border-spacing"),
        Some(PropertyValue::BorderSpacing(BorderSpacingValue {
            horizontal: Length::Px(0.0),
            vertical: Length::Px(0.0),
        }))
    );
    // 2 成分。
    assert_eq!(
        parse("10px 20px", "border-spacing"),
        Some(PropertyValue::BorderSpacing(BorderSpacingValue {
            horizontal: Length::Px(10.0),
            vertical: Length::Px(20.0),
        }))
    );
    // unitless `0` は `<length>` として受理 (WPT computed の `"0"` case)。
    assert_eq!(
        parse("0", "border-spacing"),
        Some(PropertyValue::BorderSpacing(BorderSpacingValue {
            horizontal: Length::Px(0.0),
            vertical: Length::Px(0.0),
        }))
    );
    // font-relative も plain length として受理。
    assert_eq!(
        parse("0.5em 1px", "border-spacing"),
        Some(PropertyValue::BorderSpacing(BorderSpacingValue {
            horizontal: Length::Em(0.5),
            vertical: Length::Px(1.0),
        }))
    );
    // `calc()` 混じりは deferred path が受理する (payload は `Deferred`)。
    let deferred = parse_entire("calc(10px + 0.5em) calc(10px - 0.5em)", "border-spacing");
    assert!(
        matches!(deferred, Some(PropertyValue::Deferred(_))),
        "calc border-spacing should defer, got {deferred:?}"
    );
    // `calc()` 単一値も同様。
    assert!(matches!(
        parse_entire("calc(10px + 0.5em)", "border-spacing"),
        Some(PropertyValue::Deferred(_))
    ));
    // key mapping。
    let v = PropertyValue::BorderSpacing(BorderSpacingValue {
        horizontal: Length::Px(1.0),
        vertical: Length::Px(2.0),
    });
    assert_eq!(v.key(), PropertyKey::BorderSpacing);
}

#[test]
fn border_spacing_rejects_invalid_values() {
    // `<percentage>` は Percentages: N/A のため reject。
    assert_eq!(parse("10%", "border-spacing"), None);
    // 負 length は illegal のため reject。
    assert_eq!(parse("-20px", "border-spacing"), None);
    assert_eq!(parse_entire("10px -20px", "border-spacing"), None);
    // bare non-zero number は `<length>` ではないため reject。
    assert_eq!(parse("30", "border-spacing"), None);
    // 3 成分は `{1,2}` を満たさないため reject。
    assert_eq!(parse_entire("40px 50px 60px", "border-spacing"), None);
    // keyword は `<length>` ではないため reject。
    assert_eq!(parse("auto", "border-spacing"), None);
    // `%` 混じり calc は deferred path の guard が reject
    // (`tab-size` の Percentages: N/A guard と同型)。
    assert_eq!(parse_entire("calc(10% + 5px)", "border-spacing"), None);
}

// ── caption-side (CSS Tables 3 §7) ─────────────────────────────────────
//
// WPT css/css-tables/parsing/caption-side-{valid,invalid}.html の
// 全 case の pin。

#[test]
fn caption_side_accepts_valid_values() {
    assert_eq!(
        parse("top", "caption-side"),
        Some(PropertyValue::CaptionSide(CaptionSideValue::Top))
    );
    assert_eq!(
        parse("bottom", "caption-side"),
        Some(PropertyValue::CaptionSide(CaptionSideValue::Bottom))
    );
    // ASCII case-insensitive (`table-layout` の `FIXED` case と同型)。
    assert_eq!(
        parse("TOP", "caption-side"),
        Some(PropertyValue::CaptionSide(CaptionSideValue::Top))
    );
    let v = PropertyValue::CaptionSide(CaptionSideValue::Top);
    assert_eq!(v.key(), PropertyKey::CaptionSide);
}

#[test]
fn caption_side_rejects_invalid_values() {
    assert_eq!(parse("auto", "caption-side"), None);
    assert_eq!(parse("left", "caption-side"), None);
    assert_eq!(parse("right", "caption-side"), None);
    assert_eq!(parse_entire("top bottom", "caption-side"), None);
    assert_eq!(parse("10px", "caption-side"), None);
}

// ── empty-cells (CSS Tables 3 §8) ──────────────────────────────────────
//
// WPT css/css-tables/parsing/empty-cells-{valid,invalid}.html の
// 全 case の pin。

#[test]
fn empty_cells_accepts_valid_values() {
    assert_eq!(
        parse("show", "empty-cells"),
        Some(PropertyValue::EmptyCells(EmptyCellsValue::Show))
    );
    assert_eq!(
        parse("hide", "empty-cells"),
        Some(PropertyValue::EmptyCells(EmptyCellsValue::Hide))
    );
    // ASCII case-insensitive (`table-layout` の `FIXED` case と同型)。
    assert_eq!(
        parse("HIDE", "empty-cells"),
        Some(PropertyValue::EmptyCells(EmptyCellsValue::Hide))
    );
    let v = PropertyValue::EmptyCells(EmptyCellsValue::Show);
    assert_eq!(v.key(), PropertyKey::EmptyCells);
}

#[test]
fn empty_cells_rejects_invalid_values() {
    assert_eq!(parse("auto", "empty-cells"), None);
    assert_eq!(parse_entire("show hide", "empty-cells"), None);
    assert_eq!(parse("visible", "empty-cells"), None);
}

#[test]
fn multicol_longhands_parse_and_reject_out_of_range_values() {
    assert_eq!(
        parse_entire("auto", "column-count"),
        Some(PropertyValue::ColumnCount(ColumnCountValue::Auto))
    );
    assert_eq!(
        parse_entire("3", "column-count"),
        Some(PropertyValue::ColumnCount(ColumnCountValue::Count(3)))
    );
    for source in ["0", "-1", "1.5", "3.0"] {
        assert_eq!(parse_entire(source, "column-count"), None, "{source}");
    }

    assert_eq!(
        parse_entire("auto", "column-width"),
        Some(PropertyValue::ColumnWidth(ColumnWidthValue::Auto))
    );
    assert_eq!(
        parse_entire("10px", "column-width"),
        Some(PropertyValue::ColumnWidth(ColumnWidthValue::Length(
            Length::Px(10.0)
        )))
    );
    assert!(matches!(
        parse_entire("2em", "column-width"),
        Some(PropertyValue::ColumnWidth(ColumnWidthValue::Length(Length::Em(value))))
            if (value - 2.0).abs() < f32::EPSILON
    ));
    for source in ["-1px", "10%"] {
        assert_eq!(parse_entire(source, "column-width"), None, "{source}");
    }
}

#[test]
fn multicol_columns_accepts_both_orders_and_rejects_duplicate_non_auto_components() {
    let expected = |count, width| PropertyValue::Columns(ColumnsShorthand { count, width });
    for source in ["3 100px", "100px 3"] {
        assert_eq!(
            parse_entire(source, "columns"),
            Some(expected(
                ColumnCountValue::Count(3),
                ColumnWidthValue::Length(Length::Px(100.0)),
            )),
            "{source}" // cov:ignore: assertion failure formatting is only evaluated on a regression.
        );
    }
    assert_eq!(
        parse_entire("auto 3", "columns"),
        Some(expected(ColumnCountValue::Count(3), ColumnWidthValue::Auto,))
    );
    assert_eq!(
        parse_entire("auto 200px", "columns"),
        Some(expected(
            ColumnCountValue::Auto,
            ColumnWidthValue::Length(Length::Px(200.0)),
        ))
    );
    assert_eq!(
        parse_entire("auto", "columns"),
        Some(expected(ColumnCountValue::Auto, ColumnWidthValue::Auto))
    );
    assert_eq!(
        parse_entire("3", "columns"),
        Some(expected(ColumnCountValue::Count(3), ColumnWidthValue::Auto,))
    );
    for source in ["3 4", "100px 200px", "3 100px 2"] {
        // cov:ignore: assertion failure formatting is only evaluated on a regression.
        assert_eq!(parse_entire(source, "columns"), None, "{source}");
    }
    assert_eq!(parse_entire("100px 3 2", "columns"), None);
    assert_eq!(
        parse_entire("auto auto", "columns"),
        Some(expected(ColumnCountValue::Auto, ColumnWidthValue::Auto))
    );
}

#[test]
fn serialize_length_formats_each_unit() {
    assert_eq!(serialize_length(&Length::Px(10.0)), "10px");
    assert_eq!(serialize_length(&Length::Em(1.5)), "1.5em");
    assert_eq!(serialize_length(&Length::Rem(1.0)), "1rem");
    assert_eq!(serialize_length(&Length::Percent(50.0)), "50%");
    assert_eq!(serialize_length(&Length::Pt(12.0)), "12pt");
    assert_eq!(serialize_length(&Length::Ex(1.0)), "1ex");
    assert_eq!(serialize_length(&Length::Rex(1.0)), "1rex");
    assert_eq!(serialize_length(&Length::Ch(1.0)), "1ch");
    assert_eq!(serialize_length(&Length::Rch(1.0)), "1rch");
    assert_eq!(serialize_length(&Length::Ic(1.0)), "1ic");
    assert_eq!(serialize_length(&Length::Ric(1.0)), "1ric");
    assert_eq!(serialize_length(&Length::Cm(1.0)), "1cm");
    assert_eq!(serialize_length(&Length::Mm(1.0)), "1mm");
    assert_eq!(serialize_length(&Length::Q(1.0)), "1q");
    assert_eq!(serialize_length(&Length::In(1.0)), "1in");
    assert_eq!(serialize_length(&Length::Pc(1.0)), "1pc");
    assert_eq!(serialize_length(&Length::Lh(1.0)), "1lh");
    assert_eq!(serialize_length(&Length::Rlh(1.0)), "1rlh");
}

#[test]
fn serialize_length_formats_negative_and_fractional_values() {
    assert_eq!(serialize_length(&Length::Px(-5.0)), "-5px");
    assert_eq!(serialize_length(&Length::Px(0.0)), "0px");
    assert_eq!(serialize_length(&Length::Percent(33.333332)), "33.3333%");
}

#[test]
fn serialize_length_or_auto_formats_length_and_auto() {
    assert_eq!(
        serialize_length_or_auto(&LengthOrAuto::Length(Length::Px(10.0))),
        "10px"
    );
    assert_eq!(serialize_length_or_auto(&LengthOrAuto::Auto), "auto");
}

#[test]
fn serialize_value_covers_simple_length_variants() {
    assert_eq!(
        serialize_value(&PropertyValue::PaddingTop(Length::Px(10.0))),
        Some("10px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::OutlineWidth(Length::Px(2.0))),
        Some("2px".to_owned())
    );
}

#[test]
fn serialize_value_covers_safe_length_or_auto_variants() {
    assert_eq!(
        serialize_value(&PropertyValue::Top(LengthOrAuto::Auto)),
        Some("auto".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::MarginLeft(LengthOrAuto::Length(
            Length::Percent(10.0)
        ))),
        Some("10%".to_owned())
    );
}

#[test]
fn serialize_value_excludes_width_height_min_max_variants() {
    assert_eq!(
        serialize_value(&PropertyValue::Width(LengthOrAuto::Auto)),
        None
    );
    assert_eq!(
        serialize_value(&PropertyValue::Height(LengthOrAuto::Auto)),
        None
    );
    assert_eq!(
        serialize_value(&PropertyValue::MinWidth(LengthOrAuto::Auto)),
        None
    );
    assert_eq!(
        serialize_value(&PropertyValue::MinHeight(LengthOrAuto::Auto)),
        None
    );
    assert_eq!(
        serialize_value(&PropertyValue::MaxWidth(LengthOrAuto::Auto)),
        None
    );
    assert_eq!(
        serialize_value(&PropertyValue::MaxHeight(LengthOrAuto::Auto)),
        None
    );
}

#[test]
fn serialize_value_collapses_sides_by_css_box_model_rules() {
    assert_eq!(
        serialize_value(&PropertyValue::Padding(Sides::all(Length::Px(10.0)))),
        Some("10px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::Padding(Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(1.0),
            left: Length::Px(2.0),
        })),
        Some("1px 2px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::Padding(Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(3.0),
            left: Length::Px(2.0),
        })),
        Some("1px 2px 3px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::Padding(Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(3.0),
            left: Length::Px(4.0),
        })),
        Some("1px 2px 3px 4px".to_owned())
    );
}

#[test]
fn serialize_value_collapses_start_end_pairs() {
    assert_eq!(
        serialize_value(&PropertyValue::PaddingInline(StartEnd {
            start: Length::Px(5.0),
            end: Length::Px(5.0),
        })),
        Some("5px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::PaddingInline(StartEnd {
            start: Length::Px(5.0),
            end: Length::Px(10.0),
        })),
        Some("5px 10px".to_owned())
    );
    assert_eq!(
        serialize_value(&PropertyValue::MarginBlock(StartEnd {
            start: LengthOrAuto::Auto,
            end: LengthOrAuto::Length(Length::Px(10.0)),
        })),
        Some("auto 10px".to_owned())
    );
}

#[test]
fn serialize_alpha_channel_round_trips_every_u8_through_channel_to_u8() {
    for alpha in 0u8..=255 {
        let serialized = serialize_alpha_channel(alpha);
        let reparsed: f32 = serialized
            .parse()
            .unwrap_or_else(|_| panic!("{serialized:?} should parse as a number"));
        assert_eq!(
            channel_to_u8(reparsed),
            alpha,
            "alpha {alpha} serialized as {serialized:?}, which does not round-trip"
        );
    }
}

#[test]
fn serialize_number_formats_plain_decimals_without_a_leading_dot() {
    assert_eq!(serialize_number(0.5), "0.5");
    assert_eq!(serialize_number(1.0), "1");
    assert_eq!(serialize_number(0.0), "0");
}

#[test]
fn serialize_css_color_formats_opaque_and_translucent() {
    assert_eq!(
        serialize_css_color(&CssColor {
            r: 34,
            g: 51,
            b: 68,
            a: 255
        }),
        "rgb(34, 51, 68)"
    );
    assert_eq!(
        serialize_css_color(&CssColor {
            r: 2,
            g: 3,
            b: 4,
            a: 128
        }),
        "rgba(2, 3, 4, 0.5)"
    );
    assert_eq!(
        serialize_css_color(&CssColor::TRANSPARENT),
        "rgba(0, 0, 0, 0)"
    );
}

#[test]
fn serialize_color_value_echoes_keyword_syntax_lowercased() {
    assert_eq!(
        serialize_color_value("color", "currentColor"),
        Some("currentcolor".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "transparent"),
        Some("transparent".to_owned())
    );
    assert_eq!(
        serialize_color_value("background-color", "teal"),
        Some("teal".to_owned())
    );
    assert_eq!(
        serialize_color_value("border-top-color", "red"),
        Some("red".to_owned())
    );
}

#[test]
fn serialize_color_value_canonicalizes_legacy_functional_syntax() {
    assert_eq!(
        serialize_color_value("color", "#234"),
        Some("rgb(34, 51, 68)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "rgb(100%, 0%, 0%)"),
        Some("rgb(255, 0, 0)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "hsl(120, 100%, 50%)"),
        Some("rgb(0, 255, 0)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "hsla(120, 100%, 50%, 0.25)"),
        Some("rgba(0, 255, 0, 0.25)".to_owned())
    );
    assert_eq!(
        serialize_color_value("text-decoration-color", "rgba(2, 3, 4, 50%)"),
        Some("rgba(2, 3, 4, 0.5)".to_owned())
    );
}

#[test]
fn serialize_color_value_returns_none_for_modern_functional_syntax() {
    // lab()/lch()/oklab()/oklch() are covered by
    // serialize_color_value_handles_lab_plain_values and friends —
    // color()/color-mix()/color-layers()/light-dark()/contrast-color()
    // remain out of scope (Phase2b's non-goals, see the design spec).
    assert_eq!(
        serialize_color_value("background-color", "color(srgb 1 0 0)"),
        None
    );
    assert_eq!(
        serialize_color_value("color", "color-mix(in srgb, red, blue)"),
        None
    );
}

#[test]
fn serialize_color_value_returns_none_for_relative_color_syntax_under_a_legacy_function_name() {
    // A `from` clause can appear inside `rgb()`/`hsl()`/etc. too (CSS
    // Color 4's relative color syntax) — its resolved value must
    // serialize back through the origin's own notation, not through
    // `rgb()`/`rgba()`, so this must stay unserialized just like the
    // other modern-syntax cases above.
    assert_eq!(
        serialize_color_value("background-color", "rgb(from contrast-color(blue) r g b)"),
        None
    );
    assert_eq!(
        serialize_color_value("color", "rgb(from alpha(from currentcolor / 0.5) r g b)"),
        None
    );
}

#[test]
fn serialize_color_value_returns_none_for_names_it_does_not_own() {
    assert_eq!(serialize_color_value("width", "10px"), None);
}

#[test]
fn serialize_value_defers_border_color_shorthand() {
    assert_eq!(
        serialize_value(&PropertyValue::BorderColor(Sides {
            top: BorderColor::CurrentColor,
            right: BorderColor::CurrentColor,
            bottom: BorderColor::CurrentColor,
            left: BorderColor::CurrentColor,
        })),
        None
    );
}

#[test]
fn serialize_color_value_collapses_border_color_shorthand_by_box_model_rules() {
    assert_eq!(
        serialize_color_value("border-color", "currentcolor"),
        Some("currentcolor".to_owned())
    );
    assert_eq!(
        serialize_color_value("border-color", "currentColor"),
        Some("currentcolor".to_owned())
    );
    assert_eq!(
        serialize_color_value("border-color", "red yellow green blue"),
        Some("red yellow green blue".to_owned())
    );
    assert_eq!(
        serialize_color_value("border-color", "red green"),
        Some("red green".to_owned())
    );
    // 3 authored components (top, right, bottom) with left implied
    // equal to right: the resolved sides are red/green/red/green,
    // which collapses further to the 2-value form since top == bottom
    // and right == left too.
    assert_eq!(
        serialize_color_value("border-color", "red green red"),
        Some("red green".to_owned())
    );
    // 3 authored components that do NOT collapse further stay 3-value.
    assert_eq!(
        serialize_color_value("border-color", "red green blue"),
        Some("red green blue".to_owned())
    );
}

#[test]
fn serialize_color_value_splits_border_color_components_paren_aware() {
    // A naive whitespace split would misread `rgb(0 0 255)`'s internal
    // spaces as component boundaries.
    assert_eq!(
        serialize_color_value("border-color", "rgb(0 0 255) red"),
        Some("rgb(0, 0, 255) red".to_owned())
    );
}

#[test]
fn serialize_color_value_returns_none_for_border_color_with_a_modern_component() {
    // lab() is now supported (Phase2b) — use a function still out of
    // scope (color-mix()) so this keeps testing "an unsupported modern
    // component anywhere in the shorthand forces None for the whole
    // value."
    assert_eq!(
        serialize_color_value("border-color", "red color-mix(in srgb, red, blue)"),
        None
    );
}

#[test]
fn serialize_color_value_handles_lab_plain_values() {
    assert_eq!(
        serialize_color_value("color", "lab(0 0 0)"),
        Some("lab(0 0 0)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lab(-40 0 0)"),
        Some("lab(0 0 0)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lab(400 0 10/50%)"),
        Some("lab(100 0 10 / 0.5)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lab(50% 50% -20%)"),
        Some("lab(50 62.5 -25)".to_owned())
    );
}

#[test]
fn serialize_color_value_handles_oklab_scale_factors() {
    assert_eq!(
        serialize_color_value("color", "oklab(50% 50% -20%)"),
        Some("oklab(0.5 0.2 -0.08)".to_owned())
    );
}

#[test]
fn serialize_color_value_handles_lch_chroma_and_hue() {
    assert_eq!(
        serialize_color_value("color", "lch(20 -20 0)"),
        Some("lch(20 0 0)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lch(10 20 380deg)"),
        Some("lch(10 20 20)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lch(10 20 -700deg)"),
        Some("lch(10 20 20)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lch(10 20 1.28rad)"),
        Some("lch(10 20 73.3386)".to_owned())
    );
    // A unitless hue number still needs [0, 360) normalization — the
    // grammar treats it as degrees even without the `deg` suffix.
    assert_eq!(
        serialize_color_value("color", "lch(10 20 -700)"),
        Some("lch(10 20 20)".to_owned())
    );
    assert_eq!(
        serialize_color_value("color", "lch(0.5 -20% -20)"),
        Some("lch(0.5 0 340)".to_owned())
    );
}

#[test]
fn serialize_color_value_keeps_calc_folded_hue_unnormalized_with_its_deg_unit() {
    // Unlike a bare hue, a calc()-authored one is neither scaled nor
    // range-normalized — it folds to a plain evaluated number, keeping
    // `deg` only because an actual Angle operand (`20deg`) was present.
    assert_eq!(
        serialize_color_value(
            "color",
            "lch(calc(-50 * 3) calc(0.5 + 1) calc(-20deg * 2) / calc(-0.5 * 2))"
        ),
        Some("lch(calc(-150) calc(1.5) calc(-40deg) / calc(-1))".to_owned())
    );
    // A calc() hue with no Angle operand anywhere keeps no unit at all.
    assert_eq!(
        serialize_color_value("color", "lch(none 20 calc(0.5))"),
        Some("lch(none 20 calc(0.5))".to_owned())
    );
}

#[test]
fn serialize_color_value_handles_oklch_scale_factors() {
    assert_eq!(
        serialize_color_value("color", "oklch(20% 60% 10/0.5)"),
        Some("oklch(0.2 0.24 10 / 0.5)".to_owned())
    );
}

#[test]
fn serialize_color_value_preserves_none_in_lab_family() {
    assert_eq!(
        serialize_color_value("color", "lab(none 20 calc(0.5))"),
        Some("lab(none 20 calc(0.5))".to_owned())
    );
}

#[test]
fn serialize_color_value_folds_and_reorders_calc_in_lab_family() {
    assert_eq!(
        serialize_color_value(
            "color",
            "lab(calc(50 * 3) calc(0.5 - 1) calc(1.5) / calc(-0.5 + 1))"
        ),
        Some("lab(calc(150) calc(-0.5) calc(1.5) / calc(0.5))".to_owned())
    );
    assert_eq!(
        serialize_color_value(
            "color",
            "lab(calc(50 + (sign(1em - 10px) * 10)) 30 50 / 50%)"
        ),
        Some("lab(calc(50 + (10 * sign(1em - 10px))) 30 50 / 0.5)".to_owned())
    );
}

#[test]
fn serialize_color_value_returns_none_for_relative_lab_syntax() {
    assert_eq!(
        serialize_color_value("color", "lab(from red 50 20 10)"),
        None
    );
}
