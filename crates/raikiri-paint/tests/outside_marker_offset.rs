//! The built-in painter places outside text at the printed marker edge.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

const CSS: &str = include_str!("../../raikiri-dom/tests/data/outside-marker-offset.css");
const AHEM: &[u8] = include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf");

fn red_columns(extra: &str) -> Option<(usize, usize)> {
    let html = format!(
        "<!doctype html><style>{CSS}{extra}li::marker{{color:red}}</style><ol><li>X</li></ol>"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    let page = result.page(0).unwrap();
    let (document, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_origin(&mut scene, document, cascade, page_box, origin)
        .unwrap();
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        400,
        400,
    );
    let columns: Vec<_> = rgba
        .chunks_exact(4)
        .enumerate()
        .filter_map(|(i, p)| {
            (p[0] > 200 && p[1] < 50 && p[2] < 50 && p[3] > 200).then_some(i % 400)
        })
        .collect();
    columns
        .iter()
        .min()
        .zip(columns.iter().max())
        .map(|(&left, &right)| (left, right + 1))
}

#[test]
fn outside_marker_pixels_match_the_chromium_horizontal_bounds() {
    // Ahem fills each nonblank glyph's em square. The positions come from
    // the Chromium PDF references in the DOM fixture; changing only the marker
    // color lets the CPU renderer's actual pixels verify the shared offset.
    for (css, bounds) in [
        ("li::marker{content:\"M\"}", (90, 100)),
        ("li::marker{content:\"M \"}", (80, 90)),
        ("", (70, 90)),
        (
            "li{padding-left:7px;border-left:3px solid}li::marker{content:\"M\"}",
            (90, 100),
        ),
        ("li::marker{content:\"M\\a NN \"}", (70, 90)),
    ] {
        assert_eq!(red_columns(css), Some(bounds), "{css}");
    }
}

#[test]
fn marker_display_does_not_suppress_text_or_image_pixels() {
    let image = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10'%3E%3Crect width='10' height='10' fill='red'/%3E%3C/svg%3E";
    for position in ["inside", "outside"] {
        let text = format!("li{{list-style-position:{position}}}li::marker{{content:'M'}}");
        let expected = if position == "inside" {
            (100, 110)
        } else {
            (90, 100)
        };
        assert_eq!(red_columns(&text), Some(expected));
        assert_eq!(
            red_columns(&format!("{text}li::marker{{display:none}}")),
            Some(expected)
        );
        let css = format!("li{{list-style-position:{position};list-style-image:url(\"{image}\")}}");
        let bounds = red_columns(&css).expect("the image marker is painted");
        assert_eq!(bounds.1 - bounds.0, 10);
        // Image offset compatibility is independent of marker generation.
        assert_eq!(
            red_columns(&format!("{css}li::marker{{display:none}}")),
            Some(bounds)
        );
        assert_eq!(
            red_columns(&format!("{css}li::marker{{display:none;content:none}}")),
            None
        );
        assert_eq!(red_columns(&format!("{css}li{{display:none}}")), None);
    }
}
