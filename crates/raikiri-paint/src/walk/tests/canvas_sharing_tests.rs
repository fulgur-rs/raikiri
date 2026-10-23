use super::*;
use std::sync::Arc;

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
fn canvas_page_paints_share_pixels_and_preserve_prior_scenes_after_updates() {
    let mut parsed = raikiri_html::parse(
        b"<body style='margin:0'><canvas width='2' height='2' style='width:20px;height:20px'></canvas></body>".as_slice(),
        &raikiri_html::ParseOptions { extra_stylesheets: &[], network: None, base_url: None },
    ).unwrap();
    let id = (0..parsed.dom.node_count())
        .find(|&i| parsed.dom.is_canvas_element(i))
        .unwrap();
    assert!(
        parsed
            .dom
            .canvas_fill_rect(id, 0, 0, 2, 2, [255, 0, 0, 255])
    );
    let cascade = raikiri_html::build_cascaded(&parsed).unwrap();
    raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, PageBox::A4).unwrap();
    let original = parsed.dom.canvas_bitmap_snapshot(id).unwrap();
    let weak = Arc::downgrade(&original);
    let mut old_scene = Scene::new();
    for _ in 0..2 {
        crate::paint_single_page(&mut old_scene, &parsed.dom, &cascade, PageBox::A4).unwrap();
    }
    let old_images = images(&old_scene);
    assert_eq!(old_images.len(), 2);
    for image in old_images {
        assert_eq!(image.image.data.data().as_ptr(), original.rgba.as_ptr());
        assert_eq!(image.image.data.data(), original.rgba);
    }
    assert!(
        parsed
            .dom
            .canvas_fill_rect(id, 0, 0, 2, 2, [0, 0, 255, 255])
    );
    let mut new_scene = Scene::new();
    crate::paint_single_page(&mut new_scene, &parsed.dom, &cascade, PageBox::A4).unwrap();
    assert_eq!(
        &images(&new_scene)[0].image.data.data()[..4],
        &[0, 0, 255, 255]
    );
    parsed.dom.set_element_attribute(id, "width", "2").unwrap();
    parsed.dom.set_element_attribute(id, "height", "3").unwrap();
    drop(parsed);
    drop(original);
    assert!(weak.upgrade().is_some());
    assert_eq!(
        &images(&old_scene)[0].image.data.data()[..4],
        &[255, 0, 0, 255]
    );
    let old_rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(old_scene, Affine::IDENTITY),
        20,
        20,
    );
    let new_rgba = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(new_scene, Affine::IDENTITY),
        20,
        20,
    );
    let center = (10 * 20 + 10) * 4;
    assert_eq!(&old_rgba[center..center + 4], &[255, 0, 0, 255]);
    assert_eq!(&new_rgba[center..center + 4], &[0, 0, 255, 255]);
    assert!(weak.upgrade().is_none());
}
