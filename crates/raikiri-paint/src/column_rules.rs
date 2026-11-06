//! Paint producer-computed column rules without deriving column layout.

use anyrender::PaintScene;
use kurbo::{Affine, BezPath, Cap, Circle, Rect, Stroke};
use peniko::{Color, Fill};
use raikiri_dom::ColumnRule;
use raikiri_style::property::{BorderStyle, CssColor};

use crate::bevel;

const MAX_RULE_SEGMENTS: usize = 4096;

fn color(color: CssColor) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

pub(crate) fn paint(scene: &mut impl PaintScene, rule: ColumnRule) {
    let rect = rule.rect;
    if rule.color.a == 0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let x0 = f64::from(rect.x).round();
    let y0 = f64::from(rect.y).round();
    let x1 = f64::from(rect.x + rect.width).round();
    let y1 = f64::from(rect.y + rect.height).round();
    let bounds = Rect::new(x0, y0, x1, y1);
    let width = x1 - x0;
    if width <= 0.0 || y1 <= y0 {
        return;
    }
    match rule.style {
        BorderStyle::None | BorderStyle::Hidden => {} // cov:ignore: retained source rules require positive width; these styles compute to zero.
        style if bevel::has_two_lines(style, width) => paint_double(scene, bounds, rule),
        style if bevel::has_two_bands(style, width) => paint_ridge_or_groove(scene, bounds, rule),
        BorderStyle::Dotted => paint_dotted(scene, bounds, rule),
        BorderStyle::Dashed => paint_dashed(scene, bounds, rule),
        _ => scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color(rule.color),
            None,
            &bounds,
        ),
    }
}

/// Two bars at the edges of the rule, a third of its width each, the way
/// Chromium divides a `double` border (see `bevel::third`).
fn paint_double(scene: &mut impl PaintScene, bounds: Rect, rule: ColumnRule) {
    let Rect { x0, y0, x1, y1 } = bounds;
    let bar = bevel::third(x1 - x0);
    for rect in [
        Rect::new(x0, y0, x0 + bar, y1),
        Rect::new(x1 - bar, y0, x1, y1),
    ] {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color(rule.color),
            None,
            &rect,
        );
    }
}

/// Two halves in a lighter and a darker shade of the rule's color, the left one
/// the wider when the width is odd.
fn paint_ridge_or_groove(scene: &mut impl PaintScene, bounds: Rect, rule: ColumnRule) {
    let Rect { x0, y0, x1, y1 } = bounds;
    let width = x1 - x0;
    let light = bevel::shade(rule.color, false);
    let dark = bevel::shade(rule.color, true);
    let (left, right) = if rule.style == BorderStyle::Groove {
        (dark, light)
    } else {
        (light, dark)
    };
    let middle = x0 + bevel::nearer_half(width);
    for (rect, shade) in [
        (Rect::new(x0, y0, middle, y1), left),
        (Rect::new(middle, y0, x1, y1), right),
    ] {
        scene.fill(Fill::NonZero, Affine::IDENTITY, color(shade), None, &rect);
    }
}

/// Dots a rule's width across, clipped to the rule and anchored to its
/// pattern origin.
fn paint_dotted(scene: &mut impl PaintScene, bounds: Rect, rule: ColumnRule) {
    let Rect { x0, y0, x1, y1 } = bounds;
    let width = x1 - x0;
    let origin = f64::from(rule.pattern_origin).round();
    let end = f64::from(rule.pattern_origin + rule.pattern_height).round();
    let length = end - origin;
    // Keep the complete pattern anchored across page slices while
    // bounding scene commands for hostile but finite page heights.
    let count = (length / (2.0 * width))
        .round()
        .clamp(1.0, (MAX_RULE_SEGMENTS - 1) as f64);
    let spacing = length / count;
    let first = ((y0 - width / 2.0 - origin) / spacing).ceil().max(0.0) as usize;
    let last = ((y1 + width / 2.0 - origin) / spacing).floor().min(count) as usize;
    scene.push_clip_layer(Affine::IDENTITY, &bounds);
    for index in first..=last {
        let dot = Circle::new(
            ((x0 + x1) / 2.0, origin + index as f64 * spacing),
            width / 2.0,
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color(rule.color),
            None,
            &dot,
        );
    }
    scene.pop_layer();
}

/// A dashed stroke along the rule's center line, clipped to the rule and
/// anchored to its pattern origin.
fn paint_dashed(scene: &mut impl PaintScene, bounds: Rect, rule: ColumnRule) {
    let Rect { x0, x1, .. } = bounds;
    let width = x1 - x0;
    let origin = f64::from(rule.pattern_origin).round();
    let end = f64::from(rule.pattern_origin + rule.pattern_height).round();
    let length = end - origin;
    let natural_dash = 3.0 * width;
    let natural_count = ((length + natural_dash) / (2.0 * natural_dash))
        .round()
        .max(1.0);
    let count = natural_count.min(MAX_RULE_SEGMENTS as f64);
    let dash = natural_dash * (natural_count / count);
    let (dash, gap) = if count < 2.0 {
        (length, 0.0)
    } else {
        (dash, (length - count * dash) / (count - 1.0))
    };
    let mut path = BezPath::new();
    path.move_to(((x0 + x1) / 2.0, origin));
    path.line_to(((x0 + x1) / 2.0, end));
    let stroke = Stroke::new(width)
        .with_caps(Cap::Butt)
        .with_dashes(0.0, [dash, gap]);
    scene.push_clip_layer(Affine::IDENTITY, &bounds);
    scene.stroke(&stroke, Affine::IDENTITY, color(rule.color), None, &path);
    scene.pop_layer();
}
