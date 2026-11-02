//! Self-contained `<img>` load-and-paint reftest.
//!
//! Not `#[ignore]`d and does not depend on `scripts/wpt/fetch.sh` — this is
//! the project's own substitute for the upstream WPT `<img>` basic-loading
//! coverage, which turned out to be either JS-dependent (unusable by a
//! JS-less engine) or SVG-fixture-dependent (out of this PNG-only scope).
//! Runs as a plain integration test in `cargo test --workspace`, gating CI
//! the same way any other test does.

use std::io::Write;

use raikiri::html_to_png_with_resolver;
use raikiri_net::{FileNetworkProvider, ImageResolver};

/// 2x2 RGBA8 PNG: red, green / blue, white (top-left, top-right / bottom-left,
/// bottom-right), generated the same way as the `raikiri-net`
/// `ImageResolver` fixture (see that crate's test for the generation
/// recipe) — regenerated independently for this 2x2 case rather than
/// depending on `raikiri-net`'s private test-only byte constant.
const RED_GREEN_BLUE_WHITE_2X2_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0,
    0, 0, 114, 182, 13, 36, 0, 0, 0, 23, 73, 68, 65, 84, 120, 156, 99, 249, 207, 192, 240, 159, 17,
    72, 176, 48, 50, 252, 7, 66, 6, 6, 0, 53, 94, 5, 6, 54, 228, 203, 251, 0, 0, 0, 0, 73, 69, 78,
    68, 174, 66, 96, 130,
];

#[test]
fn img_element_loads_and_paints_at_css_specified_size() {
    let mut tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
    tmp.write_all(RED_GREEN_BLUE_WHITE_2X2_PNG).unwrap();
    let url = url::Url::from_file_path(tmp.path()).unwrap();

    // 40x40 CSS box for a 2x2 intrinsic image — object-fit: fill (this
    // scope's only supported value) stretches each source pixel to a
    // 20x20 quadrant. The paint path samples the source image with
    // bilinear filtering, which interpolates between adjacent texel
    // centers (here: the 20px-spaced quadrant centers, e.g. x=10 and
    // x=30) — so a quadrant's own geometric center is actually inside the
    // blend zone between it and its neighbor, and is not a safe sample
    // point. Clamp-to-edge addressing means a source coordinate on the
    // near side of a texel's own center (e.g. device x=5, halfway
    // between the quadrant's edge at 0 and its center at 10) is pinned
    // to that texel with no blending. Sampling near each quadrant's
    // outer corner therefore gives an exact, unambiguous color, while
    // sampling at each quadrant's geometric center does not.
    let html = format!(
        r#"<html><head><style>@page {{ size: 40px 40px }} body {{ margin: 0 }} img {{ width: 40px; height: 40px }}</style></head><body><img src="{url}"></body></html>"#
    );

    let resolver = ImageResolver::new(FileNetworkProvider);
    let png = html_to_png_with_resolver(html.as_bytes(), &resolver, &resolver)
        .expect("should render the <img> end to end");

    let decoded = decode_png_for_assertion(&png);
    assert_eq!(decoded.width, 40);
    assert_eq!(decoded.height, 40);

    assert_eq!(
        pixel_at(&decoded, 5, 5),
        [255, 0, 0, 255],
        "top-left quadrant should be red"
    );
    assert_eq!(
        pixel_at(&decoded, 35, 5),
        [0, 255, 0, 255],
        "top-right quadrant should be green"
    );
    assert_eq!(
        pixel_at(&decoded, 5, 35),
        [0, 0, 255, 255],
        "bottom-left quadrant should be blue"
    );
    assert_eq!(
        pixel_at(&decoded, 35, 35),
        [255, 255, 255, 255],
        "bottom-right quadrant should be white"
    );
}

/// Solid blue 16x8 RGBA8 PNG, encoded on the fly.
fn solid_blue_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 16, 8);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("PNG header");
    writer
        .write_image_data(&[0, 0, 255, 255].repeat(16 * 8))
        .expect("PNG data");
    writer.finish().expect("PNG end");
    bytes
}

/// The columns and rows that hold a pure blue pixel, as (first, last) pairs.
fn blue_extent(image: &DecodedForAssertion) -> ((u32, u32), (u32, u32)) {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..image.height {
        for x in 0..image.width {
            if pixel_at(image, x, y) == [0, 0, 255, 255] {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    ((x0, x1), (y0, y1))
}

#[test]
fn img_at_a_fractional_position_covers_the_pixels_chromium_covers() {
    let mut tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
    tmp.write_all(&solid_blue_png()).unwrap();
    let url = url::Url::from_file_path(tmp.path()).unwrap();
    let resolver = ImageResolver::new(FileNetworkProvider);
    // Measured in Chromium at a device scale of 1: the clip of the image
    // content rounds each of its four edges to the nearest pixel, so a box on
    // half pixels covers the pixel after its far edge (x = 51..70, y = 11..20
    // for the first box) and not one pixel less.
    for (style, columns, rows) in [
        (
            "left:50.5px;top:10.5px;width:20px;height:10px",
            (51, 70),
            (11, 20),
        ),
        (
            "left:10.4px;top:10.6px;width:20.3px;height:10.2px",
            (10, 30),
            (11, 20),
        ),
        (
            "left:100.25px;top:40.75px;width:19.5px;height:9.5px",
            (100, 119),
            (41, 49),
        ),
    ] {
        let html = format!(
            r#"<html><head><style>@page {{ size: 200px 80px }} body {{ margin: 0 }} img {{ position: absolute; {style} }}</style></head><body><img src="{url}"></body></html>"#
        );
        let png = html_to_png_with_resolver(html.as_bytes(), &resolver, &resolver)
            .expect("should render the <img> end to end");
        let decoded = decode_png_for_assertion(&png);
        assert_eq!(
            blue_extent(&decoded),
            (columns, rows),
            "blue pixels of {style}"
        );
    }
}

struct DecodedForAssertion {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn decode_png_for_assertion(png_bytes: &[u8]) -> DecodedForAssertion {
    // `png::Decoder::new` requires `BufRead + Seek`, which a bare `&[u8]`
    // does not implement — wrap it the same way `raikiri-net`'s
    // `image_resolver::decode_png` does.
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().expect("valid PNG header");
    let buf_size = reader
        .output_buffer_size()
        .expect("PNG output buffer size should not overflow in this test");
    let mut buf = vec![0u8; buf_size];
    let info = reader.next_frame(&mut buf).expect("valid PNG frame");
    assert_eq!(
        info.color_type,
        png::ColorType::Rgba,
        "raikiri PNG output is always RGBA8"
    );
    DecodedForAssertion {
        width: info.width,
        height: info.height,
        rgba: buf[..info.buffer_size()].to_vec(),
    }
}

fn pixel_at(image: &DecodedForAssertion, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * image.width + x) * 4) as usize;
    [
        image.rgba[i],
        image.rgba[i + 1],
        image.rgba[i + 2],
        image.rgba[i + 3],
    ]
}
