use super::auto_sides_size;
use crate::LayoutStyle as Style;
use crate::layout::dispatch::push_calc;
use crate::layout::protocol::{AvailableSpace, LayoutInput, RequestedAxis, RunMode, SizingMode};
use raikiri_layout_geometry::{Line, Size};
use raikiri_layout_style::values::{Dimension, StyleSize};
use raikiri_style::property::CalcLengthPercentage;

mod intrinsic_keyword_tests;
mod ratio_box_tests;

#[test]
fn a_percentage_calc_on_a_replaced_min_content_width_uses_zero_basis() {
    let mut arena = Vec::new();
    let width = Dimension::calc(push_calc(
        &mut arena,
        CalcLengthPercentage {
            percent: 50.0,
            px: 40.0,
        },
        true,
    ));
    let style = Style {
        aspect_ratio: Some(2.0),
        size: StyleSize {
            width,
            height: Dimension::auto(),
        },
        ..Style::default()
    };
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MinContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };

    assert_eq!(
        auto_sides_size(
            &style,
            &inputs,
            Size {
                width: 100.0,
                height: 50.0,
            },
            crate::layout::dispatch::content_limits::ContentLimits::NONE,
            true,
        ),
        Some(Size {
            width: 40.0,
            height: 20.0,
        })
    );
}
