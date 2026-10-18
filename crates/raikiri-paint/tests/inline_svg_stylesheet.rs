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
