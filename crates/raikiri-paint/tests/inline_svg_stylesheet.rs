//! The declarations a page stylesheet gives to the descendants of an inline
//! SVG reach the built-in painter, as they reach the page's vector export.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

fn raster(body: &str) -> Vec<u8> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:50px 50px;margin:0}}body{{margin:0;background:white;font:10px/10px Ahem}}</style>{body}"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let page = result.page(0).unwrap();
    let (doc, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_origin(&mut scene, doc, cascade, page_box, origin)
        .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        50,
        50,
    )
}

fn differences(actual: &[u8], expected: &[u8]) -> usize {
    actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(actual, expected)| actual != expected)
        .count()
}

/// `body` paints exactly what `reference` does, and that is not a blank page.
#[track_caller]
fn assert_paints_like(body: &str, reference: &str) {
    let expected = raster(reference);
    assert_ne!(differences(&expected, &raster("")), 0, "{reference}");
    let diff = differences(&raster(body), &expected);
    assert_eq!(diff, 0, "{body} differs from {reference}");
}

#[test]
fn a_stylesheet_color_reaches_a_descendant() {
    assert_paints_like(
        "<style>rect{color:blue}</style><svg width=20 height=20><rect width=20 height=20 fill=currentColor /></svg>",
        "<svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
    );
}

#[test]
fn a_stylesheet_display_none_hides_a_descendant() {
    assert_paints_like(
        "<style>.gone{display:none}</style><svg width=20 height=20><rect class=gone width=10 height=20 fill=red /><rect x=10 width=10 height=20 fill=blue /></svg>",
        "<svg width=20 height=20><rect x=10 width=10 height=20 fill=blue /></svg>",
    );
}

#[test]
fn a_stylesheet_opacity_applies_to_a_descendant() {
    assert_paints_like(
        "<style>rect{opacity:.5}</style><svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
        "<svg width=20 height=20><rect width=20 height=20 fill=blue opacity=.5 /></svg>",
    );
}

#[test]
fn a_stylesheet_visibility_shows_a_descendant_of_a_hidden_root() {
    let hidden = "<style>svg{visibility:hidden}.shown{visibility:visible}</style>";
    assert_paints_like(
        &format!(
            "{hidden}<svg width=20 height=20><rect width=10 height=20 fill=red /><rect class=shown x=10 width=10 height=20 fill=blue /></svg>"
        ),
        "<svg width=20 height=20><rect x=10 width=10 height=20 fill=blue /></svg>",
    );
}

#[test]
fn a_hidden_root_whose_descendants_stay_hidden_paints_nothing() {
    assert_eq!(
        differences(
            &raster(
                "<style>svg{visibility:hidden}p{visibility:visible}</style><svg width=20 height=20><rect width=20 height=20 fill=red /></svg>"
            ),
            &raster("")
        ),
        0
    );
}

#[test]
fn a_stylesheet_fill_beats_the_fill_attribute() {
    assert_paints_like(
        "<style>rect{fill:blue}</style><svg width=20 height=20><rect width=20 height=20 fill=red /></svg>",
        "<svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
    );
}

#[test]
fn a_stylesheet_fill_on_a_group_is_inherited_by_its_shapes() {
    assert_paints_like(
        "<style>g{fill:blue}</style><svg width=20 height=20><g><rect width=20 height=20 /></g></svg>",
        "<svg width=20 height=20><g fill=blue><rect width=20 height=20 /></g></svg>",
    );
}

#[test]
fn a_stylesheet_fill_none_removes_the_fill() {
    assert_paints_like(
        "<style>.bare{fill:none}</style><svg width=20 height=20><rect class=bare width=10 height=20 fill=red /><rect x=10 width=10 height=20 fill=blue /></svg>",
        "<svg width=20 height=20><rect x=10 width=10 height=20 fill=blue /></svg>",
    );
}

#[test]
fn a_stylesheet_fill_may_reference_a_gradient() {
    let defs = "<defs><linearGradient id=g><stop offset=0 stop-color=blue /><stop offset=1 stop-color=lime /></linearGradient></defs>";
    assert_paints_like(
        &format!(
            "<style>rect{{fill:url(#g)}}</style><svg width=20 height=20>{defs}<rect width=20 height=20 fill=red /></svg>"
        ),
        &format!("<svg width=20 height=20>{defs}<rect width=20 height=20 fill='url(#g)' /></svg>"),
    );
}

#[test]
fn a_stylesheet_stroke_and_stroke_width_draw_an_outline() {
    assert_paints_like(
        "<style>rect{stroke:blue;stroke-width:4px}</style><svg width=30 height=30><rect x=5 y=5 width=20 height=20 fill=none /></svg>",
        "<svg width=30 height=30><rect x=5 y=5 width=20 height=20 fill=none stroke=blue stroke-width=4 /></svg>",
    );
}

#[test]
fn a_stylesheet_stroke_width_in_ems_follows_the_svg_font_size() {
    assert_paints_like(
        "<style>rect{stroke:blue;stroke-width:.5em}</style><svg width=30 height=30><rect x=5 y=5 width=20 height=20 fill=none /></svg>",
        "<svg width=30 height=30><rect x=5 y=5 width=20 height=20 fill=none stroke=blue stroke-width=5 /></svg>",
    );
}

#[test]
fn stylesheet_fill_and_stroke_opacities_apply_separately() {
    assert_paints_like(
        "<style>rect{fill-opacity:.25;stroke-opacity:50%}</style><svg width=30 height=30><rect x=5 y=5 width=20 height=20 fill=blue stroke=red stroke-width=4 /></svg>",
        "<svg width=30 height=30><rect x=5 y=5 width=20 height=20 fill=blue stroke=red stroke-width=4 fill-opacity=.25 stroke-opacity=.5 /></svg>",
    );
}

#[test]
fn an_important_stylesheet_fill_beats_the_style_attribute() {
    assert_paints_like(
        "<style>rect{fill:blue !important}</style><svg width=20 height=20><rect width=20 height=20 style='fill:red' /></svg>",
        "<svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
    );
}

#[test]
fn a_style_attribute_fill_beats_the_stylesheet() {
    assert_paints_like(
        "<style>rect{fill:red}</style><svg width=20 height=20><rect width=20 height=20 style='fill:blue' /></svg>",
        "<svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
    );
}

#[test]
fn a_style_element_inside_the_svg_sets_a_fill() {
    assert_paints_like(
        "<svg width=20 height=20><style>rect{fill:blue}</style><rect width=20 height=20 fill=red /></svg>",
        "<svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
    );
}

#[test]
fn the_later_of_a_page_rule_and_an_svg_style_rule_wins_a_fill() {
    assert_paints_like(
        "<style>rect{fill:red}</style><svg width=20 height=20><style>rect{fill:blue}</style><rect width=20 height=20 /></svg>",
        "<svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
    );
    assert_paints_like(
        "<svg width=20 height=20><style>rect{fill:red}</style><rect width=20 height=20 /></svg><style>rect{fill:blue}</style>",
        "<svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
    );
}

#[test]
fn a_stylesheet_fill_on_the_root_reaches_the_descendants_that_set_none() {
    assert_paints_like(
        "<style>svg{fill:blue}</style><svg width=20 height=20><rect width=10 height=20 /><rect x=10 width=10 height=20 fill=red /></svg>",
        "<svg width=20 height=20><rect width=10 height=20 fill=blue /><rect x=10 width=10 height=20 fill=red /></svg>",
    );
}

#[test]
fn a_stylesheet_fill_on_the_root_beats_the_fill_attribute_of_the_root() {
    assert_paints_like(
        "<style>svg{fill:blue}</style><svg width=20 height=20 fill=red><rect width=20 height=20 /></svg>",
        "<svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
    );
}

#[test]
fn a_stylesheet_current_color_fill_on_the_root_paints_an_icon_in_its_color() {
    assert_paints_like(
        "<style>svg{fill:currentColor;color:blue}</style><svg width=20 height=20><path d='M0 0h20v20H0z' /></svg>",
        "<svg width=20 height=20><path d='M0 0h20v20H0z' fill=blue /></svg>",
    );
}

#[test]
fn a_stylesheet_stroke_on_the_root_reaches_the_descendants() {
    assert_paints_like(
        "<style>svg{stroke:blue;stroke-width:4px}</style><svg width=20 height=20><rect x=4 y=4 width=12 height=12 fill=none /></svg>",
        "<svg width=20 height=20><rect x=4 y=4 width=12 height=12 fill=none stroke=blue stroke-width=4 /></svg>",
    );
}

#[test]
fn revert_layer_on_a_fill_rolls_back_to_the_fill_attribute() {
    assert_paints_like(
        "<style>@layer a, b; @layer b {rect{fill:revert-layer}}</style><svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
        "<svg width=20 height=20><rect width=20 height=20 fill=blue /></svg>",
    );
}

#[test]
fn an_overridden_fill_attribute_still_matches_the_svgs_attribute_selectors() {
    // `stroke-dasharray` is not a property the page cascade carries, so only
    // the SVG's own stylesheet can apply it, and only while the rule's
    // `[fill=red]` still matches the element whose `fill` the page overrides.
    assert_paints_like(
        "<style>rect{fill:blue;stroke:green;stroke-width:4px}</style><svg width=20 height=20><style>rect[fill=red]{stroke-dasharray:3 3}</style><rect x=4 y=4 width=12 height=12 fill=red /></svg>",
        "<svg width=20 height=20><rect x=4 y=4 width=12 height=12 fill=blue stroke=green stroke-width=4 stroke-dasharray='3 3' /></svg>",
    );
}
