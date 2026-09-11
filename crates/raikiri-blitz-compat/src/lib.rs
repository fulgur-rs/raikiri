#![allow(missing_docs)]
//! raikiri-blitz-compat — blitz-compatible type shape layer.
//!
//! Provides blitz-traits compatible type shapes implemented on top of
//! `raikiri-traits`. Does not depend on `blitz-traits` crate itself
//! (version-hell avoidance) — shapes are re-implemented verbatim
//! and bridged via conversions / adapters.

pub mod devtools;
pub mod events;
pub mod html;
pub mod navigation;
pub mod net;
pub mod shell;

/// `blitz_traits` namespace alias — mirrors `blitz_traits` crate path.
///
/// Allows `use raikiri_blitz_compat::blitz_traits::net::NetProvider`
/// and `use raikiri_blitz_compat::net::NetProvider` interchangeably.
pub mod blitz_traits {
    pub use crate::devtools;
    pub use crate::events;
    pub use crate::navigation;
    pub use crate::net;
    pub use crate::shell;
}

// ── raikiri-traits re-exports with blitz-compatible aliases ─────────────

/// DOM traits — re-exported from `raikiri_traits` with blitz-compatible naming.
pub mod dom {
    pub use raikiri_traits::Dom;
    pub use raikiri_traits::Element;
    pub use raikiri_traits::Node;
    pub use raikiri_traits::NodeId;
    pub use raikiri_traits::NodeKind;
    pub use raikiri_traits::QuirksMode;
    pub use raikiri_traits::StylesheetKind;
    pub use raikiri_traits::Symbol;
}

/// Replaced-element resolver — re-exported with blitz-compatible alias.
pub mod resolver {
    pub use raikiri_traits::IntrinsicBox;
    pub use raikiri_traits::ReplacedResolver;
    pub use raikiri_traits::ResolveDisposition;
    pub use raikiri_traits::ResolvedIntrinsic;
    pub use raikiri_traits::ResolverError;
    pub use raikiri_traits::ResolverRequest;
}

// Root-level type aliases for ergonomic `use raikiri_blitz_compat::NetProvider`.
pub use raikiri_traits::AbortController;
pub use raikiri_traits::AbortSignal;
pub use raikiri_traits::Body as RaikiriBody;
pub use raikiri_traits::Dom;
pub use raikiri_traits::Element;
pub use raikiri_traits::FetchedResource;
pub use raikiri_traits::HeaderMap as RaikiriHeaderMap;
pub use raikiri_traits::Method as RaikiriMethod;
pub use raikiri_traits::NetworkProvider as NetProvider;
pub use raikiri_traits::NetworkProvider;
pub use raikiri_traits::Node;
pub use raikiri_traits::NodeId;
pub use raikiri_traits::NodeKind;
pub use raikiri_traits::ReplacedResolver;
pub use raikiri_traits::Request as RaikiriRequest;

// Re-export raikiri umbrella for convenience.
pub use raikiri_traits;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn net_provider_shape_is_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn net::NetProvider>();
    }

    #[test]
    fn abort_signal_alias() {
        let c = raikiri_traits::AbortController::new();
        assert!(!c.signal.is_aborted());
        assert!(!c.signal.aborted());
        c.abort();
        assert!(c.signal.is_aborted());
        assert!(c.signal.aborted());
    }

    #[test]
    fn blitz_net_request_convert_roundtrip() {
        use net::Request;
        use url::Url;
        let url = Url::parse("https://example.com/page.html").unwrap();
        let req = Request::get(url.clone());
        assert_eq!(req.url, url);
        // Convert to raikiri and back via helper
        let raikiri_req = net::to_raikiri_request(req.clone());
        assert_eq!(raikiri_req.url, url);
        let back = net::from_raikiri_request(raikiri_req);
        assert_eq!(back.url, url);
    }

    #[test]
    fn adapter_wraps_raikiri_provider() {
        use bytes::Bytes;
        use net::{NetProvider, Request};
        use raikiri_traits::{FetchedResource, NetworkError, NetworkProvider};
        use url::Url;

        struct Dummy;
        impl NetworkProvider for Dummy {
            fn fetch(
                &self,
                _request: raikiri_traits::Request,
            ) -> Result<FetchedResource, NetworkError> {
                Ok(FetchedResource {
                    bytes: Bytes::from_static(b"hello"),
                    content_type: Some("text/html".to_string()),
                    final_url: Url::parse("https://example.com/").unwrap(),
                    encoding: None,
                })
            }
        }

        let provider = net::RaikiriNetProviderAdapter::new(Dummy);
        let req = Request::get(Url::parse("https://example.com/").unwrap());
        // Use sync adapter via blitz shape (calls raikiri under the hood, immediate callback)
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();
        struct Handler {
            called: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }
        impl net::NetHandler for Handler {
            fn bytes(self: Box<Self>, _resolved_url: String, bytes: Bytes) {
                assert_eq!(bytes, Bytes::from_static(b"hello"));
                self.called.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        provider.fetch(
            0,
            req,
            Box::new(Handler {
                called: called_clone,
            }),
        );
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }
}
