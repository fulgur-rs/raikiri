use super::auto_sides_size;
use crate::LayoutStyle as Style;
use crate::taffy_impl::push_calc;
use raikiri_style::property::CalcLengthPercentage;
use taffy::{AvailableSpace, Dimension, LayoutInput, Line, RunMode, Size, SizingMode};

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
        size: Size {
            width,
            height: Dimension::auto(),
        },
        ..Style::default()
    };
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: taffy::RequestedAxis::Both,
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
            crate::taffy_impl::content_limits::ContentLimits::NONE,
            true,
        ),
        Some(Size {
            width: 40.0,
            height: 20.0,
        })
    );
}
