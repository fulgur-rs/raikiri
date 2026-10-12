use super::*;
use std::collections::HashSet;
use std::sync::Arc;

struct SharedPixels(Arc<raikiri_traits::DecodedImage>);

impl ImagePixelSource for SharedPixels {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<raikiri_traits::DecodedImage>> {
        Some(Arc::clone(&self.0))
    }
}

fn pixels() -> SharedPixels {
    SharedPixels(Arc::new(raikiri_traits::DecodedImage {
        width: 3,
        height: 3,
        rgba: (0..36).collect(),
    }))
}

fn repeated_scene(style: &str, source: &dyn ImagePixelSource) -> Scene {
    let html = format!(
        "<body style='margin:0'><div style='{style}'></div><div style='{style}'></div></body>"
    );
    scene_for_html(&html, source)
}

fn scene_for_html(html: &str, source: &dyn ImagePixelSource) -> Scene {
    scene_for_pages(html, source, 1, false)
}

fn scene_for_pages(
    html: &str,
    source: &dyn ImagePixelSource,
    pages: u32,
    paged_entry: bool,
) -> Scene {
    let mut parsed = raikiri_html::parse(
        html.as_bytes(),
        &raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let cascade = raikiri_html::build_cascaded(&parsed).unwrap();
    parsed
        .dom
        .prepare_list_marker_images(&cascade, source, None);
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let mut scene = Scene::new();
    for page in 0..pages {
        if paged_entry {
            crate::paint_single_page_with_origin_and_page_context_named_with_fixed_page_width_and_images(
                &mut scene, &parsed.dom, &cascade, PageBox::A4, 0.0,
                page, pages, false, None, None, PageBox::A4.width, source,
            ).unwrap();
            continue;
        }
        let warnings = crate::paint_single_page_with_images_and_warnings(
            &mut scene,
            &parsed.dom,
            &cascade,
            PageBox::A4,
            source,
        )
        .unwrap();
        assert!(warnings.is_empty());
    }
    scene
}

#[test]
fn page_calls_retain_shared_pixels_after_the_source_and_document_drop() {
    let source = pixels();
    let weak = Arc::downgrade(&source.0);
    let scene = scene_for_pages(
        "<body style='background:url(https://images.test/shared.png) no-repeat'><img src='https://images.test/shared.png' style='width:30px;height:30px'></body>",
        &source,
        2,
        true,
    );
    drop(source);
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 4);
    let original = weak.upgrade().unwrap();
    assert!(
        brushes
            .iter()
            .all(|brush| brush.image.data.data().as_ptr() == original.rgba.as_ptr())
    );
    assert_eq!(brushes[0].image.data.id(), brushes[1].image.data.id());
    assert_eq!(brushes[2].image.data.id(), brushes[3].image.data.id());
    drop(original);
    drop(scene);
    assert!(weak.upgrade().is_none());
}

#[test]
fn inline_border_slices_share_between_inline_boxes() {
    let source = pixels();
    let scene = scene_for_html(
        "<style>span {border:5px solid red;border-image:url(https://images.test/shared.png) 1 fill}</style><body><p><span>x</span><span>x</span></p></body>",
        &source,
    );
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 18);
    assert_eq!(
        brushes
            .iter()
            .map(|brush| brush.image.data.id())
            .collect::<HashSet<_>>()
            .len(),
        9
    );
}

struct ChangingPixels {
    images: [Arc<raikiri_traits::DecodedImage>; 2],
    request: std::cell::Cell<usize>,
}

impl ImagePixelSource for ChangingPixels {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<raikiri_traits::DecodedImage>> {
        Some(Arc::clone(&self.images[0]))
    }

    fn get_decoded_at_size(
        &self,
        _: &url::Url,
        _: ImageRasterSize,
        _: Option<u64>,
    ) -> Option<Arc<raikiri_traits::DecodedImage>> {
        let request = self.request.get();
        self.request.set(request + 1);
        Some(Arc::clone(&self.images[request % 2]))
    }
}

#[test]
fn one_url_and_size_can_supply_updated_pixels_with_a_new_owner() {
    let source = ChangingPixels {
        images: [
            Arc::new(raikiri_traits::DecodedImage {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
            }),
            Arc::new(raikiri_traits::DecodedImage {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 255, 255],
            }),
        ],
        request: std::cell::Cell::new(0),
    };
    let scene = repeated_scene(
        "width:30px;height:30px;background:url(https://images.test/shared.png) no-repeat",
        &source,
    );
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 2);
    assert_eq!(brushes[0].image.data.data(), &[255, 0, 0, 255]);
    assert_eq!(brushes[1].image.data.data(), &[0, 0, 255, 255]);
    assert_ne!(brushes[0].image.data.id(), brushes[1].image.data.id());
}

#[test]
fn prepared_content_images_keep_the_source_pixel_buffer() {
    let source = pixels();
    let scene = scene_for_html(
        "<style>div::before {content:url(https://images.test/shared.png)}</style><body><div>x</div><div>x</div></body>",
        &source,
    );
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 2);
    for brush in brushes {
        assert_eq!(brush.image.data.data().as_ptr(), source.0.rgba.as_ptr());
    }
}

#[test]
fn prepared_list_markers_keep_the_source_pixel_buffer() {
    let source = pixels();
    let scene = scene_for_html(
        "<body><ul style='list-style-image:url(https://images.test/shared.png)'><li>x</li><li>x</li></ul></body>",
        &source,
    );
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 2);
    for brush in brushes {
        assert_eq!(brush.image.data.data().as_ptr(), source.0.rgba.as_ptr());
    }
}

fn images(scene: &Scene) -> Vec<&peniko::ImageBrush> {
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => match &fill.brush {
                anyrender::Paint::Image(image) => Some(image),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn repeated_backgrounds_keep_one_shared_pixel_buffer() {
    let source = pixels();
    let scene = repeated_scene(
        "width:30px;height:30px;background:url(https://images.test/shared.png) no-repeat",
        &source,
    );
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 2);
    assert_eq!(
        brushes
            .iter()
            .map(|b| b.image.data.id())
            .collect::<HashSet<_>>()
            .len(),
        1
    );
    assert_eq!(
        brushes[0].image.data.data().as_ptr(),
        source.0.rgba.as_ptr()
    );
}

#[test]
fn repeated_border_slices_keep_one_buffer_per_slice() {
    let source = pixels();
    let scene = repeated_scene(
        "width:30px;height:30px;border:5px solid red;border-image:url(https://images.test/shared.png) 1 fill",
        &source,
    );
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 18);
    assert_eq!(
        brushes
            .iter()
            .map(|b| b.image.data.id())
            .collect::<HashSet<_>>()
            .len(),
        9
    );
    for (first, second) in brushes[..9].iter().zip(&brushes[9..]) {
        assert_eq!(first.image.data.data(), second.image.data.data());
        assert_eq!(first.image.data.id(), second.image.data.id());
    }
}

#[test]
fn an_unsliced_border_image_keeps_the_source_pixel_buffer() {
    let source = pixels();
    let scene = repeated_scene(
        "width:30px;height:30px;border:5px solid red;border-image:url(https://images.test/shared.png) 0 fill",
        &source,
    );
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 2);
    assert_eq!(
        brushes[0].image.data.data().as_ptr(),
        source.0.rgba.as_ptr()
    );
}

#[test]
fn ordinary_images_and_backgrounds_share_the_source_buffer() {
    let source = pixels();
    let scene = scene_for_html(
        "<body style='background:url(https://images.test/shared.png) no-repeat'><img src='https://images.test/shared.png' style='width:30px;height:30px'><img src='https://images.test/shared.png' style='width:40px;height:40px'></body>",
        &source,
    );
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 3);
    for brush in &brushes {
        assert_eq!(brush.image.data.data().as_ptr(), source.0.rgba.as_ptr());
        assert_eq!(brush.image.data.id(), brushes[0].image.data.id());
    }
}

#[test]
fn page_canvas_and_element_images_share_with_nonzero_page_margins() {
    let source = pixels();
    let scene = scene_for_html(
        "<style>@page {margin:5px;background:url(https://images.test/shared.png) no-repeat}</style><body style='background:url(https://images.test/shared.png) no-repeat'><img src='https://images.test/shared.png' style='width:30px;height:30px'></body>",
        &source,
    );
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 3);
    assert!(
        brushes
            .iter()
            .all(|brush| brush.image.data.id() == brushes[0].image.data.id())
    );
}

struct SizedPixels {
    small: Arc<raikiri_traits::DecodedImage>,
    large: Arc<raikiri_traits::DecodedImage>,
}

impl ImagePixelSource for SizedPixels {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<raikiri_traits::DecodedImage>> {
        Some(Arc::clone(&self.small))
    }

    fn get_decoded_at_size(
        &self,
        _: &url::Url,
        size: ImageRasterSize,
        _: Option<u64>,
    ) -> Option<Arc<raikiri_traits::DecodedImage>> {
        Some(Arc::clone(if size.width <= 30.0 {
            &self.small
        } else {
            &self.large
        }))
    }
}

#[test]
fn one_url_can_supply_different_rasters_at_different_sizes() {
    let source = SizedPixels {
        small: Arc::new(raikiri_traits::DecodedImage {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        }),
        large: Arc::new(raikiri_traits::DecodedImage {
            width: 2,
            height: 2,
            rgba: [0, 0, 255, 255].repeat(4),
        }),
    };
    let scene = scene_for_html(
        "<body><img src='https://images.test/shared.png' style='width:30px;height:30px'><img src='https://images.test/shared.png' style='width:40px;height:40px'></body>",
        &source,
    );
    let brushes = images(&scene);
    assert_eq!(brushes.len(), 2);
    assert_eq!((brushes[0].image.width, brushes[1].image.width), (1, 2));
    assert_eq!(brushes[0].image.data.data(), &[255, 0, 0, 255]);
    assert_eq!(brushes[1].image.data.data(), &[0, 0, 255, 255].repeat(4));
    assert_ne!(brushes[0].image.data.id(), brushes[1].image.data.id());
}
