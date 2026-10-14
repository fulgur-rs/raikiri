use super::*;

fn url(raw: &str) -> Url {
    Url::parse(raw).expect("test URL parses")
}

#[test]
fn redacted_url_drops_credentials_query_and_fragment() {
    let redacted = redacted_url(&url(
        "https://user:pass@example.test/a.css?token=secret#frag",
    ));
    assert_eq!(redacted.as_str(), "https://example.test/a.css");
}

#[test]
fn byte_cap_violations_become_resource_limit_warnings() {
    for violation_type in [
        ViolationType::FetchTooLarge {
            limit: 1,
            actual: 2,
        },
        ViolationType::DecodedTooLarge {
            limit: 1,
            actual: 2,
        },
    ] {
        let violation = PolicyViolation {
            kind: ResourceKind::Image,
            url: url("https://example.test/a.png"),
            violation_type,
            details: String::new(),
        };
        assert!(matches!(
            policy_violation_warning_kind(&violation, ResourceKind::Image),
            WarningKind::ResourceLimitExceeded {
                kind: ResourceKind::Image,
                limit: 1,
                actual: 2,
            }
        ));
    }
}

#[test]
fn other_violations_are_reported_sanitized() {
    let violation = PolicyViolation {
        kind: ResourceKind::ExternalStylesheet,
        url: url("https://example.test/a.css?token=secret"),
        violation_type: ViolationType::HostNotAllowed,
        details: "provider text with secret".to_owned(),
    };
    let WarningKind::PolicyWarning { violation } =
        policy_violation_warning_kind(&violation, ResourceKind::ExternalStylesheet)
    else {
        panic!("expected a policy warning");
    };
    assert_eq!(violation.url.as_str(), "https://example.test/a.css");
    assert_eq!(violation.details, "network policy denied the request");
}

#[test]
fn mime_essence_ignores_parameters_and_whitespace() {
    assert_eq!(mime_essence(" text/css ; charset=utf-8"), "text/css");
    assert_eq!(mime_essence("image/svg+xml"), "image/svg+xml");
}

#[test]
fn min_limit_treats_none_as_unlimited() {
    assert_eq!(min_limit(Some(3), Some(5)), Some(3));
    assert_eq!(min_limit(None, Some(5)), Some(5));
    assert_eq!(min_limit::<u64>(None, None), None);
}
