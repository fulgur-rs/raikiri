use super::*;
use raikiri_traits::{DecodedImage, ImageIntrinsicSize, ImagePixelSource, ImageRasterSize};
use std::sync::Arc;

struct VectorMarker {
    intrinsic: ImageIntrinsicSize,
}

impl ImagePixelSource for VectorMarker {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::new(DecodedImage {
            width: 300,
            height: 150,
            rgba: vec![0; 300 * 150 * 4],
        }))
    }
    fn intrinsic_size(&self, _: &url::Url) -> Option<ImageIntrinsicSize> {
        Some(self.intrinsic)
    }
    fn get_decoded_at_size(
        &self,
        _: &url::Url,
        size: ImageRasterSize,
        _: Option<u64>,
    ) -> Option<Arc<DecodedImage>> {
        let width = size.width.ceil() as u32;
        let height = size.height.ceil() as u32;
        Some(Arc::new(DecodedImage {
            width,
            height,
            rgba: vec![0; (width * height * 4) as usize],
        }))
    }
}

#[test]
fn image_marker_resolves_missing_natural_dimensions_against_one_em() {
    for (intrinsic, expected) in [
        (
            ImageIntrinsicSize {
                width: None,
                height: None,
                aspect_ratio: Some(2.0),
            },
            (16, 8),
        ),
        (
            ImageIntrinsicSize {
                width: None,
                height: None,
                aspect_ratio: Some(0.5),
            },
            (8, 16),
        ),
        (
            ImageIntrinsicSize {
                width: None,
                height: None,
                aspect_ratio: None,
            },
            (16, 16),
        ),
        (
            ImageIntrinsicSize {
                width: Some(12.0),
                height: None,
                aspect_ratio: Some(2.0),
            },
            (12, 6),
        ),
        (
            ImageIntrinsicSize {
                width: None,
                height: Some(12.0),
                aspect_ratio: Some(2.0),
            },
            (24, 12),
        ),
        (
            ImageIntrinsicSize {
                width: Some(12.0),
                height: None,
                aspect_ratio: None,
            },
            (12, 16),
        ),
        (
            ImageIntrinsicSize {
                width: None,
                height: Some(12.0),
                aspect_ratio: None,
            },
            (16, 12),
        ),
        (
            ImageIntrinsicSize {
                width: Some(12.0),
                height: Some(10.0),
                aspect_ratio: Some(2.0),
            },
            (12, 10),
        ),
    ] {
        let mut doc = Document::new();
        let item = doc.append_element(
            Some(0),
            "li",
            taffy::Style::default(),
            Some("display:list-item;list-style:inside url(marker.svg);font-size:16px"),
        );
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        doc.prepare_list_marker_images(
            &cascade,
            &VectorMarker { intrinsic },
            Some(&url::Url::parse("https://images.test/").unwrap()),
        );
        let image = doc.list_marker_image(item).expect("prepared marker");
        assert_eq!((image.width, image.height), expected);
    }
}

#[test]
fn unavailable_marker_images_leave_the_text_fallback_and_clear_cached_pixels() {
    struct NoPixels;
    impl ImagePixelSource for NoPixels {
        fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
            None
        }
    }
    let mut doc = Document::new();
    for css in [
        "list-style:inside disc",
        "list-style:inside url(relative.svg)",
        "list-style:inside url(https://images.test/missing.svg)",
    ] {
        doc.append_element(
            Some(0),
            "li",
            taffy::Style::default(),
            Some(format!("display:list-item;{css}").as_str()),
        );
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    doc.prepare_list_marker_images(&cascade, &NoPixels, None);
    assert!(doc.list_marker_images.is_empty());
}

#[test]
fn marker_raster_variants_share_one_document_retained_byte_budget() {
    struct OversizedCapacityMarker;
    impl ImagePixelSource for OversizedCapacityMarker {
        fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
            None
        }
        fn intrinsic_size(&self, _: &url::Url) -> Option<ImageIntrinsicSize> {
            Some(ImageIntrinsicSize {
                width: None,
                height: None,
                aspect_ratio: Some(1.0),
            })
        }
        fn get_decoded_at_size(
            &self,
            _: &url::Url,
            size: ImageRasterSize,
            _: Option<u64>,
        ) -> Option<Arc<DecodedImage>> {
            let edge = size.width.ceil() as u32;
            // Reserve storage without touching the unused pages.
            let mut rgba = Vec::with_capacity(64 * 1024 * 1024 + 4);
            rgba.resize((edge * edge * 4) as usize, 0);
            Some(Arc::new(DecodedImage {
                width: edge,
                height: edge,
                rgba,
            }))
        }
    }
    let mut doc = Document::new();
    for em in [1, 2, 3] {
        doc.append_element(
            Some(0),
            "li",
            taffy::Style::default(),
            Some(format!("display:list-item;list-style:inside url(https://images.test/vector.svg);font-size:{em}px").as_str()),
        );
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    doc.prepare_list_marker_images(&cascade, &OversizedCapacityMarker, None);
    let retained: usize = doc
        .list_marker_images
        .values()
        .map(|image| image.pixels.rgba.capacity())
        .sum();
    assert!(retained <= 128 * 1024 * 1024, "retained {retained} bytes");
    assert_eq!(doc.list_marker_images.len(), 1);
}

fn marker_budget_document(
    sizes_and_urls: &[(u32, &str)],
) -> (Document, raikiri_style::CascadeResult, Vec<usize>) {
    let mut doc = Document::new();
    let items = sizes_and_urls
        .iter()
        .map(|(em, url)| {
            doc.append_element(
                Some(0),
                "li",
                taffy::Style::default(),
                Some(
                    format!("display:list-item;list-style:inside url({url});font-size:{em}px")
                        .as_str(),
                ),
            )
        })
        .collect();
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    (doc, cascade, items)
}

#[test]
fn sized_marker_decodes_receive_remaining_budget_and_reuse_variants_after_exhaustion() {
    struct BoundedVector {
        limits: std::cell::RefCell<Vec<Option<u64>>>,
    }
    impl ImagePixelSource for BoundedVector {
        fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
            None
        }
        fn intrinsic_size(&self, _: &url::Url) -> Option<ImageIntrinsicSize> {
            Some(ImageIntrinsicSize {
                width: None,
                height: None,
                aspect_ratio: Some(1.0),
            })
        }
        fn get_decoded_at_size(
            &self,
            _: &url::Url,
            size: ImageRasterSize,
            limit: Option<u64>,
        ) -> Option<Arc<DecodedImage>> {
            self.limits.borrow_mut().push(limit);
            let edge = size.width.ceil() as u32;
            let bytes = edge as u64 * edge as u64 * 4;
            if limit.is_some_and(|limit| bytes > limit) {
                return None;
            }
            Some(Arc::new(DecodedImage {
                width: edge,
                height: edge,
                rgba: vec![0; bytes as usize],
            }))
        }
    }
    let url = "https://images.test/dimensionless.svg";
    let (mut doc, cascade, items) =
        marker_budget_document(&[(12, url), (16, url), (20, url), (12, url)]);
    let source = BoundedVector {
        limits: Default::default(),
    };
    doc.prepare_list_marker_images_with_budget(&cascade, &source, None, 1600);
    assert_eq!(*source.limits.borrow(), [Some(1600), Some(1024), Some(0)]);
    assert_eq!(doc.list_marker_images.len(), 3);
    assert!(doc.list_marker_image(items[2]).is_none());
    assert!(Arc::ptr_eq(
        &doc.list_marker_images[&items[0]].pixels,
        &doc.list_marker_images[&items[3]].pixels
    ));
    let first = Arc::downgrade(&doc.list_marker_images[&items[0]].pixels);
    doc.prepare_list_marker_images_with_budget(&cascade, &source, None, 1600);
    assert!(first.upgrade().is_none());
    assert_eq!(
        *source.limits.borrow(),
        [
            Some(1600),
            Some(1024),
            Some(0),
            Some(1600),
            Some(1024),
            Some(0)
        ]
    );
    assert_eq!(doc.list_marker_images.len(), 3);
}

#[test]
fn marker_budget_counts_shared_pixel_allocation_once_across_urls() {
    struct SharedRaster {
        shared: Arc<DecodedImage>,
    }
    impl ImagePixelSource for SharedRaster {
        fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
            Some(if url.path() == "/unique" {
                Arc::new(DecodedImage {
                    width: 2,
                    height: 2,
                    rgba: vec![0; 16],
                })
            } else {
                Arc::clone(&self.shared)
            })
        }
    }
    let (mut doc, cascade, items) = marker_budget_document(&[
        (16, "https://images.test/a"),
        (16, "https://images.test/b"),
        (16, "https://images.test/unique"),
    ]);
    let source = SharedRaster {
        shared: Arc::new(DecodedImage {
            width: 2,
            height: 2,
            rgba: vec![0; 16],
        }),
    };
    doc.prepare_list_marker_images_with_budget(&cascade, &source, None, 32);
    assert_eq!(doc.list_marker_images.len(), 3);
    assert!(Arc::ptr_eq(
        &doc.list_marker_images[&items[0]].pixels,
        &doc.list_marker_images[&items[1]].pixels
    ));
    assert!(!Arc::ptr_eq(
        &doc.list_marker_images[&items[0]].pixels,
        &doc.list_marker_images[&items[2]].pixels
    ));
}

#[test]
fn invalid_marker_rasters_do_not_consume_the_document_budget() {
    struct InvalidRaster;
    impl ImagePixelSource for InvalidRaster {
        fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
            None
        }
        fn intrinsic_size(&self, _: &url::Url) -> Option<ImageIntrinsicSize> {
            Some(ImageIntrinsicSize {
                width: Some(1.0),
                height: Some(1.0),
                aspect_ratio: Some(1.0),
            })
        }
        fn get_decoded_at_size(
            &self,
            url: &url::Url,
            _: ImageRasterSize,
            limit: Option<u64>,
        ) -> Option<Arc<DecodedImage>> {
            assert_eq!(limit, Some(4));
            let (width, height) = match url.path() {
                "/zero-width" => (0, 1),
                "/zero-height" => (1, 0),
                _ => (1, 1),
            };
            Some(Arc::new(DecodedImage {
                width,
                height,
                rgba: vec![0; 4],
            }))
        }
    }
    let (mut doc, cascade, items) = marker_budget_document(&[
        (16, "https://images.test/zero-width"),
        (16, "https://images.test/zero-height"),
        (16, "https://images.test/valid"),
    ]);
    doc.prepare_list_marker_images_with_budget(&cascade, &InvalidRaster, None, 4);
    assert_eq!(doc.list_marker_images.len(), 1);
    assert!(doc.list_marker_image(items[2]).is_some());
}

#[test]
fn content_image_size_follows_the_default_sizing_algorithm() {
    use raikiri_style::ComputedLengthPercentageOrAuto as Length;
    let natural =
        |width: Option<f32>, height: Option<f32>, ratio: Option<f32>| ImageIntrinsicSize {
            width,
            height,
            aspect_ratio: ratio,
        };
    let sized = |width: Length, height: Length| {
        let mut style = raikiri_style::ComputedValues::initial();
        style.width = width;
        style.height = height;
        style
    };
    let size = |intrinsic, style: Option<&raikiri_style::ComputedValues>| {
        let size = content_image_size(intrinsic, style);
        (size.width, size.height)
    };
    // Natural dimensions, with the 300x150 default object size filling in.
    for (intrinsic, expected) in [
        (natural(Some(12.0), Some(10.0), Some(2.0)), (12.0, 10.0)),
        (natural(Some(12.0), None, Some(2.0)), (12.0, 6.0)),
        (natural(None, Some(12.0), Some(2.0)), (24.0, 12.0)),
        (natural(Some(12.0), None, None), (12.0, 150.0)),
        (natural(None, Some(12.0), None), (300.0, 12.0)),
        // A ratio alone is contained in the default object size.
        (natural(None, None, Some(4.0)), (300.0, 75.0)),
        (natural(None, None, Some(0.5)), (75.0, 150.0)),
        (natural(None, None, None), (300.0, 150.0)),
        // Invalid natural dimensions are ignored.
        (
            natural(Some(f32::NAN), Some(-1.0), Some(0.0)),
            (300.0, 150.0),
        ),
    ] {
        assert_eq!(size(intrinsic, None), expected, "{intrinsic:?}");
    }
    // Absolute `width` and `height` of a replacing image.
    let auto = Length::Auto;
    for (intrinsic, style, expected) in [
        (
            natural(Some(8.0), Some(4.0), None),
            sized(Length::Px(30.0), Length::Px(10.0)),
            (30.0, 10.0),
        ),
        (
            natural(Some(8.0), Some(4.0), None),
            sized(Length::Px(16.0), auto),
            (16.0, 8.0),
        ),
        (
            natural(None, Some(4.0), None),
            sized(Length::Px(16.0), auto),
            (16.0, 4.0),
        ),
        (
            natural(None, None, None),
            sized(Length::Px(16.0), auto),
            (16.0, 150.0),
        ),
        (
            natural(Some(8.0), Some(4.0), None),
            sized(auto, Length::Px(10.0)),
            (20.0, 10.0),
        ),
        (
            natural(Some(8.0), None, None),
            sized(auto, Length::Px(10.0)),
            (8.0, 10.0),
        ),
        (
            natural(None, None, None),
            sized(auto, Length::Px(10.0)),
            (300.0, 10.0),
        ),
        // Percentages and negative lengths do not size the image.
        (
            natural(Some(8.0), Some(4.0), None),
            sized(Length::Percent(50.0), Length::Px(-1.0)),
            (8.0, 4.0),
        ),
    ] {
        assert_eq!(size(intrinsic, Some(&style)), expected, "{intrinsic:?}");
    }
}

/// `html > body > p` with `css` as the author stylesheet.
fn content_image_document(css: &str) -> (Document, raikiri_style::CascadeResult, usize) {
    let mut doc = Document::new();
    let style = doc.append_element(Some(0), "style", taffy::Style::default(), None::<&str>);
    doc.append_text(style, css);
    let html = doc.append_element(Some(0), "html", taffy::Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", taffy::Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", taffy::Style::default(), None::<&str>);
    doc.append_text(p, "a");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    (doc, cascade, p)
}

#[test]
fn content_images_that_cannot_be_sized_represent_nothing() {
    struct PartialSizes;
    impl ImagePixelSource for PartialSizes {
        fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
            (!url.path().ends_with("broken.png")).then(|| {
                Arc::new(DecodedImage {
                    width: 2,
                    height: 2,
                    rgba: vec![0; 16],
                })
            })
        }
        fn intrinsic_size(&self, url: &url::Url) -> Option<ImageIntrinsicSize> {
            (!url.path().ends_with("unsized.png")).then_some(ImageIntrinsicSize {
                width: Some(2.0),
                height: Some(2.0),
                aspect_ratio: Some(1.0),
            })
        }
    }
    let (mut doc, cascade, p) = content_image_document(
        "p::before{content:url(relative.png) url(https://images.test/unsized.png) \
         url(https://images.test/ok.png) url(https://images.test/broken.png)}\
         p::after{content:url(https://images.test/ok.png);width:0px}",
    );
    // Without a base URL a relative URL cannot be resolved.
    doc.prepare_list_marker_images(&cascade, &PartialSizes, None);
    use raikiri_style::PseudoElem;
    assert!(doc.content_image(p, PseudoElem::Before, 0).is_none());
    assert!(doc.content_image(p, PseudoElem::Before, 1).is_none());
    assert_eq!(
        doc.content_image_url(p, PseudoElem::Before, 2)
            .map(url::Url::as_str),
        Some("https://images.test/ok.png")
    );
    assert_eq!(
        doc.content_image_size(p, PseudoElem::Before, 2),
        Some(ImageRasterSize {
            width: 2.0,
            height: 2.0
        })
    );
    // An image without pixels is not prepared.
    assert!(doc.content_image(p, PseudoElem::Before, 3).is_none());
    // A replacing image sized to zero is not prepared.
    assert!(doc.content_image(p, PseudoElem::After, 0).is_none());
    // `::marker` has no content images.
    assert!(doc.content_image(p, PseudoElem::Marker, 0).is_none());
    // A base URL resolves the relative image.
    doc.prepare_list_marker_images(
        &cascade,
        &PartialSizes,
        Some(&url::Url::parse("https://images.test/doc.html").unwrap()),
    );
    assert_eq!(
        doc.content_image_url(p, PseudoElem::Before, 0)
            .map(url::Url::as_str),
        Some("https://images.test/relative.png")
    );
}

#[test]
fn extracted_documents_keep_the_content_images_of_listed_elements() {
    let (mut doc, cascade, p) =
        content_image_document("p::before{content:url(https://images.test/a.png)}");
    doc.prepare_list_marker_images(
        &cascade,
        &VectorMarker {
            intrinsic: ImageIntrinsicSize {
                width: Some(4.0),
                height: Some(2.0),
                aspect_ratio: Some(2.0),
            },
        },
        None,
    );
    let body = doc.parent_of(p).unwrap();
    let html = doc.parent_of(body).unwrap();
    let text = doc.get_node(p).unwrap().children[0];
    let before = raikiri_style::PseudoElem::Before;
    let copy = doc.extract_nodes(&[0, html, body, p, text]);
    let image = copy
        .content_image(3, before, 0)
        .expect("content image follows p");
    assert_eq!((image.width, image.height), (4, 2));
    assert!(std::ptr::eq(
        image,
        doc.content_image(p, before, 0).unwrap()
    ));
    // The image of an element left out is not carried over.
    let without = doc.extract_nodes(&[0, html, body]);
    assert!(without.content_images.is_empty());
}

#[test]
fn replacing_content_images_obey_min_and_max_sizes() {
    use raikiri_style::ComputedLengthPercentageOrAuto as Length;
    let natural =
        |width: Option<f32>, height: Option<f32>, ratio: Option<f32>| ImageIntrinsicSize {
            width,
            height,
            aspect_ratio: ratio,
        };
    let wide = natural(Some(100.0), Some(50.0), None);
    let px = Length::Px;
    let auto = Length::Auto;
    // (width, height, min-width, max-width, min-height, max-height)
    for (intrinsic, [width, height, min_width, max_width, min_height, max_height], expected) in [
        // Both dimensions auto: the CSS 2.1 §10.4 table keeps the ratio.
        (wide, [auto, auto, auto, px(20.0), auto, auto], (20.0, 10.0)),
        (
            wide,
            [auto, auto, auto, auto, px(100.0), auto],
            (200.0, 100.0),
        ),
        (
            wide,
            [auto, auto, px(150.0), auto, auto, auto],
            (150.0, 75.0),
        ),
        (wide, [auto, auto, auto, auto, auto, px(10.0)], (20.0, 10.0)),
        (
            wide,
            [auto, auto, auto, px(20.0), px(40.0), auto],
            (20.0, 40.0),
        ),
        (
            wide,
            [auto, auto, px(150.0), auto, auto, px(40.0)],
            (150.0, 40.0),
        ),
        (
            wide,
            [auto, auto, auto, px(20.0), auto, px(5.0)],
            (10.0, 5.0),
        ),
        (
            wide,
            [auto, auto, auto, px(40.0), auto, px(5.0)],
            (10.0, 5.0),
        ),
        (
            wide,
            [auto, auto, auto, px(20.0), auto, px(30.0)],
            (20.0, 10.0),
        ),
        (
            wide,
            [auto, auto, px(200.0), auto, px(200.0), auto],
            (400.0, 200.0),
        ),
        (
            wide,
            [auto, auto, px(400.0), auto, px(150.0), auto],
            (400.0, 200.0),
        ),
        (
            wide,
            [auto, auto, px(400.0), auto, px(150.0), px(160.0)],
            (400.0, 160.0),
        ),
        (
            wide,
            [auto, auto, auto, auto, px(60.0), px(5.0)],
            (120.0, 60.0),
        ),
        // A max below the min is raised to the min.
        (
            wide,
            [auto, auto, px(30.0), px(10.0), auto, auto],
            (30.0, 15.0),
        ),
        // Without a ratio each dimension is clamped on its own.
        (
            natural(Some(100.0), None, None),
            [auto, auto, auto, auto, auto, px(50.0)],
            (100.0, 50.0),
        ),
        // A used width clamps before the height follows the ratio.
        (
            wide,
            [px(30.0), auto, auto, px(20.0), auto, auto],
            (20.0, 10.0),
        ),
        (
            wide,
            [auto, px(30.0), auto, auto, px(40.0), auto],
            (80.0, 40.0),
        ),
        (
            wide,
            [px(30.0), auto, auto, auto, auto, px(5.0)],
            (30.0, 5.0),
        ),
        (
            wide,
            [px(30.0), px(30.0), px(40.0), auto, auto, px(20.0)],
            (40.0, 20.0),
        ),
        // Percentages do not constrain the image.
        (
            wide,
            [
                auto,
                auto,
                Length::Percent(10.0),
                Length::Percent(10.0),
                auto,
                auto,
            ],
            (100.0, 50.0),
        ),
    ] {
        let mut style = raikiri_style::ComputedValues::initial();
        style.width = width;
        style.height = height;
        style.min_width = min_width;
        style.max_width = max_width;
        style.min_height = min_height;
        style.max_height = max_height;
        let size = content_image_size(intrinsic, Some(&style));
        assert_eq!(
            (size.width, size.height),
            expected,
            "{width:?} {height:?} {min_width:?} {max_width:?} {min_height:?} {max_height:?}"
        );
    }
}

#[test]
fn prepared_replacing_content_images_use_the_pseudo_elements_max_width() {
    let (mut doc, cascade, p) = content_image_document(
        "p::before{content:url(https://images.test/a.png);max-width:2px}\
         p::after{content:'x' url(https://images.test/a.png);max-width:2px}",
    );
    doc.prepare_list_marker_images(
        &cascade,
        &VectorMarker {
            intrinsic: ImageIntrinsicSize {
                width: Some(4.0),
                height: Some(2.0),
                aspect_ratio: Some(2.0),
            },
        },
        None,
    );
    use raikiri_style::PseudoElem;
    assert_eq!(
        doc.content_image_size(p, PseudoElem::Before, 0),
        Some(ImageRasterSize {
            width: 2.0,
            height: 1.0
        })
    );
    // An image among other content does not replace the pseudo-element.
    assert_eq!(
        doc.content_image_size(p, PseudoElem::After, 0),
        Some(ImageRasterSize {
            width: 4.0,
            height: 2.0
        })
    );
}

#[test]
fn content_images_of_unrendered_owners_are_not_decoded_or_charged() {
    let mut doc = Document::new();
    let style = doc.append_element(Some(0), "style", taffy::Style::default(), None::<&str>);
    doc.append_text(style, "p::before{content:url(https://images.test/a.png)}");
    let html = doc.append_element(Some(0), "html", taffy::Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", taffy::Style::default(), None::<&str>);
    // Owners in a `display: none` subtree, and one that is itself
    // `display: none`, come first in key order.
    let hidden = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:none"),
    );
    let in_hidden = doc.append_element(Some(hidden), "p", taffy::Style::default(), None::<&str>);
    let deep = doc.append_element(Some(hidden), "div", taffy::Style::default(), None::<&str>);
    let in_deep = doc.append_element(Some(deep), "p", taffy::Style::default(), None::<&str>);
    let itself = doc.append_element(
        Some(body),
        "p",
        taffy::Style::default(),
        Some("display:none"),
    );
    let visible = doc.append_element(Some(body), "p", taffy::Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    // The 4x2 decode costs 32 bytes: the budget holds one image.
    doc.prepare_list_marker_images_with_budget(
        &cascade,
        &VectorMarker {
            intrinsic: ImageIntrinsicSize {
                width: Some(4.0),
                height: Some(2.0),
                aspect_ratio: Some(2.0),
            },
        },
        None,
        32,
    );
    use raikiri_style::PseudoElem;
    for owner in [in_hidden, in_deep, itself] {
        assert!(
            doc.content_image(owner, PseudoElem::Before, 0).is_none(),
            "owner {owner}"
        );
    }
    assert!(doc.content_image(visible, PseudoElem::Before, 0).is_some());
}

#[test]
fn a_covering_content_image_too_large_for_the_budget_is_rastered_at_its_box_area() {
    struct BoundedVector;
    impl ImagePixelSource for BoundedVector {
        fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
            None
        }
        fn intrinsic_size(&self, _: &url::Url) -> Option<ImageIntrinsicSize> {
            Some(ImageIntrinsicSize {
                width: Some(1.0),
                height: Some(1.0),
                aspect_ratio: Some(1.0),
            })
        }
        fn get_decoded_at_size(
            &self,
            _: &url::Url,
            size: ImageRasterSize,
            limit: Option<u64>,
        ) -> Option<Arc<DecodedImage>> {
            let width = size.width.ceil() as u32;
            let height = size.height.ceil() as u32;
            let bytes = u64::from(width) * u64::from(height) * 4;
            if limit.is_some_and(|limit| bytes > limit) {
                return None;
            }
            Some(Arc::new(DecodedImage {
                width,
                height,
                rgba: vec![0; bytes as usize],
            }))
        }
    }
    // The 40x4 box covered by a square image holds a 40x40 object, which
    // costs 6400 bytes; the budget only holds a raster of the box's area.
    let (mut doc, cascade, p) = content_image_document(
        "p::before{content:url(https://images.test/a.svg);width:40px;height:4px;object-fit:cover}",
    );
    doc.prepare_list_marker_images_with_budget(&cascade, &BoundedVector, None, 1000);
    use raikiri_style::PseudoElem;
    let image = doc
        .content_image(p, PseudoElem::Before, 0)
        .expect("the image still exists");
    assert_eq!((image.width, image.height), (13, 13));
    // It is still drawn over the whole object.
    let object = doc
        .content_image_object_rect(p, PseudoElem::Before, 0)
        .unwrap();
    assert_eq!((object.width, object.height), (40.0, 40.0));
    let raster = doc
        .content_image_raster_size(p, PseudoElem::Before, 0)
        .unwrap();
    assert!((raster.width - 160f32.sqrt()).abs() < 1e-4);
    assert!((raster.height - 160f32.sqrt()).abs() < 1e-4);
}

#[test]
fn a_content_image_clipped_on_one_axis_is_rastered_at_its_visible_area() {
    struct WideVector;
    impl ImagePixelSource for WideVector {
        fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
            None
        }
        fn intrinsic_size(&self, _: &url::Url) -> Option<ImageIntrinsicSize> {
            Some(ImageIntrinsicSize {
                width: Some(1000.0),
                height: Some(1.0),
                aspect_ratio: Some(1000.0),
            })
        }
        fn get_decoded_at_size(
            &self,
            _: &url::Url,
            size: ImageRasterSize,
            limit: Option<u64>,
        ) -> Option<Arc<DecodedImage>> {
            let width = size.width.ceil() as u32;
            let height = size.height.ceil() as u32;
            let bytes = u64::from(width) * u64::from(height) * 4;
            if limit.is_some_and(|limit| bytes > limit) {
                return None;
            }
            Some(Arc::new(DecodedImage {
                width,
                height,
                rgba: vec![0; bytes as usize],
            }))
        }
    }
    // The 1000x1 object protrudes from its 100x100 box only sideways, so it
    // is smaller than the box; its 4000-byte raster does not fit the budget.
    let (mut doc, cascade, p) = content_image_document(
        "p::before{content:url(https://images.test/a.svg);width:100px;height:100px;object-fit:none}",
    );
    doc.prepare_list_marker_images_with_budget(&cascade, &WideVector, None, 2000);
    use raikiri_style::PseudoElem;
    let image = doc
        .content_image(p, PseudoElem::Before, 0)
        .expect("the image still exists");
    assert_eq!((image.width, image.height), (100, 1));
}

#[test]
fn a_content_image_moved_out_of_its_box_is_rastered_at_its_visible_area() {
    struct SmallVector;
    impl ImagePixelSource for SmallVector {
        fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
            None
        }
        fn intrinsic_size(&self, _: &url::Url) -> Option<ImageIntrinsicSize> {
            Some(ImageIntrinsicSize {
                width: Some(50.0),
                height: Some(50.0),
                aspect_ratio: Some(1.0),
            })
        }
        fn get_decoded_at_size(
            &self,
            _: &url::Url,
            size: ImageRasterSize,
            limit: Option<u64>,
        ) -> Option<Arc<DecodedImage>> {
            let width = size.width.ceil() as u32;
            let height = size.height.ceil() as u32;
            let bytes = u64::from(width) * u64::from(height) * 4;
            if limit.is_some_and(|limit| bytes > limit) {
                return None;
            }
            Some(Arc::new(DecodedImage {
                width,
                height,
                rgba: vec![0; bytes as usize],
            }))
        }
    }
    // The 50x50 object fits its 100x100 box, but `object-position` leaves
    // only a 10x10 corner inside; its 10000-byte raster does not fit the
    // budget.
    let (mut doc, cascade, p) = content_image_document(
        "p::before{content:url(https://images.test/a.svg);width:100px;height:100px;\
         object-fit:none;object-position:90px 90px}",
    );
    doc.prepare_list_marker_images_with_budget(&cascade, &SmallVector, None, 1000);
    use raikiri_style::PseudoElem;
    let image = doc
        .content_image(p, PseudoElem::Before, 0)
        .expect("the image still exists");
    assert_eq!((image.width, image.height), (10, 10));
}

#[test]
fn a_content_image_of_extreme_ratio_falls_back_to_its_visible_area() {
    struct ThinVector;
    impl ImagePixelSource for ThinVector {
        fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
            None
        }
        fn intrinsic_size(&self, _: &url::Url) -> Option<ImageIntrinsicSize> {
            Some(ImageIntrinsicSize {
                width: Some(100_000_000.0),
                height: Some(1.0),
                aspect_ratio: Some(100_000_000.0),
            })
        }
        fn get_decoded_at_size(
            &self,
            _: &url::Url,
            size: ImageRasterSize,
            limit: Option<u64>,
        ) -> Option<Arc<DecodedImage>> {
            let width = size.width.ceil() as u64;
            let height = size.height.ceil() as u64;
            let bytes = width * height * 4;
            if limit.is_some_and(|limit| bytes > limit) {
                return None;
            }
            Some(Arc::new(DecodedImage {
                width: width as u32,
                height: height as u32,
                rgba: vec![0; bytes as usize],
            }))
        }
    }
    // Only 100 pixels of the object show in its 100x100 box; scaling it by
    // area alone would leave a raster 100000 pixels wide.
    let (mut doc, cascade, p) = content_image_document(
        "p::before{content:url(https://images.test/a.svg);width:100px;height:100px;object-fit:none}",
    );
    doc.prepare_list_marker_images_with_budget(&cascade, &ThinVector, None, 40_000);
    use raikiri_style::PseudoElem;
    let image = doc
        .content_image(p, PseudoElem::Before, 0)
        .expect("the image still exists");
    assert_eq!((image.width, image.height), (100, 1));
}

#[test]
fn standalone_image_marker_sits_inside_or_before_the_padding_edge() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", taffy::Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", taffy::Style::default(), None::<&str>);
    let inside = doc.append_element(
        Some(body),
        "li",
        taffy::Style::default(),
        Some("list-style-position:inside"),
    );
    let outside = doc.append_element(Some(body), "li", taffy::Style::default(), None::<&str>);
    let cascade =
        raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).expect("cascade");
    let size = ImageRasterSize {
        width: 8.0,
        height: 6.0,
    };
    // An inside marker starts at the content edge; an outside one ends 4px
    // before it.
    assert_eq!(
        doc.standalone_marker_image_rect(&cascade.computed[inside], (2.0, 3.0), 10.0, size),
        raikiri_traits::PaintRect::new(12.0, 3.0, 8.0, 6.0)
    );
    assert_eq!(
        doc.standalone_marker_image_rect(&cascade.computed[outside], (2.0, 3.0), 10.0, size),
        raikiri_traits::PaintRect::new(0.0, 3.0, 8.0, 6.0)
    );
}
