use super::*;

#[test]
fn longhand_names_are_matched_ascii_case_insensitively() {
    assert_eq!(
        longhand_side("Margin-Block-End"),
        Some(LogicalSide::BlockEnd)
    );
    assert_eq!(
        longhand_side("padding-inline-start"),
        Some(LogicalSide::InlineStart)
    );
    assert_eq!(longhand_side("margin-left"), None);
}

#[test]
fn inset_longhand_names_are_flow_relative_sides() {
    for (name, side) in [
        ("inset-inline-start", LogicalSide::InlineStart),
        ("inset-inline-end", LogicalSide::InlineEnd),
        ("Inset-Block-Start", LogicalSide::BlockStart),
        ("inset-block-end", LogicalSide::BlockEnd),
    ] {
        assert_eq!(longhand_side(name), Some(side), "{name}");
    }
    // The physical offsets and the shorthands are not flow-relative longhands.
    for name in ["top", "left", "inset", "inset-inline", "inset-block"] {
        assert_eq!(longhand_side(name), None, "{name}");
    }
}

#[test]
fn non_box_keys_keep_their_slot() {
    let flow = FlowMapping::new(WritingMode::VerticalRl, Direction::Ltr);
    assert_eq!(
        flow.key(PropertyKey::Color, LogicalSide::BlockStart),
        PropertyKey::Color
    );
    let value = PropertyValue::Opacity(0.5);
    assert_eq!(flow.value(value.clone(), LogicalSide::BlockStart), value);
}

#[test]
fn css_wide_markers_move_with_their_side() {
    let flow = FlowMapping::new(WritingMode::VerticalRl, Direction::Ltr);
    assert_eq!(
        flow.value(PropertyValue::MarginLeftInherit, LogicalSide::BlockStart),
        PropertyValue::MarginRightInherit
    );
    for (key, side, expected) in [
        (
            PropertyKey::MarginLeft,
            LogicalSide::InlineStart,
            PropertyKey::MarginTop,
        ),
        (
            PropertyKey::PaddingTop,
            LogicalSide::BlockEnd,
            PropertyKey::PaddingLeft,
        ),
        (
            PropertyKey::PaddingRight,
            LogicalSide::InlineEnd,
            PropertyKey::PaddingBottom,
        ),
    ] {
        assert_eq!(flow.key(key, side), expected);
    }
}

fn margins_of(writing_mode: WritingMode, direction: Direction) -> [PropertyKey; 4] {
    let flow = FlowMapping::new(writing_mode, direction);
    [
        LogicalSide::BlockStart,
        LogicalSide::InlineEnd,
        LogicalSide::BlockEnd,
        LogicalSide::InlineStart,
    ]
    .map(|side| flow.key(PropertyKey::MarginTop, side))
}

#[test]
fn sides_follow_the_abstract_to_physical_table() {
    use PropertyKey::{MarginBottom as B, MarginLeft as L, MarginRight as R, MarginTop as T};
    // Sides in the order block-start, inline-end, block-end, inline-start.
    for (writing_mode, direction, expected) in [
        (WritingMode::HorizontalTb, Direction::Ltr, [T, R, B, L]),
        (WritingMode::HorizontalTb, Direction::Rtl, [T, L, B, R]),
        (WritingMode::VerticalRl, Direction::Ltr, [R, B, L, T]),
        (WritingMode::VerticalRl, Direction::Rtl, [R, T, L, B]),
        (WritingMode::SidewaysRl, Direction::Ltr, [R, B, L, T]),
        (WritingMode::VerticalLr, Direction::Ltr, [L, B, R, T]),
        (WritingMode::VerticalLr, Direction::Rtl, [L, T, R, B]),
        (WritingMode::SidewaysLr, Direction::Ltr, [L, T, R, B]),
        (WritingMode::SidewaysLr, Direction::Rtl, [L, B, R, T]),
    ] {
        assert_eq!(
            margins_of(writing_mode, direction),
            expected,
            "{writing_mode:?} {direction:?}"
        );
    }
}

#[test]
fn inherit_markers_and_deferred_values_move_to_every_side() {
    use crate::property::DeferredValue;
    let flow = FlowMapping::new(WritingMode::VerticalRl, Direction::Ltr);
    // vertical-rl ltr: inline-start top, block-start right, inline-end
    // bottom, block-end left.
    let sides = [
        LogicalSide::InlineStart,
        LogicalSide::BlockStart,
        LogicalSide::InlineEnd,
        LogicalSide::BlockEnd,
    ];
    let markers = [
        PropertyValue::MarginTopInherit,
        PropertyValue::MarginRightInherit,
        PropertyValue::MarginBottomInherit,
        PropertyValue::MarginLeftInherit,
    ];
    for (side, marker) in sides.iter().zip(&markers) {
        assert_eq!(flow.value(PropertyValue::MarginLeftInherit, *side), *marker);
    }
    let deferred = |property: &'static str, key| {
        PropertyValue::Deferred(DeferredValue {
            property: SmolStr::new_static(property),
            value: "inherit".into(),
            key,
        })
    };
    for (from, names, keys) in [
        (
            deferred("margin-inline-start", PropertyKey::MarginLeft),
            ["margin-top", "margin-right", "margin-bottom", "margin-left"],
            [
                PropertyKey::MarginTop,
                PropertyKey::MarginRight,
                PropertyKey::MarginBottom,
                PropertyKey::MarginLeft,
            ],
        ),
        (
            deferred("padding-inline-start", PropertyKey::PaddingLeft),
            [
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
            ],
            [
                PropertyKey::PaddingTop,
                PropertyKey::PaddingRight,
                PropertyKey::PaddingBottom,
                PropertyKey::PaddingLeft,
            ],
        ),
    ] {
        for ((side, name), key) in sides.iter().zip(names).zip(keys) {
            assert_eq!(
                flow.value(from.clone(), *side),
                deferred(name, key),
                "{side:?}"
            );
        }
    }
    let other = deferred("color", PropertyKey::Color);
    assert_eq!(flow.value(other.clone(), LogicalSide::BlockStart), other);
}

#[test]
fn inset_values_and_keys_follow_the_abstract_to_physical_table() {
    use crate::property::LengthOrAuto;
    use PropertyKey::{Bottom as B, Left as L, Right as R, Top as T};
    let auto = LengthOrAuto::Auto;
    // Sides in the order block-start, inline-end, block-end, inline-start.
    let sides = [
        LogicalSide::BlockStart,
        LogicalSide::InlineEnd,
        LogicalSide::BlockEnd,
        LogicalSide::InlineStart,
    ];
    for (writing_mode, direction, expected) in [
        (WritingMode::HorizontalTb, Direction::Ltr, [T, R, B, L]),
        (WritingMode::HorizontalTb, Direction::Rtl, [T, L, B, R]),
        (WritingMode::VerticalRl, Direction::Ltr, [R, B, L, T]),
        (WritingMode::VerticalLr, Direction::Rtl, [L, T, R, B]),
        (WritingMode::SidewaysLr, Direction::Ltr, [L, T, R, B]),
    ] {
        let flow = FlowMapping::new(writing_mode, direction);
        for (side, key) in sides.iter().zip(expected) {
            assert_eq!(
                flow.key(PropertyKey::Top, *side),
                key,
                "{writing_mode:?} {direction:?}"
            );
            let value = match key {
                T => PropertyValue::Top(auto.clone()),
                R => PropertyValue::Right(auto.clone()),
                B => PropertyValue::Bottom(auto.clone()),
                _ => PropertyValue::Left(auto.clone()),
            };
            // Whichever physical value the parser stored, the value lands on the mapped side.
            assert_eq!(flow.value(PropertyValue::Left(auto.clone()), *side), value);
        }
    }
}

#[test]
fn every_flow_relative_longhand_name_is_recognised() {
    // The length check in `longhand_side` must admit the length of every name in its table.
    for (name, side) in [
        ("margin-inline-start", LogicalSide::InlineStart),
        ("margin-inline-end", LogicalSide::InlineEnd),
        ("margin-block-start", LogicalSide::BlockStart),
        ("margin-block-end", LogicalSide::BlockEnd),
        ("padding-inline-start", LogicalSide::InlineStart),
        ("padding-inline-end", LogicalSide::InlineEnd),
        ("padding-block-start", LogicalSide::BlockStart),
        ("padding-block-end", LogicalSide::BlockEnd),
        ("inset-inline-start", LogicalSide::InlineStart),
        ("inset-inline-end", LogicalSide::InlineEnd),
        ("inset-block-start", LogicalSide::BlockStart),
        ("inset-block-end", LogicalSide::BlockEnd),
        ("contain-intrinsic-inline-size", LogicalSide::InlineStart),
        ("contain-intrinsic-block-size", LogicalSide::BlockStart),
    ] {
        assert_eq!(longhand_side(name), Some(side), "{name}");
    }
}

#[test]
fn intrinsic_size_longhand_names_are_flow_relative_sides() {
    for (name, side) in [
        ("contain-intrinsic-inline-size", LogicalSide::InlineStart),
        ("Contain-Intrinsic-Block-Size", LogicalSide::BlockStart),
    ] {
        assert_eq!(longhand_side(name), Some(side), "{name}");
    }
    // The physical longhands and the shorthand are not flow-relative longhands.
    for name in [
        "contain-intrinsic-width",
        "contain-intrinsic-height",
        "contain-intrinsic-size",
        "inline-size",
    ] {
        assert_eq!(longhand_side(name), None, "{name}");
    }
}

#[test]
fn intrinsic_sizes_follow_the_axis_of_their_side() {
    use crate::property::ContainIntrinsicSize;
    use PropertyKey::{ContainIntrinsicHeight as H, ContainIntrinsicWidth as W};
    let size = ContainIntrinsicSize::NONE;
    // Sides in the order inline-start, inline-end, block-start, block-end.
    let sides = [
        LogicalSide::InlineStart,
        LogicalSide::InlineEnd,
        LogicalSide::BlockStart,
        LogicalSide::BlockEnd,
    ];
    for (writing_mode, direction, expected) in [
        (WritingMode::HorizontalTb, Direction::Ltr, [W, W, H, H]),
        (WritingMode::HorizontalTb, Direction::Rtl, [W, W, H, H]),
        (WritingMode::VerticalRl, Direction::Ltr, [H, H, W, W]),
        (WritingMode::VerticalLr, Direction::Rtl, [H, H, W, W]),
        (WritingMode::SidewaysRl, Direction::Ltr, [H, H, W, W]),
        (WritingMode::SidewaysLr, Direction::Rtl, [H, H, W, W]),
    ] {
        let flow = FlowMapping::new(writing_mode, direction);
        for (side, key) in sides.iter().zip(expected) {
            // Whichever longhand the parser stored, the key and value land on the mapped axis.
            for stored in [W, H] {
                assert_eq!(
                    flow.key(stored, *side),
                    key,
                    "{writing_mode:?} {direction:?} {side:?}"
                );
            }
            let value = match key {
                W => PropertyValue::ContainIntrinsicWidth(size),
                _ => PropertyValue::ContainIntrinsicHeight(size),
            };
            assert_eq!(
                flow.value(PropertyValue::ContainIntrinsicWidth(size), *side),
                value
            );
            assert_eq!(
                flow.value(PropertyValue::ContainIntrinsicHeight(size), *side),
                value
            );
        }
    }
}

#[test]
fn intrinsic_sizes_follow_the_size_writing_mode_and_the_sides_the_layout_one() {
    use crate::property::{ContainIntrinsicSize, LengthOrAuto};
    // Block layout places the box on horizontal axes, but it declares a vertical writing mode.
    let flow = FlowMapping {
        size_writing_mode: WritingMode::VerticalRl,
        ..FlowMapping::HORIZONTAL
    };
    let size = ContainIntrinsicSize::NONE;
    assert_eq!(
        flow.key(PropertyKey::ContainIntrinsicWidth, LogicalSide::InlineStart),
        PropertyKey::ContainIntrinsicHeight
    );
    assert_eq!(
        flow.value(
            PropertyValue::ContainIntrinsicHeight(size),
            LogicalSide::BlockStart
        ),
        PropertyValue::ContainIntrinsicWidth(size)
    );
    // A margin still follows the writing mode of the layout.
    assert_eq!(
        flow.key(PropertyKey::MarginLeft, LogicalSide::InlineStart),
        PropertyKey::MarginLeft
    );
    assert_eq!(
        flow.value(
            PropertyValue::MarginLeft(LengthOrAuto::Auto),
            LogicalSide::BlockStart
        ),
        PropertyValue::MarginTop(LengthOrAuto::Auto)
    );
}

#[test]
fn deferred_intrinsic_sizes_move_to_the_mapped_longhand() {
    use crate::property::DeferredValue;
    let flow = FlowMapping::new(WritingMode::VerticalLr, Direction::Ltr);
    let deferred = |property: &'static str, key| {
        PropertyValue::Deferred(DeferredValue {
            property: SmolStr::new_static(property),
            value: "var(--x)".into(),
            key,
        })
    };
    // In a vertical box the inline size is the height and the block size the width.
    assert_eq!(
        flow.value(
            deferred(
                "contain-intrinsic-inline-size",
                PropertyKey::ContainIntrinsicWidth
            ),
            LogicalSide::InlineStart
        ),
        deferred(
            "contain-intrinsic-height",
            PropertyKey::ContainIntrinsicHeight
        )
    );
    assert_eq!(
        flow.value(
            deferred(
                "contain-intrinsic-block-size",
                PropertyKey::ContainIntrinsicHeight
            ),
            LogicalSide::BlockStart
        ),
        deferred(
            "contain-intrinsic-width",
            PropertyKey::ContainIntrinsicWidth
        )
    );
}

#[test]
fn deferred_inset_values_move_to_every_side() {
    use crate::property::DeferredValue;
    let flow = FlowMapping::HORIZONTAL;
    let deferred = |property: &'static str, key| {
        PropertyValue::Deferred(DeferredValue {
            property: SmolStr::new_static(property),
            value: "var(--x)".into(),
            key,
        })
    };
    let from = deferred("inset-inline-start", PropertyKey::Left);
    for (side, name, key) in [
        (LogicalSide::BlockStart, "top", PropertyKey::Top),
        (LogicalSide::InlineEnd, "right", PropertyKey::Right),
        (LogicalSide::BlockEnd, "bottom", PropertyKey::Bottom),
        (LogicalSide::InlineStart, "left", PropertyKey::Left),
    ] {
        assert_eq!(
            flow.value(from.clone(), side),
            deferred(name, key),
            "{side:?}"
        );
    }
}
