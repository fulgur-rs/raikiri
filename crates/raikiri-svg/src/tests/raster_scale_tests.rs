//! Rasterizing at a device scale: the picture of the CSS pixel grid, drawn
//! with more pixels per CSS pixel and with the caller's fonts.

use std::sync::Arc;

use crate::{
    SvgDocument, SvgError, SvgFontRequest, SvgFontSource, SvgImageAccess, SvgRootStyle, SvgViewport,
};
use raikiri_traits::DecodedImage;
use raikiri_traits::vector::FontFace;

const AHEM: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/Ahem.ttf"));

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

fn viewport(width: f32, height: f32) -> SvgViewport {
    SvgViewport { width, height }
}

fn document(attributes: &str, body: &str) -> SvgDocument {
    SvgDocument::parse(format!("<svg xmlns='{SVG_NAMESPACE}' {attributes}>{body}</svg>").as_bytes())
        .unwrap()
}

fn rasterize(document: &SvgDocument, viewport: SvgViewport, scale: f32) -> DecodedImage {
    document
        .rasterize_with_fonts(viewport, SvgRootStyle::default(), None, scale, None)
        .unwrap()
}

fn pixel(image: &DecodedImage, x: u32, y: u32) -> [u8; 4] {
    let start = ((y * image.width + x) * 4) as usize;
    image.rgba[start..start + 4].try_into().unwrap()
}

const RED: [u8; 4] = [255, 0, 0, 255];
const NOTHING: [u8; 4] = [0; 4];

/// The left half is red, as a `viewBox` that scales with the viewport.
fn left_half_red() -> SvgDocument {
    document(
        "width='2' height='1' viewBox='0 0 2 1'",
        "<rect width='1' height='1' fill='#f00'/>",
    )
}

#[test]
fn a_scale_of_one_is_the_picture_of_the_css_pixel_grid() {
    let svg = left_half_red();
    let scaled = rasterize(&svg, viewport(2.0, 1.0), 1.0);
    let plain = svg
        .rasterize_at_css_pixel_scale(viewport(2.0, 1.0), SvgRootStyle::default(), None)
        .unwrap();
    assert_eq!((scaled.width, scaled.height), (plain.width, plain.height));
    assert_eq!(scaled.rgba, plain.rgba);
}

#[test]
fn a_larger_scale_draws_the_same_picture_with_more_pixels() {
    let image = rasterize(&left_half_red(), viewport(2.0, 1.0), 2.0);
    assert_eq!((image.width, image.height), (4, 2));
    for y in 0..2 {
        assert_eq!(pixel(&image, 0, y), RED);
        assert_eq!(pixel(&image, 1, y), RED);
        assert_eq!(pixel(&image, 2, y), NOTHING);
        assert_eq!(pixel(&image, 3, y), NOTHING);
    }
}

#[test]
fn a_fraction_of_a_pixel_rounds_the_image_up() {
    let image = rasterize(&left_half_red(), viewport(2.5, 1.0), 2.0);
    assert_eq!((image.width, image.height), (5, 2));
    let image = rasterize(&left_half_red(), viewport(2.0, 1.0), 1.5);
    assert_eq!((image.width, image.height), (3, 2));
}

#[test]
fn a_picture_without_a_view_box_keeps_its_css_size_and_gains_pixels() {
    // The rectangle is two of the four CSS pixels, whatever the scale.
    let svg = document(
        "width='4' height='4'",
        "<rect width='2' height='4' fill='#f00'/>",
    );
    let image = rasterize(&svg, viewport(4.0, 4.0), 2.0);
    assert_eq!((image.width, image.height), (8, 8));
    assert_eq!(pixel(&image, 3, 4), RED);
    assert_eq!(pixel(&image, 4, 4), NOTHING);
}

#[test]
fn a_scale_that_is_no_size_is_refused() {
    let svg = left_half_red();
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            svg.rasterize_with_fonts(
                viewport(2.0, 1.0),
                SvgRootStyle::default(),
                None,
                scale,
                None
            ),
            Err(SvgError::InvalidViewport)
        ));
    }
}

#[test]
fn the_output_limit_counts_the_pixels_of_the_scaled_image() {
    let svg = left_half_red();
    let fits = svg.rasterize_with_fonts(
        viewport(1.0, 1.0),
        SvgRootStyle::default(),
        None,
        1.0,
        Some(4),
    );
    assert!(fits.is_ok());
    assert!(matches!(
        svg.rasterize_with_fonts(
            viewport(1.0, 1.0),
            SvgRootStyle::default(),
            None,
            2.0,
            Some(4)
        ),
        Err(SvgError::OutputLimitExceeded {
            bytes: 16,
            limit: 4
        })
    ));
}

/// Gives every request the Ahem face, in which each letter is a full square.
struct Ahem;

impl SvgFontSource for Ahem {
    fn select(&self, _request: &SvgFontRequest) -> Option<FontFace> {
        let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(AHEM);
        Some(FontFace::new(data, 0))
    }

    fn fallback(&self, _request: &SvgFontRequest, _ch: char) -> Option<FontFace> {
        None
    }
}

#[test]
fn text_is_set_in_the_fonts_given_and_left_out_without_them() {
    let svg = document(
        "width='16' height='16'",
        "<text x='0' y='8' font-family='Ahem' font-size='8'>A</text>",
    );
    let view = viewport(16.0, 16.0);
    let without = rasterize(&svg, view, 2.0);
    assert!(without.rgba.iter().all(|&byte| byte == 0));

    let with = svg
        .rasterize_with_fonts(view, SvgRootStyle::default(), Some(&Ahem), 2.0, None)
        .unwrap();
    assert_eq!((with.width, with.height), (32, 32));
    // The letter is a square of 8 CSS pixels on the baseline, 16 device
    // pixels at this scale.
    assert_eq!(pixel(&with, 8, 8), [0, 0, 0, 255]);
    assert_eq!(pixel(&with, 17, 8), NOTHING);
}

#[test]
fn a_picture_with_pictures_is_not_rasterized() {
    let svg = SvgDocument::parse_with_images(
        format!(
            "<svg xmlns='{SVG_NAMESPACE}' width='2' height='2'>\
             <image href='data:image/png;base64,AA==' width='1' height='1'/></svg>"
        )
        .as_bytes(),
        SvgImageAccess::Data,
    )
    .unwrap();
    assert!(matches!(
        svg.rasterize_with_fonts(
            viewport(2.0, 2.0),
            SvgRootStyle::default(),
            Some(&Ahem),
            2.0,
            None
        ),
        Err(SvgError::UnsupportedImages)
    ));
}
