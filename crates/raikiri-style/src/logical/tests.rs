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
fn non_box_keys_keep_their_slot() {
    let flow = FlowMapping {
        writing_mode: WritingMode::VerticalRl,
        direction: Direction::Ltr,
    };
    assert_eq!(
        flow.key(PropertyKey::Color, LogicalSide::BlockStart),
        PropertyKey::Color
    );
    let value = PropertyValue::Opacity(0.5);
    assert_eq!(flow.value(&value, LogicalSide::BlockStart), value);
}

#[test]
fn css_wide_markers_move_with_their_side() {
    let flow = FlowMapping {
        writing_mode: WritingMode::VerticalRl,
        direction: Direction::Ltr,
    };
    assert_eq!(
        flow.value(&PropertyValue::MarginLeftInherit, LogicalSide::BlockStart),
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
