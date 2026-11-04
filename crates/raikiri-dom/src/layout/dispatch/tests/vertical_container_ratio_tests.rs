use crate::Document;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, NodeId, RequestedAxis, RunMode, SizingMode,
};
use crate::layout::test_support::cascaded;
use raikiri_layout_geometry::Size;
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::{CascadeResult, ComputedLengthPercentageOrAuto as L};

fn column_item() -> (Document, CascadeResult, usize) {
    let mut doc = Document::new();
    let parent = doc.append_element(
        Some(0),
        "div",
        Some("display:flex;flex-direction:column;align-items:flex-start"),
    );
    let child = doc.append_element(
        Some(parent),
        "div",
        Some("display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1"),
    );
    doc.append_element(Some(child), "div", Some("width:30px;height:100px"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    (doc, cascade, child)
}

fn item_inputs(height: f32, intrinsic: bool, definite: bool) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::ContentSize,
        axis: RequestedAxis::Horizontal,
        known_dimensions: Size {
            width: None,
            height: Some(height),
        },
        known_dimensions_are_definite: Size {
            width: false,
            height: definite,
        },
        parent_size: Size {
            width: None,
            height: Some(height),
        },
        available_space: Size {
            width: if intrinsic {
                AvailableSpace::MinContent
            } else {
                AvailableSpace::Definite(400.0)
            },
            height: AvailableSpace::Definite(height),
        },
        ..LayoutInput::HIDDEN
    }
}

#[test]
fn intrinsic_contribution_uses_the_effective_stated_height_and_restores_the_requested_height() {
    let (mut doc, cascade, child) = column_item();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    for height in [300.0, 40.0, 300.0] {
        let output = tree.compute_child_layout(child.into(), item_inputs(height, true, true));
        assert_eq!(
            output.size,
            Size {
                width: 200.0,
                height
            }
        );
    }
    let key = tree.nodes.box_key(child).expect("item key");
    tree.with_overrides(
        vec![(
            key,
            OwnedPropertyOverride::length(LengthProperty::Height, L::Auto, false)
                .expect("auto height"),
        )],
        |tree| {
            let output = tree.compute_child_layout(child.into(), item_inputs(300.0, true, true));
            assert_eq!(
                output.size,
                Size {
                    width: 30.0,
                    height: 300.0
                }
            );
        },
    );
    assert_eq!(
        tree.compute_child_layout(child.into(), item_inputs(300.0, true, true))
            .size
            .width,
        200.0
    );
}

#[test]
fn final_cross_measurement_uses_definite_flexed_height_and_keeps_indefinite_queries_separate() {
    let (mut doc, cascade, child) = column_item();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    for (height, definite, width) in [
        (40.0, true, 80.0),
        (40.0, false, 30.0),
        (300.0, true, 600.0),
        (40.0, true, 80.0),
    ] {
        let output = tree.compute_child_layout(child.into(), item_inputs(height, false, definite));
        assert_eq!(output.size, Size { width, height });
    }
}

#[test]
fn wrapped_inline_measures_separate_physical_width_from_cyclic_edges_and_preserve_geometry() {
    let mut doc = Document::new();
    let outer = doc.append_element(Some(0), "div", Some("display:flex;width:400px"));
    let parent = doc.append_element(
        Some(outer),
        "div",
        Some("display:flex;writing-mode:vertical-rl;flex-wrap:wrap;aspect-ratio:2/1"),
    );
    let child = doc.append_element(
        Some(parent),
        "div",
        Some("width:100%;aspect-ratio:1/4;padding-top:10%;padding-bottom:10%"),
    );
    doc.append_element(Some(parent), "div", Some("width:30px;height:70px"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let before: Vec<_> = tree.nodes.iter().map(|node| node.layout).collect();
    let padding = tree.physical_style(child).padding();
    let measure = |tree: &mut LayoutSession<'_>, width| {
        super::super::intrinsic_scopes::wrapping_min_content_height(tree, parent.into(), width)
    };
    assert_eq!(measure(&mut tree, Some(40.0)), 160.0);
    let epoch = tree.layout_cache_context.epoch();
    for (room, expected) in [(Some(100.0), 400.0), (Some(40.0), 160.0), (None, 70.0)] {
        assert_eq!(measure(&mut tree, room), expected);
        assert_eq!(tree.layout_cache_context.epoch(), epoch);
        assert_eq!(tree.physical_style(child).padding(), padding);
        assert_eq!(tree.override_checkpoint(), 0);
    }
    for (definite, height) in [(false, 70.0), (true, 400.0)] {
        let output = tree.compute_child_layout(
            parent.into(),
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                known_dimensions: Size {
                    width: Some(100.0),
                    height: None,
                },
                known_dimensions_are_definite: Size {
                    width: definite,
                    height: false,
                },
                parent_size: Size {
                    width: Some(400.0),
                    height: None,
                },
                available_space: Size {
                    width: AvailableSpace::Definite(100.0),
                    height: AvailableSpace::MaxContent,
                },
                ..LayoutInput::HIDDEN
            },
        );
        assert_eq!(
            output.size,
            Size {
                width: 100.0,
                height
            }
        );
        assert_eq!(tree.layout_cache_context.epoch(), epoch);
    }
    assert_eq!(
        tree.nodes
            .iter()
            .map(|node| node.layout)
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn a_private_baseline_probe_of_a_wrapped_vertical_ratio_item_keeps_geometry_private() {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "div",
        Some("display:flex;align-items:baseline;width:400px"),
    );
    let item = doc.append_element(
        Some(root),
        "div",
        Some("display:flex;writing-mode:vertical-rl;flex-wrap:wrap;width:100px;aspect-ratio:2/1"),
    );
    for height in [60, 70] {
        doc.append_element(
            Some(item),
            "div",
            Some(format!("width:30px;height:{height}px")),
        );
    }
    doc.append_element(Some(root), "div", Some("height:40px;width:10px"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let before: Vec<_> = tree.nodes.iter().map(|node| node.layout).collect();
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        known_dimensions: Size {
            width: Some(400.0),
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        parent_size: Size {
            width: Some(800.0),
            height: Some(600.0),
        },
        available_space: Size {
            width: AvailableSpace::Definite(400.0),
            height: AvailableSpace::MaxContent,
        },
        ..LayoutInput::HIDDEN
    };
    let measured = tree.with_baseline_measure(root, |tree| {
        tree.compute_child_layout(NodeId::from(root), inputs)
    });
    assert_eq!(measured.baselines.first, Some(70.0));
    assert_eq!(
        tree.nodes
            .iter()
            .map(|node| node.layout)
            .collect::<Vec<_>>(),
        before
    );
    let placed = tree.compute_child_layout(
        root.into(),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            ..inputs
        },
    );
    assert_eq!(tree.nodes[item].layout.size.height, 70.0);
    assert_eq!(measured.baselines, placed.baselines);
    assert_eq!(measured.size, placed.size);
}
