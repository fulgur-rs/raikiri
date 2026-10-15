//! URL, request, and diagnostic helpers shared by every resource fetch path
//! (stylesheets, `@import`, images, fonts, and replaced resources).

use raikiri_traits::{
    AbortSignal, Body, Method, NetworkError, PolicyViolation, Request, ResourceKind,
    ResourcePolicy, ViolationType, WarningKind,
};
use url::Url;

/// Resolve `href` against `base`. Absolute URLs do not need a base; relative
/// URLs are rejected when no base is available.
pub(crate) fn resolve_url(href: &str, base: Option<&Url>) -> Option<Url> {
    match base {
        Some(base) => base.join(href).ok(),
        None => Url::parse(href).ok(),
    }
}

/// `url` without its fragment, the identity used to deduplicate fetches.
pub(crate) fn url_without_fragment(url: &Url) -> Url {
    let mut normalized = url.clone();
    normalized.set_fragment(None);
    normalized
}

/// The MIME type essence of a `Content-Type` value: everything before the
/// first `;`, with surrounding whitespace removed. Callers compare it ASCII
/// case-insensitively.
pub(crate) fn mime_essence(content_type: &str) -> &str {
    content_type
        .split_once(';')
        .map_or(content_type, |(essence, _)| essence)
        .trim()
}

/// Build a GET request for one resolved resource URL.
///
/// The caller passes its render abort signal through so a provider that honors
/// [`AbortSignal`] can fail fast instead of
/// buffering a response nobody will use. Providers only check the signal up
/// front; a signal that fires mid-fetch does not interrupt bytes already in
/// flight.
pub(crate) fn get_request(url: Url, kind: ResourceKind, signal: Option<&AbortSignal>) -> Request {
    Request {
        url,
        method: Method::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: signal.cloned(),
        kind,
    }
}

/// The scheme or host rule of `policy` that denies `url` for `kind`, if any.
/// URLs without a host (such as `data:`) are checked against the empty host.
pub(crate) fn url_policy_violation(
    policy: &dyn ResourcePolicy,
    url: &Url,
    kind: ResourceKind,
) -> Option<ViolationType> {
    if !policy.is_scheme_allowed(url.scheme(), kind) {
        Some(ViolationType::SchemeNotAllowed)
    } else if !policy.is_host_allowed(url.host_str().unwrap_or(""), kind) {
        Some(ViolationType::HostNotAllowed)
    } else {
        None
    }
}

/// The tighter of two optional limits; `None` means unlimited.
pub(crate) fn min_limit<T: Ord>(a: Option<T>, b: Option<T>) -> Option<T> {
    a.into_iter().chain(b).min()
}

/// Strip credentials, query, and fragment from a URL before it appears in a
/// warning. Query strings commonly carry access tokens.
pub(crate) fn redacted_url(url: &Url) -> Url {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted
}

/// Policy details are provider-controlled free text. Retain the structured
/// violation type and redacted URL, but replace the free-form detail with a
/// fixed message so warning consumers cannot receive leaked credentials or
/// tokens from an untrusted provider.
pub(crate) fn sanitize_policy_violation(mut violation: PolicyViolation) -> PolicyViolation {
    violation.url = redacted_url(&violation.url);
    violation.details = "network policy denied the request".to_owned();
    violation
}

/// Classify a policy denial for `kind`: byte-cap denials become
/// [`WarningKind::ResourceLimitExceeded`] so they stay distinguishable from
/// ordinary policy denials, which are reported sanitized.
pub(crate) fn policy_violation_warning_kind(
    violation: &PolicyViolation,
    kind: ResourceKind,
) -> WarningKind {
    match &violation.violation_type {
        ViolationType::FetchTooLarge { limit, actual }
        | ViolationType::DecodedTooLarge { limit, actual } => WarningKind::ResourceLimitExceeded {
            kind,
            limit: *limit,
            actual: *actual,
        },
        _ => WarningKind::PolicyWarning {
            violation: sanitize_policy_violation(violation.clone()),
        },
    }
}

/// Keep provider-controlled diagnostics out of parse warnings. URLs are
/// already carried in structured warning fields, and free-form network errors
/// may contain credentials, headers, or other secrets supplied by a provider.
pub(crate) fn network_error_summary(error: &NetworkError) -> String {
    match error {
        NetworkError::Aborted => "request aborted".to_owned(),
        NetworkError::PolicyViolation(_) => "request denied by network policy".to_owned(),
        NetworkError::Io(_) => "network I/O error".to_owned(),
        NetworkError::Http(status) => format!("HTTP status error: {status}"),
        NetworkError::Other(_) => "provider returned a network error".to_owned(),
        _ => "provider returned an unknown network error".to_owned(),
    }
}

#[cfg(test)]
mod tests;
