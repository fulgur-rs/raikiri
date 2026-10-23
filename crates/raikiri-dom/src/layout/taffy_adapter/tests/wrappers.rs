//! The wrappers around the library's values, and the algorithm calls that take
//! more than a conversion: each does what the library's value does.

use super::*;
use crate::layout::taffy_adapter::OofSpan;
use crate::layout::taffy_adapter::algorithms::{claims_position, static_position_from_alignment};

fn candidates(floats: &mut Floats, count: usize) -> Vec<OofCandidate> {
    (0..count)
        .map(|index| {
            let position = if index % 2 == 0 {
                OofPosition::Absolute
            } else {
                OofPosition::Fixed
            };
            candidate(floats, 10 + index, position)
        })
        .collect()
}

fn list_of(candidates: &[OofCandidate]) -> OofCandidates {
    let mut list = OofCandidates::new();
    for &candidate in candidates {
        list.push(candidate);
    }
    list
}

fn bits_of_all(candidates: &[OofCandidate]) -> Vec<u32> {
    let mut out = Vec::new();
    for candidate in candidates {
        candidate.bits(&mut out);
    }
    out
}

#[test]
fn a_new_list_is_empty() {
    let list = OofCandidates::new();
    assert!(list.is_empty());
    assert_eq!(list.iter().len(), 0);
    assert!(list.to_vec().is_empty());
    assert!(OofCandidates::NONE.is_empty());
    assert!(OofCandidates::default().is_empty());
}

#[test]
fn candidates_come_out_in_the_order_they_went_in() {
    let mut floats = Floats(0);
    let pushed = candidates(&mut floats, 5);
    let list = list_of(&pushed);
    assert!(!list.is_empty());
    assert_eq!(list.iter().len(), pushed.len());
    assert_eq!(bits_of_all(&list.to_vec()), bits_of_all(&pushed));
    assert_eq!(
        bits_of(&list),
        bits_of_all(&pushed)
            .into_iter()
            .chain([5])
            .collect::<Vec<_>>()
    );
}

#[test]
fn append_moves_the_candidates_of_the_other_list_to_the_end() {
    let mut floats = Floats(3);
    let first = candidates(&mut floats, 2);
    let second = candidates(&mut floats, 3);
    let mut list = list_of(&first);
    let mut other = list_of(&second);
    list.append(&mut other);
    assert!(other.is_empty());
    let expected: Vec<_> = first.iter().chain(&second).copied().collect();
    assert_eq!(bits_of_all(&list.to_vec()), bits_of_all(&expected));

    // Appending to an empty list takes the candidates over whole.
    let mut empty = OofCandidates::new();
    empty.append(&mut list);
    assert!(list.is_empty());
    assert_eq!(bits_of_all(&empty.to_vec()), bits_of_all(&expected));

    // Appending an empty list changes nothing.
    empty.append(&mut OofCandidates::new());
    assert_eq!(bits_of_all(&empty.to_vec()), bits_of_all(&expected));
}

#[test]
fn take_leaves_an_empty_list_and_returns_everything() {
    let mut floats = Floats(5);
    let pushed = candidates(&mut floats, 4);
    let mut list = list_of(&pushed);
    let taken = list.take();
    assert!(list.is_empty());
    assert_eq!(bits_of_all(&taken.to_vec()), bits_of_all(&pushed));
    assert!(OofCandidates::new().take().is_empty());
}

#[test]
fn translate_moves_every_static_area_as_the_library_does() {
    for offset in [
        Point { x: 3.5, y: -2.0 },
        Point { x: -0.0, y: 0.0 },
        Point {
            x: f32::NAN,
            y: 1e7,
        },
        Point {
            x: f32::INFINITY,
            y: f32::NEG_INFINITY,
        },
    ] {
        let mut floats = Floats(1);
        let pushed = candidates(&mut floats, 3);
        let mut own = list_of(&pushed);
        own.translate(offset);

        let mut theirs = taffy::OofCandidates::NONE;
        for &candidate in &pushed {
            theirs.push(candidate.into_taffy());
        }
        theirs.translate(offset.into_taffy());

        assert_eq!(
            bits_of(&own),
            bits_of(&OofCandidates::from_taffy(theirs)),
            "translated by {offset:?}"
        );
    }
}

#[test]
fn for_each_mut_edits_each_candidate_in_place() {
    let mut floats = Floats(2);
    let pushed = candidates(&mut floats, 4);
    let mut list = list_of(&pushed);
    let mut visited = 0;
    list.for_each_mut(|candidate| {
        visited += 1;
        candidate.order += visited;
        candidate.position = OofPosition::Fixed;
    });
    assert_eq!(visited, 4);
    for (index, (edited, original)) in list.iter().zip(&pushed).enumerate() {
        assert_eq!(edited.order, original.order + index as u32 + 1);
        assert_eq!(edited.node, original.node);
        assert!(matches!(edited.position, OofPosition::Fixed));
        assert_eq!(
            bits_of(&edited.static_position),
            bits_of(&original.static_position)
        );
    }
}

#[test]
fn a_list_clones_to_an_equal_independent_list() {
    let mut floats = Floats(4);
    let pushed = candidates(&mut floats, 3);
    let list = list_of(&pushed);
    let mut copy = list.clone();
    assert_eq!(copy.to_vec().len(), 3);
    copy.push(candidate(&mut floats, 99, OofPosition::Absolute));
    assert_eq!(list.iter().len(), 3);
    assert_eq!(copy.iter().len(), 4);
}

#[test]
fn margin_sets_collapse_as_the_library_collapses_them() {
    let margins = [0.0, -0.0, 4.0, -7.5, 1e7, -1e7, f32::NAN];
    for &first in &margins {
        for &second in &margins {
            for &third in &margins {
                let own = CollapsibleMarginSet::from_margin(first)
                    .collapse_with_margin(second)
                    .collapse_with_set(CollapsibleMarginSet::from_margin(third));
                let theirs = taffy::CollapsibleMarginSet::from_margin(first)
                    .collapse_with_margin(second)
                    .collapse_with_set(taffy::CollapsibleMarginSet::from_margin(third));
                // `NaN` margins make the sets unequal to themselves, so the sets
                // are compared by what they print and by what they resolve to.
                assert_eq!(format!("{:?}", own.into_taffy()), format!("{theirs:?}"));
                assert_eq!(own.resolve().to_bits(), theirs.resolve().to_bits());
            }
        }
    }
    assert_eq!(
        CollapsibleMarginSet::ZERO.into_taffy(),
        taffy::CollapsibleMarginSet::ZERO
    );
    assert_eq!(CollapsibleMarginSet::ZERO.resolve(), 0.0);
}

#[test]
fn a_batch_hands_out_the_candidates_it_was_collected_from() {
    let mut floats = Floats(6);
    let collected = candidates(&mut floats, 5);
    let batch = OofBatch::collect(collected.iter().copied());
    assert_eq!(batch.iter().len(), 5);
    assert_eq!(
        bits_of_all(&batch.iter().collect::<Vec<_>>()),
        bits_of_all(&collected)
    );
    assert_eq!(batch.all().len(), 5);
    assert!(!batch.all().is_empty());

    // A run of the batch is the run of the candidates at the same positions.
    for start in 0..=collected.len() {
        for end in start..=collected.len() {
            let span: OofSpan<'_> = batch.all().slice(start..end);
            assert_eq!(span.len(), end - start);
            assert_eq!(span.is_empty(), start == end);
            assert_eq!(
                bits_of_all(&span.iter().collect::<Vec<_>>()),
                bits_of_all(&collected[start..end])
            );
        }
    }
    // A run of a run counts from the start of the run.
    let inner = batch.all().slice(1..4).slice(1..3);
    assert_eq!(
        bits_of_all(&inner.iter().collect::<Vec<_>>()),
        bits_of_all(&collected[2..4])
    );
}

#[test]
fn an_empty_batch_has_an_empty_run() {
    let batch = OofBatch::collect(std::iter::empty());
    assert_eq!(batch.iter().len(), 0);
    assert!(batch.all().is_empty());
    assert_eq!(batch.all().len(), 0);
    assert!(batch.all().slice(0..0).is_empty());
}

#[test]
#[should_panic]
fn a_run_past_the_end_of_the_batch_panics() {
    let batch = OofBatch::collect(std::iter::empty());
    let _ = batch.all().slice(0..1);
}

#[test]
fn a_box_claims_the_positions_the_library_says_it_claims() {
    for absolute in [false, true] {
        for fixed in [false, true] {
            let claims = taffy::ContainingBlockClaims { absolute, fixed };
            for position in [OofPosition::Absolute, OofPosition::Fixed] {
                let expected = match position {
                    OofPosition::Absolute => absolute,
                    OofPosition::Fixed => fixed,
                };
                assert_eq!(
                    claims_position(claims, position),
                    expected,
                    "{claims:?} for {position:?}"
                );
                assert_eq!(
                    claims_position(claims, position),
                    claims.for_position(position.into_taffy())
                );
            }
        }
    }
}

#[test]
fn a_static_position_comes_from_the_alignment_as_the_library_computes_it() {
    let keywords = [
        taffy::AlignItemsKeyword::Normal,
        taffy::AlignItemsKeyword::Start,
        taffy::AlignItemsKeyword::End,
        taffy::AlignItemsKeyword::FlexStart,
        taffy::AlignItemsKeyword::FlexEnd,
        taffy::AlignItemsKeyword::Center,
        taffy::AlignItemsKeyword::Baseline,
        taffy::AlignItemsKeyword::Stretch,
    ];
    let safeties = [
        taffy::AlignmentSafety::Default,
        taffy::AlignmentSafety::Unsafe,
        taffy::AlignmentSafety::Safe,
    ];
    let mut floats = Floats(0);
    for keyword in keywords {
        for safety in safeties {
            for axis_is_rtl in [false, true] {
                let alignment = taffy::AlignSelf { keyword, safety };
                let area = Line {
                    start: floats.next(),
                    end: floats.next(),
                };
                let own = static_position_from_alignment(alignment, area, axis_is_rtl);
                let theirs = taffy::AxisStaticPosition::from_alignment(
                    alignment,
                    area.into_taffy(),
                    axis_is_rtl,
                );
                assert_eq!(
                    bits_of(&own),
                    bits_of(&AxisStaticPosition::from_taffy(theirs)),
                    "{alignment:?} in an rtl axis: {axis_is_rtl}"
                );
            }
        }
    }
}

#[test]
fn a_leaf_is_measured_with_the_inputs_the_library_passes() {
    use crate::layout::taffy_adapter::algorithms::compute_leaf_layout;

    struct Style;
    impl taffy::CoreStyle for Style {
        type CustomIdent = smol_str::SmolStr;
    }

    let mut floats = Floats(0);
    for run_mode in [RunMode::ComputeSize, RunMode::PerformLayout] {
        for _ in 0..extreme_floats().len() {
            let inputs = LayoutInput {
                run_mode,
                ..layout_input(&mut floats)
            };

            let mut own_seen = None;
            let own = compute_leaf_layout(
                inputs,
                &Style,
                |_, basis| basis,
                |known, available| {
                    own_seen = Some((known, available));
                    Size {
                        width: 30.0,
                        height: 12.0,
                    }
                },
            );

            let mut taffy_seen = None;
            let theirs = taffy::compute_leaf_layout(
                inputs.into_taffy(),
                &Style,
                |_, basis| basis,
                |known, available| {
                    taffy_seen = Some((known, available));
                    taffy::Size {
                        width: 30.0,
                        height: 12.0,
                    }
                },
            );

            assert_eq!(
                own_seen.map(|(known, available)| (
                    bits_of(&known),
                    bits_of(&available.width),
                    bits_of(&available.height)
                )),
                taffy_seen.map(|(known, available)| (
                    bits_of(&Size::<Option<f32>>::from_taffy(known)),
                    bits_of(&AvailableSpace::from_taffy(available.width)),
                    bits_of(&AvailableSpace::from_taffy(available.height)),
                )),
                "measure arguments for {inputs:?}"
            );
            assert_eq!(bits_of(&own), bits_of(&LayoutOutput::from_taffy(theirs)));
        }
    }
}
