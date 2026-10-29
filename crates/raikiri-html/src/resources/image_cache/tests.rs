use super::*;

fn source() -> Arc<SvgDocument> {
    Arc::new(SvgDocument::parse(
        b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><rect width='1' height='1'/></svg>",
    ).expect("SVG")).into_source_only()
}

fn cache(max: u64, process: u64) -> (DecodedImageCache, Arc<ProcessImageBudget>) {
    let budget = Arc::new(ProcessImageBudget::new(process, 4));
    let mut cache = DecodedImageCache::new(Arc::clone(&budget));
    cache.max_bytes = max;
    (cache, budget)
}

#[test]
fn rejected_new_svg_raster_does_not_publish_a_source_or_reservation() {
    let url = Url::parse("https://account.test/new.svg").expect("URL");
    let source = source();
    let (mut control, _) = cache(u64::MAX, u64::MAX);
    control
        .insert_source(
            url.clone(),
            CachedBackgroundSource::Svg(Arc::clone(&source)),
        )
        .unwrap();
    let source_cost = control.decoded_bytes;
    drop(control);
    for process_denial in [false, true] {
        let (mut cache, budget) = if process_denial {
            cache(u64::MAX, source_cost)
        } else {
            cache(source_cost, u64::MAX)
        };
        let image = Arc::new(DecodedImage::new(1, 1, vec![0; 4]));
        assert!(
            cache
                .insert_raster(
                    url.clone(),
                    CachedBackgroundSource::Svg(Arc::clone(&source)),
                    ImageRasterSize {
                        width: 1.0,
                        height: 1.0
                    },
                    image,
                )
                .is_err()
        );
        assert!(
            cache.entries.is_empty(),
            "source-only success must not leak through raster denial"
        );
        assert_eq!(cache.decoded_bytes, 0);
        assert_eq!(budget.retained(), 0);
        assert!(cache.scenes.is_empty());
        assert!(cache.raster_only.is_empty());
    }
}

#[test]
fn rejected_source_replacement_preserves_scene_and_learned_state() {
    let url = Url::parse("https://account.test/old.svg").expect("URL");
    let old = source();
    let (mut cache, budget) = cache(u64::MAX, u64::MAX);
    cache
        .insert_source(url.clone(), CachedBackgroundSource::Svg(Arc::clone(&old)))
        .unwrap();
    let size = ImageRasterSize {
        width: 1.0,
        height: 1.0,
    };
    let scene = Arc::new(VectorImage::new((1.0, 1.0)));
    cache.insert_scene(url.clone(), size, Arc::clone(&scene));
    cache.mark_raster_only(url.clone());
    let before_decoded = cache.decoded_bytes;
    let before_scene = cache.scene_bytes;
    let before_process = budget.retained();
    cache.max_bytes = 0;
    assert!(
        cache
            .insert_source(url.clone(), CachedBackgroundSource::Svg(source()))
            .is_err()
    );
    let CachedBackgroundSource::Svg(retained) = &cache.entries[&url].source else {
        panic!("SVG")
    };
    assert!(Arc::ptr_eq(retained, &old));
    assert!(Arc::ptr_eq(&cache.scene(&url, size).unwrap(), &scene));
    assert!(cache.is_raster_only(&url));
    assert_eq!(cache.decoded_bytes, before_decoded);
    assert_eq!(cache.scene_bytes, before_scene);
    assert_eq!(budget.retained(), before_process);
}

// This oracle deliberately scans entries without consulting the delta planner.
fn assert_accounting(cache: &DecodedImageCache) {
    let mut identities = HashMap::new();
    let mut total = 0;
    for (url, entry) in &cache.entries {
        total += DecodedImageCache::raster_bytes(entry);
        if let CachedBackgroundSource::Svg(document) = &entry.source {
            total += (std::mem::size_of::<CachedBackgroundImage>()
                + std::mem::size_of::<Url>()
                + 2 * std::mem::size_of::<usize>()
                + url.as_str().len()) as u64;
            identities.entry(Arc::as_ptr(document)).or_insert_with(|| {
                document
                    .source_only_retained_bytes()
                    .expect("compact source")
            });
        }
    }
    total += identities.values().sum::<u64>() + 512 * identities.len() as u64;
    assert_eq!(cache.decoded_bytes, total);
    assert_eq!(cache.source_ledger.len(), identities.len());
    assert_eq!(
        cache.source_ledger.accounted_bytes(),
        512 * identities.len() as u64
    );
}

#[test]
fn source_identity_transitions_match_independent_scan_and_release_all_bytes() {
    let (mut cache, budget) = cache(u64::MAX, u64::MAX);
    let urls: Vec<_> = (0..12)
        .map(|i| Url::parse(&format!("https://account.test/{i}.svg")).unwrap())
        .collect();
    let shared = source();
    let raster = Arc::new(DecodedImage::new(1, 1, vec![0; 4]));
    for url in &urls {
        cache
            .insert_source(
                url.clone(),
                CachedBackgroundSource::Svg(Arc::clone(&shared)),
            )
            .unwrap();
        assert_accounting(&cache);
        assert_eq!(cache.decoded_bytes, budget.retained());
    }
    for url in &urls {
        // Same identity must leave its reference count unchanged.
        cache
            .insert_source(
                url.clone(),
                CachedBackgroundSource::Svg(Arc::clone(&shared)),
            )
            .unwrap();
        assert_accounting(&cache);
    }
    for (index, url) in urls.iter().enumerate() {
        // Equal source bytes in a separate Arc are separately owned.
        if index % 2 == 0 {
            cache
                .insert_source(url.clone(), CachedBackgroundSource::Svg(source()))
                .unwrap();
        } else {
            cache
                .insert_source(
                    url.clone(),
                    CachedBackgroundSource::Raster(Arc::clone(&raster)),
                )
                .unwrap();
        }
        assert_accounting(&cache);
    }
    for url in &urls {
        cache
            .insert_source(
                url.clone(),
                CachedBackgroundSource::Raster(Arc::clone(&raster)),
            )
            .unwrap();
        assert_accounting(&cache);
    }
    assert_eq!(cache.source_ledger.len(), 0);
    assert_eq!(budget.retained(), urls.len() as u64 * 4);
    drop(cache);
    assert_eq!(budget.retained(), 0);
}

#[test]
fn raster_eviction_and_source_replacement_keep_ledger_and_budget_consistent() {
    let (mut cache, budget) = cache(u64::MAX, u64::MAX);
    let url = Url::parse("https://account.test/sizes.svg").unwrap();
    let source = source();
    for width in 1..=8 {
        cache
            .insert_raster(
                url.clone(),
                CachedBackgroundSource::Svg(Arc::clone(&source)),
                ImageRasterSize {
                    width: width as f32,
                    height: 1.0,
                },
                Arc::new(DecodedImage::new(width, 1, vec![0; width as usize * 4])),
            )
            .unwrap();
        assert_accounting(&cache);
        assert_eq!(cache.decoded_bytes, budget.retained());
    }
    assert_eq!(cache.entries[&url].rasters.len(), MAX_RASTER_SIZES);
    cache.max_bytes = cache.decoded_bytes;
    // A large candidate cannot fit even after evicting all older candidates.
    let before = cache.decoded_bytes;
    let before_sizes: Vec<_> = cache.entries[&url]
        .rasters
        .iter()
        .map(|(s, _)| s.width)
        .collect();
    assert!(
        cache
            .insert_raster(
                url.clone(),
                CachedBackgroundSource::Svg(Arc::clone(&source)),
                ImageRasterSize {
                    width: 1000.0,
                    height: 1.0
                },
                Arc::new(DecodedImage::new(1000, 1, vec![0; 4000]))
            )
            .is_err()
    );
    assert_eq!(cache.decoded_bytes, before);
    assert_eq!(budget.retained(), before);
    assert_eq!(
        cache.entries[&url]
            .rasters
            .iter()
            .map(|(s, _)| s.width)
            .collect::<Vec<_>>(),
        before_sizes
    );
    assert_accounting(&cache);
    cache
        .insert_source(
            url,
            CachedBackgroundSource::Raster(Arc::new(DecodedImage::new(1, 1, vec![0; 4]))),
        )
        .unwrap();
    assert_accounting(&cache);
    assert_eq!(budget.retained(), 4);
}

#[test]
fn process_overflow_is_rejected_even_with_unlimited_maximum() {
    let budget = ProcessImageBudget::new(u64::MAX, 1);
    assert!(budget.try_reserve(u64::MAX));
    assert!(!budget.try_reserve(1));
    assert_eq!(budget.retained(), u64::MAX);
    budget.release(u64::MAX);
    assert_eq!(budget.retained(), 0);
}

#[test]
fn failed_source_admission_does_not_change_shared_identity_ownership() {
    let document = source();
    let (mut control, _) = cache(u64::MAX, u64::MAX);
    let url = Url::parse("https://account.test/one.svg").unwrap();
    control
        .insert_source(
            url.clone(),
            CachedBackgroundSource::Svg(Arc::clone(&document)),
        )
        .unwrap();
    let cost = control.decoded_bytes;
    for process_denial in [false, true] {
        let (mut cache, budget) = if process_denial {
            cache(u64::MAX, cost)
        } else {
            cache(cost, u64::MAX)
        };
        cache
            .insert_source(
                url.clone(),
                CachedBackgroundSource::Svg(Arc::clone(&document)),
            )
            .unwrap();
        for index in 0..20 {
            assert!(
                cache
                    .insert_source(
                        Url::parse(&format!("https://account.test/denied{index}.svg")).unwrap(),
                        CachedBackgroundSource::Svg(Arc::clone(&document))
                    )
                    .is_err()
            );
            assert_accounting(&cache);
            assert_eq!(budget.retained(), cost);
        }
        cache
            .insert_source(
                url.clone(),
                CachedBackgroundSource::Raster(Arc::new(DecodedImage::new(1, 1, vec![0; 4]))),
            )
            .unwrap();
        assert_accounting(&cache);
        assert_eq!(budget.retained(), 4);
    }
}

#[test]
fn replacing_shared_sources_with_an_existing_identity_merges_ownership() {
    let (mut cache, budget) = cache(u64::MAX, u64::MAX);
    let a = source();
    let b = source();
    let urls: Vec<_> = (0..3)
        .map(|i| Url::parse(&format!("https://account.test/merge{i}.svg")).unwrap())
        .collect();
    for (index, url) in urls.iter().enumerate() {
        cache
            .insert_source(
                url.clone(),
                CachedBackgroundSource::Svg(Arc::clone(if index < 2 { &a } else { &b })),
            )
            .unwrap();
        assert_accounting(&cache);
    }
    for url in &urls[..2] {
        cache
            .insert_source(url.clone(), CachedBackgroundSource::Svg(Arc::clone(&b)))
            .unwrap();
        assert_accounting(&cache);
        assert_eq!(budget.retained(), cache.decoded_bytes);
    }
    assert_eq!(cache.source_ledger.len(), 1);
    let mut other = DecodedImageCache::new(Arc::clone(&budget));
    other
        .insert_source(urls[0].clone(), CachedBackgroundSource::Svg(Arc::clone(&b)))
        .unwrap();
    assert_accounting(&other);
    assert_eq!(budget.retained(), cache.decoded_bytes + other.decoded_bytes);
    drop(cache);
    assert_eq!(budget.retained(), other.decoded_bytes);
    drop(other);
    assert_eq!(budget.retained(), 0);
}
