use super::*;
use raikiri_style::ComputedLengthPercentageOrAuto::Px;

/// Each physical margin lands on the logical side it faces in the line.
#[test]
fn margins_map_to_the_logical_sides_of_the_line() {
    let mut cv = ComputedValues::initial();
    cv.margin = Sides {
        top: Px(1.0),
        right: Px(2.0),
        bottom: Px(3.0),
        left: Px(4.0),
    };
    let generated = GeneratedAtomic::new(0, &cv);
    let sides = |writing_mode, rtl| {
        let margins = generated
            .size(None, (None, None), writing_mode, rtl)
            .margins;
        (
            margins.inline_start,
            margins.inline_end,
            margins.block_start,
            margins.block_end,
        )
    };
    assert_eq!(
        sides(WritingMode::HorizontalTb, false),
        (4.0, 2.0, 1.0, 3.0)
    );
    assert_eq!(sides(WritingMode::HorizontalTb, true), (2.0, 4.0, 1.0, 3.0));
    assert_eq!(sides(WritingMode::VerticalRl, false), (1.0, 3.0, 2.0, 4.0));
    assert_eq!(sides(WritingMode::SidewaysRl, true), (3.0, 1.0, 2.0, 4.0));
    assert_eq!(sides(WritingMode::VerticalLr, false), (1.0, 3.0, 4.0, 2.0));
    assert_eq!(sides(WritingMode::SidewaysLr, false), (3.0, 1.0, 4.0, 2.0));
}
