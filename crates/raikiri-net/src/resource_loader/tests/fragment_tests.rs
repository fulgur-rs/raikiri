use super::*;

fn raster_loader(bytes: u64, entries: usize) -> ResourceLoader<RecordingProvider> {
    ResourceLoader::with_limits(
        RecordingProvider {
            fetch_count: AtomicUsize::new(0),
            bytes: TINY_PNG,
        },
        bytes,
        entries,
    )
}

#[test]
fn raster_fragment_aliases_fetch_and_build_one_source_without_an_alias_index() {
    let base: Url = "https://images.test/a.png".parse().unwrap();
    let resolver = raster_loader(MAX_CACHE_BYTES, 1);
    let builds = AtomicUsize::new(0);
    let mut sources = Vec::new();
    for index in 0..256 {
        let url: Url = format!("{base}#{index}").parse().unwrap();
        let source = resolver
            .load_source_with(&url, |bytes, data, svg, available| {
                builds.fetch_add(1, Ordering::Relaxed);
                resolver.build_source(bytes, data, svg, available)
            })
            .unwrap();
        assert!(Arc::ptr_eq(&source, &resolver.cached_source(&url).unwrap()));
        sources.push(source);
    }
    assert!(sources.iter().all(|s| Arc::ptr_eq(s, &sources[0])));
    assert!(Arc::ptr_eq(
        &sources[0],
        &resolver.load_source(&base).unwrap()
    ));
    assert_eq!(resolver.network.fetch_count.load(Ordering::Relaxed), 1);
    assert_eq!(builds.load(Ordering::Relaxed), 1);
    assert_eq!(
        resolver.budget.lock().unwrap().used,
        base.as_str().len() as u64 + 8
    );
    let state = resolver.state.lock().unwrap();
    assert_eq!(state.cache.len(), 1);
    assert!(state.loading.is_empty());
}

#[test]
fn long_fragment_reservation_is_refunded_after_raster_publication() {
    let base: Url = "https://images.test/a.png".parse().unwrap();
    let url: Url = format!("{base}#{}", "a".repeat(4096)).parse().unwrap();
    let resolver = raster_loader(url.as_str().len() as u64 + 8, 1);
    assert!(resolver.load_source(&url).is_ok());
    assert_eq!(
        resolver.budget.lock().unwrap().used,
        base.as_str().len() as u64 + 8
    );
    assert!(resolver.cached_source(&base).is_some());
}

#[test]
fn structured_fragments_queries_and_other_schemes_keep_distinct_sources() {
    for (a, b) in [
        (
            "https://images.test/a.png#xywh=0,0,1,1",
            "https://images.test/a.png#xywh=1,0,1,1",
        ),
        (
            "https://images.test/a.png#t=1",
            "https://images.test/a.png#t=2",
        ),
        (
            "https://images.test/a.png#unknown=value",
            "https://images.test/a.png#unknown=other",
        ),
        (
            "https://images.test/a.png#xywh%3D1",
            "https://images.test/a.png#xywh%3D2",
        ),
        (
            "https://images.test/a.png?q=1#one",
            "https://images.test/a.png?q=2#two",
        ),
        ("file:///a.png#one", "file:///a.png#two"),
        ("custom:picture#one", "custom:picture#two"),
    ] {
        let resolver = raster_loader(MAX_CACHE_BYTES, 256);
        let a: Url = a.parse().unwrap();
        let b: Url = b.parse().unwrap();
        let first = resolver.load_source(&a).unwrap();
        let second = resolver.load_source(&b).unwrap();
        assert!(!Arc::ptr_eq(&first, &second), "{a} and {b}");
        assert!(Arc::ptr_eq(&first, &resolver.load_source(&a).unwrap()));
        assert_eq!(resolver.network.fetch_count.load(Ordering::Relaxed), 2);
    }
}

struct ResponseProvider {
    bytes: &'static [u8],
    mime: &'static str,
    redirect: Option<Url>,
    calls: AtomicUsize,
    requests: Mutex<Vec<Url>>,
}
impl NetworkProvider for ResponseProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.requests.lock().unwrap().push(request.url.clone());
        if request.url.path().starts_with("/redirect")
            && let Some(location) = &self.redirect
        {
            return Ok(FetchOutcome::Redirect {
                location: location.clone(),
                status: 302,
            });
        }
        Ok(FetchOutcome::Body(
            FetchedResource::new(self.bytes.to_vec().into(), request.url)
                .content_type(Some(self.mime.into())),
        ))
    }
}
fn responding(
    bytes: &'static [u8],
    mime: &'static str,
    redirect: Option<Url>,
) -> ResourceLoader<ResponseProvider> {
    ResourceLoader::new(ResponseProvider {
        bytes,
        mime,
        redirect,
        calls: AtomicUsize::new(0),
        requests: Mutex::new(Vec::new()),
    })
}

#[test]
fn svg_mime_and_svg_path_keep_full_display_identities_and_key_charges() {
    for path in ["a.png", "a.svg"] {
        let resolver = responding(TWO_COLOR_SVG, "image/svg+xml", None);
        let base = format!("https://images.test/{path}");
        let urls: Vec<Url> = ["", "#", "#first", "#second", "#svgView(viewBox(0,0,1,1))"]
            .into_iter()
            .map(|f| format!("{base}{f}").parse().unwrap())
            .collect();
        let sources: Vec<_> = urls
            .iter()
            .map(|u| resolver.load_source(u).unwrap())
            .collect();
        for (i, url) in urls.iter().enumerate() {
            assert!(Arc::ptr_eq(
                &sources[i],
                &resolver.load_source(url).unwrap()
            ));
            for other in &sources[..i] {
                assert!(!Arc::ptr_eq(&sources[i], other));
            }
        }
        assert_eq!(resolver.network.calls.load(Ordering::Relaxed), urls.len());
        assert_eq!(
            resolver.budget.lock().unwrap().used,
            urls.iter()
                .map(|u| u.as_str().len() as u64 + TWO_COLOR_SVG.len() as u64)
                .sum::<u64>()
        );
        assert_eq!(*resolver.network.requests.lock().unwrap(), urls);
    }
}

#[test]
fn redirect_targets_only_veto_sharing_and_never_merge_distinct_entry_urls() {
    for (fragment, shared) in [
        ("", true),
        ("#plain", true),
        ("#xywh=0,0,1,1", false),
        ("#t=1", false),
        ("#xywh%3D1", false),
    ] {
        let target: Url = format!("https://cdn.test/final.png{fragment}")
            .parse()
            .unwrap();
        let resolver = responding(TINY_PNG, "image/png", Some(target));
        let a: Url = "https://images.test/redirect-a#one".parse().unwrap();
        let b: Url = "https://images.test/redirect-a#two".parse().unwrap();
        let c: Url = "https://images.test/redirect-b#one".parse().unwrap();
        let first = resolver.load_source(&a).unwrap();
        let second = resolver.load_source(&b).unwrap();
        assert_eq!(Arc::ptr_eq(&first, &second), shared);
        let third = resolver.load_source(&c).unwrap();
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(
            resolver.network.calls.load(Ordering::Relaxed),
            if shared { 4 } else { 6 }
        );
        assert_eq!(resolver.network.requests.lock().unwrap()[0], a);
    }
}

#[test]
fn failed_fragment_loads_release_full_reservations_before_retry() {
    let base: Url = "https://images.test/a.png".parse().unwrap();
    let a: Url = format!("{base}#{}", "a".repeat(1024)).parse().unwrap();
    let b: Url = format!("{base}#b").parse().unwrap();
    for panic in [false, true] {
        let resolver = raster_loader(MAX_CACHE_BYTES, 1);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            resolver.load_source_with(&a, |_, _, _, _| {
                if panic {
                    panic!("injected fragment builder panic");
                }
                Err(ResolverError::Decode("injected fragment failure".into()))
            })
        }));
        assert!(if panic {
            outcome.is_err()
        } else {
            outcome.unwrap().is_err()
        });
        assert_eq!(resolver.budget.lock().unwrap().used, 0);
        assert!(resolver.state.lock().unwrap().loading.is_empty());
        assert!(resolver.load_source(&b).is_ok());
        assert_eq!(
            resolver.budget.lock().unwrap().used,
            base.as_str().len() as u64 + 8
        );
    }
    let resolver = raster_loader(a.as_str().len() as u64 + 7, 1);
    assert!(resolver.load_source(&a).is_err());
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
    assert!(resolver.state.lock().unwrap().loading.is_empty());
    assert!(resolver.load_source(&b).is_ok());
}

struct FormatByFragment;
impl NetworkProvider for FormatByFragment {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        let (bytes, mime) = if request.url.fragment() == Some("png") {
            (TINY_PNG, "image/png")
        } else {
            (TWO_COLOR_SVG, "image/svg+xml")
        };
        Ok(FetchOutcome::Body(
            FetchedResource::new(bytes.to_vec().into(), request.url)
                .content_type(Some(mime.into())),
        ))
    }
}

#[test]
fn exact_svg_and_canonical_raster_keys_have_separate_namespaces() {
    let resolver = ResourceLoader::new(FormatByFragment);
    let base: Url = "https://images.test/a.png".parse().unwrap();
    let alias: Url = "https://images.test/a.png#png".parse().unwrap();
    let svg = resolver.load_source(&base).unwrap();
    let raster = resolver.load_source(&alias).unwrap();
    assert!(matches!(&svg.kind, ImageSourceKind::Svg(_)));
    assert!(matches!(&raster.kind, ImageSourceKind::Raster(_)));
    assert!(Arc::ptr_eq(&svg, &resolver.cached_source(&base).unwrap()));
    assert!(Arc::ptr_eq(
        &raster,
        &resolver.cached_source(&alias).unwrap()
    ));
    assert_eq!(resolver.state.lock().unwrap().cache.len(), 2);
    assert_eq!(
        resolver.budget.lock().unwrap().used,
        base.as_str().len() as u64 * 2 + TWO_COLOR_SVG.len() as u64 + 8
    );
}
