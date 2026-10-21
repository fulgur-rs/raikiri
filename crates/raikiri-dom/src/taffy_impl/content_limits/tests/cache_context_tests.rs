use super::super::{Bounds, height_bounds, scoped};
use super::*;

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
            crate::LayoutStyle::default(),
            Some("position:absolute;display:block;width:100px;max-height:max-content"),
        );
        doc.append_element(
            Some(child),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;width:10px;height:50%"),
        );
    });
    let mut doc = fixture.doc;
    crate::layout::apply_computed_to_style(&mut doc, &fixture.cascade).unwrap();
    let mut tree = LayoutSession::new(&mut doc);
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
        child = doc.append_element(Some(root), "div", crate::LayoutStyle::default(), Some("position:absolute;display:block;width:100px;max-height:max-content;line-height:10px"));
        doc.append_text(child, "AAAA BBBB");
    });
    let mut doc = fixture.doc;
    crate::layout::apply_computed_to_style(&mut doc, &fixture.cascade).unwrap();
    let mut tree = LayoutSession::new(&mut doc);
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
        tree.cache_override_changed(child);
        tree.layout_style_for_key_mut(child).unwrap().size.width = taffy::Dimension::length(40.0);
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
            crate::LayoutStyle::default(),
            Some("display:block;width:100px;max-height:max-content;line-height:10px"),
        );
        doc.append_text(child, "A");
    });
    let mut doc = fixture.doc;
    crate::layout::apply_computed_to_style(&mut doc, &fixture.cascade).unwrap();
    let mut tree = LayoutSession::new(&mut doc);
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
        tree.cache_override_changed(child);
        tree.layout_style_for_key_mut(child).unwrap().border.top =
            taffy::LengthPercentage::length(10.0);
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
            crate::LayoutStyle::default(),
            Some("width:100px;max-height:max-content;padding:10%;line-height:10px"),
        );
        doc.append_text(child, "A");
    });
    let mut doc = fixture.doc;
    crate::layout::apply_computed_to_style(&mut doc, &fixture.cascade).unwrap();
    let mut tree = LayoutSession::new(&mut doc);
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
    let mut tree = LayoutSession::new(&mut doc);
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
    let mut tree = LayoutSession::new(&mut doc);
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
            crate::LayoutStyle::default(),
            Some("position:absolute;width:50px;max-height:max-content"),
        );
    });
    let mut doc = fixture.doc;
    crate::layout::apply_computed_to_style(&mut doc, &fixture.cascade).unwrap();
    let mut tree = LayoutSession::new(&mut doc);
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
#[should_panic(expected = "size cache scopes restore in reverse order")]
fn saved_sizes_reject_restoring_an_outer_scope_first() {
    let mut doc = crate::Document::new();
    let mut tree = LayoutSession::new(&mut doc);
    let outer = super::super::SavedSizes::capture(&mut tree, 0);
    let _inner = super::super::SavedSizes::capture(&mut tree, 0);
    outer.restore(&mut tree, 0);
}

#[test]
fn measure_lengths_keep_calc_identity_and_canonicalize_nan_and_zero() {
    use super::super::MeasureLength;
    use raikiri_layout_style::AlignedCalc;
    use raikiri_style::property::CalcLengthPercentage;
    let a = AlignedCalc::new(
        CalcLengthPercentage {
            px: 1.0,
            percent: 20.0,
        },
        true,
    );
    let b = AlignedCalc::new(
        CalcLengthPercentage {
            px: 1.0,
            percent: 20.0,
        },
        true,
    );
    let calc = |value: &AlignedCalc| {
        MeasureLength::new(taffy::Dimension::calc(std::ptr::from_ref(value).cast()).into_raw())
    };
    assert_eq!(calc(&a), calc(&a));
    assert_ne!(calc(&a), calc(&b));
    let number = |value| MeasureLength::new(taffy::Dimension::length(value).into_raw());
    assert_eq!(number(f32::NAN), number(f32::from_bits(0x7fc0_0001)));
    assert_eq!(number(-0.0), number(0.0));
    assert_ne!(number(1.0), number(2.0));
}
