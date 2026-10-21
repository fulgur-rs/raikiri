use crate::layout::style_views::tests::fixture;
use crate::layout::{apply_computed_to_style, context::LayoutSession};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use taffy::CoreStyle;

fn assert_unrelated_pass_height_survives(panic_body: bool) {
    use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};

    let (mut doc, mut cascade, child, _) = fixture();
    cascade.computed[child].min_width = L::MaxContent;
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::with_content_limits(
            &mut tree,
            taffy::NodeId::from(child),
            content_limit_inputs(),
            |tree, _, _| {
                assert!(!tree.style_view(child).min_size().width.is_auto());
                tree.cache_override_changed(child);
                tree.record_pass_override(
                    child,
                    OwnedPropertyOverride::length(LengthProperty::Height, L::Px(91.0), true)
                        .unwrap(),
                );
                tree.layout_style_for_key_mut(child).unwrap().size.height =
                    taffy::Dimension::length(91.0);
                if panic_body {
                    panic!("unwind after an unrelated pass update");
                }
                taffy::LayoutOutput::HIDDEN
            },
        );
    }));
    assert_eq!(failure.is_err(), panic_body);
    assert_eq!(tree.style_override_depth(child), 0);
    assert_eq!(
        tree.style_view(child).min_size().width,
        taffy::LengthPercentageAuto::auto()
    );
    assert_eq!(
        tree.style_view(child).size().height,
        taffy::Dimension::length(91.0)
    );
    assert_eq!(
        tree.layout_style_for_key(child).unwrap().size.height,
        taffy::Dimension::length(91.0)
    );
}

#[test]
fn content_limit_scope_keeps_an_unrelated_pass_update() {
    assert_unrelated_pass_height_survives(false);
}

#[test]
fn content_limit_scope_keeps_an_unrelated_pass_update_on_unwind() {
    assert_unrelated_pass_height_survives(true);
}

#[test]
fn content_limit_resolved_scope_keeps_large_finite_used_values() {
    use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};

    let mut legacy = crate::LayoutStyle::default();
    for entry in (super::super::Bounds {
        min: Some(20_000_000.0),
        max: None,
    })
    .overrides(true, 0.0)
    {
        let _previous = entry.apply_legacy_sizing(&mut legacy).unwrap();
    }
    assert_eq!(
        legacy.min_size.width,
        taffy::LengthPercentageAuto::length(20_000_000.0)
    );

    let (mut doc, cascade, child, _) = fixture();
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let key = tree.nodes.box_key(child).unwrap();
    tree.with_overrides(
        vec![(
            key,
            OwnedPropertyOverride::length(LengthProperty::MinWidth, L::Px(20_000_000.0), true)
                .unwrap(),
        )],
        |tree| {
            assert_eq!(
                tree.layout_style_for_key(child).unwrap().min_size.width,
                taffy::LengthPercentageAuto::length(20_000_000.0)
            );
            assert_eq!(
                tree.style_view(child).min_size().width,
                taffy::LengthPercentageAuto::length(20_000_000.0)
            );
        },
    );
}

pub(super) fn content_limit_inputs() -> taffy::LayoutInput {
    taffy::LayoutInput {
        run_mode: taffy::RunMode::ComputeSize,
        sizing_mode: taffy::SizingMode::InherentSize,
        axis: taffy::RequestedAxis::Both,
        known_dimensions: taffy::Size::NONE,
        known_dimensions_are_definite: taffy::Size {
            width: false,
            height: false,
        },
        parent_size: taffy::Size {
            width: Some(100.0),
            height: None,
        },
        available_space: taffy::Size {
            width: taffy::AvailableSpace::Definite(100.0),
            height: taffy::AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: taffy::Line::FALSE,
    }
}

#[test]
fn intrinsic_limit_install_and_restore_are_visible_to_the_view() {
    let (mut doc, mut cascade, child, _) = fixture();
    cascade.computed[child].min_width = L::MaxContent;
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    let original = session.style_view(child).min_size();
    let inputs = content_limit_inputs();
    super::super::with_content_limits(
        &mut session,
        taffy::NodeId::from(child),
        inputs,
        |tree, _, _| {
            let old = tree.layout_style_for_key(child).unwrap().min_size;
            assert!(!old.width.is_auto());
            assert_eq!(tree.style_view(child).min_size(), old);
            taffy::LayoutOutput::HIDDEN
        },
    );
    assert_eq!(session.style_view(child).min_size(), original);
    assert_eq!(session.style_override_depth(child), 0);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::with_content_limits(
            &mut session,
            taffy::NodeId::from(child),
            inputs,
            |_, _, _| {
                panic!("intrinsic scope unwind");
            },
        );
    }));
    assert!(panic.is_err());
    assert_eq!(
        session.layout_style_for_key(child).unwrap().min_size,
        original
    );
    assert_eq!(session.style_view(child).min_size(), original);
    assert_eq!(session.style_override_depth(child), 0);
}

#[test]
fn width_limit_scope_restores_after_callback_unwinds() {
    let (mut doc, mut cascade, child, _) = fixture();
    cascade.computed[child].min_width = L::MaxContent;
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    let original = session.style_view(child).min_size();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::with_width_limits(
            &mut session,
            child,
            taffy::Size {
                width: Some(100.0),
                height: None,
            },
            100.0,
            |tree| {
                assert_eq!(tree.style_override_depth(child), 1);
                assert!(!tree.style_view(child).min_size().width.is_auto());
                panic!("width callback unwind");
            },
        );
    }));
    assert!(panic.is_err());
    assert_eq!(
        session.layout_style_for_key(child).unwrap().min_size,
        original
    );
    assert_eq!(session.style_view(child).min_size(), original);
    assert_eq!(session.style_override_depth(child), 0);
    assert_eq!(session.override_checkpoint(), 0);
    assert_eq!(session.context.content_limit_depth, 0);
}

#[test]
fn oof_raw_prepass_temporarily_clears_numeric_limits_in_both_readers() {
    let (mut doc, mut cascade, child, _) = fixture();
    cascade.computed[child].position = raikiri_style::property::PositionValue::Absolute;
    cascade.computed[child].min_width = L::Px(50.0);
    cascade.computed[child].max_height = L::MaxContent;
    crate::layout::test_support::with_ahem(&mut doc);
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let original = tree.layout_style_for_key(child).unwrap().min_size;
    let inputs = taffy::LayoutInput {
        run_mode: taffy::RunMode::ComputeSize,
        parent_size: taffy::Size {
            width: Some(100.0),
            height: Some(100.0),
        },
        ..taffy::LayoutInput::HIDDEN
    };
    let mut saw_raw = false;
    super::super::with_oof_content_limits(
        &mut tree,
        Some(&[taffy::NodeId::from(child)]),
        &[],
        |tree| {
            if let Some(capture) = tree.context.oof_content_limit_inputs.last_mut() {
                capture.insert(child, inputs);
            }
            let min = tree.layout_style_for_key(child).unwrap().min_size;
            saw_raw |= min.width.is_auto();
            assert_eq!(tree.style_view(child).min_size(), min);
        },
    );
    assert!(saw_raw);
    assert_eq!(tree.layout_style_for_key(child).unwrap().min_size, original);
    assert_eq!(tree.style_view(child).min_size(), original);
    assert_eq!(tree.style_override_depth(child), 0);
}

#[test]
fn height_only_content_limit_keeps_the_callers_available_width() {
    let (mut doc, mut cascade, child, _) = fixture();
    cascade.computed[child].min_height = L::MaxContent;
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let mut inputs = content_limit_inputs();
    inputs.known_dimensions.width = Some(20.0);
    inputs.available_space.width = taffy::AvailableSpace::Definite(77.0);
    super::super::with_content_limits(
        &mut tree,
        taffy::NodeId::from(child),
        inputs,
        |_, _, inputs| {
            assert_eq!(inputs.known_dimensions.width, Some(20.0));
            assert_eq!(
                inputs.available_space.width,
                taffy::AvailableSpace::Definite(77.0)
            );
            taffy::LayoutOutput::HIDDEN
        },
    );
}
