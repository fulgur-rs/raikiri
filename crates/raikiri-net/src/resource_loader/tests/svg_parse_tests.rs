use super::*;
use std::cell::Cell;

thread_local! {
    static FULL_PARSES: Cell<usize> = const { Cell::new(0) };
}

pub(in crate::resource_loader) fn record_parse() {
    FULL_PARSES.with(|count| count.set(count.get() + 1));
}

fn svg_url() -> Url {
    svg_url_for(TWO_COLOR_SVG)
}

fn svg_url_for(bytes: &[u8]) -> Url {
    let encoded = bytes
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    Url::parse(&format!("data:image/svg+xml,{encoded}")).unwrap()
}

#[test]
fn four_and_five_size_cycles_reuse_validated_svg_source() {
    for raster in [false, true] {
        for sizes in [4, 5] {
            FULL_PARSES.with(|count| count.set(0));
            let resolver = ResourceLoader::new(RejectingProvider);
            let source = resolver.load_source(&svg_url()).unwrap();
            for _ in 0..3 {
                for index in 0..sizes {
                    let size = box_of(4.0 + index as f32);
                    if raster {
                        assert_eq!(
                            source.rasterize(size, None).unwrap().width,
                            size.width as u32
                        );
                    } else {
                        assert_eq!(source.scene(size).unwrap().size, (size.width, size.height));
                    }
                }
            }
            assert_eq!(
                FULL_PARSES.with(Cell::get),
                1,
                "sizes={sizes}, raster={raster}"
            );
        }
    }
}

#[test]
fn svg_source_admission_charges_the_compact_document_allocation() {
    let url = svg_url();
    let exact = url.as_str().len() as u64 + svg_retained_bytes(TWO_COLOR_SVG);
    let resolver = ResourceLoader::with_limits(RejectingProvider, exact, MAX_CACHE_ENTRIES);
    resolver.load_source(&url).unwrap();
    assert_eq!(resolver.budget.lock().unwrap().used, exact);
    let resolver = ResourceLoader::with_limits(RejectingProvider, exact - 1, MAX_CACHE_ENTRIES);
    assert!(resolver.load_source(&url).is_err());
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
    let state = resolver.state.lock().unwrap();
    assert!(state.cache.is_empty());
    assert!(state.loading.is_empty());
}

#[test]
fn svg_source_scene_and_pixels_share_the_compact_source_budget() {
    let url = svg_url();
    let resolver = ResourceLoader::new(RejectingProvider);
    let source = resolver.load_source(&url).unwrap();
    let ImageSourceKind::Svg(document) = &source.kind else {
        panic!("SVG source");
    };
    let retained = document.source_only_retained_bytes().unwrap();
    let initial = url.as_str().len() as u64 + retained;
    assert_eq!(source.retained_byte_len(), Some(retained));
    assert_eq!(source.decoded_byte_len(), None);
    let scene = source.scene(box_of(4.0)).unwrap();
    let raster = source.rasterize(box_of(4.0), None).unwrap();
    assert_eq!(
        resolver.budget.lock().unwrap().used,
        initial + scene_bytes(&scene) + raster.rgba.len() as u64
    );
}

#[test]
fn unaccountable_svg_tree_is_rejected_and_releases_admission() {
    let resolver = ResourceLoader::new(RejectingProvider);
    let mut source = svg_source(&resolver.budget);
    source.kind = ImageSourceKind::Svg(Arc::new(SvgDocument::parse(TWO_COLOR_SVG).unwrap()));
    assert_eq!(source.retained_byte_len(), None);
    let url = svg_url();
    assert!(
        resolver
            .load_source_with(&url, |_, _, _, _| Ok(Arc::new(source)))
            .is_err()
    );
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
    let state = resolver.state.lock().unwrap();
    assert!(state.loading.is_empty());
    assert!(state.cache.is_empty());
}

#[test]
fn compact_svg_doctype_normalization_preserves_intrinsic_size_and_pixels() {
    let mut bytes = br#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">"#.to_vec();
    bytes.extend_from_slice(TWO_COLOR_SVG);
    let url = svg_url_for(&bytes);
    let charge = url.as_str().len() as u64 + svg_retained_bytes(&bytes);
    let resolver = ResourceLoader::with_limits(RejectingProvider, charge + 128, MAX_CACHE_ENTRIES);
    let source = resolver.load_source(&url).unwrap();
    assert_eq!(resolver.budget.lock().unwrap().used, charge);
    assert_eq!(source.intrinsic.width, Some(4.0));
    assert_eq!(source.intrinsic.height, Some(2.0));
    let reference = SvgDocument::parse(&bytes).unwrap();
    for width in [4.0, 8.0, 4.0] {
        let actual = source.rasterize(box_of(width), None).unwrap();
        let expected = reference
            .rasterize(
                SvgViewport { width, height: 2.0 },
                SvgRootStyle::default(),
                None,
            )
            .unwrap();
        assert_eq!(actual.rgba, expected.rgba);
    }
}

#[test]
fn insufficient_encoded_source_room_rejects_before_parsing_and_refunds_key() {
    let url = svg_url();
    let limit = (url.as_str().len() + TWO_COLOR_SVG.len() - 1) as u64;
    let resolver = ResourceLoader::with_limits(RejectingProvider, limit, MAX_CACHE_ENTRIES);
    FULL_PARSES.with(|count| count.set(0));
    assert!(resolver.load_source(&url).is_err());
    assert_eq!(FULL_PARSES.with(Cell::get), 0);
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
    let state = resolver.state.lock().unwrap();
    assert!(state.cache.is_empty());
    assert!(state.loading.is_empty());
}
