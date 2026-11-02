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
