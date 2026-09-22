//! Renderer-neutral resource configuration shared by parse and render entry points.

use std::fmt;
use std::io::Read;
use std::sync::{Arc, Mutex};

use raikiri_dom::FontFaceLoader;
use raikiri_html::ParseOptions;
use raikiri_traits::{
    Body, DecodedImage, FetchedResource, Method, NetworkError, NetworkProvider, PolicyViolation,
    RenderLimits, RenderWarning, Request, ResourceKind, ResourcePolicy, ViolationType, WarningKind,
};
use url::Url;

use crate::{FontContext, HtmlDocument, ImagePixelSource, ReplacedResolver};

/// Default maximum response size accepted through [`RenderResources`].
pub const DEFAULT_MAX_RESOURCE_BYTES: u64 = 32 * 1024 * 1024;
/// Default maximum combined response bytes accepted through [`RenderResources`].
pub const DEFAULT_MAX_AGGREGATE_RESOURCE_BYTES: u64 = 128 * 1024 * 1024;

/// Network response byte limits for one render operation.
///
/// These limits apply to responses after the consumer's [`NetworkProvider`]
/// returns. The provider must enforce transfer-time limits itself if it needs
/// to prevent buffering an oversized response before it reaches Raikiri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceLimits {
    /// Maximum response bytes for one request. Defaults to 32 MiB.
    pub max_resource_bytes: Option<u64>,
    /// Maximum response bytes across stylesheet and font requests. Defaults to 128 MiB.
    pub max_aggregate_resource_bytes: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_resource_bytes: Some(DEFAULT_MAX_RESOURCE_BYTES),
            max_aggregate_resource_bytes: Some(DEFAULT_MAX_AGGREGATE_RESOURCE_BYTES),
        }
    }
}

impl ResourceLimits {
    /// Create limits with the fail-closed defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum response size. `None` disables this cap.
    pub fn max_resource_bytes(mut self, limit: Option<u64>) -> Self {
        self.max_resource_bytes = limit;
        self
    }

    /// Set the total response-byte budget. `None` disables this cap.
    pub fn max_aggregate_resource_bytes(mut self, limit: Option<u64>) -> Self {
        self.max_aggregate_resource_bytes = limit;
        self
    }
}

/// Consumer-supplied inputs and resource policy shared by parse and render.
///
/// This type is in the `raikiri` facade so consumers do not need to depend on
/// `raikiri-html`, `raikiri-dom`, or `raikiri-traits` implementation crates.
/// Use the same value with [`crate::parse_html_with_resources`] and
/// [`crate::render_streaming_with_resources`] so both phases share stylesheet
/// sources, base URL, network provider, policy, fonts, resolver, and limits.
///
/// `FontContext::new()` is retained as the default for compatibility with the
/// existing consumer behavior. For deterministic rendering, supply a context
/// built with [`crate::FontContextBuilder`], which disables system font
/// discovery and applies bundled fonts in a stable fallback order.
#[derive(Clone)]
pub struct RenderResources<'a> {
    extra_stylesheets: Vec<String>,
    network: Option<&'a dyn NetworkProvider>,
    policy: Option<&'a dyn ResourcePolicy>,
    base_url: Option<Url>,
    font_context: FontContext,
    resolver: Option<&'a dyn ReplacedResolver>,
    image_pixel_source: Option<&'a dyn ImagePixelSource>,
    render_limits: RenderLimits,
    resource_limits: ResourceLimits,
    budget: Arc<Mutex<u64>>,
}

impl fmt::Debug for RenderResources<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderResources")
            .field("extra_stylesheet_count", &self.extra_stylesheets.len())
            .field("has_network_provider", &self.network.is_some())
            .field("has_network_policy", &self.policy.is_some())
            .field("base_url", &self.base_url)
            .field("has_font_context", &true)
            .field("has_replaced_resolver", &self.resolver.is_some())
            .field("has_image_pixel_source", &self.image_pixel_source.is_some())
            .field("render_limits", &self.render_limits)
            .field("resource_limits", &self.resource_limits)
            .finish()
    }
}

impl<'a> Default for RenderResources<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> RenderResources<'a> {
    /// Create a resource configuration with no extra stylesheets or network
    /// provider, default render/resource limits, and the platform font context.
    pub fn new() -> Self {
        Self {
            extra_stylesheets: Vec::new(),
            network: None,
            policy: None,
            base_url: None,
            font_context: FontContext::new(),
            resolver: None,
            image_pixel_source: None,
            render_limits: RenderLimits::default(),
            resource_limits: ResourceLimits::default(),
            budget: Arc::new(Mutex::new(0)),
        }
    }

    /// Append a stylesheet source as a user stylesheet.
    pub fn stylesheet(mut self, source: impl Into<String>) -> Self {
        self.extra_stylesheets.push(source.into());
        self
    }

    /// Provide the consumer's synchronous network provider.
    pub fn network_provider(mut self, provider: &'a dyn NetworkProvider) -> Self {
        self.network = Some(provider);
        self
    }

    /// Apply this consumer policy to requests made through this configuration.
    ///
    /// Scheme, host, final-URL redirect, MIME, per-resource-size, and
    /// aggregate-size checks are applied around stylesheet and font provider
    /// calls. The same scheme/host policy is checked before replaced-resource
    /// resolution, and `max_decoded_bytes(Image)` is checked before paint pixels
    /// are returned. Since [`NetworkProvider::fetch`] returns only the final
    /// URL, a provider that follows multiple redirects must enforce each hop and
    /// the hop count while fetching. The provider and any custom replaced-
    /// resource resolver remain responsible for transfer-time byte limits and
    /// fetch/decode timeouts because they materialize data before returning.
    pub fn network_policy(mut self, policy: &'a dyn ResourcePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Set the fallback base URL for document-relative resources.
    ///
    /// A valid HTML `<base href>` takes precedence, consistently for stylesheets,
    /// `@font-face`, and replaced elements.
    pub fn base_url(mut self, base_url: Url) -> Self {
        self.base_url = Some(base_url);
        self
    }

    /// Use a caller-built font context for layout and paint.
    pub fn font_context(mut self, font_context: FontContext) -> Self {
        self.font_context = font_context;
        self
    }

    /// Set the limits used when parsing HTML.
    pub fn render_limits(mut self, limits: RenderLimits) -> Self {
        self.render_limits = limits;
        self
    }

    /// Set response-size limits for stylesheet and font requests.
    pub fn resource_limits(mut self, limits: ResourceLimits) -> Self {
        self.resource_limits = limits;
        self
    }

    /// Use one cache/provider object for replaced-element sizing and paint pixels.
    ///
    /// The same object is retained as both the layout resolver and the
    /// paint-time pixel source. This matches the intended cache lifetime across
    /// layout and downstream page-fragment painting.
    pub fn replaced_resource_provider<R>(mut self, provider: &'a R) -> Self
    where
        R: ReplacedResolver + ImagePixelSource + 'a,
    {
        self.resolver = Some(provider);
        self.image_pixel_source = Some(provider);
        self
    }

    /// Set the intrinsic-size resolver when paint pixels are supplied elsewhere.
    pub fn replaced_resolver<R>(mut self, resolver: &'a R) -> Self
    where
        R: ReplacedResolver + 'a,
    {
        self.resolver = Some(resolver);
        self
    }

    /// Set a separate paint-time image source.
    pub fn image_pixel_source<I>(mut self, source: &'a I) -> Self
    where
        I: ImagePixelSource + 'a,
    {
        self.image_pixel_source = Some(source);
        self
    }

    /// The prepared font context used as the baseline for layout passes.
    pub fn font_context_ref(&self) -> &FontContext {
        &self.font_context
    }

    /// The same image source configured for downstream painting, if any.
    pub fn image_pixel_source_ref(&self) -> Option<&dyn ImagePixelSource> {
        self.image_pixel_source
            .map(|_| self as &dyn ImagePixelSource)
    }

    pub(crate) fn extra_stylesheets(&self) -> Vec<&str> {
        self.extra_stylesheets.iter().map(String::as_str).collect()
    }

    pub(crate) fn parse_options<'b>(
        &'b self,
        extra_stylesheets: &'b [&'b str],
        network: Option<&'b dyn NetworkProvider>,
    ) -> ParseOptions<'b> {
        ParseOptions {
            extra_stylesheets,
            network,
            base_url: self.base_url.clone(),
        }
    }

    pub(crate) fn parse_limits(&self) -> RenderLimits {
        self.render_limits.clone()
    }

    pub(crate) fn fallback_base_url(&self) -> Option<&Url> {
        self.base_url.as_ref()
    }

    pub(crate) fn clone_font_context(&self) -> FontContext {
        self.font_context.clone()
    }

    pub(crate) fn policy(&self) -> Option<&dyn ResourcePolicy> {
        self.policy
    }

    pub(crate) fn resolver(&self) -> Option<&dyn ReplacedResolver> {
        self.resolver
    }

    pub(crate) fn raw_image_pixel_source(&self) -> Option<&dyn ImagePixelSource> {
        self.image_pixel_source
    }

    pub(crate) fn network_adapter(&self) -> Option<ResourceNetworkProvider<'_>> {
        self.network.map(|inner| ResourceNetworkProvider {
            inner,
            policy: self.policy,
            limits: self.resource_limits,
            budget: Arc::clone(&self.budget),
        })
    }
}

impl ImagePixelSource for RenderResources<'_> {
    fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        if let Some(policy) = self.policy
            && (!policy.is_scheme_allowed(url.scheme(), ResourceKind::Image)
                || !policy.is_host_allowed(url.host_str().unwrap_or(""), ResourceKind::Image))
        {
            return None;
        }
        let image = self.image_pixel_source?.get_decoded(url)?;
        if let Some(limit) = self
            .policy
            .and_then(|policy| policy.max_decoded_bytes(ResourceKind::Image))
            && image.rgba.len() as u64 > limit
        {
            return None;
        }
        Some(image)
    }
}

/// Network provider adapter shared by stylesheet imports and font loads.
pub(crate) struct ResourceNetworkProvider<'a> {
    inner: &'a dyn NetworkProvider,
    policy: Option<&'a dyn ResourcePolicy>,
    limits: ResourceLimits,
    budget: Arc<Mutex<u64>>,
}

impl ResourceNetworkProvider<'_> {
    fn violation(
        request: &Request,
        violation_type: ViolationType,
        details: impl Into<String>,
    ) -> NetworkError {
        NetworkError::PolicyViolation(PolicyViolation {
            kind: request.kind,
            url: request.url.clone(),
            violation_type,
            details: details.into(),
        })
    }

    #[allow(clippy::result_large_err)]
    fn check_url_policy(&self, request: &Request, url: &Url) -> Result<(), NetworkError> {
        let Some(policy) = self.policy else {
            return Ok(());
        };
        if !policy.is_scheme_allowed(url.scheme(), request.kind) {
            return Err(Self::violation(
                request,
                ViolationType::SchemeNotAllowed,
                "resource URL scheme is denied by policy",
            ));
        }
        let host = url.host_str().unwrap_or("");
        if !policy.is_host_allowed(host, request.kind) {
            return Err(Self::violation(
                request,
                ViolationType::HostNotAllowed,
                "resource URL host is denied by policy",
            ));
        }
        Ok(())
    }

    fn byte_limit(&self, kind: ResourceKind) -> Option<u64> {
        let policy_limit = self.policy.and_then(|policy| policy.max_fetch_bytes(kind));
        match (self.limits.max_resource_bytes, policy_limit) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(limit), None) | (None, Some(limit)) => Some(limit),
            (None, None) => None,
        }
    }
}

impl NetworkProvider for ResourceNetworkProvider<'_> {
    fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
        self.check_url_policy(&request, &request.url)?;
        let fetched = self.inner.fetch(request.clone())?;

        if fetched.final_url != request.url {
            self.check_url_policy(&request, &fetched.final_url)?;
            if let Some(policy) = self.policy
                && (policy.max_redirect_hops(request.kind) == 0
                    || !policy.allow_redirect(&request.url, &fetched.final_url, 1))
            {
                return Err(Self::violation(
                    &request,
                    ViolationType::RedirectDenied,
                    "resource redirect is denied by policy",
                ));
            }
        }

        let actual = fetched.bytes.len() as u64;
        if let Some(limit) = self.byte_limit(request.kind)
            && actual > limit
        {
            return Err(Self::violation(
                &request,
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
                        ViolationType::MimeNotAllowed {
                            mime: "<missing>".to_owned(),
                        },
                        "resource response did not include a required MIME type",
                    )
                })?;
                let mime = content_type
                    .split(';')
                    .next()
                    .unwrap_or(content_type)
                    .trim();
                if !allowed
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(mime))
                {
                    return Err(Self::violation(
                        &request,
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
                ViolationType::FetchTooLarge {
                    limit,
                    actual: total,
                },
                "aggregate resource responses exceeded their byte limit",
            ));
        }
        *used = total;
        Ok(fetched)
    }

    fn max_import_depth(&self) -> Option<u32> {
        match (
            self.policy.map(ResourcePolicy::max_import_depth),
            self.inner.max_import_depth(),
        ) {
            (Some(policy), Some(provider)) => Some(policy.min(provider)),
            (Some(limit), None) | (None, Some(limit)) => Some(limit),
            (None, None) => None,
        }
    }
}

/// Build a GET request for one resolved resource URL.
pub(crate) fn get_request(url: Url, kind: ResourceKind) -> Request {
    Request {
        url,
        method: Method::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind,
    }
}

/// Warnings collected while resolving CSS `@font-face` sources.
pub(crate) type SharedRenderWarnings = Arc<Mutex<Vec<RenderWarning>>>;

pub(crate) fn push_resource_warning(warnings: &SharedRenderWarnings, warning: RenderWarning) {
    warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(warning);
}

pub(crate) fn redacted_url(url: &Url) -> Url {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted
}

pub(crate) fn sanitize_policy_violation(mut violation: PolicyViolation) -> PolicyViolation {
    violation.url = redacted_url(&violation.url);
    violation.details = "network policy denied the request".to_owned();
    violation
}

/// Adapter that routes font-face URLs through the same policy-checked provider
/// and aggregate byte budget used by stylesheet parsing.
pub(crate) struct NetworkFontFaceLoader<'a> {
    network: Option<&'a dyn NetworkProvider>,
    base_url: Option<&'a Url>,
    warnings: SharedRenderWarnings,
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
        }
    }
}

impl FontFaceLoader for NetworkFontFaceLoader<'_> {
    fn load(&self, source: &str) -> Option<Vec<u8>> {
        let url = Url::parse(source)
            .ok()
            .or_else(|| self.base_url.and_then(|base| base.join(source).ok()));
        let Some(url) = url else {
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
        let Some(network) = self.network else {
            push_resource_warning(
                &self.warnings,
                RenderWarning {
                    kind: WarningKind::ResourceFallback {
                        kind: ResourceKind::Font,
                        url: Some(redacted_url(&url)),
                    },
                    node_id: None,
                    details: "font-face source skipped because no network provider is configured"
                        .into(),
                },
            );
            return None;
        };

        match network.fetch(get_request(url.clone(), ResourceKind::Font)) {
            Ok(fetched) => Some(fetched.bytes.to_vec()),
            Err(NetworkError::PolicyViolation(violation)) => {
                let kind = match &violation.violation_type {
                    ViolationType::FetchTooLarge { limit, actual }
                    | ViolationType::DecodedTooLarge { limit, actual } => {
                        WarningKind::ResourceLimitExceeded {
                            kind: ResourceKind::Font,
                            limit: *limit,
                            actual: *actual,
                        }
                    }
                    _ => WarningKind::PolicyWarning {
                        violation: sanitize_policy_violation(violation.clone()),
                    },
                };
                push_resource_warning(
                    &self.warnings,
                    RenderWarning {
                        kind,
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
                            url: redacted_url(&url),
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

/// Parse HTML using the same renderer-neutral resource configuration later
/// accepted by the resource-aware render entry points.
#[allow(clippy::result_large_err)]
pub fn parse_html_with_resources<R: Read>(
    input: R,
    resources: &RenderResources<'_>,
) -> Result<HtmlDocument, crate::RenderError> {
    let network = resources.network_adapter();
    let network_ref = network
        .as_ref()
        .map(|provider| provider as &dyn NetworkProvider);
    let extra_stylesheets = resources.extra_stylesheets();
    let options = resources.parse_options(&extra_stylesheets, network_ref);
    crate::parse::parse_html_with_limits(input, &options, resources.parse_limits())
}
