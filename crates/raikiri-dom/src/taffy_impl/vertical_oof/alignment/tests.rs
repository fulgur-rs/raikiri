use super::*;

/// A box in a positioning area that spans 10..110 of its container.
fn axis_box(
    placement: Placement,
    safety: AlignmentSafety,
    reversed: bool,
    inset: (f32, f32),
    size: f32,
) -> AxisBox {
    AxisBox {
        area: Line {
            start: 10.0,
            end: 110.0,
        },
        inset: Line {
            start: inset.0,
            end: inset.1,
        },
        margin: Line {
            start: 0.0,
            end: 0.0,
        },
        size,
        placement,
        safety,
        reversed,
        scrolls: false,
    }
}

fn location(
    placement: Placement,
    safety: AlignmentSafety,
    reversed: bool,
    inset: (f32, f32),
    size: f32,
) -> f32 {
    axis_box(placement, safety, reversed, inset, size).location()
}

#[test]
fn start_edges_follow_the_writing_mode_and_direction() {
    use Axis::{Horizontal, Vertical};
    use WritingMode::{HorizontalTb, SidewaysLr, SidewaysRl, VerticalLr, VerticalRl};
    let (ltr, rtl) = (Direction::Ltr, Direction::Rtl);
    for (mode, direction, horizontal, vertical) in [
        (HorizontalTb, ltr, false, false),
        (HorizontalTb, rtl, true, false),
        (VerticalRl, ltr, true, false),
        (VerticalRl, rtl, true, true),
        (VerticalLr, ltr, false, false),
        (VerticalLr, rtl, false, true),
        (SidewaysRl, ltr, true, false),
        // The bridge hands `sideways-lr` the direction of its physical
        // inline axis.
        (SidewaysLr, rtl, false, true),
    ] {
        assert_eq!(start_is_end(mode, direction, Horizontal), horizontal);
        assert_eq!(start_is_end(mode, direction, Vertical), vertical);
    }
}

#[test]
fn self_positions_are_resolved_against_the_containing_block() {
    let placement = |alignment, own_start_is_end, start_is_end| {
        super::placement(Some(alignment), own_start_is_end, start_is_end)
    };
    // The same start edge as the containing block.
    assert_eq!(
        placement(AlignSelf::SELF_START, true, true),
        Placement::Start
    );
    assert_eq!(placement(AlignSelf::SELF_END, false, false), Placement::End);
    // The opposite one.
    assert_eq!(
        placement(AlignSelf::SELF_START, true, false),
        Placement::End
    );
    assert_eq!(
        placement(AlignSelf::SELF_END, false, true),
        Placement::Start
    );
    assert_eq!(
        placement(AlignSelf::SAFE_SELF_END, false, true),
        Placement::Start
    );
}

#[test]
fn keywords_map_to_placements() {
    let placement = |alignment| super::placement(alignment, false, false);
    assert_eq!(placement(None), Placement::Normal);
    assert_eq!(placement(Some(AlignSelf::NORMAL)), Placement::Normal);
    assert_eq!(placement(Some(AlignSelf::STRETCH)), Placement::Stretch);
    assert_eq!(placement(Some(AlignSelf::CENTER)), Placement::Center);
    assert_eq!(placement(Some(AlignSelf::START)), Placement::Start);
    assert_eq!(placement(Some(AlignSelf::FLEX_START)), Placement::Start);
    assert_eq!(placement(Some(AlignSelf::BASELINE)), Placement::Start);
    assert_eq!(placement(Some(AlignSelf::END)), Placement::End);
    assert_eq!(placement(Some(AlignSelf::FLEX_END)), Placement::End);
}

#[test]
fn a_box_is_aligned_within_the_inset_modified_containing_block() {
    use AlignmentSafety::Default;
    use Placement::{Center, End, Normal, Start, Stretch};
    for (placement, reversed, expected) in [
        (Start, false, 10.0),
        (Normal, false, 10.0),
        (Stretch, false, 10.0),
        (End, false, 60.0),
        (Center, false, 35.0),
        // The logical start edge is the physical end edge.
        (Start, true, 60.0),
        (Normal, true, 60.0),
        (End, true, 10.0),
        (Center, true, 35.0),
    ] {
        assert_eq!(
            location(placement, Default, reversed, (0.0, 0.0), 50.0),
            expected,
            "{placement:?}, reversed: {reversed}"
        );
    }
    // The insets shrink the area the box is aligned in.
    assert_eq!(
        location(End, Default, false, (10.0, 20.0), 50.0),
        10.0 + 80.0 - 50.0
    );
}

#[test]
fn margins_stay_around_the_box() {
    let mut aligned = axis_box(
        Placement::End,
        AlignmentSafety::Default,
        false,
        (0.0, 0.0),
        50.0,
    );
    aligned.margin = Line {
        start: 5.0,
        end: 15.0,
    };
    // The 70px margin box ends at the end of the area.
    assert_eq!(aligned.location(), 10.0 + 30.0 + 5.0);
}

#[test]
fn over_constrained_insets_drop_the_logical_end_inset() {
    use AlignmentSafety::Default;
    let location = |reversed| location(Placement::Normal, Default, reversed, (60.0, 60.0), 20.0);
    // The area is 100px wide: the end inset gives way and the box stays at
    // the logical start inset.
    assert_eq!(location(false), 10.0 + 60.0);
    assert_eq!(location(true), 10.0 + 40.0 - 20.0);
}

#[test]
fn an_overflowing_box_is_aligned_by_its_overflow_position() {
    use AlignmentSafety::{Default, Safe, Unsafe};
    use Placement::{End, Normal};
    // A 150px box in a 100px area. `unsafe` keeps the alignment, `safe` and
    // the default fall back to the logical start edge.
    assert_eq!(location(End, Unsafe, false, (0.0, 0.0), 150.0), 10.0 - 50.0);
    assert_eq!(location(End, Safe, false, (0.0, 0.0), 150.0), 10.0);
    assert_eq!(location(End, Default, false, (0.0, 0.0), 150.0), 10.0);
    assert_eq!(location(End, Safe, true, (0.0, 0.0), 150.0), 10.0 - 50.0);
    assert_eq!(location(End, Default, true, (0.0, 0.0), 150.0), 10.0 - 50.0);
    assert_eq!(location(End, Unsafe, true, (0.0, 0.0), 150.0), 10.0);
    // `normal` is aligned as specified whatever the overflow.
    assert_eq!(location(Normal, Default, false, (0.0, 0.0), 150.0), 10.0);
}

#[test]
fn an_overflowing_box_stays_inside_the_containing_block_when_it_can() {
    use AlignmentSafety::Default;
    use Placement::{End, Start};
    // The 60px box overflows the 50px inset-modified block but fits the area.
    assert_eq!(
        location(End, Default, false, (25.0, 25.0), 60.0),
        10.0 + 15.0
    );
    assert_eq!(
        location(Start, Default, false, (25.0, 25.0), 60.0),
        10.0 + 25.0
    );
    // Moved back from the end of the area.
    assert_eq!(
        location(Start, Default, false, (70.0, 0.0), 50.0),
        10.0 + 50.0
    );
    // Moved forward from its start.
    assert_eq!(location(End, Default, false, (0.0, 70.0), 50.0), 10.0);
}

#[test]
fn a_scroll_container_lets_the_box_overflow_in_the_scrolling_direction() {
    use AlignmentSafety::Default;
    use Placement::Start;
    let location = |reversed, scrolls| {
        let mut aligned = axis_box(Start, Default, reversed, (70.0, 0.0), 50.0);
        aligned.scrolls = scrolls;
        if reversed {
            aligned.inset = Line {
                start: 0.0,
                end: 70.0,
            };
        }
        aligned.location()
    };
    // From the start edge the box may run off the end of the area.
    assert_eq!(location(false, false), 10.0 + 50.0);
    assert_eq!(location(false, true), 10.0 + 70.0);
    // Scrolling begins at the end edge when the logical start is there.
    assert_eq!(location(true, false), 10.0);
    assert_eq!(location(true, true), 10.0 - 20.0);
}
