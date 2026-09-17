//! `ImageResolver` — fetches + decodes `<img>` PNG bytes via a `NetworkProvider`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use raikiri_traits::{
    Body, DecodedImage, FetchedResource, ImagePixelSource, IntrinsicBox, Method, NetworkProvider,
    ReplacedResolver, Request, ResolveDisposition, ResolvedIntrinsic, ResolverError,
    ResolverRequest, policy::ResourceKind,
};
use url::Url;

/// `ReplacedResolver` + `ImagePixelSource` for `<img>` elements, backed by a
/// PNG decoder and a Consumer-supplied [`NetworkProvider`].
///
/// Fetch + decode happen synchronously inside [`ReplacedResolver::resolve`];
/// results are cached by URL so a later [`ImagePixelSource::get_decoded`]
/// call for the same element (at paint time) is a cache hit, not a
/// re-fetch/re-decode.
pub struct ImageResolver<N> {
    network: N,
    cache: Mutex<HashMap<Url, Arc<DecodedImage>>>,
}

impl<N: NetworkProvider> ImageResolver<N> {
    /// Wraps `network` with PNG fetch+decode+cache.
    pub fn new(network: N) -> Self {
        Self {
            network,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// `ResolverError`'s `Network(NetworkError)` variant is ~144 bytes
    /// (`NetworkError::PolicyViolation` is the largest variant), over
    /// clippy's 128-byte `result_large_err` threshold. That size lives in
    /// `raikiri-traits`'s error shape, not this call site, so the lint is
    /// suppressed here rather than boxing just this one caller.
    #[allow(clippy::result_large_err)]
    fn fetch_and_decode(&self, url: &Url) -> Result<Arc<DecodedImage>, ResolverError> {
        if let Some(cached) = self.cache.lock().unwrap().get(url) {
            return Ok(cached.clone());
        }
        let request = Request {
            url: url.clone(),
            method: Method::Get,
            content_type: None,
            headers: Vec::new(),
            body: Body::Empty,
            signal: None,
            kind: ResourceKind::Image,
        };
        let fetched: FetchedResource = self
            .network
            .fetch(request)
            .map_err(ResolverError::Network)?;
        let decoded = decode_png(&fetched.bytes).map_err(ResolverError::Decode)?;
        let decoded = Arc::new(decoded);
        self.cache
            .lock()
            .unwrap()
            .insert(url.clone(), decoded.clone());
        Ok(decoded)
    }
}

impl<N: NetworkProvider> ReplacedResolver for ImageResolver<N> {
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        let decoded = self.fetch_and_decode(req.url())?;
        Ok(ResolvedIntrinsic {
            intrinsic: IntrinsicBox::new(decoded.width as f32, decoded.height as f32),
            disposition: ResolveDisposition::Ok,
        })
    }
}

impl<N: NetworkProvider> ImagePixelSource for ImageResolver<N> {
    fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        self.cache.lock().unwrap().get(url).cloned()
    }
}

/// Decodes PNG bytes into straight (non-premultiplied) RGBA8.
///
/// Supports 8-bit Rgba/Rgb/Grayscale/GrayscaleAlpha source color types
/// (normalizing all to RGBA8); anything else (16-bit depth, sub-byte
/// (1/2/4-bit) depth, indexed/palette) is a decode error. Interlaced PNGs
/// decode fine — `png::Reader::next_frame` de-interlaces into the output
/// buffer before this function ever sees it. This MVP only needs to decode
/// images this same pipeline or common tools produce, not the full PNG
/// format matrix.
fn decode_png(bytes: &[u8]) -> Result<DecodedImage, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let buf_size = reader
        .output_buffer_size()
        .ok_or_else(|| "PNG output buffer size overflows address space".to_string())?;
    let mut buf = vec![0u8; buf_size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("unsupported PNG bit depth: {:?}", info.bit_depth));
    }
    let raw = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => raw.to_vec(),
        png::ColorType::Rgb => raw
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => raw.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => raw
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        other => return Err(format!("unsupported PNG color type: {other:?}")),
    };
    // Defensive invariant check: `DecodedImage::rgba`'s contract (see
    // raikiri-traits::image) is exactly `width * height * 4` bytes. Catches
    // any future color-type/transformation combination that slips past the
    // arms above without normalizing to that shape, converting a would-be
    // silent out-of-bounds read at paint time into a decode error here.
    let expected_len = info.width as usize * info.height as usize * 4;
    if rgba.len() != expected_len {
        return Err(format!(
            "decoded PNG pixel buffer size mismatch: expected {expected_len} bytes, got {}",
            rgba.len()
        ));
    }
    Ok(DecodedImage {
        width: info.width,
        height: info.height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raikiri_traits::{ReplacedResolver, ResolverRequest};

    struct StaticBytesProvider(&'static [u8]);
    impl raikiri_traits::NetworkProvider for StaticBytesProvider {
        fn fetch(
            &self,
            _request: raikiri_traits::Request,
        ) -> Result<raikiri_traits::FetchedResource, raikiri_traits::NetworkError> {
            Ok(raikiri_traits::FetchedResource {
                bytes: self.0.to_vec().into(),
                content_type: Some("image/png".into()),
                final_url: url::Url::parse("file:///fixture.png").unwrap(),
                encoding: None,
            })
        }
    }

    /// A hand-encoded 2x1 RGBA8 PNG (red, green), generated once via the
    /// `png` crate's own encoder in a throwaway script — not checked-in
    /// tooling, just a fixed byte constant.
    const TINY_PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 244, 34, 127, 138, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 249, 207, 192, 240,
        159, 17, 72, 0, 0, 16, 33, 3, 3, 30, 93, 32, 80, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
        130,
    ];

    #[test]
    fn resolve_returns_intrinsic_size_from_decoded_png() {
        let resolver = ImageResolver::new(StaticBytesProvider(TINY_PNG));
        let url = url::Url::parse("file:///fixture.png").unwrap();
        let result = resolver.resolve(ResolverRequest::new(&url)).unwrap();
        assert_eq!(
            (result.intrinsic.width, result.intrinsic.height),
            (2.0, 1.0)
        );
    }

    #[test]
    fn get_decoded_after_resolve_returns_cached_pixels() {
        use raikiri_traits::ImagePixelSource;
        let resolver = ImageResolver::new(StaticBytesProvider(TINY_PNG));
        let url = url::Url::parse("file:///fixture.png").unwrap();
        resolver.resolve(ResolverRequest::new(&url)).unwrap();
        let decoded = resolver
            .get_decoded(&url)
            .expect("should be cached after resolve");
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]); // red px, green px
    }

    /// Encodes an 8-bit PNG of `color` from `data` and hands it to a fresh
    /// resolver, returning that resolver plus the fixture URL. The bytes are
    /// produced by `png::Encoder` at test time (same approach as
    /// `resolve_rejects_16_bit_depth_png`) rather than hand-crafted, so the
    /// fixtures stay valid if the `png` crate's output details change.
    fn resolver_for_encoded(
        color: png::ColorType,
        width: u32,
        height: u32,
        data: &[u8],
        palette: Option<Vec<u8>>,
    ) -> (ImageResolver<StaticBytesProvider>, url::Url) {
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, width, height);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            if let Some(palette) = palette {
                encoder.set_palette(palette);
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(data).unwrap();
        }
        (
            ImageResolver::new(StaticBytesProvider(Box::leak(buf.into_boxed_slice()))),
            url::Url::parse("file:///fixture.png").unwrap(),
        )
    }

    /// `ColorType::Rgb` is the most common real-world PNG color type, and its
    /// arm does real 3→4 byte index arithmetic. Pin both the expansion and
    /// the opaque alpha it synthesizes.
    #[test]
    fn decodes_rgb_png_to_rgba8_with_opaque_alpha() {
        use raikiri_traits::ImagePixelSource;

        // 2x1: red, then blue.
        let (resolver, url) =
            resolver_for_encoded(png::ColorType::Rgb, 2, 1, &[255, 0, 0, 0, 0, 255], None);
        let resolved = resolver.resolve(ResolverRequest::new(&url)).unwrap();
        assert_eq!(
            (resolved.intrinsic.width, resolved.intrinsic.height),
            (2.0, 1.0)
        );

        let decoded = resolver.get_decoded(&url).expect("cached after resolve");
        assert_eq!(
            decoded.rgba,
            vec![255, 0, 0, 255, 0, 0, 255, 255],
            "each 3-byte RGB pixel must expand to RGBA with alpha=255"
        );
    }

    /// `ColorType::Grayscale` expands 1 byte to 4, replicating the gray value
    /// across R/G/B and synthesizing an opaque alpha.
    #[test]
    fn decodes_grayscale_png_to_rgba8_with_replicated_channels() {
        use raikiri_traits::ImagePixelSource;

        // 3x1: black, mid-gray, white.
        let (resolver, url) =
            resolver_for_encoded(png::ColorType::Grayscale, 3, 1, &[0, 128, 255], None);
        resolver.resolve(ResolverRequest::new(&url)).unwrap();

        let decoded = resolver.get_decoded(&url).expect("cached after resolve");
        assert_eq!(
            decoded.rgba,
            vec![0, 0, 0, 255, 128, 128, 128, 255, 255, 255, 255, 255],
            "gray value must land in R, G and B, with alpha=255"
        );
    }

    /// `ColorType::GrayscaleAlpha` expands 2 bytes to 4, replicating the gray
    /// value and carrying the source alpha through rather than forcing 255.
    #[test]
    fn decodes_grayscale_alpha_png_to_rgba8_preserving_alpha() {
        use raikiri_traits::ImagePixelSource;

        // 2x1: opaque mid-gray, then half-transparent white.
        let (resolver, url) = resolver_for_encoded(
            png::ColorType::GrayscaleAlpha,
            2,
            1,
            &[128, 255, 255, 128],
            None,
        );
        resolver.resolve(ResolverRequest::new(&url)).unwrap();

        let decoded = resolver.get_decoded(&url).expect("cached after resolve");
        assert_eq!(
            decoded.rgba,
            vec![128, 128, 128, 255, 255, 255, 255, 128],
            "source alpha must be carried through, not replaced by 255"
        );
    }

    /// An indexed/palette PNG must hit `decode_png`'s `other =>` rejection
    /// arm. Decoding it as if the index bytes were pixel data would produce a
    /// buffer of the wrong length and wholly wrong colors, so this must be a
    /// decode error rather than a silent mis-decode.
    #[test]
    fn resolve_rejects_indexed_palette_png() {
        // 2x1 image of palette entries 0 and 1 (red, blue).
        let (resolver, url) = resolver_for_encoded(
            png::ColorType::Indexed,
            2,
            1,
            &[0, 1],
            Some(vec![255, 0, 0, 0, 0, 255]),
        );

        let err = resolver.resolve(ResolverRequest::new(&url)).unwrap_err();
        let raikiri_traits::ResolverError::Decode(msg) = &err else {
            panic!("expected a Decode error for an indexed PNG, got {err:?}");
        };
        assert!(
            msg.contains("unsupported PNG color type"),
            "must be rejected by decode_png's color-type arm (not some unrelated \
             upstream failure); got {msg:?}"
        );
    }

    /// A 16-bit-per-channel RGBA PNG must be rejected as a decode error
    /// rather than silently treated as 8-bit (which would return a
    /// `DecodedImage::rgba` twice the length `width * height * 4` promises,
    /// per `raikiri_traits::DecodedImage`'s doc contract).
    #[test]
    fn resolve_rejects_16_bit_depth_png() {
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Sixteen);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[0, 255, 0, 255, 0, 255, 0, 255])
                .unwrap();
        }

        let resolver = ImageResolver::new(StaticBytesProvider(Box::leak(buf.into_boxed_slice())));
        let url = url::Url::parse("file:///sixteen-bit.png").unwrap();
        let err = resolver.resolve(ResolverRequest::new(&url)).unwrap_err();
        assert!(
            matches!(err, raikiri_traits::ResolverError::Decode(_)),
            "expected Decode error for unsupported bit depth, got {err:?}"
        );
    }
}
