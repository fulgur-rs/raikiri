//! Public SVG data-image memo ownership stays within shared cache budgets.

use image::{ExtendedColorType, ImageEncoder};
use raikiri_html::RenderResources;
use raikiri_svg::SvgImageSource;
use raikiri_traits::DecodedImage;
use std::collections::HashSet;
use std::sync::{Arc, Weak};

const MIB: usize = 1024 * 1024;

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let rgba = [255, 0, 0, 255].repeat(512 * 512);
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&rgba, 512, 512, ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb88320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn variant(png: &[u8], index: usize) -> Vec<u8> {
    let text = format!("key\0{index:08}");
    let mut chunk = b"tEXt".to_vec();
    chunk.extend(text.as_bytes());
    let mut bytes = png[..png.len() - 12].to_vec();
    bytes.extend((text.len() as u32).to_be_bytes());
    bytes.extend(&chunk);
    bytes.extend(crc32(&chunk).to_be_bytes());
    bytes.extend(&png[png.len() - 12..]);
    bytes
}

fn retained(images: &[Weak<DecodedImage>]) -> (usize, usize) {
    let mut seen = HashSet::new();
    let mut total = 0;
    for weak in images {
        if let Some(image) = weak.upgrade()
            && seen.insert(Arc::as_ptr(&image) as usize)
        {
            total += image.rgba.len();
        }
    }
    (seen.len(), total)
}

fn request(source: &impl SvgImageSource, bytes: &[u8], weak: &mut Vec<Weak<DecodedImage>>) -> bool {
    let Some(image) = source.data("image/png", bytes) else {
        return false;
    };
    assert_eq!((image.width, image.height), (512, 512));
    assert_eq!(&image.rgba[..4], [255, 0, 0, 255]);
    assert_eq!(&image.rgba[image.rgba.len() - 4..], [255, 0, 0, 255]);
    weak.push(Arc::downgrade(&image));
    true
}

#[test]
fn data_memos_share_resource_and_process_limits_and_release_on_source_drop() {
    let png = png();
    let inputs = (0..160)
        .map(|index| variant(&png, index))
        .collect::<Vec<_>>();
    eprintln!(
        "SVG data fixture: encoded_bytes_per_picture={}",
        inputs[0].len()
    );
    let resources = RenderResources::new();
    let source = resources.svg_images(None);
    let mut weak = Vec::new();
    let admitted = inputs
        .iter()
        .filter(|bytes| request(&source, bytes, &mut weak))
        .count();
    let (count, bytes) = retained(&weak);
    eprintln!("single SVG data memo: admitted={admitted}, retained={count}, rgba_bytes={bytes}");
    assert!(admitted > 0);
    assert!(bytes <= 128 * MIB, "memo retains {bytes} RGBA bytes");
    drop(source);
    assert_eq!(retained(&weak), (0, 0));

    let source = resources.svg_images(None);
    weak.clear();
    for _ in 0..160 {
        assert!(request(&source, &inputs[0], &mut weak));
    }
    assert_eq!(retained(&weak), (1, MIB));
    assert!(source.take_warnings().is_empty());
    drop(source);
    assert_eq!(retained(&weak), (0, 0));

    let resources = (0..3).map(|_| RenderResources::new()).collect::<Vec<_>>();
    let sources = resources
        .iter()
        .map(|resource| resource.svg_images(None))
        .collect::<Vec<_>>();
    weak.clear();
    for (index, source) in sources.iter().enumerate() {
        let mut own = Vec::new();
        let admitted = inputs
            .iter()
            .filter(|bytes| request(source, bytes, &mut own))
            .count();
        let (count, bytes) = retained(&own);
        eprintln!(
            "shared process SVG data: set={index}, admitted={admitted}, retained={count}, rgba_bytes={bytes}"
        );
        assert!(bytes <= 128 * MIB);
        weak.extend(own);
    }
    let (count, bytes) = retained(&weak);
    eprintln!("shared process SVG data total: retained={count}, rgba_bytes={bytes}");
    assert!(bytes <= 256 * MIB, "process retains {bytes} RGBA bytes");
    drop(sources);
    assert_eq!(retained(&weak), (0, 0));
    let fresh = resources[0].svg_images(None);
    let caller = fresh.data("image/png", &inputs[0]).unwrap();
    let weak = Arc::downgrade(&caller);
    drop(fresh);
    assert_eq!(weak.strong_count(), 1);
    drop(caller);
    assert_eq!(weak.strong_count(), 0);
}
