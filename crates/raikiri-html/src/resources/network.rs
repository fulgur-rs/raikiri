//! Policy-checked network access for stylesheets, images and fonts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use raikiri_dom::FontFaceLoader;
use raikiri_traits::{
    FetchOutcome, NetworkError, NetworkProvider, PolicyViolation, RenderWarning, Request,
    ResourceKind, ResourcePolicy, ViolationType, WarningKind,
};
use url::Url;

use crate::fetch::{
    get_request, mime_essence, min_limit, policy_violation_warning_kind, redacted_url, resolve_url,
    url_policy_violation,
};

use super::{ResourceLimits, SharedRenderWarnings, push_resource_warning};

/// Network provider adapter shared by stylesheet imports and font loads.
pub(crate) struct ResourceNetworkProvider<'a> {
    pub(super) inner: &'a dyn NetworkProvider,
    pub(super) policy: Option<&'a dyn ResourcePolicy>,
    pub(super) limits: ResourceLimits,
    pub(super) budget: Arc<Mutex<u64>>,
}

impl ResourceNetworkProvider<'_> {
    /// Builds a `PolicyViolation` naming `url` — the URL the check that
    /// failed actually inspected, which after a redirect hop is not
    /// necessarily `request.url` — rather than always reporting the
    /// original request URL regardless of which URL was actually denied.
    fn violation(
        request: &Request,
        url: &Url,
        violation_type: ViolationType,
        details: impl Into<String>,
    ) -> NetworkError {
        NetworkError::PolicyViolation(Box::new(PolicyViolation {
            kind: request.kind,
            url: url.clone(),
            violation_type,
            details: details.into(),
        }))
    }

    fn check_url_policy(&self, request: &Request, url: &Url) -> Result<(), NetworkError> {
        let Some(policy) = self.policy else {
            return Ok(());
        };
        match url_policy_violation(policy, url, request.kind) {
            None => Ok(()),
            Some(violation_type) => {
                let details = match violation_type {
                    ViolationType::SchemeNotAllowed => "resource URL scheme is denied by policy",
                    _ => "resource URL host is denied by policy",
                };
                Err(Self::violation(request, url, violation_type, details))
            }
        }
    }

    fn byte_limit(&self, kind: ResourceKind) -> Option<u64> {
        min_limit(
            self.limits.max_resource_bytes,
            self.policy.and_then(|policy| policy.max_fetch_bytes(kind)),
        )
    }
}

impl NetworkProvider for ResourceNetworkProvider<'_> {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.check_url_policy(&request, &request.url)?;

        // Drives `inner.fetch_one_hop` itself rather than delegating to
        // `inner.fetch()`, so every hop's target is checked against policy
        // *before* any request reaches it — not only the initial URL and
        // the final response's URL after the fact, by which point a
        // provider that follows redirects internally (e.g. `ureq`'s
        // default behavior) would have already sent every intermediate
        // request. `hop` here is the real count of redirects actually
        // taken, unlike a fixed `1` passed to every `allow_redirect` call.
        let mut current = request.clone();
        let mut hop = 0u32;
        let fetched = loop {
            match self.inner.fetch_one_hop(current.clone())? {
                FetchOutcome::Body(fetched) => break fetched,
                FetchOutcome::Redirect { location, .. } => {
                    if let Some(policy) = self.policy
                        && (hop >= policy.max_redirect_hops(request.kind)
                            || !policy.allow_redirect(&current.url, &location, hop + 1))
                    {
                        return Err(Self::violation(
                            &request,
                            &location,
                            ViolationType::RedirectDenied,
                            "resource redirect is denied by policy",
                        ));
                    }
                    // This adapter consumes redirects internally, so the
                    // outer NetworkProvider::fetch loop cannot enforce its cap.
                    if self.policy.is_none() && hop >= raikiri_traits::MAX_AUTO_REDIRECT_HOPS {
                        return Err(NetworkError::Other("too many redirects".to_owned()));
                    }
                    hop += 1;
                    self.check_url_policy(&request, &location)?;
                    current.url = location;
                }
                // cov:ignore: `FetchOutcome` is `#[non_exhaustive]`; this
                // crate cannot construct a third variant to exercise this
                // arm from a test, only `Body`/`Redirect` exist today.
                _ => {
                    return Err(NetworkError::Other(
                        "unknown FetchOutcome variant".to_owned(),
                    ));
                }
            }
        };

        let actual = fetched.bytes.len() as u64;
        if let Some(limit) = self.byte_limit(request.kind)
            && actual > limit
        {
            return Err(Self::violation(
                &request,
                &request.url,
                ViolationType::FetchTooLarge { limit, actual },
                "resource response exceeded its byte limit",
            ));
        }

        if let Some(policy) = self.policy {
            let allowed = policy.allowed_mime_types(request.kind);
            if !allowed.is_empty() {
                let content_type = fetched.content_type.as_deref().ok_or_else(|| {
                    Self::violation(
                        &request,
                        &request.url,
                        ViolationType::MimeNotAllowed {
                            mime: "<missing>".to_owned(),
                        },
                        "resource response did not include a required MIME type",
                    )
                })?;
                let mime = mime_essence(content_type);
                if !allowed
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(mime))
                {
                    return Err(Self::violation(
                        &request,
                        &request.url,
                        ViolationType::MimeNotAllowed {
                            mime: mime.to_owned(),
                        },
                        "resource MIME type is denied by policy",
                    ));
                }
            }
        }

        let mut used = self
            .budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let total = used.saturating_add(actual);
        if let Some(limit) = self.limits.max_aggregate_resource_bytes
            && total > limit
        {
            return Err(Self::violation(
                &request,
                &request.url,
                ViolationType::FetchTooLarge {
                    limit,
                    actual: total,
                },
                "aggregate resource responses exceeded their byte limit",
            ));
        }
        *used = total;
        Ok(FetchOutcome::Body(fetched))
    }

    fn max_import_depth(&self) -> Option<u32> {
        min_limit(
            self.policy.map(ResourcePolicy::max_import_depth),
            self.inner.max_import_depth(),
        )
    }
}

/// Adapter that routes font-face URLs through the same policy-checked provider
/// and aggregate byte budget used by stylesheet parsing.
pub(crate) struct NetworkFontFaceLoader<'a> {
    network: Option<&'a dyn NetworkProvider>,
    base_url: Option<&'a Url>,
    warnings: SharedRenderWarnings,
    /// The outcome of every source loaded so far, failures included. One
    /// layout reads each `@font-face` source several times (once per
    /// cascade, and once more for the inline engine's font layer); without
    /// this every read would fetch again, spend the aggregate budget again,
    /// and repeat its warning.
    loaded: Mutex<HashMap<Url, Option<Vec<u8>>>>,
}

impl<'a> NetworkFontFaceLoader<'a> {
    pub(crate) fn new(
        network: Option<&'a dyn NetworkProvider>,
        base_url: Option<&'a Url>,
        warnings: SharedRenderWarnings,
    ) -> Self {
        Self {
            network,
            base_url,
            warnings,
            loaded: Mutex::new(HashMap::new()),
        }
    }
}

impl FontFaceLoader for NetworkFontFaceLoader<'_> {
    fn load(&self, source: &str) -> Option<Vec<u8>> {
        let Some(url) = resolve_url(source, self.base_url) else {
            push_resource_warning(
                &self.warnings,
                RenderWarning {
                    kind: WarningKind::ResourceFallback {
                        kind: ResourceKind::Font,
                        url: None,
                    },
                    node_id: None,
                    details: "font-face source could not be resolved against the document base"
                        .into(),
                },
            );
            return None;
        };
        let mut loaded = self
            .loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(outcome) = loaded.get(&url) {
            return outcome.clone();
        }
        let outcome = self.fetch_uncached(&url);
        loaded.insert(url, outcome.clone());
        outcome
    }
}

impl NetworkFontFaceLoader<'_> {
    fn fetch_uncached(&self, url: &Url) -> Option<Vec<u8>> {
        let Some(network) = self.network else {
            push_resource_warning(
                &self.warnings,
                RenderWarning {
                    kind: WarningKind::ResourceFallback {
                        kind: ResourceKind::Font,
                        url: Some(redacted_url(url)),
                    },
                    node_id: None,
                    details: "font-face source skipped because no network provider is configured"
                        .into(),
                },
            );
            return None;
        };

        match network.fetch(get_request(url.clone(), ResourceKind::Font, None)) {
            Ok(fetched) => Some(fetched.bytes.to_vec()),
            Err(NetworkError::PolicyViolation(violation)) => {
                push_resource_warning(
                    &self.warnings,
                    RenderWarning {
                        kind: policy_violation_warning_kind(&violation, ResourceKind::Font),
                        node_id: None,
                        details: "font-face source was denied by the resource policy".into(),
                    },
                );
                None
            }
            Err(_) => {
                push_resource_warning(
                    &self.warnings,
                    RenderWarning {
                        kind: WarningKind::NetworkFallback {
                            url: redacted_url(url),
                        },
                        node_id: None,
                        details: "font-face fetch failed; the family will use fallback fonts"
                            .into(),
                    },
                );
                None
            }
        }
    }
}
