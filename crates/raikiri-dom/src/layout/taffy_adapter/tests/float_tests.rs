//! The float wrappers, the slots and the block alignment call do what the
//! library's values do for the same inputs.

use super::*;
use crate::layout::protocol::{BfcSlot, FloatDirection};
use crate::layout::taffy_adapter::algorithms::compute_block_align_content_offset;
use crate::layout::taffy_adapter::{FloatContext, FloatIntrinsicWidthCalculator};
use raikiri_layout_style::ToTaffy;
use raikiri_layout_style::values::{
    AlignContent, AlignContentKeyword, AlignmentSafety, Clear, Direction, Float,
};

const CLEARS: [Clear; 4] = [Clear::None, Clear::Left, Clear::Right, Clear::Both];
const DIRECTIONS: [Direction; 2] = [Direction::Ltr, Direction::Rtl];

impl Bits for BfcSlot {
    fn bits(&self, out: &mut Vec<u32>) {
        let BfcSlot {
            segment_id,
            x,
            y,
            border_width,
            stretch_width,
        } = *self;
        segment_id.map(|id| id as u32).bits(out);
        x.bits(out);
        y.bits(out);
        border_width.bits(out);
        stretch_width.bits(out);
    }
}

fn expected_float_direction(direction: taffy::FloatDirection) -> FloatDirection {
    match direction {
        taffy::FloatDirection::Left => FloatDirection::Left,
        taffy::FloatDirection::Right => FloatDirection::Right,
    }
}

fn float_direction_in_taffy(direction: FloatDirection) -> taffy::FloatDirection {
    match direction {
        FloatDirection::Left => taffy::FloatDirection::Left,
        FloatDirection::Right => taffy::FloatDirection::Right,
    }
}

fn expected_from_float(float: taffy::Float) -> Option<FloatDirection> {
    match float {
        taffy::Float::Left => Some(FloatDirection::Left),
        taffy::Float::Right => Some(FloatDirection::Right),
        taffy::Float::None => None,
    }
}

#[test]
fn float_directions_map_one_to_one() {
    for direction in [taffy::FloatDirection::Left, taffy::FloatDirection::Right] {
        let ours = expected_float_direction(direction);
        assert_eq!(ours.into_taffy(), direction);
        assert_eq!(float_direction_in_taffy(ours), direction);
    }
}

#[test]
fn a_float_direction_indexes_the_per_side_state_left_first() {
    assert_eq!(FloatDirection::Left as usize, 0);
    assert_eq!(FloatDirection::Right as usize, 1);
}

#[test]
fn a_float_keyword_names_the_side_the_library_names() {
    for float in [Float::Left, Float::Right, Float::None] {
        let theirs = float.to_taffy();
        assert_eq!(
            FloatDirection::from_float(float),
            expected_from_float(theirs)
        );
        assert_eq!(
            FloatDirection::from_float(float),
            theirs.float_direction().map(expected_float_direction),
        );
    }
}

#[test]
fn a_slot_keeps_every_field_and_bit() {
    for start in 0..extreme_floats().len() {
        let mut floats = Floats(start);
        for segment_id in [None, Some(0), Some(7)] {
            let slot = taffy::BfcSlot {
                segment_id,
                x: floats.next(),
                y: floats.next(),
                border_width: floats.next(),
                stretch_width: floats.next(),
            };
            let ours = BfcSlot::from_taffy(slot);
            assert_eq!(ours.segment_id, slot.segment_id);
            assert_eq!(
                bits_of(&ours),
                [
                    bits_of(&slot.segment_id.map(|id| id as u32)),
                    vec![
                        slot.x.to_bits(),
                        slot.y.to_bits(),
                        slot.border_width.to_bits(),
                        slot.stretch_width.to_bits(),
                    ],
                ]
                .concat(),
            );
        }
    }
}

/// A float as the layout places it: the box, the least `y` it may take, the
/// insets of its containing block, its side and what it clears.
struct Placement {
    floated_box: Size<f32>,
    min_y: f32,
    insets: [f32; 2],
    direction: FloatDirection,
    clear: Clear,
}

/// Floats in the order they come in the document.
fn placements() -> Vec<Placement> {
    let placement = |width, height, min_y, insets, direction, clear| Placement {
        floated_box: Size { width, height },
        min_y,
        insets,
        direction,
        clear,
    };
    vec![
        placement(
            60.0,
            20.0,
            0.0,
            [0.0, 0.0],
            FloatDirection::Left,
            Clear::None,
        ),
        placement(
            40.0,
            30.0,
            0.0,
            [0.0, 0.0],
            FloatDirection::Right,
            Clear::None,
        ),
        placement(
            80.0,
            25.0,
            5.0,
            [10.0, 10.0],
            FloatDirection::Left,
            Clear::Left,
        ),
        placement(
            30.0,
            40.0,
            10.0,
            [0.0, 5.0],
            FloatDirection::Right,
            Clear::Both,
        ),
        placement(
            120.0,
            10.0,
            0.0,
            [0.0, 0.0],
            FloatDirection::Left,
            Clear::Right,
        ),
        placement(
            500.0,
            10.0,
            0.0,
            [0.0, 0.0],
            FloatDirection::Right,
            Clear::None,
        ),
        placement(
            0.0,
            0.0,
            12.0,
            [0.0, 0.0],
            FloatDirection::Left,
            Clear::None,
        ),
    ]
}

/// Every answer `ours` gives matches what `theirs` gives, bit for bit, once
/// the clearance is passed to `theirs` as the least `y` and no clearance, as
/// the wrapper does (see `FloatContext::place_floated_box`).
fn assert_same_answers(ours: &FloatContext, theirs: &taffy::FloatContext) {
    assert_eq!(ours.has_floats(), theirs.has_floats());
    for min_y in [-5.0, 0.0, 12.5, 40.0, 1e4] {
        assert_eq!(
            ours.has_active_floats(min_y),
            theirs.has_active_floats(min_y)
        );
        for clear in CLEARS {
            for insets in [[0.0, 0.0], [8.0, 3.0]] {
                for margins in [[0.0, 0.0], [4.0, 6.0], [-10.0, 0.0]] {
                    for direction in DIRECTIONS {
                        let mut after = None;
                        // Follow the segments from the top down.
                        for _ in 0..8 {
                            let slot =
                                ours.find_bfc_slot(min_y, insets, margins, direction, clear, after);
                            let expected = theirs.find_bfc_slot(
                                ours.clearance_floor(min_y, clear),
                                insets,
                                margins,
                                direction.to_taffy(),
                                taffy::Clear::None,
                                after,
                            );
                            assert_eq!(bits_of(&slot), bits_of(&BfcSlot::from_taffy(expected)));
                            let Some(segment) = slot.segment_id else {
                                break;
                            };
                            after = Some(segment);
                        }
                    }
                }
            }
        }
    }
    for clear in CLEARS {
        assert_eq!(
            bits_of(&ours.cleared_threshold(clear)),
            bits_of(&theirs.cleared_threshold(clear.to_taffy())),
        );
    }
}

#[test]
fn a_float_context_places_and_answers_like_the_library() {
    let mut ours = FloatContext::new();
    let mut theirs = taffy::FloatContext::new();
    ours.set_width(200.0);
    theirs.set_width(200.0);
    assert!(!ours.has_floats());
    assert_same_answers(&ours, &theirs);
    for Placement {
        floated_box,
        min_y,
        insets,
        direction,
        clear,
    } in placements()
    {
        let their_min_y = ours.clearance_floor(min_y, clear);
        let expected = theirs.place_floated_box(
            floated_box.into_taffy(),
            their_min_y,
            insets,
            float_direction_in_taffy(direction),
            taffy::Clear::None,
        );
        let placed = ours.place_floated_box(floated_box, min_y, insets, direction, clear);
        assert_eq!(
            bits_of(&placed),
            bits_of(&Point::<f32>::from_taffy(expected))
        );
        assert_same_answers(&ours, &theirs);
    }
    assert!(ours.has_floats());
}

/// A 60px context with a 20x30 left float at the top and a 20x30 right float
/// that clears it, below it.
fn left_float_then_right_float_clearing_it() -> FloatContext {
    let mut floats = FloatContext::new();
    floats.set_width(60.0);
    let size = Size {
        width: 20.0,
        height: 30.0,
    };
    floats.place_floated_box(size, 0.0, [0.0, 0.0], FloatDirection::Left, Clear::None);
    let right = floats.place_floated_box(size, 0.0, [0.0, 0.0], FloatDirection::Right, Clear::Left);
    assert_eq!((right.x, right.y), (40.0, 30.0));
    floats
}

#[test]
fn a_float_that_clears_one_side_sits_beside_a_float_on_the_other_side() {
    let mut floats = left_float_then_right_float_clearing_it();
    let left = floats.place_floated_box(
        Size {
            width: 20.0,
            height: 30.0,
        },
        0.0,
        [0.0, 0.0],
        FloatDirection::Left,
        Clear::Left,
    );
    // Chromium puts it beside the right float, not below it at 60.
    assert_eq!((left.x, left.y), (0.0, 30.0));
}

#[test]
fn a_box_that_clears_one_side_finds_room_beside_a_float_on_the_other_side() {
    let floats = left_float_then_right_float_clearing_it();
    let slot = floats.find_bfc_slot(
        0.0,
        [0.0, 0.0],
        [0.0, 0.0],
        Direction::Ltr,
        Clear::Left,
        None,
    );
    assert_eq!((slot.x, slot.y, slot.border_width), (0.0, 30.0, 40.0));
}

#[test]
fn a_box_that_clears_below_every_float_finds_room_beside_none() {
    let floats = left_float_then_right_float_clearing_it();
    // Below every float there is no float left to clear, so no clearance
    // applies at all.
    let slot = floats.find_bfc_slot(
        70.0,
        [0.0, 0.0],
        [0.0, 0.0],
        Direction::Ltr,
        Clear::Both,
        None,
    );
    assert_eq!(
        (slot.segment_id, slot.y, slot.border_width),
        (None, 70.0, 60.0)
    );
}

#[test]
fn a_box_clears_a_zero_height_float_below_every_segment() {
    let mut floats = FloatContext::new();
    floats.set_width(60.0);
    floats.place_floated_box(
        Size {
            width: 20.0,
            height: 30.0,
        },
        0.0,
        [0.0, 0.0],
        FloatDirection::Left,
        Clear::None,
    );
    let empty = floats.place_floated_box(
        Size {
            width: 20.0,
            height: 0.0,
        },
        80.0,
        [0.0, 0.0],
        FloatDirection::Left,
        Clear::None,
    );
    assert_eq!(empty.y, 80.0);
    // The empty float occupies no segment, so no segment reaches below 40,
    // but the box still clears it as a float clearing the left side does.
    let slot = floats.find_bfc_slot(
        40.0,
        [0.0, 0.0],
        [0.0, 0.0],
        Direction::Ltr,
        Clear::Left,
        None,
    );
    let placed = floats.place_floated_box(
        Size {
            width: 20.0,
            height: 10.0,
        },
        40.0,
        [0.0, 0.0],
        FloatDirection::Right,
        Clear::Left,
    );
    assert_eq!((slot.y, placed.y), (80.0, 80.0));
}

#[test]
fn a_cloned_float_context_does_not_follow_the_original() {
    let mut original = FloatContext::new();
    original.set_width(100.0);
    let copy = original.clone();
    original.place_floated_box(
        Size {
            width: 30.0,
            height: 10.0,
        },
        0.0,
        [0.0, 0.0],
        FloatDirection::Left,
        Clear::None,
    );
    assert!(original.has_floats());
    assert!(!copy.has_floats());
}

#[test]
fn the_width_calculator_adds_floats_like_the_library() {
    let floats = [
        (50.0, FloatDirection::Left, Clear::None),
        (30.0, FloatDirection::Left, Clear::None),
        (70.0, FloatDirection::Right, Clear::None),
        (40.0, FloatDirection::Left, Clear::Left),
        (90.0, FloatDirection::Right, Clear::Both),
        (20.0, FloatDirection::Right, Clear::Right),
    ];
    for space in [
        AvailableSpace::Definite(100.0),
        AvailableSpace::Definite(1000.0),
        AvailableSpace::MinContent,
        AvailableSpace::MaxContent,
    ] {
        let mut ours = FloatIntrinsicWidthCalculator::new(space);
        let mut theirs = taffy::FloatIntrinsicWidthCalculator::new(space.into_taffy());
        assert_eq!(ours.result().to_bits(), theirs.result().to_bits());
        for (width, direction, clear) in floats {
            ours.add_float(width, direction, clear);
            theirs.add_float(width, float_direction_in_taffy(direction), clear.to_taffy());
            assert_eq!(
                ours.result().to_bits(),
                theirs.result().to_bits(),
                "{space:?} after a float of {width}"
            );
        }
    }
}

#[test]
fn block_alignment_offsets_content_like_the_library() {
    // `normal` is left out: the library asks for it to be resolved first.
    let keywords = [
        AlignContentKeyword::Start,
        AlignContentKeyword::End,
        AlignContentKeyword::FlexStart,
        AlignContentKeyword::FlexEnd,
        AlignContentKeyword::Center,
        AlignContentKeyword::Stretch,
        AlignContentKeyword::SpaceBetween,
        AlignContentKeyword::SpaceEvenly,
        AlignContentKeyword::SpaceAround,
    ];
    let safeties = [
        AlignmentSafety::Default,
        AlignmentSafety::Unsafe,
        AlignmentSafety::Safe,
    ];
    for keyword in keywords {
        for safety in safeties {
            let align_content = AlignContent { keyword, safety };
            for free in [-20.0, 0.0, 35.5] {
                assert_eq!(
                    compute_block_align_content_offset(align_content, free).to_bits(),
                    taffy::compute_block_align_content_offset(align_content.to_taffy(), free)
                        .to_bits(),
                    "{align_content:?} with {free} free"
                );
            }
        }
    }
}
