use super::super::{Bounds, height_bounds, scoped};
use super::*;
use raikiri_layout_style::values::Dimension;
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::ComputedLengthPercentageOrAuto as L;

fn input(width: f32, height: f32) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width: Some(width),
            height: Some(height),
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        parent_size: Size {
            width: Some(200.0),
            height: Some(300.0),
        },
        available_space: Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

#[test]
fn height_memo_distinguishes_the_effective_percentage_height_basis() {
    let mut child = 0;
    let fixture = block_fixture_with_ahem("width:200px;height:300px", |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            Some("position:absolute;display:block;width:100px;max-height:max-content"),
        );
        doc.append_element(
            Some(child),
            "div",
            Some("display:block;width:10px;height:50%"),
        );
    });
    let mut doc = fixture.doc;
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    tree.sync_layout_cache_mutations();
    let limits = tree.nodes[child].content_limits;
    let heights = scoped(&mut tree, |tree| {
        [100.0, 200.0].map(|height| {
            height_bounds(
                tree,
                NodeId::from(child),
                &limits,
                &input(100.0, height),
                Bounds::default(),
            )
            .max
            .unwrap()
        })
    });
    assert_eq!(heights, [50.0, 100.0]);
}

#[test]
fn height_memo_distinguishes_preferred_width_kept_by_the_probe() {
    let mut child = 0;
    let fixture = block_fixture_with_ahem("width:200px;height:300px", |doc, root| {
        child = doc.append_element(Some(root), "div", Some("position:absolute;display:block;width:100px;max-height:max-content;line-height:10px"));
        doc.append_text(child, "AAAA BBBB");
    });
    let mut doc = fixture.doc;
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    tree.sync_layout_cache_mutations();
    let limits = tree.nodes[child].content_limits;
    let mut inputs = input(200.0, 300.0);
    inputs.known_dimensions = Size::NONE;
    inputs.known_dimensions_are_definite = Size {
        width: false,
        height: false,
    };
    let heights = scoped(&mut tree, |tree| {
        let wide = height_bounds(
            tree,
            NodeId::from(child),
            &limits,
            &inputs,
            Bounds::default(),
        )
        .max
        .unwrap();
        let key = tree.nodes.box_key(child).unwrap();
        tree.push_overrides(vec![(
            key,
            OwnedPropertyOverride::length(LengthProperty::Width, L::Px(40.0), true).unwrap(),
        )]);
        let narrow = height_bounds(
            tree,
            NodeId::from(child),
            &limits,
            &inputs,
            Bounds::default(),
        )
        .max
        .unwrap();
        [wide, narrow]
    });
    assert_eq!(heights, [10.0, 20.0]);
}

#[test]
fn height_memo_tracks_current_border_widths() {
    let mut child = 0;
    let fixture = block_fixture_with_ahem("width:200px", |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            Some("display:block;width:100px;max-height:max-content;line-height:10px"),
        );
        doc.append_text(child, "A");
    });
    let mut doc = fixture.doc;
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    tree.sync_layout_cache_mutations();
    let limits = tree.nodes[child].content_limits;
    let mut inputs = input(100.0, 0.0);
    inputs.known_dimensions.height = None;
    let heights = scoped(&mut tree, |tree| {
        let plain = height_bounds(
            tree,
            NodeId::from(child),
            &limits,
            &inputs,
            Bounds::default(),
        )
        .max
        .unwrap();
        let key = tree.nodes.box_key(child).unwrap();
        tree.push_overrides(vec![(
            key,
            OwnedPropertyOverride::length(LengthProperty::BorderTop, L::Px(10.0), true).unwrap(),
        )]);
        let bordered = height_bounds(
            tree,
            NodeId::from(child),
            &limits,
            &inputs,
            Bounds::default(),
        )
        .max
        .unwrap();
        [plain, bordered]
    });
    assert_eq!(heights, [10.0, 20.0]);
}

#[test]
fn paragraph_height_memo_tracks_the_active_flex_edge_basis() {
    let mut child = 0;
    let fixture = block_fixture_with_ahem("display:flex;width:200px", |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            Some("width:100px;max-height:max-content;padding:10%;line-height:10px"),
        );
        doc.append_text(child, "A");
    });
    let mut doc = fixture.doc;
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    tree.sync_layout_cache_mutations();
    let limits = tree.nodes[child].content_limits;
    let mut inputs = input(100.0, 0.0);
    inputs.known_dimensions.height = None;
    inputs.parent_size.width = None;
    tree.context.flex_item_percent_bases.push((
        fixture.root,
        Size {
            width: Some(100.0),
            height: None,
        },
    ));
    let heights = scoped(&mut tree, |tree| {
        let first = height_bounds(
            tree,
            NodeId::from(child),
            &limits,
            &inputs,
            Bounds::default(),
        )
        .max
        .unwrap();
        tree.context
            .flex_item_percent_bases
            .last_mut()
            .unwrap()
            .1
            .width = Some(200.0);
        let second = height_bounds(
            tree,
            NodeId::from(child),
            &limits,
            &inputs,
            Bounds::default(),
        )
        .max
        .unwrap();
        [first, second]
    });
    assert_eq!(heights[1] - heights[0], 20.0);
}

#[test]
fn intrinsic_scope_unwind_restores_percentage_height_basis() {
    let mut doc = crate::Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    tree.context
        .content_limit_percentage_height_bases
        .push((0, 50.0));
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scoped(&mut tree, |tree| {
            tree.context
                .content_limit_percentage_height_bases
                .push((0, 100.0));
            panic!("height probe unwind");
        })
    }));
    assert!(panic.is_err());
    assert_eq!(
        tree.context.content_limit_percentage_height_bases,
        [(0, 50.0)]
    );
    assert_eq!(tree.context.content_limit_depth, 0);
}

#[test]
fn oof_input_capture_restores_its_stack_and_prepass_depth_on_unwind() {
    let mut doc = crate::Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    tree.context
        .oof_content_limit_inputs
        .push(std::collections::HashMap::new());
    tree.context.oof_content_limit_prepass_depth = 2;
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::capture_oof_layout_inputs(
            &mut tree,
            &mut |tree| {
                assert_eq!(tree.context.oof_content_limit_inputs.len(), 2);
                assert_eq!(tree.context.oof_content_limit_prepass_depth, 3);
                panic!("OOF capture unwind");
            },
            true,
        );
    }));
    assert!(panic.is_err());
    assert_eq!(tree.context.oof_content_limit_inputs.len(), 1);
    assert_eq!(tree.context.oof_content_limit_prepass_depth, 2);
}

#[test]
fn oof_prepass_runs_again_after_an_unwind() {
    let mut child = 0;
    let fixture = block_fixture_with_ahem("width:200px", |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            Some("position:absolute;width:50px;max-height:max-content"),
        );
    });
    let mut doc = fixture.doc;
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let candidate = NodeId::from(child);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_oof_content_limits(&mut tree, Some(&[candidate]), &[], |_| {
            panic!("OOF prepass")
        })
    }));
    assert!(panic.is_err());
    assert_eq!(tree.context.oof_content_limit_prepass_depth, 0);
    assert!(tree.context.oof_content_limit_inputs.is_empty());
    let mut calls = 0;
    with_oof_content_limits(&mut tree, Some(&[candidate]), &[], |_| {
        calls += 1;
    });
    assert_eq!(calls, 2);
}

#[test]
#[should_panic(expected = "style scopes restore in reverse order")]
fn typed_scopes_reject_restoring_an_outer_scope_first() {
    let mut doc = crate::Document::new();
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let outer = tree.record_style_overrides(0, Vec::new());
    let _inner = tree.record_style_overrides(0, Vec::new());
    tree.restore_recorded_style_overrides(outer);
}

#[test]
fn measure_lengths_keep_calc_identity_and_canonicalize_nan_and_zero() {
    use raikiri_layout_style::AlignedCalc;
    use raikiri_layout_style::values::CalcRef;
    use raikiri_style::property::CalcLengthPercentage;
    let owner = || {
        std::sync::Arc::new(AlignedCalc::new(
            CalcLengthPercentage {
                px: 1.0,
                percent: 20.0,
            },
            true,
        ))
    };
    let (a, b) = (owner(), owner());
    let calc =
        |value: &std::sync::Arc<AlignedCalc>| Dimension::calc(CalcRef::retained(value)).identity();
    assert_eq!(calc(&a), calc(&a));
    assert_ne!(calc(&a), calc(&b));
    let number = |value| Dimension::length(value).identity();
    assert_eq!(number(f32::NAN), number(f32::from_bits(0x7fc0_0001)));
    assert_eq!(number(-0.0), number(0.0));
    assert_ne!(number(1.0), number(2.0));
}
