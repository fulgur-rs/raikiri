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
fn anonymous_block_cells_inherit_direction_and_rebuild() {
    for owner_kind in ["table", "group", "row"] {
        for direction in ["ltr", "rtl"] {
            let (mut doc, body) = document();
            let table = element(
                &mut doc,
                body,
                &format!(
                    "display:table;width:100px;table-layout:fixed;border-spacing:0;direction:{direction}"
                ),
            );
            let owner = match owner_kind {
                "group" => element(&mut doc, table, "display:table-row-group"),
                "row" => element(&mut doc, table, "display:table-row"),
                _ => table,
            };
            let block = element(
                &mut doc,
                owner,
                "display:block;width:20px;height:10px;background:green",
            );
            let left = if direction == "rtl" { 80.0 } else { 0.0 };
            let computed = layout(&mut doc);
            assert_eq!(
                doc.get_node(block).unwrap().unrounded_layout.location.x,
                left
            );
            let (mut reference, body) = document();
            element(
                &mut reference,
                body,
                &format!(
                    "position:absolute;left:{left}px;top:0;width:20px;height:10px;background:green"
                ),
            );
            let expected = layout(&mut reference);
            assert_exact_pixels(
                raster(scene(&doc, &computed)),
                raster(scene(&reference, &expected)),
            );
            let again = layout(&mut doc);
            assert_eq!(
                doc.get_node(block).unwrap().unrounded_layout.location.x,
                left
            );
            assert_exact_pixels(
                raster(scene(&doc, &again)),
                raster(scene(&reference, &expected)),
            );
            let reversed = if direction == "rtl" { "ltr" } else { "rtl" };
            doc.set_element_inline_style(table, Some(format!("display:table;width:100px;table-layout:fixed;border-spacing:0;direction:{reversed}").into()));
            let changed = layout(&mut doc);
            let changed_left = if reversed == "rtl" { 80.0 } else { 0.0 };
            assert_eq!(
                doc.get_node(block).unwrap().unrounded_layout.location.x,
                changed_left
            );
            reference.set_element_inline_style(reference.get_node(body).unwrap().children[0], Some(format!("position:absolute;left:{changed_left}px;top:0;width:20px;height:10px;background:green").into()));
            let expected = layout(&mut reference);
            assert_exact_pixels(
                raster(scene(&doc, &changed)),
                raster(scene(&reference, &expected)),
            );
        }
    }
}

#[test]
fn explicit_cell_block_direction_control() {
    let (mut doc, body) = document();
    let table = element(
        &mut doc,
        body,
        "display:table;width:100px;table-layout:fixed;border-spacing:0;direction:rtl",
    );
    let row = element(&mut doc, table, "display:table-row");
    let cell = element(&mut doc, row, "display:table-cell");
    let block = element(
        &mut doc,
        cell,
        "display:block;width:20px;height:10px;background:green",
    );
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(block).unwrap().unrounded_layout.location.x,
        80.0
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:80px;top:0;width:20px;height:10px;background:green",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn ifc_block_direction_matches_used_margin_constraints() {
    for direction in ["ltr", "rtl"] {
        for (margins, ltr_x, rtl_x) in [
            ("margin-left:3px;margin-right:7px", 3.0, 73.0),
            ("margin-left:auto;margin-right:7px", 73.0, 73.0),
            ("margin-left:3px;margin-right:auto", 3.0, 3.0),
            ("margin-left:auto;margin-right:auto", 40.0, 40.0),
        ] {
            let (mut doc, body) = document();
            let root = element(
                &mut doc,
                body,
                &format!("display:block;width:100px;direction:{direction}"),
            );
            doc.append_text(root, "A");
            let block = element(
                &mut doc,
                root,
                &format!("display:block;width:20px;height:10px;background:green;{margins}"),
            );
            let computed = layout(&mut doc);
            assert!(doc.get_node(root).unwrap().is_ifc_root());
            let left = if direction == "rtl" { rtl_x } else { ltr_x };
            assert_eq!(
                doc.get_node(block).unwrap().unrounded_layout.location,
                taffy::Point { x: left, y: 10.0 }
            );
            let (mut reference, body) = document();
            let text_left = if direction == "rtl" { 90 } else { 0 };
            element(
                &mut reference,
                body,
                &format!(
                    "position:absolute;left:{text_left}px;top:0;width:10px;height:10px;background:black"
                ),
            );
            element(
                &mut reference,
                body,
                &format!(
                    "position:absolute;left:{left}px;top:10px;width:20px;height:10px;background:green"
                ),
            );
            let expected = layout(&mut reference);
            assert_exact_pixels(
                raster(scene(&doc, &computed)),
                raster(scene(&reference, &expected)),
            );
        }
    }
}

#[test]
fn anonymous_paragraph_trace_uses_real_source_owners() {
    use raikiri_paint::{PaintTraceEvent, trace_paint_order};
    for owner_kind in ["table", "group", "row"] {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let owner = match owner_kind {
            "group" => element(&mut doc, table, "display:table-row-group"),
            "row" => element(&mut doc, table, "display:table-row"),
            _ => table,
        };
        doc.append_text(owner, "A");
        let computed = layout(&mut doc);
        let mut budget = raikiri_dom::CounterSnapshotBudget::default();
        let trace = trace_paint_order(&doc, &computed, page(), 0.0, None, &mut budget).unwrap();
        let texts: Vec<_> = trace
            .into_iter()
            .filter_map(|event| match event {
                PaintTraceEvent::Text(owner) => Some(owner),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec![owner]);
        let actual = raster(scene(&doc, &computed));
        assert_eq!(
            actual
                .chunks_exact(4)
                .filter(|pixel| pixel[..3] == [0, 0, 0] && pixel[3] > 0)
                .count(),
            100
        );
    }
}

#[test]
fn anonymous_text_trace_interleaves_generated_and_real_text_with_cells() {
    use raikiri_paint::{PaintTraceEvent, trace_paint_order};
    for generated in [false, true] {
        let (mut doc, body) = document();
        let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
        doc.append_text(sheet, "span::before{content:'A';color:black}");
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let row = element(&mut doc, table, "display:table-row");
        if generated {
            doc.append_element(
                Some(row),
                "span",
                Style::default(),
                Some("display:contents"),
            );
        } else {
            doc.append_text(row, "A");
        }
        let cell = element(
            &mut doc,
            row,
            "display:table-cell;width:10px;height:10px;background:blue;vertical-align:top",
        );
        if generated {
            doc.append_element(
                Some(row),
                "span",
                Style::default(),
                Some("display:contents"),
            );
        } else {
            doc.append_text(row, "B");
        }
        let computed = layout(&mut doc);
        let mut budget = raikiri_dom::CounterSnapshotBudget::default();
        let traced: Vec<_> = trace_paint_order(&doc, &computed, page(), 0.0, None, &mut budget)
            .unwrap()
            .into_iter()
            .filter(|event| match event {
                PaintTraceEvent::Text(_) => true,
                PaintTraceEvent::Box(id) => *id == cell,
                _ => false,
            })
            .collect();
        let expected = vec![
            PaintTraceEvent::Text(row),
            PaintTraceEvent::Box(cell),
            PaintTraceEvent::Text(row),
        ];
        assert_eq!(traced, expected);
        let slices = raikiri_dom::layout_pages(&mut doc, &computed, page()).unwrap();
        doc.project_pages(&computed, page(), &slices, &[]);
        let runs = doc.page_text_runs(&computed, 0);
        let mut projected = Vec::new();
        let mut previous_line_root = None;
        for event in doc.page_paint_order_for_text_runs(&computed, 0, &runs) {
            match event {
                raikiri_dom::PaintEvent::TextLine(line) => {
                    if previous_line_root != Some(line.root) {
                        projected.push(PaintTraceEvent::Text(
                            doc.ifc_source_owner(line.root.0 as usize),
                        ));
                    }
                    previous_line_root = Some(line.root);
                }
                raikiri_dom::PaintEvent::Box(fragment) if fragment.node().0 as usize == cell => {
                    projected.push(PaintTraceEvent::Box(cell));
                    previous_line_root = None;
                }
                _ => {}
            }
        }
        assert_eq!(projected, expected);
        let (mut reference, body) = document();
        for (left, color) in [(0, "black"), (10, "blue"), (20, "black")] {
            element(
                &mut reference,
                body,
                &format!(
                    "position:absolute;left:{left}px;top:0;width:10px;height:10px;background:{color}"
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
fn anonymous_trace_excludes_whitespace_atomic_and_hidden_text() {
    use raikiri_paint::{PaintTraceEvent, trace_paint_order};
    for content in ["whitespace", "atomic", "hidden"] {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let row = element(&mut doc, table, "display:table-row");
        match content {
            "atomic" => {
                element(
                    &mut doc,
                    row,
                    "display:inline-block;width:10px;height:10px;background:blue",
                );
            }
            "hidden" => {
                let hidden = element(&mut doc, row, "display:inline;visibility:hidden");
                doc.append_text(hidden, "A");
            }
            _ => {
                doc.append_text(row, " \t\n");
            }
        }
        let computed = layout(&mut doc);
        let mut budget = raikiri_dom::CounterSnapshotBudget::default();
        let trace = trace_paint_order(&doc, &computed, page(), 0.0, None, &mut budget).unwrap();
        assert!(
            !trace
                .iter()
                .any(|event| matches!(event, PaintTraceEvent::Text(_)))
        );
    }
}

#[test]
fn anonymous_vertical_cells_match_explicit_cells_through_contents_ancestors() {
    for mode in ["vertical-lr", "vertical-rl"] {
        let build = |anonymous: bool| {
            let (mut doc, body) = document();
            let table = element(
                &mut doc,
                body,
                &format!("display:table;writing-mode:{mode};border-spacing:0"),
            );
            let wrapper = element(&mut doc, table, "display:contents");
            let group = element(&mut doc, wrapper, "display:table-row-group");
            let wrapper = element(&mut doc, group, "display:contents");
            for _ in 0..2 {
                let row = element(&mut doc, wrapper, "display:table-row");
                let owner = if anonymous {
                    row
                } else {
                    element(&mut doc, row, "display:table-cell")
                };
                doc.append_text(owner, "A");
                element(
                    &mut doc,
                    row,
                    "display:table-cell;width:10px;height:10px;vertical-align:top",
                );
            }
            (doc, table, group)
        };
        let (mut actual, table, group) = build(true);
        let computed = layout(&mut actual);
        let (mut reference, _, _) = build(false);
        let expected = layout(&mut reference);
        assert_eq!(
            actual.get_node(table).unwrap().unrounded_layout.size,
            reference.get_node(table).unwrap().unrounded_layout.size
        );
        assert_eq!(
            actual.get_node(group).unwrap().unrounded_layout.size.width,
            actual.get_node(table).unwrap().unrounded_layout.size.width
        );
        assert_exact_pixels(
            raster(scene(&actual, &computed)),
            raster(scene(&reference, &expected)),
        );
        let again = layout(&mut actual);
        assert_exact_pixels(
            raster(scene(&actual, &again)),
            raster(scene(&reference, &expected)),
        );
    }
}

#[test]
fn anonymous_inline_atomic_child_retains_its_measured_box_and_ink() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row");
    doc.append_text(row, "A");
    let atomic = element(
        &mut doc,
        row,
        "display:inline-block;width:10px;height:10px;background:blue;vertical-align:top",
    );
    let computed = layout(&mut doc);
    let rect = doc.get_node(atomic).unwrap().unrounded_layout;
    assert_eq!(
        (
            rect.location.x,
            rect.location.y,
            rect.size.width,
            rect.size.height
        ),
        (10.0, 0.0, 10.0, 10.0)
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:0;width:10px;height:10px;background:black",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:10px;top:0;width:10px;height:10px;background:blue",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn anonymous_rows_share_a_rowspan_background_through_contents_groups() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let wrapper = element(&mut doc, table, "display:contents");
    let group = element(
        &mut doc,
        wrapper,
        "display:table-row-group;background:green",
    );
    let wrapper = element(&mut doc, group, "display:contents");
    let first = element(&mut doc, wrapper, "display:table-row");
    doc.append_text(first, "A");
    let spanning = element(
        &mut doc,
        first,
        "display:table-cell;width:10px;height:10px;background:blue;vertical-align:top",
    );
    doc.set_element_attributes(spanning, vec![("rowspan".into(), "2".into())]);
    let second = element(&mut doc, wrapper, "display:table-row");
    doc.append_text(second, "B");
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(table).unwrap().unrounded_layout.size,
        taffy::Size {
            width: 20.0,
            height: 20.0
        }
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:0;width:10px;height:20px;background:black",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:10px;top:0;width:10px;height:20px;background:blue",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn anonymous_block_cells_preserve_placement_with_installed_font_bootstrap() {
    let build = || {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
        (doc, body)
    };
    let (mut doc, body) = build();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let group = element(&mut doc, table, "display:table-row-group");
    let mut children = Vec::new();
    for _ in 0..2 {
        let row = element(&mut doc, group, "display:table-row");
        children.push(element(
            &mut doc,
            row,
            "display:block;width:20px;height:10px;background:green",
        ));
        children.push(element(
            &mut doc,
            row,
            "display:block;width:20px;height:10px;background:blue",
        ));
        element(
            &mut doc,
            row,
            "display:table-cell;width:10px;height:20px;background:red;vertical-align:top",
        );
    }
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(table).unwrap().unrounded_layout.size,
        taffy::Size {
            width: 30.0,
            height: 40.0
        }
    );
    for (index, child) in children.into_iter().enumerate() {
        let child = doc.get_node(child).unwrap();
        assert_eq!(
            (
                child.unrounded_layout.location.x,
                child.unrounded_layout.location.y
            ),
            (0.0, (index % 2) as f32 * 10.0)
        );
    }
    let (mut reference, body) = build();
    for y in [0, 20] {
        element(
            &mut reference,
            body,
            &format!("position:absolute;left:0;top:{y}px;width:20px;height:10px;background:green"),
        );
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:0;top:{}px;width:20px;height:10px;background:blue",
                y + 10
            ),
        );
    }
    element(
        &mut reference,
        body,
        "position:absolute;left:20px;top:0;width:10px;height:40px;background:red",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn anonymous_text_uses_page_content_clip_and_skips_other_page_ink() {
    let (mut doc, body) = document();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(sheet, "@page { margin-left:40px; margin-right:40px }");
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row;white-space:nowrap");
    doc.append_text(row, "AAAAAAAAAA");
    let computed = layout(&mut doc);
    let actual = raster(scene(&doc, &computed));
    assert_eq!(
        actual
            .chunks_exact(4)
            .filter(|pixel| *pixel == [0, 0, 0, 255])
            .count(),
        400
    );
    let mut later = Scene::new();
    raikiri_paint::paint_single_page_with_origin(
        &mut later,
        &doc,
        &computed,
        page(),
        100.0,
        &mut Default::default(),
    )
    .unwrap();
    assert_eq!(
        raster(later)
            .chunks_exact(4)
            .filter(|pixel| *pixel == [0, 0, 0, 255])
            .count(),
        0
    );
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

#[test]
fn anonymous_content_keeps_row_group_backgrounds_and_source_box_geometry() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let group = element(&mut doc, table, "display:table-row-group;background:cyan");
    let mut rows = Vec::new();
    for (text, color) in [("A", "red"), ("B", "transparent")] {
        let row = element(
            &mut doc,
            group,
            &format!("display:table-row;background:{color}"),
        );
        doc.append_text(row, text);
        element(
            &mut doc,
            row,
            "display:table-cell;width:30px;height:20px;vertical-align:top",
        );
        rows.push(row);
    }
    let source_count = doc.node_count();
    let computed = layout(&mut doc);
    assert_eq!(doc.node_count(), source_count);
    for (index, &row) in rows.iter().enumerate() {
        let box_layout = doc.get_node(row).unwrap().unrounded_layout;
        assert_eq!(
            (box_layout.location.x, box_layout.location.y),
            (0.0, index as f32 * 20.0)
        );
        assert_eq!(
            (box_layout.size.width, box_layout.size.height),
            (40.0, 20.0)
        );
    }
    let group_box = doc.get_node(group).unwrap().unrounded_layout;
    assert_eq!((group_box.size.width, group_box.size.height), (40.0, 40.0));
    let (mut reference, body) = document();
    for (text, color) in [("A", "red"), ("B", "cyan")] {
        let row = element(
            &mut reference,
            body,
            &format!("display:block;background:{color};width:40px;height:20px"),
        );
        reference.append_text(row, text);
    }
    let reference_computed = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &reference_computed)),
    );
    let computed_again = layout(&mut doc);
    assert_eq!(doc.node_count(), source_count);
    assert_exact_pixels(
        raster(scene(&doc, &computed_again)),
        raster(scene(&reference, &reference_computed)),
    );
}

#[test]
fn anonymous_cells_use_initial_vertical_alignment_without_copying_row_style() {
    let build = |anonymous: bool| {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let row = element(&mut doc, table, "display:table-row;vertical-align:top");
        if anonymous {
            doc.append_text(row, "A");
        } else {
            let cell = element(&mut doc, row, "display:table-cell");
            doc.append_text(cell, "A");
        }
        let cell = element(
            &mut doc,
            row,
            "display:table-cell;font-size:20px;line-height:20px;vertical-align:baseline",
        );
        doc.append_text(cell, "B");
        let computed = layout(&mut doc);
        (doc, computed)
    };
    let (actual, computed) = build(true);
    let (reference, reference_computed) = build(false);
    assert_exact_pixels(
        raster(scene(&actual, &computed)),
        raster(scene(&reference, &reference_computed)),
    );
}

#[test]
fn anonymous_row_backgrounds_leave_separated_border_gaps_on_the_table() {
    let (mut doc, body) = document();
    let table = element(
        &mut doc,
        body,
        "display:table;border-spacing:2px;background:yellow",
    );
    let group = element(&mut doc, table, "display:table-row-group;background:cyan");
    for (text, color) in [("A", "red"), ("B", "transparent")] {
        let row = element(
            &mut doc,
            group,
            &format!("display:table-row;background:{color}"),
        );
        doc.append_text(row, text);
        element(
            &mut doc,
            row,
            "display:table-cell;width:30px;height:20px;vertical-align:top",
        );
    }
    let computed = layout(&mut doc);
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "display:block;width:46px;height:46px;background:yellow",
    );
    for (text, color, top) in [("A", "red", 2), ("B", "cyan", 24)] {
        let first = element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:2px;top:{top}px;width:10px;height:20px;background:{color}"
            ),
        );
        reference.append_text(first, text);
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:14px;top:{top}px;width:30px;height:20px;background:{color}"
            ),
        );
    }
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
    let slices = raikiri_dom::layout_pages(&mut doc, &computed, page()).unwrap();
    doc.project_pages(&computed, page(), &slices, &[]);
    let events = doc.page_paint_order(&computed, 0, None);
    let mut group_clips = Vec::new();
    for triple in events.windows(3) {
        if let [
            raikiri_dom::PaintEvent::PushClip(clip, raikiri_dom::ClipKind::TableCell),
            raikiri_dom::PaintEvent::Box(fragment),
            raikiri_dom::PaintEvent::PopClip,
        ] = triple
            && fragment.node() == raikiri_traits::NodeId::new(group as u64)
        {
            group_clips.push((clip.rect.x, clip.rect.y, clip.rect.width, clip.rect.height));
        }
    }
    assert_eq!(
        group_clips,
        vec![
            (2.0, 2.0, 10.0, 20.0),
            (14.0, 2.0, 30.0, 20.0),
            (2.0, 24.0, 10.0, 20.0),
            (14.0, 24.0, 30.0, 20.0)
        ]
    );
}

#[test]
fn anonymous_cell_text_runs_keep_ancestor_decoration_context() {
    let (mut doc, body) = document();
    let table = element(
        &mut doc,
        body,
        "display:table;border-spacing:0;text-decoration:underline red",
    );
    let row = element(&mut doc, table, "display:table-row");
    let text = doc.append_text(row, "A");
    element(
        &mut doc,
        row,
        "display:table-cell;width:10px;height:10px;vertical-align:top",
    );
    let computed = layout(&mut doc);
    let slices = raikiri_dom::layout_pages(&mut doc, &computed, page()).unwrap();
    doc.project_pages(&computed, page(), &slices, &[]);
    let runs = doc.page_text_runs(&computed, 0);
    let run = runs
        .iter()
        .find(|run| {
            run.source == raikiri_dom::RunSource::Text(raikiri_traits::NodeId::new(text as u64))
        })
        .unwrap();
    assert_eq!(run.decorations.len(), 1);
    assert_eq!(
        run.decorations[0].color,
        raikiri_style::property::CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        (run.decorations[0].x_start, run.decorations[0].x_end),
        (0.0, 10.0)
    );
}

#[test]
fn anonymous_groups_rebuild_after_source_child_reordering() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row");
    let block = element(
        &mut doc,
        row,
        "display:block;width:20px;height:10px;background:green",
    );
    let cell = element(
        &mut doc,
        row,
        "display:table-cell;width:10px;height:10px;background:blue;vertical-align:top",
    );
    let count = doc.node_count();
    for reordered in [false, true] {
        if reordered {
            assert_eq!(doc.detach_from_parent(block), Some(row));
            doc.attach_child(row, block);
        }
        let computed = layout(&mut doc);
        assert_eq!(doc.node_count(), count);
        assert_eq!(doc.parent_of(block), Some(row));
        assert_eq!(
            doc.get_node(row).unwrap().children,
            if reordered {
                vec![cell, block]
            } else {
                vec![block, cell]
            }
        );
        let (mut reference, body) = document();
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:{}px;top:0;width:20px;height:10px;background:green",
                if reordered { 10 } else { 0 }
            ),
        );
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:{}px;top:0;width:10px;height:10px;background:blue",
                if reordered { 0 } else { 20 }
            ),
        );
        let expected = layout(&mut reference);
        assert_exact_pixels(
            raster(scene(&doc, &computed)),
            raster(scene(&reference, &expected)),
        );
    }
}

#[test]
fn anonymous_cell_preserves_out_of_flow_descendant_paint() {
    let build = |anonymous: bool| {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let row = element(&mut doc, table, "display:table-row");
        let parent = if anonymous {
            row
        } else {
            element(&mut doc, row, "display:table-cell")
        };
        let block = element(
            &mut doc,
            parent,
            "display:block;position:relative;width:20px;height:10px;background:green",
        );
        element(
            &mut doc,
            block,
            "position:absolute;left:5px;top:2px;width:5px;height:4px;background:red",
        );
        element(
            &mut doc,
            row,
            "display:table-cell;width:10px;height:10px;background:blue;vertical-align:top",
        );
        let computed = layout(&mut doc);
        (doc, computed)
    };
    let (actual, computed) = build(true);
    let (reference, expected) = build(false);
    assert_exact_pixels(
        raster(scene(&actual, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn anonymous_cell_keeps_generated_text_of_its_inline_source_child() {
    let build = |anonymous: bool| {
        let (mut doc, body) = document();
        let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
        doc.append_text(
            sheet,
            "span::before { content:'X';color:red } span::after { content:'Y';color:blue }",
        );
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let row = element(&mut doc, table, "display:table-row");
        let parent = if anonymous {
            row
        } else {
            element(&mut doc, row, "display:table-cell")
        };
        let span = doc.append_element(
            Some(parent),
            "span",
            Style::default(),
            Some("display:inline"),
        );
        doc.append_text(span, "A");
        element(
            &mut doc,
            row,
            "display:table-cell;width:10px;height:10px;background:green;vertical-align:top",
        );
        let computed = layout(&mut doc);
        (doc, computed)
    };
    let (actual, computed) = build(true);
    let (reference, expected) = build(false);
    assert_exact_pixels(
        raster(scene(&actual, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn removing_anonymous_content_restores_fresh_explicit_row_geometry() {
    for collapse in ["separate", "collapse"] {
        let build = |anonymous: bool| {
            let (mut doc, body) = document();
            let table = element(
                &mut doc,
                body,
                &format!("display:table;border-spacing:0;border-collapse:{collapse}"),
            );
            let group = element(&mut doc, table, "display:table-row-group;background:cyan");
            let mut texts = Vec::new();
            let mut rows = Vec::new();
            for _ in 0..2 {
                let row = element(&mut doc, group, "display:table-row");
                if anonymous {
                    texts.push(doc.append_text(row, "A"));
                }
                let cell = element(
                    &mut doc,
                    row,
                    "display:table-cell;width:10px;height:10px;background:red;vertical-align:top",
                );
                doc.append_text(cell, "B");
                rows.push(row);
            }
            (doc, texts, rows, group)
        };
        let (mut actual, texts, rows, group) = build(true);
        layout(&mut actual);
        for text in texts {
            actual.detach_from_parent(text);
        }
        let computed = layout(&mut actual);
        let (mut reference, _, reference_rows, reference_group) = build(false);
        let expected = layout(&mut reference);
        for (actual_row, reference_row) in rows.into_iter().zip(reference_rows) {
            assert_eq!(
                actual.get_node(actual_row).unwrap().unrounded_layout,
                reference.get_node(reference_row).unwrap().unrounded_layout
            );
        }
        assert_eq!(
            actual.get_node(group).unwrap().unrounded_layout,
            reference
                .get_node(reference_group)
                .unwrap()
                .unrounded_layout
        );
        assert_exact_pixels(
            raster(scene(&actual, &computed)),
            raster(scene(&reference, &expected)),
        );
        let slices = raikiri_dom::layout_pages(&mut actual, &computed, page()).unwrap();
        actual.project_pages(&computed, page(), &slices, &[]);
        let slices = raikiri_dom::layout_pages(&mut reference, &expected, page()).unwrap();
        reference.project_pages(&expected, page(), &slices, &[]);
        let actual_runs: Vec<_> = actual
            .page_text_runs(&computed, 0)
            .into_iter()
            .map(|run| (run.text, run.origin))
            .collect();
        let expected_runs: Vec<_> = reference
            .page_text_runs(&expected, 0)
            .into_iter()
            .map(|run| (run.text, run.origin))
            .collect();
        assert_eq!(actual_runs, expected_runs);
    }
}

#[test]
fn anonymous_cells_share_css22_empty_row_classification_after_main_integration() {
    for (row_visibility, child_style, policy, expected_row_height, expected_table_height) in [
        (
            "hidden",
            "visibility:visible",
            "empty-cells:hide",
            0.0,
            30.0,
        ),
        (
            "visible",
            "visibility:hidden",
            "empty-cells:hide",
            0.0,
            30.0,
        ),
        (
            "visible",
            "position:absolute;visibility:visible",
            "empty-cells:hide",
            0.0,
            30.0,
        ),
        (
            "visible",
            "visibility:visible",
            "empty-cells:hide",
            20.0,
            55.0,
        ),
        (
            "hidden",
            "visibility:visible",
            "empty-cells:show",
            20.0,
            55.0,
        ),
        (
            "hidden",
            "visibility:visible",
            "empty-cells:hide;border-collapse:collapse",
            20.0,
            40.0,
        ),
    ] {
        let build = |anonymous: bool| {
            let (mut doc, body) = document();
            let table = element(
                &mut doc,
                body,
                &format!("display:table;width:40px;border-spacing:0 5px;{policy}"),
            );
            let row = element(
                &mut doc,
                table,
                &format!("display:table-row;visibility:{row_visibility}"),
            );
            let owner = if anonymous {
                row
            } else {
                element(&mut doc, row, "display:table-cell")
            };
            element(
                &mut doc,
                owner,
                &format!("display:block;width:40px;height:20px;background:blue;{child_style}"),
            );
            let following_row = element(&mut doc, table, "display:table-row");
            let following_cell = element(
                &mut doc,
                following_row,
                "display:table-cell;vertical-align:top",
            );
            element(
                &mut doc,
                following_cell,
                "display:block;width:40px;height:20px;background:green",
            );
            let computed = layout(&mut doc);
            (doc, computed, table, row, following_row)
        };
        let (mut actual, computed, table, row, following_row) = build(true);
        let (reference, reference_computed, _, _, _) = build(false);
        assert_eq!(
            actual.get_node(table).unwrap().unrounded_layout.size.height,
            expected_table_height
        );
        assert_eq!(
            actual.get_node(row).unwrap().unrounded_layout.size.height,
            expected_row_height
        );
        let virtual_cell = actual.anonymous_table_cells(row).next().unwrap().1;
        assert_eq!(
            virtual_cell.unrounded_layout.size.height,
            expected_row_height
        );
        let expected_following_y = if policy.contains("collapse") {
            20.0
        } else {
            5.0 + expected_row_height + if expected_row_height > 0.0 { 5.0 } else { 0.0 }
        };
        assert_eq!(
            actual
                .get_node(following_row)
                .unwrap()
                .unrounded_layout
                .location
                .y,
            expected_following_y
        );
        assert_exact_pixels(
            raster(scene(&actual, &computed)),
            raster(scene(&reference, &reference_computed)),
        );
        let (mut rectangles, body) = document();
        if expected_row_height > 0.0 {
            let top = if policy.contains("collapse") {
                0.0
            } else {
                5.0
            };
            element(
                &mut rectangles,
                body,
                &format!(
                    "display:block;position:absolute;left:0;top:{top}px;width:40px;height:20px;background:blue"
                ),
            );
        }
        element(
            &mut rectangles,
            body,
            &format!(
                "display:block;position:absolute;left:0;top:{expected_following_y}px;width:40px;height:20px;background:green"
            ),
        );
        let rectangles_computed = layout(&mut rectangles);
        assert_exact_pixels(
            raster(scene(&actual, &computed)),
            raster(scene(&rectangles, &rectangles_computed)),
        );
        let computed_again = layout(&mut actual);
        assert_eq!(
            actual.get_node(table).unwrap().unrounded_layout.size.height,
            expected_table_height
        );
        assert_exact_pixels(
            raster(scene(&actual, &computed_again)),
            raster(scene(&reference, &reference_computed)),
        );
    }
}

#[test]
fn anonymous_empty_cell_flags_rebuild_across_policy_and_content_changes() {
    let (mut actual, body) = document();
    let table = element(
        &mut actual,
        body,
        "display:table;width:40px;border-spacing:0 5px;empty-cells:hide",
    );
    let row = element(&mut actual, table, "display:table-row");
    let child = element(
        &mut actual,
        row,
        "display:block;width:40px;height:20px;visibility:hidden",
    );
    for (policy, child_visibility, expected) in [
        ("hide", "hidden", 5.0),
        ("show", "hidden", 30.0),
        ("hide", "visible", 30.0),
        ("hide", "hidden", 5.0),
    ] {
        actual.set_element_inline_style(
            table,
            Some(
                format!("display:table;width:40px;border-spacing:0 5px;empty-cells:{policy}")
                    .into(),
            ),
        );
        actual.set_element_inline_style(
            child,
            Some(
                format!("display:block;width:40px;height:20px;visibility:{child_visibility}")
                    .into(),
            ),
        );
        let count = actual.node_count();
        let computed = layout(&mut actual);
        assert_eq!(actual.node_count(), count);
        assert_eq!(
            actual.get_node(table).unwrap().unrounded_layout.size.height,
            expected
        );
        assert_eq!(actual.anonymous_table_cells(row).count(), 1);
        let (mut reference, body) = document();
        element(&mut reference, body, "display:block;width:40px;height:5px");
        let expected_computed = layout(&mut reference);
        assert_exact_pixels(
            raster(scene(&actual, &computed)),
            raster(scene(&reference, &expected_computed)),
        );
    }
    let cell = element(
        &mut actual,
        row,
        "display:table-cell;height:20px;empty-cells:show",
    );
    let computed = layout(&mut actual);
    assert_eq!(
        actual.get_node(table).unwrap().unrounded_layout.size.height,
        30.0
    );
    assert_eq!(
        actual.get_node(cell).unwrap().unrounded_layout.size.height,
        20.0
    );
    assert_eq!(
        actual.get_node(row).unwrap().unrounded_layout.size.height,
        20.0
    );
    assert_eq!(
        actual
            .anonymous_table_cells(row)
            .next()
            .unwrap()
            .1
            .unrounded_layout
            .size
            .height,
        20.0
    );
    let (mut reference, body) = document();
    element(&mut reference, body, "display:block;width:40px;height:30px");
    let expected_computed = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&actual, &computed)),
        raster(scene(&reference, &expected_computed)),
    );
}

#[test]
fn review_anonymous_paragraphs_interleave_with_explicit_cell_events() {
    for relative in [false, true] {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let row = element(&mut doc, table, "display:table-row;color:green");
        let first = doc.append_text(row, "A");
        let cell = element(
            &mut doc,
            row,
            if relative {
                "display:table-cell;color:red;position:relative"
            } else {
                "display:table-cell;color:red"
            },
        );
        let middle = doc.append_text(cell, "A");
        let span = element(&mut doc, row, "display:inline;color:blue;margin-left:-10px");
        let last = doc.append_text(span, "A");
        let computed = layout(&mut doc);
        let slices = raikiri_dom::layout_pages(&mut doc, &computed, page()).unwrap();
        doc.project_pages(&computed, page(), &slices, &[]);
        let events = doc.page_paint_order(&computed, 0, None);
        let owners: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                raikiri_dom::PaintEvent::Text(fragment) => Some(fragment.node()),
                _ => None,
            })
            .collect();
        let (mut reference, body) = document();
        for (left, color) in [(0, "green"), (10, if relative { "red" } else { "blue" })] {
            element(
                &mut reference,
                body,
                &format!(
                    "position:absolute;left:{left}px;top:0;width:10px;height:10px;background:{color}"
                ),
            );
        }
        let expected = layout(&mut reference);
        assert_exact_pixels(
            raster(scene(&doc, &computed)),
            raster(scene(&reference, &expected)),
        );
        assert_eq!(
            owners,
            (if relative {
                vec![first, last, middle]
            } else {
                vec![first, middle, last]
            })
            .into_iter()
            .map(|id| raikiri_traits::NodeId::new(id as u64))
            .collect::<Vec<_>>()
        );
    }
}

#[test]
fn review_anonymous_named_page_probe_ignores_unprojected_hidden_text() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0;page:good");
    let row = element(&mut doc, table, "display:table-row");
    let span = element(&mut doc, row, "display:inline");
    let hidden = element(&mut doc, span, "display:none;page:wrong");
    let hidden_text = doc.append_text(hidden, "H");
    let visible = doc.append_text(span, "A");
    let computed = layout(&mut doc);
    assert!(matches!(
        computed.page_values[hidden],
        raikiri_style::property::PageValue::Named(_)
    ));
    assert!(matches!(
        computed.page_values[hidden_text],
        raikiri_style::property::PageValue::Auto
    ));
    assert!(matches!(
        computed.page_values[visible],
        raikiri_style::property::PageValue::Auto
    ));
    let key = doc.anonymous_table_cells(row).next().unwrap().0;
    assert_eq!(
        doc.ifc_text_lines_by_node(key)
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![visible]
    );
    let mut actual = Scene::new();
    raikiri_paint::paint_single_page_with_origin_and_page_context_named(
        &mut actual,
        &doc,
        &computed,
        page(),
        0.0,
        0,
        1,
        false,
        None,
        Some("good"),
        &mut Default::default(),
    )
    .unwrap();
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:0;width:10px;height:10px;background:black",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(raster(actual), raster(scene(&reference, &expected)));
}

#[test]
fn review_contents_pseudos_survive_anonymous_cell_projection() {
    for real_content in [false, true] {
        let (mut doc, body) = document();
        let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
        doc.append_text(
            sheet,
            "span::before{content:'X';color:red}span::after{content:'Y';color:blue}",
        );
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let row = element(&mut doc, table, "display:table-row");
        let span = doc.append_element(
            Some(row),
            "span",
            Style::default(),
            Some("display:contents"),
        );
        if real_content {
            doc.append_text(span, "A");
        }
        element(
            &mut doc,
            row,
            "display:table-cell;width:10px;height:10px;background:green;vertical-align:top",
        );
        let computed = layout(&mut doc);
        let (mut reference, body) = document();
        let colors: &[&str] = if real_content {
            &["red", "black", "blue", "green"]
        } else {
            &["red", "blue", "green"]
        };
        for (index, color) in colors.iter().enumerate() {
            element(
                &mut reference,
                body,
                &format!(
                    "position:absolute;left:{}px;top:0;width:10px;height:10px;background:{color}",
                    index * 10
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
fn review_contents_generated_cells_stay_around_structural_cells_in_source_order() {
    for owner_display in ["table", "table-row-group", "table-row"] {
        let (mut doc, body) = document();
        let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
        doc.append_text(
            sheet,
            "span::before{content:'X';color:red}span::after{content:'Y';color:blue}",
        );
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let owner = if owner_display == "table" {
            table
        } else {
            element(&mut doc, table, &format!("display:{owner_display}"))
        };
        let span = doc.append_element(
            Some(owner),
            "span",
            Style::default(),
            Some("display:contents"),
        );
        let nested = element(&mut doc, span, "display:contents");
        element(
            &mut doc,
            nested,
            "display:table-cell;width:10px;height:10px;background:green;vertical-align:top",
        );
        let original_children = doc.get_node(owner).unwrap().children.clone();
        let count = doc.node_count();
        let computed = layout(&mut doc);
        assert_eq!(doc.node_count(), count);
        assert_eq!(doc.get_node(owner).unwrap().children, original_children);
        assert_eq!(doc.anonymous_table_cells(owner).count(), 2);
        let (mut reference, body) = document();
        for (index, color) in ["red", "green", "blue"].iter().enumerate() {
            element(
                &mut reference,
                body,
                &format!(
                    "position:absolute;left:{}px;top:0;width:10px;height:10px;background:{color}",
                    index * 10
                ),
            );
        }
        let expected = layout(&mut reference);
        assert_exact_pixels(
            raster(scene(&doc, &computed)),
            raster(scene(&reference, &expected)),
        );
        let again = layout(&mut doc);
        assert_exact_pixels(
            raster(scene(&doc, &again)),
            raster(scene(&reference, &expected)),
        );
    }
}

#[test]
fn review_contents_pseudo_runs_keep_real_owners_and_utf8_sources() {
    let (mut doc, body) = document();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(
        sheet,
        "span::before{content:'X';color:red}span::after{content:'Y';color:blue}",
    );
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row");
    let span = doc.append_element(
        Some(row),
        "span",
        Style::default(),
        Some("display:contents"),
    );
    let text = doc.append_text(span, "Ω");
    let count = doc.node_count();
    let computed = layout(&mut doc);
    let slices = raikiri_dom::layout_pages(&mut doc, &computed, page()).unwrap();
    doc.project_pages(&computed, page(), &slices, &[]);
    let runs = doc.page_text_runs(&computed, 0);
    assert_eq!(doc.node_count(), count);
    assert_eq!(doc.parent_of(text), Some(span));
    assert_eq!(
        runs.iter().map(|run| run.text).collect::<Vec<_>>(),
        vec!["X", "Ω", "Y"]
    );
    assert_eq!(
        runs.iter().map(|run| run.source).collect::<Vec<_>>(),
        vec![
            raikiri_dom::RunSource::Generated(
                raikiri_traits::NodeId::new(span as u64),
                raikiri_dom::GeneratedKind::Before
            ),
            raikiri_dom::RunSource::Text(raikiri_traits::NodeId::new(text as u64)),
            raikiri_dom::RunSource::Generated(
                raikiri_traits::NodeId::new(span as u64),
                raikiri_dom::GeneratedKind::After
            ),
        ]
    );
    let (mut reference, body) = document();
    for (index, color) in ["red", "black", "blue"].iter().enumerate() {
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:{}px;top:0;width:10px;height:10px;background:{color}",
                index * 10
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
fn review_contents_pseudo_lines_interleave_with_structural_cell_events() {
    let (mut doc, body) = document();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(sheet, "span::before{content:'X'}span::after{content:'Y'}");
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row");
    let span = doc.append_element(
        Some(row),
        "span",
        Style::default(),
        Some("display:contents"),
    );
    let cell = element(&mut doc, span, "display:table-cell");
    doc.append_text(cell, "A");
    let computed = layout(&mut doc);
    let keys: Vec<_> = doc.anonymous_table_cells(row).map(|(key, _)| key).collect();
    let slices = raikiri_dom::layout_pages(&mut doc, &computed, page()).unwrap();
    doc.project_pages(&computed, page(), &slices, &[]);
    let runs = doc.page_text_runs(&computed, 0);
    let mut texts = runs.iter().map(|run| run.text).collect::<Vec<_>>();
    texts.sort_unstable();
    assert_eq!(texts, vec!["A", "X", "Y"]);
    let roots: Vec<_> = doc
        .page_paint_order_for_text_runs(&computed, 0, &runs)
        .iter()
        .filter_map(|event| match event {
            raikiri_dom::PaintEvent::TextLine(line) => Some(line.root),
            _ => None,
        })
        .collect();
    assert_eq!(
        roots,
        vec![keys[0], cell, keys[1]]
            .into_iter()
            .map(|id| raikiri_traits::NodeId::new(id as u64))
            .collect::<Vec<_>>()
    );
}

#[test]
fn review_contents_pseudos_remain_outside_a_structural_row() {
    let (mut doc, body) = document();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(
        sheet,
        "span::before{content:'X';color:red}span::after{content:'Y';color:blue}",
    );
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let span = doc.append_element(
        Some(table),
        "span",
        Style::default(),
        Some("display:contents"),
    );
    let row = element(&mut doc, span, "display:table-row");
    let cell = element(&mut doc, row, "display:table-cell;color:green");
    doc.append_text(cell, "A");
    let source_children = doc.get_node(span).unwrap().children.clone();
    for _ in 0..2 {
        let computed = layout(&mut doc);
        assert_eq!(doc.get_node(span).unwrap().children, source_children);
        assert_eq!(doc.parent_of(row), Some(span));
        assert_eq!(doc.anonymous_table_cells(table).count(), 2);
        assert_eq!(doc.get_node(row).unwrap().unrounded_layout.location.y, 10.0);
        let (mut reference, body) = document();
        for (index, color) in ["red", "green", "blue"].iter().enumerate() {
            element(
                &mut reference,
                body,
                &format!(
                    "position:absolute;left:0;top:{}px;width:10px;height:10px;background:{color}",
                    index * 10
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
fn review_contents_generated_values_share_counters_and_keep_empty_negatives() {
    for declaration in [
        "content:attr(data-label)",
        "content:counter(sample)",
        "content:open-quote",
    ] {
        let (mut doc, body) = document();
        let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
        doc.append_text(sheet, format!(
            "span{{counter-reset:sample 1;quotes:'X' 'Y'}}span::before{{{declaration};color:red}}"
        ));
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        let span = doc.append_element(
            Some(table),
            "span",
            Style::default(),
            Some("display:contents"),
        );
        doc.set_element_attribute(span, "data-label", "X").unwrap();
        let computed = layout(&mut doc);
        let (mut reference, body) = document();
        element(
            &mut reference,
            body,
            "position:absolute;left:0;top:0;width:10px;height:10px;background:red",
        );
        let expected = layout(&mut reference);
        assert_exact_pixels(
            raster(scene(&doc, &computed)),
            raster(scene(&reference, &expected)),
        );
    }
    for (pseudo_style, overlay) in [
        ("content:none", false),
        ("content:''", false),
        ("content:'X';display:none", false),
        ("content:'X';position:absolute", true),
    ] {
        let (mut doc, body) = document();
        let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
        doc.append_text(sheet, format!("span::before{{{pseudo_style}}}"));
        let table = element(&mut doc, body, "display:table;border-spacing:0");
        doc.append_element(
            Some(table),
            "span",
            Style::default(),
            Some("display:contents"),
        );
        let computed = layout(&mut doc);
        if overlay {
            assert_eq!(doc.anonymous_table_cells(table).count(), 0);
        }
        let (mut reference, body) = document();
        if overlay {
            // Out-of-flow generated text keeps the existing overlay path;
            // it must not become an in-flow anonymous cell.
            element(
                &mut reference,
                body,
                "position:absolute;left:0;top:0;width:10px;height:10px;background:black",
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
fn review_contents_generated_cells_survive_cancelled_pagination() {
    let (mut doc, body) = document();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(
        sheet,
        "span::before{content:'X';color:red}span::after{content:'Y';color:blue}",
    );
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let span = doc.append_element(
        Some(table),
        "span",
        Style::default(),
        Some("display:contents"),
    );
    let count = doc.node_count();
    let computed = layout(&mut doc);
    let abort = || true;
    let control = raikiri_dom::PageLayoutControl::new(Some(10)).with_abort_check(&abort);
    assert!(matches!(
        raikiri_dom::layout_pages_with_page_geometry_and_control(
            &mut doc,
            &computed,
            page(),
            &[],
            &[],
            &control
        ),
        Err(raikiri_traits::LayoutError::Aborted)
    ));
    let computed = layout(&mut doc);
    assert_eq!(doc.node_count(), count);
    assert_eq!(doc.parent_of(span), Some(table));
    assert!(doc.get_node(span).unwrap().children.is_empty());
    let (mut reference, body) = document();
    for (index, color) in ["red", "blue"].iter().enumerate() {
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:{}px;top:0;width:10px;height:10px;background:{color}",
                index * 10
            ),
        );
    }
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}
