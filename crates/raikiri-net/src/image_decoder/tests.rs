use super::*;
use std::io::Cursor;

/// A hand-encoded 2x1 RGBA8 PNG (red, green).
const TINY_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 244, 34, 127, 138, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 249, 207, 192, 240, 159,
    17, 72, 0, 0, 16, 33, 3, 3, 30, 93, 32, 80, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[test]
fn decodes_raw_bytes_without_url_or_provider() {
    let decoded = ImageDecoder.decode(TINY_PNG, 32 * 1024 * 1024).unwrap();
    assert_eq!((decoded.width, decoded.height), (2, 1));
    assert_eq!(decoded.rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]);
}

#[test]
fn rejects_invalid_image_bytes() {
    let error = ImageDecoder
        .decode(b"not an image", 32 * 1024 * 1024)
        .unwrap_err();
    assert!(!error.is_empty());
}

#[test]
fn normalizes_common_raster_formats_to_rgba8() {
    let source = image::RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 255, 0]).unwrap();
    for format in [
        image::ImageFormat::Jpeg,
        image::ImageFormat::Gif,
        image::ImageFormat::WebP,
    ] {
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(source.clone())
            .write_to(&mut encoded, format)
            .unwrap();
        let decoded = ImageDecoder
            .decode(encoded.get_ref(), 32 * 1024 * 1024)
            .unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.rgba.len(), 8);
        assert!(decoded.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }
}

#[test]
fn rejects_output_over_budget_before_normalization() {
    assert!(ImageDecoder.decode(TINY_PNG, 7).is_err());
    assert_eq!(ImageDecoder.decode(TINY_PNG, 8).unwrap().rgba.len(), 8);
}
