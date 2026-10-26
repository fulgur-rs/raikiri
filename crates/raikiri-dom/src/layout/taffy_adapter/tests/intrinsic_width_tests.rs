//! The intrinsic widths a node keeps next to its measured sizes.

use super::*;

fn width_input(parent_width: f32) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Horizontal,
        known_dimensions: Size {
            width: None,
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        parent_size: Size {
            width: Some(parent_width),
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::MinContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line {
            start: false,
            end: false,
        },
    }
}

#[test]
fn a_width_answers_only_the_input_it_was_stored_for() {
    let stored = width_input(100.0);
    let mut cache = LayoutCache::new();
    assert_eq!(cache.width(&stored), None);
    cache.store_width(&stored, 42.0);
    assert_eq!(cache.width(&stored), Some(42.0));

    // Every field of the input is part of the key, the ones that the library's
    // cache leaves out of a measurement's key included.
    let others = [
        width_input(101.0),
        LayoutInput {
            parent_size: Size {
                width: Some(100.0),
                height: Some(50.0),
            },
            ..stored
        },
        LayoutInput {
            available_space: Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
            ..stored
        },
        LayoutInput {
            known_dimensions: Size {
                width: None,
                height: Some(10.0),
            },
            ..stored
        },
        LayoutInput {
            sizing_mode: SizingMode::ContentSize,
            ..stored
        },
        LayoutInput {
            vertical_margins_are_collapsible: Line {
                start: true,
                end: false,
            },
            ..stored
        },
    ];
    for other in others {
        assert_eq!(cache.width(&other), None, "{other:?}");
    }
}

#[test]
fn storing_a_width_again_replaces_it() {
    let input = width_input(100.0);
    let mut cache = LayoutCache::new();
    cache.store_width(&input, 42.0);
    cache.store_width(&input, 43.0);
    assert_eq!(cache.width(&input), Some(43.0));
}

#[test]
fn the_oldest_width_goes_when_there_are_too_many() {
    let mut cache = LayoutCache::new();
    for index in 0..=super::super::cache::MAX_WIDTHS {
        cache.store_width(&width_input(index as f32), index as f32);
    }
    assert_eq!(cache.width(&width_input(0.0)), None);
    for index in 1..=super::super::cache::MAX_WIDTHS {
        assert_eq!(
            cache.width(&width_input(index as f32)),
            Some(index as f32),
            "width {index}"
        );
    }
}

#[test]
fn clear_forgets_the_widths_and_widths_are_not_measurements() {
    let input = width_input(100.0);
    let mut cache = LayoutCache::new();
    cache.store_width(&input, 42.0);
    assert!(
        cache.get(&input).is_none(),
        "a width is not a measured size"
    );
    cache.clear();
    assert_eq!(cache.width(&input), None);
}
