use super::*;

fn distant_shadow() -> &'static str {
    "<!doctype html><style>body{margin:0}.scaled{transform:scale(.001);transform-origin:0 0}.columns{columns:3;column-fill:auto;height:auto;orphans:3;widows:3;font:16px/1 Ahem}p{margin:0}.shadow{font:10000px/100px Ahem;color:transparent;text-shadow:200000px 5000px blue}</style><div class=scaled><div class=columns><div style='column-span:all;height:0'></div><p>a<br>b<br>c</p><p class=shadow>X<br>X<br>X<br>X<br>X<br>X<br>X<br></p></div></div>"
}

#[test]
fn column_fragments_keep_distant_shadow_inside_a_scaled_page() {
    let scene = paged_scene(distant_shadow(), None);
    let clips = clip_layer_bounds(&scene);
    assert!(!clips.is_empty());
    assert!(clips.iter().all(|clip| {
        [clip.x0, clip.y0, clip.x1, clip.y1]
            .into_iter()
            .all(f64::is_finite)
    }));
    assert!(clips.iter().all(|clip| clip.x1 >= 800.0));
    let image = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        800,
        600,
    );
    assert_eq!(&image[(5 * 800 + 205) * 4..][..4], &[0, 0, 255, 255]);
}

#[test]
fn open_fragment_clips_are_bounded_by_the_current_transformed_page() {
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    let mut clip =
        raikiri_traits::PaintClip::new(raikiri_traits::PaintRect::new(0.0, 20.0, 0.0, 40.0));
    clip.clip_x = false;
    for transform in [
        Affine::IDENTITY,
        Affine::scale(0.001),
        Affine::scale(-0.001),
        Affine::rotate(0.5),
        Affine::scale(1e-150),
        Affine::scale(1e150),
    ] {
        let bounds = fragment_clip_bounds(clip, (0.0, 0.0), transform, &page);
        let page_bounds = page_bounds_in_scene(transform, &page);
        assert_eq!((bounds.x0, bounds.x1), (page_bounds.x0, page_bounds.x1));
        assert!(bounds.y0 >= page_bounds.y0 && bounds.y1 <= page_bounds.y1);
        assert!(bounds.y0 >= 20.0 || bounds.height() == 0.0);
        assert!(bounds.y1 <= 60.0 || bounds.height() == 0.0);
        assert!(
            [bounds.x0, bounds.y0, bounds.x1, bounds.y1]
                .into_iter()
                .all(f64::is_finite)
        );
    }
    for transform in [
        Affine::scale(0.0),
        Affine::scale(1e-310),
        Affine::scale(f64::INFINITY),
        Affine::scale(f64::NAN),
        Affine::new([1e-300, 0.0, 0.0, 1.0, 1e300, 0.0]),
    ] {
        assert_eq!(
            fragment_clip_bounds(clip, (0.0, 0.0), transform, &page),
            Rect::ZERO
        );
    }
}

#[test]
fn fragment_clip_lowering_keeps_empty_and_off_page_clips_empty() {
    let page = PageBox::new();
    for rect in [
        raikiri_traits::PaintRect::new(10.0, 20.0, 0.0, 40.0),
        raikiri_traits::PaintRect::new(-100.0, 20.0, 10.0, 40.0),
        raikiri_traits::PaintRect::new(f32::NAN, 20.0, 10.0, 40.0),
        raikiri_traits::PaintRect::new(0.0, f32::NAN, 10.0, 40.0),
        raikiri_traits::PaintRect::new(0.0, 20.0, f32::INFINITY, 40.0),
    ] {
        let clip = raikiri_traits::PaintClip::new(rect);
        assert_eq!(
            fragment_clip_bounds(clip, (0.0, 0.0), Affine::IDENTITY, &page).area(),
            0.0
        );
    }
}

#[test]
fn a_fragment_open_on_both_axes_uses_only_the_page_bounds() {
    let page = PageBox::new();
    let mut clip = raikiri_traits::PaintClip::new(raikiri_traits::PaintRect::new(
        f32::NAN,
        f32::NAN,
        0.0,
        0.0,
    ));
    clip.clip_x = false;
    clip.clip_y = false;
    let transform = Affine::scale(0.001);
    assert_eq!(
        fragment_clip_bounds(clip, (0.0, 0.0), transform, &page),
        page_bounds_in_scene(transform, &page)
    );
}

#[test]
fn snapping_does_not_reopen_empty_or_off_page_fragment_clips() {
    let page = PageBox::new();
    for rect in [
        raikiri_traits::PaintRect::new(10.5, 20.5, 0.0, 40.0),
        raikiri_traits::PaintRect::new(10.5, 20.5, 40.0, 0.0),
        raikiri_traits::PaintRect::new(-100.0, 20.5, 10.0, 40.0),
    ] {
        let clip = raikiri_traits::PaintClip::new(rect);
        let bounds = fragment_clip_bounds(clip, (0.0, 0.0), Affine::IDENTITY, &page);
        let snapped = snap_column_clip_outward(
            bounds,
            Some(DeviceGrid::new(1.0, 0.5, 1.0, 0.5)),
            OpenBlockEdges {
                top: true,
                bottom: true,
            },
        );
        assert_eq!(snapped.area(), 0.0);
    }
}
