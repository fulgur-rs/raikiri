use super::*;
use crate::layout::protocol::{
    AvailableSpace, AxisStaticAlign, AxisStaticEdge, AxisStaticPosition, Baselines,
    CollapsibleMarginSet, OofCandidate, OofPositioningArea, RequestedAxis, RunMode, SizingMode,
};
use crate::layout::style_views::tests::fixture_with_styles;
use crate::taffy_impl::vertical_block::{TransposedBlock, with_transposed_frame};
use raikiri_layout_geometry::{Edges, Line, Point, Size};
use taffy::{AlignmentSafety, Position};

/// The two block directions: `vertical-lr` and `vertical-rl`.
const BLOCK_DIRECTIONS: [bool; 2] = [false, true];

fn rect(left: f32, right: f32, top: f32, bottom: f32) -> Edges<f32> {
    Edges {
        left,
        right,
        top,
        bottom,
    }
}

fn static_position(start: f32, end: f32, keyword: AxisStaticEdge) -> AxisStaticPosition {
    AxisStaticPosition {
        area: Line { start, end },
        align: AxisStaticAlign {
            keyword,
            safety: AlignmentSafety::Default,
            fallback: AxisStaticEdge::Start,
        },
    }
}

fn candidate(x: AxisStaticPosition, y: AxisStaticPosition) -> OofCandidate {
    OofCandidate {
        node: NodeId::from(1usize),
        order: 0,
        position: Position::Absolute,
        static_position: Point { x, y },
    }
}

/// Inputs whose every per-axis field differs between the two axes: a
/// known width that is definite, an unknown height, a parent that is known
/// in one direction only, and unlike available spaces.
fn asymmetric_inputs() -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Horizontal,
        known_dimensions: Size {
            width: Some(1.0),
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        parent_size: Size {
            width: Some(3.0),
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::Definite(5.0),
            height: AvailableSpace::MinContent,
        },
        vertical_margins_are_collapsible: Line::TRUE,
    }
}

#[test]
fn container_inputs_swap_every_per_axis_field_together() {
    for right_to_left in BLOCK_DIRECTIONS {
        let swapped = AxisAdapter::new(right_to_left).container_inputs(asymmetric_inputs());
        assert_eq!(
            swapped.known_dimensions,
            Size {
                width: None,
                height: Some(1.0)
            }
        );
        // The definiteness travels with the dimension it describes.
        assert_eq!(
            swapped.known_dimensions_are_definite,
            Size {
                width: false,
                height: true
            }
        );
        // A parent known on one side only stays known on that side.
        assert_eq!(
            swapped.parent_size,
            Size {
                width: None,
                height: Some(3.0)
            }
        );
        assert_eq!(
            swapped.available_space,
            Size {
                width: AvailableSpace::MinContent,
                height: AvailableSpace::Definite(5.0)
            }
        );
        assert_eq!(swapped.run_mode, RunMode::PerformLayout);
        assert_eq!(swapped.sizing_mode, SizingMode::InherentSize);
    }
}

#[test]
fn container_inputs_swap_the_requested_axis_and_keep_both() {
    for (before, after) in [
        (RequestedAxis::Horizontal, RequestedAxis::Vertical),
        (RequestedAxis::Vertical, RequestedAxis::Horizontal),
        (RequestedAxis::Both, RequestedAxis::Both),
    ] {
        let inputs = LayoutInput {
            axis: before,
            ..asymmetric_inputs()
        };
        for right_to_left in BLOCK_DIRECTIONS {
            let swapped = AxisAdapter::new(right_to_left).container_inputs(inputs);
            assert_eq!(swapped.axis, after, "{before:?}");
        }
    }
}

#[test]
fn run_and_sizing_modes_are_kept() {
    for (run_mode, sizing_mode) in [
        (RunMode::ComputeSize, SizingMode::ContentSize),
        (RunMode::PerformLayout, SizingMode::ContentSize),
        (RunMode::PerformHiddenLayout, SizingMode::InherentSize),
    ] {
        let inputs = LayoutInput {
            run_mode,
            sizing_mode,
            ..asymmetric_inputs()
        };
        let adapter = AxisAdapter::new(true);
        let container = adapter.container_inputs(inputs);
        assert_eq!(
            (container.run_mode, container.sizing_mode),
            (run_mode, sizing_mode)
        );
        let child = adapter.child_inputs(inputs).inputs;
        assert_eq!((child.run_mode, child.sizing_mode), (run_mode, sizing_mode));
    }
}

#[test]
fn no_vertical_margin_collapse_crosses_the_boundary_in_either_direction() {
    for collapsible in [
        Line::TRUE,
        Line::FALSE,
        Line {
            start: true,
            end: false,
        },
    ] {
        let inputs = LayoutInput {
            vertical_margins_are_collapsible: collapsible,
            ..asymmetric_inputs()
        };
        for right_to_left in BLOCK_DIRECTIONS {
            let adapter = AxisAdapter::new(right_to_left);
            assert_eq!(
                adapter
                    .container_inputs(inputs)
                    .vertical_margins_are_collapsible,
                Line::FALSE
            );
            assert_eq!(
                adapter
                    .child_inputs(inputs)
                    .inputs
                    .vertical_margins_are_collapsible,
                Line::FALSE
            );
        }
    }
}

#[test]
fn a_child_gets_back_the_inputs_taffy_built_on_the_physical_axes() {
    for right_to_left in BLOCK_DIRECTIONS {
        let adapter = AxisAdapter::new(right_to_left);
        let physical = LayoutInput {
            vertical_margins_are_collapsible: Line::FALSE,
            ..asymmetric_inputs()
        };
        let on_layout_axes = adapter.container_inputs(physical);
        assert_eq!(adapter.child_inputs(on_layout_axes).inputs, physical);
    }
}

#[test]
fn the_padding_basis_is_the_inline_size_of_the_container() {
    // On layout axes the width of the parent is the container's inline size.
    let known = LayoutInput {
        parent_size: Size {
            width: Some(200.0),
            height: Some(300.0),
        },
        ..asymmetric_inputs()
    };
    let unknown = LayoutInput {
        parent_size: Size {
            width: None,
            height: Some(300.0),
        },
        ..asymmetric_inputs()
    };
    for right_to_left in BLOCK_DIRECTIONS {
        let adapter = AxisAdapter::new(right_to_left);
        let child = adapter.child_inputs(known);
        assert_eq!(child.padding_basis, Some(200.0));
        // The same size is the parent height of the child's physical axes.
        assert_eq!(child.inputs.parent_size.height, Some(200.0));
        assert_eq!(child.inputs.parent_size.width, Some(300.0));
        assert_eq!(adapter.child_inputs(unknown).padding_basis, None);
    }
}

/// An output as taffy produces it for a container on layout axes: 180
/// along the inline axis and 120 along the block axis, with an out-of-flow
/// positioning area 180 by 100 at offset (5, 7).
fn output_on_layout_axes() -> LayoutOutput {
    let mut output = LayoutOutput {
        size: Size {
            width: 180.0,
            height: 120.0,
        },
        scrollable_overflow_rect: rect(0.0, 11.0, 0.0, 13.0),
        top_margin: CollapsibleMarginSet::from_margin(4.0),
        bottom_margin: CollapsibleMarginSet::from_margin(6.0),
        margins_can_collapse_through: true,
        oof_positioning_area: Some(OofPositioningArea {
            size: Size {
                width: 180.0,
                height: 100.0,
            },
            offset: Point { x: 5.0, y: 7.0 },
        }),
        baselines: Baselines {
            first: Some(1.0),
            last: Some(2.0),
        },
        ..LayoutOutput::HIDDEN
    };
    output.oof_candidates.push(candidate(
        static_position(2.0, 4.0, AxisStaticEdge::Start),
        static_position(10.0, 30.0, AxisStaticEdge::End),
    ));
    output
}

#[test]
fn a_container_output_swaps_the_sizes_for_either_block_direction() {
    for right_to_left in BLOCK_DIRECTIONS {
        let output = AxisAdapter::new(right_to_left).container_output(output_on_layout_axes());
        assert_eq!(
            output.size,
            Size {
                width: 120.0,
                height: 180.0
            }
        );
        // The baselines are offsets along the block axis on both sides.
        assert_eq!(
            output.baselines,
            Baselines {
                first: Some(1.0),
                last: Some(2.0)
            }
        );
    }
}

/// An adapter for a container whose borders on the layout axes are 1 and 2
/// on the inline sides and 3 and 4 on the block sides.
fn bordered_adapter(right_to_left: bool, inline_reversed: bool) -> AxisAdapter {
    AxisAdapter {
        right_to_left,
        inline_reversed,
        container_border: rect(1.0, 2.0, 3.0, 4.0),
    }
}

/// A container output of 180 along the inline axis and 120 along the block
/// axis, whose scrollable overflow starts 5 before the inline start and 10
/// before the block start, reaches 30 along the inline axis, and ends 140
/// along the block axis, past the padding box.
fn overflowing_output() -> LayoutOutput {
    LayoutOutput {
        size: Size {
            width: 180.0,
            height: 120.0,
        },
        scrollable_overflow_rect: rect(-5.0, 30.0, -10.0, 140.0),
        ..LayoutOutput::HIDDEN
    }
}

#[test]
fn a_container_overflow_keeps_its_offsets_from_the_left_in_vertical_lr() {
    let output = AxisAdapter::new(false).container_output(overflowing_output());
    // The block axis is the physical x axis, in the same direction.
    assert_eq!(
        output.scrollable_overflow_rect,
        rect(-10.0, 140.0, -5.0, 30.0)
    );
}

#[test]
fn a_container_overflow_is_mirrored_within_the_padding_box_in_vertical_rl() {
    // The padding box is 120 - 3 - 4 = 113 wide along the block axis. The
    // overflow that ends 140 from the block start is 27 to the left of the
    // box, and the 10 before the block start reaches 10 past the physical
    // right edge.
    let output = bordered_adapter(true, false).container_output(overflowing_output());
    assert_eq!(
        output.scrollable_overflow_rect,
        rect(-27.0, 123.0, -5.0, 30.0)
    );
    // Without borders the padding box is the border box.
    let output = AxisAdapter::new(true).container_output(overflowing_output());
    assert_eq!(
        output.scrollable_overflow_rect,
        rect(-20.0, 130.0, -5.0, 30.0)
    );
}

#[test]
fn a_container_overflow_is_mirrored_along_the_inline_axis_when_it_is_reversed() {
    // The padding box is 180 - 1 - 2 = 177 high along the inline axis, and
    // the overflow is measured from its bottom.
    let output = bordered_adapter(false, true).container_output(overflowing_output());
    assert_eq!(
        output.scrollable_overflow_rect,
        rect(-10.0, 140.0, 147.0, 182.0)
    );
    let output = bordered_adapter(true, true).container_output(overflowing_output());
    assert_eq!(
        output.scrollable_overflow_rect,
        rect(-27.0, 123.0, 147.0, 182.0)
    );
}

#[test]
fn an_overflow_rect_that_is_zero_stays_zero() {
    let output = LayoutOutput {
        scrollable_overflow_rect: Edges::ZERO,
        ..overflowing_output()
    };
    for right_to_left in BLOCK_DIRECTIONS {
        let output = bordered_adapter(right_to_left, true).container_output(output.clone());
        assert_eq!(output.scrollable_overflow_rect, Edges::ZERO);
    }
}

#[test]
fn an_output_for_a_single_axis_request_has_nothing_to_mirror() {
    // Taffy answers a request for the inline axis alone with that size only:
    // no out-of-flow data, and a zero block size.
    let size_only = LayoutOutput::from_outer_size(Size {
        width: 180.0,
        height: 0.0,
    });
    for right_to_left in BLOCK_DIRECTIONS {
        let output = AxisAdapter::new(right_to_left).container_output(size_only.clone());
        assert_eq!(
            output.size,
            Size {
                width: 0.0,
                height: 180.0
            }
        );
        assert_eq!(output.oof_positioning_area, None);
        assert!(output.oof_candidates.as_slice().is_empty());
    }
}

#[test]
fn the_positioning_area_returns_to_physical_axes_in_vertical_lr() {
    let output = AxisAdapter::new(false).container_output(output_on_layout_axes());
    assert_eq!(
        output.oof_positioning_area,
        Some(OofPositioningArea {
            size: Size {
                width: 100.0,
                height: 180.0
            },
            offset: Point { x: 7.0, y: 5.0 }
        })
    );
}

#[test]
fn the_positioning_area_is_mirrored_within_the_block_extent_in_vertical_rl() {
    // Logical size (180, 100) at offset (5, 7) in a block extent of 120:
    // the area is 100 wide and ends 7 before the block-end side, so it
    // starts 120 - 7 - 100 = 13 from the physical left edge.
    let output = AxisAdapter::new(true).container_output(output_on_layout_axes());
    assert_eq!(
        output.oof_positioning_area,
        Some(OofPositioningArea {
            size: Size {
                width: 100.0,
                height: 180.0
            },
            offset: Point { x: 13.0, y: 5.0 }
        })
    );
}

#[test]
fn a_container_output_keeps_no_margins_to_collapse() {
    for right_to_left in BLOCK_DIRECTIONS {
        let output = AxisAdapter::new(right_to_left).container_output(output_on_layout_axes());
        assert_eq!(output.top_margin, CollapsibleMarginSet::ZERO);
        assert_eq!(output.bottom_margin, CollapsibleMarginSet::ZERO);
        assert!(!output.margins_can_collapse_through);
    }
}

#[test]
fn static_positions_follow_their_axes_and_only_the_block_axis_is_mirrored() {
    let left_to_right = AxisAdapter::new(false).container_output(output_on_layout_axes());
    let placed = left_to_right.oof_candidates.as_slice()[0].static_position;
    // The block axis becomes the physical x axis, the inline axis the y axis.
    assert_eq!(placed.x, static_position(10.0, 30.0, AxisStaticEdge::End));
    assert_eq!(placed.y, static_position(2.0, 4.0, AxisStaticEdge::Start));

    let right_to_left = AxisAdapter::new(true).container_output(output_on_layout_axes());
    let placed = right_to_left.oof_candidates.as_slice()[0].static_position;
    // 120 - 30 and 120 - 10; the end alignment becomes a start alignment
    // and the start fallback an end fallback.
    assert_eq!(
        placed.x,
        AxisStaticPosition {
            area: Line {
                start: 90.0,
                end: 110.0
            },
            align: AxisStaticAlign {
                keyword: AxisStaticEdge::Start,
                safety: AlignmentSafety::Default,
                fallback: AxisStaticEdge::End,
            },
        }
    );
    assert_eq!(placed.y, static_position(2.0, 4.0, AxisStaticEdge::Start));
}

#[test]
fn mirroring_a_static_position_keeps_center_alignment_and_safety() {
    let mut output = output_on_layout_axes();
    output.oof_candidates.as_mut_slice()[0].static_position.y = AxisStaticPosition {
        area: Line {
            start: 10.0,
            end: 40.0,
        },
        align: AxisStaticAlign {
            keyword: AxisStaticEdge::Center,
            safety: AlignmentSafety::Safe,
            fallback: AxisStaticEdge::End,
        },
    };
    let output = AxisAdapter::new(true).container_output(output);
    let placed = output.oof_candidates.as_slice()[0].static_position.x;
    assert_eq!(
        placed.area,
        Line {
            start: 80.0,
            end: 110.0
        }
    );
    assert_eq!(placed.align.keyword, AxisStaticEdge::Center);
    assert_eq!(placed.align.safety, AlignmentSafety::Safe);
    assert_eq!(placed.align.fallback, AxisStaticEdge::Start);
}

#[test]
fn the_inline_axis_is_never_mirrored() {
    // An item taffy placed flush with the inline end (the far end of the
    // inline axis, which is where a reversed inline direction puts the first
    // item) stays at the far end of the physical y axis.
    let on_layout_axes = BoxLayout {
        location: Point { x: 150.0, y: 8.0 },
        size: Size {
            width: 30.0,
            height: 20.0,
        },
        ..BoxLayout::new()
    };
    for right_to_left in BLOCK_DIRECTIONS {
        let placed = AxisAdapter::new(right_to_left).placed_layout(&on_layout_axes, 100.0);
        assert_eq!(placed.location.y, 150.0, "right_to_left: {right_to_left}");
        assert_eq!(placed.size.height, 30.0);
    }
}

#[test]
fn a_placed_layout_starts_from_the_left_edge_in_vertical_lr() {
    let on_layout_axes = BoxLayout {
        order: 3,
        location: Point { x: 20.0, y: 8.0 },
        size: Size {
            width: 70.0,
            height: 30.0,
        },
        scrollable_overflow_rect: rect(0.0, 5.0, 0.0, 6.0),
        scrollbar_size: Size {
            width: 1.0,
            height: 2.0,
        },
        // Inline start and end are the layout left and right sides, block
        // start and end the top and bottom.
        border: rect(1.0, 2.0, 3.0, 4.0),
        padding: rect(5.0, 6.0, 7.0, 8.0),
        margin: rect(9.0, 10.0, 11.0, 12.0),
    };

    let placed = AxisAdapter::new(false).placed_layout(&on_layout_axes, 100.0);

    assert_eq!(placed.order, 3);
    assert_eq!(placed.location, Point { x: 8.0, y: 20.0 });
    assert_eq!(
        placed.size,
        Size {
            width: 30.0,
            height: 70.0
        }
    );
    assert_eq!(placed.scrollable_overflow_rect, rect(0.0, 6.0, 0.0, 5.0));
    assert_eq!(
        placed.scrollbar_size,
        Size {
            width: 2.0,
            height: 1.0
        }
    );
    // The block-start side is the physical left, the inline start the top.
    assert_eq!(placed.border, rect(3.0, 4.0, 1.0, 2.0));
    assert_eq!(placed.padding, rect(7.0, 8.0, 5.0, 6.0));
    assert_eq!(placed.margin, rect(11.0, 12.0, 9.0, 10.0));
}

#[test]
fn a_placed_layout_starts_from_the_right_edge_in_vertical_rl() {
    let on_layout_axes = BoxLayout {
        order: 3,
        location: Point { x: 20.0, y: 8.0 },
        size: Size {
            width: 70.0,
            height: 30.0,
        },
        border: rect(1.0, 2.0, 3.0, 4.0),
        padding: rect(5.0, 6.0, 7.0, 8.0),
        margin: rect(9.0, 10.0, 11.0, 12.0),
        ..BoxLayout::new()
    };

    let placed = AxisAdapter::new(true).placed_layout(&on_layout_axes, 100.0);

    // 100 - 8 - 30: the container's physical width, less the block offset
    // and the physical width of the child.
    assert_eq!(placed.location, Point { x: 62.0, y: 20.0 });
    assert_eq!(
        placed.size,
        Size {
            width: 30.0,
            height: 70.0
        }
    );
    // The block-start side is the physical right, the inline start the top.
    assert_eq!(placed.border, rect(4.0, 3.0, 1.0, 2.0));
    assert_eq!(placed.padding, rect(8.0, 7.0, 5.0, 6.0));
    assert_eq!(placed.margin, rect(12.0, 11.0, 9.0, 10.0));
}

#[test]
fn a_child_output_enters_the_layout_axes_and_leaves_them_unchanged() {
    for right_to_left in BLOCK_DIRECTIONS {
        let adapter = AxisAdapter::new(right_to_left);
        let mut physical = LayoutOutput {
            size: Size {
                width: 120.0,
                height: 180.0,
            },
            scrollable_overflow_rect: rect(0.0, 9.0, 0.0, 15.0),
            top_margin: CollapsibleMarginSet::from_margin(3.0),
            margins_can_collapse_through: true,
            oof_positioning_area: Some(OofPositioningArea {
                size: Size {
                    width: 100.0,
                    height: 180.0,
                },
                offset: Point { x: 13.0, y: 5.0 },
            }),
            ..LayoutOutput::HIDDEN
        };
        physical.oof_candidates.push(candidate(
            static_position(1.0, 3.0, AxisStaticEdge::Start),
            static_position(20.0, 50.0, AxisStaticEdge::Center),
        ));

        let on_layout_axes = adapter.child_output(physical.clone(), Edges::ZERO, 3.5);
        // The algorithm aligns the child by the baseline it is given.
        assert_eq!(
            on_layout_axes.baselines,
            Baselines {
                first: Some(3.5),
                last: None
            }
        );

        assert_eq!(
            on_layout_axes.size,
            Size {
                width: 180.0,
                height: 120.0
            }
        );
        // No margin of the child collapses through the container.
        assert_eq!(on_layout_axes.top_margin, CollapsibleMarginSet::ZERO);
        assert!(!on_layout_axes.margins_can_collapse_through);
        // The child's own width is the extent: 120 - 13 - 100 = 7 from the
        // block start in `vertical-rl`.
        let area = on_layout_axes.oof_positioning_area.unwrap();
        assert_eq!(
            area.size,
            Size {
                width: 180.0,
                height: 100.0
            }
        );
        assert_eq!(
            area.offset,
            Point {
                x: 5.0,
                y: if right_to_left { 7.0 } else { 13.0 }
            }
        );

        let back = adapter.container_output(on_layout_axes);
        assert_eq!(back.size, physical.size);
        assert_eq!(
            back.scrollable_overflow_rect,
            physical.scrollable_overflow_rect
        );
        assert_eq!(back.oof_positioning_area, physical.oof_positioning_area);
        assert_eq!(
            back.oof_candidates.as_slice(),
            physical.oof_candidates.as_slice()
        );
    }
}

/// The inputs a flex container gives an item of a container that is 200 wide
/// along its inline axis and 300 along its block axis.
fn item_inputs() -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size {
            width: Some(200.0),
            height: Some(300.0),
        },
        available_space: Size {
            width: AvailableSpace::Definite(200.0),
            height: AvailableSpace::Definite(300.0),
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

/// Lay out the only child of a container in `vertical-rl`, through the
/// adapter, while the container's frame holds the child.
fn adapted_item_output(child_css: &str, flows_horizontally: bool) -> LayoutOutput {
    let (mut doc, cascade, child, _) = fixture_with_styles("display:block", child_css);
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let frame = TransposedBlock::new(&mut tree, parent, vec![child], true);
    let (output, _) = with_transposed_frame(&mut tree, frame, |tree| {
        let output = tree
            .compute_adapted_child(NodeId::from(child), item_inputs(), flows_horizontally)
            .expect("the frame holds the child");
        assert_eq!(tree.context.transposed_blocks.len(), 1);
        output
    });
    output
}

#[test]
fn a_child_measures_itself_on_the_axes_of_its_own_writing_mode() {
    // 31 wide and 7 tall on the physical axes, so 7 along the container's
    // inline axis and 31 along its block axis. The container's transposed
    // view would have read the sizes the other way round.
    let output = adapted_item_output("display:block;width:31px;height:7px", true);
    assert_eq!(
        output.size,
        Size {
            width: 7.0,
            height: 31.0
        }
    );
}

#[test]
fn a_horizontal_child_resolves_percentage_padding_against_the_inline_size() {
    // 10% of the container's inline size, 200, is 20 on every side.
    let output = adapted_item_output("display:block;width:31px;height:7px;padding:10%", true);
    assert_eq!(
        output.size,
        Size {
            width: 7.0 + 40.0,
            height: 31.0 + 40.0
        }
    );
}

#[test]
fn a_child_on_vertical_axes_is_left_to_find_the_inline_size_itself() {
    // Without the basis, taffy resolves the padding against the width of the
    // parent on the child's physical axes, the container's block size, 300.
    let output = adapted_item_output("display:block;width:31px;height:7px;padding:10%", false);
    assert_eq!(
        output.size,
        Size {
            width: 7.0 + 60.0,
            height: 31.0 + 60.0
        }
    );
}

#[test]
fn only_the_container_that_holds_the_child_adapts_it() {
    let (mut doc, cascade, child, _) = fixture_with_styles("display:block", "width:31px");
    let parent = doc.nodes[child].parent.unwrap();
    let other = doc.nodes[parent].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let call = |tree: &mut LayoutSession<'_>, node: usize| {
        tree.compute_adapted_child(NodeId::from(node), item_inputs(), true)
    };
    assert!(call(&mut tree, child).is_none(), "no frame is pushed");
    let frame = TransposedBlock::new(&mut tree, parent, vec![child], false);
    let ((for_other, for_child), _) = with_transposed_frame(&mut tree, frame, |tree| {
        (call(tree, other), call(tree, child))
    });
    assert!(for_other.is_none(), "the frame holds only the child");
    assert!(for_child.is_some());
}

#[test]
fn a_child_output_is_mirrored_for_the_direction_of_the_frame() {
    // A child with an out-of-flow descendant reports where that descendant
    // would sit: at the start of the child's physical width, 31 wide here,
    // which is the block start of the container on the left, but its end on
    // the right.
    let static_block_position = |right_to_left| {
        let mut doc = crate::Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block"));
        let child = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:31px;height:7px"),
        );
        doc.append_element(
            Some(child),
            "div",
            Some("position:absolute;width:5px;height:5px"),
        );
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let frame = TransposedBlock::new(&mut tree, body, vec![child], right_to_left);
        let (output, _) = with_transposed_frame(&mut tree, frame, |tree| {
            tree.compute_adapted_child(NodeId::from(child), item_inputs(), true)
                .expect("the frame holds the child")
        });
        // The child's physical block axis is the layout y axis.
        output.oof_candidates.as_slice()[0].static_position.y
    };

    let left_to_right = static_block_position(false);
    assert_eq!(
        left_to_right.area,
        Line {
            start: 0.0,
            end: 0.0
        }
    );
    assert_eq!(left_to_right.align.keyword, AxisStaticEdge::Start);
    let right_to_left = static_block_position(true);
    assert_eq!(
        right_to_left.area,
        Line {
            start: 31.0,
            end: 31.0
        }
    );
    assert_eq!(right_to_left.align.keyword, AxisStaticEdge::End);
}

#[test]
fn a_placed_layout_mirrors_the_overflow_within_the_padding_box_of_the_child() {
    // 70 by 30 on the layout axes with borders of 1 and 2 on the inline sides
    // and 3 and 4 on the block sides: a padding box of 67 by 23.
    let on_layout_axes = BoxLayout {
        size: Size {
            width: 70.0,
            height: 30.0,
        },
        border: rect(1.0, 2.0, 3.0, 4.0),
        scrollable_overflow_rect: rect(-5.0, 20.0, -2.0, 40.0),
        ..BoxLayout::new()
    };

    let left_to_right = AxisAdapter::new(false).placed_layout(&on_layout_axes, 100.0);
    assert_eq!(
        left_to_right.scrollable_overflow_rect,
        rect(-2.0, 40.0, -5.0, 20.0)
    );
    // The overflow 17 past the block end of the child lies to its left.
    let right_to_left = AxisAdapter::new(true).placed_layout(&on_layout_axes, 100.0);
    assert_eq!(
        right_to_left.scrollable_overflow_rect,
        rect(23.0 - 40.0, 23.0 + 2.0, -5.0, 20.0)
    );
    let reversed = bordered_adapter(true, true).placed_layout(&on_layout_axes, 100.0);
    assert_eq!(
        reversed.scrollable_overflow_rect,
        rect(23.0 - 40.0, 23.0 + 2.0, 67.0 - 20.0, 67.0 + 5.0)
    );
}

#[test]
fn a_child_overflow_goes_to_the_layout_axes_and_back_through_the_placed_layout() {
    // A child that is 31 wide and 7 high with borders of 2, 3, 5 and 1 on its
    // left, right, top and bottom sides, and overflow in physical
    // coordinates that lies past its right edge and below its bottom edge.
    let border = rect(2.0, 3.0, 5.0, 1.0);
    let physical = LayoutOutput {
        size: Size {
            width: 31.0,
            height: 7.0,
        },
        scrollable_overflow_rect: rect(-4.0, 50.0, -1.0, 20.0),
        ..LayoutOutput::HIDDEN
    };
    for (right_to_left, inline_reversed) in [(false, false), (true, false), (true, true)] {
        let adapter = bordered_adapter(right_to_left, inline_reversed);
        let on_layout_axes = adapter.child_output(physical.clone(), border, 0.0);
        // The padding box is 26 wide, along the block axis, and 1 high,
        // along the inline axis.
        let expected = match (right_to_left, inline_reversed) {
            (false, false) => rect(-1.0, 20.0, -4.0, 50.0),
            (true, false) => rect(-1.0, 20.0, 26.0 - 50.0, 26.0 + 4.0),
            (_, _) => rect(1.0 - 20.0, 1.0 + 1.0, 26.0 - 50.0, 26.0 + 4.0),
        };
        assert_eq!(on_layout_axes.scrollable_overflow_rect, expected);

        let placed = adapter.placed_layout(
            &BoxLayout {
                size: on_layout_axes.size,
                border: rect(border.top, border.bottom, border.left, border.right),
                scrollable_overflow_rect: on_layout_axes.scrollable_overflow_rect,
                ..BoxLayout::new()
            },
            100.0,
        );
        assert_eq!(
            placed.scrollable_overflow_rect, physical.scrollable_overflow_rect,
            "right_to_left: {right_to_left}, inline_reversed: {inline_reversed}"
        );
    }
}

/// The adapter built for a frame of a container styled `container_css`,
/// whose block axis runs from right to left when `right_to_left` is set.
fn adapter_for_container(container_css: &str, right_to_left: bool) -> AxisAdapter {
    let (mut doc, cascade, child, _) = fixture_with_styles(container_css, "display:block");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let frame = TransposedBlock::new(&mut tree, parent, vec![child], right_to_left);
    let (adapter, _) = with_transposed_frame(&mut tree, frame, |tree| {
        AxisAdapter::for_frame(tree.context.transposed_blocks.last().unwrap())
    });
    adapter
}

#[test]
fn a_frame_gives_the_borders_of_its_container_on_the_layout_axes() {
    // The inline sides of a vertical container are its physical top and
    // bottom, the block sides its physical right and left.
    let adapter = adapter_for_container(
        "display:block;writing-mode:vertical-rl;border-style:solid;\
         border-top-width:3px;border-right-width:5px;\
         border-bottom-width:7px;border-left-width:11px",
        true,
    );
    let border = adapter.container_border;
    assert_eq!(border.left + border.right, 3.0 + 7.0);
    assert_eq!(border.top + border.bottom, 5.0 + 11.0);
}

#[test]
fn a_frame_reads_a_reversed_inline_direction_from_its_container() {
    let inline_reversed =
        |css: &str, right_to_left| adapter_for_container(css, right_to_left).inline_reversed;
    assert!(!inline_reversed("writing-mode:vertical-rl", true));
    assert!(!inline_reversed("writing-mode:vertical-lr", false));
    assert!(inline_reversed(
        "writing-mode:vertical-rl;direction:rtl",
        true
    ));
    assert!(inline_reversed(
        "writing-mode:vertical-lr;direction:rtl",
        false
    ));
    // `sideways-lr` runs its lines from the bottom to the top.
    assert!(inline_reversed("writing-mode:sideways-lr", false));
    assert!(!inline_reversed("writing-mode:sideways-rl", true));
}

/// The scrollable overflow rectangle that a child styled `child_css`, with a
/// 50 by 20 box in it, hands to a container whose block axis runs from right
/// to left when `right_to_left` is set.
fn laid_out_child_overflow(child_css: &str, right_to_left: bool) -> Edges<f32> {
    let mut doc = crate::Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let child = doc.append_element(Some(body), "div", Some(child_css));
    doc.append_element(
        Some(child),
        "div",
        Some("display:block;width:50px;height:20px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let frame = TransposedBlock::new(&mut tree, body, vec![child], right_to_left);
    let (output, _) = with_transposed_frame(&mut tree, frame, |tree| {
        tree.compute_adapted_child(NodeId::from(child), item_inputs(), true)
            .expect("the frame holds the child")
    });
    output.scrollable_overflow_rect
}

#[test]
fn the_overflow_of_a_laid_out_child_is_mirrored_within_its_padding_box() {
    // A 31 by 7 child with a 50 by 20 box in it: in physical coordinates its
    // overflow reaches 50 to the right and 20 down. In `vertical-rl` the right
    // edge of the child is its block start, so the 19 that lie past it are
    // before the block start: the layout rectangle starts 19 above the top.
    // The inline axis is the physical y axis, as it is.
    let child_css = "display:block;width:31px;height:7px";
    assert_eq!(
        laid_out_child_overflow(child_css, false),
        rect(0.0, 20.0, 0.0, 50.0)
    );
    assert_eq!(
        laid_out_child_overflow(child_css, true),
        rect(0.0, 20.0, 31.0 - 50.0, 31.0)
    );
}

#[test]
fn the_borders_of_a_laid_out_child_stay_out_of_the_extent_it_is_mirrored_within() {
    // The 2px borders widen the border box to 35, but the overflow is
    // measured from the padding box, 31 wide. The rectangle reaches 2 before
    // the padding box on the physical left and top, where the borders lie.
    let child_css = "display:block;width:31px;height:7px;border:2px solid";
    assert_eq!(
        laid_out_child_overflow(child_css, true),
        rect(-2.0, 20.0, 31.0 - 50.0, 31.0 + 2.0)
    );
}
