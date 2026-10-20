use super::*;
use crate::taffy_impl::content_limits::ContentLimits;
use crate::taffy_impl::replaced_size::keyword_bounds;
use taffy::{BoxSizing, LengthPercentage, Rect};

fn inputs(known_width: Option<f32>) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: taffy::RequestedAxis::Both,
        known_dimensions: Size {
            width: known_width,
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

/// A `border-box` replaced box with a ratio of `ratio` and 80px of padding on
/// the right and 50px at the bottom, so that the content box and the border
/// box have different ratios.
fn padded_border_box(ratio: f32) -> Style {
    Style {
        aspect_ratio: Some(ratio),
        box_sizing: BoxSizing::BorderBox,
        padding: Rect {
            left: LengthPercentage::length(0.0),
            right: LengthPercentage::length(80.0),
            top: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(50.0),
        },
        ..Style::default()
    }
}

fn natural() -> Size<f32> {
    Size {
        width: 20.0,
        height: 50.0,
    }
}

fn size(width: f32, height: f32) -> Option<Size<f32>> {
    Some(Size { width, height })
}

#[test]
fn a_width_the_caller_fixes_gives_a_border_box_height_through_the_border_box_ratio() {
    // A square border box 100px wide leaves a 20px wide content box, which is
    // 50px high once the 50px of padding at the bottom is counted in.
    let style = padded_border_box(1.0);
    assert_eq!(
        auto_sides_size(
            &style,
            &inputs(Some(100.0)),
            natural(),
            ContentLimits::NONE,
            false
        ),
        size(20.0, 50.0)
    );
}

#[test]
fn a_content_box_ratio_relates_the_content_boxes_of_a_border_box_box() {
    let style = padded_border_box(1.0);
    assert_eq!(
        auto_sides_size(
            &style,
            &inputs(Some(100.0)),
            natural(),
            ContentLimits::NONE,
            true
        ),
        size(20.0, 20.0)
    );
}

#[test]
fn a_content_box_box_relates_its_content_boxes_whatever_the_ratio_is() {
    let style = Style {
        box_sizing: BoxSizing::ContentBox,
        ..padded_border_box(1.0)
    };
    assert_eq!(
        auto_sides_size(
            &style,
            &inputs(Some(100.0)),
            natural(),
            ContentLimits::NONE,
            false
        ),
        size(20.0, 20.0)
    );
}

#[test]
fn an_authored_side_gives_the_other_side_through_the_border_box_ratio() {
    let width = Style {
        size: Size {
            width: Dimension::length(100.0),
            height: Dimension::auto(),
        },
        ..padded_border_box(1.0)
    };
    assert_eq!(
        auto_sides_size(&width, &inputs(None), natural(), ContentLimits::NONE, false),
        size(20.0, 50.0)
    );
    let height = Style {
        size: Size {
            width: Dimension::auto(),
            height: Dimension::length(100.0),
        },
        ..padded_border_box(1.0)
    };
    assert_eq!(
        auto_sides_size(
            &height,
            &inputs(None),
            natural(),
            ContentLimits::NONE,
            false
        ),
        size(20.0, 50.0)
    );
}

#[test]
fn a_border_box_smaller_than_its_padding_has_an_empty_content_box() {
    // A 4:1 border box 100px wide is 25px high, less than the 50px of padding
    // at the bottom.
    let style = padded_border_box(4.0);
    assert_eq!(
        auto_sides_size(
            &style,
            &inputs(Some(100.0)),
            natural(),
            ContentLimits::NONE,
            false
        ),
        size(20.0, 0.0)
    );
}

#[test]
fn a_ratio_the_box_states_replaces_the_natural_ratio_of_its_natural_size() {
    // The 20px natural width stays and the border box is 2:1: 100px wide and
    // 50px high, an empty content box.
    let style = padded_border_box(2.0);
    assert_eq!(
        auto_sides_size(&style, &inputs(None), natural(), ContentLimits::NONE, false),
        size(20.0, 0.0)
    );
    // A `min-height` of 200px moves the width along the border box ratio.
    let min_height = Style {
        min_size: Size {
            width: taffy::LengthPercentageAuto::auto(),
            height: taffy::LengthPercentageAuto::length(200.0),
        },
        ..padded_border_box(2.0)
    };
    assert_eq!(
        auto_sides_size(
            &min_height,
            &inputs(None),
            natural(),
            ContentLimits::NONE,
            false
        ),
        size(320.0, 150.0)
    );
}

#[test]
fn a_content_box_ratio_keeps_the_natural_size() {
    let style = padded_border_box(0.4);
    assert_eq!(
        auto_sides_size(&style, &inputs(None), natural(), ContentLimits::NONE, true),
        size(20.0, 50.0)
    );
}

#[test]
fn keyword_bounds_transfer_across_the_border_box_ratio() {
    // A 100px wide border box of ratio 2 is 50px high, 40px of it content once
    // 10px of padding at the bottom is taken off. A content box ratio would
    // give a content box of 60 by 30, so a border box 40px high.
    let style = Style {
        size: Size {
            width: Dimension::length(100.0),
            height: Dimension::auto(),
        },
        padding: Rect {
            left: LengthPercentage::length(0.0),
            right: LengthPercentage::length(40.0),
            top: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(10.0),
        },
        ..padded_border_box(2.0)
    };
    let limits = ContentLimits {
        min: Size {
            width: Dimension::auto(),
            height: Dimension::min_content(),
        },
        max: Size::auto(),
    };
    let border_box = keyword_bounds(&style, &inputs(None), natural(), limits, false).unwrap();
    let content_box = keyword_bounds(&style, &inputs(None), natural(), limits, true).unwrap();

    assert_eq!(border_box.height.min, Some(50.0));
    assert_eq!(content_box.height.min, Some(40.0));
}

#[test]
fn keyword_bounds_carry_a_height_bound_to_the_width_across_the_border_box_ratio() {
    // The `min-height: min-content` of the 50px high image is a border box
    // 60px high with 10px of padding at the bottom. In a 2:1 border box that
    // is 120px wide, and the 40px of padding at the right leaves a content
    // box of 80px. A content box ratio would double the 50px high content box
    // instead.
    let style = Style {
        padding: Rect {
            left: LengthPercentage::length(0.0),
            right: LengthPercentage::length(40.0),
            top: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(10.0),
        },
        ..padded_border_box(2.0)
    };
    let limits = ContentLimits {
        min: Size {
            width: Dimension::auto(),
            height: Dimension::min_content(),
        },
        max: Size::auto(),
    };
    let border_box = keyword_bounds(&style, &inputs(None), natural(), limits, false).unwrap();
    let content_box = keyword_bounds(&style, &inputs(None), natural(), limits, true).unwrap();

    assert_eq!(border_box.height.min, Some(60.0));
    assert_eq!(border_box.width.min, Some(120.0));
    assert_eq!(content_box.width.min, Some(140.0));
}

#[test]
fn a_keyword_width_takes_the_border_box_width_the_authored_height_gives() {
    // A 2:1 border box 100px high is 200px wide; the content box ratio would
    // give the 90px high content box 180px and a 220px wide border box.
    let style = Style {
        size: Size {
            width: Dimension::auto(),
            height: Dimension::length(100.0),
        },
        padding: Rect {
            left: LengthPercentage::length(0.0),
            right: LengthPercentage::length(40.0),
            top: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(10.0),
        },
        ..padded_border_box(2.0)
    };
    let limits = ContentLimits {
        min: Size {
            width: Dimension::min_content(),
            height: Dimension::auto(),
        },
        max: Size::auto(),
    };
    let border_box = keyword_bounds(&style, &inputs(None), natural(), limits, false).unwrap();
    let content_box = keyword_bounds(&style, &inputs(None), natural(), limits, true).unwrap();

    assert_eq!(border_box.width.min, Some(200.0));
    assert_eq!(content_box.width.min, Some(220.0));
}

#[test]
fn a_keyword_height_never_falls_below_the_padding_across_the_border_box_ratio() {
    // A 4:1 border box 100px wide is 25px high, less than the 50px of padding
    // at the bottom, so the content box the width gives is empty and a
    // `max-height` that stands for it is the padding alone.
    let style = Style {
        size: Size {
            width: Dimension::length(100.0),
            height: Dimension::auto(),
        },
        ..padded_border_box(4.0)
    };
    let limits = ContentLimits {
        min: Size::auto(),
        max: Size {
            width: Dimension::auto(),
            height: Dimension::max_content(),
        },
    };
    let bounds = keyword_bounds(&style, &inputs(None), natural(), limits, false).unwrap();

    assert_eq!(bounds.height.max, Some(50.0));
}

/// A `border-box` replaced box with a ratio of 1 and 20px of padding on the
/// right and 10px at the bottom, its `min-*` and `max-*` sizes as given.
fn small_padded_square(
    min: Size<taffy::LengthPercentageAuto>,
    max: Size<taffy::LengthPercentageAuto>,
) -> Style {
    Style {
        padding: Rect {
            left: LengthPercentage::length(0.0),
            right: LengthPercentage::length(20.0),
            top: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(10.0),
        },
        min_size: min,
        max_size: max,
        ..padded_border_box(1.0)
    }
}

#[test]
fn a_stated_ratio_brings_the_border_box_within_the_min_and_max_sizes() {
    use taffy::LengthPercentageAuto as Length;
    let none = Size {
        width: Length::auto(),
        height: Length::auto(),
    };
    // The natural 20px width makes a 40px square border box. Each bound
    // moves the border box along its ratio, and a browser gives the border
    // boxes 80x80, 30x30 and 30x30.
    for (name, min, max, expected) in [
        (
            "min-width: 80px",
            Size {
                width: Length::length(80.0),
                height: Length::auto(),
            },
            none,
            size(60.0, 70.0),
        ),
        (
            "max-width: 30px",
            Size {
                width: Length::auto(),
                height: Length::auto(),
            },
            Size {
                width: Length::length(30.0),
                height: Length::auto(),
            },
            size(10.0, 20.0),
        ),
        (
            "max-height: 30px",
            none,
            Size {
                width: Length::auto(),
                height: Length::length(30.0),
            },
            size(10.0, 20.0),
        ),
    ] {
        let style = small_padded_square(min, max);
        assert_eq!(
            auto_sides_size(&style, &inputs(None), natural(), ContentLimits::NONE, false),
            expected,
            "{name}"
        );
    }
}
