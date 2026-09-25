use super::*;

#[test]
fn policy_violation_display_includes_kind_url_violation_type_details() {
    let violation = PolicyViolation {
        kind: ResourceKind::ExternalStylesheet,
        url: Url::parse("https://example.test/main.css").expect("valid url"),
        violation_type: ViolationType::MimeNotAllowed {
            mime: "text/plain".into(),
        },
        details: "expected text/css".to_string(),
    };
    let s = format!("{violation}");
    // kind (Display of ResourceKind, not Debug)
    assert!(
        s.contains("external stylesheet"),
        "display must include kind via ResourceKind::Display: got {s:?}"
    );
    // Regression check: Debug format must not leak (auto-derived Debug can
    // silently change when variant fields are added; new tests below check
    // every ResourceKind variant's Display string).
    assert!(
        !s.contains("ExternalStylesheet"),
        "display must not leak ResourceKind Debug format: got {s:?}"
    );
    // violation_type (Display of ViolationType, which for MimeNotAllowed includes the mime)
    assert!(
        s.contains("MIME type not allowed"),
        "display must include violation_type: got {s:?}"
    );
    assert!(
        s.contains("text/plain"),
        "display must include violation_type payload: got {s:?}"
    );
    // url
    assert!(
        s.contains("https://example.test/main.css"),
        "display must include url: got {s:?}"
    );
    // details
    assert!(
        s.contains("expected text/css"),
        "display must include details: got {s:?}"
    );
}

#[test]
fn private_network_blocked_display_is_distinct_from_host_not_allowed() {
    let blocked = PolicyViolation {
        kind: ResourceKind::Image,
        url: Url::parse("https://example.test/x.png").expect("valid url"),
        violation_type: ViolationType::PrivateNetworkBlocked,
        details: "resolved IP is not globally routable".to_string(),
    };
    let host_denied = PolicyViolation {
        violation_type: ViolationType::HostNotAllowed,
        ..blocked.clone()
    };
    let blocked_display = format!("{blocked}");
    let host_denied_display = format!("{host_denied}");
    assert_ne!(
        blocked_display, host_denied_display,
        "PrivateNetworkBlocked must render distinctly from HostNotAllowed \
         so logs/telemetry can tell an SSRF-floor rejection apart from an \
         ordinary Consumer-policy host denial"
    );
    assert!(blocked_display.contains("private network"));
}
