//! Every variant of the library's enums that the protocol mirrors.
//!
//! Each `expected_*` function matches on the type it takes without a catch-all
//! arm, so a variant added to either side stops the build here and has to be
//! given a counterpart on purpose.

use super::*;

fn expected_run_mode(mode: taffy::RunMode) -> RunMode {
    match mode {
        taffy::RunMode::PerformLayout => RunMode::PerformLayout,
        taffy::RunMode::ComputeSize => RunMode::ComputeSize,
        taffy::RunMode::PerformHiddenLayout => RunMode::PerformHiddenLayout,
    }
}

fn run_mode_in_taffy(mode: RunMode) -> taffy::RunMode {
    match mode {
        RunMode::PerformLayout => taffy::RunMode::PerformLayout,
        RunMode::ComputeSize => taffy::RunMode::ComputeSize,
        RunMode::PerformHiddenLayout => taffy::RunMode::PerformHiddenLayout,
    }
}

#[test]
fn run_modes_map_one_to_one() {
    for mode in [
        taffy::RunMode::PerformLayout,
        taffy::RunMode::ComputeSize,
        taffy::RunMode::PerformHiddenLayout,
    ] {
        assert_eq!(RunMode::from_taffy(mode), expected_run_mode(mode));
        assert_eq!(expected_run_mode(mode).into_taffy(), mode);
        assert_eq!(run_mode_in_taffy(expected_run_mode(mode)), mode);
    }
}

fn expected_sizing_mode(mode: taffy::SizingMode) -> SizingMode {
    match mode {
        taffy::SizingMode::ContentSize => SizingMode::ContentSize,
        taffy::SizingMode::InherentSize => SizingMode::InherentSize,
    }
}

fn sizing_mode_in_taffy(mode: SizingMode) -> taffy::SizingMode {
    match mode {
        SizingMode::ContentSize => taffy::SizingMode::ContentSize,
        SizingMode::InherentSize => taffy::SizingMode::InherentSize,
    }
}

#[test]
fn sizing_modes_map_one_to_one() {
    for mode in [
        taffy::SizingMode::ContentSize,
        taffy::SizingMode::InherentSize,
    ] {
        assert_eq!(SizingMode::from_taffy(mode), expected_sizing_mode(mode));
        assert_eq!(expected_sizing_mode(mode).into_taffy(), mode);
        assert_eq!(sizing_mode_in_taffy(expected_sizing_mode(mode)), mode);
    }
}

fn expected_requested_axis(axis: taffy::RequestedAxis) -> RequestedAxis {
    match axis {
        taffy::RequestedAxis::Horizontal => RequestedAxis::Horizontal,
        taffy::RequestedAxis::Vertical => RequestedAxis::Vertical,
        taffy::RequestedAxis::Both => RequestedAxis::Both,
    }
}

fn requested_axis_in_taffy(axis: RequestedAxis) -> taffy::RequestedAxis {
    match axis {
        RequestedAxis::Horizontal => taffy::RequestedAxis::Horizontal,
        RequestedAxis::Vertical => taffy::RequestedAxis::Vertical,
        RequestedAxis::Both => taffy::RequestedAxis::Both,
    }
}

#[test]
fn requested_axes_map_one_to_one() {
    for axis in [
        taffy::RequestedAxis::Horizontal,
        taffy::RequestedAxis::Vertical,
        taffy::RequestedAxis::Both,
    ] {
        assert_eq!(
            RequestedAxis::from_taffy(axis),
            expected_requested_axis(axis)
        );
        assert_eq!(expected_requested_axis(axis).into_taffy(), axis);
        assert_eq!(requested_axis_in_taffy(expected_requested_axis(axis)), axis);
    }
}

fn expected_absolute_axis(axis: taffy::AbsoluteAxis) -> AbsoluteAxis {
    match axis {
        taffy::AbsoluteAxis::Horizontal => AbsoluteAxis::Horizontal,
        taffy::AbsoluteAxis::Vertical => AbsoluteAxis::Vertical,
    }
}

fn absolute_axis_in_taffy(axis: AbsoluteAxis) -> taffy::AbsoluteAxis {
    match axis {
        AbsoluteAxis::Horizontal => taffy::AbsoluteAxis::Horizontal,
        AbsoluteAxis::Vertical => taffy::AbsoluteAxis::Vertical,
    }
}

#[test]
fn absolute_axes_map_one_to_one() {
    for axis in [
        taffy::AbsoluteAxis::Horizontal,
        taffy::AbsoluteAxis::Vertical,
    ] {
        assert_eq!(AbsoluteAxis::from_taffy(axis), expected_absolute_axis(axis));
        assert_eq!(expected_absolute_axis(axis).into_taffy(), axis);
        assert_eq!(absolute_axis_in_taffy(expected_absolute_axis(axis)), axis);
        assert_eq!(
            expected_absolute_axis(axis.other_axis()),
            expected_absolute_axis(axis).other_axis()
        );
    }
}

#[test]
fn an_absolute_axis_and_a_requested_axis_agree() {
    for axis in [AbsoluteAxis::Horizontal, AbsoluteAxis::Vertical] {
        assert_eq!(AbsoluteAxis::try_from(RequestedAxis::from(axis)), Ok(axis));
    }
    assert_eq!(AbsoluteAxis::try_from(RequestedAxis::Both), Err(()));
}

fn expected_available_space(space: taffy::AvailableSpace) -> AvailableSpace {
    match space {
        taffy::AvailableSpace::Definite(value) => AvailableSpace::Definite(value),
        taffy::AvailableSpace::MinContent => AvailableSpace::MinContent,
        taffy::AvailableSpace::MaxContent => AvailableSpace::MaxContent,
    }
}

fn available_space_in_taffy(space: AvailableSpace) -> taffy::AvailableSpace {
    match space {
        AvailableSpace::Definite(value) => taffy::AvailableSpace::Definite(value),
        AvailableSpace::MinContent => taffy::AvailableSpace::MinContent,
        AvailableSpace::MaxContent => taffy::AvailableSpace::MaxContent,
    }
}

#[test]
fn available_spaces_map_one_to_one() {
    for space in [
        taffy::AvailableSpace::Definite(12.5),
        taffy::AvailableSpace::MinContent,
        taffy::AvailableSpace::MaxContent,
    ] {
        assert_eq!(
            AvailableSpace::from_taffy(space),
            expected_available_space(space)
        );
        assert_eq!(expected_available_space(space).into_taffy(), space);
        assert_eq!(
            available_space_in_taffy(expected_available_space(space)),
            space
        );
    }
}

fn expected_static_edge(edge: taffy::AxisStaticEdge) -> AxisStaticEdge {
    match edge {
        taffy::AxisStaticEdge::Start => AxisStaticEdge::Start,
        taffy::AxisStaticEdge::Center => AxisStaticEdge::Center,
        taffy::AxisStaticEdge::End => AxisStaticEdge::End,
    }
}

fn static_edge_in_taffy(edge: AxisStaticEdge) -> taffy::AxisStaticEdge {
    match edge {
        AxisStaticEdge::Start => taffy::AxisStaticEdge::Start,
        AxisStaticEdge::Center => taffy::AxisStaticEdge::Center,
        AxisStaticEdge::End => taffy::AxisStaticEdge::End,
    }
}

#[test]
fn static_edges_map_one_to_one() {
    for edge in [
        taffy::AxisStaticEdge::Start,
        taffy::AxisStaticEdge::Center,
        taffy::AxisStaticEdge::End,
    ] {
        assert_eq!(AxisStaticEdge::from_taffy(edge), expected_static_edge(edge));
        assert_eq!(expected_static_edge(edge).into_taffy(), edge);
        assert_eq!(static_edge_in_taffy(expected_static_edge(edge)), edge);
    }
}

fn expected_safety(safety: taffy::AlignmentSafety) -> StaticAlignSafety {
    match safety {
        taffy::AlignmentSafety::Default => StaticAlignSafety::Default,
        taffy::AlignmentSafety::Unsafe => StaticAlignSafety::Unsafe,
        taffy::AlignmentSafety::Safe => StaticAlignSafety::Safe,
    }
}

fn safety_in_taffy(safety: StaticAlignSafety) -> taffy::AlignmentSafety {
    match safety {
        StaticAlignSafety::Default => taffy::AlignmentSafety::Default,
        StaticAlignSafety::Unsafe => taffy::AlignmentSafety::Unsafe,
        StaticAlignSafety::Safe => taffy::AlignmentSafety::Safe,
    }
}

#[test]
fn alignment_safeties_map_one_to_one() {
    for safety in [
        taffy::AlignmentSafety::Default,
        taffy::AlignmentSafety::Unsafe,
        taffy::AlignmentSafety::Safe,
    ] {
        assert_eq!(
            StaticAlignSafety::from_taffy(safety),
            expected_safety(safety)
        );
        assert_eq!(expected_safety(safety).into_taffy(), safety);
        assert_eq!(safety_in_taffy(expected_safety(safety)), safety);
    }
}

/// The positions of the library that a candidate can have, with the owned
/// counterpart of each; the others are in flow and are never emitted.
fn expected_position(position: taffy::Position) -> Option<OofPosition> {
    match position {
        taffy::Position::Absolute => Some(OofPosition::Absolute),
        taffy::Position::Fixed => Some(OofPosition::Fixed),
        taffy::Position::Static | taffy::Position::Relative | taffy::Position::Sticky => None,
    }
}

fn position_in_taffy(position: OofPosition) -> taffy::Position {
    match position {
        OofPosition::Absolute => taffy::Position::Absolute,
        OofPosition::Fixed => taffy::Position::Fixed,
    }
}

#[test]
fn out_of_flow_positions_map_one_to_one() {
    for position in [
        taffy::Position::Static,
        taffy::Position::Relative,
        taffy::Position::Absolute,
        taffy::Position::Fixed,
        taffy::Position::Sticky,
    ] {
        match expected_position(position) {
            Some(expected) => {
                assert_eq!(OofPosition::from_taffy(position), expected);
                assert_eq!(expected.into_taffy(), position);
                assert_eq!(position_in_taffy(expected), position);
            }
            None => assert!(
                !position.is_out_of_flow(),
                "a position without a counterpart is one the library keeps in flow"
            ),
        }
    }
}

#[test]
fn the_library_emits_candidates_for_exactly_the_positions_that_map() {
    for position in [
        taffy::Position::Static,
        taffy::Position::Relative,
        taffy::Position::Absolute,
        taffy::Position::Fixed,
        taffy::Position::Sticky,
    ] {
        assert_eq!(
            position.is_out_of_flow(),
            expected_position(position).is_some()
        );
    }
}
