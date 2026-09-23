//! Tests for the color parsers in `parse/color.rs`.

use super::*;

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
