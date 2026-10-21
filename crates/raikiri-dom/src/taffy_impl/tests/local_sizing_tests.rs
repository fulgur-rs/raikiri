use super::*;
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::{ComputedLengthPercentageOrAuto as L, property::CalcLengthPercentage};

#[test]
fn local_sizing_overrides_keep_explicit_none_calc_and_signed_zero_without_leaking() {
    let base = Style {
        aspect_ratio: Some(2.0),
        ..Style::default()
    };
    let sibling = base.clone();
    let mut local = base.clone();
    let entries = [
        OwnedPropertyOverride::aspect_ratio(None),
        OwnedPropertyOverride::length(LengthProperty::Width, L::Px(20_000_000.0), true).unwrap(),
        OwnedPropertyOverride::length(
            LengthProperty::MinWidth,
            L::Calc(CalcLengthPercentage {
                px: -10.0,
                percent: 25.0,
            }),
            false,
        )
        .unwrap(),
        OwnedPropertyOverride::length(LengthProperty::MaxWidth, L::Percent(50.0), false).unwrap(),
    ];
    apply_local_sizing(&mut local, &entries);
    assert_eq!(local.aspect_ratio, None);
    assert_eq!(local.size.width, taffy::Dimension::length(20_000_000.0));
    let payload = entries[2].calc_owner().unwrap();
    assert_eq!(
        local.min_size.width.into_raw().calc_value(),
        std::ptr::from_ref(payload.as_ref()).cast()
    );
    assert_eq!(
        resolve_calc(local.min_size.width.into_raw().calc_value(), 100.0),
        15.0
    );
    assert_eq!(
        local.max_size.width,
        taffy::LengthPercentageAuto::percent(0.5)
    );
    let mut recursive = local.clone();
    apply_local_sizing(
        &mut recursive,
        &[
            OwnedPropertyOverride::aspect_ratio(Some(3.0)),
            OwnedPropertyOverride::length(LengthProperty::Width, L::Px(-0.0), true).unwrap(),
        ],
    );
    assert_eq!(recursive.aspect_ratio, Some(3.0));
    assert_eq!(
        recursive.size.width.into_raw().value().to_bits(),
        (-0.0f32).to_bits()
    );
    assert_eq!(local.aspect_ratio, None);
    assert_eq!(local.size.width, taffy::Dimension::length(20_000_000.0));
    for unchanged in [&base, &sibling] {
        assert_eq!(unchanged.aspect_ratio, Some(2.0));
        assert!(unchanged.size.width.is_auto());
        assert!(unchanged.min_size.width.is_auto());
        assert!(unchanged.max_size.width.is_auto());
    }
}

#[test]
fn local_sizing_overrides_do_not_publish_session_or_sibling_changes() {
    let (mut doc, cascade, child, sibling) = crate::layout::style_views::tests::fixture();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let epoch = tree.layout_cache_context.epoch();
    let original = tree.layout_style_for_key(child).unwrap().size;
    let sibling_size = tree.layout_style_for_key(sibling).unwrap().size;
    let mut local = tree.layout_style_for_key(child).unwrap().clone();
    apply_local_sizing(
        &mut local,
        &[
            OwnedPropertyOverride::length(LengthProperty::Width, L::Auto, false).unwrap(),
            OwnedPropertyOverride::aspect_ratio(Some(4.0)),
        ],
    );
    assert!(local.size.width.is_auto());
    assert_eq!(local.aspect_ratio, Some(4.0));
    assert_eq!(tree.layout_style_for_key(child).unwrap().size, original);
    assert_eq!(tree.style_view(child).size(), original);
    assert_eq!(
        tree.layout_style_for_key(sibling).unwrap().size,
        sibling_size
    );
    assert_eq!(tree.layout_cache_context.epoch(), epoch);
    assert_eq!(tree.override_checkpoint(), 0);
}
