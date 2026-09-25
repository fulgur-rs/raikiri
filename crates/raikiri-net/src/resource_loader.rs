//! Resource loading primitives used by image consumers.
//!
//! URL-scheme handling and `data:` decoding belong in this resource layer,
//! before bytes reach a format decoder. This keeps `ImageResolver` as a thin
//! replaced-element adapter and leaves the decoder independent of URLs and
//! providers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use raikiri_traits::{
    Body, DecodedImage, FetchedResource, Method, NetworkProvider, Request, ResolverError,
    policy::ResourceKind,
};
use url::Url;

use crate::image_decoder::ImageDecoder;

/// Loads and caches decoded image resources.
///
/// `ResourceLoader` owns provider dispatch, `data:` URL decoding, request
/// construction, and the decoded-image cache. [`ImageDecoder`] remains a pure
/// bytes-to-pixels component.
pub(crate) struct ResourceLoader<N> {
    network: N,
    decoder: ImageDecoder,
    cache: Mutex<HashMap<Url, Arc<DecodedImage>>>,
}

impl<N: NetworkProvider> ResourceLoader<N> {
    pub(crate) fn new(network: N) -> Self {
        Self {
            network,
            decoder: ImageDecoder,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Fetches, decodes, and caches one image resource.
    #[allow(clippy::result_large_err)]
    pub(crate) fn load(&self, url: &Url) -> Result<Arc<DecodedImage>, ResolverError> {
        if let Some(cached) = self.cache.lock().unwrap().get(url) {
            return Ok(cached.clone());
        }

        let (bytes, is_data_url) = if url.scheme() == "data" {
            (decode_data_url(url.as_str())?, true)
        } else {
            (self.fetch(url)?.bytes.to_vec(), false)
        };
        let decoded = self.decoder.decode(&bytes).map_err(|error| {
            if is_data_url {
                ResolverError::Decode(format!("data URL image decode failed: {error}"))
            } else {
                ResolverError::Decode(error)
            }
        })?;
        let decoded = Arc::new(decoded);
        self.cache
            .lock()
            .unwrap()
            .insert(url.clone(), decoded.clone());
        Ok(decoded)
    }

    pub(crate) fn cached(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        self.cache.lock().unwrap().get(url).cloned()
    }

    #[allow(clippy::result_large_err)]
    fn fetch(&self, url: &Url) -> Result<FetchedResource, ResolverError> {
        self.network
            .fetch(Request {
                url: url.clone(),
                method: Method::Get,
                content_type: None,
                headers: Vec::new(),
                body: Body::Empty,
                signal: None,
                kind: ResourceKind::Image,
            })
            .map_err(ResolverError::Network)
    }
}

/// Decodes the body of a `data:` URL in the resource layer.
#[allow(clippy::result_large_err)]
fn decode_data_url(url: &str) -> Result<Vec<u8>, ResolverError> {
    let data_url = data_url::DataUrl::process(url)
        .map_err(|error| ResolverError::Decode(format!("invalid data URL: {error}")))?;
    let (bytes, _fragment) = data_url
        .decode_to_vec()
        .map_err(|error| ResolverError::Decode(format!("invalid data URL payload: {error}")))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raikiri_traits::{FetchOutcome, NetworkError, NetworkProvider};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RejectingProvider;

    impl NetworkProvider for RejectingProvider {
        fn fetch_one_hop(&self, _request: Request) -> Result<FetchOutcome, NetworkError> {
            Err(NetworkError::Other("provider was called".into()))
        }
    }

    #[test]
    fn base64_data_url_is_decoded_without_calling_provider() {
        let resolver = ResourceLoader::new(RejectingProvider);
        let url = Url::parse(concat!(
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEElEQVR4nGP5z8Dw",
            "nxFIAAAQIQMDHl0gUAAAAABJRU5ErkJggg==",
        ))
        .unwrap();

        let decoded = resolver.load(&url).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
    }

    /// A hand-encoded 2x1 RGBA8 PNG (red, green).
    const TINY_PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 244, 34, 127, 138, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 249, 207, 192, 240,
        159, 17, 72, 0, 0, 16, 33, 3, 3, 30, 93, 32, 80, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
        130,
    ];

    #[test]
    fn percent_encoded_data_url_is_decoded_without_calling_provider() {
        let encoded = TINY_PNG
            .iter()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        let resolver = ResourceLoader::new(RejectingProvider);
        let url = Url::parse(&format!("data:image/png,{encoded}")).unwrap();

        let decoded = resolver.load(&url).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
    }

    #[test]
    fn malformed_data_url_is_a_decode_error() {
        let resolver = ResourceLoader::new(RejectingProvider);
        let url = Url::parse("data:image/png;base64,not-valid").unwrap();
        let error = resolver.load(&url).unwrap_err();

        assert!(matches!(
            error,
            ResolverError::Decode(message)
                if message.contains("invalid data URL payload")
        ));
    }

    #[test]
    fn invalid_data_url_image_bytes_are_decode_errors() {
        let resolver = ResourceLoader::new(RejectingProvider);
        let url = Url::parse("data:image/png,not-an-image").unwrap();
        let error = resolver.load(&url).unwrap_err();

        assert!(matches!(
            error,
            ResolverError::Decode(message)
                if message.contains("data URL image decode failed")
        ));
    }

    struct RecordingProvider {
        fetch_count: AtomicUsize,
        bytes: &'static [u8],
    }

    impl NetworkProvider for RecordingProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            assert_eq!(request.method, Method::Get);
            assert!(matches!(request.kind, ResourceKind::Image));
            self.fetch_count.fetch_add(1, Ordering::Relaxed);
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: self.bytes.to_vec().into(),
                content_type: Some("image/png".into()),
                final_url: request.url,
                encoding: None,
            }))
        }
    }

    #[test]
    fn fetched_invalid_image_bytes_are_decode_errors() {
        let resolver = ResourceLoader::new(RecordingProvider {
            fetch_count: AtomicUsize::new(0),
            bytes: b"not an image",
        });
        let url = Url::parse("file:///fixture.png").unwrap();
        let error = resolver.load(&url).unwrap_err();

        assert!(matches!(error, ResolverError::Decode(message) if !message.is_empty()));
    }

    #[test]
    fn resource_loader_caches_successful_decodes() {
        let provider = RecordingProvider {
            fetch_count: AtomicUsize::new(0),
            bytes: TINY_PNG,
        };
        let resolver = ResourceLoader::new(provider);
        let url = Url::parse("file:///fixture.png").unwrap();

        let first = resolver.load(&url).unwrap();
        let second = resolver.load(&url).unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.width, 2);
        assert_eq!(first.height, 1);
        assert_eq!(resolver.network.fetch_count.load(Ordering::Relaxed), 1);
        assert!(resolver.cached(&url).is_some());
    }

    #[test]
    fn provider_errors_remain_network_errors() {
        let resolver = ResourceLoader::new(RejectingProvider);
        let url = Url::parse("file:///fixture.png").unwrap();
        let error = resolver.load(&url).unwrap_err();
        assert!(matches!(
            error,
            ResolverError::Network(NetworkError::Other(_))
        ));
    }
}
