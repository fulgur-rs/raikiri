//! The hue a gradient stop takes when the gradient is interpolated in `lch`
//! or `oklch`, as Chromium draws it.
//!
//! CSS Color 4 §12.4 makes the hue of an achromatic color powerless, so a
//! ramp to `white` would take the hue of the stop next to it. Chromium settles
//! the hue twice, in two places that measure color differently:
//!
//! 1. Blink converts each stop to the interpolation space. A stop written in
//!    another space whose chroma is at most 0.02 gets a missing hue, and a
//!    missing component is taken from the neighboring stop, on each side of
//!    the stop separately. Blink then hands the stop to Skia as an sRGB
//!    color. Its Lab uses the white point (0.9642, 1, 0.8251).
//! 2. Skia converts the sRGB colors to the interpolation space again, with
//!    the CSS white point (0.3457 / 0.3585, 1, 0.2958 / 0.3585) for Lab and
//!    a direct linear sRGB to Oklab matrix. A hue is powerless only when the
//!    chroma is at most 0.01 in lch, or 1e-6 in oklch.
//!
//! The white points of the two Lab conversions differ by about 1e-4. So white
//! keeps a chroma of about 0.013 in the second step, above Skia's limit, and
//! its hue is the direction of that difference, near 100 degrees. A ramp from
//! `lch(50% 100 40)` to `white` therefore runs through yellow, where the
//! spec's carried hue keeps it red. Gray and black lose their hue in both
//! steps and carry the neighbor's. This module repeats both steps on the
//! stops, so the stops it returns carry the hue Chromium interpolates.

use std::sync::LazyLock;

use peniko::color::{ColorSpaceTag, DynamicColor, Flags};

use super::{ConicBudget, push_stop, stop_vector};

type Vec3 = [f64; 3];
type Mat3 = [Vec3; 3];

/// Linear sRGB to XYZ D50, as Skia's and Blink's sRGB gamut stores it: the
/// ICC 16.16 fixed-point values.
const SRGB_TO_XYZ_D50: Mat3 = [
    [
        0x6FA2 as f64 / 65536.0,
        0x6299 as f64 / 65536.0,
        0x24A0 as f64 / 65536.0,
    ],
    [
        0x38F5 as f64 / 65536.0,
        0xB785 as f64 / 65536.0,
        0x0F84 as f64 / 65536.0,
    ],
    [
        0x0390 as f64 / 65536.0,
        0x18DA as f64 / 65536.0,
        0xB6CF as f64 / 65536.0,
    ],
];

/// The white point Blink's Lab conversions divide by.
const BLINK_D50: Vec3 = [0.9642, 1.0, 0.8251];

/// The white point of CSS Lab, which Skia's Lab conversion uses.
const CSS_D50: Vec3 = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];

/// The Bradford cone response matrix that skcms adapts white points with.
const BRADFORD: Mat3 = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

/// The D50 white skcms adapts to.
const SKCMS_D50: Vec3 = [0.96422, 1.0, 0.82521];

/// Blink's XYZ D65 to Oklab LMS matrix.
const XYZ_D65_TO_LMS: Mat3 = [
    [0.8190224432164319, 0.3619062562801221, -0.12887378261216414],
    [0.0329836671980271, 0.9292868468965546, 0.03614466816999844],
    [
        0.048177199566046255,
        0.26423952494422764,
        0.6335478258136937,
    ],
];

/// Blink's Oklab to LMS matrix.
const OKLAB_TO_LMS: Mat3 = [
    [0.9999999984505198, 0.39633779217376786, 0.2158037580607588],
    [
        1.0000000088817609,
        -0.10556134232365635,
        -0.06385417477170591,
    ],
    [
        1.0000000546724108,
        -0.08948418209496575,
        -1.2914855378640917,
    ],
];

/// Skia's linear sRGB to Oklab LMS matrix.
const LINEAR_SRGB_TO_LMS: Mat3 = [
    [0.4122214708, 0.5363325363, 0.0514459929],
    [0.2119034982, 0.6806995451, 0.1073969566],
    [0.0883024619, 0.2817188376, 0.6299787005],
];

/// Skia's cube-root LMS to Oklab matrix.
const LMS_TO_OKLAB: Mat3 = [
    [0.2104542553, 0.7936177850, -0.0040720468],
    [1.9779984951, -2.4285922050, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.8086757660],
];

/// Largest chroma at which Blink makes the hue of a converted stop missing.
const BLINK_ACHROMATIC_CHROMA: f64 = 0.02;

/// Largest lch chroma at which Skia makes a hue powerless.
const SKIA_LCH_ACHROMATIC_CHROMA: f64 = 1e-2;

/// Largest oklch chroma at which Skia makes a hue powerless.
const SKIA_OKLCH_ACHROMATIC_CHROMA: f64 = 1e-6;

/// The stops of a gradient interpolated in `space`, with the hue each one
/// takes on each side as Chromium resolves it (see the module comment). A
/// stop whose two sides differ becomes two stops at its offset. Other spaces
/// keep the stops as they are.
pub(super) fn chromium_polar_stops(
    stops: Vec<(f64, DynamicColor)>,
    space: ColorSpaceTag,
    budget: Option<&ConicBudget<'_>>,
) -> Option<Vec<(f64, DynamicColor)>> {
    if !matches!(space, ColorSpaceTag::Lch | ColorSpaceTag::Oklch) || stops.len() < 2 {
        return Some(stops);
    }
    let sides = chromium_polar_sides(&stops, space, budget)?;
    let mut out = stop_vector(stops.len(), budget)?;
    for ((offset, _), [before, after]) in stops.iter().zip(sides) {
        push_stop(&mut out, (*offset, before), budget)?;
        if after != before {
            push_stop(&mut out, (*offset, after), budget)?;
        }
    }
    Some(out)
}

/// The color each stop of a gradient interpolated in `lch` or `oklch` takes
/// toward its previous and its next neighbor, as Chromium resolves its hue
/// (see the module comment). The first and the last stop take the color of
/// their one side on both. `stops` holds at least two.
pub(super) fn chromium_polar_sides(
    stops: &[(f64, DynamicColor)],
    space: ColorSpaceTag,
    budget: Option<&ConicBudget<'_>>,
) -> Option<Vec<[DynamicColor; 2]>> {
    let polar = Polar::of(space);
    if let Some(budget) = budget {
        budget.charge::<BlinkStop>(stops.len())?;
        budget.charge::<[Option<Lch>; 2]>(stops.len())?;
    }
    let blink: Vec<BlinkStop> = stops
        .iter()
        .map(|(_, color)| BlinkStop::new(*color, polar))
        .collect();
    // Each stop facing its previous and its next neighbor, after Blink and
    // after Skia's conversion.
    let last = stops.len() - 1;
    let sides: Vec<[Option<Lch>; 2]> = (0..stops.len())
        .map(|i| {
            let side =
                |neighbor: usize| polar.skia(polar.blink_xyz(blink[i].resolved(&blink[neighbor])));
            [
                (i > 0).then(|| side(i - 1)),
                (i < last).then(|| side(i + 1)),
            ]
        })
        .collect();
    let mut out = stop_vector(stops.len(), budget)?;
    for (i, (_, color)) in stops.iter().enumerate() {
        // Skia takes a powerless hue from the color next to it in its own
        // list: the neighbor's side that faces this stop.
        let resolved = [0, 1].map(|side| {
            let mut lch = sides[i][side]?;
            if lch.powerless {
                let neighbor = if side == 0 { i - 1 } else { i + 1 };
                lch.hue = sides[neighbor][1 - side]?.hue;
            }
            Some(lch.to_dynamic(space, alpha_on_side(&blink, i, side)))
        });
        // A stop whose hue comes out as its own keeps its authored color,
        // which the round trip through the two conversions would only move
        // by rounding.
        let own = blink[i].own;
        let keeps_own = !own.flags.missing().contains(2)
            && !blink[i].alpha_missing
            && resolved
                .iter()
                .flatten()
                .all(|side| hue_distance(side.components[2], own.components[2]) < SAME_HUE_DEGREES);
        // Every stop has a neighbor, so it has at least one side, which an
        // end stop takes on both.
        let [before, after] = resolved;
        let pair = if keeps_own {
            [*color, *color]
        } else {
            [before.or(after), after.or(before)].map(|side| side.unwrap_or(*color))
        };
        push_stop(&mut out, pair, budget)?;
    }
    Some(out)
}

/// Largest hue difference, in degrees, that counts as a stop's own hue.
const SAME_HUE_DEGREES: f32 = 0.1;

/// The angle between two hues in degrees, from 0 to 180.
fn hue_distance(a: f32, b: f32) -> f32 {
    let difference = (a - b).rem_euclid(360.0);
    difference.min(360.0 - difference)
}

/// The polar interpolation space, `lch` or `oklch`.
#[derive(Clone, Copy)]
enum Polar {
    Lch,
    Oklch,
}

impl Polar {
    fn of(space: ColorSpaceTag) -> Self {
        if space == ColorSpaceTag::Lch {
            Self::Lch
        } else {
            Self::Oklch
        }
    }

    fn tag(self) -> ColorSpaceTag {
        match self {
            Self::Lch => ColorSpaceTag::Lch,
            Self::Oklch => ColorSpaceTag::Oklch,
        }
    }

    /// The rectangular space next to this one: Lab or Oklab.
    fn rectangular(self) -> ColorSpaceTag {
        match self {
            Self::Lch => ColorSpaceTag::Lab,
            Self::Oklch => ColorSpaceTag::Oklab,
        }
    }

    /// Blink's lightness, a and b of an XYZ D50 color.
    fn blink_lab(self, xyz: Vec3) -> Vec3 {
        match self {
            Self::Lch => xyz_to_lab(xyz, BLINK_D50),
            Self::Oklch => {
                let d65 = mul(&DERIVED.d50_to_d65, xyz);
                let lms = mul(&XYZ_D65_TO_LMS, d65).map(f64::cbrt);
                mul(&DERIVED.lms_to_oklab, lms)
            }
        }
    }

    /// The XYZ D50 color Blink hands to Skia for a polar color in this space.
    fn blink_xyz(self, lch: Vec3) -> Vec3 {
        let lab = polar_to_rectangular(lch);
        match self {
            Self::Lch => lab_to_xyz(lab, BLINK_D50),
            Self::Oklch => blink_oklab_to_xyz(lab),
        }
    }

    /// Skia's polar color for an XYZ D50 color, and whether its hue is
    /// powerless.
    fn skia(self, xyz: Vec3) -> Lch {
        let (lab, limit) = match self {
            Self::Lch => (xyz_to_lab(xyz, CSS_D50), SKIA_LCH_ACHROMATIC_CHROMA),
            Self::Oklch => {
                let linear = mul(&DERIVED.xyz_d50_to_srgb, xyz);
                let lms = mul(&LINEAR_SRGB_TO_LMS, linear).map(f64::cbrt);
                (mul(&LMS_TO_OKLAB, lms), SKIA_OKLCH_ACHROMATIC_CHROMA)
            }
        };
        let [lightness, chroma, hue] = rectangular_to_polar(lab);
        Lch {
            lightness,
            chroma,
            hue,
            powerless: chroma <= limit,
        }
    }
}

/// A stop as Blink holds it in the interpolation space, before missing
/// components are taken from a neighbor.
struct BlinkStop {
    /// Lightness, chroma and hue. A missing one holds the value Blink keeps
    /// beside the flag, which a neighbor's missing component takes.
    lch: Vec3,
    missing: [bool; 3],
    alpha: f32,
    alpha_missing: bool,
    /// The authored color in the interpolation space, as CSS converts it.
    own: DynamicColor,
}

impl BlinkStop {
    fn new(color: DynamicColor, polar: Polar) -> Self {
        let flags = color.flags.missing();
        let alpha = color.components[3];
        let alpha_missing = flags.contains(3);
        let own = color.convert(polar.tag());
        if color.cs == polar.tag() {
            let [l, c, h, _] = color.components.map(f64::from);
            return Self {
                lch: [l, c, h],
                missing: [flags.contains(0), flags.contains(1), flags.contains(2)],
                alpha,
                alpha_missing,
                own,
            };
        }
        let lab = if color.cs == polar.rectangular() {
            let [l, a, b, _] = color.components.map(f64::from);
            [l, a, b]
        } else {
            polar.blink_lab(blink_xyz_of(color))
        };
        let lch = rectangular_to_polar(lab);
        // Components missing in the authored color stay missing when they are
        // analogous to one of this space (CSS Color 4 §12.2).
        let carried = own.flags.missing();
        Self {
            lch,
            missing: [
                carried.contains(0),
                carried.contains(1),
                lch[1] <= BLINK_ACHROMATIC_CHROMA || carried.contains(2),
            ],
            alpha,
            alpha_missing,
            own,
        }
    }

    /// This stop with its missing components taken from `neighbor`.
    fn resolved(&self, neighbor: &Self) -> Vec3 {
        let mut lch = self.lch;
        for (i, value) in lch.iter_mut().enumerate() {
            if self.missing[i] {
                *value = neighbor.lch[i];
            }
        }
        lch
    }
}

/// The alpha of stop `i` on side `side` (0 before it, 1 after it): its own,
/// or the neighbor's on that side when it is missing.
fn alpha_on_side(stops: &[BlinkStop], i: usize, side: usize) -> f32 {
    let stop = &stops[i];
    if !stop.alpha_missing {
        return stop.alpha;
    }
    let neighbor = if side == 0 { i - 1 } else { i + 1 };
    stops[neighbor].alpha
}

/// A polar color in Skia's list of stops.
#[derive(Clone, Copy)]
struct Lch {
    lightness: f64,
    chroma: f64,
    hue: f64,
    powerless: bool,
}

impl Lch {
    fn to_dynamic(self, space: ColorSpaceTag, alpha: f32) -> DynamicColor {
        DynamicColor {
            cs: space,
            flags: Flags::default(),
            components: [
                self.lightness as f32,
                self.chroma as f32,
                self.hue as f32,
                alpha,
            ],
        }
    }
}

/// The XYZ D50 color Blink measures a stop by: Lab and LCH through Blink's
/// white point, Oklab and OKLCh through Blink's Oklab matrices, and other
/// colors by their linear sRGB components through the fixed-point sRGB gamut.
fn blink_xyz_of(color: DynamicColor) -> Vec3 {
    let [first, second, third, _] = color.components.map(f64::from);
    let coordinates = [first, second, third];
    match color.cs {
        ColorSpaceTag::Lab => lab_to_xyz(coordinates, BLINK_D50),
        ColorSpaceTag::Lch => lab_to_xyz(polar_to_rectangular(coordinates), BLINK_D50),
        ColorSpaceTag::Oklab => blink_oklab_to_xyz(coordinates),
        ColorSpaceTag::Oklch => blink_oklab_to_xyz(polar_to_rectangular(coordinates)),
        _ => {
            let [r, g, b, _] = color.convert(ColorSpaceTag::LinearSrgb).components;
            mul(&SRGB_TO_XYZ_D50, [r, g, b].map(f64::from))
        }
    }
}

/// Blink's XYZ D50 color of an Oklab color.
fn blink_oklab_to_xyz(lab: Vec3) -> Vec3 {
    let lms = mul(&OKLAB_TO_LMS, lab).map(|v| v * v * v);
    mul(&DERIVED.d65_to_d50, mul(&DERIVED.lms_to_xyz_d65, lms))
}

/// The inverses and the white point adaptation the conversions use, worked
/// out once from the constants above.
struct Derived {
    d65_to_d50: Mat3,
    d50_to_d65: Mat3,
    lms_to_xyz_d65: Mat3,
    lms_to_oklab: Mat3,
    xyz_d50_to_srgb: Mat3,
}

static DERIVED: LazyLock<Derived> = LazyLock::new(|| {
    let d65_to_d50 = d65_to_d50();
    Derived {
        d65_to_d50,
        d50_to_d65: inverse(&d65_to_d50),
        lms_to_xyz_d65: inverse(&XYZ_D65_TO_LMS),
        lms_to_oklab: inverse(&OKLAB_TO_LMS),
        xyz_d50_to_srgb: inverse(&SRGB_TO_XYZ_D50),
    }
});

/// skcms's Bradford adaptation from the D65 white (0.3127, 0.3290) to D50.
fn d65_to_d50() -> Mat3 {
    let (x, y) = (0.3127, 0.3290);
    let d65 = [x / y, 1.0, (1.0 - x - y) / y];
    let source = mul(&BRADFORD, d65);
    let target = mul(&BRADFORD, SKCMS_D50);
    let scale = [
        [target[0] / source[0], 0.0, 0.0],
        [0.0, target[1] / source[1], 0.0],
        [0.0, 0.0, target[2] / source[2]],
    ];
    product(&inverse(&BRADFORD), &product(&scale, &BRADFORD))
}

const LAB_EPSILON: f64 = 216.0 / 24389.0;
const LAB_KAPPA: f64 = 24389.0 / 27.0;

fn xyz_to_lab(xyz: Vec3, white: Vec3) -> Vec3 {
    let f = |v: f64| {
        if v > LAB_EPSILON {
            v.cbrt()
        } else {
            (LAB_KAPPA * v + 16.0) / 116.0
        }
    };
    let [fx, fy, fz] = [
        f(xyz[0] / white[0]),
        f(xyz[1] / white[1]),
        f(xyz[2] / white[2]),
    ];
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn lab_to_xyz(lab: Vec3, white: Vec3) -> Vec3 {
    let [l, a, b] = lab;
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let cube_or_linear = |f: f64| {
        let cube = f * f * f;
        if cube > LAB_EPSILON {
            cube
        } else {
            (116.0 * f - 16.0) / LAB_KAPPA
        }
    };
    let y = if l > LAB_KAPPA * LAB_EPSILON {
        fy * fy * fy
    } else {
        l / LAB_KAPPA
    };
    [
        cube_or_linear(fx) * white[0],
        y * white[1],
        cube_or_linear(fz) * white[2],
    ]
}

fn rectangular_to_polar([l, a, b]: Vec3) -> Vec3 {
    let hue = b.atan2(a).to_degrees();
    [l, a.hypot(b), if hue < 0.0 { hue + 360.0 } else { hue }]
}

fn polar_to_rectangular([l, c, h]: Vec3) -> Vec3 {
    let (sin, cos) = h.to_radians().sin_cos();
    [l, c * cos, c * sin]
}

fn mul(m: &Mat3, v: Vec3) -> Vec3 {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

fn product(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn inverse(m: &Mat3) -> Mat3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [
        [
            (e * i - f * h) / det,
            (c * h - b * i) / det,
            (b * f - c * e) / det,
        ],
        [
            (f * g - d * i) / det,
            (a * i - c * g) / det,
            (c * d - a * f) / det,
        ],
        [
            (d * h - e * g) / det,
            (b * g - a * h) / det,
            (a * e - b * d) / det,
        ],
    ]
}
