use crate::Document;
use crate::layout::protocol::{
    AvailableSpace, AxisStaticEdge, LayoutInput, LayoutOutput, NodeId, OofPosition, RequestedAxis,
    RunMode, SizingMode,
};
use crate::layout::test_support::{restyle, with_bridged_session};
use raikiri_layout_geometry::{Line, Size};
use raikiri_style::property::{DisplayValue, PositionValue};
use taffy::Float;

fn block_layout(
    tree: &mut Document,
    node_id: NodeId,
    inputs: LayoutInput,
    context: Option<&mut super::context::BlockContext<'_>>,
) -> LayoutOutput {
    with_bridged_session(tree, |session| {
        crate::taffy_impl::block_layout(session, node_id, inputs, context)
    })
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
        Some("display:flow-root;box-sizing:border-box"),
    );
    let child = tree.append_element(
        Some(root),
        "div",
        Some("display:block;box-sizing:border-box;min-height:80px"),
    );
    tree.append_element(
        Some(child),
        "div",
        Some("display:block;box-sizing:border-box;height:20px;margin-bottom:100px"),
    );
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    // The margin neither escapes the block nor adds to the height that
    // `min-height` fixed.
    assert_eq!(tree.nodes[child].layout.size.height, 80.0);
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
        let mut css = String::from("display:flow-root;box-sizing:content-box;padding-bottom:40px");
        if let Some(height) = height {
            css.push_str(&format!(";height:{height}px"));
        }
        if let Some(max) = max {
            css.push_str(&format!(";max-height:{max}px"));
        }
        let root = tree.append_element(Some(0), "div", Some(css));
        tree.append_element(
            Some(root),
            "div",
            Some("display:block;box-sizing:border-box;width:20px;height:80px;float:left"),
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
        Some("display:flow-root;box-sizing:border-box"),
    );
    let fixed = tree.append_element(
        Some(root),
        "div",
        Some("display:flex;box-sizing:border-box;position:fixed;width:20px;height:30px;top:15px;left:10px"),
    );
    let sibling = tree.append_element(
        Some(root),
        "div",
        Some("display:flex;box-sizing:border-box;width:40px;height:10px"),
    );
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(output.size.height, 10.0);
    assert_eq!(tree.nodes[sibling].layout.location.y, 0.0);
    let candidate = output.oof_candidates.iter().next().unwrap();
    assert_eq!(candidate.node, NodeId::from(fixed));
    assert_eq!(candidate.position, OofPosition::Fixed);
}

#[test]
fn inline_absolute_candidates_keep_the_rtl_float_slot_point() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;direction:rtl;width:200px;height:100px;text-align:center"),
    );
    tree.append_element(
        Some(root),
        "div",
        Some("display:block;float:left;width:50px;height:50px"),
    );
    let abs = tree.append_element(
        Some(root),
        "div",
        Some("position:absolute;width:40px;height:10px"),
    );

    let output = with_bridged_session(&mut tree, |session| {
        // The bridge blockifies an absolutely positioned box; this one stands
        // for a box whose static position is that of an inline-level box.
        session.nodes[abs].set_box_properties(DisplayValue::Inline, PositionValue::Absolute);
        crate::taffy_impl::block_layout(session, NodeId::from(root), inputs(), None)
    });
    let candidate = output.oof_candidates.iter().next().unwrap();
    assert_eq!(candidate.static_position.x.area.start, 125.0);
    assert_eq!(
        candidate.static_position.x.align.keyword,
        AxisStaticEdge::End
    );
    assert_eq!(candidate.static_position.y.area.start, 0.0);
}

#[test]
fn absolute_child_overflow_is_reported_without_growing_the_block() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box;position:relative"),
    );
    tree.append_element(
        Some(root),
        "div",
        Some("display:flex;box-sizing:border-box;position:absolute;width:40px;height:30px;top:15px;left:210px"),
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
        Some(format!(
            "display:block;box-sizing:border-box;width:{width}px;height:{height}px"
        )),
    )
}

#[test]
fn float_avoiding_blocks_align_within_the_available_slot() {
    for (direction, left_auto, right_auto, expected) in [
        ("rtl", false, false, 160.0),
        ("ltr", true, true, 105.0),
        ("ltr", true, false, 160.0),
        ("ltr", false, true, 50.0),
    ] {
        let mut tree = Document::new();
        let root = tree.append_element(
            Some(0),
            "div",
            Some(format!(
                "display:flow-root;box-sizing:border-box;direction:{direction}"
            )),
        );
        let float = block(&mut tree, root, 50.0, 20.0);
        restyle(&mut tree, float, "float:left");
        let child = block(&mut tree, root, 40.0, 10.0);
        restyle(&mut tree, child, "display:flow-root");
        let side = |auto: bool| if auto { "auto" } else { "0" };
        restyle(
            &mut tree,
            child,
            &format!(
                "margin-left:{};margin-right:{}",
                side(left_auto),
                side(right_auto)
            ),
        );
        block_layout(&mut tree, NodeId::from(root), inputs(), None);
        assert_eq!(tree.nodes[child].layout.location.x, expected);
    }
}

#[test]
fn fit_content_children_keep_the_finite_available_width() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box"),
    );
    let child = tree.append_element(
        Some(root),
        "div",
        Some("display:flow-root;box-sizing:border-box;width:fit-content"),
    );
    for _ in 0..2 {
        let float = block(&mut tree, child, 100.0, 10.0);
        restyle(&mut tree, float, "float:left");
    }
    let mut input = inputs();
    input.known_dimensions.width = Some(150.0);
    input.available_space.width = AvailableSpace::Definite(150.0);
    block_layout(&mut tree, NodeId::from(root), input, None);
    assert_eq!(
        tree.nodes[child].layout.size,
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
        Some("display:flow-root;box-sizing:border-box"),
    );
    let child = tree.append_element(
        Some(root),
        "div",
        Some("display:flow-root;box-sizing:border-box;width:max-content;margin-right:auto;margin-left:auto"),
    );
    block(&mut tree, child, 50.0, 10.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(tree.nodes[child].layout.location.x, 75.0);
    assert_eq!(tree.nodes[child].layout.size.width, 50.0);
}

#[test]
fn nested_out_of_flow_grids_preserve_the_outer_grid_area() {
    use crate::layout::test_support::{absolute_rect, page_box_800x600};
    let mut tree = Document::new();
    let html = tree.append_element(Some(0), "html", None::<&str>);
    let body = tree.append_element(Some(html), "body", Some("margin:0"));
    let grid = tree.append_element(Some(body), "div", Some("display:grid;position:relative;width:200px;height:20px;grid-template-columns:100px 100px"));
    let nested = tree.append_element(Some(grid), "div", Some("display:grid;position:absolute;left:0;top:0;width:10px;height:10px;grid-template-columns:10px"));
    tree.append_element(Some(nested), "div", Some("width:10px;height:10px"));
    let child = tree.append_element(
        Some(grid),
        "div",
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
    for alignment in ["space-around", "space-evenly"] {
        let mut tree = Document::new();
        let root = tree.append_element(
            Some(0),
            "div",
            Some(format!(
                "display:flow-root;box-sizing:border-box;height:100px;align-content:{alignment}"
            )),
        );
        let child = block(&mut tree, root, 40.0, 20.0);
        block_layout(&mut tree, NodeId::from(root), inputs(), None);
        assert_eq!(tree.nodes[child].layout.location.y, 40.0);
    }
}

#[test]
fn block_alignment_moves_scrollable_overflow_with_the_children() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box;height:100px;align-content:end"),
    );
    let child = block(&mut tree, root, 40.0, 20.0);
    restyle(&mut tree, child, "display:flow-root");
    block(&mut tree, child, 40.0, 150.0);
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(tree.nodes[child].layout.location.y, 80.0);
    assert_eq!(output.scrollable_overflow_rect.bottom, 230.0);
}

#[test]
fn block_alignment_moves_the_exported_baseline() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box;height:100px;align-content:end"),
    );
    let child = tree.append_element(
        Some(root),
        "div",
        Some("display:flex;box-sizing:border-box;width:40px;height:20px"),
    );
    block(&mut tree, child, 40.0, 20.0);
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(output.baselines.first, Some(100.0));
}

/// A block `root` holding a 20px-tall flex container whose only item is
/// `item_height` tall and shifted by `item_margin_top`, with `overflow` set on
/// the container.
fn baseline_probe(overflow: &str, item_height: f32, item_margin_top: f32) -> LayoutOutput {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box"),
    );
    let child = tree.append_element(
        Some(root),
        "div",
        Some(format!(
            "display:flex;box-sizing:border-box;width:100px;height:20px;overflow:{overflow}"
        )),
    );
    tree.append_element(
        Some(child),
        "div",
        Some(format!(
            "display:block;box-sizing:border-box;width:20px;height:{item_height}px;margin:{item_margin_top}px 0 0 0"
        )),
    );
    block_layout(&mut tree, NodeId::from(root), inputs(), None)
}

#[test]
fn a_scroll_containers_baselines_are_clamped_to_its_border_box() {
    let output = baseline_probe("hidden", 80.0, 0.0);
    assert_eq!(output.baselines.first, Some(20.0));
    assert_eq!(output.baselines.last, Some(20.0));
}

#[test]
fn a_scroll_containers_baselines_are_clamped_above_its_border_box() {
    let output = baseline_probe("scroll", 20.0, -100.0);
    assert_eq!(output.baselines.first, Some(0.0));
    assert_eq!(output.baselines.last, Some(0.0));
}

#[test]
fn a_scroll_container_keeps_a_baseline_inside_its_border_box() {
    let output = baseline_probe("hidden", 10.0, 0.0);
    assert_eq!(output.baselines.first, Some(10.0));
}

#[test]
fn a_box_that_is_not_a_scroll_container_keeps_a_baseline_below_its_border_box() {
    let output = baseline_probe("visible", 80.0, 0.0);
    assert_eq!(output.baselines.first, Some(80.0));
    assert_eq!(output.baselines.last, Some(80.0));
}

#[test]
fn containment_limits_descendant_overflow_to_the_child_border_box() {
    for contain_css in ["contain:layout", "contain:paint"] {
        let mut tree = Document::new();
        let root = tree.append_element(
            Some(0),
            "div",
            Some("display:flow-root;box-sizing:border-box"),
        );
        let child = block(&mut tree, root, 100.0, 10.0);
        restyle(&mut tree, child, contain_css);
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
        Some("display:flow-root;box-sizing:border-box;aspect-ratio:2"),
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

/// A flow-root box with a 2:1 `aspect-ratio` and the `extra` declarations.
fn ratio_box(tree: &mut Document, extra: &str) -> usize {
    tree.append_element(
        Some(0),
        "div",
        Some(format!(
            "display:flow-root;box-sizing:border-box;aspect-ratio:2;{extra}"
        )),
    )
}

fn laid_out_size(tree: &mut Document, root: usize, known_width: Option<f32>) -> Size<f32> {
    let mut input = inputs();
    input.known_dimensions.width = known_width;
    block_layout(tree, NodeId::from(root), input, None).size
}

#[test]
fn a_height_taken_from_the_aspect_ratio_grows_to_fit_the_content() {
    let mut tree = Document::new();
    let root = ratio_box(&mut tree, "width:100px");
    block(&mut tree, root, 20.0, 100.0);
    assert_eq!(
        laid_out_size(&mut tree, root, None),
        Size {
            width: 100.0,
            height: 100.0
        }
    );
}

#[test]
fn a_stretched_width_gives_a_ratio_height_that_the_content_can_raise() {
    let mut tree = Document::new();
    let root = ratio_box(&mut tree, "");
    block(&mut tree, root, 20.0, 150.0);
    assert_eq!(
        laid_out_size(&mut tree, root, Some(200.0)),
        Size {
            width: 200.0,
            height: 150.0
        }
    );
}

fn padded_ratio_box(tree: &mut Document, box_sizing: &str) -> usize {
    ratio_box(tree, &format!("box-sizing:{box_sizing};padding:10px"))
}

#[test]
fn a_known_border_box_width_gives_a_content_box_ratio_height() {
    // The 100px width includes 20px of padding, so the 80px content box is
    // 40px tall by 2:1, and the padding adds 20px to that.
    let mut tree = Document::new();
    let root = padded_ratio_box(&mut tree, "content-box");
    assert_eq!(
        laid_out_size(&mut tree, root, Some(100.0)),
        Size {
            width: 100.0,
            height: 60.0
        }
    );
}

#[test]
fn a_known_border_box_width_gives_a_border_box_ratio_height() {
    let mut tree = Document::new();
    let root = padded_ratio_box(&mut tree, "border-box");
    assert_eq!(
        laid_out_size(&mut tree, root, Some(100.0)),
        Size {
            width: 100.0,
            height: 50.0
        }
    );
}

#[test]
fn the_maximum_height_still_caps_a_height_the_content_raised() {
    let mut tree = Document::new();
    let root = ratio_box(&mut tree, "width:100px;max-height:80px");
    block(&mut tree, root, 20.0, 100.0);
    assert_eq!(laid_out_size(&mut tree, root, None).height, 80.0);
}

#[test]
fn a_scroll_container_keeps_the_height_the_aspect_ratio_gives() {
    let mut tree = Document::new();
    let root = ratio_box(&mut tree, "width:100px;overflow:hidden");
    block(&mut tree, root, 20.0, 100.0);
    assert_eq!(laid_out_size(&mut tree, root, None).height, 50.0);
}

#[test]
fn a_zero_minimum_height_keeps_the_height_the_aspect_ratio_gives() {
    let mut tree = Document::new();
    let root = ratio_box(&mut tree, "width:100px;min-height:0px");
    block(&mut tree, root, 20.0, 100.0);
    assert_eq!(laid_out_size(&mut tree, root, None).height, 50.0);
}

#[test]
fn an_authored_height_is_not_raised_by_the_content() {
    let mut tree = Document::new();
    let root = ratio_box(&mut tree, "height:50px");
    block(&mut tree, root, 20.0, 100.0);
    assert_eq!(laid_out_size(&mut tree, root, None).height, 50.0);
}

#[test]
fn a_width_taken_from_the_aspect_ratio_grows_to_the_min_content_width() {
    let mut tree = Document::new();
    let root = ratio_box(&mut tree, "height:50px");
    block(&mut tree, root, 300.0, 10.0);
    assert_eq!(
        laid_out_size(&mut tree, root, None),
        Size {
            width: 300.0,
            height: 50.0
        }
    );
}

#[test]
fn scrollable_overflow_keeps_the_end_padding_after_content() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box;height:50px;padding-right:20px;padding-bottom:20px;overflow-x:scroll;overflow-y:scroll"),
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
        Some("display:block;box-sizing:border-box"),
    )
}

#[test]
fn a_formatting_context_pushed_below_an_adjoining_float_separates_its_margin() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box"),
    );
    let outer = plain(&mut tree, root);
    let holder = plain(&mut tree, outer);
    let float = block(&mut tree, holder, 200.0, 200.0);
    restyle(&mut tree, float, "float:left");
    let context = block(&mut tree, outer, 200.0, 1.0);
    restyle(&mut tree, context, "display:flow-root");
    restyle(&mut tree, context, "margin-top:200px");
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    // The box does not fit beside the float, so its margin no longer
    // collapses with the parent's and the float stays at the top.
    assert_eq!(tree.nodes[outer].layout.location.y, 0.0);
    assert_eq!(tree.nodes[float].layout.location.y, 0.0);
    assert_eq!(tree.nodes[context].layout.location.y, 200.0);
    assert_eq!(output.size.height, 201.0);
}

#[test]
fn a_formatting_context_beside_an_adjoining_float_keeps_its_margin() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box"),
    );
    let outer = plain(&mut tree, root);
    let holder = plain(&mut tree, outer);
    let float = block(&mut tree, holder, 100.0, 10.0);
    restyle(&mut tree, float, "float:left");
    let context = block(&mut tree, outer, 100.0, 10.0);
    restyle(&mut tree, context, "display:flow-root");
    restyle(&mut tree, context, "margin-top:50px");
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    // The box fits beside the float, so the margin escapes and carries
    // the float down with it.
    assert_eq!(tree.nodes[outer].layout.location.y, 50.0);
    assert_eq!(tree.nodes[float].layout.location.y, 0.0);
    assert_eq!(tree.nodes[context].layout.location.y, 0.0);
    assert_eq!(tree.nodes[context].layout.location.x, 100.0);
}

#[test]
fn a_formatting_context_clearing_an_adjoining_float_starts_below_it() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box"),
    );
    let outer = plain(&mut tree, root);
    let float = block(&mut tree, outer, 100.0, 50.0);
    restyle(&mut tree, float, "float:left");
    let context = block(&mut tree, outer, 100.0, 50.0);
    restyle(&mut tree, context, "display:flow-root");
    restyle(&mut tree, context, "clear:left");
    restyle(&mut tree, context, "margin-top:300px");
    let output = block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(tree.nodes[outer].layout.location.y, 0.0);
    assert_eq!(tree.nodes[context].layout.location.y, 50.0);
    assert_eq!(output.size.height, 100.0);
}

/// The leading margin of `inner` escapes above it while its origin is still
/// a guess; the cleared child must land at the float's bottom.
#[test]
fn clearance_under_an_unresolved_origin_excludes_the_escaping_margin() {
    let mut tree = Document::new();
    let root = tree.append_element(Some(0), "div", Some("display:block;box-sizing:border-box"));
    let float = block(&mut tree, root, 50.0, 100.0);
    restyle(&mut tree, float, "float:left");
    let padded = plain(&mut tree, root);
    restyle(&mut tree, padded, "padding-top:1px");
    let inner = plain(&mut tree, padded);
    let first = plain(&mut tree, inner);
    restyle(&mut tree, first, "margin-bottom:49px");
    let cleared = plain(&mut tree, inner);
    restyle(&mut tree, cleared, "clear:left");
    restyle(&mut tree, cleared, "margin-top:98px");
    // Like the body of a page, the root's own top margin may still collapse
    // with its children, so no origin in it is resolved.
    let mut root_inputs = inputs();
    root_inputs.vertical_margins_are_collapsible.start = true;
    block_layout(&mut tree, NodeId::from(root), root_inputs, None);
    let inner_y = tree.nodes[inner].layout.location.y;
    assert_eq!(inner_y, 50.0);
    assert_eq!(inner_y + tree.nodes[cleared].layout.location.y, 100.0);
}

#[test]
fn clearance_under_an_unresolved_origin_counts_the_block_margin_once() {
    let mut tree = Document::new();
    let root = plain(&mut tree, 0);
    let float = block(&mut tree, root, 50.0, 150.0);
    restyle(&mut tree, float, "float:left");
    let padded = plain(&mut tree, root);
    restyle(&mut tree, padded, "padding-top:1px");
    let inner = plain(&mut tree, padded);
    restyle(&mut tree, inner, "margin-top:20px");
    let first = plain(&mut tree, inner);
    restyle(&mut tree, first, "margin-bottom:49px");
    let cleared = plain(&mut tree, inner);
    restyle(&mut tree, cleared, "clear:left");
    restyle(&mut tree, cleared, "margin-top:50px");
    let mut root_inputs = inputs();
    root_inputs.vertical_margins_are_collapsible.start = true;
    block_layout(&mut tree, NodeId::from(root), root_inputs, None);
    // `inner` sits below its own margin collapsed with `first`'s: 1 + 49.
    let inner_y = tree.nodes[inner].layout.location.y;
    assert_eq!(inner_y, 50.0);
    assert_eq!(inner_y + tree.nodes[cleared].layout.location.y, 150.0);
}

#[test]
fn clearance_past_a_float_on_the_same_strut_keeps_the_float_coordinates() {
    let mut tree = Document::new();
    let root = plain(&mut tree, 0);
    let spacer = plain(&mut tree, root);
    restyle(&mut tree, spacer, "margin-bottom:49px");
    let float = block(&mut tree, root, 50.0, 10.0);
    restyle(&mut tree, float, "float:left");
    let cleared = block(&mut tree, root, 50.0, 10.0);
    restyle(&mut tree, cleared, "clear:left");
    let mut root_inputs = inputs();
    root_inputs.vertical_margins_are_collapsible.start = true;
    block_layout(&mut tree, NodeId::from(root), root_inputs, None);
    let float_y = tree.nodes[float].layout.location.y;
    assert_eq!(tree.nodes[cleared].layout.location.y, float_y + 10.0);
}

#[test]
fn a_last_child_margin_escapes_when_min_height_equals_the_content() {
    let mut tree = Document::new();
    let root = tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box"),
    );
    let parent = plain(&mut tree, root);
    restyle(&mut tree, parent, "min-height:20px");
    let child = block(&mut tree, parent, 100.0, 20.0);
    restyle(&mut tree, child, "margin-bottom:100px");
    let next = block(&mut tree, root, 100.0, 10.0);
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    assert_eq!(tree.nodes[parent].layout.size.height, 20.0);
    assert_eq!(tree.nodes[next].layout.location.y, 120.0);
}

fn flow_root_container(tree: &mut Document) -> usize {
    tree.append_element(
        Some(0),
        "div",
        Some("display:flow-root;box-sizing:border-box"),
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
    let side = match side {
        Float::Left => "left",
        Float::Right => "right",
        Float::None => "none",
    };
    let clear = match clear {
        taffy::Clear::None => "none",
        taffy::Clear::Left => "left",
        taffy::Clear::Right => "right",
        taffy::Clear::Both => "both",
    };
    restyle(tree, float, &format!("float:{side};clear:{clear}"));
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
    restyle(tree, child, "display:flow-root");
    if width.is_none() {
        restyle(tree, child, "width:auto");
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
    let layout = tree.nodes[tall].layout;
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
    let layout = tree.nodes[short].layout;
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
    let layout = tree.nodes[wide].layout;
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
    let layout = tree.nodes[sliding].layout;
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
    let layout = tree.nodes[stretching].layout;
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
    let layout = tree.nodes[stretching].layout;
    assert_eq!(
        (layout.location.x, layout.location.y, layout.size.width),
        (180.0, 0.0, 20.0)
    );
}

#[test]
fn a_square_formatting_context_settles_in_the_widest_room_it_fits_beside_the_floats() {
    let mut tree = Document::new();
    let root = flow_root_container(&mut tree);
    // Three bands to the right of the floats: 140px from y = 0, 40px from
    // y = 5 and 10px from y = 40.
    for (width, height) in [(60.0, 5.0), (160.0, 35.0), (190.0, 10.0)] {
        float_box(
            &mut tree,
            root,
            Float::Left,
            taffy::Clear::Left,
            width,
            height,
        );
    }
    let square = formatting_context_box(&mut tree, root, None, 0.0);
    restyle(&mut tree, square, "height:auto;aspect-ratio:1");
    block_layout(&mut tree, NodeId::from(root), inputs(), None);
    // 140px wide it would be 140px tall and reach the 10px band, which
    // narrows it to 10px. Only 10px tall, it no longer reaches that band, and
    // the room left is the 40px the first two bands share, where it is a
    // 40px square that ends as the last band starts.
    let layout = tree.nodes[square].layout;
    assert_eq!(
        (
            layout.location.x,
            layout.location.y,
            layout.size.width,
            layout.size.height
        ),
        (160.0, 0.0, 40.0, 40.0)
    );
}
