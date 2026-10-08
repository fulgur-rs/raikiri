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
    layout(&mut doc, root, context, 100.0);
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
