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
