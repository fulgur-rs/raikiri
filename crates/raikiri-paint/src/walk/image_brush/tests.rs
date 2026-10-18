use super::*;

fn decoded() -> Arc<DecodedImage> {
    Arc::new(DecodedImage {
        width: 4,
        height: 2,
        rgba: (0..32).collect(),
    })
}

#[test]
fn recorded_pixels_survive_cache_drop_and_source_mutation() {
    let mut source = decoded();
    let cache = ImageBrushCache::default();
    let brush = cache.whole(&source);
    drop(cache);
    Arc::make_mut(&mut source).rgba.fill(255);
    assert_eq!(brush.image.data.data(), &(0..32).collect::<Vec<u8>>());
    assert_eq!(source.rgba, vec![255; 32]);
}

#[test]
fn isolated_crops_share_only_the_same_owner_and_rectangle() {
    let first = decoded();
    let second = Arc::new(DecodedImage {
        rgba: vec![255; 32],
        ..first.as_ref().clone()
    });
    let cache = ImageBrushCache::default();
    let crop = PixelCrop {
        x0: 1,
        y0: 0,
        x1: 2,
        y1: 2,
    };
    let brush = cache.crop(&first, crop).unwrap();
    assert_eq!(brush.image.data.data(), &[4, 5, 6, 7, 20, 21, 22, 23]);
    assert_eq!(
        brush.image.data.id(),
        cache.crop(&first, crop).unwrap().image.data.id()
    );
    let other = cache.crop(&second, crop).unwrap();
    assert_eq!(other.image.data.data(), &[255; 8]);
    assert_ne!(brush.image.data.id(), other.image.data.id());
    let neighbour = cache
        .crop(
            &first,
            PixelCrop {
                x0: 2,
                x1: 3,
                ..crop
            },
        )
        .unwrap();
    assert_eq!(neighbour.image.data.data(), &[8, 9, 10, 11, 24, 25, 26, 27]);
    assert_ne!(brush.image.data.id(), neighbour.image.data.id());
}

#[test]
fn crop_owners_are_released_with_the_paint_cache() {
    let source = decoded();
    let weak = Arc::downgrade(&source);
    let cache = ImageBrushCache::default();
    let brush = cache
        .crop(
            &source,
            PixelCrop {
                x0: 1,
                y0: 0,
                x1: 2,
                y1: 2,
            },
        )
        .unwrap();
    drop(source);
    assert!(weak.upgrade().is_some());
    drop(cache);
    assert!(weak.upgrade().is_none());
    assert_eq!(brush.image.data.data(), &[4, 5, 6, 7, 20, 21, 22, 23]);
}

#[test]
fn a_whole_brush_does_not_hide_missing_crop_pixels() {
    let source = Arc::new(DecodedImage {
        width: 4,
        height: 2,
        rgba: vec![255; 8],
    });
    let cache = ImageBrushCache::default();
    let _ = cache.whole(&source);
    assert!(cache.crop(&source, PixelCrop::whole(&source)).is_none());
    assert!(
        cache
            .crop(
                &source,
                PixelCrop {
                    x0: 1,
                    y0: 1,
                    x1: 2,
                    y1: 2
                }
            )
            .is_none()
    );
}

#[test]
fn the_device_grid_follows_the_root_and_paint_transforms() {
    let cache = ImageBrushCache::default();
    // A vector output has no device pixels, nor does an invalid scale.
    assert_eq!(cache.device_grid(Affine::IDENTITY), None);
    for invalid in [-1.0, 0.0, f32::NAN] {
        let cache = ImageBrushCache::default().with_device_scale(Some(invalid));
        assert_eq!(cache.device_grid(Affine::IDENTITY), None);
    }
    let cache = cache.with_device_scale(Some(2.0));
    cache.set_root_transform(Affine::translate((0.25, 0.0)));
    assert_eq!(
        cache.device_grid(Affine::scale_non_uniform(1.5, 3.0)),
        Some(DeviceGrid {
            scale_x: 3.0,
            offset_x: 0.5,
            scale_y: 6.0,
            offset_y: 0.0,
        })
    );
    // A rotation leaves no axis-aligned grid to snap to.
    assert_eq!(cache.device_grid(Affine::rotate(0.5)), None);
}

fn row(pixels: &[[u8; 4]]) -> DecodedImage {
    DecodedImage {
        width: pixels.len() as u32,
        height: 1,
        rgba: pixels.concat(),
    }
}

#[test]
fn averaging_weighs_each_pixel_by_the_area_a_result_pixel_covers() {
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let blue = [0, 0, 255, 255];
    let image = row(&[red, green, blue, blue]);
    // Whole pixels: two to one result pixel.
    assert_eq!(
        box_average(&image, [0.0, 0.0, 4.0, 1.0], (2, 1)).unwrap(),
        [128, 128, 0, 255, 0, 0, 255, 255]
    );
    // A region that starts and ends inside a pixel counts those pixels in part.
    assert_eq!(
        box_average(&image, [0.5, 0.0, 2.5, 1.0], (1, 1)).unwrap(),
        [64, 128, 64, 255]
    );
    // A seam inside one result pixel mixes in proportion to its position:
    // three quarters of a pixel on each side of it.
    assert_eq!(
        box_average(&image, [0.25, 0.0, 1.75, 1.0], (1, 1)).unwrap(),
        [128, 128, 0, 255]
    );
    assert_eq!(
        box_average(&image, [0.5, 0.0, 1.25, 1.0], (1, 1)).unwrap(),
        [170, 85, 0, 255]
    );
    // The rows are averaged the same way.
    let column = DecodedImage {
        width: 1,
        height: 4,
        rgba: [red, red, blue, blue].concat(),
    };
    assert_eq!(
        box_average(&column, [0.0, 0.0, 1.0, 4.0], (1, 1)).unwrap(),
        [128, 0, 128, 255]
    );
    assert_eq!(
        box_average(&column, [0.0, 1.0, 1.0, 3.0], (1, 2)).unwrap(),
        [255, 0, 0, 255, 0, 0, 255, 255]
    );
}

#[test]
fn averaging_runs_over_both_axes_at_once() {
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let blue = [0, 0, 255, 255];
    let image = DecodedImage {
        width: 2,
        height: 2,
        rgba: [red, red, blue, green].concat(),
    };
    let whole = [0.0, 0.0, 2.0, 2.0];
    assert_eq!(
        box_average(&image, whole, (1, 1)).unwrap(),
        [128, 64, 64, 255]
    );
    // Only the rows are averaged together: each column keeps its own colors.
    assert_eq!(
        box_average(&image, whole, (2, 1)).unwrap(),
        [128, 0, 128, 255, 128, 128, 0, 255]
    );
    assert_eq!(
        box_average(&image, whole, (1, 2)).unwrap(),
        [255, 0, 0, 255, 0, 128, 128, 255]
    );
}

#[test]
fn averaging_a_region_past_the_image_uses_the_pixels_inside() {
    let image = row(&[[255, 0, 0, 255], [0, 255, 0, 255]]);
    assert_eq!(
        box_average(&image, [1.0, 0.0, 3.0, 1.0], (1, 1)).unwrap(),
        [0, 255, 0, 255]
    );
    assert_eq!(
        box_average(&image, [-1.0, 0.0, 1.0, 1.0], (1, 1)).unwrap(),
        [255, 0, 0, 255]
    );
    // A result pixel that lies wholly past the image has nothing to average.
    assert_eq!(
        box_average(&image, [1.0, 0.0, 3.0, 1.0], (2, 1)).unwrap(),
        [0, 255, 0, 255, 0, 0, 0, 0]
    );
    let column = DecodedImage {
        width: 1,
        height: 2,
        rgba: image.rgba.clone(),
    };
    assert_eq!(
        box_average(&column, [0.0, 1.0, 1.0, 3.0], (1, 2)).unwrap(),
        [0, 255, 0, 255, 0, 0, 0, 0]
    );
}

#[test]
fn averaging_colors_with_their_alpha() {
    // A transparent pixel adds no color: half the area is opaque white.
    let image = row(&[[255, 255, 255, 255], [9, 99, 199, 0]]);
    assert_eq!(
        box_average(&image, [0.0, 0.0, 2.0, 1.0], (1, 1)).unwrap(),
        [255, 255, 255, 128]
    );
    // Nothing opaque at all stays transparent.
    let clear = row(&[[9, 99, 199, 0], [9, 99, 199, 0]]);
    assert_eq!(
        box_average(&clear, [0.0, 0.0, 2.0, 1.0], (1, 1)).unwrap(),
        [0, 0, 0, 0]
    );
}

#[test]
fn averaging_gives_nothing_without_pixels_to_average() {
    let image = row(&[[1, 2, 3, 255], [4, 5, 6, 255]]);
    let whole = [0.0, 0.0, 2.0, 1.0];
    assert!(box_average(&image, whole, (0, 1)).is_none());
    assert!(box_average(&image, whole, (1, 0)).is_none());
    // An empty, inverted, non-finite or outside region.
    for region in [
        [1.0, 0.0, 1.0, 1.0],
        [2.0, 0.0, 1.0, 1.0],
        [0.0, 0.0, f64::NAN, 1.0],
        [0.0, 0.0, 2.0, f64::INFINITY],
        [2.0, 0.0, 3.0, 1.0],
        [-2.0, 0.0, 0.0, 1.0],
    ] {
        assert!(box_average(&image, region, (1, 1)).is_none(), "{region:?}");
    }
    // A buffer shorter than its size says.
    let short = DecodedImage {
        rgba: vec![0; 4],
        ..image
    };
    assert!(box_average(&short, whole, (1, 1)).is_none());
}

#[test]
fn averaged_brushes_are_shared_per_image_region_and_size() {
    let source = decoded();
    let cache = ImageBrushCache::default();
    let region = [0.0, 0.0, 4.0, 2.0];
    let brush = cache.averaged(&source, region, (2, 1)).unwrap();
    assert_eq!((brush.image.width, brush.image.height), (2, 1));
    assert_eq!(
        brush.image.data.id(),
        cache
            .averaged(&source, region, (2, 1))
            .unwrap()
            .image
            .data
            .id()
    );
    let smaller = cache.averaged(&source, region, (1, 1)).unwrap();
    assert_ne!(brush.image.data.id(), smaller.image.data.id());
    let shifted = cache
        .averaged(&source, [0.0, 0.0, 3.0, 2.0], (2, 1))
        .unwrap();
    assert_ne!(brush.image.data.id(), shifted.image.data.id());
    // Nothing to average gives no brush, and keeps none.
    assert!(cache.averaged(&source, region, (0, 1)).is_none());
    // The brush owns its pixels, so the paint cache can go.
    let weak = Arc::downgrade(&source);
    drop(source);
    assert!(weak.upgrade().is_some());
    drop(cache);
    assert!(weak.upgrade().is_none());
    assert_eq!((brush.image.width, brush.image.height), (2, 1));
}
