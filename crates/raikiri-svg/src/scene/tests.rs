use super::*;
use crate::{SvgElementStyle, SvgRootFont};
use raikiri_traits::vector::Node;
use resvg::tiny_skia;

const VIEWPORT: SvgViewport = SvgViewport {
    width: 40.0,
    height: 20.0,
};

fn document(body: &str) -> SvgDocument {
    SvgDocument::parse(
        format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">{body}</svg>"##)
            .as_bytes(),
    )
    .unwrap()
}

fn document_with_view_box(attributes: &str, body: &str) -> SvgDocument {
    SvgDocument::parse(
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" {attributes}>{body}</svg>"##
        )
        .as_bytes(),
    )
    .unwrap()
}

fn style(opacity: f32) -> SvgRootStyle {
    SvgRootStyle {
        opacity,
        ..SvgRootStyle::default()
    }
}

fn tiny_transform(transform: PaintTransform) -> tiny_skia::Transform {
    tiny_skia::Transform::from_row(
        transform.a,
        transform.b,
        transform.c,
        transform.d,
        transform.e,
        transform.f,
    )
}

fn tiny_color(color: PaintColor, opacity: f32) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(color.red, color.green, color.blue, color.alpha * opacity).unwrap()
}

fn tiny_stops(stops: &[GradientStop], opacity: f32) -> Vec<tiny_skia::GradientStop> {
    stops
        .iter()
        .map(|stop| tiny_skia::GradientStop::new(stop.offset, tiny_color(stop.color, opacity)))
        .collect()
}

fn tiny_spread(spread: SpreadMethod) -> tiny_skia::SpreadMode {
    match spread {
        SpreadMethod::Pad => tiny_skia::SpreadMode::Pad,
        SpreadMethod::Reflect => tiny_skia::SpreadMode::Reflect,
        SpreadMethod::Repeat => tiny_skia::SpreadMode::Repeat,
        _ => unreachable!(),
    }
}

fn solid(paint: &Paint, opacity: f32) -> tiny_skia::Paint<'static> {
    let mut out = tiny_skia::Paint::default();
    match paint {
        Paint::Color(color) => out.set_color(tiny_color(*color, opacity)),
        Paint::LinearGradient(gradient) => {
            out.shader = tiny_skia::LinearGradient::new(
                gradient.start.into(),
                gradient.end.into(),
                tiny_stops(&gradient.stops, opacity),
                tiny_spread(gradient.spread),
                tiny_transform(gradient.transform),
            )
            .expect("a drawable gradient");
        }
        Paint::RadialGradient(gradient) => {
            out.shader = tiny_skia::RadialGradient::new(
                gradient.focal.into(),
                gradient.focal_radius,
                gradient.center.into(),
                gradient.radius,
                tiny_stops(&gradient.stops, opacity),
                tiny_spread(gradient.spread),
                tiny_transform(gradient.transform),
            )
            .expect("a drawable gradient");
        }
        _ => unreachable!(),
    }
    out
}

fn outline(segments: &[PathSegment]) -> tiny_skia::Path {
    let mut builder = tiny_skia::PathBuilder::new();
    for segment in segments {
        match *segment {
            PathSegment::MoveTo((x, y)) => builder.move_to(x, y),
            PathSegment::LineTo((x, y)) => builder.line_to(x, y),
            PathSegment::QuadTo { control, to } => {
                builder.quad_to(control.0, control.1, to.0, to.1)
            }
            PathSegment::CubicTo {
                control1,
                control2,
                to,
            } => builder.cubic_to(control1.0, control1.1, control2.0, control2.1, to.0, to.1),
            PathSegment::Close => builder.close(),
            _ => unreachable!("the builder emits only these segments"),
        }
    }
    builder.finish().expect("a drawable path")
}

fn draw_path(pixmap: &mut tiny_skia::Pixmap, path: &Path, transform: tiny_skia::Transform) {
    let outline = outline(&path.segments);
    let draw_fill = |pixmap: &mut tiny_skia::Pixmap| {
        if let Some(fill) = &path.fill {
            pixmap.fill_path(
                &outline,
                &solid(&fill.paint, fill.opacity),
                fill_rule(fill.rule),
                transform,
                None,
            );
        }
    };
    let draw_stroke = |pixmap: &mut tiny_skia::Pixmap| {
        if let Some(stroke) = &path.stroke {
            let mut line = tiny_skia::Stroke {
                width: stroke.width,
                miter_limit: stroke.miter_limit,
                line_cap: match stroke.cap {
                    LineCap::Butt => tiny_skia::LineCap::Butt,
                    LineCap::Round => tiny_skia::LineCap::Round,
                    LineCap::Square => tiny_skia::LineCap::Square,
                    _ => unreachable!(),
                },
                line_join: match stroke.join {
                    LineJoin::Miter => tiny_skia::LineJoin::Miter,
                    LineJoin::MiterClip => tiny_skia::LineJoin::MiterClip,
                    LineJoin::Round => tiny_skia::LineJoin::Round,
                    LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
                    _ => unreachable!(),
                },
                dash: None,
            };
            if let Some(dash) = &stroke.dash {
                line.dash = tiny_skia::StrokeDash::new(dash.array.clone(), dash.offset);
            }
            let mut paint = solid(&stroke.paint, stroke.opacity);
            paint.anti_alias = path.anti_alias;
            pixmap.stroke_path(&outline, &paint, &line, transform, None);
        }
    };
    match path.paint_order {
        PaintOrder::FillThenStroke => {
            draw_fill(pixmap);
            draw_stroke(pixmap);
        }
        PaintOrder::StrokeThenFill => {
            draw_stroke(pixmap);
            draw_fill(pixmap);
        }
        _ => unreachable!(),
    }
}

fn fill_rule(rule: FillRule) -> tiny_skia::FillRule {
    match rule {
        FillRule::NonZero => tiny_skia::FillRule::Winding,
        FillRule::EvenOdd => tiny_skia::FillRule::EvenOdd,
        _ => unreachable!(),
    }
}

/// Draws the shapes of a clip in black with `mode`, the way the raster does.
fn draw_clip_shapes(
    pixmap: &mut tiny_skia::Pixmap,
    group: &Group,
    mode: tiny_skia::BlendMode,
    transform: tiny_skia::Transform,
) {
    for child in &group.children {
        match child {
            Node::Path(path) => {
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(0, 0, 0, 255);
                paint.blend_mode = mode;
                let rule = path
                    .fill
                    .as_ref()
                    .map_or(FillRule::NonZero, |fill| fill.rule);
                pixmap.fill_path(
                    &outline(&path.segments),
                    &paint,
                    fill_rule(rule),
                    transform,
                    None,
                );
            }
            Node::Group(inner) => {
                let transform = transform.pre_concat(tiny_transform(inner.transform));
                match &inner.clip {
                    // A shape that is clipped itself is drawn on a canvas of
                    // its own, clipped, and only then added to the clip.
                    Some(clip) => {
                        let mut layer =
                            tiny_skia::Pixmap::new(pixmap.width(), pixmap.height()).unwrap();
                        draw_clip_shapes(
                            &mut layer,
                            inner,
                            tiny_skia::BlendMode::SourceOver,
                            transform,
                        );
                        apply_clip(&mut layer, clip, transform);
                        pixmap.draw_pixmap(
                            0,
                            0,
                            layer.as_ref(),
                            &tiny_skia::PixmapPaint {
                                blend_mode: tiny_skia::BlendMode::Xor,
                                ..tiny_skia::PixmapPaint::default()
                            },
                            tiny_skia::Transform::identity(),
                            None,
                        );
                    }
                    None => draw_clip_shapes(pixmap, inner, mode, transform),
                }
            }
            _ => unreachable!(),
        }
    }
}

/// Keeps what `clip` covers of `pixmap`; `transform` is the space of the
/// clipped group's children.
fn apply_clip(pixmap: &mut tiny_skia::Pixmap, clip: &ClipPath, transform: tiny_skia::Transform) {
    let mut area = tiny_skia::Pixmap::new(pixmap.width(), pixmap.height()).unwrap();
    area.fill(tiny_skia::Color::BLACK);
    draw_clip_shapes(
        &mut area,
        &clip.root,
        tiny_skia::BlendMode::Clear,
        transform.pre_concat(tiny_transform(clip.transform)),
    );
    if let Some(chained) = &clip.clip {
        apply_clip(pixmap, chained, transform);
    }
    let mut mask = tiny_skia::Mask::from_pixmap(area.as_ref(), tiny_skia::MaskType::Alpha);
    mask.invert();
    pixmap.apply_mask(&mask);
}

fn draw_group(pixmap: &mut tiny_skia::Pixmap, group: &Group, parent: tiny_skia::Transform) {
    let transform = parent.pre_concat(tiny_transform(group.transform));
    let mut layer = (group.opacity < 1.0 || group.clip.is_some())
        .then(|| tiny_skia::Pixmap::new(pixmap.width(), pixmap.height()).unwrap());
    for child in &group.children {
        let target = layer.as_mut().unwrap_or(pixmap);
        match child {
            Node::Group(group) => draw_group(target, group, transform),
            Node::Path(path) => draw_path(target, path, transform),
            _ => unreachable!(),
        }
    }
    if let Some(mut layer) = layer {
        if let Some(clip) = &group.clip {
            apply_clip(&mut layer, clip, transform);
        }
        pixmap.draw_pixmap(
            0,
            0,
            layer.as_ref(),
            &tiny_skia::PixmapPaint {
                opacity: group.opacity,
                ..tiny_skia::PixmapPaint::default()
            },
            tiny_skia::Transform::identity(),
            None,
        );
    }
}

/// The scene drawn by tiny-skia on a pixmap of its size, straight alpha.
fn render(scene: &VectorImage) -> Vec<u8> {
    let mut pixmap =
        tiny_skia::Pixmap::new(scene.size.0.ceil() as u32, scene.size.1.ceil() as u32).unwrap();
    draw_group(&mut pixmap, &scene.root, tiny_skia::Transform::identity());
    pixmap.take_demultiplied()
}

/// Compares the pictures with premultiplied colors: an edge pixel of low
/// alpha has a straight color that moves by several levels for one level of
/// rounding.
fn assert_close(scene: &[u8], raster: &[u8], tolerance: u8) {
    assert_eq!(scene.len(), raster.len());
    assert!(
        raster.chunks_exact(4).any(|pixel| pixel[3] != 0),
        "the raster is blank, so the comparison proves nothing"
    );
    let premultiplied = |pixel: &[u8]| {
        let alpha = u16::from(pixel[3]);
        let channel = |value: u8| ((u16::from(value) * alpha + 127) / 255) as u8;
        [
            channel(pixel[0]),
            channel(pixel[1]),
            channel(pixel[2]),
            pixel[3],
        ]
    };
    for (index, (scene, raster)) in scene
        .chunks_exact(4)
        .zip(raster.chunks_exact(4))
        .enumerate()
    {
        let (scene, raster) = (premultiplied(scene), premultiplied(raster));
        for channel in 0..4 {
            assert!(
                scene[channel].abs_diff(raster[channel]) <= tolerance,
                "pixel {index} channel {channel}: scene {scene:?}, raster {raster:?}"
            );
        }
    }
}

/// Builds the scene and the raster of `document` with the same inputs and
/// checks that they draw the same picture.
fn assert_scene_matches_raster(document: &SvgDocument, root_style: SvgRootStyle, tolerance: u8) {
    let scene = document.scene(VIEWPORT, root_style).unwrap();
    assert_eq!(scene.size, (40.0, 20.0));
    let raster = document
        .rasterize_at_css_pixel_scale(
            VIEWPORT,
            SvgRootStyle {
                neutralize_root_opacity: true,
                ..root_style
            },
            None,
        )
        .unwrap();
    assert_close(&render(&scene), &raster.rgba, tolerance);
}

fn paths(group: &Group) -> Vec<&Path> {
    let mut found = Vec::new();
    for child in &group.children {
        match child {
            Node::Group(group) => found.extend(paths(group)),
            Node::Path(path) => found.push(path),
            _ => unreachable!(),
        }
    }
    found
}

fn groups(group: &Group) -> Vec<&Group> {
    let mut found = vec![group];
    for child in &group.children {
        if let Node::Group(group) = child {
            found.extend(groups(group));
        }
    }
    found
}

fn scene_error(document: &SvgDocument) -> SvgSceneError {
    document.scene(VIEWPORT, style(1.0)).unwrap_err()
}

// ── Equivalence with the raster ─────────────────────────────

#[test]
fn shapes_and_paths_draw_like_the_raster() {
    let document = document(
        r##"<rect x="2" y="3" width="12" height="9" fill="#e11"/>
            <circle cx="28" cy="10" r="7" fill="none" stroke="#14c" stroke-width="3"
                    stroke-dasharray="4 2" stroke-dashoffset="1"/>
            <ellipse cx="10" cy="14" rx="6" ry="3" fill="#0a0" stroke="#000"/>
            <polygon points="20,2 24,12 14,8" fill="#fa0" fill-rule="evenodd"/>
            <path d="M2 18 Q 10 10 18 18 T 34 18 C 36 12 38 12 38 18 Z" fill="#808" stroke="#fff"
                  stroke-width="0.5"/>
            <line x1="1" y1="1" x2="39" y2="1" stroke="#333" stroke-linecap="round" stroke-width="2"/>"##,
    );
    assert_scene_matches_raster(&document, style(1.0), 2);
}

#[test]
fn stroke_properties_and_paint_order_draw_like_the_raster() {
    let document = document(
        r##"<polyline points="4,16 12,4 20,16 28,4 36,16" fill="none" stroke="#c20"
                      stroke-width="4" stroke-linejoin="round" stroke-linecap="square"/>
            <path d="M6 6 L18 6 L18 18 Z" fill="#06c" stroke="#fc0" stroke-width="4"
                  stroke-linejoin="bevel" paint-order="stroke"/>
            <path d="M24 6 L36 6 L24 18 Z" fill="none" stroke="#0a0" stroke-width="3"
                  stroke-miterlimit="1.5" stroke-linejoin="miter"/>"##,
    );
    assert_scene_matches_raster(&document, style(1.0), 2);
}

#[test]
fn view_box_and_aspect_ratio_are_folded_into_the_transforms() {
    for aspect_ratio in [
        "",
        r#"preserveAspectRatio="xMinYMin meet""#,
        r#"preserveAspectRatio="xMaxYMax meet""#,
        r#"preserveAspectRatio="xMidYMid slice""#,
        r#"preserveAspectRatio="none""#,
    ] {
        let document = document_with_view_box(
            aspect_ratio,
            r##"<rect x="1" y="1" width="8" height="5" fill="#d33"/>
                <circle cx="5" cy="7" r="2.5" fill="#26d" stroke="#000" stroke-width="0.4"/>"##,
        );
        assert_scene_matches_raster(&document, style(1.0), 2);
    }
}

#[test]
fn a_view_box_without_a_size_scales_to_the_viewport() {
    let document = document_with_view_box("", r##"<rect width="5" height="5" fill="#d33"/>"##);
    let scene = document
        .scene(
            SvgViewport {
                width: 100.0,
                height: 50.0,
            },
            style(1.0),
        )
        .unwrap();
    assert_eq!(scene.size, (100.0, 50.0));
    // The box is fitted by the transforms, so the geometry keeps its own
    // coordinates.
    let [path] = paths(&scene.root)[..] else {
        panic!("one path");
    };
    assert_eq!(
        path.segments[..2],
        [
            PathSegment::MoveTo((0.0, 0.0)),
            PathSegment::LineTo((5.0, 0.0))
        ]
    );
    let to_viewport = groups(&scene.root)
        .into_iter()
        .fold(PaintTransform::IDENTITY, |outer, group| {
            outer * group.transform
        });
    // 10 user units across 50 pixels of height, centered in the 100 wide box.
    assert_eq!(to_viewport.apply((0.0, 0.0)), (25.0, 0.0));
    assert_eq!(to_viewport.apply((10.0, 10.0)), (75.0, 50.0));
}

#[test]
fn a_fractional_viewport_is_kept_exactly() {
    let document = document_with_view_box("", r##"<rect width="10" height="10" fill="#d33"/>"##);
    let scene = document
        .scene(
            SvgViewport {
                width: 12.5,
                height: 12.5,
            },
            style(1.0),
        )
        .unwrap();
    assert_eq!(scene.size, (12.5, 12.5));
    let to_viewport = groups(&scene.root)
        .into_iter()
        .fold(PaintTransform::IDENTITY, |outer, group| {
            outer * group.transform
        });
    assert_eq!(to_viewport.apply((10.0, 10.0)), (12.5, 12.5));
}

#[test]
fn nested_transforms_draw_like_the_raster() {
    let document = document(
        r##"<g transform="translate(20 10) rotate(30)">
              <g transform="scale(1.5 0.8) skewX(10)">
                <rect x="-6" y="-4" width="12" height="8" fill="#a3c"/>
              </g>
              <rect x="4" y="-2" width="6" height="4" fill="#3ac" transform="matrix(1 0.2 -0.1 1 1 2)"/>
            </g>"##,
    );
    assert_scene_matches_raster(&document, style(1.0), 2);
}

#[test]
fn use_and_switch_are_resolved_by_the_builder() {
    let document = document(
        r##"<defs><rect id="r" width="6" height="6" fill="#d33"/>
              <g id="g"><circle cx="1" cy="1" r="1" fill="#36d"/></g></defs>
            <use href="#r" x="2" y="2"/><use href="#r" x="12" y="6" fill="#0a0"/>
            <use href="#g" transform="translate(28 4) scale(4)"/>
            <switch><text systemLanguage="xx">hidden</text><rect x="2" y="14" width="10" height="4" fill="#fa0"/></switch>"##,
    );
    assert_scene_matches_raster(&document, style(1.0), 2);
}

#[test]
fn opacities_draw_like_the_raster() {
    let document = document(
        r##"<g opacity="0.5">
              <rect x="2" y="2" width="20" height="12" fill="#e11" fill-opacity="0.8"/>
              <rect x="10" y="6" width="20" height="12" fill="#14c" stroke="#000" stroke-opacity="0.4"
                    stroke-width="3"/>
            </g>
            <g opacity="0.25"><g opacity="0.5"><rect x="30" y="2" width="8" height="8" fill="#0a0"/></g></g>"##,
    );
    assert_scene_matches_raster(&document, style(1.0), 3);
    let scene = document.scene(VIEWPORT, style(1.0)).unwrap();
    let opacities: Vec<f32> = groups(&scene.root)
        .iter()
        .map(|group| group.opacity)
        .collect();
    assert!(opacities.contains(&0.5) && opacities.contains(&0.25));
}

// ── Gradients ───────────────────────────────────────────────

const STOPS: &str = r##"<stop offset="0" stop-color="#e11"/>
    <stop offset="0.5" stop-color="#1c4" stop-opacity="0.5"/>
    <stop offset="1" stop-color="#14c"/>"##;

fn gradient_paint(scene: &VectorImage) -> &Paint {
    let [path] = paths(&scene.root)[..] else {
        panic!("one path");
    };
    &path.fill.as_ref().expect("a fill").paint
}

#[test]
fn linear_gradients_draw_like_the_raster() {
    for spread in ["pad", "reflect", "repeat"] {
        let document = document(&format!(
            r##"<defs><linearGradient id="g" x1="5" y1="2" x2="15" y2="6" gradientUnits="userSpaceOnUse"
                    spreadMethod="{spread}">{STOPS}</linearGradient></defs>
                <rect width="40" height="20" fill="url(#g)"/>"##
        ));
        assert_scene_matches_raster(&document, style(1.0), 2);
    }
    // Bounding box units, a gradient transform and the fill opacity.
    let document = document(&format!(
        r##"<defs><linearGradient id="g" gradientTransform="rotate(30 .5 .5)">{STOPS}</linearGradient></defs>
            <rect x="4" y="2" width="30" height="16" fill="url(#g)" fill-opacity="0.7"/>"##
    ));
    assert_scene_matches_raster(&document, style(1.0), 2);
}

#[test]
fn the_linear_gradient_is_resolved_to_the_space_of_its_path() {
    let document = document(&format!(
        r##"<defs><linearGradient id="g" gradientTransform="translate(1 2)" spreadMethod="reflect">{STOPS}</linearGradient></defs>
            <rect x="10" y="4" width="20" height="10" fill="url(#g)"/>"##
    ));
    let scene = document.scene(VIEWPORT, style(1.0)).unwrap();
    let Paint::LinearGradient(gradient) = gradient_paint(&scene) else {
        panic!("a linear gradient");
    };
    // The bounding box of the rect is folded into the transform.
    assert_eq!((gradient.start, gradient.end), ((0.0, 0.0), (1.0, 0.0)));
    assert_eq!(
        gradient.transform,
        PaintTransform::new(20.0, 0.0, 0.0, 10.0, 10.0, 4.0) * PaintTransform::translate(1.0, 2.0)
    );
    assert_eq!(gradient.spread, SpreadMethod::Reflect);
    let alphas: Vec<f32> = gradient.stops.iter().map(|stop| stop.color.alpha).collect();
    let offsets: Vec<f32> = gradient.stops.iter().map(|stop| stop.offset).collect();
    assert_eq!(
        (offsets, alphas),
        (vec![0.0, 0.5, 1.0], vec![1.0, 0.5, 1.0])
    );
    assert_eq!(
        gradient.stops[2].color,
        PaintColor::new(17.0 / 255.0, 68.0 / 255.0, 204.0 / 255.0, 1.0)
    );
}

#[test]
fn radial_gradients_draw_like_the_raster() {
    for spread in ["pad", "reflect", "repeat"] {
        let document = document(&format!(
            r##"<defs><radialGradient id="g" cx="20" cy="10" r="8" fx="16" fy="8" fr="1"
                    gradientUnits="userSpaceOnUse" spreadMethod="{spread}">{STOPS}</radialGradient></defs>
                <rect width="40" height="20" fill="url(#g)"/>"##
        ));
        assert_scene_matches_raster(&document, style(1.0), 2);
    }
    let document = document(&format!(
        r##"<defs><radialGradient id="g" gradientTransform="scale(1 0.5)">{STOPS}</radialGradient></defs>
            <ellipse cx="20" cy="10" rx="18" ry="9" fill="url(#g)"/>"##
    ));
    assert_scene_matches_raster(&document, style(1.0), 2);
}

#[test]
fn the_radial_gradient_keeps_its_focal_circle() {
    let document = document(&format!(
        r##"<defs><radialGradient id="g" cx="20" cy="10" r="8" fx="16" fy="8" fr="1"
                gradientUnits="userSpaceOnUse">{STOPS}</radialGradient></defs>
            <rect width="40" height="20" fill="url(#g)"/>"##
    ));
    let scene = document.scene(VIEWPORT, style(1.0)).unwrap();
    let Paint::RadialGradient(gradient) = gradient_paint(&scene) else {
        panic!("a radial gradient");
    };
    assert_eq!(
        (
            gradient.center,
            gradient.radius,
            gradient.focal,
            gradient.focal_radius
        ),
        ((20.0, 10.0), 8.0, (16.0, 8.0), 1.0)
    );
    assert_eq!(gradient.transform, PaintTransform::IDENTITY);
    assert_eq!(gradient.spread, SpreadMethod::Pad);
}

#[test]
fn strokes_take_gradients_too() {
    let document = document(&format!(
        r##"<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" x2="40">{STOPS}</linearGradient>
              <radialGradient id="r" gradientUnits="userSpaceOnUse" cx="30" cy="10" r="12">{STOPS}</radialGradient></defs>
            <rect x="3" y="3" width="20" height="14" fill="none" stroke="url(#g)" stroke-width="4"/>
            <circle cx="30" cy="10" r="6" fill="none" stroke="url(#r)" stroke-width="4"
                    stroke-opacity="0.6" stroke-dasharray="3 2"/>"##
    ));
    // The raster rounds the alpha of the stops to eight bits.
    assert_scene_matches_raster(&document, style(1.0), 3);
    let scene = document.scene(VIEWPORT, style(1.0)).unwrap();
    let found = paths(&scene.root);
    assert!(matches!(
        found[0].stroke.as_ref().unwrap().paint,
        Paint::LinearGradient(_)
    ));
    assert!(matches!(
        found[1].stroke.as_ref().unwrap().paint,
        Paint::RadialGradient(_)
    ));
}

#[test]
fn a_gradient_without_length_paints_its_last_stop() {
    let document = document(&format!(
        r##"<defs><linearGradient id="g" x1="3" y1="3" x2="3" y2="3" gradientUnits="userSpaceOnUse">{STOPS}</linearGradient></defs>
            <rect width="40" height="20" fill="url(#g)"/>"##
    ));
    let scene = document.scene(VIEWPORT, style(1.0)).unwrap();
    assert_eq!(
        *gradient_paint(&scene),
        Paint::Color(PaintColor::new(
            17.0 / 255.0,
            68.0 / 255.0,
            204.0 / 255.0,
            1.0
        ))
    );
    assert_scene_matches_raster(&document, style(1.0), 2);
}

#[test]
fn a_gradient_that_cannot_be_inverted_paints_nothing() {
    let tree = raw_tree(&format!(
        r##"<defs><linearGradient id="g" gradientTransform="matrix(1 2 2 4 0 0)">{STOPS}</linearGradient>
              <radialGradient id="r" gradientTransform="matrix(1 2 2 4 0 0)">{STOPS}</radialGradient></defs>
            <rect width="9" height="9" fill="url(#g)"/>
            <rect x="10" width="9" height="9" fill="url(#r)"/>
            <rect x="20" width="9" height="9" fill="none" stroke="url(#g)"/>
            <rect x="30" width="9" height="9" fill="none" stroke="url(#r)"/>
            <rect y="10" width="9" height="9" fill="url(#g)" stroke="#000"/>"##
    ));
    let scene = scene_of(&tree, 1.0).unwrap();
    // The first four have nothing left to paint, and the last one keeps its
    // stroke, like in the raster.
    let found = paths(&scene.root);
    assert_eq!(found.len(), 1);
    assert!(found[0].fill.is_none() && found[0].stroke.is_some());
}

// ── Clip paths ──────────────────────────────────────────────

#[test]
fn clip_paths_draw_like_the_raster() {
    let bodies = [
        // A plain clip, and the fill rule of its shapes.
        r##"<defs><clipPath id="c"><rect x="4" y="2" width="14" height="10"/></clipPath></defs>
            <rect width="40" height="20" fill="#e11" clip-path="url(#c)"/>"##,
        r##"<defs><clipPath id="c" clip-rule="evenodd"><path clip-rule="evenodd"
              d="M4 2 H30 V16 H4 Z M10 6 H24 V12 H10 Z"/></clipPath></defs>
            <rect width="40" height="20" fill="#14c" clip-path="url(#c)"/>"##,
        // Bounding box units, a clip transform and several shapes.
        r##"<defs><clipPath id="c" clipPathUnits="objectBoundingBox" transform="rotate(10)">
              <circle cx="0.3" cy="0.5" r="0.3"/><rect x="0.5" y="0.2" width="0.4" height="0.5"/>
            </clipPath></defs>
            <rect x="2" y="2" width="36" height="16" fill="#1c4" clip-path="url(#c)"/>"##,
        // A clip on a group with its own transform, and a clip that is clipped.
        r##"<defs><clipPath id="inner"><rect x="0" y="0" width="12" height="20"/></clipPath>
              <clipPath id="c" clip-path="url(#inner)"><circle cx="10" cy="10" r="9"/></clipPath></defs>
            <g transform="translate(6 0) scale(1.5 1)" clip-path="url(#c)">
              <rect width="30" height="20" fill="#e11"/>
              <rect x="10" width="30" height="20" fill="#14c" fill-opacity="0.5"/>
            </g>"##,
        // A shape inside a clip that carries a clip of its own.
        r##"<defs><clipPath id="inner"><rect x="12" y="0" width="10" height="20"/></clipPath>
              <clipPath id="c"><rect x="2" y="2" width="30" height="14" clip-path="url(#inner)"/>
                <circle cx="30" cy="10" r="6"/></clipPath></defs>
            <rect width="40" height="20" fill="#e11" clip-path="url(#c)"/>"##,
        // A clip together with the opacity of the group.
        r##"<defs><clipPath id="c"><rect x="4" y="2" width="20" height="14"/></clipPath></defs>
            <g opacity="0.5" clip-path="url(#c)"><rect width="20" height="20" fill="#e11"/>
              <rect x="10" width="20" height="20" fill="#14c"/></g>"##,
    ];
    for body in bodies {
        assert_scene_matches_raster(&document(body), style(1.0), 3);
    }
}

#[test]
fn a_clip_is_part_of_its_group() {
    let document = document(
        r##"<defs><clipPath id="inner"><rect width="12" height="20"/></clipPath>
              <clipPath id="c" clip-path="url(#inner)"><circle cx="10" cy="10" r="9"/></clipPath></defs>
            <g clip-path="url(#c)" transform="translate(3 0)"><rect width="30" height="20" fill="#e11"/></g>"##,
    );
    let scene = document.scene(VIEWPORT, style(1.0)).unwrap();
    let [Node::Group(group)] = &scene.root.children[..] else {
        panic!("one group");
    };
    assert_eq!(group.transform, PaintTransform::translate(3.0, 0.0));
    assert_eq!(group.opacity, 1.0);
    let clip = group.clip.as_ref().expect("a clip");
    let [Node::Path(circle)] = &clip.root.children[..] else {
        panic!("one shape");
    };
    assert_eq!(circle.fill.as_ref().unwrap().rule, FillRule::NonZero);
    let chained = clip.clip.as_ref().expect("a chained clip");
    assert!(chained.clip.is_none());
    assert_eq!(chained.root.children.len(), 1);
    // Both clips and their shapes are nodes of the scene.
    assert_eq!(scene.node_count(), 1 + 1 + 1 + 2 + 2);
}

#[test]
fn a_clip_with_nothing_to_clip_leaves_no_group() {
    let document = document(
        r##"<defs><clipPath id="c"><rect width="5" height="5"/></clipPath></defs>
            <g clip-path="url(#c)"><rect width="5" height="5" display="none"/></g>"##,
    );
    let scene = document.scene(VIEWPORT, style(1.0)).unwrap();
    assert!(scene.root.children.is_empty());
    assert_eq!(scene.node_count(), 1);
}

// ── The host's inputs ───────────────────────────────────────

#[test]
fn the_host_opacity_is_never_part_of_the_scene() {
    let body = r##"<rect width="20" height="10" fill="#e11"/>"##;
    let plain = document(body);
    let boxed = document_with_view_box("", body);
    for document in [&plain, &boxed] {
        let opaque = document.scene(VIEWPORT, style(1.0)).unwrap();
        for opacity in [0.5, 0.25] {
            let scene = document.scene(VIEWPORT, style(opacity)).unwrap();
            assert_eq!(scene, opaque, "host opacity {opacity}");
            assert!(groups(&scene.root).iter().all(|group| group.opacity == 1.0));
        }
        // Whatever `neutralize_root_opacity` says: it is implied.
        let scene = document
            .scene(
                VIEWPORT,
                SvgRootStyle {
                    neutralize_root_opacity: false,
                    ..style(0.5)
                },
            )
            .unwrap();
        assert_eq!(scene, opaque);
    }
}

#[test]
fn inherited_opacity_sees_the_host_opacity() {
    let document = document(r##"<rect width="20" height="10" fill="#e11" opacity="inherit"/>"##);
    let scene = document.scene(VIEWPORT, style(0.5)).unwrap();
    let opacities: Vec<f32> = groups(&scene.root)
        .iter()
        .map(|group| group.opacity)
        .collect();
    // The root group is left out of the host opacity; the child keeps it.
    assert_eq!(
        opacities.iter().filter(|opacity| **opacity == 0.5).count(),
        1
    );
    assert_scene_matches_raster(&document, style(0.5), 3);
}

#[test]
fn the_view_box_wrapper_is_not_taken_for_the_root_group() {
    let document = document_with_view_box(
        r#"width="40" height="20" opacity="0.7""#,
        r##"<g opacity="0.7"><rect width="5" height="5" fill="#e11"/></g>"##,
    );
    let scene = document.scene(VIEWPORT, style(0.7)).unwrap();
    // Only the element's own group is neutral; the author's group keeps 0.7.
    let opacities: Vec<f32> = groups(&scene.root)
        .iter()
        .map(|group| group.opacity)
        .collect();
    assert_eq!(
        opacities.iter().filter(|opacity| **opacity == 0.7).count(),
        1
    );
    assert_scene_matches_raster(&document, style(0.7), 3);
}

#[test]
fn a_transparent_host_leaves_an_empty_scene() {
    let document = document(r##"<rect width="20" height="10" fill="#e11"/>"##);
    let scene = document.scene(VIEWPORT, style(0.0)).unwrap();
    assert_eq!(scene, VectorImage::new((40.0, 20.0)));
}

#[test]
fn an_empty_document_needs_no_root_group() {
    let document = document("");
    for opacity in [1.0, 0.5] {
        let scene = document.scene(VIEWPORT, style(opacity)).unwrap();
        assert!(scene.root.children.is_empty());
        assert_eq!(scene.node_count(), 1);
    }
}

#[test]
fn current_color_follows_the_host_color() {
    let document = document(r##"<rect width="20" height="10" fill="currentColor"/>"##);
    let scene = document
        .scene(
            VIEWPORT,
            SvgRootStyle {
                inherited_color: [0, 0, 255, 255],
                ..SvgRootStyle::default()
            },
        )
        .unwrap();
    let [path] = paths(&scene.root)[..] else {
        panic!("one path");
    };
    let Some(Fill {
        paint: Paint::Color(color),
        ..
    }) = &path.fill
    else {
        panic!("a solid fill");
    };
    assert_eq!(
        (color.red, color.green, color.blue, color.alpha),
        (0.0, 0.0, 1.0, 1.0)
    );
}

#[test]
fn a_hidden_root_still_shows_visible_descendants() {
    let document = document(
        r##"<rect width="10" height="10" fill="#e11"/>
            <g visibility="visible"><rect x="20" width="10" height="10" fill="#14c"/></g>"##,
    );
    let hidden = SvgRootStyle {
        visible: false,
        ..SvgRootStyle::default()
    };
    let scene = document.scene(VIEWPORT, hidden).unwrap();
    assert_eq!(paths(&scene.root).len(), 1);
    assert_scene_matches_raster(&document, hidden, 2);
}

#[test]
fn resolved_styles_apply_like_they_do_to_the_raster() {
    let document = document(
        r##"<rect width="10" height="10" fill="currentColor"/>
            <rect x="20" width="10" height="10" fill="#14c"/>"##,
    );
    let elements = [SvgElementStyle {
        element_index: 2,
        declarations: "display:none",
    }];
    let resolved = SvgResolvedStyles {
        root_color: [200, 0, 0, 255],
        font: SvgRootFont {
            size: 16.0,
            family: "serif",
            weight: 400.0,
            style: "normal",
        },
        elements: &elements,
    };
    let scene = document
        .scene_with_resolved_styles(VIEWPORT, SvgRootStyle::default(), resolved)
        .unwrap();
    // The second rect is hidden by its resolved declaration.
    let found = paths(&scene.root);
    assert_eq!(found.len(), 1);
    let raster = document
        .rasterize_at_css_pixel_scale_with_resolved_styles(
            VIEWPORT,
            SvgRootStyle::default(),
            resolved,
            None,
        )
        .unwrap();
    assert_close(&render(&scene), &raster.rgba, 2);
    // A malformed font is the same error as for the raster.
    let bad = SvgResolvedStyles {
        font: SvgRootFont {
            size: -1.0,
            ..resolved.font
        },
        ..resolved
    };
    assert!(matches!(
        document.scene_with_resolved_styles(VIEWPORT, SvgRootStyle::default(), bad),
        Err(SvgSceneError::Svg(SvgError::InvalidDocument(_)))
    ));
}

// ── The shape of the scene ──────────────────────────────────

#[test]
fn paths_carry_their_paint_properties() {
    let document = document(
        r##"<path d="M1 1 L9 1 Q 9 5 5 5 C 3 5 1 7 1 9 Z" fill="#102030" fill-opacity="0.5"
                  fill-rule="evenodd" stroke="#405060" stroke-opacity="0.25" stroke-width="2"
                  stroke-linecap="round" stroke-linejoin="miter-clip" stroke-miterlimit="3"
                  stroke-dasharray="3 1" stroke-dashoffset="1" paint-order="stroke"/>"##,
    );
    let scene = document.scene(VIEWPORT, style(1.0)).unwrap();
    let [path] = paths(&scene.root)[..] else {
        panic!("one path");
    };
    assert_eq!(
        path.segments,
        [
            PathSegment::MoveTo((1.0, 1.0)),
            PathSegment::LineTo((9.0, 1.0)),
            PathSegment::QuadTo {
                control: (9.0, 5.0),
                to: (5.0, 5.0)
            },
            PathSegment::CubicTo {
                control1: (3.0, 5.0),
                control2: (1.0, 7.0),
                to: (1.0, 9.0)
            },
            PathSegment::Close,
        ]
    );
    assert_eq!(path.paint_order, PaintOrder::StrokeThenFill);
    let fill = path.fill.as_ref().unwrap();
    assert_eq!((fill.opacity, fill.rule), (0.5, FillRule::EvenOdd));
    assert_eq!(
        fill.paint,
        Paint::Color(PaintColor::new(
            16.0 / 255.0,
            32.0 / 255.0,
            48.0 / 255.0,
            1.0
        ))
    );
    let stroke = path.stroke.as_ref().unwrap();
    assert_eq!(
        (
            stroke.opacity,
            stroke.width,
            stroke.cap,
            stroke.join,
            stroke.miter_limit
        ),
        (0.25, 2.0, LineCap::Round, LineJoin::MiterClip, 3.0)
    );
    assert_eq!(stroke.dash, Some(Dash::new(vec![3.0, 1.0], 1.0)));
}

#[test]
fn shape_rendering_decides_whether_edges_are_smoothed() {
    let body = |rendering: &str| {
        format!(
            r##"<path d="M1.5 1.5 L18.5 3.5 L10.5 17.5 Z" fill="#e11" stroke="#14c" stroke-width="2.5"
                  shape-rendering="{rendering}"/>"##
        )
    };
    let smooth = document(&body("geometricPrecision"));
    let scene = smooth.scene(VIEWPORT, style(1.0)).unwrap();
    assert!(paths(&scene.root)[0].anti_alias);
    assert_scene_matches_raster(&smooth, style(1.0), 2);
    let smooth_picture = render(&scene);
    for rendering in ["crispEdges", "optimizeSpeed"] {
        let hard = document(&body(rendering));
        let scene = hard.scene(VIEWPORT, style(1.0)).unwrap();
        assert!(!paths(&scene.root)[0].anti_alias);
        assert_scene_matches_raster(&hard, style(1.0), 2);
        // Hard edges are a different picture, so the comparison above can
        // only pass when the flag reaches the drawing.
        assert_ne!(render(&scene), smooth_picture);
    }
}

#[test]
fn line_joins_and_solid_strokes_are_mapped() {
    for (value, join) in [
        ("miter", LineJoin::Miter),
        ("round", LineJoin::Round),
        ("bevel", LineJoin::Bevel),
    ] {
        let document = document(&format!(
            r##"<path d="M1 1 L9 1 L9 9" fill="none" stroke="#000" stroke-linejoin="{value}"/>"##
        ));
        let scene = document.scene(VIEWPORT, style(1.0)).unwrap();
        let stroke = paths(&scene.root)[0].stroke.as_ref().unwrap();
        assert_eq!(
            (stroke.join, stroke.cap, stroke.dash.is_none()),
            (join, LineCap::Butt, true)
        );
    }
}

#[test]
fn what_draws_nothing_is_left_out() {
    let document = document(
        r##"<g><rect width="5" height="5" visibility="hidden" fill="#000"/></g>
            <rect width="5" height="5" fill="none"/>
            <rect width="5" height="5" display="none" fill="#000"/>
            <g opacity="0.5"><circle r="0" fill="#000"/></g>"##,
    );
    let scene = document.scene(VIEWPORT, style(1.0)).unwrap();
    assert!(scene.root.children.is_empty());
    assert_eq!(scene.node_count(), 1);
}

#[test]
fn scenes_are_deterministic_and_report_their_cost() {
    let document = document(r##"<rect width="20" height="10" fill="#e11"/>"##);
    let first = document.scene(VIEWPORT, style(1.0)).unwrap();
    assert_eq!(first, document.scene(VIEWPORT, style(1.0)).unwrap());
    assert!(first.node_count() >= 2);
    assert!(first.estimated_heap_bytes() > VectorImage::new((40.0, 20.0)).estimated_heap_bytes());
}

// ── Fallback ────────────────────────────────────────────────

#[test]
fn content_outside_the_subset_asks_for_the_raster() {
    let cases = [
        (
            r##"<defs><pattern id="p" width="4" height="4" patternUnits="userSpaceOnUse"><rect width="2" height="2"/></pattern></defs>
                <rect width="10" height="10" fill="url(#p)"/>"##,
            SvgSceneFeature::Pattern,
        ),
        (
            r##"<defs><pattern id="p" width="4" height="4" patternUnits="userSpaceOnUse"><rect width="2" height="2"/></pattern></defs>
                <rect width="10" height="10" fill="none" stroke="url(#p)" stroke-width="2"/>"##,
            SvgSceneFeature::Pattern,
        ),
        (
            r##"<defs><mask id="m"><rect width="5" height="5" fill="#fff"/></mask></defs>
                <rect width="10" height="10" mask="url(#m)"/>"##,
            SvgSceneFeature::Mask,
        ),
        (
            r##"<rect width="10" height="10" style="mix-blend-mode:multiply"/>"##,
            SvgSceneFeature::BlendMode,
        ),
    ];
    for (body, feature) in cases {
        let error = scene_error(&document(body));
        assert!(
            matches!(error, SvgSceneError::Unsupported(found) if found == feature),
            "{feature:?}: {error}"
        );
        assert!(error.falls_back_to_raster());
    }
}

#[test]
fn unused_definitions_do_not_force_the_fallback() {
    let document = document(
        r##"<defs><linearGradient id="g"><stop offset="0" stop-color="#000"/></linearGradient>
              <clipPath id="c"><rect width="5" height="5"/></clipPath></defs>
            <rect width="10" height="10" fill="#e11"/>"##,
    );
    assert!(document.scene(VIEWPORT, style(1.0)).is_ok());
}

/// A tree as the parser alone builds it. A document would not hand some of
/// these over: it rejects filters, and it resolves no image reference.
fn raw_tree(body: &str) -> usvg::Tree {
    usvg::Tree::from_str(
        &format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">{body}</svg>"##
        ),
        &usvg::Options::default(),
    )
    .unwrap()
}

fn scene_of(tree: &usvg::Tree, host_opacity: f32) -> Result<VectorImage, SvgSceneError> {
    scene_from_tree(tree, VIEWPORT, host_opacity, SceneLimits::default())
}

#[test]
fn nodes_the_document_parser_never_hands_over_are_not_drawn() {
    let image = raw_tree(
        r##"<image width="10" height="10"
            href="data:image/svg+xml;utf8,&lt;svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'&gt;&lt;rect width='1' height='1'/&gt;&lt;/svg&gt;"/>"##,
    );
    assert!(matches!(
        scene_of(&image, 1.0),
        Err(SvgSceneError::Unsupported(SvgSceneFeature::Image))
    ));
    // A filter is an error and not a fallback, like it is for the raster.
    let blurred = raw_tree(
        r##"<filter id="f"><feGaussianBlur stdDeviation="1"/></filter>
            <rect width="5" height="5" filter="url(#f)"/>"##,
    );
    let error = scene_of(&blurred, 1.0).unwrap_err();
    assert!(matches!(
        error,
        SvgSceneError::Svg(SvgError::UnsupportedFilterEffects)
    ));
    assert!(!error.falls_back_to_raster());
}

#[test]
fn the_host_group_is_told_from_the_content_by_its_place_and_opacity() {
    let rects = raw_tree(r##"<rect width="5" height="5"/><rect x="6" width="5" height="5"/>"##);
    // Not one group.
    assert_eq!(host_group_depth(rects.root(), 0.5), None);
    // One group of an opacity that is neither the host's nor one.
    let dimmer = raw_tree(r##"<g opacity="0.3"><rect width="5" height="5"/></g>"##);
    assert_eq!(host_group_depth(dimmer.root(), 0.5), None);
    assert_eq!(host_group_depth(dimmer.root(), 0.3), Some(1));
    // Wrappers of opacity one lead to it, up to the search depth.
    let wrapped = |wrappers: usize| {
        raw_tree(&format!(
            r##"{}<g opacity="0.5"><rect width="5" height="5"/></g>{}"##,
            r##"<g transform="translate(1)">"##.repeat(wrappers),
            "</g>".repeat(wrappers),
        ))
    };
    assert_eq!(host_group_depth(wrapped(1).root(), 0.5), Some(2));
    assert_eq!(
        host_group_depth(wrapped(ROOT_GROUP_SEARCH_DEPTH).root(), 0.5),
        None
    );
}

#[test]
fn a_tree_without_the_host_group_asks_for_the_raster() {
    let dimmer = raw_tree(r##"<g opacity="0.3"><rect width="5" height="5"/></g>"##);
    assert!(matches!(
        scene_of(&dimmer, 0.5),
        Err(SvgSceneError::Unsupported(SvgSceneFeature::RootOpacity))
    ));
    // The group that carries the host opacity is found and left out.
    let scene = scene_of(&dimmer, 0.3).unwrap();
    // Only the root of the scene is left, and the path is its child.
    assert_eq!(groups(&scene.root).len(), 1);
    assert_eq!(paths(&scene.root).len(), 1);
    assert_eq!(scene.root.opacity, 1.0);
    // Nothing is drawn, so nothing has to be told apart.
    let empty = raw_tree(r##"<rect width="5" height="5" fill="none"/>"##);
    assert!(scene_of(&empty, 0.5).unwrap().root.children.is_empty());
}

fn many_selectors() -> SvgDocument {
    // More selector matches than the style preparation accepts.
    let source = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><style>{}</style>{}</svg>"##,
        "rect { fill:red }".repeat(257),
        r##"<rect width="1" height="1"/>"##.repeat(256),
    );
    SvgDocument::parse(source.as_bytes()).unwrap()
}

#[test]
fn a_hidden_root_keeps_an_empty_scene_where_a_visible_one_is_rejected() {
    let document = many_selectors();
    let hidden = SvgRootStyle {
        visible: false,
        ..SvgRootStyle::default()
    };
    let scene = document.scene(VIEWPORT, hidden).unwrap();
    assert!(scene.root.children.is_empty());
    let raster = document
        .rasterize_at_css_pixel_scale(VIEWPORT, hidden, None)
        .unwrap();
    assert!(raster.rgba.iter().all(|&byte| byte == 0));
    // The same document is rejected, as the raster rejects it.
    let error = document.scene(VIEWPORT, style(1.0)).unwrap_err();
    assert!(matches!(
        error,
        SvgSceneError::Svg(SvgError::InvalidDocument(ref message))
            if message.contains("selector freezing resource limit")
    ));
    assert!(!error.falls_back_to_raster());
}

// ── Limits and invalid requests ─────────────────────────────

fn limited(document: &SvgDocument, limits: SceneLimits) -> Result<VectorImage, SvgSceneError> {
    document.build_scene(VIEWPORT, style(1.0), None, limits)
}

#[test]
fn budgets_are_errors_and_not_fallbacks() {
    let wide = document(
        r##"<rect width="1" height="1"/><rect width="1" height="1"/><rect width="1" height="1"/>"##,
    );
    let deep = document(
        r##"<g opacity="0.9"><g opacity="0.9"><g opacity="0.9"><rect width="1" height="1"/></g></g></g>"##,
    );
    let cases = [
        (
            &wide,
            SceneLimits {
                nodes: 3,
                ..SceneLimits::default()
            },
            SvgSceneLimit::Nodes,
            3,
        ),
        (
            &wide,
            SceneLimits {
                segments: 10,
                ..SceneLimits::default()
            },
            SvgSceneLimit::PathSegments,
            10,
        ),
        (
            &deep,
            SceneLimits {
                group_depth: 2,
                ..SceneLimits::default()
            },
            SvgSceneLimit::GroupDepth,
            2,
        ),
    ];
    for (document, limits, limit, max) in cases {
        let error = limited(document, limits).unwrap_err();
        assert!(
            matches!(error, SvgSceneError::LimitExceeded { limit: found, max: found_max }
                if found == limit && found_max == max),
            "{limit:?}: {error}"
        );
        assert!(!error.falls_back_to_raster());
    }
    // The same documents fit their budgets exactly.
    assert!(
        limited(
            &wide,
            SceneLimits {
                nodes: 4,
                segments: 15,
                ..SceneLimits::default()
            }
        )
        .is_ok()
    );
    assert!(
        limited(
            &deep,
            SceneLimits {
                group_depth: 3,
                ..SceneLimits::default()
            }
        )
        .is_ok()
    );
}

#[test]
fn gradient_and_clip_budgets_are_errors_and_not_fallbacks() {
    let two_gradients = document(&format!(
        r##"<defs><linearGradient id="g">{STOPS}</linearGradient></defs>
            <rect width="5" height="5" fill="url(#g)"/><rect x="6" width="5" height="5" fill="url(#g)"/>"##
    ));
    let chained = document(
        r##"<defs><clipPath id="inner"><rect width="9" height="9"/></clipPath>
              <clipPath id="c" clip-path="url(#inner)"><rect width="9" height="9"/></clipPath></defs>
            <rect width="10" height="10" clip-path="url(#c)"/>"##,
    );
    let nested = document(
        r##"<defs><clipPath id="inner"><rect width="9" height="9"/></clipPath>
              <clipPath id="c"><rect width="9" height="9" clip-path="url(#inner)"/></clipPath></defs>
            <rect width="10" height="10" clip-path="url(#c)"/>"##,
    );
    let exceeded = |document: &SvgDocument,
                    limits: SceneLimits,
                    expected: (SvgSceneLimit, usize)| {
        let error = limited(document, limits).unwrap_err();
        assert!(
            matches!(error, SvgSceneError::LimitExceeded { limit, max } if (limit, max) == expected),
            "{expected:?}: {error}"
        );
        assert!(!error.falls_back_to_raster());
    };
    let limits = SceneLimits::default();
    // Three stops in a gradient, six in the scene.
    exceeded(
        &two_gradients,
        SceneLimits {
            gradient_stops: 2,
            ..limits
        },
        (SvgSceneLimit::GradientStops, 2),
    );
    exceeded(
        &two_gradients,
        SceneLimits {
            scene_gradient_stops: 5,
            ..limits
        },
        (SvgSceneLimit::SceneGradientStops, 5),
    );
    // Two clips on top of each other, and a clip inside a clip.
    for document in [&chained, &nested] {
        exceeded(
            document,
            SceneLimits {
                clip_depth: 1,
                ..limits
            },
            (SvgSceneLimit::ClipDepth, 1),
        );
        assert!(
            limited(
                document,
                SceneLimits {
                    clip_depth: 2,
                    ..limits
                }
            )
            .is_ok()
        );
    }
    // The same documents fit their budgets exactly.
    assert!(
        limited(
            &two_gradients,
            SceneLimits {
                gradient_stops: 3,
                scene_gradient_stops: 6,
                ..limits
            }
        )
        .is_ok()
    );
    // The nodes of a clip are charged to the budget of the scene.
    for document in [&chained, &nested] {
        let nodes = document.scene(VIEWPORT, style(1.0)).unwrap().node_count();
        assert!(limited(document, SceneLimits { nodes, ..limits }).is_ok());
        exceeded(
            document,
            SceneLimits {
                nodes: nodes - 1,
                ..limits
            },
            (SvgSceneLimit::Nodes, nodes - 1),
        );
    }
}

#[test]
fn invalid_requests_are_svg_errors() {
    let document = document(r##"<rect width="1" height="1"/>"##);
    for (width, height) in [
        (0.0, 10.0),
        (10.0, -1.0),
        (f32::NAN, 10.0),
        (10.0, f32::INFINITY),
        (1.0e12, 10.0),
    ] {
        let error = document
            .scene(SvgViewport { width, height }, style(1.0))
            .unwrap_err();
        assert!(
            matches!(error, SvgSceneError::Svg(SvgError::InvalidViewport)),
            "{width}x{height}: {error}"
        );
        assert!(!error.falls_back_to_raster());
    }
    for opacity in [-0.1, 1.5, f32::NAN] {
        assert!(matches!(
            document.scene(VIEWPORT, style(opacity)),
            Err(SvgSceneError::Svg(SvgError::InvalidOpacity))
        ));
    }
}

#[test]
fn errors_describe_themselves() {
    let svg = SvgSceneError::from(SvgError::InvalidViewport);
    assert_eq!(svg.to_string(), SvgError::InvalidViewport.to_string());
    assert!(std::error::Error::source(&svg).is_some());
    let unsupported = SvgSceneError::Unsupported(SvgSceneFeature::Mask);
    assert_eq!(
        unsupported.to_string(),
        "SVG vector rendering does not cover masks"
    );
    assert!(std::error::Error::source(&unsupported).is_none());
    let limit = SvgSceneError::LimitExceeded {
        limit: SvgSceneLimit::PathSegments,
        max: 7,
    };
    assert_eq!(
        limit.to_string(),
        "SVG scene exceeds the limit of 7 path segments"
    );
    for feature in [
        SvgSceneFeature::Pattern,
        SvgSceneFeature::BlendMode,
        SvgSceneFeature::Image,
        SvgSceneFeature::RootOpacity,
    ] {
        assert!(!feature.name().is_empty());
    }
    for limit in [
        SvgSceneLimit::Nodes,
        SvgSceneLimit::GroupDepth,
        SvgSceneLimit::ClipDepth,
        SvgSceneLimit::GradientStops,
        SvgSceneLimit::SceneGradientStops,
    ] {
        assert!(!limit.name().is_empty());
    }
}

#[test]
fn the_default_budgets_are_the_documented_ones() {
    let limits = SceneLimits::default();
    assert_eq!(
        (
            limits.nodes,
            limits.segments,
            limits.group_depth,
            limits.clip_depth,
            limits.gradient_stops,
            limits.scene_gradient_stops
        ),
        (1_000_000, 4_000_000, 256, 64, 1_024, 1_000_000)
    );
}
