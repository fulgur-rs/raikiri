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
