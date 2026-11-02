use super::*;

fn computed(style: &str) -> ComputedValues {
    let mut document = Document::new();
    let id = document.append_element(Some(document.root_index()), "div", Some(style));
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    cascade.computed[id].clone()
}

fn fill(style: &str, cache: &ImageBrushCache) -> gradient::GradientFill {
    let cv = computed(style);
    let conic = conic_of(&cv.background_layers()[0].image);
    conic_to_peniko(cache, &conic, Point::new(50.0, 50.0), cv.color).expect("conic fill")
}

fn assert_blue_fallback(fill: gradient::GradientFill) {
    let gradient::GradientFill::Solid(color) = fill else {
        panic!("expected bounded solid fallback");
    };
    assert_eq!(color, Color::from_rgba8(0, 0, 255, 255));
}

fn duplicate_stops(count: usize) -> String {
    std::iter::repeat_n("red 0", count - 1)
        .chain(std::iter::once("blue .0244140625%"))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn conic_stop_product_is_bounded() {
    // The reduced trigger expands 524,288 stops before the fix, avoiding
    // the original input's 32,768,000-stop allocation in the red run.
    let style = format!(
        "background-image:repeating-conic-gradient({})",
        duplicate_stops(128)
    );
    assert_blue_fallback(fill(&style, &ImageBrushCache::default()));
}

#[test]
fn bounded_fallback_resolves_current_color() {
    let stops = duplicate_stops(128).replace("blue .0244140625%", "currentColor .0244140625%");
    let style = format!("color:rgb(17,34,51);background-image:repeating-conic-gradient({stops})");
    let gradient::GradientFill::Solid(color) = fill(&style, &ImageBrushCache::default()) else {
        panic!("bounded fallback");
    };
    assert_eq!(color, Color::from_rgba8(17, 34, 51, 255));
}

#[test]
fn original_trigger_is_bounded_through_shorthand_and_longhand() {
    let stops = duplicate_stops(8_000);
    for property in ["background", "background-image"] {
        let style = format!("{property}:repeating-conic-gradient({stops})");
        assert!(style.len() < 64 * 1024);
        let cv = computed(&style);
        let conic = conic_of(&cv.background_layers()[0].image);
        assert_eq!(conic.stops.len(), 8_000);
        assert_blue_fallback(
            conic_to_peniko(
                &ImageBrushCache::default(),
                &conic,
                Point::new(50.0, 50.0),
                cv.color,
            )
            .expect("conic fill"),
        );
    }
}

#[test]
fn nonduplicate_positions_cannot_bypass_the_product_bound() {
    let stops = (0..128)
        .map(|index| {
            let color = if index % 2 == 0 { "red" } else { "blue" };
            format!("{color} {}%", f64::from(index) / 127.0 / 40.96)
        })
        .collect::<Vec<_>>()
        .join(",");
    let style = format!("background-image:repeating-conic-gradient({stops})");
    assert_blue_fallback(fill(&style, &ImageBrushCache::default()));
}

#[test]
fn period_threshold_keeps_the_average_below_and_hard_seams_at_or_above() {
    let threshold = 0.024_414_063_f32;
    for (end, average) in [
        (f32::from_bits(threshold.to_bits() - 1), true),
        (threshold, false),
        (f32::from_bits(threshold.to_bits() + 1), false),
    ] {
        let style =
            format!("background-image:repeating-conic-gradient(in srgb, red 0, blue {end}%)");
        let result = fill(&style, &ImageBrushCache::default());
        if average {
            let gradient::GradientFill::Solid(color) = result else {
                panic!("expected the ordinary tiny-period average");
            };
            assert_eq!(color, Color::from_rgba8(128, 0, 128, 255));
        } else {
            let gradient::GradientFill::Brush(brush, _) = result else {
                panic!("expected individual periods");
            };
            assert!(brush.stops.len() > 2 && brush.stops.len() <= 65_536);
            assert!(brush.stops.windows(2).any(|pair| {
                pair[0].offset == pair[1].offset && pair[0].color != pair[1].color
            }));
        }
        let large = duplicate_stops(128).replace(".0244140625%", &format!("{end}%"));
        let style = format!("background-image:repeating-conic-gradient({large})");
        // Below the threshold this remains a cheap average; at or above it,
        // the repeated product exceeds the vector limit instead.
        assert!(matches!(
            fill(&style, &ImageBrushCache::default()),
            gradient::GradientFill::Solid(_)
        ));
    }
}

#[test]
fn hint_expansion_is_counted_before_repeating() {
    let expanded = "background-image:repeating-conic-gradient(in srgb, red 0, .006103515625%, blue .0244140625%)";
    assert_blue_fallback(fill(expanded, &ImageBrushCache::default()));
    for ordinary in [
        "background-image:repeating-conic-gradient(in srgb, red 0, blue .0244140625%)",
        "background-image:repeating-conic-gradient(in srgb, red 0, .01220703125%, blue .0244140625%)",
        "background-image:repeating-conic-gradient(in srgb, red 0, .006103515625%, red .0244140625%)",
        "background-image:repeating-conic-gradient(in oklch longer hue, red 0, .006103515625%, blue .0244140625%)",
    ] {
        assert!(matches!(
            fill(ordinary, &ImageBrushCache::default()),
            gradient::GradientFill::Brush(_, _)
        ));
    }
}

#[test]
fn huge_hint_lists_are_bounded_even_without_repetition() {
    let mut stops = vec!["red 0deg".to_owned()];
    for index in 1..2_500 {
        stops.push(format!("{}.25deg", index - 1));
        stops.push(format!(
            "{} {index}deg",
            if index % 2 == 0 { "red" } else { "blue" }
        ));
    }
    let style = format!("background-image:conic-gradient({})", stops.join(","));
    assert!(style.len() < 64 * 1024);
    assert_blue_fallback(fill(&style, &ImageBrushCache::default()));
}

#[test]
fn many_elements_share_the_paint_budget_and_a_new_paint_resets_it() {
    let style = format!(
        "display:block;width:10px;height:10px;background-image:repeating-conic-gradient({})",
        std::iter::repeat_n("red 0", 7)
            .chain(std::iter::once("blue .048828125%"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    let body = document.append_element(Some(html), "body", Some("display:block"));
    for _ in 0..20 {
        document.append_element(Some(body), "div", Some(&style));
    }
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    raikiri_dom::layout_single_page(&mut document, &cascade, PageBox::A4).expect("layout");
    let paint_counts = || {
        let mut scene = Scene::new();
        crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4).expect("paint");
        let mut gradients = 0;
        let mut fallback = 0;
        let mut stops = 0;
        for command in scene.commands {
            if let RenderCommand::Fill(fill) = command {
                match fill.brush {
                    anyrender::Paint::Gradient(gradient) => {
                        gradients += 1;
                        stops += gradient.stops.len();
                    }
                    anyrender::Paint::Solid(color)
                        if color == Color::from_rgba8(0, 0, 255, 255) =>
                    {
                        fallback += 1
                    }
                    _ => {}
                }
            }
        }
        (gradients, fallback, stops)
    };
    let first = paint_counts();
    assert!(first.0 > 0 && first.0 < 20);
    assert!(first.1 > 0);
    assert_eq!(first.0 + first.1, 20);
    assert!(first.2 <= 1_048_576);
    assert_eq!(paint_counts(), first);
}
