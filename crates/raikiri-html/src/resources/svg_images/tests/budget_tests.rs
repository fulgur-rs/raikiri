use super::*;
use crate::resources::image_cache::{DecodedImageCache, ProcessImageBudget};
use raikiri_traits::{ImageRasterSize, vector::VectorImage};

mod concurrency_tests;

fn key_bytes(bytes: &[u8]) -> u64 {
    DATA_ENTRY_METADATA_BYTES + "image/png".len() as u64 + bytes.len() as u64
}

fn fresh() -> Arc<DecodedImage> {
    Arc::new(DecodedImage::new(1, 1, vec![255, 0, 0, 255]))
}

fn isolated(limit: u64, process_limit: u64) -> (RenderResources<'static>, Arc<ProcessImageBudget>) {
    let process = Arc::new(ProcessImageBudget::new(process_limit, 4));
    let resources = RenderResources::new().with_test_process_budget(Arc::clone(&process));
    resources.image_cache.lock().unwrap().max_bytes = limit;
    (resources, process)
}

fn used(resources: &RenderResources<'_>) -> u64 {
    resources
        .image_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .inline_bytes
}

#[test]
fn hits_and_source_drop_share_one_reservation_while_callers_outlive_it() {
    let bytes = b"encoded";
    let cost = key_bytes(bytes) + 4;
    let (resources, process) = isolated(cost, u64::MAX);
    let images = resources.svg_images(None);
    let first = images
        .data_with("image/png", bytes, |_, _, _| Ok(fresh()))
        .unwrap();
    let again = images
        .data_with("image/png", bytes, |_, _, _| {
            panic!("memo hit decoded again")
        })
        .unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(used(&resources), cost);
    assert_eq!(process.retained(), cost);
    assert_eq!(*resources.budget.lock().unwrap(), bytes.len() as u64);
    drop(again);
    let weak = Arc::downgrade(&first);
    drop(images);
    assert_eq!(used(&resources), 0);
    assert_eq!(process.retained(), 0);
    assert_eq!(weak.strong_count(), 1);
    drop(first);
    assert_eq!(weak.strong_count(), 0);
}

#[test]
fn key_rejection_and_aggregate_rejection_do_not_keep_or_charge_an_entry() {
    let bytes = b"encoded";
    for process_denial in [false, true] {
        let (resources, process) = if process_denial {
            isolated(u64::MAX, key_bytes(bytes) - 1)
        } else {
            isolated(key_bytes(bytes) - 1, u64::MAX)
        };
        let images = resources.svg_images(None);
        for _ in 0..2 {
            assert!(
                images
                    .data_with("image/png", bytes, |_, _, _| panic!(
                        "a rejected key decoded"
                    ))
                    .is_none()
            );
        }
        assert!(images.state().data.is_empty());
        assert_eq!(used(&resources), 0);
        assert_eq!(process.retained(), 0);
        assert_eq!(*resources.budget.lock().unwrap(), 0);
        assert!(!images.take_warnings().is_empty());
    }
    let (resources, process) = isolated(u64::MAX, u64::MAX);
    let resources = resources
        .resource_limits(crate::ResourceLimits::new().max_aggregate_resource_bytes(Some(0)));
    let images = resources.svg_images(None);
    for _ in 0..2 {
        assert!(
            images
                .data_with("image/png", bytes, |_, _, _| panic!(
                    "an over-aggregate key decoded"
                ))
                .is_none()
        );
    }
    assert!(images.state().data.is_empty());
    assert_eq!(used(&resources), 0);
    assert_eq!(process.retained(), 0);
    assert_eq!(*resources.budget.lock().unwrap(), 0);
    let (resources, process) = isolated(key_bytes(bytes), u64::MAX);
    let images = resources.svg_images(None);
    assert!(
        images
            .data_with(&"x".repeat(4096), bytes, |_, _, _| panic!(
                "a huge MIME key decoded"
            ))
            .is_none()
    );
    assert_eq!(process.retained(), 0);
    assert!(images.state().data.is_empty());
}

#[test]
fn failed_decode_keeps_only_its_checked_key_and_never_charges_again() {
    let bytes = b"invalid encoded image";
    let (resources, process) = isolated(key_bytes(bytes), u64::MAX);
    let images = resources.svg_images(None);
    assert!(
        images
            .data_with("image/png", bytes, |_, _, limit| {
                assert_eq!(limit, 0);
                Err(Box::new(RenderWarning::new(
                    WarningKind::ResourceFallback {
                        kind: ResourceKind::Image,
                        url: None,
                    },
                    "injected decode failure".into(),
                )))
            })
            .is_none()
    );
    assert!(
        images
            .data_with("image/png", bytes, |_, _, _| panic!(
                "failed outcome decoded again"
            ))
            .is_none()
    );
    assert_eq!(images.state().attempts, 1);
    assert_eq!(images.state().data.len(), 1);
    assert_eq!(used(&resources), key_bytes(bytes));
    assert_eq!(process.retained(), key_bytes(bytes));
    assert_eq!(*resources.budget.lock().unwrap(), bytes.len() as u64);
    drop(images);
    assert_eq!(process.retained(), 0);
}

#[test]
fn inline_does_not_take_replacement_credit_from_the_data_stub_url() {
    let bytes = b"encoded";
    let (resources, process) = isolated(key_bytes(bytes) + 7, u64::MAX);
    let stub = Url::parse("data:,").unwrap();
    resources
        .cache_image_source(stub.clone(), CachedBackgroundSource::Raster(fresh()))
        .unwrap();
    let images = resources.svg_images(None);
    assert!(
        images
            .data_with("image/png", bytes, |_, _, limit| {
                assert_eq!(limit, 3);
                Ok(fresh())
            })
            .is_none()
    );
    assert_eq!(used(&resources), key_bytes(bytes));
    assert_eq!(process.retained(), key_bytes(bytes) + 4);
    assert_eq!(resources.image_cache_effective_remaining(&stub), 7);
    assert!(
        images
            .data_with("image/png", bytes, |_, _, _| panic!(
                "pixel rejection retried"
            ))
            .is_none()
    );
    assert_eq!(*resources.budget.lock().unwrap(), bytes.len() as u64);
    drop(images);
    assert_eq!(process.retained(), 4);
    drop(resources);
    assert_eq!(process.retained(), 0);
}

#[test]
fn inline_reservations_limit_background_source_raster_and_scene_admission() {
    let bytes = b"encoded";
    let cost = key_bytes(bytes) + 4;
    let (resources, process) = isolated(cost, u64::MAX);
    let images = resources.svg_images(None);
    let _image = images
        .data_with("image/png", bytes, |_, _, _| Ok(fresh()))
        .unwrap();
    let url = Url::parse("https://images.test/p.png").unwrap();
    assert_eq!(resources.image_cache_effective_remaining(&url), 0);
    assert!(
        resources
            .cache_image_source(url.clone(), CachedBackgroundSource::Raster(fresh()))
            .is_err()
    );
    assert!(
        resources
            .cache_image_raster(
                url.clone(),
                CachedBackgroundSource::Raster(fresh()),
                ImageRasterSize {
                    width: 1.0,
                    height: 1.0
                },
                fresh()
            )
            .is_err()
    );
    let size = ImageRasterSize {
        width: 1.0,
        height: 1.0,
    };
    resources.cache_image_scene(url.clone(), size, Arc::new(VectorImage::new((1.0, 1.0))));
    assert!(resources.cached_scene(&url, size).is_none());
    assert_eq!(process.retained(), cost);
    drop(images);
    assert_eq!(process.retained(), 0);
    resources
        .cache_image_source(url, CachedBackgroundSource::Raster(fresh()))
        .unwrap();
    assert_eq!(process.retained(), 4);
}

#[test]
fn inline_bytes_are_in_scene_eviction_and_reverse_admission() {
    let bytes = b"encoded";
    let cost = key_bytes(bytes) + 4;
    let urls = [
        "https://images.test/a.svg",
        "https://images.test/b.svg",
        "https://images.test/c.svg",
    ]
    .map(|url| Url::parse(url).unwrap());
    let size = ImageRasterSize {
        width: 1.0,
        height: 1.0,
    };
    let scene = Arc::new(VectorImage::new((1.0, 1.0)));
    let scene_cost = DecodedImageCache::scene_bytes(&urls[0], &scene);
    let (resources, process) = isolated(cost + 2 * scene_cost, u64::MAX);
    let images = resources.svg_images(None);
    let _image = images
        .data_with("image/png", bytes, |_, _, _| Ok(fresh()))
        .unwrap();
    for url in &urls {
        resources.cache_image_scene(url.clone(), size, Arc::clone(&scene));
    }
    assert!(resources.cached_scene(&urls[0], size).is_none());
    assert!(resources.cached_scene(&urls[1], size).is_some());
    assert!(resources.cached_scene(&urls[2], size).is_some());
    assert_eq!(process.retained(), cost + 2 * scene_cost);
    drop(images);
    assert_eq!(process.retained(), 2 * scene_cost);

    let (resources, process) = isolated(scene_cost + key_bytes(bytes) + 3, u64::MAX);
    resources.cache_image_scene(urls[0].clone(), size, scene);
    let images = resources.svg_images(None);
    assert!(
        images
            .data_with("image/png", bytes, |_, _, limit| {
                assert_eq!(limit, 3);
                Ok(fresh())
            })
            .is_none()
    );
    assert_eq!(process.retained(), scene_cost + key_bytes(bytes));
}

#[test]
fn clones_and_reconfigured_caches_refund_their_original_reservations() {
    let bytes = b"encoded";
    let cost = key_bytes(bytes) + 4;
    let (resources, process) = isolated(2 * cost, 3 * cost);
    let clone = resources.clone();
    let changed = resources
        .clone()
        .resource_limits(crate::ResourceLimits::new());
    assert!(Arc::ptr_eq(&resources.image_cache, &clone.image_cache));
    assert!(!Arc::ptr_eq(&resources.image_cache, &changed.image_cache));
    let a = resources.svg_images(None);
    let b = clone.svg_images(None);
    let c = changed.svg_images(None);
    let _a = a
        .data_with("image/png", bytes, |_, _, _| Ok(fresh()))
        .unwrap();
    let _b = b
        .data_with("image/png", bytes, |_, _, _| Ok(fresh()))
        .unwrap();
    let _c = c
        .data_with("image/png", bytes, |_, _, _| Ok(fresh()))
        .unwrap();
    assert_eq!(used(&resources), 2 * cost);
    assert_eq!(used(&changed), cost);
    assert_eq!(process.retained(), 3 * cost);
    drop(a);
    assert_eq!(used(&resources), cost);
    assert_eq!(used(&changed), cost);
    assert_eq!(process.retained(), 2 * cost);
    drop(b);
    drop(c);
    assert_eq!(process.retained(), 0);
}

#[test]
fn a_poisoned_cache_still_refunds_complete_memo_reservations() {
    let bytes = b"encoded";
    let (resources, process) = isolated(u64::MAX, u64::MAX);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _cache = resources.image_cache.lock().unwrap();
        panic!("inject cache poison");
    }));
    let images = resources.svg_images(None);
    let image = images
        .data_with("image/png", bytes, |_, _, _| Ok(fresh()))
        .unwrap();
    assert_eq!(process.retained(), key_bytes(bytes) + 4);
    drop(images);
    assert_eq!(process.retained(), 0);
    assert_eq!(image.rgba, [255, 0, 0, 255]);
}
