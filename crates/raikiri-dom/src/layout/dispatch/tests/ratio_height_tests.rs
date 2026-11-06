use super::*;
use crate::layout::style_views::tests::fixture_with_styles;
use raikiri_layout_style::values::ExpandedDimension;

#[test]
fn calc_height_takes_the_ratio_only_without_a_percentage_basis() {
    for (basis, expected_height) in [(None, Some(50.0)), (Some(80.0), None)] {
        let (mut doc, cascade, child, _) = fixture_with_styles(
            "display:block",
            "display:flex;width:100px;height:calc(50% + 2px);aspect-ratio:2/1",
        );
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let height = block_flow::style_input::physical(&tree, child).size.height;
        let ExpandedDimension::Calc(calc) = height.expand() else {
            panic!("the fixture must retain its mixed calc expression");
        };
        assert!(calc.has_percentage());
        assert_eq!(height.maybe_resolve(basis), basis.map(|_| 42.0));
        let inputs = LayoutInput {
            run_mode: RunMode::ComputeSize,
            axis: RequestedAxis::Vertical,
            known_dimensions: Size {
                width: Some(100.0),
                height: None,
            },
            known_dimensions_are_definite: Size {
                width: true,
                height: false,
            },
            parent_size: Size {
                width: Some(800.0),
                height: basis,
            },
            ..LayoutInput::HIDDEN
        };
        let mut seen = None;
        with_ratio_height(
            &mut tree,
            NodeId::from(child),
            inputs,
            flex_layout_sized,
            |_, settled| {
                seen = Some((
                    settled.known_dimensions.height,
                    settled.known_dimensions_are_definite.height,
                ));
                LayoutOutput::HIDDEN
            },
        );
        assert_eq!(seen, Some((expected_height, expected_height.is_some())));
    }
}

#[test]
fn calc_ratio_height_preserves_known_heights_and_width_only_probes() {
    for (axis, known_height) in [
        (RequestedAxis::Both, Some(23.0)),
        (RequestedAxis::Horizontal, None),
    ] {
        let (mut doc, cascade, child, _) = fixture_with_styles(
            "display:block",
            "display:flex;width:100px;height:calc(50% + 2px);aspect-ratio:2/1",
        );
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let inputs = LayoutInput {
            run_mode: RunMode::ComputeSize,
            axis,
            known_dimensions: Size {
                width: Some(100.0),
                height: known_height,
            },
            known_dimensions_are_definite: Size {
                width: true,
                height: known_height.is_some(),
            },
            ..LayoutInput::HIDDEN
        };
        let mut seen = None;
        with_ratio_height(
            &mut tree,
            NodeId::from(child),
            inputs,
            |_, _, _| panic!("a settled height or width-only probe must not measure content"),
            |_, forwarded| {
                seen = Some((
                    forwarded.known_dimensions,
                    forwarded.known_dimensions_are_definite,
                ));
                LayoutOutput::HIDDEN
            },
        );
        assert_eq!(
            seen,
            Some((
                inputs.known_dimensions,
                inputs.known_dimensions_are_definite
            ))
        );
    }
}
