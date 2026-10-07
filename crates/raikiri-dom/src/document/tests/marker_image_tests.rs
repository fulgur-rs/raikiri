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
