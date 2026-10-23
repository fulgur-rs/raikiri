use super::*;
use taffy::tree::RunMode;
use taffy::{
    AlignmentSafety, AvailableSpace, AxisStaticAlign, Baselines, NodeId, OofCandidate,
    OofPositioningArea, Position, SizingMode,
};

fn rect(left: f32, right: f32, top: f32, bottom: f32) -> Rect<f32> {
    Rect {
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

#[test]
fn reversing_a_center_static_position_keeps_its_alignment_centered() {
    let position = taffy::AxisStaticPosition {
        area: taffy::Line {
            start: 10.0,
            end: 40.0,
        },
        align: taffy::AxisStaticAlign {
            keyword: taffy::AxisStaticEdge::Center,
            safety: taffy::AlignmentSafety::Safe,
            fallback: taffy::AxisStaticEdge::End,
        },
    };

    let reversed = super::reverse_static_position(position, 100.0);

    assert_eq!(
        reversed.area,
        taffy::Line {
            start: 60.0,
            end: 90.0
        }
    );
    assert_eq!(reversed.align.keyword, taffy::AxisStaticEdge::Center);
    assert_eq!(reversed.align.safety, taffy::AlignmentSafety::Safe);
    assert_eq!(reversed.align.fallback, taffy::AxisStaticEdge::Start);
}

#[test]
fn reversing_start_and_end_swaps_them_and_mirrors_the_area() {
    let reversed =
        reverse_static_position(static_position(10.0, 40.0, AxisStaticEdge::Start), 100.0);
    assert_eq!(reversed.align.keyword, AxisStaticEdge::End);
    assert_eq!(reversed.align.fallback, AxisStaticEdge::End);
    assert_eq!(
        reversed.area,
        Line {
            start: 60.0,
            end: 90.0
        }
    );
    let again = reverse_static_position(reversed, 100.0);
    assert_eq!(
        again.area,
        Line {
            start: 10.0,
            end: 40.0
        }
    );
    assert_eq!(again.align.keyword, AxisStaticEdge::Start);
}

#[test]
fn a_rect_survives_the_trip_to_the_transposed_axes_and_back() {
    let physical = rect(1.0, 2.0, 3.0, 4.0);
    for right_to_left in [false, true] {
        assert_eq!(
            physical_rect(transpose_rect(physical, right_to_left), right_to_left),
            physical
        );
    }
}

#[test]
fn the_block_start_side_becomes_the_transposed_top() {
    let physical = rect(1.0, 2.0, 3.0, 4.0);
    // vertical-lr starts at the left edge, vertical-rl at the right edge; the
    // physical top and bottom become the transposed left and right.
    assert_eq!(transpose_rect(physical, false), rect(3.0, 4.0, 1.0, 2.0));
    assert_eq!(transpose_rect(physical, true), rect(3.0, 4.0, 2.0, 1.0));
}

#[test]
fn swapping_a_rect_of_axes_twice_returns_it() {
    let overflow = rect(-1.0, 8.0, -2.0, 9.0);
    assert_eq!(swap_rect_axes(overflow), rect(-2.0, 9.0, -1.0, 8.0));
    assert_eq!(swap_rect_axes(swap_rect_axes(overflow)), overflow);
}

fn sample_inputs() -> LayoutInput {
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
fn transposing_inputs_swaps_every_per_axis_field() {
    let swapped = transpose_inputs(sample_inputs(), true);
    assert_eq!(swapped.axis, RequestedAxis::Vertical);
    assert_eq!(
        swapped.known_dimensions,
        Size {
            width: None,
            height: Some(1.0)
        }
    );
    assert_eq!(
        swapped.known_dimensions_are_definite,
        Size {
            width: false,
            height: true
        }
    );
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

#[test]
fn requesting_both_axes_stays_both_and_a_vertical_request_turns_horizontal() {
    for (before, after) in [
        (RequestedAxis::Both, RequestedAxis::Both),
        (RequestedAxis::Vertical, RequestedAxis::Horizontal),
    ] {
        let inputs = LayoutInput {
            axis: before,
            ..sample_inputs()
        };
        assert_eq!(transpose_inputs(inputs, true).axis, after);
    }
}

#[test]
fn margin_collapsing_survives_only_inside_one_block_formatting_context() {
    let kept = transpose_inputs(sample_inputs(), true);
    assert_eq!(kept.vertical_margins_are_collapsible, Line::TRUE);
    let dropped = transpose_inputs(sample_inputs(), false);
    assert_eq!(dropped.vertical_margins_are_collapsible, Line::FALSE);
}

#[test]
fn transposing_the_inputs_twice_returns_them() {
    let inputs = sample_inputs();
    let back = transpose_inputs(transpose_inputs(inputs, true), true);
    assert_eq!(back.axis, inputs.axis);
    assert_eq!(back.known_dimensions, inputs.known_dimensions);
    assert_eq!(
        back.known_dimensions_are_definite,
        inputs.known_dimensions_are_definite
    );
    assert_eq!(back.parent_size, inputs.parent_size);
    assert_eq!(back.available_space, inputs.available_space);
}

fn transposed_output() -> LayoutOutput {
    let mut output = LayoutOutput {
        size: Size {
            width: 180.0,
            height: 120.0,
        },
        scrollable_overflow_rect: rect(0.0, 11.0, 0.0, 13.0),
        top_margin: CollapsibleMarginSet::from_margin(4.0),
        bottom_margin: CollapsibleMarginSet::from_margin(6.0),
        margins_can_collapse_through: true,
        oof_positioning_area: Some(taffy::OofPositioningArea {
            size: Size {
                width: 180.0,
                height: 100.0,
            },
            offset: Point { x: 5.0, y: 7.0 },
        }),
        baselines: Baselines {
            first: Some(3.0),
            last: None,
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
fn a_transposed_output_returns_to_physical_axes_from_the_block_start_edge() {
    let output = transpose_output_to_physical(transposed_output(), false, 120.0, true);
    assert_eq!(
        output.size,
        Size {
            width: 120.0,
            height: 180.0
        }
    );
    assert_eq!(output.scrollable_overflow_rect, rect(0.0, 13.0, 0.0, 11.0));
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
    let placed = output.oof_candidates.as_slice()[0].static_position;
    assert_eq!(placed.x, static_position(10.0, 30.0, AxisStaticEdge::End));
    assert_eq!(placed.y, static_position(2.0, 4.0, AxisStaticEdge::Start));
    // A baseline is an offset along the block axis on both sides.
    assert_eq!(
        output.baselines,
        Baselines {
            first: Some(3.0),
            last: None
        }
    );
}

#[test]
fn a_right_to_left_output_mirrors_the_block_axis_within_the_extent() {
    let output = transpose_output_to_physical(transposed_output(), true, 120.0, true);
    // 120 - 7 - 100: the area ends 7 before the block-end side and 13 from
    // the physical left edge.
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
    let placed = output.oof_candidates.as_slice()[0].static_position;
    assert_eq!(
        placed.x,
        reverse_static_position(static_position(10.0, 30.0, AxisStaticEdge::End), 120.0)
    );
    assert_eq!(placed.y, static_position(2.0, 4.0, AxisStaticEdge::Start));
    assert_eq!(placed.x.align.keyword, AxisStaticEdge::Start);
}

#[test]
fn margins_leave_a_converted_output_unless_the_formatting_context_is_shared() {
    let kept = transpose_output_to_physical(transposed_output(), false, 120.0, true);
    assert_eq!(kept.top_margin, CollapsibleMarginSet::from_margin(4.0));
    assert_eq!(kept.bottom_margin, CollapsibleMarginSet::from_margin(6.0));
    assert!(kept.margins_can_collapse_through);

    let dropped = transpose_output_to_physical(transposed_output(), false, 120.0, false);
    assert_eq!(dropped.top_margin, CollapsibleMarginSet::ZERO);
    assert_eq!(dropped.bottom_margin, CollapsibleMarginSet::ZERO);
    assert!(!dropped.margins_can_collapse_through);
}

#[test]
fn a_child_output_round_trips_through_the_transposed_axes() {
    for right_to_left in [false, true] {
        let mut physical = LayoutOutput {
            size: Size {
                width: 120.0,
                height: 180.0,
            },
            scrollable_overflow_rect: rect(0.0, 9.0, 0.0, 15.0),
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

        let transposed =
            transpose_child_output_to_vertical(physical.clone(), right_to_left, 120.0, false);
        assert_eq!(
            transposed.size,
            Size {
                width: 180.0,
                height: 120.0
            }
        );
        let back = transpose_output_to_physical(transposed, right_to_left, 120.0, false);

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

#[test]
fn a_placed_child_gets_its_physical_layout_from_the_block_start_edge() {
    let transposed = Layout {
        order: 3,
        location: Point { x: 8.0, y: 20.0 },
        size: Size {
            width: 70.0,
            height: 30.0,
        },
        scrollable_overflow_rect: rect(0.0, 5.0, 0.0, 6.0),
        scrollbar_size: Size {
            width: 1.0,
            height: 2.0,
        },
        border: rect(1.0, 2.0, 3.0, 4.0),
        padding: rect(5.0, 6.0, 7.0, 8.0),
        margin: rect(9.0, 10.0, 11.0, 12.0),
    };

    let left_to_right = physical_layout(&transposed, 100.0, false);
    assert_eq!(left_to_right.order, 3);
    assert_eq!(left_to_right.location, Point { x: 20.0, y: 8.0 });
    assert_eq!(
        left_to_right.size,
        Size {
            width: 30.0,
            height: 70.0
        }
    );
    assert_eq!(
        left_to_right.scrollable_overflow_rect,
        rect(0.0, 6.0, 0.0, 5.0)
    );
    assert_eq!(
        left_to_right.scrollbar_size,
        Size {
            width: 2.0,
            height: 1.0
        }
    );
    assert_eq!(
        left_to_right.border,
        physical_rect(transposed.border, false)
    );
    assert_eq!(
        left_to_right.padding,
        physical_rect(transposed.padding, false)
    );
    assert_eq!(
        left_to_right.margin,
        physical_rect(transposed.margin, false)
    );

    // 100 - 20 - 30: the block-start edge is the physical right edge.
    let right_to_left = physical_layout(&transposed, 100.0, true);
    assert_eq!(right_to_left.location, Point { x: 50.0, y: 8.0 });
    assert_eq!(right_to_left.border, physical_rect(transposed.border, true));
}

/// The four ways the layout axes can run against the physical ones.
const OVERFLOW_ORIENTATIONS: [(bool, bool); 4] =
    [(false, false), (true, false), (false, true), (true, true)];

/// A padding box 80 along the inline axis and 60 along the block axis.
fn overflow_frame(block_reversed: bool, inline_reversed: bool) -> OverflowFrame {
    OverflowFrame {
        block_reversed,
        inline_reversed,
        padding_box: Size {
            width: 80.0,
            height: 60.0,
        },
    }
}

/// The physical position of a point at `(inline, block)` from the scroll
/// origin of the padding box of `overflow_frame`.
fn physical_point(frame: OverflowFrame, inline: f32, block: f32) -> (f32, f32) {
    let x = if frame.block_reversed {
        frame.padding_box.height - block
    } else {
        block
    };
    let y = if frame.inline_reversed {
        frame.padding_box.width - inline
    } else {
        inline
    };
    (x, y)
}

#[test]
fn overflow_in_vertical_lr_swaps_the_axes() {
    let frame = overflow_frame(false, false);
    assert_eq!(
        frame.physical(rect(-5.0, 30.0, -10.0, 75.0)),
        rect(-10.0, 75.0, -5.0, 30.0)
    );
}

#[test]
fn overflow_past_the_block_end_of_vertical_rl_lies_left_of_the_box() {
    let frame = overflow_frame(true, false);
    // The block axis is 60 long. The overflow reaches 75 from the block
    // start, 15 past the block end, which is the physical left edge; the 10
    // before the block start lie past the physical right edge.
    assert_eq!(
        frame.physical(rect(-5.0, 30.0, -10.0, 75.0)),
        rect(-15.0, 70.0, -5.0, 30.0)
    );
}

#[test]
fn overflow_past_the_inline_end_of_a_reversed_inline_axis_lies_above_the_box() {
    let frame = overflow_frame(false, true);
    // The inline axis is 80 long and read from the physical bottom: the
    // overflow 95 along it is 15 above the top edge, and the 5 before the
    // inline start lie below the bottom edge.
    assert_eq!(
        frame.physical(rect(-5.0, 95.0, 0.0, 20.0)),
        rect(0.0, 20.0, -15.0, 85.0)
    );
}

#[test]
fn overflow_that_stays_inside_the_padding_box_stays_inside_it() {
    // Content in the first 20 of the 60 along the block axis is at the
    // right-hand end of the box in vertical-rl.
    let frame = overflow_frame(true, false);
    assert_eq!(
        frame.physical(rect(0.0, 30.0, 0.0, 20.0)),
        rect(40.0, 60.0, 0.0, 30.0)
    );
}

#[test]
fn overflow_agrees_with_the_position_of_each_of_its_corners() {
    let overflow = rect(-5.0, 30.0, -10.0, 75.0);
    for (block_reversed, inline_reversed) in OVERFLOW_ORIENTATIONS {
        let frame = overflow_frame(block_reversed, inline_reversed);
        let corners = [
            physical_point(frame, overflow.left, overflow.top),
            physical_point(frame, overflow.right, overflow.top),
            physical_point(frame, overflow.left, overflow.bottom),
            physical_point(frame, overflow.right, overflow.bottom),
        ];
        let min =
            |value: fn(&(f32, f32)) -> f32| corners.iter().map(value).fold(f32::MAX, f32::min);
        let max =
            |value: fn(&(f32, f32)) -> f32| corners.iter().map(value).fold(f32::MIN, f32::max);
        assert_eq!(
            frame.physical(overflow),
            rect(min(|c| c.0), max(|c| c.0), min(|c| c.1), max(|c| c.1)),
            "block reversed: {block_reversed}, inline reversed: {inline_reversed}"
        );
    }
}

#[test]
fn overflow_returns_to_the_layout_axes_by_the_same_mirroring() {
    for rect in [
        rect(-5.0, 30.0, -10.0, 75.0),
        rect(0.0, 80.0, 0.0, 60.0),
        rect(-12.5, 0.0, 0.0, 2.25),
        rect(0.0, 100.0, -3.0, 0.0),
    ] {
        for (block_reversed, inline_reversed) in OVERFLOW_ORIENTATIONS {
            let frame = overflow_frame(block_reversed, inline_reversed);
            assert_eq!(frame.on_layout_axes(frame.physical(rect)), rect);
            assert_eq!(frame.physical(frame.on_layout_axes(rect)), rect);
        }
    }
}

#[test]
fn a_zero_overflow_rect_holds_nothing_to_mirror() {
    for (block_reversed, inline_reversed) in OVERFLOW_ORIENTATIONS {
        let frame = overflow_frame(block_reversed, inline_reversed);
        assert_eq!(frame.physical(Rect::ZERO), Rect::ZERO);
        assert_eq!(frame.on_layout_axes(Rect::ZERO), Rect::ZERO);
    }
}

#[test]
fn the_padding_box_is_the_border_box_less_the_borders() {
    assert_eq!(
        padding_box(
            Size {
                width: 100.0,
                height: 50.0
            },
            rect(1.0, 2.0, 3.0, 4.0)
        ),
        Size {
            width: 97.0,
            height: 43.0
        }
    );
    // Borders larger than the box leave nothing.
    assert_eq!(
        padding_box(
            Size {
                width: 2.0,
                height: 2.0
            },
            rect(3.0, 3.0, 3.0, 3.0)
        ),
        Size::ZERO
    );
}
