use super::*;

const GRAY: CssColor = CssColor {
    r: 128,
    g: 128,
    b: 128,
    a: 255,
};

fn rgb(r: u8, g: u8, b: u8) -> CssColor {
    CssColor { r, g, b, a: 255 }
}

#[test]
fn a_bevel_shades_a_color_the_way_chromium_does() {
    // (color, dark side, light side), as Chromium draws `inset` and `outset`.
    let table = [
        (rgb(128, 128, 128), rgb(44, 44, 44), rgb(212, 212, 212)),
        (rgb(255, 255, 255), rgb(171, 171, 171), rgb(255, 255, 255)),
        (rgb(255, 0, 0), rgb(171, 0, 0), rgb(255, 0, 0)),
        (rgb(0, 128, 0), rgb(0, 44, 0), rgb(0, 212, 0)),
        (rgb(10, 20, 30), rgb(38, 76, 114), rgb(66, 132, 198)),
        // Black cannot get darker, so its dark side is lightened.
        (rgb(0, 0, 0), rgb(84, 84, 84), rgb(168, 168, 168)),
        // A gray at the dark limit is still too dark to darken...
        (rgb(32, 32, 32), rgb(116, 116, 116), rgb(200, 200, 200)),
        // ...and one just above it darkens to black.
        (rgb(33, 33, 33), rgb(0, 0, 0), rgb(117, 117, 117)),
        // A gray at the light limit still lightens to white, and one above it
        // keeps its light side.
        (rgb(235, 235, 235), rgb(151, 151, 151), rgb(255, 255, 255)),
        (rgb(240, 240, 240), rgb(156, 156, 156), rgb(240, 240, 240)),
    ];
    for (input, dark, light) in table {
        assert_eq!(shade(input, true), dark, "dark side of {input:?}");
        assert_eq!(shade(input, false), light, "light side of {input:?}");
    }
}

#[test]
fn a_shade_keeps_the_alpha_of_its_color() {
    let translucent = CssColor { a: 99, ..GRAY };
    assert_eq!(shade(translucent, true).a, 99);
    assert_eq!(shade(translucent, false).a, 99);
    let black = CssColor {
        a: 7,
        ..rgb(0, 0, 0)
    };
    assert_eq!(shade(black, true).a, 7);
}

#[test]
fn the_nearer_half_of_an_odd_width_takes_the_extra_pixel() {
    assert_eq!(nearer_half(1.0), 1.0);
    assert_eq!(nearer_half(2.0), 1.0);
    assert_eq!(nearer_half(7.0), 4.0);
    assert_eq!(nearer_half(12.0), 6.0);
}

#[test]
fn a_third_rounds_as_a_width_plus_one_over_three_down() {
    // (width, line, gap) of a double border, as Chromium divides it.
    for (width, line, gap) in [
        (3.0, 1.0, 1.0),
        (4.0, 1.0, 2.0),
        (5.0, 2.0, 1.0),
        (6.0, 2.0, 2.0),
        (7.0, 2.0, 3.0),
        (8.0, 3.0, 2.0),
        (9.0, 3.0, 3.0),
    ] {
        assert_eq!(third(width), line, "line of a {width}px double");
        assert_eq!(width - 2.0 * third(width), gap, "gap of a {width}px double");
    }
}

#[test]
fn only_a_groove_or_ridge_of_two_pixels_or_more_has_two_bands() {
    for style in [BorderStyle::Groove, BorderStyle::Ridge] {
        assert!(!has_two_bands(style, 1.0), "{style:?} of 1px");
        assert!(has_two_bands(style, 2.0), "{style:?} of 2px");
    }
    for style in [BorderStyle::Solid, BorderStyle::Inset, BorderStyle::Double] {
        assert!(!has_two_bands(style, 8.0), "{style:?}");
    }
}

#[test]
fn only_a_double_of_three_pixels_or_more_has_two_lines() {
    assert!(!has_two_lines(BorderStyle::Double, 2.0));
    assert!(has_two_lines(BorderStyle::Double, 3.0));
    assert!(!has_two_lines(BorderStyle::Solid, 3.0));
    assert!(!has_two_lines(BorderStyle::Groove, 8.0));
}
