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
/// (normalizing all to RGBA8); anything else (16-bit depth, indexed/palette,
/// interlaced-unsupported-by-this-normalization) is a decode error. This
/// MVP only needs to decode images this same pipeline or common tools
/// produce, not the full PNG format matrix.
fn decode_png(bytes: &[u8]) -> Result<DecodedImage, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let buf_size = reader
        .output_buffer_size()
        .ok_or_else(|| "PNG output buffer size overflows address space".to_string())?;
    let mut buf = vec![0u8; buf_size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let raw = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
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
}
