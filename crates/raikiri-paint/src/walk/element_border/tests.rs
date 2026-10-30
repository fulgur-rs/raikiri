use super::*;
use anyrender::Scene;
use anyrender::recording::RenderCommand;
use raikiri_style::property::Border;
use raikiri_style::{ComputedLength, ResolveContext, resolve_border};

const GREEN: CssColor = CssColor {
    r: 0,
    g: 128,
    b: 0,
    a: 255,
};

const TEXT: CssColor = CssColor {
    r: 10,
    g: 20,
    b: 30,
    a: 255,
};

fn side(
    width: f32,
    style: BorderStyle,
    color: BorderColor,
) -> raikiri_style::resolve::ComputedBorder {
    let mut authored = Border::new();
    authored.width = Length::Px(width);
    authored.style = style;
    authored.color = color;
    resolve_border(
        authored,
        ComputedLength(16.0),
        None,
        &ResolveContext::new(ComputedLength(16.0)),
    )
}

fn all_sides(width: f32, style: BorderStyle) -> Sides<raikiri_style::resolve::ComputedBorder> {
    let border = side(width, style, BorderColor::Resolved(GREEN));
    Sides {
        top: border,
        right: border,
        bottom: border,
        left: border,
    }
}

/// The rectangles and colors that the border of a 100 × 50 box at (10, 20)
/// fills, in the order it fills them.
fn fills(
    border: &Sides<raikiri_style::resolve::ComputedBorder>,
    paint_top: bool,
) -> Vec<(Rect, Color)> {
    let mut scene = Scene::new();
    if let Some(border) = ElementBorder::new(100.0, 50.0, 10.0, 20.0, border, TEXT, paint_top) {
        border.paint(&mut scene);
    }
    scene
        .commands
        .iter()
        .map(|command| {
            let RenderCommand::Fill(fill) = command else {
                panic!("expected a fill, got {command:?}");
            };
            let anyrender::Paint::Solid(color) = fill.brush else {
                panic!("expected a solid brush, got {:?}", fill.brush);
            };
            (kurbo::Shape::bounding_box(&fill.shape), color)
        })
        .collect()
}

fn green() -> Color {
    Color::from_rgba8(0, 128, 0, 255)
}

#[test]
fn a_solid_border_fills_a_strip_per_side_top_bottom_left_right() {
    assert_eq!(
        fills(&all_sides(4.0, BorderStyle::Solid), true),
        [
            (Rect::new(10.0, 20.0, 110.0, 24.0), green()),
            (Rect::new(10.0, 66.0, 110.0, 70.0), green()),
            (Rect::new(10.0, 24.0, 14.0, 66.0), green()),
            (Rect::new(106.0, 24.0, 110.0, 66.0), green()),
        ]
    );
}

#[test]
fn a_continuation_without_a_top_strip_starts_its_side_strips_at_the_box_top() {
    assert_eq!(
        fills(&all_sides(4.0, BorderStyle::Solid), false),
        [
            (Rect::new(10.0, 66.0, 110.0, 70.0), green()),
            (Rect::new(10.0, 20.0, 14.0, 66.0), green()),
            (Rect::new(106.0, 20.0, 110.0, 66.0), green()),
        ]
    );
}

#[test]
fn the_edges_of_the_box_are_rounded_to_whole_pixels() {
    let mut scene = Scene::new();
    ElementBorder::new(
        10.4,
        10.4,
        0.6,
        0.6,
        &all_sides(1.0, BorderStyle::Solid),
        TEXT,
        true,
    )
    .expect("a box with a border")
    .paint(&mut scene);
    let RenderCommand::Fill(top) = &scene.commands[0] else {
        panic!("expected a fill");
    };
    // 0.6 rounds to 1 and 0.6 + 10.4 rounds to 11.
    assert_eq!(
        kurbo::Shape::bounding_box(&top.shape),
        Rect::new(1.0, 1.0, 11.0, 2.0)
    );
}

#[test]
fn a_double_border_leaves_the_middle_third_of_each_side_empty() {
    let green = green();
    assert_eq!(
        fills(&all_sides(6.0, BorderStyle::Double), true),
        [
            // Top: the outer and the inner third.
            (Rect::new(10.0, 20.0, 110.0, 22.0), green),
            (Rect::new(10.0, 24.0, 110.0, 26.0), green),
            // Bottom.
            (Rect::new(10.0, 64.0, 110.0, 66.0), green),
            (Rect::new(10.0, 68.0, 110.0, 70.0), green),
            // Left, between the top and the bottom strips.
            (Rect::new(10.0, 26.0, 12.0, 64.0), green),
            (Rect::new(14.0, 26.0, 16.0, 64.0), green),
            // Right.
            (Rect::new(104.0, 26.0, 106.0, 64.0), green),
            (Rect::new(108.0, 26.0, 110.0, 64.0), green),
        ]
    );
}

#[test]
fn a_double_border_under_three_pixels_is_solid() {
    assert_eq!(
        fills(&all_sides(2.0, BorderStyle::Double), true),
        fills(&all_sides(2.0, BorderStyle::Solid), true)
    );
}

#[test]
fn a_side_without_a_style_or_a_visible_color_paints_nothing() {
    let green_side = side(4.0, BorderStyle::Solid, BorderColor::Resolved(GREEN));
    let clear = CssColor { a: 0, ..GREEN };
    let borders = Sides {
        top: side(4.0, BorderStyle::None, BorderColor::Resolved(GREEN)),
        right: side(4.0, BorderStyle::Hidden, BorderColor::Resolved(GREEN)),
        bottom: side(4.0, BorderStyle::Solid, BorderColor::Resolved(clear)),
        left: green_side,
    };
    // A side with no style has no used width, so the left strip starts at the
    // top of the box; the bottom side keeps its width without painting.
    assert_eq!(
        fills(&borders, true),
        [(Rect::new(10.0, 20.0, 14.0, 66.0), green())]
    );
}

#[test]
fn a_current_color_side_takes_the_text_color() {
    let border = side(4.0, BorderStyle::Solid, BorderColor::CurrentColor);
    let borders = Sides {
        top: border,
        right: border,
        bottom: border,
        left: border,
    };
    let text = Color::from_rgba8(10, 20, 30, 255);
    assert!(
        fills(&borders, true)
            .iter()
            .all(|(_, color)| *color == text)
    );
}

#[test]
fn a_side_strip_survives_a_box_that_the_other_sides_fill_completely() {
    // Left and right fill the 20px box between them, and top and bottom leave
    // no height for the content in between: the side strips are not drawn.
    let mut scene = Scene::new();
    let border = all_sides(10.0, BorderStyle::Solid);
    ElementBorder::new(20.0, 20.0, 0.0, 0.0, &border, TEXT, true)
        .expect("a box with a border")
        .paint(&mut scene);
    assert_eq!(scene.commands.len(), 2, "only the top and the bottom strip");
}

#[test]
fn an_empty_box_or_a_border_without_width_has_nothing_to_paint() {
    let solid = all_sides(4.0, BorderStyle::Solid);
    for (width, height) in [(0.0, 50.0), (100.0, 0.0), (-1.0, 50.0)] {
        assert!(ElementBorder::new(width, height, 0.0, 0.0, &solid, TEXT, true).is_none());
    }
    let flat = all_sides(0.0, BorderStyle::Solid);
    assert!(ElementBorder::new(100.0, 50.0, 0.0, 0.0, &flat, TEXT, true).is_none());
}

/// The bounding rectangles of the pieces that the border of a 100 × 50 box at
/// (10, 20) fills, in the order it fills them: every dash or dot of a path is
/// a piece of its own. The clip layers around them are ignored.
fn shapes(border: &Sides<raikiri_style::resolve::ComputedBorder>) -> Vec<Rect> {
    let mut scene = Scene::new();
    ElementBorder::new(100.0, 50.0, 10.0, 20.0, border, TEXT, true)
        .expect("a box with a border")
        .paint(&mut scene);
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(pieces(&fill.shape)),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The bounding rectangles of the closed subpaths of `path`.
fn pieces(path: &BezPath) -> Vec<Rect> {
    let mut pieces = Vec::new();
    let mut current = BezPath::new();
    for element in path.elements() {
        if matches!(element, kurbo::PathEl::MoveTo(_)) && !current.is_empty() {
            pieces.push(kurbo::Shape::bounding_box(&current));
            current = BezPath::new();
        }
        current.push(*element);
    }
    if !current.is_empty() {
        pieces.push(kurbo::Shape::bounding_box(&current));
    }
    pieces
}

fn close(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 1e-9
}

#[test]
fn a_dash_is_twice_the_thickness_and_a_gap_once_from_three_pixels_on() {
    // The pattern is laid out so that it starts and ends with a dash. Chromium
    // draws nine 8px dashes 3.5px apart along 100px, 5 dashes 5px apart for
    // 8px, 3 dashes 3px apart along 30px, and the same count of 6px dashes in a
    // 2px border (measured on its render).
    let Dashes { dash, gap, count } = dash_lengths(100.0, 4.0, false).expect("dashes");
    assert!(
        close(dash, 8.0) && close(gap, 3.5) && count == 9,
        "{dash} {gap}"
    );
    let Dashes { dash, gap, count } = dash_lengths(30.0, 4.0, false).expect("dashes");
    assert!(
        close(dash, 8.0) && close(gap, 3.0) && count == 3,
        "{dash} {gap}"
    );
    let Dashes { dash, gap, count } = dash_lengths(100.0, 8.0, false).expect("dashes");
    assert!(
        close(dash, 16.0) && close(gap, 5.0) && count == 5,
        "{dash} {gap}"
    );
    // Below 3px a dash is three times the thickness and a gap twice.
    let Dashes { dash, gap, count } = dash_lengths(100.0, 2.0, false).expect("dashes");
    assert!(close(dash, 6.0) && count == 10, "{dash}");
    assert!(close((100.0 + gap) / (dash + gap), 10.0), "{gap}");
    let Dashes { dash, gap, count } = dash_lengths(100.0, 1.0, false).expect("dashes");
    assert!(close(dash, 3.0) && count == 20, "{dash}");
    assert!(close((100.0 + gap) / (dash + gap), 20.0), "{gap}");
}

#[test]
fn a_dot_is_as_long_as_it_is_thick_and_a_gap_the_same() {
    let Dashes {
        dash: dot,
        gap,
        count,
    } = dash_lengths(100.0, 4.0, true).expect("dots");
    assert!(
        close(dot, 4.0) && close(gap, 4.0) && count == 13,
        "{dot} {gap}"
    );
    // 17 dots along 131px, a little closer than 4px.
    let Dashes {
        dash: dot,
        gap,
        count,
    } = dash_lengths(131.0, 4.0, true).expect("dots");
    assert!(close(dot, 4.0) && count == 17, "{dot}");
    assert!(close((131.0 + gap) / (dot + gap), 17.0), "{gap}");
}

#[test]
fn a_side_that_cannot_hold_two_dashes_has_none() {
    assert_eq!(dash_lengths(16.0, 4.0, false), None);
    assert_eq!(dash_lengths(30.0, 8.0, false), None);
    assert_eq!(dash_lengths(8.0, 4.0, true), None);
    // Room for two dashes and not for the gap in between at its natural
    // length: both shrink in proportion.
    let Dashes { dash, gap, count } = dash_lengths(18.0, 4.0, false).expect("two dashes");
    assert!(
        close(dash, 7.2) && close(gap, 3.6) && count == 2,
        "{dash} {gap}"
    );
    assert!(close(2.0 * dash + gap, 18.0));
}

#[test]
fn two_dots_keep_their_size_and_only_the_gap_between_them_shrinks() {
    // Chromium draws a 4px dot at each end of a 9px side, 1px apart, and 10px
    // dots on a 25px one, 5px apart.
    for (length, thickness, gap) in [(9.0, 4.0, 1.0), (25.0, 10.0, 5.0), (5.0, 2.0, 1.0)] {
        let dots = dash_lengths(length, thickness, true).expect("two dots");
        assert_eq!(
            dots,
            Dashes {
                dash: thickness,
                gap,
                count: 2
            }
        );
    }
}

#[test]
fn the_dots_of_a_short_dotted_side_stay_inside_the_box() {
    // The 25px top and bottom sides of the border-box hold two 10px dots.
    let box_edges = Rect::new(10.0, 20.0, 35.0, 100.0);
    let mut scene = Scene::new();
    ElementBorder::new(
        25.0,
        80.0,
        10.0,
        20.0,
        &all_sides(10.0, BorderStyle::Dotted),
        TEXT,
        true,
    )
    .expect("a box with a border")
    .paint(&mut scene);
    let dots = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(pieces(&fill.shape)),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    assert!(!dots.is_empty());
    for dot in &dots {
        assert!(
            dot.x0 >= box_edges.x0 - 1e-6
                && dot.x1 <= box_edges.x1 + 1e-6
                && dot.y0 >= box_edges.y0 - 1e-6
                && dot.y1 <= box_edges.y1 + 1e-6,
            "{dot:?} sticks out of {box_edges:?}"
        );
    }
    // Two dots along the top, the first at the left edge and the last at the
    // right one.
    for (dot, expected) in dots.iter().zip([
        Rect::new(10.0, 20.0, 20.0, 30.0),
        Rect::new(25.0, 20.0, 35.0, 30.0),
    ]) {
        assert!(
            close(dot.x0, expected.x0)
                && close(dot.y0, expected.y0)
                && close(dot.x1, expected.x1)
                && close(dot.y1, expected.y1),
            "{dot:?} is not {expected:?}"
        );
    }
}

#[test]
fn a_dashed_border_draws_dashes_from_corner_to_corner_of_each_side() {
    let rects = shapes(&all_sides(4.0, BorderStyle::Dashed));
    // Top and bottom: nine dashes of 8px each, 3.5px apart, the first at the
    // left edge and the last ending at the right one.
    let top = &rects[..9];
    assert!(top.iter().all(|r| close(r.y0, 20.0) && close(r.y1, 24.0)));
    assert!(close(top[0].x0, 10.0) && close(top[0].x1, 18.0));
    assert!(close(top[1].x0, 21.5) && close(top[1].x1, 29.5));
    assert!(close(top[8].x0, 102.0) && close(top[8].x1, 110.0));
    let bottom = &rects[9..18];
    assert!(
        bottom
            .iter()
            .all(|r| close(r.y0, 66.0) && close(r.y1, 70.0))
    );
    assert!(close(bottom[0].x0, 10.0) && close(bottom[8].x1, 110.0));
    // Left: five dashes of 8px, 2.5px apart over the whole 50px, drawn only
    // between the top and bottom strips, which own the corners.
    let left = &rects[18..23];
    assert!(left.iter().all(|r| close(r.x0, 10.0) && close(r.x1, 14.0)));
    assert!(close(left[0].y0, 24.0) && close(left[0].y1, 28.0));
    assert!(close(left[1].y0, 30.5) && close(left[1].y1, 38.5));
    assert!(close(left[4].y0, 62.0) && close(left[4].y1, 66.0));
    let right = &rects[23..];
    assert_eq!(right.len(), 5);
    assert!(
        right
            .iter()
            .all(|r| close(r.x0, 106.0) && close(r.x1, 110.0))
    );
}

#[test]
fn a_dotted_border_draws_round_dots_a_border_wide() {
    let rects = shapes(&all_sides(4.0, BorderStyle::Dotted));
    // 13 dots along the top, 13 along the bottom, and 5 along each of the
    // sides: of the 7 that fit along 50px, the first and the last lie in the
    // corners that the top and bottom dots cover.
    assert_eq!(rects.len(), 13 + 13 + 5 + 5);
    assert_eq!(rects[0], Rect::new(10.0, 20.0, 14.0, 24.0));
    assert_eq!(rects[12], Rect::new(106.0, 20.0, 110.0, 24.0));
    assert!(rects[26..].iter().all(|r| close(r.width(), 4.0)));
    let mut scene = Scene::new();
    ElementBorder::new(
        100.0,
        50.0,
        10.0,
        20.0,
        &all_sides(4.0, BorderStyle::Dotted),
        TEXT,
        true,
    )
    .expect("a box with a border")
    .paint(&mut scene);
    // The sides' dots are clipped to their strips; the top and bottom ones
    // run the whole line and need no clip.
    let clips = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::PushClipLayer(_)))
        .count();
    assert_eq!(clips, 2);
}

#[test]
fn a_dotted_border_up_to_three_pixels_wide_has_square_dots() {
    let rects = shapes(&all_sides(2.0, BorderStyle::Dotted));
    assert!(
        rects
            .iter()
            .all(|r| close(r.width(), 2.0) || close(r.height(), 2.0))
    );
    assert_eq!(rects[0], Rect::new(10.0, 20.0, 12.0, 22.0));
}

#[test]
fn a_side_too_short_for_two_dashes_is_filled_solid() {
    // 8px dashes are 16px long and need 32px for two of them: the 50px sides
    // hold them and the 30px ones of this box do not.
    let mut scene = Scene::new();
    ElementBorder::new(
        100.0,
        30.0,
        10.0,
        20.0,
        &all_sides(8.0, BorderStyle::Dashed),
        TEXT,
        true,
    )
    .expect("a box with a border")
    .paint(&mut scene);
    let solid = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(kurbo::Shape::bounding_box(&fill.shape)),
            _ => None,
        })
        .collect::<Vec<_>>();
    // Left and right are the strips between the 8px top and bottom ones.
    assert!(solid.contains(&Rect::new(10.0, 28.0, 18.0, 42.0)));
    assert!(solid.contains(&Rect::new(102.0, 28.0, 110.0, 42.0)));
}

#[test]
fn a_side_far_too_long_for_its_dashes_is_filled_solid() {
    let mut scene = Scene::new();
    ElementBorder::new(
        1.0e7,
        50.0,
        0.0,
        0.0,
        &all_sides(2.0, BorderStyle::Dashed),
        TEXT,
        true,
    )
    .expect("a box with a border")
    .paint(&mut scene);
    let fills = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(pieces(&fill.shape)),
            _ => None,
        })
        .collect::<Vec<_>>();
    // The long sides are two solid strips and the short ones keep their dashes.
    assert_eq!(fills.len(), 4);
    assert_eq!(fills[0], [Rect::new(0.0, 0.0, 1.0e7, 2.0)]);
    assert_eq!(fills[1], [Rect::new(0.0, 48.0, 1.0e7, 50.0)]);
    assert!(fills[2].len() > 1 && fills[3].len() > 1);
}

#[test]
fn a_side_that_is_not_finite_is_filled_solid() {
    for length in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(dash_lengths(length, 2.0, false), None);
        assert_eq!(dash_lengths(length, 4.0, true), None);
    }
    assert_eq!(dash_lengths(100.0, f64::NAN, false), None);
    assert_eq!(dash_lengths(100.0, f64::INFINITY, true), None);
}

#[test]
fn the_dashes_of_a_side_are_one_fill() {
    let mut scene = Scene::new();
    ElementBorder::new(
        100.0,
        50.0,
        10.0,
        20.0,
        &all_sides(4.0, BorderStyle::Dashed),
        TEXT,
        true,
    )
    .expect("a box with a border")
    .paint(&mut scene);
    assert_eq!(scene.commands.len(), 4, "one fill per side");
}

#[test]
fn each_side_keeps_its_own_style() {
    let solid = side(4.0, BorderStyle::Solid, BorderColor::Resolved(GREEN));
    let dashed = side(4.0, BorderStyle::Dashed, BorderColor::Resolved(GREEN));
    let borders = Sides {
        top: dashed,
        right: solid,
        bottom: solid,
        left: dashed,
    };
    let rects = shapes(&borders);
    // Nine top dashes, the solid bottom strip, five left dashes and the solid
    // right strip.
    assert_eq!(rects.len(), 9 + 1 + 5 + 1);
    assert_eq!(rects[9], Rect::new(10.0, 66.0, 110.0, 70.0));
    assert_eq!(rects[15], Rect::new(106.0, 24.0, 110.0, 66.0));
}

const GRAY: CssColor = CssColor {
    r: 128,
    g: 128,
    b: 128,
    a: 255,
};

fn rgb(r: u8, g: u8, b: u8) -> CssColor {
    CssColor { r, g, b, a: 255 }
}

fn color(color: CssColor) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

/// All four sides `width` thick in `style` and `color`.
fn colored_sides(
    width: f32,
    style: BorderStyle,
    color: CssColor,
) -> Sides<raikiri_style::resolve::ComputedBorder> {
    let border = side(width, style, BorderColor::Resolved(color));
    Sides {
        top: border,
        right: border,
        bottom: border,
        left: border,
    }
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
        assert_eq!(bevel::shade(input, true), dark, "dark side of {input:?}");
        assert_eq!(bevel::shade(input, false), light, "light side of {input:?}");
    }
}

#[test]
fn a_shade_keeps_the_alpha_of_its_color() {
    let translucent = CssColor { a: 99, ..GRAY };
    assert_eq!(bevel::shade(translucent, true).a, 99);
    assert_eq!(bevel::shade(translucent, false).a, 99);
    let black = CssColor {
        a: 7,
        ..rgb(0, 0, 0)
    };
    assert_eq!(bevel::shade(black, true).a, 7);
}

#[test]
fn an_outset_border_is_light_on_its_top_and_left_and_dark_on_its_bottom_and_right() {
    let light = color(rgb(212, 212, 212));
    let dark = color(rgb(44, 44, 44));
    assert_eq!(
        fills(&colored_sides(4.0, BorderStyle::Outset, GRAY), true),
        [
            (Rect::new(10.0, 20.0, 110.0, 24.0), light),
            (Rect::new(10.0, 66.0, 110.0, 70.0), dark),
            (Rect::new(10.0, 24.0, 14.0, 66.0), light),
            (Rect::new(106.0, 24.0, 110.0, 66.0), dark),
        ]
    );
}

#[test]
fn an_inset_border_is_dark_on_its_top_and_left_and_light_on_its_bottom_and_right() {
    let light = color(rgb(212, 212, 212));
    let dark = color(rgb(44, 44, 44));
    assert_eq!(
        fills(&colored_sides(4.0, BorderStyle::Inset, GRAY), true),
        [
            (Rect::new(10.0, 20.0, 110.0, 24.0), dark),
            (Rect::new(10.0, 66.0, 110.0, 70.0), light),
            (Rect::new(10.0, 24.0, 14.0, 66.0), dark),
            (Rect::new(106.0, 24.0, 110.0, 66.0), light),
        ]
    );
}

#[test]
fn a_groove_is_an_inset_band_around_an_outset_one() {
    let light = color(rgb(212, 212, 212));
    let dark = color(rgb(44, 44, 44));
    assert_eq!(
        fills(&colored_sides(4.0, BorderStyle::Groove, GRAY), true),
        [
            (Rect::new(10.0, 20.0, 110.0, 22.0), dark),
            (Rect::new(10.0, 68.0, 110.0, 70.0), light),
            (Rect::new(10.0, 22.0, 12.0, 68.0), dark),
            (Rect::new(108.0, 22.0, 110.0, 68.0), light),
            (Rect::new(12.0, 22.0, 108.0, 24.0), light),
            (Rect::new(12.0, 66.0, 108.0, 68.0), dark),
            (Rect::new(12.0, 24.0, 14.0, 66.0), light),
            (Rect::new(106.0, 24.0, 108.0, 66.0), dark),
        ]
    );
}

#[test]
fn a_ridge_is_an_outset_band_around_an_inset_one() {
    let light = color(rgb(212, 212, 212));
    let dark = color(rgb(44, 44, 44));
    assert_eq!(
        fills(&colored_sides(4.0, BorderStyle::Ridge, GRAY), true),
        [
            (Rect::new(10.0, 20.0, 110.0, 22.0), light),
            (Rect::new(10.0, 68.0, 110.0, 70.0), dark),
            (Rect::new(10.0, 22.0, 12.0, 68.0), light),
            (Rect::new(108.0, 22.0, 110.0, 68.0), dark),
            (Rect::new(12.0, 22.0, 108.0, 24.0), dark),
            (Rect::new(12.0, 66.0, 108.0, 68.0), light),
            (Rect::new(12.0, 24.0, 14.0, 66.0), dark),
            (Rect::new(106.0, 24.0, 108.0, 66.0), light),
        ]
    );
}

#[test]
fn an_odd_groove_gives_its_outer_band_the_larger_half() {
    let rects = shapes(&colored_sides(3.0, BorderStyle::Groove, GRAY));
    assert_eq!(rects[0], Rect::new(10.0, 20.0, 110.0, 22.0), "outer top");
    assert_eq!(rects[4], Rect::new(12.0, 22.0, 108.0, 23.0), "inner top");
    assert_eq!(rects[7], Rect::new(107.0, 23.0, 108.0, 67.0), "inner right");
}

#[test]
fn a_one_pixel_groove_or_ridge_is_a_single_band_in_the_plain_color() {
    for style in [BorderStyle::Groove, BorderStyle::Ridge] {
        let fills = fills(&colored_sides(1.0, style, GRAY), true);
        assert_eq!(fills.len(), 4, "{style:?}");
        assert!(
            fills.iter().all(|(_, fill)| *fill == color(GRAY)),
            "{style:?}"
        );
    }
}

#[test]
fn the_bands_of_a_continuation_leave_out_the_top_of_the_box() {
    let light = color(rgb(212, 212, 212));
    let dark = color(rgb(44, 44, 44));
    assert_eq!(
        fills(&colored_sides(4.0, BorderStyle::Groove, GRAY), false),
        [
            (Rect::new(10.0, 68.0, 110.0, 70.0), light),
            (Rect::new(10.0, 20.0, 12.0, 68.0), dark),
            (Rect::new(108.0, 20.0, 110.0, 68.0), light),
            (Rect::new(12.0, 66.0, 108.0, 68.0), dark),
            (Rect::new(12.0, 20.0, 14.0, 66.0), light),
            (Rect::new(106.0, 20.0, 108.0, 66.0), dark),
        ]
    );
}

#[test]
fn the_inner_band_of_a_groove_sits_inside_the_full_width_of_the_other_sides() {
    let solid = side(4.0, BorderStyle::Solid, BorderColor::Resolved(GRAY));
    let groove = side(4.0, BorderStyle::Groove, BorderColor::Resolved(GRAY));
    let borders = Sides {
        top: solid,
        right: solid,
        bottom: solid,
        left: groove,
    };
    assert_eq!(
        fills(&borders, true),
        [
            (Rect::new(10.0, 20.0, 110.0, 24.0), color(GRAY)),
            (Rect::new(10.0, 66.0, 110.0, 70.0), color(GRAY)),
            (Rect::new(10.0, 24.0, 12.0, 66.0), color(rgb(44, 44, 44))),
            (Rect::new(106.0, 24.0, 110.0, 66.0), color(GRAY)),
            (Rect::new(12.0, 24.0, 14.0, 66.0), color(rgb(212, 212, 212))),
        ]
    );
}

#[test]
fn a_groove_wider_than_its_box_has_no_inner_band() {
    // Layout never makes a box narrower than its borders, but the painter must
    // not draw outside the box if it is handed one: 21px sides leave no room
    // for the bands inside the outer ones, 11px each way on a 20px span.
    for (width, height) in [(20.0, 20.0), (100.0, 20.0)] {
        let border = colored_sides(21.0, BorderStyle::Groove, GRAY);
        let mut scene = Scene::new();
        ElementBorder::new(width, height, 0.0, 0.0, &border, TEXT, true)
            .expect("a box with a border")
            .paint(&mut scene);
        assert_eq!(
            scene.commands.len(),
            2,
            "{width} x {height}: top and bottom"
        );
    }
}

#[test]
fn a_bevel_with_a_transparent_color_paints_nothing() {
    let clear = CssColor { a: 0, ..GRAY };
    for style in [
        BorderStyle::Inset,
        BorderStyle::Outset,
        BorderStyle::Groove,
        BorderStyle::Ridge,
    ] {
        assert_eq!(
            fills(&colored_sides(4.0, style, clear), true),
            [],
            "{style:?}"
        );
    }
}

#[test]
fn a_bevel_without_a_color_takes_its_shades_from_the_text_color() {
    let border = side(4.0, BorderStyle::Outset, BorderColor::CurrentColor);
    let borders = Sides {
        top: border,
        right: border,
        bottom: border,
        left: border,
    };
    let fills = fills(&borders, true);
    // TEXT is (10, 20, 30).
    assert_eq!(fills[0].1, color(rgb(66, 132, 198)));
    assert_eq!(fills[1].1, color(rgb(38, 76, 114)));
}
