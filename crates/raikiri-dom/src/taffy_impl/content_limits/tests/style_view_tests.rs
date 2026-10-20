use crate::layout::style_views::tests::fixture;
use crate::layout::{apply_computed_to_style, context::LayoutSession};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use taffy::CoreStyle;

#[test]
fn intrinsic_limit_install_and_restore_are_visible_to_the_view() {
    let (mut doc, mut cascade, child, _) = fixture();
    cascade.computed[child].min_width = L::MaxContent;
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session.prepare_style_views();
    let original = session.style_view(child).min_size();
    let inputs = taffy::LayoutInput {
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
    };
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
