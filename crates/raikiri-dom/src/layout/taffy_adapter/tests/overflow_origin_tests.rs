use super::*;
use crate::layout::protocol::OverflowOrigin;

#[test]
fn the_final_cache_keeps_origin_across_size_stores_clone_and_replacement() {
    let final_input = LayoutInput {
        run_mode: RunMode::PerformLayout,
        ..LayoutInput::HIDDEN
    };
    let size_input = LayoutInput {
        run_mode: RunMode::ComputeSize,
        ..final_input
    };
    let mut cache = LayoutCache::new();
    let mut physical = LayoutOutput::from_sizes(
        Size {
            width: 100.0,
            height: 100.0,
        },
        Edges {
            left: -50.0,
            right: 100.0,
            top: 0.0,
            bottom: 100.0,
        },
    );
    physical.overflow_origin = OverflowOrigin::Physical;
    cache.store(&final_input, physical.clone());
    cache.store(&size_input, LayoutOutput::from_outer_size(physical.size));
    assert_eq!(cache.get(&final_input), Some(physical.clone()));
    let size = cache.get(&size_input).unwrap();
    assert_eq!(size.scrollable_overflow_rect, Edges::ZERO);
    assert_eq!(size.overflow_origin, OverflowOrigin::ScrollOrigin);
    let mut cloned = cache.clone();
    assert_eq!(cloned.get(&final_input), Some(physical.clone()));
    cache.forget_final_layout();
    assert!(cache.get(&final_input).is_none());
    let mut legacy = physical.clone();
    legacy.overflow_origin = OverflowOrigin::ScrollOrigin;
    cache.store(&final_input, legacy.clone());
    assert_eq!(cache.get(&final_input), Some(legacy.clone()));
    let changed_mode = LayoutInput {
        sizing_mode: SizingMode::ContentSize,
        ..size_input
    };
    cache.store(&changed_mode, LayoutOutput::from_outer_size(physical.size));
    assert!(cache.get(&final_input).is_none());
    cache.store(&final_input, physical.clone());
    assert_eq!(cache.get(&final_input), Some(physical));
    cache.clear();
    assert!(cache.get(&final_input).is_none());
    cache.store(&final_input, legacy.clone());
    assert_eq!(cache.get(&final_input), Some(legacy));
}

#[test]
fn physical_and_legacy_overflow_keep_empty_rectangles_and_padding_extent() {
    let mut output = LayoutOutput::from_sizes(
        Size {
            width: 110.0,
            height: 107.0,
        },
        Edges {
            left: -50.0,
            right: 100.0,
            top: -2.0,
            bottom: 105.0,
        },
    );
    output.overflow_origin = OverflowOrigin::Physical;
    let physical = output.scrollable_overflow_rect;
    assert_eq!(output.physical_overflow(100.0, true), physical);
    output.encode_overflow(100.0, true);
    assert_eq!(
        (
            output.scrollable_overflow_rect.left,
            output.scrollable_overflow_rect.right
        ),
        (0.0, 150.0)
    );
    assert_eq!(output.physical_overflow(100.0, true), physical);
    let encoded = output.scrollable_overflow_rect;
    output.encode_overflow(100.0, true);
    assert_eq!(output.scrollable_overflow_rect, encoded);
    assert_eq!(output.overflow_origin, OverflowOrigin::ScrollOrigin);
    output.scrollable_overflow_rect = Edges::ZERO;
    assert_eq!(output.physical_overflow(100.0, true), Edges::ZERO);
    output.overflow_origin = OverflowOrigin::Physical;
    output.encode_overflow(100.0, true);
    assert_eq!(output.scrollable_overflow_rect, Edges::ZERO);
}
