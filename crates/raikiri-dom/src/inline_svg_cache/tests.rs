use super::*;

fn fixture() -> (Document, usize, usize) {
    let mut document = Document::new();
    let svg = document.append_element(Some(0), "svg", None::<&str>);
    document.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    let path = document.append_element(Some(svg), "path", None::<&str>);
    document.set_element_namespace(path, Some("http://www.w3.org/2000/svg".into()));
    document.set_element_attributes(path, vec![("d".into(), "M0 0H1V1Z".into())]);
    (document, svg, path)
}

fn declarations(text: &str) -> Vec<InlineSvgElementDeclarations> {
    vec![InlineSvgElementDeclarations {
        element_index: 1,
        declarations: text.to_owned(),
    }]
}

fn raster_of(byte: u8) -> InlineSvgRaster {
    InlineSvgRaster {
        width: 1,
        height: 1,
        rgba: Arc::new(vec![byte; 4]),
    }
}

#[test]
fn source_memo_is_shared_until_each_kind_of_source_mutation() {
    let (mut document, svg, path) = fixture();
    let mut previous = document.cached_svg_subtree(svg).unwrap().unwrap();
    assert!(Arc::ptr_eq(
        &previous,
        &document.cached_svg_subtree(svg).unwrap().unwrap()
    ));
    type Mutation = Box<dyn Fn(&mut Document)>;
    let mutations: Vec<Mutation> = vec![
        Box::new(move |doc| {
            doc.set_element_attributes(path, vec![("d".into(), "M0 0H2V2Z".into())])
        }),
        Box::new(move |doc| doc.set_element_inline_style(path, Some("fill:red".into()))),
        Box::new(move |doc| {
            doc.set_element_namespace_info(
                path,
                Some("http://www.w3.org/2000/svg".into()),
                Some("s".into()),
            )
        }),
        Box::new(move |doc| {
            doc.set_element_attribute(path, "stroke", "blue").unwrap();
        }),
        Box::new(move |doc| {
            doc.remove_element_attribute(path, "stroke").unwrap();
        }),
        Box::new(move |doc| {
            doc.append_text(svg, "hello");
        }),
    ];
    for mutate in mutations {
        mutate(&mut document);
        let next = document.cached_svg_subtree(svg).unwrap().unwrap();
        assert!(!Arc::ptr_eq(&previous, &next));
        assert_eq!(
            &*next,
            document.serialize_svg_subtree(svg).unwrap().unwrap()
        );
        previous = next;
    }
    assert!(
        document
            .clone()
            .inline_svg_cache
            .0
            .lock()
            .unwrap()
            .sources
            .is_empty()
    );
}

#[test]
fn raster_keys_are_exact_and_oversized_rasters_are_not_kept() {
    let (document, svg, _) = fixture();
    let raster = raster_of(0);
    document.cache_inline_svg_raster(svg, (1, 2), "style".into(), Vec::new(), raster.clone());
    let got = document
        .cached_inline_svg_raster(svg, (1, 2), "style", |found| found.is_empty())
        .unwrap();
    assert!(Arc::ptr_eq(&raster.rgba, &got.rgba));
    assert!(
        document
            .cached_inline_svg_raster(svg, (1, 3), "style", |_| true)
            .is_none()
    );
    assert!(
        document
            .cached_inline_svg_raster(svg, (1, 2), "STYLE", |_| true)
            .is_none()
    );
    assert!(
        document
            .cached_inline_svg_raster(svg + 1, (1, 2), "style", |_| true)
            .is_none()
    );
    let huge = InlineSvgRaster {
        width: 1,
        height: 1,
        rgba: Arc::new(vec![0; MAX_BYTES + 1]),
    };
    document.cache_inline_svg_raster(svg, (1, 2), "huge".into(), Vec::new(), huge);
    assert!(
        document
            .cached_inline_svg_raster(svg, (1, 2), "huge", |_| true)
            .is_none()
    );
    for at in 0..MAX_ENTRIES * 2 {
        document.cache_inline_svg_raster(svg, (1, 2), at.to_string(), Vec::new(), raster.clone());
    }
    let cache = document.inline_svg_cache.0.lock().unwrap();
    assert!(cache.rasters.len() <= MAX_ENTRIES);
    assert!(cache.bytes <= MAX_BYTES);
}

#[test]
fn the_declarations_of_the_subtree_are_part_of_the_raster_key() {
    let (document, svg, _) = fixture();
    let (red, blue) = (raster_of(1), raster_of(2));
    document.cache_inline_svg_raster(
        svg,
        (1, 2),
        "s".into(),
        declarations("fill:red"),
        red.clone(),
    );
    document.cache_inline_svg_raster(
        svg,
        (1, 2),
        "s".into(),
        declarations("fill:blue"),
        blue.clone(),
    );
    let asking = |text: &'static str| {
        document.cached_inline_svg_raster(svg, (1, 2), "s", |found| found == declarations(text))
    };
    assert!(Arc::ptr_eq(&asking("fill:red").unwrap().rgba, &red.rgba));
    assert!(Arc::ptr_eq(&asking("fill:blue").unwrap().rgba, &blue.rgba));
    assert!(asking("fill:green").is_none());
    assert_eq!(document.inline_svg_cache.0.lock().unwrap().rasters.len(), 2);
    // The same key replaces its raster instead of adding another entry.
    let newer = raster_of(3);
    document.cache_inline_svg_raster(
        svg,
        (1, 2),
        "s".into(),
        declarations("fill:red"),
        newer.clone(),
    );
    assert_eq!(document.inline_svg_cache.0.lock().unwrap().rasters.len(), 2);
    assert!(Arc::ptr_eq(&asking("fill:red").unwrap().rgba, &newer.rgba));
}

#[test]
fn candidates_are_asked_most_recently_used_first_and_a_hit_becomes_the_most_recent() {
    let (document, svg, _) = fixture();
    for (at, text) in ["a", "b", "c"].into_iter().enumerate() {
        document.cache_inline_svg_raster(
            svg,
            (1, 2),
            "s".into(),
            declarations(text),
            raster_of(at as u8),
        );
    }
    let mut asked = Vec::new();
    assert!(
        document
            .cached_inline_svg_raster(svg, (1, 2), "s", |found| {
                asked.push(found[0].declarations.clone());
                found[0].declarations == "a"
            })
            .is_some()
    );
    assert_eq!(asked, ["c", "b", "a"]);
    // "a" was the oldest and is now the newest.
    asked.clear();
    document.cached_inline_svg_raster(svg, (1, 2), "s", |found| {
        asked.push(found[0].declarations.clone());
        false
    });
    assert_eq!(asked, ["a", "c", "b"]);
}

#[test]
fn a_hit_on_an_entry_evicted_while_it_was_compared_still_returns_its_pixels() {
    let (document, svg, _) = fixture();
    let pixels = raster_of(7);
    document.cache_inline_svg_raster(svg, (1, 2), "s".into(), declarations("x"), pixels.clone());
    let got = document
        .cached_inline_svg_raster(svg, (1, 2), "s", |_| {
            // Another paint fills the memo while this comparison runs.
            document.inline_svg_cache.0.lock().unwrap().rasters.clear();
            true
        })
        .unwrap();
    assert!(Arc::ptr_eq(&got.rgba, &pixels.rgba));
    assert!(
        document
            .inline_svg_cache
            .0
            .lock()
            .unwrap()
            .rasters
            .is_empty()
    );
}

#[test]
fn the_declarations_of_an_entry_are_charged_to_the_memo() {
    let (document, svg, _) = fixture();
    document.cache_inline_svg_raster(svg, (1, 2), "s".into(), Vec::new(), raster_of(0));
    let plain = document.inline_svg_cache.0.lock().unwrap().bytes;
    let text = "x".repeat(10_000);
    document.cache_inline_svg_raster(svg, (1, 2), "t".into(), declarations(&text), raster_of(0));
    let charged = document.inline_svg_cache.0.lock().unwrap().bytes - plain;
    assert!(charged >= 10_000, "{charged}");
}

#[test]
fn source_entry_eviction_missing_nodes_and_serialization_errors_are_safe() {
    let (mut document, svg, path) = fixture();
    assert!(document.cached_svg_subtree(usize::MAX).unwrap().is_none());
    for _ in 0..MAX_ENTRIES + 1 {
        let next = document.append_element(Some(0), "svg", None::<&str>);
        document.set_element_namespace(next, Some("http://www.w3.org/2000/svg".into()));
    }
    let roots: Vec<_> = (0..document.node_count())
        .filter(|&id| document.get_node(id).unwrap().tag_name() == Some("svg"))
        .collect();
    for root in &roots {
        document.cached_svg_subtree(*root).unwrap().unwrap();
    }
    {
        let cache = document.inline_svg_cache.0.lock().unwrap();
        assert_eq!(cache.sources.len(), MAX_ENTRIES);
        assert!(cache.sources.iter().all(|entry| entry.node != svg));
    }
    document.set_element_namespace_info(
        path,
        Some("http://www.w3.org/2000/svg".into()),
        Some("bad prefix".into()),
    );
    assert!(document.cached_svg_subtree(svg).is_err());
    assert!(document.cached_svg_subtree(svg).is_err());
    assert!(
        document
            .inline_svg_cache
            .0
            .lock()
            .unwrap()
            .sources
            .is_empty()
    );
}

#[test]
fn replacing_rasters_and_byte_pressure_release_only_cache_ownership() {
    let (document, svg, _) = fixture();
    let pixels = Arc::new(vec![0; 4]);
    let raster = InlineSvgRaster {
        width: 1,
        height: 1,
        rgba: Arc::clone(&pixels),
    };
    document.cache_inline_svg_raster(svg, (1, 2), "same".into(), Vec::new(), raster.clone());
    let bytes = document.inline_svg_cache.0.lock().unwrap().bytes;
    document.cache_inline_svg_raster(svg, (1, 2), "same".into(), Vec::new(), raster);
    assert_eq!(document.inline_svg_cache.0.lock().unwrap().bytes, bytes);
    let held = document
        .cached_inline_svg_raster(svg, (1, 2), "same", |_| true)
        .unwrap();
    let large = InlineSvgRaster {
        width: 1,
        height: 1,
        rgba: Arc::new(vec![0; MAX_BYTES / 2]),
    };
    for at in 0..3 {
        document.cache_inline_svg_raster(svg, (1, 2), at.to_string(), Vec::new(), large.clone());
    }
    assert!(
        document
            .cached_inline_svg_raster(svg, (1, 2), "same", |_| true)
            .is_none()
    );
    assert!(Arc::ptr_eq(&held.rgba, &pixels));
    assert!(document.inline_svg_cache.0.lock().unwrap().bytes <= MAX_BYTES);
    drop(document);
    assert_eq!(Arc::strong_count(&pixels), 2);
    drop(held);
    assert_eq!(Arc::strong_count(&pixels), 1);
}
