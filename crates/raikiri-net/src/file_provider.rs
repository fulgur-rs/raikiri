//! `FileNetworkProvider` — reads local files for `file://` URLs.
//!
//! Verification-only [`NetworkProvider`] used to exercise the fetch → decode
//! → layout → paint pipeline end to end without a real HTTP client. A real
//! HTTP client is the Consumer's responsibility (see
//! `raikiri_traits::NetworkProvider`'s doc comment) — this type only reads
//! bytes off the local filesystem for `file://` URLs.

use raikiri_traits::{FetchOutcome, FetchedResource, NetworkError, NetworkProvider, Request};

/// Reads local files for `file://` URLs. Any other scheme is rejected.
#[derive(Debug, Default)]
pub struct FileNetworkProvider;

impl NetworkProvider for FileNetworkProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        if request.url.scheme() != "file" {
            return Err(NetworkError::Other(format!(
                "FileNetworkProvider only supports file:// URLs, got: {}",
                request.url
            )));
        }
        let path = request
            .url
            .to_file_path()
            .map_err(|_| NetworkError::Other(format!("invalid file:// URL: {}", request.url)))?;
        let bytes = std::fs::read(&path).map_err(NetworkError::Io)?;
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: bytes.into(),
            content_type: Some("image/png".to_string()),
            final_url: request.url,
            encoding: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raikiri_traits::{Body, Method};
    use std::io::Write;

    #[test]
    fn reads_bytes_from_a_local_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"hello").unwrap();
        let url = url::Url::from_file_path(tmp.path()).unwrap();

        let provider = FileNetworkProvider;
        let result = provider
            .fetch(Request {
                url: url.clone(),
                method: Method::Get,
                content_type: None,
                headers: Vec::new(),
                body: Body::Empty,
                signal: None,
                kind: raikiri_traits::ResourceKind::Image,
            })
            .expect("fetch should succeed");

        assert_eq!(&result.bytes[..], b"hello");
        assert_eq!(result.final_url, url);
    }

    #[test]
    fn rejects_non_file_scheme() {
        let provider = FileNetworkProvider;
        let url = url::Url::parse("https://example.com/x.png").unwrap();
        let err = provider
            .fetch(Request {
                url,
                method: Method::Get,
                content_type: None,
                headers: Vec::new(),
                body: Body::Empty,
                signal: None,
                kind: raikiri_traits::ResourceKind::Image,
            })
            .unwrap_err();
        assert!(matches!(err, NetworkError::Other(_)));
    }
}
