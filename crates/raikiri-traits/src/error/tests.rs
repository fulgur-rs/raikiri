use super::*;

#[test]
fn unimplemented_display_includes_feature_and_hint() {
    let err = RenderError::Unimplemented {
        feature: "plan",
        migration_hint: "pagination 実装後に populate予定",
    };
    let s = format!("{err}");
    assert!(
        s.contains("plan"),
        "display must include feature: got {s:?}"
    );
    assert!(
        s.contains("pagination"),
        "display must include hint: got {s:?}"
    );
    assert!(
        s.contains("not implemented"),
        "display must include 'not implemented': got {s:?}"
    );
}

#[test]
fn unimplemented_source_is_none() {
    use std::error::Error;
    let err = RenderError::Unimplemented {
        feature: "future feature",
        migration_hint: "hint",
    };
    assert!(err.source().is_none(), "Unimplemented has no inner cause");
}

#[test]
fn page_geometry_nonconvergence_is_structured_and_terminal() {
    use std::error::Error;

    let err = RenderError::PageGeometryDidNotConverge { iterations: 3 };
    assert_eq!(
        err.to_string(),
        "page geometry did not converge in 3 iterations"
    );
    assert!(err.source().is_none());
}

#[test]
fn layout_error_resolver_variant_converts_to_render_error_resolver() {
    let re = ResolverError::Decode("bad PNG".into());
    let le = LayoutError::Resolver(re);
    let render_err: RenderError = le.into();
    assert!(matches!(render_err, RenderError::Resolver(_)));
}
#[test]
fn fragment_limit_error_reports_the_aggregate_cap() {
    let error = LayoutError::FragmentLimitExceeded { limit: 65_536 };
    assert_eq!(
        error.to_string(),
        "Layout fragment or break-flow work limit exceeded: 65536"
    );
}

#[test]
fn page_limit_and_abort_layout_errors_have_stable_messages() {
    assert_eq!(
        LayoutError::PageLimitExceeded {
            limit: 2,
            actual: 3,
        }
        .to_string(),
        "Layout page limit exceeded: 3 pages (limit 2)"
    );
    assert_eq!(LayoutError::Aborted.to_string(), "Layout aborted");
}

#[test]
fn counter_snapshot_layout_limit_converts_to_a_render_limit() {
    let layout_error = LayoutError::CounterSnapshotLimitExceeded {
        limit: 32,
        actual: 33,
    };
    assert_eq!(
        layout_error.to_string(),
        "Layout counter snapshot limit exceeded: 33 bytes (limit 32)"
    );

    let render_error: RenderError = layout_error.into();
    assert!(matches!(
        render_error,
        RenderError::LimitExceeded {
            kind: LimitKind::CounterSnapshots,
            limit: 32,
            actual: 33,
        }
    ));
}

#[test]
fn a_cascade_limit_converts_to_the_render_limit_it_comes_from() {
    for (cascade, render) in [
        (
            CascadeLimitKind::CandidatesPerElement,
            LimitKind::CascadeCandidatesPerElement,
        ),
        (
            CascadeLimitKind::DeclarationsVisited,
            LimitKind::CascadeDeclarations,
        ),
        (
            CascadeLimitKind::SelectorTests,
            LimitKind::CascadeSelectorTests,
        ),
        (
            CascadeLimitKind::RetainedBytes,
            LimitKind::CascadeRetainedBytes,
        ),
        (CascadeLimitKind::OutputBytes, LimitKind::CascadeOutputBytes),
    ] {
        let error: RenderError = CascadeError::LimitExceeded {
            kind: cascade,
            limit: 7,
            actual: 8,
        }
        .into();
        assert!(
            matches!(
                error,
                RenderError::LimitExceeded {
                    kind,
                    limit: 7,
                    actual: 8,
                } if kind == render
            ),
            "{cascade:?}: {error:?}"
        );
    }
}

#[test]
fn other_cascade_errors_stay_cascade_errors() {
    use std::error::Error;

    let error: RenderError = CascadeError::ResourceExhausted { bytes: 64 }.into();
    assert!(matches!(
        error,
        RenderError::Cascade(CascadeError::ResourceExhausted { bytes: 64 })
    ));
    assert_eq!(
        error.source().map(ToString::to_string).as_deref(),
        Some("CSS cascade could not allocate 64 bytes")
    );
    let error: RenderError = CascadeError::Internal {
        message: "broken".into(),
    }
    .into();
    assert!(matches!(
        error,
        RenderError::Cascade(CascadeError::Internal { .. })
    ));
}
