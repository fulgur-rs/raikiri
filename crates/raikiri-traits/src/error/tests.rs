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
        feature: "render_streaming",
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
