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
    let raster = InlineSvgRaster {
        width: 1,
        height: 1,
        rgba: Arc::new(vec![0; 4]),
    };
    document.cache_inline_svg_raster(svg, (1, 2), "style".into(), raster.clone());
    let got = document
        .cached_inline_svg_raster(svg, (1, 2), "style")
        .unwrap();
    assert!(Arc::ptr_eq(&raster.rgba, &got.rgba));
    assert!(
        document
            .cached_inline_svg_raster(svg, (1, 3), "style")
            .is_none()
    );
    assert!(
        document
            .cached_inline_svg_raster(svg, (1, 2), "STYLE")
            .is_none()
    );
    let huge = InlineSvgRaster {
        width: 1,
        height: 1,
        rgba: Arc::new(vec![0; MAX_BYTES + 1]),
    };
    document.cache_inline_svg_raster(svg, (1, 2), "huge".into(), huge);
    assert!(
        document
            .cached_inline_svg_raster(svg, (1, 2), "huge")
            .is_none()
    );
    for at in 0..MAX_ENTRIES * 2 {
        document.cache_inline_svg_raster(svg, (1, 2), at.to_string(), raster.clone());
    }
    let cache = document.inline_svg_cache.0.lock().unwrap();
    assert!(cache.rasters.len() <= MAX_ENTRIES);
    assert!(cache.bytes <= MAX_BYTES);
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
    document.cache_inline_svg_raster(svg, (1, 2), "same".into(), raster.clone());
    let bytes = document.inline_svg_cache.0.lock().unwrap().bytes;
    document.cache_inline_svg_raster(svg, (1, 2), "same".into(), raster);
    assert_eq!(document.inline_svg_cache.0.lock().unwrap().bytes, bytes);
    let held = document
        .cached_inline_svg_raster(svg, (1, 2), "same")
        .unwrap();
    let large = InlineSvgRaster {
        width: 1,
        height: 1,
        rgba: Arc::new(vec![0; MAX_BYTES / 2]),
    };
    for at in 0..3 {
        document.cache_inline_svg_raster(svg, (1, 2), at.to_string(), large.clone());
    }
    assert!(
        document
            .cached_inline_svg_raster(svg, (1, 2), "same")
            .is_none()
    );
    assert!(Arc::ptr_eq(&held.rgba, &pixels));
    assert!(document.inline_svg_cache.0.lock().unwrap().bytes <= MAX_BYTES);
    drop(document);
    assert_eq!(Arc::strong_count(&pixels), 2);
    drop(held);
    assert_eq!(Arc::strong_count(&pixels), 1);
}
