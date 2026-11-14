use super::*;
use std::sync::Arc;

struct RedPixels;
impl ImagePixelSource for RedPixels {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<raikiri_traits::DecodedImage>> {
        Some(Arc::new(raikiri_traits::DecodedImage::new(
            20,
            20,
            [255, 0, 0, 255].repeat(400),
        )))
    }
}

fn red_bounds(prefix: u32, page_index: usize) -> Option<(usize, usize)> {
    let markup = format!(
        "<!doctype html><style>body{{margin:0;font:20px/30px Ahem}}ul{{margin:0;padding:0 0 0 40px;list-style-image:url(https://images.test/red.png)}}</style><ul><li><div style='height:{prefix}px'></div>z</li></ul>"
    );
    let mut parsed =
        raikiri_html::parse(markup.as_bytes(), &raikiri_html::ParseOptions::new()).unwrap();
    let fonts = raikiri_dom::build_wpt_font_collection(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../raikiri-dom/tests/data/text-autospace"
    )))
    .unwrap();
    parsed.dom.set_font_collection(fonts);
    let cascade = raikiri_html::build_cascaded(&parsed).unwrap();
    parsed
        .dom
        .prepare_list_marker_images(&cascade, &RedPixels, None);
    let page = square_page(100.0);
    let pages = raikiri_dom::layout_pages(&mut parsed.dom, &cascade, page).unwrap();
    parsed
        .dom
        .project_pages(&cascade, page, &pages, &[])
        .unwrap();
    let mut scene = Scene::new();
    crate::paint_single_page_with_origin_and_page_context_named_with_fixed_page_width(
        &mut scene,
        &parsed.dom,
        &cascade,
        page,
        pages[page_index].content_origin_y,
        page_index as u32,
        pages.len() as u32,
        true,
        None,
        None,
        page.width,
    )
    .unwrap();
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        100,
        100,
    );
    let rows: Vec<_> = rgba
        .chunks_exact(4)
        .enumerate()
        .filter_map(|(i, p)| {
            (p[0] > 200 && p[1] < 50 && p[2] < 50 && p[3] > 200).then_some(i / 100)
        })
        .collect();
    rows.iter()
        .min()
        .zip(rows.iter().max())
        .map(|(&a, &b)| (a, b + 1))
}

#[test]
fn outside_image_pixels_follow_the_first_line() {
    assert_eq!(red_bounds(65, 0), Some((66, 86)));
    assert_eq!(red_bounds(85, 0), None);
    assert_eq!(red_bounds(85, 1), Some((1, 21)));
}

#[test]
fn outside_image_pixels_use_the_border_box_inline_start() {
    for (direction, marker, width, text, expected) in [
        ("ltr", "ltr", 100, "z", (13, 33)),
        ("rtl", "rtl", 100, "z", (177, 197)),
        ("ltr", "rtl", 100, "z", (13, 33)),
        ("rtl", "ltr", 100, "z", (177, 197)),
        ("rtl", "rtl", 0, "", (47, 67)),
    ] {
        let (padding, border) = if width == 0 { (0, 0) } else { (12, 3) };
        let markup = format!(
            "<!doctype html><style>body{{margin:0;font:20px/30px Ahem}}li{{margin-left:40px;display:list-item;width:{width}px;height:40px;padding:0 {padding}px;border:{border}px solid black;direction:{direction};list-style-image:url(https://images.test/red.png)}}li::marker{{direction:{marker}}}</style><li>{text}</li>"
        );
        let mut parsed =
            raikiri_html::parse(markup.as_bytes(), &raikiri_html::ParseOptions::new()).unwrap();
        let fonts = raikiri_dom::build_wpt_font_collection(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../raikiri-dom/tests/data/text-autospace"
        )))
        .unwrap();
        parsed.dom.set_font_collection(fonts);
        let cascade = raikiri_html::build_cascaded(&parsed).unwrap();
        parsed
            .dom
            .prepare_list_marker_images(&cascade, &RedPixels, None);
        let page = square_page(240.0);
        let pages = raikiri_dom::layout_pages(&mut parsed.dom, &cascade, page).unwrap();
        parsed
            .dom
            .project_pages(&cascade, page, &pages, &[])
            .unwrap();
        let mut scene = Scene::new();
        crate::paint_single_page_with_origin_and_page_context_named_with_fixed_page_width(
            &mut scene,
            &parsed.dom,
            &cascade,
            page,
            pages[0].content_origin_y,
            0,
            pages.len() as u32,
            true,
            None,
            None,
            page.width,
        )
        .unwrap();
        let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |renderer| renderer.append_scene(scene, Affine::IDENTITY),
            240,
            240,
        );
        let columns: Vec<_> = rgba
            .chunks_exact(4)
            .enumerate()
            .filter_map(|(i, p)| {
                (p[0] > 200 && p[1] < 50 && p[2] < 50 && p[3] > 200).then_some(i % 240)
            })
            .collect();
        let bounds = columns
            .iter()
            .min()
            .zip(columns.iter().max())
            .map(|(&a, &b)| (a, b + 1));
        assert_eq!(
            bounds,
            Some(expected),
            "{direction}, marker {marker}, width {width}"
        );
    }
}

/// The rows of the red pixels that the marker of a list item with
/// `item_css`, whose first line follows a block `prefix` px tall, paints on
/// the first page, after layout projected the pages or, when `project` is
/// false, as laid out.
fn red_marker_rows(item_css: &str, prefix: u32, project: bool) -> Option<(usize, usize)> {
    let markup = format!(
        "<!doctype html><style>body{{margin:0;font:20px/30px Ahem}}ul{{margin:0;padding:0 0 0 40px}}li{{color:white}}li::marker{{color:red}}{item_css}</style><ul><li><div style='height:{prefix}px'></div>z</li></ul>"
    );
    let mut parsed =
        raikiri_html::parse(markup.as_bytes(), &raikiri_html::ParseOptions::new()).unwrap();
    let fonts = raikiri_dom::build_wpt_font_collection(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../raikiri-dom/tests/data/text-autospace"
    )))
    .unwrap();
    parsed.dom.set_font_collection(fonts);
    let cascade = raikiri_html::build_cascaded(&parsed).unwrap();
    parsed
        .dom
        .prepare_list_marker_images(&cascade, &RedPixels, None);
    let page = square_page(100.0);
    let pages = raikiri_dom::layout_pages(&mut parsed.dom, &cascade, page).unwrap();
    if project {
        parsed
            .dom
            .project_pages(&cascade, page, &pages, &[])
            .unwrap();
    }
    let mut scene = Scene::new();
    crate::paint_single_page_with_origin_and_page_context_named_with_fixed_page_width(
        &mut scene,
        &parsed.dom,
        &cascade,
        page,
        pages[0].content_origin_y,
        0,
        pages.len() as u32,
        true,
        None,
        None,
        page.width,
    )
    .unwrap();
    let rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |renderer| renderer.append_scene(scene, Affine::IDENTITY),
        100,
        100,
    );
    let rows: Vec<_> = rgba
        .chunks_exact(4)
        .enumerate()
        .filter_map(|(i, p)| {
            (p[0] > 200 && p[1] < 50 && p[2] < 50 && p[3] > 200).then_some(i / 100)
        })
        .collect();
    rows.iter()
        .min()
        .zip(rows.iter().max())
        .map(|(&a, &b)| (a, b + 1))
}

#[test]
fn outside_markers_meet_the_first_baseline_with_or_without_projected_pages() {
    // Ahem 20px/30px puts the first baseline 5px of half-leading plus the
    // 16px ascent down. Chromium sits the text marker's baseline and the
    // 20px image's bottom on it, whether or not the pages carry margin boxes.
    for (item_css, expected) in [
        ("li::marker{content:'X'}", (5, 25)),
        (
            "ul{list-style-image:url(https://images.test/red.png)}",
            (1, 21),
        ),
    ] {
        for project in [true, false] {
            assert_eq!(
                red_marker_rows(item_css, 0, project),
                Some(expected),
                "{item_css}, projected {project}"
            );
        }
        // A first line pushed to the next page leaves the unprojected
        // marker at the item's border edge on its first page, as before,
        // rather than past that page's end.
        assert_eq!(
            red_marker_rows(item_css, 85, false),
            Some((0, 20)),
            "{item_css}, pushed"
        );
    }
}
