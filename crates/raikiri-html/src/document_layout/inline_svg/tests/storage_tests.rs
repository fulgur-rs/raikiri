use super::*;

#[test]
fn equal_prepared_xml_is_one_allocation_across_roots() {
    let cache = InlineSvgCache::new(Arc::default());
    let source = document(RECT);
    let first_key = (0, 40f32.to_bits(), 20f32.to_bits());
    cache.insert(
        first_key,
        PreparedSvg::new(source.clone(), None, SvgRendering::Vector),
    );
    let first = cache.get(&first_key).unwrap();
    for root in 1..32 {
        let key = (root, 40f32.to_bits(), 20f32.to_bits());
        cache.insert(
            key,
            PreparedSvg::new(source.clone(), None, SvgRendering::Vector),
        );
        let prepared = cache.get(&key).unwrap();
        assert!(Arc::ptr_eq(&first.source, &prepared.source));
    }
}

#[test]
fn prepared_root_metadata_has_a_fixed_entry_bound() {
    let cache = InlineSvgCache::new(Arc::default());
    for root in 0..300 {
        cache.insert(
            (root, 1f32.to_bits(), 1f32.to_bits()),
            PreparedSvg::new("<svg/>".into(), None, SvgRendering::Vector),
        );
    }
    assert!(cache.len() <= 256);
}

use crate::document_layout::inline_svg::storage::{Limits, source_charge};

fn small_cache(roots: usize, pictures: usize, xml_bytes: usize) -> InlineSvgCache {
    let cache = InlineSvgCache::with_limits(
        Arc::default(),
        Limits {
            roots,
            pictures,
            ..Limits::default()
        },
    );
    cache.with_storage(|storage| {
        let metadata = storage.metadata_bytes();
        storage.limits_mut().storage = metadata + xml_bytes;
    });
    cache
}

fn root_key(id: usize) -> SvgKey {
    (id, 1f32.to_bits(), 1f32.to_bits())
}

#[test]
fn source_union_counts_root_and_picture_owners_once_and_releases_last_cache_owner() {
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    let c = "c".repeat(40);
    let charge = source_charge(&a);
    let cache = small_cache(2, 2, 2 * charge);
    let canonical = cache.insert(
        root_key(0),
        PreparedSvg::new(a.clone(), None, SvgRendering::Vector),
    );
    let key_a = PictureKey::new(&canonical, PaintRect::new(0.0, 0.0, 1.0, 1.0));
    let caller = raster_of(4);
    cache.keep(key_a.clone(), &caller);
    assert_eq!(cache.charges().1, charge);
    cache.insert(
        root_key(1),
        PreparedSvg::new(b.clone(), None, SvgRendering::Vector),
    );
    cache.insert(
        root_key(2),
        PreparedSvg::new(b.clone(), None, SvgRendering::Vector),
    );
    assert!(cache.get(&root_key(0)).is_none());
    assert!(cache.picture(&key_a).is_some());
    assert_eq!(cache.charges().1, 2 * charge);
    cache.keep(picture_key(&b, SvgRendering::Vector), &raster_of(7));
    cache.keep(picture_key(&c, SvgRendering::Vector), &raster_of(9));
    assert!(cache.picture(&key_a).is_none());
    assert_eq!(cache.charges().1, 2 * charge);
    assert_eq!(cache.charges().3, 16);
    let ImageContent::Raster(pixels) = caller else {
        panic!("pixels")
    };
    assert_eq!(pixels.rgba.len(), 4);
    assert_eq!(canonical.source.as_ref(), a);
}

#[test]
fn returned_canonical_xml_preserves_candidate_metadata_and_picture_key_dimensions() {
    let source = document(RECT);
    let cache = small_cache(0, 4, source_charge(&source));
    let first = PreparedSvg::new(source.clone(), None, SvgRendering::Vector);
    let key = PictureKey::new(&first, PaintRect::new(0.0, 0.0, 40.0, 20.0));
    cache.keep(key.clone(), &raster_of(4));
    let candidate = cache.insert(
        root_key(1),
        PreparedSvg::new(source.clone(), Some(0.5), SvgRendering::Raster),
    );
    assert!(cache.get(&root_key(1)).is_none());
    assert!(Arc::ptr_eq(&key.source, &candidate.source));
    assert_eq!(candidate.host_opacity, Some(0.5));
    assert_eq!(candidate.rendering, SvgRendering::Raster);
    let other = PictureKey::new(&candidate, PaintRect::new(0.0, 0.0, 40.0, 20.0));
    let wider = PictureKey::new(&candidate, PaintRect::new(0.0, 0.0, 41.0, 20.0));
    cache.keep(other.clone(), &raster_of(5));
    cache.keep(wider.clone(), &raster_of(6));
    assert_eq!(cache.kept_pictures(), 3);
    assert_eq!(cache.charges().1, source_charge(&source));
    assert_eq!(cache.charges().3, 15);
    for (key, bytes) in [(key, 4), (other, 5), (wider, 6)] {
        let ImageContent::Raster(pixels) = cache.picture(&key).unwrap() else {
            panic!("pixels")
        };
        assert_eq!(pixels.rgba.len(), bytes);
    }
}

#[test]
fn root_count_pressure_preserves_the_single_shared_picture() {
    let source = document(RECT);
    let cache = InlineSvgCache::new(Arc::default());
    let canonical = cache.insert(
        root_key(0),
        PreparedSvg::new(source.clone(), None, SvgRendering::Vector),
    );
    let key = PictureKey::new(&canonical, PaintRect::new(0.0, 0.0, 40.0, 20.0));
    let content = picture(&source, |_| {}).unwrap();
    cache.keep(key.clone(), &content);
    for root in 1..MAX_PREPARED_ROOTS + 32 {
        let returned = cache.insert(
            root_key(root),
            PreparedSvg::new(source.clone(), None, SvgRendering::Vector),
        );
        assert!(Arc::ptr_eq(&returned.source, &canonical.source));
    }
    assert_eq!(cache.len(), MAX_PREPARED_ROOTS);
    assert_eq!(cache.kept_pictures(), 1);
    assert_eq!(cache.charges().1, source_charge(&source));
    let (ImageContent::Vector(cached), ImageContent::Vector(caller)) =
        (cache.picture(&key).unwrap(), content)
    else {
        panic!("scene")
    };
    assert!(Arc::ptr_eq(&cached, &caller));
}

#[test]
fn source_hash_collisions_do_not_merge_xml_or_pictures() {
    let cache = InlineSvgCache::new(Arc::default());
    let mut a = PreparedSvg::new("a".into(), None, SvgRendering::Vector);
    let mut b = PreparedSvg::new("b".into(), None, SvgRendering::Vector);
    a.source_hash = 17;
    b.source_hash = 17;
    let a = cache.insert(root_key(0), a);
    let b = cache.insert(root_key(1), b);
    assert!(!Arc::ptr_eq(&a.source, &b.source));
    let key_a = PictureKey::new(&a, PaintRect::new(0.0, 0.0, 1.0, 1.0));
    let key_b = PictureKey::new(&b, PaintRect::new(0.0, 0.0, 1.0, 1.0));
    assert_eq!(key_a.fingerprint, key_b.fingerprint);
    cache.keep(key_a.clone(), &raster_of(4));
    cache.keep(key_b.clone(), &raster_of(8));
    for (key, bytes) in [(key_a, 4), (key_b, 8)] {
        let ImageContent::Raster(pixels) = cache.picture(&key).unwrap() else {
            panic!("pixels")
        };
        assert_eq!(pixels.rgba.len(), bytes);
    }
}

#[test]
fn oversized_source_and_payload_bypass_without_evicting_useful_entries() {
    let source = "a".repeat(40);
    let cache = small_cache(2, 2, source_charge(&source));
    let kept = cache.insert(
        root_key(0),
        PreparedSvg::new(source.clone(), None, SvgRendering::Vector),
    );
    let key = PictureKey::new(&kept, PaintRect::new(0.0, 0.0, 1.0, 1.0));
    cache.keep(key.clone(), &raster_of(4));
    let before = cache.charges();
    let huge = "x".repeat(1024);
    let unkept = cache.insert(
        root_key(1),
        PreparedSvg::new(huge.clone(), None, SvgRendering::Vector),
    );
    assert_eq!(unkept.source.as_ref(), huge);
    assert!(cache.get(&root_key(1)).is_none());
    cache.keep(picture_key(&huge, SvgRendering::Vector), &raster_of(4));
    cache.with_storage(|storage| storage.limits_mut().rasters = 4);
    cache.keep(picture_key("b", SvgRendering::Vector), &raster_of(1));
    cache.with_storage(|storage| storage.limits_mut().scenes = 0);
    cache.keep(
        picture_key("b", SvgRendering::Vector),
        &picture(&document(RECT), |_| {}).unwrap(),
    );
    assert!(cache.get(&root_key(0)).is_some());
    assert!(cache.picture(&key).is_some());
    assert_eq!(cache.charges(), before);
}

#[test]
fn picture_owned_xml_can_bypass_root_admission_then_admit_a_successful_picture() {
    let a = document(RECT);
    let b = a.replace("#c00", "#00c");
    let budget = source_charge(&a).max(source_charge(&b));
    let cache = small_cache(2, 2, budget);
    let a = cache.insert(
        root_key(0),
        PreparedSvg::new(a.clone(), None, SvgRendering::Vector),
    );
    let key_a = PictureKey::new(&a, PaintRect::new(0.0, 0.0, 40.0, 20.0));
    let caller = picture(&a.source, |_| {}).unwrap();
    cache.keep(key_a.clone(), &caller);
    let b = cache.insert(root_key(1), PreparedSvg::new(b, None, SvgRendering::Vector));
    assert!(cache.get(&root_key(0)).is_some());
    assert!(cache.get(&root_key(1)).is_none());
    let key_b = PictureKey::new(&b, PaintRect::new(0.0, 0.0, 40.0, 20.0));
    let built = picture(&b.source, |_| {}).unwrap();
    cache.keep(key_b.clone(), &built);
    assert!(cache.picture(&key_a).is_none());
    assert!(cache.picture(&key_b).is_some());
    assert_eq!(cache.charges().1, source_charge(&b.source));
    let ImageContent::Vector(old) = caller else {
        panic!("scene")
    };
    assert_eq!(old.size, (40.0, 20.0));
    assert!(old.node_count() > 1);
}

#[test]
fn failed_picture_build_after_root_bypass_preserves_successful_pictures() {
    let source = document(RECT);
    let broken = document(&format!(
        "<filter id='f'>{}</filter><rect width='40' height='20' filter='url(#f)'/>",
        "<feOffset dx='1'/>".repeat(40)
    ));
    let cache = small_cache(2, 2, source_charge(&broken).max(source_charge(&source)));
    let prepared = cache.insert(
        root_key(0),
        PreparedSvg::new(source.clone(), None, SvgRendering::Vector),
    );
    let key = PictureKey::new(&prepared, PaintRect::new(0.0, 0.0, 40.0, 20.0));
    cache.keep(key.clone(), &picture(&source, |_| {}).unwrap());
    let failed = cache.insert(
        root_key(1),
        PreparedSvg::new(broken, None, SvgRendering::Vector),
    );
    assert!(cache.get(&root_key(1)).is_none());
    assert!(picture(&failed.source, |request| request.raster_scale = 100.0).is_err());
    assert!(cache.picture(&key).is_some());
    assert_eq!(cache.kept_pictures(), 1);
}

#[test]
fn picture_count_eviction_updates_scene_and_raster_accounting() {
    let cache = small_cache(0, 1, 1024);
    cache.keep(picture_key("a", SvgRendering::Vector), &raster_of(10));
    assert_eq!((cache.charges().2, cache.charges().3), (0, 10));
    let source = document(RECT);
    let scene = picture(&source, |_| {}).unwrap();
    let ImageContent::Vector(vector) = &scene else {
        panic!("scene")
    };
    let bytes = vector.estimated_heap_bytes();
    cache.keep(picture_key(&source, SvgRendering::Vector), &scene);
    assert_eq!((cache.charges().2, cache.charges().3), (bytes, 0));
    cache.keep(picture_key("c", SvgRendering::Vector), &raster_of(7));
    assert_eq!((cache.charges().2, cache.charges().3), (0, 7));
}

#[test]
fn distinct_sources_stay_under_storage_and_entry_limits_across_revisits() {
    let cache = small_cache(8, 8, 4 * source_charge(&"x".repeat(80)));
    for pass in 0..2 {
        for root in 0..64 {
            let source = format!("{root:04}{}", "x".repeat(76));
            let prepared = cache.insert(
                root_key(root),
                PreparedSvg::new(source, None, SvgRendering::Vector),
            );
            let key = PictureKey::new(&prepared, PaintRect::new(0.0, 0.0, 1.0, 1.0));
            cache.keep(key, &raster_of(4));
            let (metadata, xml, _, pixels) = cache.charges();
            assert!(metadata + xml <= cache.with_storage(|storage| storage.limits_mut().storage));
            assert!(cache.len() <= 8 && cache.kept_pictures() <= 8);
            assert!(pixels <= 32);
        }
        assert!(cache.kept_pictures() > 0, "pass {pass}");
    }
}

#[test]
fn root_only_byte_pressure_releases_sources_without_charging_caller_arcs() {
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    let c = "c".repeat(40);
    let cache = small_cache(4, 0, 2 * source_charge(&a));
    let caller = cache.insert(root_key(0), PreparedSvg::new(a, None, SvgRendering::Vector));
    cache.insert(root_key(1), PreparedSvg::new(b, None, SvgRendering::Vector));
    cache.insert(root_key(2), PreparedSvg::new(c, None, SvgRendering::Vector));
    assert!(cache.get(&root_key(0)).is_none());
    assert!(cache.get(&root_key(1)).is_some() && cache.get(&root_key(2)).is_some());
    assert_eq!(cache.charges().1, 2 * source_charge(&caller.source));
    cache.keep(picture_key("unused", SvgRendering::Vector), &raster_of(4));
    assert_eq!(cache.kept_pictures(), 0);
    assert_eq!(cache.charges().3, 0);
    let before_metadata = cache.charges().0;
    let oversized = "x".repeat(1024);
    cache.insert(
        root_key(1),
        PreparedSvg::new(oversized, None, SvgRendering::Vector),
    );
    assert!(cache.get(&root_key(1)).is_none());
    assert!(cache.get(&root_key(2)).is_some());
    assert_eq!(cache.charges().0, before_metadata);
    assert_eq!(cache.charges().1, source_charge(&caller.source));
}

#[test]
fn unused_svg_caches_do_not_allocate_slot_arrays() {
    let cache = InlineSvgCache::new(Arc::default());
    assert!(cache.get(&root_key(0)).is_none());
    assert!(
        cache
            .picture(&picture_key("missing", SvgRendering::Vector))
            .is_none()
    );
    assert_eq!(cache.len(), 0);
    assert_eq!(cache.kept_pictures(), 0);
    assert!(cache.storage.borrow().is_none());
    assert_eq!(
        cache.charges(),
        (std::mem::size_of::<InlineSvgCache>(), 0, 0, 0)
    );
}
