use super::*;
use taffy::Style;

fn fixture(markup: &str) -> (Document, raikiri_style::CascadeResult) {
    let options = raikiri_html::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let foreign = raikiri_html::parse(markup.as_bytes(), &options)
        .unwrap()
        .dom;
    let mut doc = Document::new();
    let mut pending = vec![(0, 0)];
    while let Some((source, parent)) = pending.pop() {
        for &child in &foreign.get_node(source).unwrap().children {
            let node = foreign.get_node(child).unwrap();
            if let Some(tag) = node.tag_name() {
                let css = format!(
                    "display:block;margin:0;{}",
                    node.attribute("style").unwrap_or("")
                );
                let own =
                    doc.append_element(Some(parent), tag, Style::default(), Some(css.as_str()));
                if let Some(name) = node.attribute("id") {
                    doc.set_element_attribute(own, "id", name).unwrap();
                }
                pending.push((child, own));
            } else if let Some(text) = node.text_content() {
                doc.append_text(parent, text);
            }
        }
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    (doc, cascade)
}

fn laid_out(markup: &str) -> Document {
    let (mut doc, cascade) = fixture(markup);
    let mut page = raikiri_traits::PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &cascade, page).unwrap();
    doc
}

fn id(doc: &Document, name: &str) -> usize {
    doc.nodes
        .iter()
        .position(|node| node.attribute("id") == Some(name))
        .unwrap()
}

fn boxes(doc: &Document, name: &str) -> Vec<(usize, f32, f32, f32, f32)> {
    let node = id(doc, name);
    doc.layout_fragments()
        .iter()
        .filter(|f| f.node_id == node)
        .map(|f| {
            (
                f.fragmentainer,
                f.rect.x,
                f.rect.y,
                f.rect.width,
                f.rect.height,
            )
        })
        .collect()
}

#[test]
fn nested_break_avoidance_resumes_a_wrapper_in_the_next_column() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:160px">
      <div id="wrapper"><div style="height:50px;break-inside:avoid"></div><div style="height:50px;break-inside:avoid"></div><div id="third" style="height:50px;break-inside:avoid"></div></div>
      <div id="last" style="height:50px;break-before:avoid;break-inside:avoid"></div>
    </div>"#,
    );
    assert_eq!(
        boxes(&doc, "wrapper"),
        vec![(0, 0.0, 0.0, 50.0, 100.0), (1, 50.0, 0.0, 50.0, 50.0)]
    );
    assert_eq!(boxes(&doc, "third"), vec![(1, 0.0, 0.0, 50.0, 50.0)]);
    assert_eq!(boxes(&doc, "last"), vec![(1, 50.0, 50.0, 50.0, 50.0)]);
}

#[test]
fn a_nested_forced_edge_overrides_the_following_avoidance() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:250px">
      <div style="height:50px"></div><div><div><div style="height:50px;break-after:column"></div><div id="forced" style="height:50px;break-before:avoid"></div></div></div>
    </div>"#,
    );
    assert_eq!(boxes(&doc, "forced"), vec![(1, 0.0, 0.0, 50.0, 50.0)]);
}

#[test]
fn a_first_child_constraint_propagates_to_an_inside_avoided_parent() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:250px">
      <div style="height:100px"></div><div id="second" style="height:100px"></div><div id="last" style="break-inside:avoid"><div style="height:100px;break-before:avoid"></div></div>
    </div>"#,
    );
    assert_eq!(boxes(&doc, "second"), vec![(1, 50.0, 0.0, 50.0, 100.0)]);
    assert_eq!(boxes(&doc, "last"), vec![(1, 50.0, 100.0, 50.0, 100.0)]);
}

#[test]
fn parallel_float_height_does_not_consume_the_connected_run() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:150px">
      <div style="height:100px"></div><div id="second" style="height:30px"></div>
      <div id="float" style="float:left;width:50%;height:20px"></div>
      <div id="last" style="height:70px;break-before:avoid;break-inside:avoid"></div>
    </div>"#,
    );
    assert_eq!(boxes(&doc, "second"), vec![(1, 50.0, 0.0, 50.0, 30.0)]);
    assert_eq!(boxes(&doc, "float"), vec![(1, 50.0, 30.0, 25.0, 20.0)]);
    assert_eq!(boxes(&doc, "last"), vec![(1, 50.0, 30.0, 50.0, 70.0)]);
}

#[test]
fn an_oversized_breakable_box_continues_across_additional_columns() {
    let doc = laid_out(
        r#"<div style="columns:4;column-fill:auto;gap:0;width:100px;height:100px">
      <div style="height:50px"></div><div id="previous" style="height:50px"></div>
      <div style="height:50px;break-before:avoid"></div><div id="tall" style="height:200px"></div>
    </div>"#,
    );
    assert_eq!(boxes(&doc, "previous"), vec![(1, 25.0, 0.0, 25.0, 50.0)]);
    assert_eq!(
        boxes(&doc, "tall"),
        vec![(2, 50.0, 0.0, 25.0, 100.0), (3, 75.0, 0.0, 25.0, 100.0)]
    );
}

#[test]
fn review_fixed_height_descendants_are_not_replayed_across_columns() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:100px">
        <div style="height:30px"></div>
        <div id="atomic" style="height:150px;break-before:avoid"><div id="first" style="height:75px"></div><div id="second" style="height:75px"></div></div>
        </div>"#,
    );
    assert_eq!(boxes(&doc, "atomic"), vec![(1, 50.0, 0.0, 50.0, 150.0)]);
    assert_eq!(
        doc.nodes[id(&doc, "first")].unrounded_layout.location.y,
        0.0
    );
    assert_eq!(
        doc.nodes[id(&doc, "second")].unrounded_layout.location.y,
        75.0
    );
}

#[test]
fn review_container_fragment_keeps_its_border_box_width() {
    for (width, expected) in [("120px", 120.0), ("50%", 400.0), ("auto", 800.0)] {
        let doc = laid_out(&format!(
            "<div id='columns' style='columns:2;column-fill:auto;gap:0;box-sizing:border-box;width:{width};height:130px;padding:10px;border:5px solid blue'><div style='height:20px;break-before:avoid'></div></div>",
        ));
        let root = id(&doc, "columns");
        assert_eq!(doc.nodes[root].unrounded_layout.size.width, expected);
        assert_eq!(boxes(&doc, "columns"), vec![(0, 0.0, 0.0, expected, 130.0)]);
    }
}

#[test]
fn review_an_oversized_unbreakable_box_moves_once_to_an_empty_column() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:100px">
        <div style="height:30px"></div><div id="atomic" style="height:150px;break-inside:avoid-column"></div>
        <div id="next" style="height:20px"></div></div>"#,
    );
    assert_eq!(boxes(&doc, "atomic"), vec![(1, 50.0, 0.0, 50.0, 150.0)]);
    assert_eq!(boxes(&doc, "next"), vec![(2, 100.0, 0.0, 50.0, 20.0)]);
}

#[test]
fn review_an_oversized_first_box_keeps_progress_on_the_empty_column() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:100px">
        <div id="atomic" style="height:150px;break-inside:avoid-column"></div>
        <div id="next" style="height:20px"></div></div>"#,
    );
    assert_eq!(boxes(&doc, "atomic"), vec![(0, 0.0, 0.0, 50.0, 150.0)]);
    assert_eq!(boxes(&doc, "next"), vec![(1, 50.0, 0.0, 50.0, 20.0)]);
}

#[test]
fn review_an_oversized_float_keeps_its_parallel_flow_position() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:100px">
        <div style="height:30px"></div><div id="float" style="float:left;width:25px;height:150px;break-inside:avoid-column"></div>
        <div id="next" style="height:20px"></div></div>"#,
    );
    assert_eq!(boxes(&doc, "float"), vec![(0, 0.0, 30.0, 25.0, 150.0)]);
    assert_eq!(boxes(&doc, "next"), vec![(0, 0.0, 30.0, 50.0, 20.0)]);
}

#[test]
fn fixed_height_flow_advance_preserves_visible_descendant_overflow() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:100px">
      <div style="height:50px"></div><div style="height:50px"></div>
      <div id="short" style="height:10px"><div style="height:20px"></div><div style="height:20px"></div></div>
      <div id="last" style="height:90px;break-before:avoid;break-inside:avoid"></div>
    </div>"#,
    );
    assert_eq!(boxes(&doc, "short"), vec![(1, 50.0, 0.0, 50.0, 10.0)]);
    assert_eq!(boxes(&doc, "last"), vec![(1, 50.0, 10.0, 50.0, 90.0)]);
    let child = doc.nodes[id(&doc, "short")].children[0];
    assert_eq!(doc.nodes[child].unrounded_layout.size.height, 20.0);
}

#[test]
fn inside_only_avoidance_keeps_the_existing_strategy() {
    let doc = laid_out(
        r#"<div id="columns" style="columns:2;column-fill:auto;gap:0;width:100px;height:160px"><div style="height:50px;break-inside:avoid"></div></div>"#,
    );
    let root = id(&doc, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(160.0), doc.nodes[root].multicol.unwrap())
            .unwrap();
    assert!(!supports(&doc, root, context));
}

#[test]
fn excessive_wrapper_depth_keeps_the_bounded_fallback() {
    let mut markup = String::from("<div id='columns' style='columns:2;width:100px;height:100px'>");
    for _ in 0..128 {
        markup.push_str("<div>");
    }
    markup.push_str("<div style='height:50px;break-before:avoid'></div>");
    for _ in 0..129 {
        markup.push_str("</div>");
    }
    let (mut doc, cascade) = fixture(&markup);
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let root = id(&doc, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap();
    assert!(!supports(&doc, root, context));
}

#[test]
fn decorated_and_positioned_boxes_keep_the_existing_geometry_strategy() {
    for decoration in [
        "margin-bottom:10px",
        "padding-top:10px",
        "border-top:2px solid",
        "position:relative;top:10px",
        "width:30px",
    ] {
        let markup = format!(
            "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div style='{decoration}'><div style='height:20px'></div><div style='height:20px;break-before:avoid'></div></div></div>"
        );
        let (mut doc, cascade) = fixture(&markup);
        apply_computed_to_style(&mut doc, &cascade).unwrap();
        let root = id(&doc, "columns");
        let context =
            FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
                .unwrap();
        assert!(!supports(&doc, root, context), "{decoration}");
    }
}

#[test]
fn an_inline_block_that_fits_a_column_stays_atomic() {
    let mut doc = laid_out(
        r#"<div id="columns" style="columns:2;column-fill:auto;gap:0;width:100px;height:100px">
        <div style="height:50px;break-before:avoid"></div>
        <div id="atomic" style="display:inline-block;width:100%"><div id="first" style="height:40px"></div><div id="second" style="height:40px"></div></div>
        </div>"#,
    );
    let root = id(&doc, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap();
    layout(
        &mut doc,
        root,
        context,
        Size {
            width: 100.0,
            height: 100.0,
        },
        Point::zero(),
    );
    assert_eq!(boxes(&doc, "atomic"), vec![(1, 50.0, 0.0, 50.0, 80.0)]);
    assert_eq!(
        doc.nodes[id(&doc, "first")].unrounded_layout.location.y,
        0.0
    );
    assert_eq!(
        doc.nodes[id(&doc, "second")].unrounded_layout.location.y,
        40.0
    );
}

#[test]
fn geometry_block_margins_preserve_the_following_box_position() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:50px"><div style="height:10px;margin-bottom:10px"></div><div id="next" style="height:20px;break-before:avoid-column"></div></div>"#,
    );
    assert_eq!(
        doc.nodes[id(&doc, "next")].unrounded_layout.location.y,
        20.0
    );
}

#[test]
fn geometry_a_decorated_wrapper_preserves_its_content_inset_and_width() {
    let doc = laid_out(
        r#"<div style="columns:2;column-fill:auto;gap:0;width:100px;height:100px"><div id="wrapper" style="width:30px;padding-top:10px;border-top:2px solid"><div id="first" style="height:20px"></div><div style="height:20px;break-before:avoid"></div></div></div>"#,
    );
    let wrapper = &doc.nodes[id(&doc, "wrapper")].unrounded_layout;
    assert_eq!(wrapper.size.width, 30.0);
    assert_eq!(
        doc.nodes[id(&doc, "first")].unrounded_layout.location.y,
        12.0
    );
}

#[test]
fn geometry_inline_margins_keep_the_existing_unfragmented_geometry() {
    for css in [
        "margin-left:10px",
        "width:20px;margin-left:auto;margin-right:auto",
    ] {
        let markup = |constraint| {
            format!(
                "<div id='columns' style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px'></div><div id='target' style='height:20px;{constraint};{css}'></div></div>"
            )
        };
        let reference = laid_out(&markup(""));
        let doc = laid_out(&markup("break-before:avoid-column"));
        let target = &doc.nodes[id(&doc, "target")].unrounded_layout;
        let original = &reference.nodes[id(&reference, "target")].unrounded_layout;
        assert_eq!(target.size.width, original.size.width, "{css}");
        assert_eq!(target.location.x, original.location.x, "{css}");
        let root = id(&doc, "columns");
        let context =
            FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
                .unwrap();
        assert!(!supports(&doc, root, context), "{css}");
    }
}

#[test]
fn geometry_wrapper_sizing_constraints_keep_the_existing_strategy() {
    let mut differences = Vec::new();
    for css in [
        "min-height:60px",
        "max-height:20px",
        "min-width:60px",
        "max-width:30px",
    ] {
        let markup = |constraint| {
            format!(
                "<div id='columns' style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div id='wrapper' style='{css}'><div style='height:20px'></div><div style='height:20px'></div></div><div id='next' style='height:20px;{constraint}'></div></div>"
            )
        };
        let reference = laid_out(&markup(""));
        let doc = laid_out(&markup("break-before:avoid-column"));
        let geometry = |tree: &Document| {
            (
                tree.nodes[id(tree, "wrapper")].unrounded_layout.size,
                tree.nodes[id(tree, "next")].unrounded_layout.location.y,
            )
        };
        if geometry(&doc) != geometry(&reference) {
            differences.push((css, geometry(&doc), geometry(&reference)));
        }
        let root = id(&doc, "columns");
        let context =
            FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
                .unwrap();
        assert!(!supports(&doc, root, context), "{css}");
    }
    assert!(differences.is_empty(), "{differences:?}");
}

#[test]
fn geometry_overflow_wrapper_preserves_float_occupied_height() {
    let markup = |constraint| {
        format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div id='wrapper' style='overflow:hidden'><div style='float:left;width:20px;height:40px'></div></div><div id='next' style='height:20px;{constraint}'></div></div>"
        )
    };
    let reference = laid_out(&markup(""));
    let doc = laid_out(&markup("break-before:avoid-column"));
    assert_eq!(
        doc.nodes[id(&doc, "next")].unrounded_layout.location.y,
        reference.nodes[id(&reference, "next")]
            .unrounded_layout
            .location
            .y,
    );
}

#[test]
fn geometry_clearance_preserves_the_existing_following_box_position() {
    let markup = |constraint| {
        format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='float:left;width:20px;height:40px'></div><div id='next' style='clear:both;height:20px;{constraint}'></div></div>"
        )
    };
    let reference = laid_out(&markup(""));
    let doc = laid_out(&markup("break-before:avoid-column"));
    assert_eq!(
        doc.nodes[id(&doc, "next")].unrounded_layout.location.y,
        reference.nodes[id(&reference, "next")]
            .unrounded_layout
            .location
            .y,
    );
}

fn prepared_flow() -> (Document, usize, FragmentationContext) {
    let (mut doc, cascade) = fixture(
        "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div id='wrapper'><div style='height:20px;break-before:avoid'></div><div style='height:20px'></div></div></div>",
    );
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let root = id(&doc, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap();
    (doc, root, context)
}

fn fanout_markup(count: usize, atomic: bool) -> String {
    let mut markup = String::from(
        "<div id='columns' style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'>",
    );
    if atomic {
        markup.push_str("<div style='height:150px;break-before:avoid'>");
    }
    for index in 0..count {
        markup.push_str(&format!(
            "<div style='break-before:avoid'><div id='leaf-{index}' style='height:1px'></div></div>"
        ));
    }
    if atomic {
        markup.push_str("</div>");
    }
    markup.push_str("</div>");
    markup
}

fn fanout_flow(count: usize, atomic: bool) -> (Document, usize, FragmentationContext) {
    let markup = fanout_markup(count, atomic);
    let (mut doc, cascade) = fixture(&markup);
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let root = id(&doc, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap();
    (doc, root, context)
}

#[test]
fn security_fanout_budget_stops_before_remeasuring_descendants() {
    let mut observed = Vec::new();
    for atomic in [false, true] {
        let (mut doc, root, context) = fanout_flow(1024, atomic);
        doc.fragment_tree.limit = 64;
        let last = id(&doc, "leaf-1023");
        assert_eq!(doc.nodes[last].unrounded_layout.size.height, 0.0);
        layout(
            &mut doc,
            root,
            context,
            Size {
                width: 100.0,
                height: 100.0,
            },
            Point::zero(),
        );
        observed.push((
            atomic,
            doc.nodes[last].unrounded_layout.size.height,
            doc.fragment_tree.limit_exceeded,
            doc.fragment_tree.fragments.len(),
        ));
    }
    assert_eq!(observed, vec![(false, 0.0, true, 0), (true, 0.0, true, 0)]);
}

#[test]
fn security_fanout_budget_preserves_under_budget_layout_and_typed_error() {
    for (count, limit) in [(4, 32), (1024, 64)] {
        let (mut doc, _, _) = fanout_flow(count, false);
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        doc.fragment_tree.limit = limit;
        let mut page = raikiri_traits::PageBox::new();
        page.width = 800.0;
        page.height = 600.0;
        let result = layout_single_page(&mut doc, &cascade, page);
        if count == 4 {
            assert!(result.is_ok());
            assert!(!doc.fragment_tree.limit_exceeded);
            assert_eq!(
                doc.nodes[id(&doc, "leaf-3")].unrounded_layout.size.height,
                1.0
            );
        } else {
            assert!(matches!(
                result,
                Err(LayoutError::FragmentLimitExceeded { limit: 64 })
            ));
            doc.fragment_tree.limit = 4096;
            assert!(layout_single_page(&mut doc, &cascade, page).is_ok());
            assert!(!doc.fragment_tree.limit_exceeded);
            assert_eq!(doc.fragment_tree.break_flow_work_used, 4096);
        }
    }
    let markup = fanout_markup(4, false).replace("break-before:avoid", "");
    let (mut doc, cascade) = fixture(&markup);
    doc.fragment_tree.limit = 32;
    assert!(layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::new()).is_ok());
    assert_eq!(doc.fragment_tree.break_flow_work_used, 0);
}

#[test]
fn security_atomic_measurement_work_is_shared_and_resets_on_clear() {
    let first = fanout_markup(4, true);
    let second = first
        .replace("columns", "other-columns")
        .replace("leaf-", "other-");
    let (mut doc, cascade) = fixture(&format!("{first}{second}"));
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    doc.fragment_tree.limit = 40;
    let first = id(&doc, "columns");
    let second = id(&doc, "other-columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[first].multicol.unwrap())
            .unwrap();
    let size = Size {
        width: 100.0,
        height: 100.0,
    };
    layout(&mut doc, first, context, size, Point::zero());
    assert!(!doc.fragment_tree.limit_exceeded);
    assert_eq!(doc.fragment_tree.break_flow_work_used, 23);
    assert_eq!(doc.fragment_tree.fragments.len(), 2);
    layout(&mut doc, second, context, size, Point::zero());
    assert!(doc.fragment_tree.limit_exceeded);
    assert_eq!(doc.fragment_tree.break_flow_work_used, 23);
    assert_eq!(
        doc.nodes[id(&doc, "other-3")].unrounded_layout.size.height,
        0.0
    );
    doc.fragment_tree.clear();
    assert_eq!(doc.fragment_tree.break_flow_work_used, 0);
    layout(&mut doc, second, context, size, Point::zero());
    assert!(!doc.fragment_tree.limit_exceeded);
    assert_eq!(doc.fragment_tree.break_flow_work_used, 23);
}

#[test]
fn security_remaining_fragment_capacity_bounds_measurement_work() {
    let (mut doc, cascade) = fixture(
        "<div id='first' style='columns:2;gap:0;width:100px;height:100px'><div style='height:900px;break-before:avoid'></div></div><div id='second' style='columns:2;gap:0;width:100px;height:100px'><div><div id='last' style='height:1px;break-before:avoid'></div></div></div>",
    );
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    doc.fragment_tree.limit = 12;
    let first = id(&doc, "first");
    let second = id(&doc, "second");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[first].multicol.unwrap())
            .unwrap();
    let size = Size {
        width: 100.0,
        height: 100.0,
    };
    layout(&mut doc, first, context, size, Point::zero());
    assert_eq!(doc.fragment_tree.fragments.len(), 10);
    assert_eq!(doc.fragment_tree.break_flow_work_used, 1);
    layout(&mut doc, second, context, size, Point::zero());
    assert!(doc.fragment_tree.limit_exceeded);
    assert_eq!(
        doc.nodes[id(&doc, "last")].unrounded_layout.size.height,
        0.0
    );
    assert_eq!(doc.fragment_tree.fragments.len(), 10);
}

#[test]
fn security_measurement_budget_charges_repeated_wrapper_visits_at_the_boundary() {
    for limit in [8, 9] {
        let (mut doc, cascade) = fixture(
            "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div><div><div id='last' style='height:1px;break-before:avoid'></div></div></div></div>",
        );
        apply_computed_to_style(&mut doc, &cascade).unwrap();
        doc.fragment_tree.limit = limit;
        let root = id(&doc, "columns");
        let context =
            FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
                .unwrap();
        layout(
            &mut doc,
            root,
            context,
            Size {
                width: 100.0,
                height: 100.0,
            },
            Point::zero(),
        );
        if limit == 8 {
            assert!(doc.fragment_tree.limit_exceeded);
            assert_eq!(
                doc.nodes[id(&doc, "last")].unrounded_layout.size.height,
                0.0
            );
        } else {
            assert!(!doc.fragment_tree.limit_exceeded);
            assert_eq!(doc.fragment_tree.break_flow_work_used, 9);
            assert_eq!(
                doc.nodes[id(&doc, "last")].unrounded_layout.size.height,
                1.0
            );
        }
    }
}

#[test]
fn security_measurement_counts_nested_out_of_flow_but_skips_unmeasured_roots() {
    let leaves = "<div style='height:1px'></div>".repeat(128);
    let (mut doc, cascade) = fixture(&format!(
        "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div style='height:150px;break-before:avoid'><div style='position:absolute'>{leaves}</div></div></div>",
    ));
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    doc.fragment_tree.limit = 64;
    let root = id(&doc, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap();
    assert!(supports(&doc, root, context));
    assert!(!reserve_measurement_work(&mut doc, root));
    assert!(doc.fragment_tree.limit_exceeded);

    let (mut doc, cascade) = fixture(&format!(
        "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div style='display:none'>{leaves}</div><div style='position:absolute'>{leaves}</div> <div id='last' style='height:1px;break-before:avoid'></div></div>",
    ));
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    doc.fragment_tree.limit = 8;
    let root = id(&doc, "columns");
    assert!(reserve_measurement_work(&mut doc, root));
    assert_eq!(doc.fragment_tree.break_flow_work_used, 1);
}

#[test]
fn security_hidden_children_of_a_measured_atomic_box_consume_work_budget() {
    let hidden = "<div style='display:none'></div>".repeat(128);
    let (mut doc, cascade) = fixture(&format!(
        "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div style='height:150px;break-before:avoid'>{hidden}<div style='height:1px'></div></div></div>",
    ));
    doc.fragment_tree.limit = 64;
    assert!(matches!(
        layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::new()),
        Err(LayoutError::FragmentLimitExceeded { limit: 64 })
    ));
}

#[test]
fn security_atomic_child_index_and_whitespace_scans_consume_work_budget() {
    for leading in [String::new(), " ".repeat(128)] {
        let children = "<div style='height:1px'></div>".repeat(16);
        let (mut doc, cascade) = fixture(&format!(
            "<div style='columns:2;gap:0;width:100px;height:100px'><div style='height:150px;break-before:avoid'>{leading}{children}</div></div>",
        ));
        doc.fragment_tree.limit = 64;
        assert!(matches!(
            layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::new()),
            Err(LayoutError::FragmentLimitExceeded { limit: 64 })
        ));
    }
}

#[test]
fn security_filtered_metadata_still_consumes_its_parent_scan_work() {
    let (mut doc, root, _) = prepared_flow();
    let wrapper = id(&doc, "wrapper");
    let comment = doc.append_comment(Some(wrapper), "opaque");
    doc.mark_in_document_flags();
    assert!(!doc.nodes[comment].is_in_document());
    doc.fragment_tree.limit = 9;
    assert!(reserve_measurement_work(&mut doc, root));
    assert_eq!(doc.fragment_tree.break_flow_work_used, 9);
}

#[test]
fn security_nested_out_of_flow_fragment_exhaustion_stops_collection() {
    let (mut doc, cascade) = fixture(
        "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div><div style='height:150px;break-before:avoid'><div style='position:absolute;columns:2;gap:0;width:100px;height:100px'><div style='height:3500px;break-before:avoid'></div></div></div></div></div>",
    );
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    doc.fragment_tree.limit = 32;
    let root = id(&doc, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap();
    assert!(supports(&doc, root, context));
    assert_eq!(
        layout(
            &mut doc,
            root,
            context,
            Size {
                width: 100.0,
                height: 100.0
            },
            Point::zero()
        ),
        100.0,
    );
    assert!(doc.fragment_tree.limit_exceeded);
    assert_eq!(doc.fragment_tree.fragments.len(), 32);
    assert!(doc.fragment_tree.break_flow_work_used < 32);
    assert!(boxes(&doc, "columns").is_empty());
}

#[test]
fn unsupported_text_and_parallel_or_hidden_nodes_keep_the_scope_bounded() {
    let (mut doc, root, context) = prepared_flow();
    doc.append_comment(Some(root), "opaque");
    doc.append_text(root, " ");
    let hidden = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
    doc.nodes[hidden].style.display = Display::None;
    let positioned = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
    doc.nodes[positioned].style.position = TaffyPosition::Absolute;
    doc.mark_in_document_flags();
    assert!(supports(&doc, root, context));
    doc.append_text(root, "visible");
    doc.mark_in_document_flags();
    assert!(!supports(&doc, root, context));
}

#[test]
fn an_outer_forced_edge_and_the_depth_limit_preserve_own_constraints() {
    assert_eq!(
        propagated(BreakBetween::Column, BreakBetween::AvoidColumn),
        BreakBetween::Column,
    );
    let (mut doc, _, _) = prepared_flow();
    let wrapper = id(&doc, "wrapper");
    doc.nodes[wrapper].break_before = BreakBetween::Always;
    assert_eq!(
        descendant_edge(&doc, wrapper, true, 128),
        BreakBetween::Always,
    );
}

#[test]
fn fragment_budget_exhaustion_stops_each_projection_stage() {
    for limit in 0..=2 {
        let (mut doc, root, context) = prepared_flow();
        doc.fragment_tree.limit = limit;
        assert_eq!(
            layout(
                &mut doc,
                root,
                context,
                Size {
                    width: 100.0,
                    height: 100.0
                },
                Point::zero()
            ),
            100.0
        );
        assert!(doc.fragment_tree.limit_exceeded);
        assert!(doc.fragment_tree.fragments.is_empty());
    }
    for limit in 4..=6 {
        let (mut doc, cascade) = fixture(
            "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div><div style='height:350px;break-before:avoid'></div></div></div>",
        );
        apply_computed_to_style(&mut doc, &cascade).unwrap();
        let root = id(&doc, "columns");
        let context =
            FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
                .unwrap();
        doc.fragment_tree.limit = limit;
        layout(
            &mut doc,
            root,
            context,
            Size {
                width: 100.0,
                height: 100.0,
            },
            Point::zero(),
        );
        assert!(doc.fragment_tree.limit_exceeded);
        assert_eq!(doc.fragment_tree.fragments.len(), limit);
    }
    let (mut empty, cascade) =
        fixture("<div id='columns' style='columns:2;gap:0;width:100px;height:100px'></div>");
    apply_computed_to_style(&mut empty, &cascade).unwrap();
    let root = id(&empty, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), empty.nodes[root].multicol.unwrap())
            .unwrap();
    empty.fragment_tree.limit = 0;
    layout(
        &mut empty,
        root,
        context,
        Size {
            width: 100.0,
            height: 100.0,
        },
        Point::zero(),
    );
    assert!(empty.fragment_tree.limit_exceeded);
    let (mut doc, root, mut context) = prepared_flow();
    context.available_height = None;
    assert_eq!(
        layout(
            &mut doc,
            root,
            context,
            Size {
                width: 100.0,
                height: 100.0
            },
            Point::zero()
        ),
        100.0
    );
    assert!(doc.fragment_tree.fragments.is_empty());
}

#[test]
fn collection_stops_and_unwinds_ancestors_when_the_budget_was_exhausted() {
    let (mut doc, root, context) = prepared_flow();
    doc.fragment_tree.limit = 0;
    assert!(
        doc.fragment_tree
            .try_push(fragment(
                root,
                None,
                0,
                FragmentRect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0
                },
            ))
            .is_none()
    );
    let mut ancestors = Vec::new();
    let mut out = Vec::new();
    let wrapper = id(&doc, "wrapper");
    collect(&mut doc, wrapper, context, &mut ancestors, &mut out);
    assert!(ancestors.is_empty());
    assert!(out.is_empty());
    assert_eq!(
        layout(
            &mut doc,
            root,
            context,
            Size {
                width: 100.0,
                height: 100.0
            },
            Point::zero()
        ),
        100.0
    );
}

#[test]
fn review_nested_positioned_children_of_projected_wrappers_keep_the_previous_strategy() {
    let (mut doc, cascade) = fixture(
        "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div style='position:relative'><div style='height:75px;break-before:avoid'></div><div style='height:75px'></div><div style='position:absolute;left:0;top:0;width:10px;height:10px'></div></div></div>",
    );
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let root = id(&doc, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap();
    assert!(!supports(&doc, root, context));
}

#[test]
fn review_parallel_float_does_not_enlarge_its_normal_flow_wrapper() {
    let doc = laid_out(
        "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div id='wrapper'><div style='float:left;width:10px;height:80px'></div><div style='height:20px;break-before:avoid'></div></div></div>",
    );
    assert_eq!(boxes(&doc, "wrapper"), vec![(0, 0.0, 0.0, 50.0, 20.0)]);
}

#[test]
fn review_interacting_floats_keep_the_existing_geometry_strategy() {
    let (mut doc, cascade) = fixture(
        "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div style='float:left;width:30px;height:20px'></div><div style='float:left;width:30px;height:20px'></div><div style='height:20px;break-before:avoid'></div></div>",
    );
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let root = id(&doc, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap();
    assert!(!supports(&doc, root, context));
}

#[test]
fn review_other_block_displays_observe_forced_column_boundaries() {
    for display in ["flow-root", "list-item"] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px'></div><div id='next' style='display:{display};list-style:none;height:20px;break-before:column'></div></div>"
        ));
        assert_eq!(
            boxes(&doc, "next"),
            vec![(1, 50.0, 0.0, 50.0, 20.0)],
            "{display}"
        );
    }
}

#[test]
fn review_auto_width_atomic_inline_box_preserves_its_measured_width() {
    let mut doc = laid_out(
        "<div id='columns' style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px;break-before:avoid'></div><div id='atomic' style='display:inline-block'><div style='width:20px;height:20px'></div></div></div>",
    );
    let root = id(&doc, "columns");
    let context =
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap();
    doc.fragment_tree.clear();
    layout(
        &mut doc,
        root,
        context,
        Size {
            width: 100.0,
            height: 100.0,
        },
        Point::zero(),
    );
    assert_eq!(boxes(&doc, "atomic")[0].3, 20.0);
}

#[test]
fn review_auto_width_replaced_box_preserves_its_intrinsic_size() {
    let (mut doc, _) = fixture(
        "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px;break-before:avoid'></div><img id='image'></div>",
    );
    let image = id(&doc, "image");
    doc.set_element_attribute(image, "src", "green.png")
        .unwrap();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &cascade, page).unwrap();
    assert_eq!(boxes(&doc, "image")[0].3, 100.0);
    assert_eq!(boxes(&doc, "image")[0].4, 50.0);
}

struct ReviewIntrinsicResolver;
impl raikiri_traits::ReplacedResolver for ReviewIntrinsicResolver {
    fn resolve(
        &self,
        _request: raikiri_traits::ResolverRequest<'_>,
    ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
        Ok(raikiri_traits::ResolvedIntrinsic {
            intrinsic: raikiri_traits::IntrinsicBox::new(20.0, 10.0),
            disposition: raikiri_traits::ResolveDisposition::Ok,
        })
    }
}

#[test]
fn review_resolved_and_unresolved_images_keep_their_natural_auto_width() {
    for resolved in [false, true] {
        let (mut doc, _) = fixture(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px;break-before:avoid'></div><img id='image'></div>",
        );
        let image = id(&doc, "image");
        doc.set_element_attribute(image, "src", "https://example.test/intrinsic.png")
            .unwrap();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        let mut page = raikiri_traits::PageBox::new();
        page.width = 800.0;
        page.height = 600.0;
        if resolved {
            crate::layout::layout_single_page_with_resolver(
                &mut doc,
                &cascade,
                page,
                &ReviewIntrinsicResolver,
            )
            .unwrap();
        } else {
            layout_single_page(&mut doc, &cascade, page).unwrap();
        }
        assert_eq!(boxes(&doc, "image")[0].3, if resolved { 20.0 } else { 0.0 });
        assert_eq!(boxes(&doc, "image")[0].4, if resolved { 10.0 } else { 0.0 });
    }
}

#[test]
fn review_multiple_float_fallback_matches_the_unconstrained_geometry() {
    let mut observed = Vec::new();
    for edge in ["", "break-before:avoid"] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div id='a' style='float:left;width:30px;height:20px'></div><div id='b' style='float:left;width:30px;height:20px'></div><div style='height:20px;{edge}'></div></div>"
        ));
        observed.push([
            doc.nodes[id(&doc, "a")].unrounded_layout,
            doc.nodes[id(&doc, "b")].unrounded_layout,
        ]);
    }
    assert_eq!(observed[0], observed[1]);
}

#[test]
fn review_flow_root_preserves_float_height_inside_its_atomic_box() {
    let doc = laid_out(
        "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:70px'></div><div id='atomic' style='display:flow-root;break-before:avoid'><div style='float:left;width:10px;height:40px'></div></div></div>",
    );
    assert_eq!(boxes(&doc, "atomic"), vec![(1, 50.0, 0.0, 50.0, 40.0)]);
}

#[test]
fn review_replaced_leaves_overflow_once_without_replaying_columns() {
    for tag in ["img", "canvas"] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:30px;break-before:avoid'></div><{tag} id='replaced' style='width:20px;height:150px'></{tag}></div>",
        ));
        assert_eq!(boxes(&doc, "replaced"), vec![(1, 50.0, 0.0, 20.0, 150.0)]);
    }
}

#[test]
fn review_replaced_first_leaf_overflows_its_empty_column_once() {
    for tag in ["img", "canvas"] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><{tag} id='replaced' style='width:20px;height:150px;break-before:avoid'></{tag}></div>",
        ));
        assert_eq!(boxes(&doc, "replaced"), vec![(0, 0.0, 0.0, 20.0, 150.0)]);
    }
}

#[test]
fn review_rtl_columns_progress_leftward_including_overflow_columns() {
    let doc = laid_out(
        "<div style='direction:rtl;columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div id='first' style='height:60px'></div><div id='second' style='height:60px;break-before:avoid'></div><div id='third' style='height:60px;break-before:column'></div></div>",
    );
    assert_eq!(boxes(&doc, "first"), vec![(0, 50.0, 0.0, 50.0, 60.0)]);
    assert_eq!(boxes(&doc, "second"), vec![(1, 0.0, 0.0, 50.0, 60.0)]);
    assert_eq!(boxes(&doc, "third"), vec![(2, -50.0, 0.0, 50.0, 60.0)]);
}

#[test]
fn review_rtl_narrow_boxes_use_their_containing_blocks_inline_direction() {
    for direction in ["ltr", "rtl"] {
        let doc = laid_out(&format!(
            "<div style='direction:rtl;columns:2;column-fill:auto;gap:10px;width:110px;height:100px'><div id='first' style='direction:{direction};width:20px;height:20px'></div><div id='second' style='direction:{direction};width:20px;height:20px;break-before:column'></div></div>",
        ));
        assert_eq!(boxes(&doc, "first"), vec![(0, 90.0, 0.0, 20.0, 20.0)]);
        assert_eq!(boxes(&doc, "second"), vec![(1, 30.0, 0.0, 20.0, 20.0)]);
    }
}

#[test]
fn review_empty_flow_box_keeps_its_forced_after_boundary() {
    for edge in ["column", "always"] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:0;break-after:{edge}'></div><div id='next' style='height:20px'></div></div>",
        ));
        assert_eq!(boxes(&doc, "next"), vec![(1, 50.0, 0.0, 50.0, 20.0)]);
    }
}

#[test]
fn review_empty_flow_box_allows_a_following_forced_before_boundary() {
    let doc = laid_out(
        "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:0'></div><div id='next' style='height:20px;break-before:column'></div></div>",
    );
    assert_eq!(boxes(&doc, "next"), vec![(1, 50.0, 0.0, 50.0, 20.0)]);
}

#[test]
fn review_leading_forced_edge_and_parallel_float_do_not_make_empty_columns() {
    for prefix in [
        "",
        "<div style='height:0'></div>",
        "<div style='float:left;width:10px;height:20px;break-after:column'></div>",
    ] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'>{prefix}<div id='next' style='height:20px;break-before:avoid'></div></div>",
        ));
        assert_eq!(boxes(&doc, "next"), vec![(0, 0.0, 0.0, 50.0, 20.0)]);
    }
    let doc = laid_out(
        "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div id='next' style='height:20px;break-before:column'></div></div>",
    );
    assert_eq!(boxes(&doc, "next"), vec![(0, 0.0, 0.0, 50.0, 20.0)]);
}

#[test]
fn review_rtl_column_alignment_preserves_physical_float_sides() {
    for direction in ["ltr", "rtl"] {
        for (side, inline_offset) in [("left", 0.0), ("right", 30.0)] {
            let doc = laid_out(&format!(
                "<div style='direction:{direction};columns:2;column-fill:auto;gap:10px;width:110px;height:100px'><div style='height:20px;break-before:avoid'></div><div id='float' style='float:{side};width:20px;height:20px'></div></div>",
            ));
            let column_origin = if direction == "rtl" { 60.0 } else { 0.0 };
            assert_eq!(
                boxes(&doc, "float"),
                vec![(0, column_origin + inline_offset, 20.0, 20.0, 20.0)]
            );
        }
    }
}

#[test]
fn review_nested_rtl_column_wrapper_keeps_local_inline_alignment() {
    let doc = laid_out(
        "<div style='direction:rtl;columns:2;column-fill:auto;gap:10px;width:110px;height:100px'><div id='wrapper'><div id='first' style='width:20px;height:60px'></div><div id='second' style='width:20px;height:60px;break-before:avoid'></div></div></div>",
    );
    assert_eq!(
        boxes(&doc, "wrapper"),
        vec![(0, 60.0, 0.0, 50.0, 60.0), (1, 0.0, 0.0, 50.0, 60.0)]
    );
    assert_eq!(boxes(&doc, "first"), vec![(0, 30.0, 0.0, 20.0, 60.0)]);
    assert_eq!(boxes(&doc, "second"), vec![(1, 30.0, 0.0, 20.0, 60.0)]);
}

#[test]
fn review_atomic_text_box_honors_its_forced_outer_column_edge() {
    for edge in ["column", "always"] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px'></div><div id='text' style='height:40px;break-before:{edge}'>Text</div></div>",
        ));
        assert_eq!(boxes(&doc, "text"), vec![(1, 50.0, 0.0, 50.0, 40.0)]);
    }
}

#[test]
fn review_oversized_text_box_keeps_one_measured_text_subtree() {
    let doc = laid_out(
        "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px;break-before:avoid'></div><div id='text' style='height:150px'>Text</div></div>",
    );
    assert_eq!(boxes(&doc, "text"), vec![(1, 50.0, 0.0, 50.0, 150.0)]);
}

#[test]
fn review_normal_flow_forced_after_moves_the_following_float_boundary_once() {
    for edge in ["column", "always"] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:30px;break-after:{edge}'></div><div id='float' style='float:left;width:10px;height:20px'></div><div id='next' style='height:20px'></div></div>",
        ));
        assert_eq!(boxes(&doc, "float"), vec![(1, 50.0, 0.0, 10.0, 20.0)]);
        assert_eq!(boxes(&doc, "next"), vec![(1, 50.0, 0.0, 50.0, 20.0)]);
    }
}

#[test]
fn review_atomic_text_box_propagates_its_forced_after_edge() {
    let doc = laid_out(
        "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:40px;break-after:column'>Text</div><div id='next' style='height:20px'></div></div>",
    );
    assert_eq!(boxes(&doc, "next"), vec![(1, 50.0, 0.0, 50.0, 20.0)]);
}

#[test]
fn review_directly_projected_text_keeps_the_existing_strategy() {
    let (mut doc, cascade) = fixture(
        "<div id='columns' style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'>Text<div style='height:20px;break-before:column'></div></div>",
    );
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    let root = id(&doc, "columns");
    assert!(!supports(
        &doc,
        root,
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap()
    ));
}

fn review_formatting_context_forced_edges(display: &str) {
    for edge in ["column", "always"] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px'></div><div id='atomic' style='display:{display};height:40px;break-before:{edge}'><div style='width:10px;height:10px'></div></div></div>",
        ));
        assert_eq!(boxes(&doc, "atomic"), vec![(1, 50.0, 0.0, 50.0, 40.0)]);
    }
    let doc = laid_out(&format!(
        "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='display:{display};height:40px;break-after:column'><div style='height:10px'></div></div><div id='next' style='height:20px'></div></div>",
    ));
    assert_eq!(boxes(&doc, "next"), vec![(1, 50.0, 0.0, 50.0, 20.0)]);
}

#[test]
fn review_flex_root_observes_its_forced_outer_column_edges() {
    review_formatting_context_forced_edges("flex");
}

#[test]
fn review_grid_root_observes_its_forced_outer_column_edges() {
    review_formatting_context_forced_edges("grid");
}

#[test]
fn review_formatting_context_auto_height_keeps_measured_child_positions() {
    for display in ["flex", "grid"] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px'></div><div id='atomic' style='display:{display};break-before:column'><div id='child' style='width:10px;height:30px'></div></div></div>",
        ));
        assert_eq!(boxes(&doc, "atomic"), vec![(1, 50.0, 0.0, 50.0, 30.0)]);
        assert!(boxes(&doc, "child").is_empty());
        assert_eq!(
            doc.nodes[id(&doc, "child")].unrounded_layout.location.y,
            0.0
        );
        assert_eq!(
            doc.nodes[id(&doc, "child")].unrounded_layout.size.height,
            30.0
        );
    }
}

#[test]
fn review_internal_flex_and_grid_breaks_keep_the_existing_strategy() {
    for display in ["flex", "grid"] {
        let (mut doc, cascade) = fixture(&format!(
            "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div style='height:20px'></div><div style='display:{display};break-before:column'><div style='height:20px;break-before:column'></div></div></div>",
        ));
        apply_computed_to_style(&mut doc, &cascade).unwrap();
        let root = id(&doc, "columns");
        assert!(!supports(
            &doc,
            root,
            FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
                .unwrap()
        ));
    }
}

#[test]
fn review_formatting_context_declared_width_and_oversize_stay_monolithic() {
    for display in ["flex", "grid"] {
        let doc = laid_out(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='height:20px'></div><div id='atomic' style='display:{display};width:20px;height:150px;break-before:column'><div style='height:10px'></div></div></div>",
        ));
        assert_eq!(boxes(&doc, "atomic"), vec![(1, 50.0, 0.0, 20.0, 150.0)]);
    }
}

#[test]
fn review_inline_formatting_contexts_keep_the_existing_strategy() {
    for display in ["inline-flex", "inline-grid"] {
        let (mut doc, cascade) = fixture(&format!(
            "<div id='columns' style='columns:2;gap:0;width:100px;height:100px'><div style='height:20px'></div><div style='display:{display};height:20px;break-before:column'></div></div>",
        ));
        apply_computed_to_style(&mut doc, &cascade).unwrap();
        let root = id(&doc, "columns");
        assert!(!supports(
            &doc,
            root,
            FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
                .unwrap()
        ));
    }
}

#[test]
fn review_root_column_fragments_start_at_the_resolved_content_origin() {
    let doc = laid_out(
        "<div id='columns' style='columns:2;column-fill:auto;gap:0;box-sizing:border-box;width:130px;height:130px;padding:10px;border:5px solid blue'><div id='first' style='height:20px'></div><div id='next' style='height:20px;break-before:column'></div></div>",
    );
    assert_eq!(boxes(&doc, "columns"), vec![(0, 0.0, 0.0, 130.0, 130.0)]);
    assert_eq!(boxes(&doc, "first"), vec![(0, 15.0, 15.0, 50.0, 20.0)]);
    assert_eq!(boxes(&doc, "next"), vec![(1, 65.0, 15.0, 50.0, 20.0)]);
}

#[test]
fn review_resumed_wrappers_keep_the_content_origin_only_at_the_root_edge() {
    let doc = laid_out(
        "<div style='columns:2;column-fill:auto;gap:0;box-sizing:border-box;width:130px;height:130px;padding:10px;border:5px solid blue'><div id='wrapper'><div id='first' style='height:20px'></div><div id='next' style='height:20px;break-before:column'></div></div></div>",
    );
    assert_eq!(
        boxes(&doc, "wrapper"),
        vec![(0, 15.0, 15.0, 50.0, 20.0), (1, 65.0, 15.0, 50.0, 20.0)]
    );
    assert_eq!(boxes(&doc, "first"), vec![(0, 0.0, 0.0, 50.0, 20.0)]);
    assert_eq!(boxes(&doc, "next"), vec![(1, 0.0, 0.0, 50.0, 20.0)]);
}

#[test]
fn review_between_box_constraints_preserve_joint_float_text_exclusion() {
    for edge in ["", "break-before:avoid"] {
        let (mut doc, cascade) = fixture(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='float:left;width:20px;height:40px'></div><div id='text' style='height:40px;font-family:Ahem;font-size:20px;line-height:20px'>M M</div><div style='height:1px;{edge}'></div></div>",
        ));
        doc.set_font_collection(crate::layout::test_support::ifc_ahem_fonts());
        layout_single_page(
            &mut doc,
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let lines = doc.nodes[id(&doc, "text")]
            .ifc
            .as_ref()
            .unwrap()
            .lines
            .as_ref()
            .unwrap();
        assert!(lines.beside_floats, "edge={edge:?}");
    }
}

#[test]
fn review_percentage_content_origin_uses_the_parent_basis_in_both_directions() {
    for (direction, first, next) in [("ltr", 25.0, 65.0), ("rtl", 65.0, 25.0)] {
        let doc = laid_out(&format!(
            "<div style='width:200px'><div id='columns' style='direction:{direction};columns:2;column-fill:auto;gap:0;box-sizing:border-box;width:130px;height:130px;padding:10%;border:5px solid blue'><div id='first' style='height:20px'></div><div id='next' style='height:20px;break-before:column'></div></div></div>"
        ));
        assert_eq!(boxes(&doc, "columns"), vec![(0, 0.0, 0.0, 130.0, 130.0)]);
        assert_eq!(boxes(&doc, "first"), vec![(0, first, 25.0, 40.0, 20.0)]);
        assert_eq!(boxes(&doc, "next"), vec![(1, next, 25.0, 40.0, 20.0)]);
    }
}

#[test]
fn review_generated_content_provenance_is_refreshed_by_the_canonical_bridge() {
    let (mut doc, _) = fixture(
        "<style style='display:none!important'>#wrapper.enabled::before{content:'X'}#wrapper.disabled::before{content:none}#wrapper.hidden::after{content:'Y';display:none}#wrapper.empty::before{content:''}#wrapper.positioned::after{content:'Y';position:absolute}#wrapper.floated::before{content:'X';float:left}</style><div id='columns' style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div id='wrapper'><div style='height:60px'></div><div style='height:60px;break-before:avoid'></div></div></div>",
    );
    let wrapper = id(&doc, "wrapper");
    for (class, expected) in [
        ("enabled", true),
        ("disabled", false),
        ("hidden", false),
        ("empty", true),
        ("positioned", true),
        ("floated", true),
        ("", false),
        ("enabled", true),
    ] {
        doc.set_element_attribute(wrapper, "class", class).unwrap();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        apply_computed_to_style(&mut doc, &cascade).unwrap();
        assert_eq!(doc.nodes[wrapper].has_before_or_after_content, expected);
        let root = id(&doc, "columns");
        let context =
            FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
                .unwrap();
        assert_eq!(supports(&doc, root, context), !expected);
    }
}

#[test]
fn review_generated_only_atomic_box_has_one_measured_fragment() {
    let doc = laid_out(
        "<style style='display:none!important'>#wrapper::before{content:'X'}#wrapper::after{content:'Y'}</style><div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div id='wrapper' style='height:150px'></div><div style='height:1px;break-before:avoid'></div></div>",
    );
    assert_eq!(boxes(&doc, "wrapper"), vec![(0, 0.0, 0.0, 50.0, 150.0)]);
}

#[test]
fn review_atomic_internal_float_text_keeps_its_measured_subtree() {
    let doc = laid_out(
        "<div id='columns' style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div id='atomic' style='display:flow-root;height:40px;break-before:avoid'><div style='float:left;width:20px;height:40px'></div><div id='text' style='height:40px;font-size:20px;line-height:20px'>MMMM</div></div></div>",
    );
    assert_eq!(boxes(&doc, "atomic"), vec![(0, 0.0, 0.0, 50.0, 40.0)]);
    assert!(
        doc.nodes[id(&doc, "text")]
            .ifc
            .as_ref()
            .unwrap()
            .lines
            .is_some()
    );
    let root = id(&doc, "columns");
    assert!(supports(
        &doc,
        root,
        FragmentationContext::resolve(100.0, Some(100.0), doc.nodes[root].multicol.unwrap())
            .unwrap()
    ));
}

#[test]
fn review_full_width_float_preserves_vertically_displaced_text_lines() {
    for edge in ["", "break-before:avoid"] {
        let (mut doc, cascade) = fixture(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'><div style='float:left;width:100px;height:40px'></div><div id='text' style='height:60px;font-family:Ahem;font-size:20px;line-height:20px'>M M</div><div style='height:1px;{edge}'></div></div>"
        ));
        doc.set_font_collection(crate::layout::test_support::ifc_ahem_fonts());
        layout_single_page(
            &mut doc,
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let lines = doc.nodes[id(&doc, "text")]
            .ifc
            .as_ref()
            .unwrap()
            .lines
            .as_ref()
            .unwrap();
        assert!(!lines.beside_floats);
        assert_eq!(lines.line_top(0), 40.0, "edge={edge:?}");
    }
}
