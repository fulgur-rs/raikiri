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
