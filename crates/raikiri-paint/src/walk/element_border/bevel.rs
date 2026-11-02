//! The lighter and darker shades of a color in which an `inset`, `outset`,
//! `groove` or `ridge` border is drawn (CSS Backgrounds 3 §3.2). The spec leaves
//! the shades to the user agent; these are the ones Chromium computes, so that a
//! bevel looks the same in both.

use raikiri_style::property::CssColor;
use std::sync::LazyLock;

/// The multiplier that scales a channel in `0.0..=1.0` to `0..=255` with no
/// rounding up: it is just below 256, so only a full channel reaches 255.
const SCALE: f32 = f32::from_bits(0x437f_ffff);

/// Black, which has no brightness to scale, is lifted to this gray instead.
const BLACK_LIFTED: u8 = 0x54;

/// A color at or below this gray is too dark to be darkened further.
const DARK_LIMIT: u8 = 0x20;

/// A color above this gray is too light to be lightened further.
const LIGHT_LIMIT: u8 = 0xEB;

/// The luminances of the grays at `DARK_LIMIT` and `LIGHT_LIMIT`.
static LIMIT_LUMINANCES: LazyLock<(f64, f64)> = LazyLock::new(|| {
    (
        luminance_of_gray(DARK_LIMIT),
        luminance_of_gray(LIGHT_LIMIT),
    )
});

/// How much `darker` and `lighter` take from or add to the brightest channel.
const STEP: f32 = 0.33;

/// The shade of `color` for the dark side of a bevel when `dark`, and for the
/// light side otherwise. The alpha channel is kept.
///
/// A very dark color cannot get darker, so its dark side is the lighter shade
/// and its light side a lighter one still; a very light color cannot get
/// lighter, so its light side is the color itself.
pub(super) fn shade(color: CssColor, dark: bool) -> CssColor {
    let luminance = luminance(color);
    let (dark_limit, light_limit) = *LIMIT_LUMINANCES;
    if luminance <= dark_limit {
        let lit = lighter(color);
        if dark { lit } else { lighter(lit) }
    } else if dark {
        darker(color)
    } else if luminance <= light_limit {
        lighter(color)
    } else {
        color
    }
}

/// The color with its brightest channel lowered by `STEP`, the others scaled
/// along with it, or black when that is not enough.
fn darker(color: CssColor) -> CssColor {
    let brightest = brightest(color);
    let scale = ((brightest - STEP) / brightest).max(0.0);
    scaled(color, scale)
}

/// The color with its brightest channel raised by `STEP`, the others scaled
/// along with it, up to white.
fn lighter(color: CssColor) -> CssColor {
    let brightest = brightest(color);
    if brightest == 0.0 {
        return CssColor {
            r: BLACK_LIFTED,
            g: BLACK_LIFTED,
            b: BLACK_LIFTED,
            ..color
        };
    }
    let scale = (brightest + STEP).min(1.0) / brightest;
    scaled(color, scale)
}

/// The largest of the color channels, in `0.0..=1.0`.
fn brightest(color: CssColor) -> f32 {
    f32::from(color.r.max(color.g).max(color.b)) / 255.0
}

/// The color with each of its channels multiplied by `scale`.
fn scaled(color: CssColor, scale: f32) -> CssColor {
    let channel = |value: u8| (scale * (f32::from(value) / 255.0) * SCALE) as u8;
    CssColor {
        r: channel(color.r),
        g: channel(color.g),
        b: channel(color.b),
        ..color
    }
}

/// The relative luminance of the color's red, green and blue channels (WCAG 2.x).
fn luminance(color: CssColor) -> f64 {
    let linear = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn luminance_of_gray(gray: u8) -> f64 {
    luminance(CssColor {
        r: gray,
        g: gray,
        b: gray,
        a: 255,
    })
}
