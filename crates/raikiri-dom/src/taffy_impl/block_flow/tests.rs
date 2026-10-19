use crate::Document;
use crate::layout::context::LayoutSession;
use taffy::{
    AvailableSpace, Dimension, Display, Float, LayoutInput, LengthPercentage, LengthPercentageAuto,
    Line, NodeId, RequestedAxis, RunMode, Size, SizingMode, Style,
};

fn block_layout(
    tree: &mut Document,
    node_id: NodeId,
    inputs: LayoutInput,
    context: Option<&mut super::context::BlockContext<'_>>,
) -> taffy::LayoutOutput {
    let mut session = LayoutSession::new(tree);
    crate::taffy_impl::block_layout(&mut session, node_id, inputs, context)
}

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
fn minimum_height_drops_the_last_child_margin_it_separates() {
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
    // The margin neither escapes the block nor adds to the height that
    // `min-height` fixed.
    assert_eq!(tree.nodes[child].unrounded_layout.size.height, 80.0);
    assert_eq!(output.size.height, 80.0);
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
    let candidate = output.oof_candidates.as_slice().first().unwrap();
    assert_eq!(candidate.node, NodeId::from(fixed));
    assert_eq!(candidate.position, taffy::Position::Fixed);
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

fn block(tree: &mut Document, parent: usize, width: f32, height: f32) -> usize {
    tree.append_element(
        Some(parent),
        "div",
        Style {
            display: Display::Block,
            size: Size {
                width: Dimension::length(width),
                height: Dimension::length(height),
            },
            ..Style::default()
        },
        None::<&str>,
    )
}

#[test]
fn float_avoiding_blocks_align_within_the_available_slot() {
    for (direction, left_auto, right_auto, expected) in [
        (taffy::Direction::Rtl, false, false, 160.0),
        (taffy::Direction::Ltr, true, true, 105.0),
        (taffy::Direction::Ltr, true, false, 160.0),
        (taffy::Direction::Ltr, false, true, 50.0),
    ] {
        let mut tree = Document::new();
        let root = tree.append_element(
            Some(0),
            "div",
            Style {
                display: Display::FlowRoot,
                direction,
                ..Style::default()
            },
            None::<&str>,
        );
        let float = block(&mut tree, root, 50.0, 20.0);
        tree.layout_node_with_style_mut(float).unwrap().style.float = Float::Left;
        let child = block(&mut tree, root, 40.0, 10.0);
        tree.layout_node_with_style_mut(child)
            .unwrap()
            .style
            .display = Display::FlowRoot;
        tree.layout_node_with_style_mut(child)
            .unwrap()
            .style
            .margin
            .left = if left_auto {
            LengthPercentageAuto::auto()
        } else {
            LengthPercentageAuto::length(0.0)
        };
        tree.layout_node_with_style_mut(child)
            .unwrap()
            .style
            .margin
            .right = if right_auto {
            LengthPercentageAuto::auto()
        } else {
            LengthPercentageAuto::length(0.0)
        };
        block_layout(&mut tree, NodeId::from(root), inputs(), None);
        assert_eq!(tree.nodes[child].unrounded_layout.location.x, expected);
    }
}

#[test]
fn fit_content_children_keep_the_finite_available_width() {
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
            display: Display::FlowRoot,
            size: Size {
                width: Dimension::fit_content(),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    for _ in 0..2 {
        let float = block(&mut tree, child, 100.0, 10.0);
        tree.layout_node_with_style_mut(float).unwrap().style.float = Float::Left;
    }
    let mut input = inputs();
    input.known_dimensions.width = Some(150.0);
    input.available_space.width = AvailableSpace::Definite(150.0);
    block_layout(&mut tree, NodeId::from(root), input, None);
    assert_eq!(
        tree.nodes[child].unrounded_layout.size,
        Size {
            width: 150.0,
            height: 20.0
        }
    );
}

#[test]
fn intrinsic_width_children_use_their_final_width_for_auto_margins() {
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
            display: Display::FlowRoot,
            size: Size {
                width: Dimension::max_content(),
                height: Dimension::auto(),
            },
            margin: taffy::Rect {
                left: LengthPercentageAuto::auto(),
                right: LengthPercentageAuto::auto(),
                ..Style::<String>::default().margin
            },
            ..Style::default()
        },
        None::<&str>,
    );
    block(&mut tree, child, 50.0, 10.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(tree.nodes[child].unrounded_layout.location.x, 75.0);
    assert_eq!(tree.nodes[child].unrounded_layout.size.width, 50.0);
}

#[test]
fn nested_out_of_flow_grids_preserve_the_outer_grid_area() {
    use crate::layout::test_support::{absolute_rect, page_box_800x600};
    let mut tree = Document::new();
    let html = tree.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = tree.append_element(Some(html), "body", Style::default(), Some("margin:0"));
    let grid = tree.append_element(Some(body), "div", Style::default(), Some("display:grid;position:relative;width:200px;height:20px;grid-template-columns:100px 100px"));
    let nested = tree.append_element(Some(grid), "div", Style::default(), Some("display:grid;position:absolute;left:0;top:0;width:10px;height:10px;grid-template-columns:10px"));
    tree.append_element(
        Some(nested),
        "div",
        Style::default(),
        Some("width:10px;height:10px"),
    );
    let child = tree.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("position:absolute;grid-column:2;left:0;top:0;width:10px;height:10px"),
    );
    tree.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&tree);
    let cascade = raikiri_style::cascade(&tree, &rules).expect("cascade");
    crate::layout::layout_single_page(&mut tree, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(
        absolute_rect(&tree, child).0 - absolute_rect(&tree, grid).0,
        100.0
    );
}

#[test]
fn block_distribution_alignment_uses_the_single_subject_fallback() {
    for alignment in [
        taffy::AlignContent::SPACE_AROUND,
        taffy::AlignContent::SPACE_EVENLY,
    ] {
        let mut tree = Document::new();
        let root = tree.append_element(
            Some(0),
            "div",
            Style {
                display: Display::FlowRoot,
                size: Size {
                    width: Dimension::auto(),
                    height: Dimension::length(100.0),
                },
                align_content: alignment,
                ..Style::default()
            },
            None::<&str>,
        );
        let child = block(&mut tree, root, 40.0, 20.0);
        block_layout(&mut tree, NodeId::from(root), inputs(), None);
        assert_eq!(tree.nodes[child].unrounded_layout.location.y, 40.0);
    }
}

#[test]
fn block_alignment_moves_scrollable_overflow_with_the_children() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Style {
            display: Display::FlowRoot,
            size: Size {
                width: Dimension::auto(),
                height: Dimension::length(100.0),
            },
            align_content: taffy::AlignContent::END,
            ..Style::default()
        },
        None::<&str>,
    );
    let child = block(&mut tree, root, 40.0, 20.0);
    tree.layout_node_with_style_mut(child)
        .unwrap()
        .style
        .display = Display::FlowRoot;
    block(&mut tree, child, 40.0, 150.0);
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(tree.nodes[child].unrounded_layout.location.y, 80.0);
    assert_eq!(output.scrollable_overflow_rect.bottom, 230.0);
}

#[test]
fn block_alignment_moves_the_exported_baseline() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Style {
            display: Display::FlowRoot,
            size: Size {
                width: Dimension::auto(),
                height: Dimension::length(100.0),
            },
            align_content: taffy::AlignContent::END,
            ..Style::default()
        },
        None::<&str>,
    );
    let child = tree.append_element(
        Some(root),
        "div",
        Style {
            display: Display::Flex,
            size: Size {
                width: Dimension::length(40.0),
                height: Dimension::length(20.0),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    block(&mut tree, child, 40.0, 20.0);
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(output.baselines.first, Some(100.0));
}

/// A block `root` holding a 20px-tall flex container whose only item is
/// `item_height` tall and shifted by `item_margin_top`, with `overflow` set on
/// the container.
fn baseline_probe(
    overflow: taffy::Overflow,
    item_height: f32,
    item_margin_top: f32,
) -> taffy::LayoutOutput {
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
            display: Display::Flex,
            overflow: taffy::Point {
                x: overflow,
                y: overflow,
            },
            size: Size {
                width: Dimension::length(100.0),
                height: Dimension::length(20.0),
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
                width: Dimension::length(20.0),
                height: Dimension::length(item_height),
            },
            margin: taffy::Rect {
                left: LengthPercentageAuto::length(0.0),
                right: LengthPercentageAuto::length(0.0),
                top: LengthPercentageAuto::length(item_margin_top),
                bottom: LengthPercentageAuto::length(0.0),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    block_layout(&mut tree, NodeId::from(root), inputs(), None)
}

#[test]
fn a_scroll_containers_baselines_are_clamped_to_its_border_box() {
    let output = baseline_probe(taffy::Overflow::Hidden, 80.0, 0.0);
    assert_eq!(output.baselines.first, Some(20.0));
    assert_eq!(output.baselines.last, Some(20.0));
}

#[test]
fn a_scroll_containers_baselines_are_clamped_above_its_border_box() {
    let output = baseline_probe(taffy::Overflow::Scroll, 20.0, -100.0);
    assert_eq!(output.baselines.first, Some(0.0));
    assert_eq!(output.baselines.last, Some(0.0));
}

#[test]
fn a_scroll_container_keeps_a_baseline_inside_its_border_box() {
    let output = baseline_probe(taffy::Overflow::Hidden, 10.0, 0.0);
    assert_eq!(output.baselines.first, Some(10.0));
}

#[test]
fn a_box_that_is_not_a_scroll_container_keeps_a_baseline_below_its_border_box() {
    let output = baseline_probe(taffy::Overflow::Visible, 80.0, 0.0);
    assert_eq!(output.baselines.first, Some(80.0));
    assert_eq!(output.baselines.last, Some(80.0));
}

#[test]
fn containment_limits_descendant_overflow_to_the_child_border_box() {
    for contain in [taffy::Contain::LAYOUT, taffy::Contain::PAINT] {
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
        let child = block(&mut tree, root, 100.0, 10.0);
        tree.layout_node_with_style_mut(child)
            .unwrap()
            .style
            .contain = contain;
        block(&mut tree, child, 300.0, 10.0);
        let mut input = inputs();
        input.known_dimensions.width = Some(100.0);
        let output = block_layout(&mut tree, NodeId::from(root), input, None);
        assert_eq!(output.scrollable_overflow_rect.right, 100.0);
    }
}

#[test]
fn a_known_width_transfers_through_the_block_aspect_ratio() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Style {
            display: Display::FlowRoot,
            aspect_ratio: Some(2.0),
            ..Style::default()
        },
        None::<&str>,
    );
    block(&mut tree, root, 20.0, 10.0);
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(
        output.size,
        Size {
            width: 200.0,
            height: 100.0
        }
    );
}

#[test]
fn scrollable_overflow_keeps_the_end_padding_after_content() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Style {
            display: Display::FlowRoot,
            size: Size {
                width: Dimension::auto(),
                height: Dimension::length(50.0),
            },
            overflow: taffy::Point {
                x: taffy::Overflow::Scroll,
                y: taffy::Overflow::Scroll,
            },
            padding: taffy::Rect {
                right: LengthPercentage::length(20.0),
                bottom: LengthPercentage::length(20.0),
                ..Style::<String>::default().padding
            },
            ..Style::default()
        },
        None::<&str>,
    );
    block(&mut tree, root, 300.0, 100.0);
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(output.scrollable_overflow_rect.bottom, 120.0);
    assert_eq!(output.scrollable_overflow_rect.right, 320.0);
}

fn plain(tree: &mut Document, parent: usize) -> usize {
    tree.append_element(
        Some(parent),
        "div",
        Style {
            display: Display::Block,
            ..Style::default()
        },
        None::<&str>,
    )
}

#[test]
fn a_formatting_context_pushed_below_an_adjoining_float_separates_its_margin() {
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
    let outer = plain(&mut tree, root);
    let holder = plain(&mut tree, outer);
    let float = block(&mut tree, holder, 200.0, 200.0);
    tree.layout_node_with_style_mut(float).unwrap().style.float = Float::Left;
    let context = block(&mut tree, outer, 200.0, 1.0);
    tree.layout_node_with_style_mut(context)
        .unwrap()
        .style
        .display = Display::FlowRoot;
    tree.layout_node_with_style_mut(context)
        .unwrap()
        .style
        .margin
        .top = LengthPercentageAuto::length(200.0);
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    // The box does not fit beside the float, so its margin no longer
    // collapses with the parent's and the float stays at the top.
    assert_eq!(tree.nodes[outer].unrounded_layout.location.y, 0.0);
    assert_eq!(tree.nodes[float].unrounded_layout.location.y, 0.0);
    assert_eq!(tree.nodes[context].unrounded_layout.location.y, 200.0);
    assert_eq!(output.size.height, 201.0);
}

#[test]
fn a_formatting_context_beside_an_adjoining_float_keeps_its_margin() {
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
    let outer = plain(&mut tree, root);
    let holder = plain(&mut tree, outer);
    let float = block(&mut tree, holder, 100.0, 10.0);
    tree.layout_node_with_style_mut(float).unwrap().style.float = Float::Left;
    let context = block(&mut tree, outer, 100.0, 10.0);
    tree.layout_node_with_style_mut(context)
        .unwrap()
        .style
        .display = Display::FlowRoot;
    tree.layout_node_with_style_mut(context)
        .unwrap()
        .style
        .margin
        .top = LengthPercentageAuto::length(50.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    // The box fits beside the float, so the margin escapes and carries
    // the float down with it.
    assert_eq!(tree.nodes[outer].unrounded_layout.location.y, 50.0);
    assert_eq!(tree.nodes[float].unrounded_layout.location.y, 0.0);
    assert_eq!(tree.nodes[context].unrounded_layout.location.y, 0.0);
    assert_eq!(tree.nodes[context].unrounded_layout.location.x, 100.0);
}

#[test]
fn a_formatting_context_clearing_an_adjoining_float_starts_below_it() {
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
    let outer = plain(&mut tree, root);
    let float = block(&mut tree, outer, 100.0, 50.0);
    tree.layout_node_with_style_mut(float).unwrap().style.float = Float::Left;
    let context = block(&mut tree, outer, 100.0, 50.0);
    tree.layout_node_with_style_mut(context)
        .unwrap()
        .style
        .display = Display::FlowRoot;
    tree.layout_node_with_style_mut(context)
        .unwrap()
        .style
        .clear = taffy::Clear::Left;
    tree.layout_node_with_style_mut(context)
        .unwrap()
        .style
        .margin
        .top = LengthPercentageAuto::length(300.0);
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(tree.nodes[outer].unrounded_layout.location.y, 0.0);
    assert_eq!(tree.nodes[context].unrounded_layout.location.y, 50.0);
    assert_eq!(output.size.height, 100.0);
}

/// The leading margin of `inner` escapes above it while its origin is still
/// a guess; the cleared child must land at the float's bottom.
#[test]
fn clearance_under_an_unresolved_origin_excludes_the_escaping_margin() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Style {
            display: Display::Block,
            ..Style::default()
        },
        None::<&str>,
    );
    let float = block(&mut tree, root, 50.0, 100.0);
    tree.layout_node_with_style_mut(float).unwrap().style.float = Float::Left;
    let padded = plain(&mut tree, root);
    tree.layout_node_with_style_mut(padded)
        .unwrap()
        .style
        .padding
        .top = LengthPercentage::length(1.0);
    let inner = plain(&mut tree, padded);
    let first = plain(&mut tree, inner);
    tree.layout_node_with_style_mut(first)
        .unwrap()
        .style
        .margin
        .bottom = LengthPercentageAuto::length(49.0);
    let cleared = plain(&mut tree, inner);
    tree.layout_node_with_style_mut(cleared)
        .unwrap()
        .style
        .clear = taffy::Clear::Left;
    tree.layout_node_with_style_mut(cleared)
        .unwrap()
        .style
        .margin
        .top = LengthPercentageAuto::length(98.0);
    // Like the body of a page, the root's own top margin may still collapse
    // with its children, so no origin in it is resolved.
    let mut root_inputs = inputs();
    root_inputs.vertical_margins_are_collapsible.start = true;
    block_layout(&mut tree, NodeId::from(root), root_inputs, None);
    let inner_y = tree.nodes[inner].unrounded_layout.location.y;
    assert_eq!(inner_y, 50.0);
    assert_eq!(
        inner_y + tree.nodes[cleared].unrounded_layout.location.y,
        100.0
    );
}

#[test]
fn clearance_under_an_unresolved_origin_counts_the_block_margin_once() {
    let mut tree = Document::new();
    let root = plain(&mut tree, 0);
    let float = block(&mut tree, root, 50.0, 150.0);
    tree.layout_node_with_style_mut(float).unwrap().style.float = Float::Left;
    let padded = plain(&mut tree, root);
    tree.layout_node_with_style_mut(padded)
        .unwrap()
        .style
        .padding
        .top = LengthPercentage::length(1.0);
    let inner = plain(&mut tree, padded);
    tree.layout_node_with_style_mut(inner)
        .unwrap()
        .style
        .margin
        .top = LengthPercentageAuto::length(20.0);
    let first = plain(&mut tree, inner);
    tree.layout_node_with_style_mut(first)
        .unwrap()
        .style
        .margin
        .bottom = LengthPercentageAuto::length(49.0);
    let cleared = plain(&mut tree, inner);
    tree.layout_node_with_style_mut(cleared)
        .unwrap()
        .style
        .clear = taffy::Clear::Left;
    tree.layout_node_with_style_mut(cleared)
        .unwrap()
        .style
        .margin
        .top = LengthPercentageAuto::length(50.0);
    let mut root_inputs = inputs();
    root_inputs.vertical_margins_are_collapsible.start = true;
    block_layout(&mut tree, NodeId::from(root), root_inputs, None);
    // `inner` sits below its own margin collapsed with `first`'s: 1 + 49.
    let inner_y = tree.nodes[inner].unrounded_layout.location.y;
    assert_eq!(inner_y, 50.0);
    assert_eq!(
        inner_y + tree.nodes[cleared].unrounded_layout.location.y,
        150.0
    );
}

#[test]
fn clearance_past_a_float_on_the_same_strut_keeps_the_float_coordinates() {
    let mut tree = Document::new();
    let root = plain(&mut tree, 0);
    let spacer = plain(&mut tree, root);
    tree.layout_node_with_style_mut(spacer)
        .unwrap()
        .style
        .margin
        .bottom = LengthPercentageAuto::length(49.0);
    let float = block(&mut tree, root, 50.0, 10.0);
    tree.layout_node_with_style_mut(float).unwrap().style.float = Float::Left;
    let cleared = block(&mut tree, root, 50.0, 10.0);
    tree.layout_node_with_style_mut(cleared)
        .unwrap()
        .style
        .clear = taffy::Clear::Left;
    let mut root_inputs = inputs();
    root_inputs.vertical_margins_are_collapsible.start = true;
    block_layout(&mut tree, NodeId::from(root), root_inputs, None);
    let float_y = tree.nodes[float].unrounded_layout.location.y;
    assert_eq!(
        tree.nodes[cleared].unrounded_layout.location.y,
        float_y + 10.0
    );
}

#[test]
fn a_last_child_margin_escapes_when_min_height_equals_the_content() {
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
    let parent = plain(&mut tree, root);
    tree.layout_node_with_style_mut(parent)
        .unwrap()
        .style
        .min_size
        .height = LengthPercentageAuto::length(20.0);
    let child = block(&mut tree, parent, 100.0, 20.0);
    tree.layout_node_with_style_mut(child)
        .unwrap()
        .style
        .margin
        .bottom = LengthPercentageAuto::length(100.0);
    let next = block(&mut tree, root, 100.0, 10.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(tree.nodes[parent].unrounded_layout.size.height, 20.0);
    assert_eq!(tree.nodes[next].unrounded_layout.location.y, 120.0);
}

fn flow_root_container(tree: &mut Document) -> usize {
    tree.append_element(
        Some(0),
        "div",
        Style {
            display: Display::FlowRoot,
            ..Style::default()
        },
        None::<&str>,
    )
}

fn float_box(
    tree: &mut Document,
    parent: usize,
    side: Float,
    clear: taffy::Clear,
    width: f32,
    height: f32,
) -> usize {
    let float = block(tree, parent, width, height);
    let style = &mut tree.layout_node_with_style_mut(float).unwrap().style;
    style.float = side;
    style.clear = clear;
    float
}

/// A `display: flow-root` child `height` tall, `width` wide or `auto` wide.
fn formatting_context_box(
    tree: &mut Document,
    parent: usize,
    width: Option<f32>,
    height: f32,
) -> usize {
    let child = block(tree, parent, width.unwrap_or(0.0), height);
    let style = &mut tree.layout_node_with_style_mut(child).unwrap().style;
    style.display = Display::FlowRoot;
    if width.is_none() {
        style.size.width = Dimension::auto();
    }
    child
}

#[test]
fn a_formatting_context_taller_than_its_gap_moves_below_the_float_it_would_overlap() {
    let mut tree = Document::new();
    let root = flow_root_container(&mut tree);
    // 140 + 80 > 200, so the second float sits below the first, at the right.
    float_box(
        &mut tree,
        root,
        Float::Left,
        taffy::Clear::None,
        140.0,
        10.0,
    );
    float_box(
        &mut tree,
        root,
        Float::Right,
        taffy::Clear::None,
        80.0,
        30.0,
    );
    // 40px fit in the 60px gap beside the first float, but that gap ends
    // after 10px while the box goes on for 30.
    let tall = formatting_context_box(&mut tree, root, Some(40.0), 30.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    let layout = tree.nodes[tall].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 10.0));
}

#[test]
fn a_formatting_context_no_taller_than_its_gap_stays_beside_the_first_float() {
    let mut tree = Document::new();
    let root = flow_root_container(&mut tree);
    float_box(
        &mut tree,
        root,
        Float::Left,
        taffy::Clear::None,
        140.0,
        10.0,
    );
    float_box(
        &mut tree,
        root,
        Float::Right,
        taffy::Clear::None,
        80.0,
        30.0,
    );
    let short = formatting_context_box(&mut tree, root, Some(40.0), 10.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    let layout = tree.nodes[short].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (140.0, 0.0));
}

#[test]
fn a_formatting_context_that_fits_no_band_goes_below_all_the_floats() {
    let mut tree = Document::new();
    let root = flow_root_container(&mut tree);
    float_box(
        &mut tree,
        root,
        Float::Left,
        taffy::Clear::None,
        140.0,
        10.0,
    );
    float_box(
        &mut tree,
        root,
        Float::Right,
        taffy::Clear::None,
        80.0,
        30.0,
    );
    // Wider than the 60px gap, and the band below it is shared with the
    // right float for 30px more.
    let wide = formatting_context_box(&mut tree, root, Some(130.0), 20.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    let layout = tree.nodes[wide].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 40.0));
}

#[test]
fn a_formatting_context_slides_to_the_room_every_band_it_reaches_shares() {
    let mut tree = Document::new();
    let root = flow_root_container(&mut tree);
    float_box(&mut tree, root, Float::Left, taffy::Clear::None, 60.0, 10.0);
    float_box(
        &mut tree,
        root,
        Float::Left,
        taffy::Clear::Left,
        100.0,
        10.0,
    );
    // The second band starts 40px further in than the first, so the box
    // keeps clear of it too.
    let sliding = formatting_context_box(&mut tree, root, Some(50.0), 30.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    let layout = tree.nodes[sliding].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (100.0, 0.0));
}

#[test]
fn an_auto_width_formatting_context_narrows_to_the_room_every_band_it_reaches_shares() {
    let mut tree = Document::new();
    let root = flow_root_container(&mut tree);
    float_box(&mut tree, root, Float::Left, taffy::Clear::None, 60.0, 10.0);
    float_box(
        &mut tree,
        root,
        Float::Left,
        taffy::Clear::Left,
        100.0,
        10.0,
    );
    let stretching = formatting_context_box(&mut tree, root, None, 30.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    let layout = tree.nodes[stretching].unrounded_layout;
    assert_eq!(
        (layout.location.x, layout.location.y, layout.size.width),
        (100.0, 0.0, 100.0)
    );
}

#[test]
fn an_auto_width_formatting_context_narrows_to_the_narrowest_of_several_bands() {
    let mut tree = Document::new();
    let root = flow_root_container(&mut tree);
    float_box(
        &mut tree,
        root,
        Float::Left,
        taffy::Clear::None,
        100.0,
        10.0,
    );
    float_box(
        &mut tree,
        root,
        Float::Left,
        taffy::Clear::Left,
        150.0,
        10.0,
    );
    float_box(
        &mut tree,
        root,
        Float::Left,
        taffy::Clear::Left,
        180.0,
        10.0,
    );
    // The box reaches all three bands, and the last starts furthest in.
    let stretching = formatting_context_box(&mut tree, root, None, 25.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    let layout = tree.nodes[stretching].unrounded_layout;
    assert_eq!(
        (layout.location.x, layout.location.y, layout.size.width),
        (180.0, 0.0, 20.0)
    );
}
