use super::*;
use crate::layout::box_style_data::LayoutBase;
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use std::sync::Arc;

fn fixture() -> (crate::Document, raikiri_style::CascadeResult, usize, usize) {
    let mut item = 0;
    let fixture = block_fixture("display:grid;writing-mode:vertical-rl", |doc, root| {
        item = doc.append_element(
            Some(root),
            "div",
            Some("display:grid;height:min-content;min-height:10px"),
        );
        doc.append_element(Some(item), "div", Some("width:10px;height:10px"));
    });
    (fixture.doc, fixture.cascade, fixture.root, item)
}

fn inputs() -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size::splat(false),
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MinContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

fn held(tree: &LayoutSession<'_>, root: usize) -> Vec<(usize, f32)> {
    super::super::items_held_to_minimum_inline_size(tree, NodeId::from(root), &inputs(), true)
}

#[test]
fn non_ifc_height_keywords_respect_source_and_height_override_provenance() {
    for pass in [false, true] {
        let (mut doc, cascade, root, item) = fixture();
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let key = tree.nodes.box_key(item).unwrap();
        assert!(tree.nodes[item].ifc.is_none());
        assert!(held(&tree, root).is_empty());
        let minimum =
            OwnedPropertyOverride::length(LengthProperty::MinHeight, L::Px(20.0), true).unwrap();
        tree.with_overrides(vec![(key, minimum)], |tree| {
            assert!(held(tree, root).is_empty());
        });
        let fixed =
            OwnedPropertyOverride::length(LengthProperty::Height, L::Px(20.0), true).unwrap();
        tree.with_overrides(vec![(key, fixed)], |tree| {
            assert!(held(tree, root).is_empty());
        });
        let auto = OwnedPropertyOverride::length(LengthProperty::Height, L::Auto, false).unwrap();
        if pass {
            tree.record_pass_override(item, auto);
            assert_eq!(held(&tree, root), vec![(item, 10.0)]);
        } else {
            tree.with_overrides(vec![(key, auto)], |tree| {
                assert_eq!(held(tree, root), vec![(item, 10.0)]);
            });
            assert!(held(&tree, root).is_empty());
        }
    }
    let (mut doc, cascade, root, item) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(item).unwrap();
    tree.box_style_data.get_mut(key).unwrap().base =
        LayoutBase::Synthetic(Arc::new(cascade.computed[item].clone()));
    assert_eq!(held(&tree, root), vec![(item, 10.0)]);
}

#[test]
fn vertical_minimum_probe_restores_overrides_and_keeps_the_cache_epoch_on_unwind() {
    for unwind in [false, true] {
        let (mut doc, mut cascade, root, item) = fixture();
        cascade.computed[item].height_keyword = Some(raikiri_style::ContentSizeKeyword::FitContent);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        tree.sync_layout_cache_mutations();
        let epoch = tree.layout_cache_context.epoch();
        let mut calls = 0;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_grid_item_content_limits(
                &mut tree,
                NodeId::from(root),
                inputs(),
                Size::NONE,
                |tree, _, input| {
                    calls += 1;
                    assert_eq!(tree.layout_cache_context.epoch(), epoch);
                    if calls == 1 {
                        assert_eq!(input.axis, RequestedAxis::Vertical);
                        assert_eq!(
                            tree.physical_style(item).size().height,
                            Dimension::length(10.0)
                        );
                        if unwind {
                            panic!("vertical minimum probe unwind");
                        }
                    } else {
                        assert!(tree.physical_style(item).size().height.is_auto());
                        assert_eq!(input.known_dimensions.height, Some(10.0));
                        assert_eq!(input.available_space.height, AvailableSpace::Definite(10.0));
                    }
                    LayoutOutput::from_outer_size(Size {
                        width: 20.0,
                        height: 10.0,
                    })
                },
            )
        }));
        assert_eq!(result.is_err(), unwind);
        assert_eq!(calls, if unwind { 1 } else { 2 });
        assert!(tree.physical_style(item).size().height.is_auto());
        assert_eq!(tree.override_checkpoint(), 0);
        assert_eq!(tree.layout_cache_context.epoch(), epoch);
        assert!(tree.context.content_limit_probes.is_empty());
    }
}
