use super::*;
use raikiri_style::property::{BorderColor, CssColor};

const RED: CssColor = CssColor {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};
const BLACK: CssColor = CssColor {
    r: 0,
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
const TRANSPARENT: CssColor = CssColor {
    r: 0,
    g: 0,
    b: 0,
    a: 0,
};

#[test]
fn current_color_resolves_to_computed_color() {
    // Hazard 1: <div style="color: red; border-color: currentColor">
    assert_eq!(resolve_border_color(BorderColor::CurrentColor, RED), RED);
}

#[test]
fn initial_current_color_resolves_to_computed_color() {
    // Hazard 2: <div style="color: red"> (border-color omitted → initial
    // is CurrentColor per CSS Backgrounds 3 §3.1). Same helper path;
    // cascade-side check is in raikiri-style's initial_values_match_spec,
    // here we check the paint-side resolution.
    assert_eq!(resolve_border_color(BorderColor::CurrentColor, RED), RED);
}

#[test]
fn resolved_black_is_independent_of_current_color() {
    // Hazard 3: <div style="border-color: black"> with a different
    // currentColor — Resolved must ignore current_color.
    assert_eq!(
        resolve_border_color(BorderColor::Resolved(BLACK), RED),
        BLACK
    );
    assert_eq!(
        resolve_border_color(BorderColor::Resolved(BLACK), BLUE),
        BLACK
    );
}

#[test]
fn resolved_transparent_preserved() {
    // `transparent` is a valid <color> (CssColor::TRANSPARENT) that must
    // pass through unchanged even when currentColor is opaque red.
    assert_eq!(
        resolve_border_color(BorderColor::Resolved(TRANSPARENT), RED),
        TRANSPARENT
    );
}

#[test]
fn current_color_with_transparent_current() {
    // CurrentColor where the computed color itself is transparent — still
    // just returns current_color verbatim (no special-casing).
    assert_eq!(
        resolve_border_color(BorderColor::CurrentColor, TRANSPARENT),
        TRANSPARENT
    );
}

#[test]
fn resolved_red_ignores_transparent_current() {
    assert_eq!(
        resolve_border_color(BorderColor::Resolved(RED), TRANSPARENT),
        RED
    );
}
