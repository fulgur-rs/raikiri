//! A box that is not visible paints none of its own decorations, while its
//! visible descendants still paint (CSS Display 3 §4).

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
        "<!doctype html><style>@page{{size:50px 50px;margin:0}}body{{margin:0;background:white;font:10px/10px Ahem}}table{{border-spacing:0}}td{{padding:0;width:20px;height:20px;vertical-align:top}}.g::before{{content:'';display:inline-block;width:20px;height:20px;background:red;visibility:hidden}}.i::before{{content:'XX';background:red;border:5px solid red;visibility:hidden}}</style>{body}"
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

/// `body` paints exactly what `reference` does.
#[track_caller]
fn assert_paints_like(body: &str, reference: &str) {
    let diff = differences(&raster(body), &raster(reference));
    assert_eq!(diff, 0, "{body} differs from {reference}");
}

#[test]
fn a_box_that_is_not_visible_paints_no_decoration() {
    for keyword in ["hidden", "collapse"] {
        for decoration in [
            "background:red",
            "border:5px solid red",
            "outline:5px solid red;margin:10px",
            "box-shadow:5px 5px 0 red;margin:10px",
        ] {
            let style = format!("visibility:{keyword};{decoration};width:10px;height:10px");
            assert_paints_like(&format!("<div style='{style}'></div>"), "");
            assert_paints_like(
                &format!("<span style='display:inline-block;{style}'></span>"),
                "",
            );
        }
    }
}

#[test]
fn an_inline_box_that_is_not_visible_paints_no_decoration() {
    for keyword in ["hidden", "collapse"] {
        for decoration in ["background:red", "border:5px solid red"] {
            assert_paints_like(
                &format!("<span style='visibility:{keyword};{decoration}'>XX</span>"),
                "",
            );
        }
    }
}

#[test]
fn a_generated_box_that_is_not_visible_paints_no_decoration() {
    assert_paints_like("<div class=g></div>", "");
    assert_paints_like("<div class=i></div>", "");
}

#[test]
fn a_table_part_that_is_not_visible_paints_no_decoration() {
    let empty = "<table><tr><td></td></tr></table>";
    for keyword in ["hidden", "collapse"] {
        for decoration in ["background:red", "border:5px solid red"] {
            let style = format!("visibility:{keyword};{decoration}");
            assert_paints_like(
                &format!("<table style='{style}'><tr><td></td></tr></table>"),
                empty,
            );
            assert_paints_like(
                &format!("<table><tr><td style='{style}'></td></tr></table>"),
                empty,
            );
            assert_paints_like(
                &format!("<table><tr style='{style}'><td></td></tr></table>"),
                empty,
            );
        }
    }
}

#[test]
fn a_cell_that_inherits_collapse_from_its_row_paints_no_background() {
    assert_paints_like(
        "<table><tr style='visibility:collapse'><td rowspan=2 style='background:red'></td></tr><tr><td></td></tr></table>",
        "<table><tr><td rowspan=2></td></tr><tr><td></td></tr></table>",
    );
}

/// Without a pixel source, an image whose file name names a color stands in
/// as a box of that color.
#[test]
fn an_image_that_is_not_visible_paints_no_fallback_box() {
    for keyword in ["hidden", "collapse"] {
        assert_paints_like(
            &format!("<img src='red.png' style='visibility:{keyword};width:20px;height:20px'>"),
            "",
        );
    }
}

#[test]
fn an_inline_svg_that_is_not_visible_paints_nothing() {
    for keyword in ["hidden", "collapse"] {
        assert_paints_like(
            &format!(
                "<svg width=20 height=20 style='visibility:{keyword}'><rect width=20 height=20 fill=red /></svg>"
            ),
            "",
        );
    }
}

#[test]
fn an_inline_svg_that_is_not_visible_still_paints_its_visible_descendants() {
    let visible_half = "<svg width=20 height=20><rect x=10 width=10 height=20 fill=blue /></svg>";
    assert_ne!(differences(&raster(visible_half), &raster("")), 0);
    for keyword in ["hidden", "collapse"] {
        assert_paints_like(
            &format!(
                "<svg width=20 height=20 style='visibility:{keyword}'><rect width=10 height=20 fill=red /><rect x=10 width=10 height=20 fill=blue visibility=visible /></svg>"
            ),
            visible_half,
        );
    }
}

#[test]
fn an_inline_svg_hidden_by_a_stylesheet_paints_only_what_sets_its_own_visibility() {
    let hidden = "<style>svg{visibility:hidden}</style>";
    assert_paints_like(
        &format!("{hidden}<svg width=20 height=20><rect width=20 height=20 fill=red /></svg>"),
        "",
    );
    assert_paints_like(
        &format!(
            "{hidden}<svg width=20 height=20><rect width=10 height=20 fill=red /><rect x=10 width=10 height=20 fill=blue visibility=visible /></svg>"
        ),
        "<svg width=20 height=20><rect x=10 width=10 height=20 fill=blue /></svg>",
    );
}

#[test]
fn visible_descendants_of_a_box_that_is_not_visible_still_paint() {
    let child = "<div style='visibility:visible;width:10px;height:10px;background:blue'></div>";
    for keyword in ["hidden", "collapse"] {
        assert_paints_like(
            &format!(
                "<div style='visibility:{keyword};background:red;border:5px solid red;width:20px;height:20px'>{child}</div>"
            ),
            &format!("<div style='margin:5px;width:20px;height:20px'>{child}</div>"),
        );
    }
}

#[test]
fn a_visible_box_still_paints_its_decoration() {
    let reference = raster("");
    for body in [
        "<div style='background:red;width:10px;height:10px'></div>",
        "<span style='background:red'>XX</span>",
        "<table><tr><td style='background:red'></td></tr></table>",
        "<img src='red.png' style='width:20px;height:20px'>",
        "<svg width=20 height=20><rect width=20 height=20 fill=red /></svg>",
    ] {
        assert_ne!(differences(&raster(body), &reference), 0, "{body}");
    }
}
