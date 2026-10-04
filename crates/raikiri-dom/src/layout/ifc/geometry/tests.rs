use super::IfcAxes;
use shodo::geometry::{Direction, LogicalRect, PhysicalSize, WritingMode};

#[test]
fn inline_and_block_extents_follow_the_writing_mode() {
    let size = PhysicalSize {
        width: 80.0,
        height: 30.0,
    };
    for (mode, inline, block) in [
        (WritingMode::HorizontalTb, 80.0, 30.0),
        (WritingMode::VerticalRl, 30.0, 80.0),
        (WritingMode::VerticalLr, 30.0, 80.0),
    ] {
        let axes = IfcAxes::new(mode, Direction::Ltr);
        assert_eq!(axes.inline_extent(size), inline, "{mode:?}");
        assert_eq!(axes.block_extent(size), block, "{mode:?}");
    }
}

#[test]
fn vertical_rl_maps_rectangles() {
    let axes = IfcAxes::new(WritingMode::VerticalRl, Direction::Ltr);
    let size = PhysicalSize {
        width: 80.0,
        height: 30.0,
    };

    assert_eq!(
        axes.rect(
            size,
            LogicalRect {
                inline_start: 7.0,
                block_start: 11.0,
                inline_size: 5.0,
                block_size: 9.0,
            }
        ),
        shodo::geometry::PhysicalRect {
            x: 60.0,
            y: 7.0,
            width: 9.0,
            height: 5.0,
        }
    );
}

#[test]
fn vertical_lr_maps_block_progression_from_the_left() {
    let axes = IfcAxes::new(WritingMode::VerticalLr, Direction::Ltr);
    let size = PhysicalSize {
        width: 80.0,
        height: 30.0,
    };

    assert_eq!(
        axes.rect(
            size,
            LogicalRect {
                inline_start: 7.0,
                block_start: 11.0,
                inline_size: 5.0,
                block_size: 9.0,
            }
        ),
        shodo::geometry::PhysicalRect {
            x: 11.0,
            y: 7.0,
            width: 9.0,
            height: 5.0,
        }
    );
}

#[test]
fn horizontal_rtl_maps_inline_box_rectangles() {
    let axes = IfcAxes::new(WritingMode::HorizontalTb, Direction::Rtl);
    let size = PhysicalSize {
        width: 80.0,
        height: 30.0,
    };
    assert_eq!(
        axes.rect(
            size,
            LogicalRect {
                inline_start: 7.0,
                block_start: 11.0,
                inline_size: 5.0,
                block_size: 9.0,
            }
        ),
        shodo::geometry::PhysicalRect {
            x: 68.0,
            y: 11.0,
            width: 5.0,
            height: 9.0,
        }
    );
}
