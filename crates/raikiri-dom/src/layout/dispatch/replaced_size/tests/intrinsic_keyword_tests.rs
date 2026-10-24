use super::*;
use crate::layout::dispatch::content_limits::ContentLimits;
use crate::layout::dispatch::replaced_size::keyword_bounds;
use crate::layout::protocol::RequestedAxis;
use raikiri_layout_geometry::{Edges, Size};
use raikiri_layout_style::values::{BoxSizing, LengthPercentage, LengthPercentageAuto};

fn inputs() -> LayoutInput {
    LayoutInput {
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
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

fn limits(min: Size<Dimension>, max: Size<Dimension>) -> ContentLimits {
    ContentLimits { min, max }
}

fn natural_size() -> Size<f32> {
    Size {
        width: 10.0,
        height: 10.0,
    }
}

fn ratio_style(width: Dimension, height: Dimension) -> Style {
    Style {
        aspect_ratio: Some(1.0),
        size: Size { width, height },
        ..Style::default()
    }
}

#[test]
fn min_width_min_content_transfers_height_bound_through_natural_ratio() {
    let style = ratio_style(Dimension::length(10.0), Dimension::length(100.0));
    let limits = limits(
        Size {
            width: Dimension::min_content(),
            height: Dimension::auto(),
        },
        Size::splat(Dimension::auto()),
    );

    assert_eq!(
        auto_sides_size(&style, &inputs(), natural_size(), limits, true),
        Some(Size {
            width: 100.0,
            height: 100.0,
        })
    );
}

#[test]
fn min_height_min_content_transfers_width_bound_through_natural_ratio() {
    let style = ratio_style(Dimension::length(100.0), Dimension::length(10.0));
    let limits = limits(
        Size {
            width: Dimension::auto(),
            height: Dimension::min_content(),
        },
        Size::splat(Dimension::auto()),
    );

    assert_eq!(
        auto_sides_size(&style, &inputs(), natural_size(), limits, true),
        Some(Size {
            width: 100.0,
            height: 100.0,
        })
    );
}

#[test]
fn max_width_min_content_transfers_height_bound_through_natural_ratio() {
    let style = ratio_style(Dimension::length(1000.0), Dimension::length(100.0));
    let limits = limits(
        Size::splat(Dimension::auto()),
        Size {
            width: Dimension::min_content(),
            height: Dimension::auto(),
        },
    );

    assert_eq!(
        auto_sides_size(&style, &inputs(), natural_size(), limits, true),
        Some(Size {
            width: 100.0,
            height: 100.0,
        })
    );
}

#[test]
fn max_height_min_content_transfers_width_bound_through_natural_ratio() {
    let style = ratio_style(Dimension::length(100.0), Dimension::length(1000.0));
    let limits = limits(
        Size::splat(Dimension::auto()),
        Size {
            width: Dimension::auto(),
            height: Dimension::min_content(),
        },
    );

    assert_eq!(
        auto_sides_size(&style, &inputs(), natural_size(), limits, true),
        Some(Size {
            width: 100.0,
            height: 100.0,
        })
    );
}

#[test]
fn missing_ratio_or_natural_dimensions_keeps_existing_leaf_fallback() {
    let limits = limits(
        Size {
            width: Dimension::min_content(),
            height: Dimension::auto(),
        },
        Size::splat(Dimension::auto()),
    );
    let without_ratio = Style::default();
    let with_ratio = ratio_style(Dimension::auto(), Dimension::auto());

    assert_eq!(
        auto_sides_size(&without_ratio, &inputs(), natural_size(), limits, true),
        None
    );
    assert_eq!(
        auto_sides_size(
            &with_ratio,
            &inputs(),
            Size {
                width: 0.0,
                height: 10.0,
            },
            limits,
            true,
        ),
        None
    );
}

#[test]
fn content_box_min_content_bound_subtracts_padding_from_border_box_size() {
    let style = Style {
        box_sizing: BoxSizing::ContentBox,
        padding: Edges {
            left: LengthPercentage::length(5.0),
            right: LengthPercentage::length(5.0),
            ..Style::default().padding
        },
        border: Edges {
            left: LengthPercentage::length(1.0),
            right: LengthPercentage::length(1.0),
            ..Style::default().border
        },
        ..ratio_style(Dimension::auto(), Dimension::auto())
    };
    let limits = limits(
        Size {
            width: Dimension::min_content(),
            height: Dimension::auto(),
        },
        Size::splat(Dimension::auto()),
    );

    assert_eq!(
        keyword_bounds(&style, &inputs(), natural_size(), limits, true)
            .unwrap()
            .width
            .min,
        Some(22.0)
    );
    assert_eq!(
        auto_sides_size(&style, &inputs(), natural_size(), limits, true),
        Some(natural_size())
    );
}

#[test]
fn content_box_max_content_bound_subtracts_padding_from_border_box_size() {
    let style = Style {
        box_sizing: BoxSizing::ContentBox,
        size: Size {
            width: Dimension::length(100.0),
            height: Dimension::auto(),
        },
        padding: Edges {
            left: LengthPercentage::length(5.0),
            right: LengthPercentage::length(5.0),
            ..Style::default().padding
        },
        border: Edges {
            left: LengthPercentage::length(1.0),
            right: LengthPercentage::length(1.0),
            ..Style::default().border
        },
        ..ratio_style(Dimension::auto(), Dimension::auto())
    };
    let limits = limits(
        Size::splat(Dimension::auto()),
        Size {
            width: Dimension::max_content(),
            height: Dimension::auto(),
        },
    );

    assert_eq!(
        keyword_bounds(&style, &inputs(), natural_size(), limits, true)
            .unwrap()
            .width
            .max,
        Some(22.0)
    );
    assert_eq!(
        auto_sides_size(&style, &inputs(), natural_size(), limits, true),
        Some(natural_size())
    );
}

#[test]
fn keyword_bounds_transfer_all_min_and_max_sides_through_the_ratio() {
    let style = ratio_style(Dimension::auto(), Dimension::auto());
    let limits = limits(
        Size {
            width: Dimension::min_content(),
            height: Dimension::min_content(),
        },
        Size {
            width: Dimension::max_content(),
            height: Dimension::max_content(),
        },
    );

    let bounds = keyword_bounds(&style, &inputs(), natural_size(), limits, true).unwrap();

    assert_eq!(bounds.width.min, Some(10.0));
    assert_eq!(bounds.width.max, Some(10.0));
    assert_eq!(bounds.height.min, Some(10.0));
    assert_eq!(bounds.height.max, Some(10.0));
}

#[test]
fn keyword_bounds_reject_nonpositive_natural_dimensions() {
    let limits = limits(
        Size {
            width: Dimension::min_content(),
            height: Dimension::auto(),
        },
        Size::splat(Dimension::auto()),
    );

    assert_eq!(
        keyword_bounds(
            &ratio_style(Dimension::auto(), Dimension::auto()),
            &inputs(),
            Size {
                width: 0.0,
                height: 10.0,
            },
            limits,
            true,
        ),
        None
    );
}

#[test]
fn conflicting_numeric_bounds_raise_max_to_min() {
    let style = Style {
        aspect_ratio: Some(1.0),
        min_size: Size {
            width: LengthPercentageAuto::length(100.0),
            height: LengthPercentageAuto::auto(),
        },
        max_size: Size {
            width: LengthPercentageAuto::length(50.0),
            height: LengthPercentageAuto::auto(),
        },
        ..Style::default()
    };
    let limits = limits(
        Size::splat(Dimension::auto()),
        Size {
            width: Dimension::min_content(),
            height: Dimension::auto(),
        },
    );

    assert_eq!(
        auto_sides_size(&style, &inputs(), natural_size(), limits, true),
        Some(Size {
            width: 100.0,
            height: 100.0,
        })
    );
}

#[test]
fn cyclic_percentage_still_uses_zero_basis_with_intrinsic_limits() {
    let mut arena = Vec::new();
    let height = Dimension::calc(crate::layout::dispatch::push_calc(
        &mut arena,
        raikiri_style::property::CalcLengthPercentage {
            percent: 50.0,
            px: 40.0,
        },
        true,
    ));
    let style = ratio_style(Dimension::auto(), height);
    let limits = limits(
        Size {
            width: Dimension::min_content(),
            height: Dimension::auto(),
        },
        Size::splat(Dimension::auto()),
    );
    let mut layout_inputs = inputs();
    layout_inputs.available_space.width = AvailableSpace::MinContent;

    assert_eq!(
        auto_sides_size(&style, &layout_inputs, natural_size(), limits, true),
        Some(Size {
            width: 40.0,
            height: 40.0,
        })
    );
}
