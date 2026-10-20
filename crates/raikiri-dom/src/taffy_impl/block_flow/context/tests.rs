use super::{BlockFormattingContext, BoxProbe};
use taffy::CollapsibleMarginSet;
use taffy::{Clear, Direction, FloatDirection, Point, Size};

#[test]
fn rollback_preserves_prior_floats_and_discards_speculative_clearance_and_ceiling() {
    let mut owner = BlockFormattingContext::new();
    let mut flow = owner.root_block_context();
    flow.set_width(100.0);
    flow.place_floated_box(
        Size {
            width: 30.0,
            height: 20.0,
        },
        0.0,
        FloatDirection::Left,
        Clear::None,
        true,
    );
    let saved = flow.snapshot();
    flow.place_floated_box(
        Size {
            width: 20.0,
            height: 40.0,
        },
        100.0,
        FloatDirection::Right,
        Clear::None,
        true,
    );
    assert_eq!(flow.cleared_threshold(Clear::Right), Some(140.0));
    flow.commit_strut();
    flow.restore(&saved);
    let slot = flow.find_content_slot(0.0, Clear::None, None);
    assert_eq!((slot.x, slot.y, slot.width), (30.0, 0.0, 70.0));
    assert_eq!(flow.cleared_threshold(Clear::Left), Some(20.0));
    assert_eq!(flow.cleared_threshold(Clear::Right), None);
    assert_eq!(flow.floated_content_height_contribution(), 20.0);
    assert!(flow.has_adjoining_float(Clear::Left));
    assert!(!flow.has_adjoining_float(Clear::Right));
    let placed = flow.place_floated_box(
        Size {
            width: 20.0,
            height: 40.0,
        },
        0.0,
        FloatDirection::Right,
        Clear::None,
        false,
    );
    assert_eq!(placed, Point { x: 80.0, y: 0.0 });
}

#[test]
fn subcontext_maps_float_placement_and_clearance_to_its_local_border_box() {
    let mut owner = BlockFormattingContext::new();
    let mut root = owner.root_block_context();
    root.set_width(200.0);
    assert!(root.is_bfc_root());
    {
        let mut child = root.sub_context(15.0, [10.0, 20.0]);
        assert!(!child.is_bfc_root());
        assert_eq!(
            child.place_floated_box(
                Size {
                    width: 30.0,
                    height: 40.0
                },
                5.0,
                FloatDirection::Left,
                Clear::None,
                false
            ),
            Point { x: 0.0, y: 5.0 }
        );
        assert_eq!(child.cleared_threshold(Clear::Left), Some(45.0));
        assert_eq!(child.floated_content_height_contribution(), 45.0);
    }
    let slot = root.find_content_slot(20.0, Clear::None, None);
    assert_eq!((slot.x, slot.y, slot.width), (40.0, 20.0, 160.0));
    assert_eq!(root.cleared_threshold(Clear::Left), Some(60.0));
    root.add_child_floated_content_height_contribution(60.0);
    assert_eq!(root.floated_content_height_contribution(), 60.0);
}

#[test]
fn nested_leading_strut_retains_signed_ancestor_margins_at_the_hypothetical_origin() {
    let mut owner = BlockFormattingContext::new();
    let mut root = owner.root_block_context();
    let mut child = root.sub_context_for_collapsing_child(
        10.0,
        [0.0, 0.0],
        0.0,
        CollapsibleMarginSet::from_margin(30.0),
        false,
    );
    assert_eq!(
        child.hypothetical_top_position(CollapsibleMarginSet::from_margin(-20.0)),
        Some(0.0)
    );
    let grandchild = child.sub_context_for_collapsing_child(
        5.0,
        [0.0, 0.0],
        99.0,
        CollapsibleMarginSet::from_margin(-40.0),
        true,
    );
    assert_eq!(
        grandchild.hypothetical_top_position(CollapsibleMarginSet::ZERO),
        Some(-25.0)
    );
    assert_eq!(
        grandchild.hypothetical_top_position(CollapsibleMarginSet::from_margin(50.0)),
        Some(-5.0)
    );
}

#[test]
fn committing_flow_freezes_the_leading_float_flags_before_later_children() {
    let mut owner = BlockFormattingContext::new();
    let mut root = owner.root_block_context();
    root.set_width(100.0);
    root.place_floated_box(
        Size {
            width: 10.0,
            height: 10.0,
        },
        0.0,
        FloatDirection::Left,
        Clear::None,
        true,
    );
    root.commit_strut();
    assert!(!root.has_adjoining_float(Clear::Both));
    assert!(!root.has_top_adjoining_float(Clear::None));
    assert!(root.has_top_adjoining_float(Clear::Left));
    assert!(!root.has_top_adjoining_float(Clear::Right));
    assert_eq!(root.top_adjoining_floats(), [true, false]);
    let flags = {
        let mut child = root.sub_context(0.0, [0.0, 0.0]);
        child.place_floated_box(
            Size {
                width: 10.0,
                height: 10.0,
            },
            0.0,
            FloatDirection::Right,
            Clear::None,
            true,
        );
        child.commit_strut();
        child.top_adjoining_floats()
    };
    root.merge_adjoining_floats(flags);
    assert!(root.has_adjoining_float(Clear::Right));
    assert!(!root.has_adjoining_float(Clear::Left));
    assert_eq!(root.top_adjoining_floats(), [true, false]);
}

#[test]
fn independent_context_has_no_outer_float_or_clearance() {
    let mut outer_owner = BlockFormattingContext::new();
    let mut outer = outer_owner.root_block_context();
    outer.set_width(100.0);
    outer.place_floated_box(
        Size {
            width: 30.0,
            height: 40.0,
        },
        0.0,
        FloatDirection::Left,
        Clear::None,
        true,
    );
    let mut inner_owner = BlockFormattingContext::new();
    let mut inner = inner_owner.root_block_context();
    inner.set_width(100.0);
    assert!(!inner.has_floats());
    assert_eq!(inner.cleared_threshold(Clear::Both), None);
    let slot = inner.find_content_slot(0.0, Clear::None, None);
    assert_eq!((slot.x, slot.width), (0.0, 100.0));
    assert!(outer.has_active_floats(0.0));
}

#[test]
fn rollback_restores_content_edges_and_origin_resolution() {
    let mut owner = BlockFormattingContext::new();
    let mut root = owner.root_block_context();
    root.set_width(100.0);
    root.set_origin_resolved(true);
    root.apply_content_box_inset([10.0, 20.0]);
    let saved = root.snapshot();
    root.set_origin_resolved(false);
    root.apply_content_box_inset([30.0, 40.0]);
    root.restore(&saved);
    assert!(root.origin_resolved());
    let slot = root.find_content_slot(0.0, Clear::None, None);
    assert_eq!((slot.x, slot.width), (10.0, 70.0));
}

/// Two floats of 10px height, the left one 60px wide at the top and the
/// right one 80px wide just below it, in a 200px container.
fn stacked_floats(owner: &mut BlockFormattingContext) -> super::BlockContext<'_> {
    let mut flow = owner.root_block_context();
    flow.set_width(200.0);
    for (width, direction, clear) in [
        (60.0, FloatDirection::Left, Clear::None),
        (80.0, FloatDirection::Right, Clear::Left),
    ] {
        flow.place_floated_box(
            Size {
                width,
                height: 10.0,
            },
            0.0,
            direction,
            clear,
            false,
        );
    }
    flow
}

/// An `auto`-width box 20px tall that stretches over what its slot leaves.
fn stretching(probe: BoxProbe) -> f32 {
    match probe {
        BoxProbe::Width { stretch } => stretch,
        BoxProbe::Height { .. } => 20.0,
    }
}

#[test]
fn a_trailing_margin_is_taken_off_the_stretch_only_once() {
    let mut owner = BlockFormattingContext::new();
    let flow = stacked_floats(&mut owner);
    // The first segment leaves x = 60..200, and with the 20px margin an auto
    // width stretches over 120px of it. The 80px float below leaves
    // x = 0..120 and swallows that margin, so the room both leave is 60px.
    let placed = flow.find_box_slot(0.0, [0.0, 20.0], Direction::Ltr, Clear::None, stretching);
    assert_eq!(
        (placed.slot.x, placed.slot.y, placed.slot.border_width),
        (60.0, 0.0, 60.0)
    );
    assert_eq!((placed.slot.stretch_width, placed.width), (60.0, 60.0));
}

#[test]
fn the_room_beside_stacked_floats_is_the_same_from_the_right() {
    let mut owner = BlockFormattingContext::new();
    let flow = stacked_floats(&mut owner);
    let placed = flow.find_box_slot(0.0, [0.0, 20.0], Direction::Rtl, Clear::None, stretching);
    assert_eq!(
        (placed.slot.x, placed.slot.y, placed.slot.border_width),
        (60.0, 0.0, 60.0)
    );
    assert_eq!((placed.slot.stretch_width, placed.width), (60.0, 60.0));
}

/// Three left floats in a 200px container, each too wide to sit beside the
/// one above it, so they leave three segments to their right: 140px wide at
/// y = 0, 40px wide from y = 5, and 10px wide from y = 40.
fn three_segments(owner: &mut BlockFormattingContext) -> super::BlockContext<'_> {
    let mut flow = owner.root_block_context();
    flow.set_width(200.0);
    for (width, height) in [(60.0, 5.0), (160.0, 35.0), (190.0, 10.0)] {
        flow.place_floated_box(
            Size { width, height },
            0.0,
            FloatDirection::Left,
            Clear::None,
            false,
        );
    }
    flow
}

#[test]
fn a_box_whose_height_follows_its_width_settles_in_the_widest_room_it_fits() {
    let mut owner = BlockFormattingContext::new();
    let flow = three_segments(&mut owner);
    // A square that stretches over its room. At 140px it is tall enough to
    // reach the 10px segment, which narrows it to 10px. At that width it is
    // only 10px tall and reaches no further than the 40px segment, whose
    // room it then takes: 40px square, ending where the lowest segment
    // starts.
    let square = |probe: BoxProbe| match probe {
        BoxProbe::Width { stretch } => stretch,
        BoxProbe::Height { width } => width,
    };
    let placed = flow.find_box_slot(0.0, [0.0; 2], Direction::Ltr, Clear::None, square);
    assert_eq!(
        (placed.slot.x, placed.slot.y, placed.slot.border_width),
        (160.0, 0.0, 40.0)
    );
    assert_eq!(placed.width, 40.0);
}

#[test]
fn a_box_whose_width_and_height_chase_each_other_still_settles_clear_of_the_floats() {
    let mut owner = BlockFormattingContext::new();
    let flow = three_segments(&mut owner);
    // Wide, the box is tall enough for every segment; at the width that
    // leaves, it is short enough for the first only. The two never agree, so
    // the taller of the two heights decides.
    let chasing = |probe: BoxProbe| match probe {
        BoxProbe::Width { stretch } => stretch,
        BoxProbe::Height { width } if width > 100.0 => 100.0,
        BoxProbe::Height { .. } => 1.0,
    };
    let placed = flow.find_box_slot(0.0, [0.0; 2], Direction::Ltr, Clear::None, chasing);
    assert_eq!(
        (placed.slot.x, placed.slot.y, placed.slot.border_width),
        (190.0, 0.0, 10.0)
    );
    assert_eq!(placed.width, 10.0);
}
