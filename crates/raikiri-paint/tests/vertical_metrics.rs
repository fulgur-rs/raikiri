//! Parent font metrics, checked against literal positions with bundled Ahem.

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

fn fixture(align: &str, wrapper: bool, atomic: bool) -> (Document, CascadeResult, usize, usize) {
    let mut doc = Document::new();
    doc.set_font_collection(
        build_bundled_font_collection(
            vec![BundledFace {
                family: "Ahem".into(),
                bytes: AHEM.to_vec(),
            }],
            false,
        )
        .expect("Ahem only"),
    );
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let root = doc.append_element(Some(body), "div", Style::default(), Some("display:block;font-family:Ahem;font-size:20px;line-height:40px;width:200px;color:black"));
    doc.append_text(root, "X");
    let parent = if wrapper {
        doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("font-size:30px;line-height:30px"),
        )
    } else {
        root
    };
    let css = format!(
        "font-size:10px;line-height:10px;vertical-align:{align};{}",
        if atomic {
            "width:10px;height:10px;background:green"
        } else {
            "color:green"
        }
    );
    let child = doc.append_element(
        Some(parent),
        if atomic { "img" } else { "span" },
        Style::default(),
        Some(&css),
    );
    if !atomic {
        doc.append_text(child, "X");
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let result = cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &result, page()).expect("layout");
    (doc, result, root, child)
}

fn page() -> PageBox {
    let mut page = PageBox::new();
    page.width = 200.0;
    page.height = 80.0;
    page
}

fn scene(doc: &Document, cascade: &CascadeResult) -> Scene {
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page(&mut scene, doc, cascade, page()).expect("paint");
    scene
}

#[test]
fn inline_keyword_baselines_use_the_immediate_parent_font() {
    // Ahem ascent/descent/x-height are 0.8/0.2/0.8 em. The 20px root's
    // 40px strut has baseline 26, content edges 10/30, and middle at 18.
    // A 30px wrapper has content edges 2/32 and middle at 14, on baseline 26.
    for (wrapper, align, expected) in [
        (false, "middle", 21.0),
        (false, "text-top", 18.0),
        (false, "text-bottom", 28.0),
        (true, "middle", 17.0),
        (true, "text-top", 10.0),
        (true, "text-bottom", 30.0),
    ] {
        let (doc, cascade, root, child) = fixture(align, wrapper, false);
        assert!(doc.get_node(root).unwrap().is_ifc_root());
        assert!(doc.get_node(child).unwrap().in_ifc_subtree());
        let baselines: Vec<f64> = scene(&doc, &cascade)
            .commands
            .iter()
            .filter_map(|command| {
                let RenderCommand::GlyphRun(run) = command else {
                    return None;
                };
                (run.font_size == 10.0)
                    .then(|| run.transform.as_coeffs()[5] + f64::from(run.glyphs[0].y))
            })
            .collect();
        assert_eq!(baselines, [expected], "{wrapper}/{align}");
    }
}

#[test]
fn replaced_keyword_positions_use_parent_content_edges_and_half_x_height() {
    for (wrapper, align, top) in [
        (false, "middle", 13.0),
        (false, "text-top", 10.0),
        (false, "text-bottom", 20.0),
        (true, "middle", 9.0),
        (true, "text-top", 2.0),
        (true, "text-bottom", 22.0),
    ] {
        let (doc, _, root, child) = fixture(align, wrapper, true);
        assert!(doc.get_node(root).unwrap().is_ifc_root());
        assert!(!doc.get_node(child).unwrap().in_ifc_subtree());
        assert_eq!(
            doc.get_node(child).unwrap().unrounded_layout.location.y,
            top,
            "{wrapper}/{align}"
        );
    }
}

#[test]
fn keyword_text_and_replaced_ink_match_literal_positioned_references_exactly() {
    for (wrapper, align, top) in [
        (false, "middle", 13.0),
        (false, "text-top", 10.0),
        (false, "text-bottom", 20.0),
        (true, "middle", 9.0),
        (true, "text-top", 2.0),
        (true, "text-bottom", 22.0),
    ] {
        for atomic in [false, true] {
            let (doc, cascade, _, _) = fixture(align, wrapper, atomic);
            let actual = scene(&doc, &cascade);
            // Build a separate positioned reference without vertical-align.
            let mut reference = Document::new();
            let html =
                reference.append_element(Some(0), "html", Style::default(), Some("display:block"));
            let body = reference.append_element(
                Some(html),
                "body",
                Style::default(),
                Some("display:block"),
            );
            reference.append_element(
                Some(body),
                "div",
                Style::default(),
                Some("position:absolute;left:0;top:10px;width:20px;height:20px;background:black"),
            );
            reference.append_element(Some(body), "div", Style::default(), Some(&format!("position:absolute;left:20px;top:{top}px;width:10px;height:10px;background:green")));
            reference.mark_in_document_flags();
            let tree = build_rule_tree(&reference);
            let expected_cascade = raikiri_style::cascade(&reference, &tree).unwrap();
            layout_single_page(&mut reference, &expected_cascade, page()).unwrap();
            let expected = scene(&reference, &expected_cascade);
            let raster = |scene| {
                anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
                    |out| out.append_scene(scene, Affine::IDENTITY),
                    200,
                    80,
                )
            };
            assert_eq!(
                raster(actual),
                raster(expected),
                "{wrapper}/{align}/{atomic}"
            );
        }
    }
}

#[test]
fn legacy_paint_does_not_align_blockified_or_out_of_flow_boxes_to_parent_metrics() {
    // Flex/grid items, floats and absolute boxes do not participate in an
    // inline line box. Their parent is not an IFC root, so the walk reaches
    // its legacy shift helper. CSS vertical-align does not apply: baseline
    // 8 and a square at (0, 0) are independent of the parent's 20px metrics.
    for (route, display, item_style) in [
        ("flex", "flex", "display:inline"),
        ("grid", "grid", "display:inline"),
        ("float", "block", "display:inline-block;float:left"),
        (
            "absolute",
            "block",
            "display:inline-block;position:absolute;left:0;top:0",
        ),
    ] {
        for align in ["baseline", "middle", "text-top", "text-bottom"] {
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
            let body =
                doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
            let root = doc.append_element(Some(body), "div", Style::default(), Some(&format!("display:{display};position:relative;font-family:Ahem;font-size:20px;line-height:40px;width:100px;height:40px")));
            let item = doc.append_element(Some(root), "span", Style::default(), Some(&format!("{item_style};font-size:10px;line-height:10px;width:10px;height:10px;vertical-align:{align}")));
            doc.append_text(item, "X");
            doc.mark_in_document_flags();
            let tree = build_rule_tree(&doc);
            let cascade = cascade(&doc, &tree).unwrap();
            layout_single_page(&mut doc, &cascade, page()).unwrap();
            assert!(!doc.get_node(root).unwrap().is_ifc_root());
            assert!(doc.get_node(item).unwrap().is_ifc_root());
            assert_eq!(doc.get_node(item).unwrap().unrounded_layout.location.y, 0.0);
            let scene = scene(&doc, &cascade);
            let baselines: Vec<f64> = scene
                .commands
                .iter()
                .filter_map(|command| {
                    let RenderCommand::GlyphRun(run) = command else {
                        return None;
                    };
                    Some(run.transform.as_coeffs()[5] + f64::from(run.glyphs[0].y))
                })
                .collect();
            assert_eq!(baselines, [8.0], "{route}/{align}");
            let pixels = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
                |out| out.append_scene(scene, Affine::IDENTITY),
                200,
                80,
            );
            for y in 0..80 {
                for x in 0..200 {
                    let expected = if x < 10 && y < 10 {
                        [0, 0, 0, 255]
                    } else {
                        [255, 255, 255, 255]
                    };
                    assert_eq!(
                        &pixels[(y * 200 + x) * 4..(y * 200 + x) * 4 + 4],
                        &expected,
                        "{route}/{align}/{x}/{y}"
                    );
                }
            }
        }
    }
}
