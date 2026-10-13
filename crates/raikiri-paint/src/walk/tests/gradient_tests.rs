use super::*;

/// The computed values of a `div` with `style`.
fn computed_for(style: &str) -> ComputedValues {
    let mut document = Document::new();
    let id = document.append_element(
        Some(document.root_index()),
        "div",
        taffy::Style::default(),
        Some(style),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    cascade.computed[id].clone()
}

fn gradient_of(cv: &ComputedValues) -> Gradient {
    match &cv.background_layers()[0].image {
        BackgroundImage::Gradient(gradient) => (**gradient).clone(),
        other => panic!("expected a gradient, got {other:?}"),
    }
}

/// Rasterize a gradient layer over a `width` by `height` box at the origin.
fn raster_layer(style: &str, width: u32, height: u32) -> Vec<u8> {
    let cv = computed_for(style);
    let gradient = gradient_of(&cv);
    let area = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    let mut scene = Scene::new();
    assert!(gradient::paint_gradient_layer(
        &mut scene,
        &gradient,
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    ));
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        width,
        height,
    )
}

fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

#[test]
fn linear_gradient_hard_stops_split_the_box() {
    let rgba = raster_layer(
        "background-image: linear-gradient(to right, red 50%, blue 50%)",
        40,
        10,
    );
    assert_eq!(pixel(&rgba, 40, 5, 5), RED);
    assert_eq!(pixel(&rgba, 40, 35, 5), BLUE);
}

#[test]
fn linear_gradient_angle_and_corner_follow_the_gradient_line() {
    let rgba = raster_layer(
        "background-image: linear-gradient(180deg, red 50%, blue 50%)",
        10,
        40,
    );
    assert_eq!(pixel(&rgba, 10, 5, 5), RED);
    assert_eq!(pixel(&rgba, 10, 5, 35), BLUE);
    // The 50% line of a corner gradient runs through the other two corners.
    let rgba = raster_layer(
        "background-image: linear-gradient(to bottom right, red 50%, blue 50%)",
        40,
        20,
    );
    assert_eq!(pixel(&rgba, 40, 2, 2), RED);
    assert_eq!(pixel(&rgba, 40, 37, 17), BLUE);
    assert_eq!(pixel(&rgba, 40, 30, 2), RED);
    assert_eq!(pixel(&rgba, 40, 34, 4), BLUE);
}

#[test]
fn linear_gradient_pads_before_and_after_its_stops() {
    let rgba = raster_layer(
        "background-image: linear-gradient(to right, red 25%, blue 75%)",
        40,
        10,
    );
    assert_eq!(pixel(&rgba, 40, 2, 5), RED);
    assert_eq!(pixel(&rgba, 40, 38, 5), BLUE);
    let middle = pixel(&rgba, 40, 20, 5);
    assert!(middle[0] > 0 && middle[2] > 0, "{middle:?}");
}

#[test]
fn repeating_linear_gradient_repeats_its_stops() {
    let rgba = raster_layer(
        "background-image: repeating-linear-gradient(to right, red 0, red 10px, blue 10px, blue 20px)",
        40,
        10,
    );
    assert_eq!(pixel(&rgba, 40, 5, 5), RED);
    assert_eq!(pixel(&rgba, 40, 15, 5), BLUE);
    assert_eq!(pixel(&rgba, 40, 25, 5), RED);
    assert_eq!(pixel(&rgba, 40, 35, 5), BLUE);
}

#[test]
fn gradient_tiles_follow_background_size() {
    let rgba = raster_layer(
        "background-image: linear-gradient(to right, red 50%, blue 50%); background-size: 20px 20px",
        40,
        20,
    );
    assert_eq!(pixel(&rgba, 40, 5, 10), RED);
    assert_eq!(pixel(&rgba, 40, 15, 10), BLUE);
    assert_eq!(pixel(&rgba, 40, 25, 10), RED);
    assert_eq!(pixel(&rgba, 40, 35, 10), BLUE);
}

#[test]
fn radial_gradient_sizes_circles_and_ellipses() {
    let rgba = raster_layer(
        "background-image: radial-gradient(circle 10px, red 50%, blue 50%)",
        40,
        40,
    );
    assert_eq!(pixel(&rgba, 40, 20, 20), RED);
    assert_eq!(pixel(&rgba, 40, 28, 20), BLUE);
    // A closest-side ellipse in a wide box is wider than it is tall.
    let rgba = raster_layer(
        "background-image: radial-gradient(closest-side, red 80%, blue 80%)",
        80,
        20,
    );
    assert_eq!(pixel(&rgba, 80, 40, 10), RED);
    assert_eq!(pixel(&rgba, 80, 70, 10), RED);
    assert_eq!(pixel(&rgba, 80, 40, 1), BLUE);
}

#[test]
fn radial_extents_measure_sides_and_corners() {
    let tile = Rect::new(0.0, 0.0, 40.0, 20.0);
    let center = Point::new(10.0, 10.0);
    let radii = |style: &str| match gradient_of(&computed_for(style)) {
        Gradient::Radial(radial) => gradient::radial_radii(&radial, center, tile),
        other => panic!("expected radial, got {other:?}"),
    };
    assert_eq!(
        radii("background-image: radial-gradient(circle closest-side, red, blue)"),
        (10.0, 10.0)
    );
    assert_eq!(
        radii("background-image: radial-gradient(circle farthest-side, red, blue)"),
        (30.0, 30.0)
    );
    let (r, _) = radii("background-image: radial-gradient(circle farthest-corner, red, blue)");
    assert!((r - 30.0f64.hypot(10.0)).abs() < 1e-9);
    let (r, _) = radii("background-image: radial-gradient(circle closest-corner, red, blue)");
    assert!((r - 10.0f64.hypot(10.0)).abs() < 1e-9);
    assert_eq!(
        radii("background-image: radial-gradient(ellipse farthest-side, red, blue)"),
        (30.0, 10.0)
    );
    // A corner ellipse keeps the side extent's aspect ratio.
    let (rx, ry) = radii("background-image: radial-gradient(ellipse closest-corner, red, blue)");
    assert!((rx - 10.0 * 2f64.sqrt()).abs() < 1e-9);
    assert!((ry - 10.0 * 2f64.sqrt()).abs() < 1e-9);
    let (rx, ry) = radii("background-image: radial-gradient(red, blue)");
    assert!((rx / ry - 3.0).abs() < 1e-9);
    assert!((rx - 30.0 * 2f64.sqrt()).abs() < 1e-9);
    assert_eq!(
        radii("background-image: radial-gradient(ellipse 50% 5px, red, blue)"),
        (20.0, 5.0)
    );
}

#[test]
fn degenerate_gradients_paint_a_switch_or_an_average() {
    // A repeating gradient with no period paints the average stop color.
    let rgba = raster_layer(
        "background-image: repeating-linear-gradient(to right, red 10px, blue 10px)",
        40,
        10,
    );
    let average = pixel(&rgba, 40, 5, 5);
    assert!((127..=128).contains(&average[0]) && (127..=128).contains(&average[2]));
    // Stops sharing one position switch colors there.
    let rgba = raster_layer(
        "background-image: linear-gradient(to right, red 20px, blue 20px)",
        40,
        10,
    );
    assert_eq!(pixel(&rgba, 40, 10, 5), RED);
    assert_eq!(pixel(&rgba, 40, 30, 5), BLUE);
    // The average weighs each pair of neighboring stops equally.
    let rgba = raster_layer(
        "background-image: repeating-linear-gradient(red 0, white 0, blue 0)",
        10,
        10,
    );
    assert_eq!(pixel(&rgba, 10, 5, 5), [191, 128, 191, 255]);
}

#[test]
fn degenerate_radial_gradients_follow_their_thin_ending_shape() {
    // A zero-radius circle shows its last color outside the center.
    let rgba = raster_layer(
        "background-image: radial-gradient(circle 0px, red, blue)",
        40,
        40,
    );
    assert_eq!(pixel(&rgba, 40, 30, 30), BLUE);
    assert_eq!(pixel(&rgba, 40, 21, 20), BLUE);
    // Lengths keep their size, so the circle still grows to them.
    let rgba = raster_layer(
        "background-image: radial-gradient(circle 0px, red 10px, blue 10px)",
        40,
        40,
    );
    assert_eq!(pixel(&rgba, 40, 22, 22), RED);
    assert_eq!(pixel(&rgba, 40, 35, 20), BLUE);
    // A zero-width ellipse is a horizontal gradient mirrored at the center.
    let rgba = raster_layer(
        "background-image: radial-gradient(0px 20px, red 10px, blue 10px)",
        40,
        40,
    );
    for y in [2, 20, 37] {
        assert_eq!(pixel(&rgba, 40, 15, y), RED);
        assert_eq!(pixel(&rgba, 40, 25, y), RED);
        assert_eq!(pixel(&rgba, 40, 5, y), BLUE);
        assert_eq!(pixel(&rgba, 40, 35, y), BLUE);
    }
    // A zero-height ellipse shows its last color, or the average of a
    // repeating one.
    let rgba = raster_layer(
        "background-image: radial-gradient(20px 0px, red 10px, blue 10px)",
        40,
        40,
    );
    assert_eq!(pixel(&rgba, 40, 20, 20), BLUE);
    let rgba = raster_layer(
        "background-image: repeating-radial-gradient(20px 0px, red 0px, blue 10px)",
        40,
        40,
    );
    let average = pixel(&rgba, 40, 20, 20);
    assert!((127..=128).contains(&average[0]) && (127..=128).contains(&average[2]));
    // The average of a period weighs each pair by the distance it spans.
    let rgba = raster_layer(
        "background-image: repeating-radial-gradient(20px 0px, red 0px, white 1px, blue 10px)",
        40,
        40,
    );
    let [r, g, b, a] = pixel(&rgba, 40, 20, 20);
    assert!(r.abs_diff(140) <= 1 && g.abs_diff(128) <= 1 && b.abs_diff(242) <= 1 && a == 255);
}

#[test]
fn single_tiles_keep_a_subpixel_size() {
    let rgba = raster_layer(
        "background-image: linear-gradient(red, red); background-size: 0.5px 10px; \
         background-repeat: no-repeat",
        10,
        10,
    );
    let edge = pixel(&rgba, 10, 0, 5);
    assert!((100..=160).contains(&edge[3]), "{edge:?}");
    assert_eq!(pixel(&rgba, 10, 1, 5)[3], 0);
}

#[test]
fn radial_stops_before_the_center_are_cut_at_zero() {
    let rgba = raster_layer(
        "background-image: radial-gradient(circle 20px, red -20px, blue 20px)",
        40,
        40,
    );
    // Halfway between the stops the center is the mix of both colors.
    let center = pixel(&rgba, 40, 20, 20);
    assert!(center[0] > 64 && center[2] > 64, "{center:?}");
    // The mix follows the gradient's Oklab interpolation, so the colors
    // next to the center continue it without a jump.
    assert!(center[1] > 30, "{center:?}");
    let next = pixel(&rgba, 40, 21, 20);
    assert!(
        (0..3).all(|channel| center[channel].abs_diff(next[channel]) <= 12),
        "{center:?} {next:?}"
    );
    // The mix at the center is premultiplied, so a transparent stop does
    // not darken it.
    let rgba = raster_layer(
        "background-image: radial-gradient(circle 20px, transparent -20px, blue 20px)",
        40,
        40,
    );
    let center = pixel(&rgba, 40, 20, 20);
    assert!(center[0] == 0 && center[1] == 0, "{center:?}");
    assert!((120..=136).contains(&center[3]), "{center:?}");
    assert!(center[2] > 245 || center[2] >= center[3] - 2, "{center:?}");
    let rgba = raster_layer(
        "background-image: radial-gradient(circle 20px, red -20px, blue -10px)",
        40,
        40,
    );
    assert_eq!(pixel(&rgba, 40, 20, 20), BLUE);
    let rgba = raster_layer(
        "background-image: repeating-radial-gradient(circle 20px, red -10px, red 0px, blue 0px, blue 10px)",
        40,
        40,
    );
    assert_eq!(pixel(&rgba, 40, 25, 20), BLUE);
    assert_eq!(pixel(&rgba, 40, 35, 20), RED);
}

#[test]
fn conic_gradients_are_left_to_their_own_painter() {
    let cv = computed_for("background-image: conic-gradient(red, blue)");
    let area = Rect::new(0.0, 0.0, 10.0, 10.0);
    let mut scene = Scene::new();
    assert!(!gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    ));
    assert!(scene.commands.is_empty());
    assert!(gradient::gradient_fill(&gradient_of(&cv), area, cv.color).is_none());
}

#[test]
fn gradient_size_ignores_natural_dimensions() {
    let area = Rect::new(0.0, 0.0, 80.0, 40.0);
    let size = |style: &str| {
        gradient::gradient_image_size(&computed_for(style).background_layers()[0].size, area)
    };
    assert_eq!(size("background-size: auto"), Some((80.0, 40.0)));
    assert_eq!(size("background-size: contain"), Some((80.0, 40.0)));
    assert_eq!(size("background-size: 25% auto"), Some((20.0, 40.0)));
    assert_eq!(size("background-size: auto 10px"), Some((80.0, 10.0)));
    assert_eq!(size("background-size: 20px 0px"), None);
}

#[test]
fn element_background_paints_the_gradient_over_its_color() {
    let cv = computed_for(
        "background: rgb(0, 255, 0) linear-gradient(to right, transparent 50%, blue 50%)",
    );
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_element_background(
        &ImageBrushCache::default(),
        &mut scene,
        40.0,
        10.0,
        0.0,
        0.0,
        cv.background_color,
        cv.background_layers(),
        cv.color,
        &cv.border_radius,
        &cv.border,
        &taffy::Rect {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        },
        None,
        &mut warnings,
    );
    assert!(warnings.is_empty());
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        40,
        10,
    );
    assert_eq!(pixel(&rgba, 40, 5, 5), [0, 255, 0, 255]);
    assert_eq!(pixel(&rgba, 40, 35, 5), BLUE);
}

#[test]
fn gradient_keeps_its_clip_inside_a_matching_border() {
    let cv = computed_for(
        "background: transparent linear-gradient(red, red); background-clip: content-box; \
         border: 10px solid red",
    );
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_element_background(
        &ImageBrushCache::default(),
        &mut scene,
        40.0,
        40.0,
        0.0,
        0.0,
        cv.background_color,
        cv.background_layers(),
        cv.color,
        &cv.border_radius,
        &cv.border,
        &taffy::Rect {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        },
        None,
        &mut warnings,
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        40,
        40,
    );
    assert_eq!(pixel(&rgba, 40, 20, 20), RED);
    assert_eq!(pixel(&rgba, 40, 5, 20)[3], 0);
}

/// Paint `cv`'s element background over a `width` by `height` box at the
/// origin and rasterize it.
fn raster_element_background(cv: &ComputedValues, width: u32, height: u32) -> Vec<u8> {
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_element_background(
        &ImageBrushCache::default(),
        &mut scene,
        width as f32,
        height as f32,
        0.0,
        0.0,
        cv.background_color,
        cv.background_layers(),
        cv.color,
        &cv.border_radius,
        &cv.border,
        &taffy::Rect {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        },
        None,
        &mut warnings,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        width,
        height,
    )
}

#[test]
fn background_layers_stack_first_on_top_over_the_color() {
    // CSS Backgrounds 3 §2.1: the first layer is closest to the user and the
    // background color is painted below every layer.
    let cv = computed_for(
        "background: linear-gradient(red, red) 0 0 / 10px 10px no-repeat, \
         linear-gradient(blue, blue) 0 0 / 20px 10px no-repeat rgb(0, 255, 0)",
    );
    assert_eq!(cv.background_layers().len(), 2);
    let rgba = raster_element_background(&cv, 40, 10);
    assert_eq!(pixel(&rgba, 40, 5, 5), RED);
    assert_eq!(pixel(&rgba, 40, 15, 5), BLUE);
    assert_eq!(pixel(&rgba, 40, 35, 5), [0, 255, 0, 255]);
}

#[test]
fn background_color_takes_the_final_layer_clip() {
    // CSS Backgrounds 3 §2.10: the color is clipped like the final layer,
    // whatever the clips of the layers above it.
    let cv = computed_for(
        "background: linear-gradient(transparent, transparent) border-box, \
         linear-gradient(transparent, transparent) content-box rgb(0, 255, 0); \
         border: 10px solid red",
    );
    let rgba = raster_element_background(&cv, 40, 40);
    assert_eq!(pixel(&rgba, 40, 20, 20), [0, 255, 0, 255]);
    assert_eq!(pixel(&rgba, 40, 5, 20)[3], 0);
}

#[test]
fn canvas_background_paints_a_propagated_body_gradient() {
    let (document, cascade) = canvas_fixture(
        Some("@page { size: 100px 40px; margin: 0 }"),
        Some("background: linear-gradient(90deg, rgb(211, 211, 211) 50%, white 50%)"),
    );
    let mut page_box = PageBox::A4;
    page_box.width = 100.0;
    page_box.height = 40.0;
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_canvas_background(
        &ImageBrushCache::default(),
        &mut scene,
        &document,
        &cascade,
        page_box,
        None,
        &mut warnings,
    );
    assert!(warnings.is_empty());
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        40,
    );
    assert_eq!(pixel(&rgba, 100, 10, 20), [211, 211, 211, 255]);
    assert_eq!(pixel(&rgba, 100, 90, 20), [255, 255, 255, 255]);
}

#[test]
fn canvas_gradient_ignores_background_clip() {
    for clip in ["text", "content-box"] {
        let (document, cascade) = canvas_fixture(
            Some("@page { size: 100px 40px; margin: 0 }"),
            Some(&format!(
                "background: linear-gradient(red, red); background-clip: {clip}; \
                 padding: 10px; border: 5px solid transparent"
            )),
        );
        let mut page_box = PageBox::A4;
        page_box.width = 100.0;
        page_box.height = 40.0;
        let mut scene = Scene::new();
        let mut warnings = Vec::new();
        paint_canvas_background(
            &ImageBrushCache::default(),
            &mut scene,
            &document,
            &cascade,
            page_box,
            None,
            &mut warnings,
        );
        assert!(warnings.is_empty(), "{clip}: {warnings:?}");
        let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| out.append_scene(scene, Affine::IDENTITY),
            100,
            40,
        );
        assert_eq!(pixel(&rgba, 100, 1, 1), RED, "{clip}");
    }
}

#[test]
fn longer_hue_turns_between_equal_stops() {
    let rgba = raster_layer(
        "background-image: linear-gradient(to right in lch longer hue, red, red)",
        100,
        4,
    );
    let [r, g, b, _] = pixel(&rgba, 100, 50, 2);
    assert!(
        r < 100 && g.max(b) > 100,
        "the middle of the turn is not red: {r} {g} {b}"
    );
}

#[test]
fn longer_hue_padding_stays_solid() {
    let rgba = raster_layer(
        "background-image: linear-gradient(to right in lch longer hue, red 0px, blue 0px)",
        100,
        4,
    );
    for x in [10, 50, 90] {
        let [r, g, b, _] = pixel(&rgba, 100, x, 2);
        assert!(r < 8 && g < 8 && b > 240, "x={x} must stay blue");
    }
    let rgba = raster_layer(
        "background-image: linear-gradient(to right in lch longer hue, red 0px, blue 100px)",
        100,
        4,
    );
    let [_, g, _, _] = pixel(&rgba, 100, 50, 2);
    assert!(
        g > 60,
        "the longer hue arc passes through green at the middle"
    );
}

#[test]
fn page_box_paints_its_gradient_over_its_color() {
    let (document, cascade) = canvas_fixture(
        Some(
            "@page { size: 100px 40px; margin: 0; background-color: white; \
             background-image: linear-gradient(90deg, red 50%, blue 50%) }",
        ),
        None,
    );
    let mut page_box = PageBox::A4;
    page_box.width = 100.0;
    page_box.height = 40.0;
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_canvas_background(
        &ImageBrushCache::default(),
        &mut scene,
        &document,
        &cascade,
        page_box,
        None,
        &mut warnings,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        40,
    );
    assert_eq!(pixel(&rgba, 100, 10, 20), RED);
    assert_eq!(pixel(&rgba, 100, 90, 20), BLUE);
}

#[test]
fn short_segments_keep_their_interpolation_space() {
    // A segment 0.02% of a 10,000px line still spans 2px, so it follows
    // Oklab rather than sRGB.
    let rgba = raster_layer(
        "background-image: linear-gradient(to right in oklab, red 49.99%, blue 50.01%)",
        10_000,
        1,
    );
    let middle = pixel(&rgba, 10_000, 4_999, 0);
    let srgb = raster_layer(
        "background-image: linear-gradient(to right in srgb, red 49.99%, blue 50.01%)",
        10_000,
        1,
    );
    let srgb_middle = pixel(&srgb, 10_000, 4_999, 0);
    assert!(
        middle[1] > srgb_middle[1] + 20,
        "{middle:?} vs {srgb_middle:?}"
    );
}

#[test]
fn many_tiny_tiles_paint_their_average_color() {
    let cv = computed_for(
        "background-image: linear-gradient(to right, red 50%, blue 50%); \
         background-size: 1px 1px",
    );
    let area = Rect::new(0.0, 0.0, 1000.0, 1000.0);
    let mut scene = Scene::new();
    assert!(gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    ));
    assert!(
        scene.commands.len() < 10,
        "{} commands",
        scene.commands.len()
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        10,
        10,
    );
    let [r, g, b, a] = pixel(&rgba, 10, 5, 5);
    assert!(r.abs_diff(128) <= 1 && g == 0 && b.abs_diff(128) <= 1 && a == 255);
}

#[test]
fn a_long_row_of_tiny_tiles_still_paints() {
    let cv = computed_for(
        "background-image: linear-gradient(to right, red 50%, blue 50%); \
         background-size: 1px 10px; background-repeat: repeat-x",
    );
    let area = Rect::new(0.0, 0.0, 20_000.0, 10.0);
    let mut scene = Scene::new();
    assert!(gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    ));
    assert!(!scene.commands.is_empty());
    assert!(
        scene.commands.len() < 10,
        "{} commands",
        scene.commands.len()
    );

    // The axis that does not repeat keeps its single tile.
    let area = Rect::new(0.0, 0.0, 20_000.0, 100.0);
    let mut scene = Scene::new();
    gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        20,
        100,
    );
    assert_eq!(pixel(&rgba, 20, 5, 5)[3], 255);
    assert_eq!(pixel(&rgba, 20, 5, 50)[3], 0);
}

#[test]
fn single_tiles_keep_fractional_edges() {
    let rgba = raster_layer(
        "background-image: linear-gradient(red, red); background-size: 10px 10px; \
         background-position: 0.5px 0; background-repeat: no-repeat",
        20,
        10,
    );
    let [.., left] = pixel(&rgba, 20, 0, 5);
    let [.., right] = pixel(&rgba, 20, 10, 5);
    assert!((100..=160).contains(&left), "{left}");
    assert_eq!(pixel(&rgba, 20, 5, 5), RED);
    assert!((100..=160).contains(&right), "{right}");
}

#[test]
fn positioned_stops_clamp_before_the_others_are_spread() {
    // Blue clamps to 80%, so yellow sits halfway to black at 90%.
    let rgba = raster_layer(
        "background-image: linear-gradient(to right, red 80%, green, blue 40%, yellow, black 100%)",
        100,
        4,
    );
    let [r, g, b, _] = pixel(&rgba, 100, 89, 2);
    assert!(r > 200 && g > 200 && b < 100, "{r} {g} {b}");
}

#[test]
fn border_area_gradients_paint_the_ring() {
    let cv = computed_for(
        "background: linear-gradient(to right, red 50%, blue 50%); \
         background-clip: border-area; background-origin: border-box; \
         border: 10px solid transparent",
    );
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_element_background(
        &ImageBrushCache::default(),
        &mut scene,
        40.0,
        40.0,
        0.0,
        0.0,
        cv.background_color,
        cv.background_layers(),
        cv.color,
        &cv.border_radius,
        &cv.border,
        &taffy::Rect {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        },
        None,
        &mut warnings,
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        40,
        40,
    );
    assert_eq!(pixel(&rgba, 40, 5, 20), RED);
    assert_eq!(pixel(&rgba, 40, 35, 20), BLUE);
    assert_eq!(pixel(&rgba, 40, 20, 20)[3], 0);
}

#[test]
fn the_tile_fallback_keeps_single_space_tiles_and_padding() {
    let cv = computed_for(
        "background-image: linear-gradient(red, red); background-size: 1px 60px; \
         background-position: center; background-repeat: repeat space",
    );
    let area = Rect::new(0.0, 0.0, 20_000.0, 100.0);
    let mut scene = Scene::new();
    gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        10,
        100,
    );
    assert_eq!(pixel(&rgba, 10, 5, 10)[3], 0);
    assert_eq!(pixel(&rgba, 10, 5, 50), RED);

    // The padded red before the first stop counts toward the average.
    let cv = computed_for(
        "background-image: linear-gradient(to right in srgb, red 80%, blue 100%); \
         background-size: 1px 1px",
    );
    let area = Rect::new(0.0, 0.0, 1000.0, 1000.0);
    let mut scene = Scene::new();
    gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        10,
        10,
    );
    let [r, _, b, _] = pixel(&rgba, 10, 5, 5);
    assert!(r > 215 && b < 45, "{r} {b}");
}

#[test]
fn page_gradients_resolve_font_lengths_with_the_page_font() {
    let (document, cascade) = canvas_fixture(
        Some(
            "@page { size: 100px 40px; margin: 0; font-size: 32px; \
             background-image: linear-gradient(to right, red 1em, blue 1em) }",
        ),
        None,
    );
    let mut page_box = PageBox::A4;
    page_box.width = 100.0;
    page_box.height = 40.0;
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_canvas_background(
        &ImageBrushCache::default(),
        &mut scene,
        &document,
        &cascade,
        page_box,
        None,
        &mut warnings,
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        40,
    );
    assert_eq!(pixel(&rgba, 100, 24, 20), RED);
    assert_eq!(pixel(&rgba, 100, 40, 20), BLUE);
}

#[test]
fn page_gradients_inherit_current_color_from_the_root() {
    let (document, cascade) = canvas_fixture(
        Some(
            "html { color: blue } @page { size: 20px 20px; margin: 0; \
             background-image: linear-gradient(currentColor, currentColor) }",
        ),
        None,
    );
    let mut page_box = PageBox::A4;
    page_box.width = 20.0;
    page_box.height = 20.0;
    let mut scene = Scene::new();
    let mut warnings = Vec::new();
    paint_canvas_background(
        &ImageBrushCache::default(),
        &mut scene,
        &document,
        &cascade,
        page_box,
        None,
        &mut warnings,
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        20,
        20,
    );
    assert_eq!(pixel(&rgba, 20, 10, 10), BLUE);
}

#[test]
fn short_periods_in_long_gradients_still_repeat() {
    let cv = computed_for(
        "background-image: repeating-linear-gradient(to right in srgb, red 0, blue 5px); \
         background-size: 10000000px 10px",
    );
    let area = Rect::new(0.0, 0.0, 100.0, 10.0);
    let mut scene = Scene::new();
    gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        10,
    );
    let [r0, _, b0, _] = pixel(&rgba, 100, 0, 5);
    let [r4, _, b4, _] = pixel(&rgba, 100, 4, 5);
    let [r5, _, b5, _] = pixel(&rgba, 100, 5, 5);
    assert!(r0 > 200 && b0 < 60, "start of a period: {r0} {b0}");
    assert!(b4 > 150 && r4 < 100, "end of a period: {r4} {b4}");
    assert!(r5 > 200 && b5 < 60, "next period: {r5} {b5}");
}

#[test]
fn many_coincident_stops_make_one_hard_edge() {
    let stops: String = (0..101)
        .map(|index| {
            if index % 2 == 0 {
                "red 50%"
            } else {
                "blue 50%"
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let cv = computed_for(&format!(
        "background-image: linear-gradient(to right, {stops})"
    ));
    let area = Rect::new(0.0, 0.0, 10_000.0, 10.0);
    let mut scene = Scene::new();
    gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    );
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| {
            out.append_scene(
                scene,
                Affine::IDENTITY.then_translate((-4_990.0, 0.0).into()),
            )
        },
        20,
        10,
    );
    // The last color, red, starts right at 50%: no band of mixed colors.
    assert_eq!(pixel(&rgba, 20, 5, 5), RED);
    assert_eq!(pixel(&rgba, 20, 11, 5), RED);
}

#[test]
fn a_space_axis_too_short_for_two_enlarged_tiles_places_one() {
    let cv = computed_for(
        "background-image: linear-gradient(red, red); background-size: 0.4px 0.4px; \
         background-position: center; background-repeat: space",
    );
    let area = Rect::new(0.0, 0.0, 1.5, 1.5);
    let mut scene = Scene::new();
    gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    );
    // One centered one-pixel tile, with its fractional edges kept.
    let shapes: Vec<Rect> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            anyrender::recording::RenderCommand::Fill(fill) => {
                Some(kurbo::Shape::bounding_box(&fill.shape))
            }
            _ => None,
        })
        .collect();
    assert_eq!(shapes, [Rect::new(0.25, 0.25, 1.25, 1.25)]);
}

#[test]
fn a_narrow_ellipse_keeps_its_tall_period() {
    let cv = computed_for(
        "background-image: repeating-radial-gradient(ellipse 0.01px 100px, red 0%, blue 100%)",
    );
    let area = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert!(matches!(
        gradient::gradient_fill(&gradient_of(&cv), area, cv.color),
        Some(gradient::GradientFill::Brush(..))
    ));
}

#[test]
fn a_lone_round_tile_keeps_its_fractional_edge() {
    let cv = computed_for(
        "background-image: linear-gradient(red, red); background-size: 20px 20px; \
         background-repeat: round",
    );
    let area = Rect::new(0.0, 0.0, 10.4, 10.4);
    let mut scene = Scene::new();
    gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        area,
        None,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    );
    let shapes: Vec<Rect> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            anyrender::recording::RenderCommand::Fill(fill) => {
                Some(kurbo::Shape::bounding_box(&fill.shape))
            }
            _ => None,
        })
        .collect();
    assert_eq!(shapes, [area]);
}

/// Paint a layer of `style` over `area`, painted inside `painting`.
fn layer_scene(style: &str, area: Rect, painting: Rect, clip: Option<&BezPath>) -> Scene {
    let cv = computed_for(style);
    let mut scene = Scene::new();
    assert!(gradient::paint_gradient_layer(
        &mut scene,
        &gradient_of(&cv),
        area,
        painting,
        clip,
        &cv.background_layers()[0].size,
        &cv.background_layers()[0].position,
        &cv.background_layers()[0].repeat,
        cv.color,
    ));
    scene
}

#[test]
fn snapped_tiles_stretch_the_brush_over_the_whole_tile() {
    // 2.5px tiles: the tile at x = 2.5 snaps to 3..5, and its brush starts
    // at 3 so the tile still begins with red.
    let rgba = raster_layer(
        "background-image: linear-gradient(to right in srgb, red, blue); \
         background-size: 2.5px 10px; background-repeat: repeat-x",
        10,
        10,
    );
    let [r, _, b, _] = pixel(&rgba, 10, 3, 5);
    assert!(r > b, "the tile starts red: {r} {b}");
}

#[test]
fn calc_sizes_and_empty_layers() {
    let area = Rect::new(0.0, 0.0, 100.0, 100.0);
    let size = raikiri_style::ComputedBackgroundSize::Explicit {
        width: raikiri_style::ComputedLengthPercentageOrAuto::Calc(
            raikiri_style::property::CalcLengthPercentage {
                percent: 50.0,
                px: 10.0,
            },
        ),
        height: raikiri_style::ComputedLengthPercentageOrAuto::Px(20.0),
    };
    assert_eq!(
        gradient::gradient_image_size(&size, area),
        Some((60.0, 20.0))
    );
    // A zero-size tile or an empty painting area paints nothing.
    let empty = layer_scene(
        "background-image: linear-gradient(red, blue); background-size: 0px 10px",
        area,
        area,
        None,
    );
    assert!(empty.commands.is_empty());
    let empty = layer_scene(
        "background-image: linear-gradient(red, blue)",
        area,
        Rect::new(0.0, 0.0, 0.0, 100.0),
        None,
    );
    assert!(empty.commands.is_empty());
}

#[test]
fn the_tile_fallback_fills_solid_tiles_inside_a_clip() {
    let area = Rect::new(0.0, 0.0, 1000.0, 1000.0);
    let clip = kurbo::Shape::to_path(&area, 0.1);
    let scene = layer_scene(
        "background-image: repeating-linear-gradient(red 10px, blue 10px); \
         background-size: 1px 1px",
        area,
        area,
        Some(&clip),
    );
    assert!(
        scene.commands.len() < 10,
        "{} commands",
        scene.commands.len()
    );
}

#[test]
fn the_tile_fallback_pads_after_the_last_stop() {
    // Blue from 50% to 100% weighs three times the red-to-blue half.
    let rgba = raster_layer(
        "background-image: linear-gradient(to right in srgb, red 0%, blue 50%); \
         background-size: 1px 1px",
        1000,
        10,
    );
    let [r, _, b, _] = pixel(&rgba, 1000, 5, 5);
    assert!(b > r, "{r} {b}");
}

#[test]
fn side_directions_point_left_and_up() {
    let rgba = raster_layer(
        "background-image: linear-gradient(to left, red 50%, blue 50%)",
        40,
        40,
    );
    assert_eq!(pixel(&rgba, 40, 5, 20), BLUE);
    let rgba = raster_layer(
        "background-image: linear-gradient(to top, red 50%, blue 50%)",
        40,
        40,
    );
    assert_eq!(pixel(&rgba, 40, 20, 5), BLUE);
}

#[test]
fn degenerate_shapes_and_lengths_still_resolve() {
    let area = Rect::new(0.0, 0.0, 100.0, 100.0);
    let fill = |style: &str, area: Rect| {
        let cv = computed_for(style);
        gradient::gradient_fill(&gradient_of(&cv), area, cv.color)
    };
    // A corner-sized ellipse centered on an edge has no side extent.
    assert!(
        fill(
            "background-image: radial-gradient(ellipse closest-corner at 0 50%, red, blue)",
            area
        )
        .is_some()
    );
    // A repeating radial gradient with one stop position is its average.
    assert!(matches!(
        fill(
            "background-image: repeating-radial-gradient(red 10px, blue 10px)",
            area
        ),
        Some(gradient::GradientFill::Solid(_))
    ));
    // Font-relative stops resolve; a gradient line of no length collapses.
    assert!(
        fill(
            "background-image: linear-gradient(to right, red 1em, blue 2em)",
            area
        )
        .is_some()
    );
    assert!(matches!(
        fill(
            "background-image: repeating-linear-gradient(to right, red 0%, blue 10px)",
            Rect::new(0.0, 0.0, 0.0, 100.0)
        ),
        Some(gradient::GradientFill::Solid(_))
    ));
}

#[test]
fn average_colors_of_few_or_transparent_stops() {
    let red = CssColor::from_hex("ff0000").unwrap();
    let clear = CssColor::from_hex("00000000").unwrap();
    assert_eq!(gradient::average_color(&[]), Color::TRANSPARENT);
    assert_eq!(
        gradient::average_color(&[(0.0, red)]),
        Color::from_rgba8(255, 0, 0, 255)
    );
    assert_eq!(
        gradient::average_color(&[(0.0, clear), (1.0, clear)]),
        Color::from_rgba8(0, 0, 0, 0)
    );
}
