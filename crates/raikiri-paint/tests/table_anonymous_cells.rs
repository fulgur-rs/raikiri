//! Anonymous table-cell groups and HTML column dimensions against literal references.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_dom::{BundledFace, Document, build_bundled_font_collection, layout_single_page};
use raikiri_style::{CascadeResult, build_rule_tree, cascade};
use raikiri_traits::PageBox;
use taffy::Style;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn document() -> (Document, usize) {
    let mut doc = Document::new();
    doc.set_font_collection(
        build_bundled_font_collection(
            vec![BundledFace {
                family: "Ahem".into(),
                bytes: AHEM.to_vec(),
            }],
            false,
        )
        .unwrap(),
    );
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    (doc, body)
}

fn element(doc: &mut Document, parent: usize, style: &str) -> usize {
    doc.append_element(Some(parent), "div", Style::default(), Some(style))
}

fn page() -> PageBox {
    let mut page = PageBox::new();
    page.width = 120.0;
    page.height = 100.0;
    page
}

fn layout(doc: &mut Document) -> CascadeResult {
    doc.mark_in_document_flags();
    let rules = build_rule_tree(doc);
    let computed = cascade(doc, &rules).unwrap();
    layout_single_page(doc, &computed, page()).unwrap();
    computed
}

fn scene(doc: &Document, computed: &CascadeResult) -> Scene {
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page(&mut scene, doc, computed, page()).unwrap();
    scene
}

fn raster(scene: Scene) -> Vec<u8> {
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        120,
        100,
    )
}

fn assert_exact_pixels(actual: Vec<u8>, expected: Vec<u8>) {
    assert_eq!(actual.len(), expected.len());
    let differences: Vec<_> = actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .enumerate()
        .filter(|(_, (actual, expected))| actual != expected)
        .collect();
    let first: Vec<_> = differences
        .iter()
        .take(8)
        .map(|(index, colors)| (index % 120, index / 120, colors))
        .collect();
    assert_eq!(differences.len(), 0, "first pixel differences: {first:?}");
}

#[test]
fn html_col_width_reaches_fixed_and_auto_layout_and_exact_paint() {
    for (table_style, attribute, author_width, expected) in [
        (
            "display:table;table-layout:fixed;width:100px;border-spacing:0",
            "40",
            "",
            [40.0, 60.0],
        ),
        (
            "display:table;table-layout:fixed;width:100px;border-spacing:0",
            "40%",
            "",
            [40.0, 60.0],
        ),
        (
            "display:table;table-layout:fixed;width:100px;border-spacing:0",
            "40",
            "width:12px",
            [12.0, 88.0],
        ),
        ("display:table;border-spacing:0", "40", "", [40.0, 10.0]),
    ] {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, table_style);
        let col = doc.append_element(
            Some(table),
            "col",
            Style::default(),
            Some(format!("display:table-column;{author_width}")),
        );
        doc.set_element_attributes(col, vec![("width".into(), attribute.into())]);
        element(&mut doc, table, "display:table-column");
        let row = element(&mut doc, table, "display:table-row");
        let first = element(
            &mut doc,
            row,
            "display:table-cell;height:20px;background:green;vertical-align:top",
        );
        let second = element(
            &mut doc,
            row,
            if table_style.contains("table-layout:fixed") {
                "display:table-cell;height:20px;background:blue;vertical-align:top"
            } else {
                "display:table-cell;width:10px;height:20px;background:blue;vertical-align:top"
            },
        );
        let computed = layout(&mut doc);
        assert_eq!(
            [
                doc.get_node(first).unwrap().unrounded_layout.size.width,
                doc.get_node(second).unwrap().unrounded_layout.size.width
            ],
            expected
        );
        let (mut reference, body) = document();
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:0;top:0;width:{}px;height:20px;background:green",
                expected[0]
            ),
        );
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:{}px;top:0;width:{}px;height:20px;background:blue",
                expected[0], expected[1]
            ),
        );
        let reference_computed = layout(&mut reference);
        assert_exact_pixels(
            raster(scene(&doc, &computed)),
            raster(scene(&reference, &reference_computed)),
        );
    }
}

#[test]
fn anonymous_cells_group_consecutive_block_boxes_under_explicit_and_anonymous_rows() {
    for explicit in [true, false] {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let row = if explicit {
            element(&mut doc, table, "display:table-row")
        } else {
            table
        };
        element(
            &mut doc,
            row,
            "display:block;width:20px;height:10px;background:green",
        );
        element(
            &mut doc,
            row,
            "display:block;width:20px;height:10px;background:blue",
        );
        let authored_cell = element(
            &mut doc,
            row,
            "display:table-cell;width:10px;height:20px;background:red;vertical-align:top",
        );
        let original_children = doc.get_node(row).unwrap().children.clone();
        let original_count = doc.node_count();
        let computed = layout(&mut doc);
        assert_eq!(
            doc.get_node(table).unwrap().unrounded_layout.size.width,
            30.0
        );
        assert_eq!(
            doc.get_node(authored_cell)
                .unwrap()
                .unrounded_layout
                .location
                .x,
            20.0
        );
        assert_eq!(doc.get_node(row).unwrap().children, original_children);
        assert_eq!(doc.node_count(), original_count);
        let (mut reference, body) = document();
        for (left, top, width, height, color) in [
            (0, 0, 20, 10, "green"),
            (0, 10, 20, 10, "blue"),
            (20, 0, 10, 20, "red"),
        ] {
            element(
                &mut reference,
                body,
                &format!(
                    "position:absolute;left:{left}px;top:{top}px;width:{width}px;height:{height}px;background:{color}"
                ),
            );
        }
        let expected = layout(&mut reference);
        assert_exact_pixels(
            raster(scene(&doc, &computed)),
            raster(scene(&reference, &expected)),
        );
    }
}

#[test]
fn anonymous_cell_keeps_inline_text_and_span_in_one_paragraph() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row;color:green");
    doc.append_text(row, "X");
    let span = element(&mut doc, row, "display:inline");
    doc.append_text(span, "Y");
    element(
        &mut doc,
        row,
        "display:table-cell;width:10px;height:20px;background:red;vertical-align:top",
    );
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(table).unwrap().unrounded_layout.size.width,
        30.0
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:0;width:20px;height:10px;background:green",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:20px;top:0;width:10px;height:20px;background:red",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn anonymous_groups_keep_distinct_columns_and_public_text_source_ids() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(
        &mut doc,
        table,
        "display:table-row;color:green;text-indent:0",
    );
    let first = doc.append_text(row, "X");
    let span = element(&mut doc, row, "display:inline");
    let second = doc.append_text(span, "Y");
    element(
        &mut doc,
        row,
        "display:table-cell;width:10px;height:20px;background:red;vertical-align:top",
    );
    let third = doc.append_text(row, "Z");
    let source_children = doc.get_node(row).unwrap().children.clone();
    let source_count = doc.node_count();
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(table).unwrap().unrounded_layout.size.width,
        40.0
    );
    let slices = raikiri_dom::layout_pages(&mut doc, &computed, page()).unwrap();
    doc.project_pages(&computed, page(), &slices, &[]);
    let runs = doc.page_text_runs(&computed, 0);
    let glyphs: Vec<_> = runs
        .iter()
        .map(|run| (run.source, run.text, run.origin))
        .collect();
    assert_eq!(
        glyphs,
        vec![
            (
                raikiri_dom::RunSource::Text(raikiri_traits::NodeId::new(first as u64)),
                "X",
                (0.0, 8.0)
            ),
            (
                raikiri_dom::RunSource::Text(raikiri_traits::NodeId::new(second as u64)),
                "Y",
                (10.0, 8.0)
            ),
            (
                raikiri_dom::RunSource::Text(raikiri_traits::NodeId::new(third as u64)),
                "Z",
                (30.0, 8.0)
            ),
        ]
    );
    assert_eq!(doc.node_count(), source_count);
    assert_eq!(doc.get_node(row).unwrap().children, source_children);
    assert_eq!(doc.parent_of(second), Some(span));
    let events = doc.page_paint_order(&computed, 0, None);
    let text_ids: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            raikiri_dom::PaintEvent::Text(fragment) => Some(fragment.node()),
            _ => None,
        })
        .collect();
    assert_eq!(
        text_ids,
        vec![
            raikiri_traits::NodeId::new(first as u64),
            raikiri_traits::NodeId::new(second as u64),
            raikiri_traits::NodeId::new(third as u64)
        ]
    );
    assert_eq!(events.iter().filter(|event| matches!(event, raikiri_dom::PaintEvent::Box(fragment) if fragment.node() == raikiri_traits::NodeId::new(span as u64))).count(), 1);

    let (mut reference, body) = document();
    for (left, width, height, color) in [
        (0, 20, 10, "green"),
        (20, 10, 20, "red"),
        (30, 10, 10, "green"),
    ] {
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:{left}px;top:0;width:{width}px;height:{height}px;background:{color}"
            ),
        );
    }
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn anonymous_cells_preserve_authored_margin_padding_and_border() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row");
    let block = element(
        &mut doc,
        row,
        "display:block;box-sizing:content-box;width:20px;height:10px;margin:3px 4px;padding:2px;border:1px solid blue;background:green",
    );
    let cell = element(
        &mut doc,
        row,
        "display:table-cell;width:10px;height:22px;background:red;vertical-align:top",
    );
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(table).unwrap().unrounded_layout.size.width,
        44.0
    );
    let box_rect = doc.get_node(block).unwrap().unrounded_layout;
    assert_eq!(
        (
            box_rect.location.x,
            box_rect.location.y,
            box_rect.size.width,
            box_rect.size.height
        ),
        (4.0, 3.0, 26.0, 16.0)
    );
    assert_eq!(
        doc.get_node(cell).unwrap().unrounded_layout.location.x,
        34.0
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:4px;top:3px;box-sizing:content-box;width:20px;height:10px;padding:2px;border:1px solid blue;background:green",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:34px;top:0;width:10px;height:22px;background:red",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn inter_cell_whitespace_and_hidden_content_do_not_make_cells() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row;white-space:pre");
    doc.append_text(row, " \n");
    element(
        &mut doc,
        row,
        "display:table-cell;width:10px;height:10px;background:green;vertical-align:top",
    );
    doc.append_text(row, " \t");
    let hidden = element(&mut doc, row, "display:none");
    doc.append_text(hidden, "unrendered");
    element(
        &mut doc,
        row,
        "display:table-cell;width:20px;height:10px;background:blue;vertical-align:top",
    );
    doc.append_text(row, " \n");
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(table).unwrap().unrounded_layout.size.width,
        30.0
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:0;width:10px;height:10px;background:green",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:10px;top:0;width:20px;height:10px;background:blue",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}
