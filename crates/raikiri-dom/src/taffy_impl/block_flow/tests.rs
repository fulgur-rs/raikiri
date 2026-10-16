use crate::Document;
use crate::taffy_impl::block_layout;
use taffy::{
    AvailableSpace, Dimension, Display, Float, LayoutInput, LengthPercentage, LengthPercentageAuto,
    Line, NodeId, RequestedAxis, RunMode, Size, SizingMode, Style,
};

fn inputs() -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width: Some(200.0),
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        parent_size: Size {
            width: Some(200.0),
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::Definite(200.0),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

#[test]
fn minimum_height_keeps_the_last_child_margin_inside_the_block() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Style {
            display: Display::FlowRoot,
            ..Style::default()
        },
        None::<&str>,
    );
    let child = tree.append_element(
        Some(root),
        "div",
        Style {
            display: Display::Block,
            min_size: Size {
                width: LengthPercentageAuto::auto(),
                height: LengthPercentageAuto::length(80.0),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    tree.append_element(
        Some(child),
        "div",
        Style {
            display: Display::Block,
            size: Size {
                width: Dimension::auto(),
                height: Dimension::length(20.0),
            },
            margin: taffy::Rect {
                bottom: LengthPercentageAuto::length(100.0),
                ..Style::<String>::default().margin
            },
            ..Style::default()
        },
        None::<&str>,
    );
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(tree.nodes[child].unrounded_layout.size.height, 120.0);
    assert_eq!(output.size.height, 120.0);
}

#[test]
fn auto_height_encloses_a_float_before_end_padding() {
    for (height, max, expected) in [
        (None, None, 120.0),
        (Some(30.0), None, 70.0),
        (None, Some(60.0), 100.0),
    ] {
        let mut tree = Document::new();
        let root = tree.append_element(
            Some(0),
            "div",
            Style {
                display: Display::FlowRoot,
                box_sizing: taffy::BoxSizing::ContentBox,
                size: Size {
                    width: Dimension::auto(),
                    height: height.map_or_else(Dimension::auto, Dimension::length),
                },
                max_size: Size {
                    width: LengthPercentageAuto::auto(),
                    height: max
                        .map_or_else(LengthPercentageAuto::auto, LengthPercentageAuto::length),
                },
                padding: taffy::Rect {
                    bottom: LengthPercentage::length(40.0),
                    ..Style::<String>::default().padding
                },
                ..Style::default()
            },
            None::<&str>,
        );
        tree.append_element(
            Some(root),
            "div",
            Style {
                display: Display::Block,
                float: Float::Left,
                size: Size {
                    width: Dimension::length(20.0),
                    height: Dimension::length(80.0),
                },
                ..Style::default()
            },
            None::<&str>,
        );
        let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
        assert_eq!(output.size.height, expected);
    }
}

#[test]
fn fixed_children_do_not_participate_in_block_flow() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Style {
            display: Display::FlowRoot,
            ..Style::default()
        },
        None::<&str>,
    );
    let fixed = tree.append_element(
        Some(root),
        "div",
        Style {
            position: taffy::Position::Fixed,
            size: Size {
                width: Dimension::length(20.0),
                height: Dimension::length(30.0),
            },
            inset: taffy::Rect {
                left: LengthPercentageAuto::length(10.0),
                top: LengthPercentageAuto::length(15.0),
                ..Style::<String>::default().inset
            },
            ..Style::default()
        },
        None::<&str>,
    );
    let sibling = tree.append_element(
        Some(root),
        "div",
        Style {
            size: Size {
                width: Dimension::length(40.0),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(output.size.height, 10.0);
    assert_eq!(tree.nodes[sibling].unrounded_layout.location.y, 0.0);
    assert_eq!(
        tree.nodes[fixed].unrounded_layout.location,
        taffy::Point { x: 10.0, y: 15.0 }
    );
}

#[test]
fn absolute_child_overflow_is_reported_without_growing_the_block() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Style {
            display: Display::FlowRoot,
            position: taffy::Position::Relative,
            ..Style::default()
        },
        None::<&str>,
    );
    tree.append_element(
        Some(root),
        "div",
        Style {
            position: taffy::Position::Absolute,
            size: Size {
                width: Dimension::length(40.0),
                height: Dimension::length(30.0),
            },
            inset: taffy::Rect {
                left: LengthPercentageAuto::length(210.0),
                top: LengthPercentageAuto::length(15.0),
                ..Style::<String>::default().inset
            },
            ..Style::default()
        },
        None::<&str>,
    );
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(
        output.size,
        Size {
            width: 200.0,
            height: 0.0
        }
    );
    assert_eq!(output.scrollable_overflow_rect.right, 250.0);
    assert_eq!(output.scrollable_overflow_rect.bottom, 45.0);
}
