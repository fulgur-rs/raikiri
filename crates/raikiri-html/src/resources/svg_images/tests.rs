use super::*;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Work {
    comparisons: usize,
    candidate_bytes: usize,
    hashes: usize,
    hashed_input_bytes: usize,
}

thread_local! {
    static WORK: Cell<Work> = Cell::new(Work::default());
}

pub(super) fn record_comparison(bytes: usize) {
    WORK.with(|work| {
        let mut value = work.get();
        value.comparisons += 1;
        value.candidate_bytes += bytes;
        work.set(value);
    });
}

pub(super) fn record_hash(bytes: usize) {
    WORK.with(|work| {
        let mut value = work.get();
        value.hashes += 1;
        value.hashed_input_bytes += bytes;
        work.set(value);
    });
}

fn measured<T>(run: impl FnOnce() -> T) -> (T, Work) {
    WORK.set(Work::default());
    let value = run();
    (value, WORK.get())
}

fn populate(images: &RenderSvgImages<'_, '_>, count: usize) -> Vec<(Vec<u8>, Arc<DecodedImage>)> {
    let pictures: Vec<_> = (0..count)
        .map(|index| {
            let mut bytes = vec![b'x'; 16 * 1024];
            bytes.extend(format!("{index:08}").as_bytes());
            (
                bytes,
                Arc::new(DecodedImage::new(1, 1, vec![255, 0, 0, 255])),
            )
        })
        .collect();
    let mut state = images.state();
    for (bytes, image) in &pictures {
        state.remember_data("image/png", bytes, Some(Arc::clone(image)));
    }
    state.attempts = count;
    pictures
}

#[test]
fn tail_hit_filters_full_payload_comparisons_independently_of_candidate_count() {
    for count in [17, 64, 256] {
        let resources = RenderResources::new();
        let images = resources.svg_images(None);
        let pictures = populate(&images, count);
        let (bytes, expected) = pictures.last().unwrap();
        let copied = bytes.clone();
        assert_ne!(copied.as_ptr(), bytes.as_ptr());
        let (image, work) = measured(|| images.data("image/png", &copied).unwrap());
        assert!(Arc::ptr_eq(&image, expected));
        assert!(work.comparisons <= 2, "count={count}, work={work:?}");
        assert!(work.candidate_bytes <= copied.len() * 2);
        assert_eq!(work.hashes, 1);
        assert_eq!(work.hashed_input_bytes, copied.len() + "image/png".len());
        assert!(images.take_warnings().is_empty());
    }
}

#[test]
fn first_and_single_entry_hits_keep_one_full_comparison() {
    for count in [1, 256] {
        let resources = RenderResources::new();
        let images = resources.svg_images(None);
        let pictures = populate(&images, count);
        let (bytes, expected) = &pictures[0];
        let (image, work) = measured(|| images.data("image/png", bytes).unwrap());
        assert!(Arc::ptr_eq(&image, expected));
        assert_eq!(
            work,
            Work {
                comparisons: 1,
                candidate_bytes: bytes.len(),
                hashes: 0,
                hashed_input_bytes: 0,
            }
        );
    }
}

#[test]
fn forged_collisions_after_first_entry_still_check_mime_and_complete_bytes() {
    let resources = RenderResources::new();
    let images = resources.svg_images(None);
    let pictures = populate(&images, 32);
    let requested = pictures[31].0.clone();
    {
        let mut state = images.state();
        let fingerprint = state.fingerprint("image/png", &requested);
        state.data[1].fingerprint = fingerprint;
        state.data[1].image = None;
        state.data[2].fingerprint = fingerprint;
        state.data[2].bytes = requested.clone();
        state.data[2].mime = "image/gif".into();
    }
    let (image, work) = measured(|| images.data("image/png", &requested).unwrap());
    assert!(Arc::ptr_eq(&image, &pictures[31].1));
    assert_eq!(work.hashes, 1);
    assert_eq!(work.comparisons, 3);
    assert!(images.take_warnings().is_empty());
}

#[test]
fn same_bytes_with_different_mime_and_negative_results_remain_distinct() {
    let resources = RenderResources::new();
    let images = resources.svg_images(None);
    let _other_pictures = populate(&images, 32);
    let expected = Arc::new(DecodedImage::new(1, 1, vec![1, 2, 3, 255]));
    let bytes = b"encoded";
    {
        let mut state = images.state();
        state.remember_data("image/png", bytes, None);
        state.remember_data("image/gif", bytes, Some(Arc::clone(&expected)));
        state.attempts = 34;
    }
    assert!(images.data("image/png", bytes).is_none());
    assert!(Arc::ptr_eq(
        &images.data("image/gif", bytes).unwrap(),
        &expected
    ));
    assert_eq!(images.state().attempts, 34);
    assert_eq!(images.state().data.len(), 34);
    assert!(images.take_warnings().is_empty());
}

#[test]
fn impossible_encoded_or_mime_lengths_do_not_hash_during_lookup() {
    let resources = RenderResources::new();
    let images = resources.svg_images(None);
    let pictures = populate(&images, 256);
    let impossible = vec![0; 2 * 1024 * 1024];
    for (mime, bytes) in [
        ("image/png", impossible.as_slice()),
        ("image/longer-mime", pictures[0].0.as_slice()),
    ] {
        let state = images.state();
        let (found, work) = measured(|| state.cached_data(mime, bytes));
        assert!(found.is_none());
        assert_eq!(work, Work::default());
    }
}

#[test]
fn cached_tail_is_available_after_request_limit_and_abort() {
    use raikiri_traits::AbortController;

    let resources = RenderResources::new();
    let controller = AbortController::new();
    let images = resources
        .svg_images(None)
        .abort_signal(controller.signal.clone());
    let pictures = populate(&images, 256);
    controller.abort();
    let (bytes, expected) = pictures.last().unwrap();
    assert!(Arc::ptr_eq(
        &images.data("image/png", bytes).unwrap(),
        expected
    ));
    assert!(images.take_warnings().is_empty());
    for _ in 0..2 {
        assert!(images.data("image/png", b"unknown").is_none());
    }
    assert_eq!(images.state().data.len(), 256);
    assert_eq!(images.state().attempts, 257);
    let warnings = images.take_warnings();
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        warnings[0].kind,
        WarningKind::ResourceLimitExceeded {
            limit: 256,
            actual: 257,
            ..
        }
    ));
}

#[test]
fn rejection_and_abort_do_not_remember_unchecked_data() {
    use crate::ResourceLimits;
    use raikiri_traits::AbortController;

    let resources =
        RenderResources::new().resource_limits(ResourceLimits::new().max_resource_bytes(Some(0)));
    let images = resources.svg_images(None);
    for _ in 0..2 {
        assert!(images.data("image/png", b"x").is_none());
    }
    assert_eq!(images.state().attempts, 2);
    assert!(images.state().data.is_empty());
    assert!(!images.take_warnings().is_empty());
    let resources = RenderResources::new();
    let controller = AbortController::new();
    let images = resources
        .svg_images(None)
        .abort_signal(controller.signal.clone());
    controller.abort();
    assert!(images.data("image/png", b"x").is_none());
    assert_eq!(images.state().attempts, 1);
    assert!(images.state().data.is_empty());
    assert!(images.take_warnings().is_empty());
}

#[test]
fn poisoned_state_keeps_whole_entries_and_their_fingerprint_seed() {
    let resources = RenderResources::new();
    let images = resources.svg_images(None);
    let pictures = populate(&images, 32);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = images.state.lock().unwrap();
            panic!("exercise complete-entry poison recovery");
        }))
        .is_err()
    );
    let (bytes, expected) = pictures.last().unwrap();
    assert!(Arc::ptr_eq(
        &images.data("image/png", bytes).unwrap(),
        expected
    ));
    assert!(images.take_warnings().is_empty());
}

#[test]
fn small_cache_tail_hits_keep_linear_comparisons_without_query_hashing() {
    for count in [2, 4, 8, 16] {
        let resources = RenderResources::new();
        let images = resources.svg_images(None);
        let pictures = populate(&images, count);
        let (bytes, expected) = pictures.last().unwrap();
        let (image, work) = measured(|| images.data("image/png", bytes).unwrap());
        assert!(Arc::ptr_eq(&image, expected));
        assert_eq!(work.hashes, 0, "count={count}, work={work:?}");
        assert_eq!(work.comparisons, count);
        assert_eq!(work.candidate_bytes, count * bytes.len());
    }
}
