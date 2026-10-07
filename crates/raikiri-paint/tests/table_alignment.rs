//! Literal geometry and exact ink for the native table layout.

use anyrender::{PaintScene, Scene, recording::RenderCommand};
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
fn cell_keywords_align_content_without_moving_cell_backgrounds() {
    for (align, baseline, ink_top) in [("top", 11.0, 3), ("middle", 26.0, 18), ("bottom", 41.0, 33)]
    {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0;width:40px");
        let row = element(&mut doc, table, "display:table-row;height:46px");
        let cell = element(
            &mut doc,
            row,
            &format!(
                "display:table-cell;width:34px;padding:2px;border:1px solid black;background:blue;color:green;vertical-align:{align}"
            ),
        );
        doc.append_text(cell, "X");
        let computed = layout(&mut doc);
        assert_eq!(
            doc.get_node(cell).unwrap().unrounded_layout.size.height,
            46.0
        );
        let actual = scene(&doc, &computed);
        let baselines: Vec<_> = actual
            .commands
            .iter()
            .filter_map(|command| match command {
                RenderCommand::GlyphRun(run) => {
                    Some(run.transform.as_coeffs()[5] + f64::from(run.glyphs[0].y))
                }
                _ => None,
            })
            .collect();
        assert_eq!(baselines, [baseline], "{align}");
        let (mut reference, body) = document();
        element(
            &mut reference,
            body,
            "position:absolute;left:0;top:0;width:40px;height:46px;background:black",
        );
        element(
            &mut reference,
            body,
            "position:absolute;left:1px;top:1px;width:38px;height:44px;background:blue",
        );
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:3px;top:{ink_top}px;width:10px;height:10px;background:green"
            ),
        );
        let expected = layout(&mut reference);
        assert_eq!(
            raster(actual),
            raster(scene(&reference, &expected)),
            "{align}"
        );
    }
}

#[test]
fn baseline_cells_share_the_first_line_baseline_and_grow_row_height() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row");
    let small = element(
        &mut doc,
        row,
        "display:table-cell;font-size:10px;line-height:10px;vertical-align:baseline",
    );
    doc.append_text(small, "X");
    let large = element(
        &mut doc,
        row,
        "display:table-cell;font-size:20px;line-height:20px;vertical-align:baseline",
    );
    doc.append_text(large, "X");
    let computed = layout(&mut doc);
    let baselines: Vec<_> = scene(&doc, &computed)
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(run) => {
                Some(run.transform.as_coeffs()[5] + f64::from(run.glyphs[0].y))
            }
            _ => None,
        })
        .collect();
    assert_eq!(baselines, [16.0, 16.0]);
    assert_eq!(
        doc.get_node(small).unwrap().unrounded_layout.size.height,
        20.0
    );
}

#[test]
fn captions_are_outside_the_grid_and_reserve_normal_flow_space() {
    for (side, cell_top, caption_top) in [("top", 20.0, 0.0), ("bottom", 0.0, 10.0)] {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0;width:40px");
        let caption = element(
            &mut doc,
            table,
            &format!(
                "display:table-caption;caption-side:{side};width:40px;height:20px;background:blue"
            ),
        );
        let row = element(&mut doc, table, "display:table-row");
        let cell = element(
            &mut doc,
            row,
            "display:table-cell;height:10px;width:40px;background:green",
        );
        let after = element(&mut doc, body, "width:40px;height:10px;background:red");
        let computed = layout(&mut doc);
        assert_eq!(
            doc.get_node(cell).unwrap().unrounded_layout.location.y,
            cell_top,
            "{side}"
        );
        assert_eq!(
            doc.get_node(caption).unwrap().unrounded_layout.location.y,
            caption_top,
            "{side}"
        );
        assert_eq!(
            doc.get_node(after).unwrap().unrounded_layout.location.y,
            30.0,
            "{side}"
        );
        let (mut reference, body) = document();
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:0;top:{cell_top}px;width:40px;height:10px;background:green"
            ),
        );
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:0;top:{caption_top}px;width:40px;height:20px;background:blue"
            ),
        );
        element(
            &mut reference,
            body,
            "position:absolute;left:0;top:30px;width:40px;height:10px;background:red",
        );
        let expected = layout(&mut reference);
        assert_eq!(
            raster(scene(&doc, &computed)),
            raster(scene(&reference, &expected)),
            "{side}"
        );
    }
}

#[test]
fn empty_cells_hide_only_separated_empty_backgrounds_and_borders() {
    for (collapse, contents, hidden) in [
        ("separate", "", true),
        ("separate", " \t\n", true),
        ("separate", "X", false),
        ("collapse", "", false),
    ] {
        let (mut doc, body) = document();
        let table = element(
            &mut doc,
            body,
            &format!(
                "display:table;border-spacing:0;width:20px;border-collapse:{collapse};background:green"
            ),
        );
        let row = element(&mut doc, table, "display:table-row;height:20px");
        let cell = element(
            &mut doc,
            row,
            "display:table-cell;empty-cells:hide;background:blue;border:2px solid black;color:blue;width:16px;vertical-align:top",
        );
        if !contents.is_empty() {
            doc.append_text(cell, contents);
        }
        let computed = layout(&mut doc);
        let pixels = raster(scene(&doc, &computed));
        assert_eq!(
            &pixels[(5 * 120 + 5) * 4..(5 * 120 + 5) * 4 + 4],
            if hidden {
                &[0, 128, 0, 255]
            } else {
                &[0, 0, 255, 255]
            },
            "{collapse}/{contents:?}"
        );
        if collapse == "separate" {
            assert_eq!(
                &pixels[(5 * 120) * 4..(5 * 120) * 4 + 4],
                if hidden {
                    &[0, 128, 0, 255]
                } else {
                    &[0, 0, 0, 255]
                },
                "{collapse}/{contents:?}"
            );
        }
    }
}

#[test]
fn fixed_columns_honor_the_first_row_min_width() {
    let (mut doc, body) = document();
    let table = element(
        &mut doc,
        body,
        "display:table;table-layout:fixed;width:50px;border-spacing:0",
    );
    let row = element(&mut doc, table, "display:table-row");
    let first = element(
        &mut doc,
        row,
        "display:table-cell;width:10px;min-width:30px;height:10px;background:green",
    );
    let second = element(
        &mut doc,
        row,
        "display:table-cell;height:10px;background:blue",
    );
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(first).unwrap().unrounded_layout.size.width,
        30.0
    );
    assert_eq!(
        doc.get_node(second).unwrap().unrounded_layout.location.x,
        30.0
    );
    assert_eq!(
        doc.get_node(second).unwrap().unrounded_layout.size.width,
        20.0
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:0;width:30px;height:10px;background:green",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:30px;top:0;width:20px;height:10px;background:blue",
    );
    let expected = layout(&mut reference);
    assert_eq!(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected))
    );
}

#[test]
fn html_ua_middle_and_row_inheritance_align_cell_ink() {
    for (section, cell, baseline) in [
        ("", "", 23.0),
        ("vertical-align:bottom", "", 38.0),
        ("vertical-align:bottom", "vertical-align:top", 8.0),
    ] {
        let source = format!(
            "<!doctype html><style>html,body{{margin:0;font-family:Ahem;font-size:10px;line-height:10px}}table{{border-spacing:0}}tr{{height:40px}}td{{padding:0}}</style><table><tbody style='{section}'><tr><td style='{cell}'>X</td></tr></tbody></table>"
        );
        let options = raikiri_html::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let mut document = raikiri_html::parse(source.as_bytes(), &options).unwrap();
        document.dom.set_font_collection(
            build_bundled_font_collection(
                vec![BundledFace {
                    family: "Ahem".into(),
                    bytes: AHEM.to_vec(),
                }],
                false,
            )
            .unwrap(),
        );
        let computed = raikiri_html::build_cascaded(&document);
        layout_single_page(&mut document.dom, &computed, page()).unwrap();
        let baselines: Vec<_> = scene(&document.dom, &computed)
            .commands
            .iter()
            .filter_map(|command| match command {
                RenderCommand::GlyphRun(run) => {
                    Some(run.transform.as_coeffs()[5] + f64::from(run.glyphs[0].y))
                }
                _ => None,
            })
            .collect();
        assert_eq!(baselines, [baseline], "{section}/{cell}");
    }
}

#[test]
fn caption_area_does_not_receive_the_table_background() {
    for (side, grid_top, blank_top) in [("top", 20, 0), ("bottom", 0, 10)] {
        let (mut doc, body) = document();
        let table = element(
            &mut doc,
            body,
            "display:table;border-spacing:0;width:40px;background:green",
        );
        element(
            &mut doc,
            table,
            &format!("display:table-caption;caption-side:{side};height:20px;width:40px"),
        );
        let row = element(&mut doc, table, "display:table-row");
        element(&mut doc, row, "display:table-cell;width:40px;height:10px");
        let computed = layout(&mut doc);
        let pixels = raster(scene(&doc, &computed));
        let pixel = |y: usize| &pixels[(y * 120 + 5) * 4..(y * 120 + 5) * 4 + 4];
        assert_eq!(pixel(grid_top + 5), &[0, 128, 0, 255], "{side}");
        assert_eq!(pixel(blank_top + 5), &[255, 255, 255, 255], "{side}");
    }
}

#[test]
fn block_cell_content_alignment_does_not_accumulate_across_layouts() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0;width:40px");
    let row = element(&mut doc, table, "display:table-row;height:40px");
    let cell = element(&mut doc, row, "display:table-cell;vertical-align:middle");
    let child = element(
        &mut doc,
        cell,
        "display:block;height:10px;width:10px;background:green",
    );
    let computed = layout(&mut doc);
    for _ in 0..3 {
        layout_single_page(&mut doc, &computed, page()).unwrap();
        assert_eq!(
            doc.get_node(child).unwrap().unrounded_layout.location.y,
            15.0
        );
    }
}

#[test]
fn baseline_alignment_counts_cell_and_nested_block_padding_once() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row");
    for (cell_padding, child_padding) in [(40, 0), (20, 0), (0, 0), (12, 3), (0, 40)] {
        let cell = element(
            &mut doc,
            row,
            &format!("display:table-cell;vertical-align:baseline;padding-top:{cell_padding}px"),
        );
        let child = element(
            &mut doc,
            cell,
            &format!("display:block;padding-top:{child_padding}px"),
        );
        doc.append_text(child, "X");
    }
    let computed = layout(&mut doc);
    let baselines: Vec<_> = scene(&doc, &computed)
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(run) => {
                Some(run.transform.as_coeffs()[5] + f64::from(run.glyphs[0].y))
            }
            _ => None,
        })
        .collect();
    assert_eq!(baselines, [48.0; 5]);
}

#[test]
fn fixed_first_row_min_width_uses_the_cell_box_sizing_domain() {
    for (sizing, expected) in [("content-box", 36.0), ("border-box", 30.0)] {
        let (mut doc, body) = document();
        let table = element(
            &mut doc,
            body,
            "display:table;table-layout:fixed;border-spacing:0;width:50px",
        );
        let row = element(&mut doc, table, "display:table-row");
        let first = element(
            &mut doc,
            row,
            &format!(
                "display:table-cell;width:10px;min-width:30px;padding:2px;border:1px solid black;box-sizing:{sizing}"
            ),
        );
        let second = element(&mut doc, row, "display:table-cell;height:10px");
        layout(&mut doc);
        assert_eq!(
            doc.get_node(first).unwrap().unrounded_layout.size.width,
            expected,
            "{sizing}"
        );
        assert_eq!(
            doc.get_node(second).unwrap().unrounded_layout.size.width,
            50.0 - expected,
            "{sizing}"
        );
    }
}

#[test]
fn table_grid_projection_and_relayout_exclude_caption_space() {
    for side in ["top", "bottom"] {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0;width:40px");
        element(
            &mut doc,
            table,
            &format!("display:table-caption;caption-side:{side};height:20px;width:40px"),
        );
        let row = element(&mut doc, table, "display:table-row");
        element(&mut doc, row, "display:table-cell;width:40px;height:10px");
        let computed = layout(&mut doc);
        for _ in 0..2 {
            doc.project_pages(
                &computed,
                page(),
                &[raikiri_dom::PageSlice {
                    page_index: 0,
                    content_origin_y: 0.0,
                    page_name: None,
                }],
                &[(
                    page(),
                    raikiri_dom::PageMargins::default(),
                    raikiri_dom::PageContentInsets::default(),
                )],
            );
            let rect = doc
                .page_fragments(0)
                .find(|fragment| fragment.node().0 == table as u64)
                .unwrap()
                .paint_rect();
            assert_eq!(
                (rect.x, rect.y, rect.width, rect.height),
                (0.0, if side == "top" { 20.0 } else { 0.0 }, 40.0, 10.0)
            );
            layout_single_page(&mut doc, &computed, page()).unwrap();
        }
        doc.set_element_inline_style(table, Some("display:block;width:40px".into()));
        layout(&mut doc);
        assert_eq!(doc.get_node(table).unwrap().table_grid_box(), None);
    }
}

#[test]
fn empty_cell_classification_preserves_in_flow_content_and_absolute_descendants() {
    for (contents, hidden) in [
        (0, true),
        (1, true),
        (2, false),
        (3, true),
        (4, true),
        (5, false),
        (6, false),
        (7, true),
        (8, false),
        (9, true),
        (10, false),
        (11, true),
        (12, false),
    ] {
        let (mut doc, body) = document();
        let table = element(
            &mut doc,
            body,
            "display:table;border-spacing:0;width:20px;background:green",
        );
        let row = element(&mut doc, table, "display:table-row;height:20px");
        let cell = element(
            &mut doc,
            row,
            "display:table-cell;empty-cells:hide;background:blue;border:1px solid black;vertical-align:top",
        );
        match contents {
            1 => {
                doc.append_text(cell, " \t\n");
            }
            2 => {
                doc.set_element_inline_style(cell,Some("display:table-cell;empty-cells:hide;white-space:pre;background:blue;border:1px solid black".into()));
                doc.append_text(cell, " ");
            }
            3 => {
                element(&mut doc, cell, "display:none");
            }
            4 => {
                element(
                    &mut doc,
                    cell,
                    "position:absolute;left:25px;top:0;width:5px;height:5px;background:green",
                );
            }
            5 => {
                element(&mut doc, cell, "display:inline");
            }
            6 => {
                element(&mut doc, cell, "float:left;width:1px;height:1px");
            }
            7 => {
                element(&mut doc, cell, "display:contents");
            }
            8 => {
                let wrapper = element(&mut doc, cell, "display:contents");
                doc.append_text(wrapper, "X");
            }
            9 => {
                doc.set_element_inline_style(
                    cell,
                    Some("display:table-cell;empty-cells:hide;visibility:hidden".into()),
                );
                doc.append_text(cell, "X");
            }
            10 | 11 => {
                doc.set_element_inline_style(
                    cell,
                    Some("display:table-cell;empty-cells:hide;white-space:pre-line".into()),
                );
                doc.append_text(cell, if contents == 10 { "\n" } else { " " });
            }
            12 => {
                let sheet = doc.append_element(Some(body), "style", Style::default(), None::<&str>);
                doc.append_text(sheet, "span::before{content:'X'}");
                doc.append_element(
                    Some(cell),
                    "span",
                    Style::default(),
                    Some("display:contents"),
                );
            }
            _ => {}
        }
        let computed = layout(&mut doc);
        assert_eq!(
            raikiri_dom::paint_rules::hides_empty_table_cell(&doc, &computed, cell),
            hidden,
            "{contents}"
        );
        doc.project_pages(
            &computed,
            page(),
            &[raikiri_dom::PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None,
            }],
            &[(
                page(),
                raikiri_dom::PageMargins::default(),
                raikiri_dom::PageContentInsets::default(),
            )],
        );
        let boxes=doc.page_paint_order(&computed,0,None).iter().filter(|event|matches!(event,raikiri_dom::PaintEvent::Box(fragment) if fragment.node().0==cell as u64)).count();
        assert_eq!(boxes, usize::from(!hidden), "{contents}");
        if contents == 4 {
            let pixels = raster(scene(&doc, &computed));
            assert_eq!(
                &pixels[(2 * 120 + 27) * 4..(2 * 120 + 27) * 4 + 4],
                &[0, 128, 0, 255]
            );
        }
    }
}

#[test]
fn captions_reserve_their_border_box_and_vertical_margins() {
    for side in ["top", "bottom"] {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0;width:40px");
        let caption = element(
            &mut doc,
            table,
            &format!(
                "display:table-caption;caption-side:{side};height:10px;padding:2px;border:1px solid black;margin:3px 0 4px;background:blue"
            ),
        );
        let row = element(&mut doc, table, "display:table-row");
        let cell = element(
            &mut doc,
            row,
            "display:table-cell;height:10px;background:green",
        );
        let after = element(
            &mut doc,
            body,
            "display:block;height:5px;width:40px;background:red",
        );
        let computed = layout(&mut doc);
        assert_eq!(
            doc.get_node(caption).unwrap().unrounded_layout.size.height,
            16.0,
            "{side}"
        );
        assert_eq!(
            doc.get_node(caption).unwrap().unrounded_layout.location.y,
            if side == "top" { 3.0 } else { 13.0 },
            "{side}"
        );
        assert_eq!(
            doc.get_node(cell).unwrap().unrounded_layout.location.y,
            if side == "top" { 23.0 } else { 0.0 },
            "{side}"
        );
        assert_eq!(
            doc.get_node(after).unwrap().unrounded_layout.location.y,
            33.0,
            "{side}"
        );
        let (mut reference, body) = document();
        let caption_top = if side == "top" { 3 } else { 13 };
        let row_top = if side == "top" { 23 } else { 0 };
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:0;top:{caption_top}px;width:40px;height:16px;background:black"
            ),
        );
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:1px;top:{}px;width:38px;height:14px;background:blue",
                caption_top + 1
            ),
        );
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:0;top:{row_top}px;width:40px;height:10px;background:green"
            ),
        );
        element(
            &mut reference,
            body,
            "position:absolute;left:0;top:33px;width:40px;height:5px;background:red",
        );
        let expected = layout(&mut reference);
        assert_exact_pixels(
            raster(scene(&doc, &computed)),
            raster(scene(&reference, &expected)),
        );
    }
}

#[test]
fn baseline_rowspan_aligns_its_first_row_without_shifting_the_second_row_ink() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;border-spacing:0;width:30px");
    let first_row = element(&mut doc, table, "display:table-row");
    let first = element(
        &mut doc,
        first_row,
        "display:table-cell;vertical-align:baseline;width:10px;color:green",
    );
    doc.append_text(first, "X");
    let span = element(
        &mut doc,
        first_row,
        "display:table-cell;vertical-align:baseline;width:20px;font-size:20px;line-height:20px;color:blue",
    );
    doc.set_element_attributes(span, vec![("rowspan".into(), "2".into())]);
    doc.append_text(span, "X");
    let second_row = element(&mut doc, table, "display:table-row");
    let second = element(
        &mut doc,
        second_row,
        "display:table-cell;vertical-align:baseline;width:10px;color:red",
    );
    doc.append_text(second, "X");
    let computed = layout(&mut doc);
    let baselines: Vec<_> = scene(&doc, &computed)
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(run) => {
                Some(run.transform.as_coeffs()[5] + f64::from(run.glyphs[0].y))
            }
            _ => None,
        })
        .collect();
    assert_eq!(baselines, [16.0, 16.0, 26.0]);
    assert_eq!(
        doc.get_node(first).unwrap().unrounded_layout.size.height,
        18.0
    );
    assert_eq!(
        doc.get_node(span).unwrap().unrounded_layout.size.height,
        28.0
    );
    assert_eq!(
        doc.get_node(second).unwrap().unrounded_layout.location.y,
        18.0
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:8px;width:10px;height:10px;background:green",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:10px;top:0;width:20px;height:20px;background:blue",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:18px;width:10px;height:10px;background:red",
    );
    let expected = layout(&mut reference);
    assert_eq!(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected))
    );
}

#[test]
fn empty_cell_height_includes_its_content_box_border_and_padding() {
    for (sizing, height) in [("content-box", 39.4), ("border-box", 37.4)] {
        let (mut doc, body) = document();
        let table = element(&mut doc, body, "display:table;border-spacing:0;width:122px");
        let row = element(&mut doc, table, "display:table-row");
        let cell = element(
            &mut doc,
            row,
            &format!(
                "display:table-cell;width:120px;height:37.4px;border:1px solid blue;padding:0;box-sizing:{sizing}"
            ),
        );
        layout(&mut doc);
        assert_eq!(
            doc.get_node(cell).unwrap().unrounded_layout.size.height,
            height
        );
    }
}

#[test]
fn empty_table_captions_remain_outside_the_grid_and_reserve_flow_space() {
    for (side, caption_top, grid_top) in [("top", 0.0, 20.0), ("bottom", 10.0, 0.0)] {
        let (mut doc, body) = document();
        let table = element(
            &mut doc,
            body,
            "display:table;width:40px;height:10px;background:green",
        );
        let caption = element(
            &mut doc,
            table,
            &format!("display:table-caption;caption-side:{side};height:20px;background:blue"),
        );
        let after = element(&mut doc, body, "width:40px;height:10px;background:red");
        let computed = layout(&mut doc);
        assert_eq!(
            doc.get_node(table).unwrap().unrounded_layout.size.height,
            30.0
        );
        assert_eq!(
            doc.get_node(caption).unwrap().unrounded_layout.location.y,
            caption_top
        );
        assert_eq!(
            doc.get_node(after).unwrap().unrounded_layout.location.y,
            30.0
        );
        let (mut reference, body) = document();
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:0;top:{grid_top}px;width:40px;height:10px;background:green"
            ),
        );
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:0;top:{caption_top}px;width:40px;height:20px;background:blue"
            ),
        );
        element(
            &mut reference,
            body,
            "position:absolute;left:0;top:30px;width:40px;height:10px;background:red",
        );
        let expected = layout(&mut reference);
        assert_exact_pixels(
            raster(scene(&doc, &computed)),
            raster(scene(&reference, &expected)),
        );
    }
}

#[test]
fn negative_caption_margin_keeps_the_grid_paint_extent() {
    let (mut doc, body) = document();
    element(&mut doc, body, "height:30px");
    let table = element(
        &mut doc,
        body,
        "display:table;width:40px;height:10px;background:green",
    );
    element(
        &mut doc,
        table,
        "display:table-caption;height:20px;margin-bottom:-30px;background:blue",
    );
    element(&mut doc, body, "width:40px;height:5px;background:red");
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(table).unwrap().unrounded_layout.size.height,
        0.0
    );
    let grid = doc.get_node(table).unwrap().table_grid_box().unwrap();
    assert_eq!((grid.y, grid.height), (-10.0, 10.0));
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:20px;width:40px;height:10px;background:green",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:30px;width:40px;height:20px;background:blue",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:30px;width:40px;height:5px;background:red",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn nested_table_uses_its_first_row_baseline_including_caption_offset() {
    let (mut doc, body) = document();
    let outer = element(&mut doc, body, "display:table;width:40px;border-spacing:0");
    let row = element(&mut doc, outer, "display:table-row");
    let left = element(
        &mut doc,
        row,
        "display:table-cell;width:10px;vertical-align:baseline;color:green",
    );
    doc.append_text(left, "X");
    let right = element(
        &mut doc,
        row,
        "display:table-cell;width:30px;vertical-align:baseline",
    );
    let inner = element(&mut doc, right, "display:table;width:30px;border-spacing:0");
    element(&mut doc, inner, "display:table-caption;height:20px");
    let inner_row = element(&mut doc, inner, "display:table-row;height:40px");
    let first = element(
        &mut doc,
        inner_row,
        "display:table-cell;width:10px;vertical-align:top;color:blue",
    );
    doc.append_text(first, "X");
    let second = element(
        &mut doc,
        inner_row,
        "display:table-cell;width:20px;vertical-align:baseline;font-size:20px;line-height:20px;color:red",
    );
    doc.append_text(second, "X");
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(left).unwrap().unrounded_layout.padding.top,
        28.0
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:28px;width:10px;height:10px;background:green",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:10px;top:20px;width:10px;height:10px;background:blue",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:20px;top:20px;width:20px;height:20px;background:red",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn specified_empty_cell_height_floors_the_row_without_moving_text_baselines() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;width:20px;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row");
    let text = element(
        &mut doc,
        row,
        "display:table-cell;width:10px;height:30px;vertical-align:baseline;color:green;background:blue",
    );
    doc.append_text(text, "X");
    let empty = element(
        &mut doc,
        row,
        "display:table-cell;width:10px;height:30px;vertical-align:baseline;background:blue",
    );
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(text).unwrap().unrounded_layout.size.height,
        30.0
    );
    assert_eq!(
        doc.get_node(text).unwrap().unrounded_layout.padding.top,
        0.0
    );
    assert_eq!(
        doc.get_node(empty).unwrap().unrounded_layout.padding.top,
        8.0
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:0;width:20px;height:30px;background:blue",
    );
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:0;width:10px;height:10px;background:green",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn generated_pseudo_boxes_keep_separate_cell_backgrounds_and_borders() {
    for pseudo in ["before", "after"] {
        let (mut doc, body) = document();
        let sheet = doc.append_element(Some(body), "style", Style::default(), None::<&str>);
        doc.append_text(sheet, format!("td::{pseudo} {{ content:'X';color:green }}"));
        let table = element(&mut doc, body, "display:table;width:20px;border-spacing:0");
        let row = element(&mut doc, table, "display:table-row;height:20px");
        let cell = doc.append_element(Some(row), "td", Style::default(), Some("display:table-cell;width:18px;vertical-align:top;empty-cells:hide;background:blue;border:1px solid black"));
        let computed = layout(&mut doc);
        assert!(!raikiri_dom::paint_rules::hides_empty_table_cell(
            &doc, &computed, cell
        ));
        let (mut reference, body) = document();
        element(
            &mut reference,
            body,
            "position:absolute;left:0;top:0;width:20px;height:20px;background:black",
        );
        element(
            &mut reference,
            body,
            "position:absolute;left:1px;top:1px;width:18px;height:18px;background:blue",
        );
        element(
            &mut reference,
            body,
            "position:absolute;left:1px;top:1px;width:10px;height:10px;background:green",
        );
        let expected = layout(&mut reference);
        assert_exact_pixels(
            raster(scene(&doc, &computed)),
            raster(scene(&reference, &expected)),
        );
    }
}

#[test]
fn spanning_only_baseline_cell_keeps_its_baseline_inside_the_first_row() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;width:40px;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row");
    let cell = element(
        &mut doc,
        row,
        "display:table-cell;width:40px;vertical-align:baseline;font-size:40px;line-height:40px;color:green",
    );
    doc.set_element_attributes(cell, vec![("rowspan".into(), "2".into())]);
    doc.append_text(cell, "X");
    element(&mut doc, table, "display:table-row");
    let computed = layout(&mut doc);
    assert_eq!(
        doc.get_node(cell).unwrap().unrounded_layout.size.height,
        40.0
    );
    let (mut reference, body) = document();
    element(
        &mut reference,
        body,
        "position:absolute;left:0;top:0;width:40px;height:40px;background:green",
    );
    let expected = layout(&mut reference);
    assert_exact_pixels(
        raster(scene(&doc, &computed)),
        raster(scene(&reference, &expected)),
    );
}

#[test]
fn generated_box_empty_cell_classification_respects_box_generation_and_flow() {
    for (declarations, hidden) in [
        ("content:none", true),
        ("content:normal", true),
        ("content:'X';display:none", true),
        ("content:'X';position:absolute", true),
        ("content:'X';position:fixed", true),
        ("content:'X';float:left", false),
        ("content:''", false),
        ("content:url(data:image/svg+xml,invalid)", false),
    ] {
        let (mut doc, body) = document();
        let sheet = doc.append_element(Some(body), "style", Style::default(), None::<&str>);
        doc.append_text(sheet, format!("td::before {{ {declarations} }}"));
        let table = element(&mut doc, body, "display:table;width:20px;border-spacing:0");
        let row = element(&mut doc, table, "display:table-row;height:20px");
        let cell = doc.append_element(
            Some(row),
            "td",
            Style::default(),
            Some("display:table-cell;empty-cells:hide"),
        );
        let computed = layout(&mut doc);
        assert_eq!(
            raikiri_dom::paint_rules::hides_empty_table_cell(&doc, &computed, cell),
            hidden,
            "{declarations}"
        );
    }
}

#[test]
fn comments_and_processing_instructions_leave_a_cell_empty_after_membership_refresh() {
    let (mut doc, body) = document();
    let table = element(&mut doc, body, "display:table;width:20px;border-spacing:0");
    let row = element(&mut doc, table, "display:table-row");
    let cell = element(&mut doc, row, "display:table-cell;empty-cells:hide");
    let comment = doc.append_comment(Some(cell), "gap");
    let pi = doc.append_processing_instruction(Some(cell), "example", "gap");
    let computed = layout(&mut doc);
    assert!(!doc.get_node(comment).unwrap().is_in_document());
    assert!(!doc.get_node(pi).unwrap().is_in_document());
    assert!(raikiri_dom::paint_rules::hides_empty_table_cell(
        &doc, &computed, cell
    ));
}

#[test]
fn block_in_inline_and_preserved_lines_align_to_identical_cell_content() {
    let html = "<style>html,body{margin:0;font-family:Ahem;font-size:10px;line-height:10px}table{border-spacing:0}td{padding:0}.inline{display:inline}.block{display:block;margin:10px 0}pre{font-family:Ahem;font-size:10px;line-height:10px;margin:10px 0}</style><table><tr><td><div class=block><div class=inline>A<div class=block>B</div><div class=block>C</div>D<div class=block>E</div>F<div class=block>G</div></div></div></td><td><pre>A\n\nB\n\nC\n\nD\n\nE\n\nF\n\nG</pre></td></tr></table>";
    let options = raikiri_html::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let mut parsed = raikiri_html::parse(html.as_bytes(), &options).unwrap();
    parsed.dom.set_font_collection(
        build_bundled_font_collection(
            vec![BundledFace {
                family: "Ahem".into(),
                bytes: AHEM.to_vec(),
            }],
            false,
        )
        .unwrap(),
    );
    let computed = raikiri_html::build_cascaded(&parsed);
    layout_single_page(&mut parsed.dom, &computed, page()).unwrap();
    for id in 0..parsed.dom.node_count() {
        let node = parsed.dom.get_node(id).unwrap();
        if node.tag_name() == Some("td") {
            assert_eq!(node.unrounded_layout.size.height, 150.0);
            assert_eq!(node.unrounded_layout.padding.top, 0.0);
        }
    }
    let painted = scene(&parsed.dom, &computed);
    for x in [0.0, 10.0] {
        let mut baselines: Vec<_> = painted
            .commands
            .iter()
            .filter_map(|command| match command {
                RenderCommand::GlyphRun(run) if run.transform.as_coeffs()[4] == x => Some(
                    run.glyphs
                        .iter()
                        .map(|glyph| run.transform.as_coeffs()[5] + f64::from(glyph.y))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .filter(|baseline| *baseline < f64::from(page().height))
            .collect();
        baselines.sort_by(f64::total_cmp);
        assert_eq!(baselines, [18.0, 38.0, 58.0, 78.0, 98.0]);
    }
    let (mut reference, body) = document();
    for top in [10, 30, 50, 70, 90, 110, 130] {
        element(
            &mut reference,
            body,
            &format!(
                "position:absolute;left:0;top:{top}px;width:20px;height:10px;background:black"
            ),
        );
    }
    let expected = layout(&mut reference);
    assert_exact_pixels(raster(painted), raster(scene(&reference, &expected)));
}
