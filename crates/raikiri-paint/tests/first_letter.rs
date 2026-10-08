//! Literal font and paint checks for typographic first-letter styling.

use anyrender::{Scene, recording::RenderCommand};
use raikiri_dom::{BundledFace, Document, build_bundled_font_collection, layout_single_page};
use raikiri_style::{CascadeResult, build_rule_tree, cascade};
use raikiri_traits::PageBox;
use taffy::Style;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn pixels(doc: &Document, computed: &CascadeResult) -> Vec<u8> {
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page(&mut scene, doc, computed, page).unwrap();
    scene_pixels(scene)
}

fn scene_pixels(scene: Scene) -> Vec<u8> {
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| {
            use anyrender::PaintScene;
            out.append_scene(scene, kurbo::Affine::IDENTITY);
        },
        100,
        80,
    )
}

#[test]
fn inside_text_marker_keeps_its_ink_with_and_without_first_letter_opacity() {
    for opacity in [false, true] {
        let sheet = if opacity {
            "li::first-letter{color:red;opacity:.5}"
        } else {
            ""
        };
        let (mut doc, _, empty) = fixture(sheet, None, "");
        let root = doc.parent_of(empty).unwrap();
        let ol = doc.append_element(
            Some(root),
            "ol",
            Style::default(),
            Some("display:block;list-style:decimal-leading-zero inside"),
        );
        let li = doc.append_element(Some(ol), "li", Style::default(), Some("display:list-item"));
        doc.append_text(li, "XX");
        let (mut reference, _, prefix) = fixture("", None, "01. ");
        let reference_root = reference.parent_of(prefix).unwrap();
        let first = reference.append_element(
            Some(reference_root),
            "span",
            Style::default(),
            Some(if opacity {
                "color:#ff7f7f"
            } else {
                "color:black"
            }),
        );
        reference.append_text(first, "X");
        reference.append_text(reference_root, "X");
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 80.0;
        doc.mark_in_document_flags();
        reference.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        let reference_cv = cascade(&reference, &build_rule_tree(&reference)).unwrap();
        layout_single_page(&mut doc, &computed, page).unwrap();
        layout_single_page(&mut reference, &reference_cv, page).unwrap();
        let rgba = pixels(&doc, &computed);
        assert_eq!(rgba, pixels(&reference, &reference_cv));
        assert_eq!(
            rgba.chunks_exact(4)
                .filter(|pixel| *pixel == [0, 0, 0, 255])
                .count(),
            if opacity { 400 } else { 500 }
        );
    }
}

#[test]
fn nested_first_letter_opacity_keeps_each_group_and_remainder_separate() {
    let (doc, computed, _) = fixture(
        "body::first-letter{opacity:.5} div::first-letter{font-size:20px;color:red;background:blue;opacity:.5}",
        None,
        "XX",
    );
    let rgba = pixels(&doc, &computed);
    let first = (15 * 100 + 5) * 4;
    let rest = (20 * 100 + 25) * 4;
    assert_eq!(&rgba[first..first + 4], &[255, 191, 191, 255]);
    assert_eq!(&rgba[rest..rest + 4], &[0, 0, 0, 255]);
}

#[test]
fn transformed_first_letter_opacity_keeps_its_clip_in_page_coordinates() {
    let (doc, computed, _) = fixture(
        "div{position:absolute;left:900px;top:0;transform:matrix(1,0,0,1,-900,0)} div::first-letter{font-size:20px;color:red;background:blue;opacity:.5}",
        None,
        "XX",
    );
    let rgba = pixels(&doc, &computed);
    let first = (15 * 100 + 5) * 4;
    let rest = (20 * 100 + 25) * 4;
    assert_eq!(&rgba[first..first + 4], &[255, 127, 127, 255]);
    assert_eq!(&rgba[rest..rest + 4], &[0, 0, 0, 255]);
}

#[test]
fn singular_ancestor_transform_leaves_no_first_letter_opacity_ink() {
    let (doc, computed, _) = fixture(
        "div{transform:matrix(0,0,0,0,0,0)} div::first-letter{font-size:20px;color:red;background:blue;opacity:.5}",
        None,
        "XX",
    );
    assert!(
        pixels(&doc, &computed)
            .chunks_exact(4)
            .all(|pixel| pixel == [255, 255, 255, 255])
    );
}

#[test]
fn cumulative_tiny_transforms_leave_no_first_letter_opacity_ink() {
    let (mut doc, _, empty) = fixture(
        "div{transform:scale(1e-40);transform-origin:0 0} div::first-letter{color:red;background:blue;opacity:.5}",
        None,
        "",
    );
    let mut root = doc.parent_of(empty).unwrap();
    for _ in 0..3 {
        root = doc.append_element(Some(root), "div", Style::default(), Some("display:block"));
    }
    doc.append_text(root, "XX");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page(&mut scene, &doc, &computed, page).unwrap();
    let layers: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::PushLayer(layer) if layer.alpha == 0.5 => Some(layer),
            _ => None,
        })
        .collect();
    assert_eq!(layers.len(), 4);
    assert!(layers.iter().all(|layer| {
        layer
            .transform
            .as_coeffs()
            .iter()
            .all(|value| value.is_finite())
    }));
    assert!(
        scene_pixels(scene)
            .chunks_exact(4)
            .all(|pixel| pixel == [255, 255, 255, 255])
    );
}

#[test]
fn first_letter_url_background_uses_the_image_pixel_source() {
    use raikiri_traits::{DecodedImage, ImagePixelSource};
    use std::sync::Arc;
    use url::Url;
    struct Pixels;
    impl ImagePixelSource for Pixels {
        fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
            (url.as_str() == "file:///first-letter.png").then(|| {
                Arc::new(DecodedImage {
                    width: 1,
                    height: 1,
                    rgba: vec![0, 128, 0, 255],
                })
            })
        }
    }
    let (doc, computed, _) = fixture(
        "div::first-letter{font-size:20px;color:transparent;background-color:blue;background-image:url(file:///first-letter.png)}",
        None,
        "XX",
    );
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(
        &mut scene,
        &doc,
        &computed,
        page,
        &Pixels,
        &mut raikiri_dom::CounterSnapshotBudget::default(),
    )
    .unwrap();
    let rgba = scene_pixels(scene);
    let first = (15 * 100 + 5) * 4;
    assert_eq!(&rgba[first..first + 4], &[0, 128, 0, 255]);
}

#[test]
fn first_letter_opacity_spans_punctuation_split_by_an_inline_boundary() {
    let sheet = "div::first-letter{font-size:20px;color:red;background:blue;opacity:.5}";
    let (mut split, _, text) = fixture(sheet, Some("display:inline"), "\"");
    let root = split.parent_of(split.parent_of(text).unwrap()).unwrap();
    split.append_text(root, "XY");
    split.mark_in_document_flags();
    let computed = cascade(&split, &build_rule_tree(&split)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut split, &computed, page).unwrap();
    let (joined, joined_cv, _) = fixture(sheet, None, "\"XY");
    let rgba = pixels(&split, &computed);
    assert_eq!(rgba, pixels(&joined, &joined_cv));
    assert_eq!(
        &rgba[(15 * 100 + 5) * 4..(15 * 100 + 5) * 4 + 4],
        &[255, 127, 127, 255]
    );
    assert_eq!(
        &rgba[(15 * 100 + 25) * 4..(15 * 100 + 25) * 4 + 4],
        &[255, 127, 127, 255]
    );
    assert_eq!(rgba[(20 * 100 + 45) * 4 + 3], 255);
}

#[test]
fn first_letter_opacity_composites_overlapping_background_and_glyph_once() {
    for (opacity, expected) in [("0", [255, 255, 255, 255]), ("0.5", [255, 127, 127, 255])] {
        let (doc, computed, _) = fixture(
            &format!(
                "div::first-letter{{font-size:20px;color:red;background:blue;opacity:{opacity}}}"
            ),
            None,
            "XX",
        );
        let rgba = pixels(&doc, &computed);
        let first = (15 * 100 + 5) * 4;
        let rest = (20 * 100 + 25) * 4;
        assert_eq!(&rgba[first..first + 4], &expected, "opacity={opacity}");
        assert_eq!(&rgba[rest..rest + 4], &[0, 0, 0, 255]);
    }
}

#[test]
fn first_letter_background_gradient_is_painted_over_its_color() {
    let (doc, computed, _) = fixture(
        "div::first-letter{font-size:20px;color:transparent;background-color:blue;background-image:linear-gradient(green,green)}",
        None,
        "XX",
    );
    let rgba = pixels(&doc, &computed);
    let first = (15 * 100 + 5) * 4;
    assert_eq!(&rgba[first..first + 4], &[0, 128, 0, 255]);
}

fn fixture(
    sheet: &str,
    nested_style: Option<&str>,
    text: &str,
) -> (Document, CascadeResult, usize) {
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
    let style = doc.append_element(Some(html), "style", Style::default(), Some("display:none"));
    doc.append_text(style, sheet);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let root = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:100px;font:10px/40px Ahem;color:black"),
    );
    let parent = nested_style.map_or(root, |style| {
        doc.append_element(Some(root), "span", Style::default(), Some(style))
    });
    let text_node = doc.append_text(parent, text);
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    (doc, computed, text_node)
}

fn glyph_sizes(doc: &Document, computed: &CascadeResult) -> Vec<(f32, usize)> {
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page(&mut scene, doc, computed, page).unwrap();
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(run) => Some((run.font_size, run.glyphs.len())),
            _ => None,
        })
        .collect()
}

#[test]
fn first_letter_changes_only_the_first_typographic_unit() {
    let (doc, computed, _) = fixture("div::first-letter { font-size:20px;color:red }", None, "XX");
    assert_eq!(glyph_sizes(&doc, &computed), [(20.0, 1), (10.0, 1)]);
}

#[test]
fn first_letter_inherits_from_the_actual_inline_parent() {
    let (doc, computed, _) = fixture(
        "div::first-letter { font-size:2em;color:red }",
        Some("font-size:20px;color:blue"),
        "XX",
    );
    assert_eq!(glyph_sizes(&doc, &computed), [(40.0, 1), (20.0, 1)]);
}

#[test]
fn first_letter_keeps_punctuation_and_leading_whitespace_together() {
    for text in ["\"X\"Y", "  \"X\"Y"] {
        let (doc, computed, _) = fixture("div::first-letter { font-size:20px }", None, text);
        assert_eq!(glyph_sizes(&doc, &computed), [(20.0, 3), (10.0, 1)]);
    }
}

#[test]
fn punctuation_only_text_and_preserved_initial_line_break_have_no_first_letter() {
    for (style, text) in [(None, "\"\""), (Some("white-space:pre"), "\nXX")] {
        let (doc, computed, _) = fixture("div::first-letter { font-size:20px }", style, text);
        assert_eq!(glyph_sizes(&doc, &computed), [(10.0, 2)]);
    }
}

#[test]
fn first_letter_ink_matches_literal_rectangles_and_preserves_dom_offsets() {
    use kurbo::Affine;
    use shodo::node::{NodeId, TextSource};
    for (nested, first_rect, second_rect) in [
        (
            None,
            "left:0;top:10px;width:20px;height:20px;background:red",
            "left:20px;top:18px;width:10px;height:10px;background:black",
        ),
        (
            Some("position:relative;left:7px;top:9px"),
            "left:7px;top:19px;width:20px;height:20px;background:red",
            "left:27px;top:27px;width:10px;height:10px;background:black",
        ),
    ] {
        let (doc, computed, text) = fixture(
            "div::first-letter { font-size:20px;color:red }",
            nested,
            "XX",
        );
        let parent = doc.parent_of(text).unwrap();
        let root = if nested.is_some() {
            doc.parent_of(parent).unwrap()
        } else {
            parent
        };
        let lines = raikiri_dom::PositionedLines::new(&doc, &computed, root, None).unwrap();
        let runs: Vec<_> = lines.lines().flat_map(|line| line.runs).collect();
        assert_eq!(
            runs.iter().map(|run| run.run.source()).collect::<Vec<_>>(),
            [
                Some(TextSource::Dom {
                    node: NodeId(text as u64),
                    offset: 0
                }),
                Some(TextSource::Dom {
                    node: NodeId(text as u64),
                    offset: 1
                }),
            ]
        );
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 80.0;
        let mut actual = Scene::new();
        raikiri_paint::paint_single_page(&mut actual, &doc, &computed, page).unwrap();
        let mut expected = Document::new();
        let html =
            expected.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body =
            expected.append_element(Some(html), "body", Style::default(), Some("display:block"));
        expected.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(format!("position:absolute;{first_rect}").as_str()),
        );
        expected.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(format!("position:absolute;{second_rect}").as_str()),
        );
        expected.mark_in_document_flags();
        let expected_cv = cascade(&expected, &build_rule_tree(&expected)).unwrap();
        layout_single_page(&mut expected, &expected_cv, page).unwrap();
        let mut reference = Scene::new();
        raikiri_paint::paint_single_page(&mut reference, &expected, &expected_cv, page).unwrap();
        let raster = |scene| {
            anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
                |out| {
                    use anyrender::PaintScene;
                    out.append_scene(scene, Affine::IDENTITY);
                },
                100,
                80,
            )
        };
        assert_eq!(raster(actual), raster(reference));
    }
}

#[test]
fn generated_first_letter_has_distinct_paint_from_its_generated_remainder() {
    let (doc, computed, text) = fixture(
        "div::before {content:'XX';color:blue} div::first-letter {color:red;font-size:20px}",
        None,
        "Y",
    );
    assert_eq!(
        glyph_sizes(&doc, &computed),
        [(20.0, 1), (10.0, 1), (10.0, 1)]
    );
    let root = doc.parent_of(text).unwrap();
    let lines = raikiri_dom::PositionedLines::new(&doc, &computed, root, None).unwrap();
    let colors: Vec<_> = lines
        .lines()
        .flat_map(|line| line.runs)
        .map(|run| run.style.color)
        .collect();
    assert_eq!(
        colors,
        [
            raikiri_style::property::CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            },
            raikiri_style::property::CssColor {
                r: 0,
                g: 0,
                b: 255,
                a: 255
            },
            raikiri_style::property::CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255
            },
        ]
    );
}

#[test]
fn ancestor_and_inner_block_letters_nest_over_the_actual_text_parent() {
    let (doc, computed, _) = fixture(
        "body::first-letter{font-size:2em} div::first-letter{font-size:2em}",
        None,
        "XX",
    );
    assert_eq!(glyph_sizes(&doc, &computed), [(40.0, 1), (10.0, 1)]);
}

#[test]
fn comments_do_not_split_the_first_typographic_unit() {
    let (mut doc, _, text) = fixture("div::first-letter{font-size:20px}", None, "\"");
    let root = doc.parent_of(text).unwrap();
    doc.append_comment(Some(root), "gap");
    doc.append_text(root, "X\"Y");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let sizes = glyph_sizes(&doc, &computed);
    assert_eq!(
        sizes
            .iter()
            .filter(|&&(size, _)| size == 20.0)
            .map(|&(_, count)| count)
            .sum::<usize>(),
        3
    );
    assert_eq!(
        sizes
            .iter()
            .filter(|&&(size, _)| size == 10.0)
            .map(|&(_, count)| count)
            .sum::<usize>(),
        1
    );
}

#[test]
fn source_byte_offsets_follow_multibyte_and_comment_split_graphemes() {
    use shodo::node::{NodeId, TextSource};
    let (mut doc, _, text) = fixture("div::first-letter{font-size:20px}", None, "X");
    let root = doc.parent_of(text).unwrap();
    doc.append_comment(Some(root), "grapheme boundary is not a DOM boundary");
    let rest = doc.append_text(root, "\u{301}Y");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let lines = raikiri_dom::PositionedLines::new(&doc, &computed, root, None).unwrap();
    let sources: Vec<_> = lines
        .lines()
        .flat_map(|line| line.runs)
        .map(|run| (run.run.source(), run.run.font_size()))
        .collect();
    assert!(sources.contains(&(
        Some(TextSource::Dom {
            node: NodeId(text as u64),
            offset: 0
        }),
        20.0
    )));
    assert!(sources.contains(&(
        Some(TextSource::Dom {
            node: NodeId(rest as u64),
            offset: 2
        }),
        10.0
    )));
}

#[test]
fn a_separate_whitespace_node_does_not_consume_the_letter() {
    let (mut doc, _, text) = fixture("div::first-letter{font-size:20px}", None, "\u{a0}");
    let root = doc.parent_of(text).unwrap();
    let span = doc.append_element(Some(root), "span", Style::default(), None::<&str>);
    doc.append_text(span, "XY");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    assert_eq!(
        glyph_sizes(&doc, &computed),
        [(10.0, 1), (20.0, 1), (10.0, 1)]
    );
}

#[test]
fn first_letter_inherits_first_line_without_overriding_explicit_inline_values() {
    for (inline, expected) in [
        (None, 40.0),
        (Some("font-size:10px"), 20.0),
        (Some("font-size:1.5em"), 60.0),
    ] {
        let (doc, computed, _) = fixture(
            "div::first-line{font-size:20px;color:blue} div::first-letter{font-size:2em}",
            inline,
            "XX",
        );
        let sizes = glyph_sizes(&doc, &computed);
        assert_eq!(sizes[0], (expected, 1));
    }
}

#[test]
fn first_line_relative_fonts_and_real_custom_properties_reach_the_letter() {
    for inline in ["font-size:200%;--ink:red", "font-size:2em;--ink:red"] {
        let (doc, computed, text) = fixture(
            "div{--ink:black} div::first-line{font-size:20px;--ink:green} div::first-letter{font-size:50%;color:var(--ink)}",
            Some(inline),
            "XX",
        );
        assert_eq!(glyph_sizes(&doc, &computed)[0], (20.0, 1));
        let root = doc.parent_of(doc.parent_of(text).unwrap()).unwrap();
        let lines = raikiri_dom::PositionedLines::new(&doc, &computed, root, None).unwrap();
        assert_eq!(
            lines
                .lines()
                .flat_map(|line| line.runs)
                .next()
                .unwrap()
                .style
                .color,
            raikiri_style::property::CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }
        );
    }
    let (doc, computed, _) = fixture(
        "div::first-line{font-size:20px} div::before{content:'XX';font-size:200%} div::first-letter{font-size:50%}",
        None,
        "Y",
    );
    assert_eq!(glyph_sizes(&doc, &computed)[0], (20.0, 1));
}

#[test]
fn a_comment_split_typographic_unit_has_one_set_of_inline_edges() {
    use kurbo::Affine;
    let sheet =
        "div::first-letter{font-size:20px;color:red;padding:0 1px;border-left:1px solid blue}";
    let (mut split, _, first) = fixture(sheet, None, "A");
    let root = split.parent_of(first).unwrap();
    split.append_comment(
        Some(root),
        "source segmentation does not duplicate the pseudo box",
    );
    split.append_text(root, "!Y");
    split.mark_in_document_flags();
    let computed = cascade(&split, &build_rule_tree(&split)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut split, &computed, page).unwrap();
    let (joined, joined_computed, _) = fixture(sheet, None, "A!Y");
    let render = |doc: &Document, computed: &CascadeResult| {
        let mut scene = Scene::new();
        raikiri_paint::paint_single_page(&mut scene, doc, computed, page).unwrap();
        anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| {
                use anyrender::PaintScene;
                out.append_scene(scene, Affine::IDENTITY);
            },
            100,
            80,
        )
    };
    assert_eq!(render(&split, &computed), render(&joined, &joined_computed));
}

#[test]
fn only_the_first_in_flow_block_supplies_an_ancestors_letter() {
    let run = |prefix: &str| {
        let (mut doc, _, text) = fixture("body::first-letter{font-size:20px}", None, "XX");
        let root = doc.parent_of(text).unwrap();
        let body = doc.parent_of(root).unwrap();
        let first = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;font:10px/40px Ahem"),
        );
        let target = doc.append_text(first, "XX");
        let previous = if let Some(text) = prefix.strip_prefix("text:") {
            doc.append_text(body, text)
        } else if prefix == "comment" {
            doc.append_comment(Some(body), "ignored")
        } else {
            doc.append_element(
                Some(body),
                if prefix == "nonrendered" {
                    "meta"
                } else {
                    "div"
                },
                Style::default(),
                Some(if prefix == "nonrendered" {
                    "display:block"
                } else {
                    prefix
                }),
            )
        };
        doc.detach_from_parent(root).unwrap();
        doc.detach_from_parent(previous).unwrap();
        doc.insert_child_before(body, first, previous);
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 80.0;
        layout_single_page(&mut doc, &computed, page).unwrap();
        let lines = raikiri_dom::PositionedLines::new(&doc, &computed, first, None).unwrap();
        lines
            .lines()
            .flat_map(|line| line.runs)
            .filter(|run| run.owner == target)
            .map(|run| run.run.font_size())
            .collect::<Vec<_>>()
    };
    for prefix in [
        "display:none",
        "display:block;float:left",
        "display:block;position:absolute",
        "nonrendered",
        "comment",
        "text: ",
    ] {
        assert_eq!(run(prefix), [20.0, 10.0], "{prefix}");
    }
    for prefix in ["display:block", "text:X"] {
        assert_eq!(run(prefix), [10.0], "{prefix}");
    }
}

#[test]
fn inline_boundary_slices_keep_one_set_of_first_letter_edges() {
    use kurbo::Affine;
    let sheet = "div::first-letter{font-size:20px;color:red;padding:0 1px;border-left:1px solid blue;border-right:1px solid green;background:yellow}";
    let render = |doc: &Document, computed: &CascadeResult| {
        let mut scene = Scene::new();
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 80.0;
        raikiri_paint::paint_single_page(&mut scene, doc, computed, page).unwrap();
        anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| {
                use anyrender::PaintScene;
                out.append_scene(scene, Affine::IDENTITY);
            },
            100,
            80,
        )
    };
    for trailing in [false, true] {
        let (mut split, _, text) = fixture(
            sheet,
            if trailing {
                None
            } else {
                Some("display:inline")
            },
            if trailing { "A" } else { "\"" },
        );
        let parent = split.parent_of(text).unwrap();
        let root = if trailing {
            parent
        } else {
            split.parent_of(parent).unwrap()
        };
        if trailing {
            let span =
                split.append_element(Some(root), "span", Style::default(), Some("display:inline"));
            split.append_text(span, "\"");
        } else {
            split.append_text(root, "A");
        }
        split.append_text(root, "Y");
        split.mark_in_document_flags();
        let computed = cascade(&split, &build_rule_tree(&split)).unwrap();
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 80.0;
        layout_single_page(&mut split, &computed, page).unwrap();
        let (joined, joined_computed, _) =
            fixture(sheet, None, if trailing { "A\"Y" } else { "\"AY" });
        assert!(
            render(&split, &computed) == render(&joined, &joined_computed),
            "trailing={trailing}"
        );
    }
}

#[test]
fn split_first_letter_uses_each_inline_parents_source_font_color_and_offsets() {
    use shodo::node::{NodeId, TextSource};
    for (transparent, first_line) in [(false, false), (true, false), (false, true), (true, true)] {
        let sheet = format!(
            "div::first-letter{{font-size:50%;background:currentcolor;text-decoration:underline}} {}",
            if first_line {
                "div::first-line{font-size:20px}"
            } else {
                ""
            }
        );
        let nested = if transparent {
            "display:contents;font-size:200%;color:red"
        } else {
            "display:inline;font-size:200%;color:red;position:relative;left:3px"
        };
        let (mut doc, _, quote) = fixture(&sheet, Some(nested), "  “");
        let parent = doc.parent_of(quote).unwrap();
        let root = doc.parent_of(parent).unwrap();
        let letter = doc.append_text(root, "A");
        doc.append_text(root, "Y");
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 80.0;
        layout_single_page(&mut doc, &computed, page).unwrap();
        let positioned = raikiri_dom::PositionedLines::new(&doc, &computed, root, None).unwrap();
        let runs: Vec<_> = positioned
            .lines()
            .flat_map(|line| line.runs)
            .filter(|run| run.owner == quote || run.owner == letter)
            .collect();
        let selected_quote = runs
            .iter()
            .find(|run| run.owner == quote && run.style_owner != quote)
            .unwrap();
        assert_eq!(
            selected_quote.run.source(),
            Some(TextSource::Dom {
                node: NodeId(quote as u64),
                offset: 2
            })
        );
        let quote_size = if first_line { 20.0 } else { 10.0 };
        let letter_size = if first_line { 10.0 } else { 5.0 };
        assert_eq!(selected_quote.run.font_size(), quote_size);
        assert_eq!(
            selected_quote.offset,
            if transparent { (0.0, 0.0) } else { (3.0, 0.0) }
        );
        assert_eq!(
            selected_quote.style.color,
            raikiri_style::property::CssColor::from_hex("ff0000").unwrap()
        );
        let selected_letter = runs.iter().find(|run| run.owner == letter).unwrap();
        assert_eq!(selected_letter.run.font_size(), letter_size);
        assert_eq!(
            selected_letter.style.color,
            raikiri_style::property::CssColor::from_hex("000000").unwrap()
        );
        assert_eq!(
            selected_letter.run.source(),
            Some(TextSource::Dom {
                node: NodeId(letter as u64),
                offset: 0
            })
        );
        assert_eq!(selected_letter.offset, (0.0, 0.0));
        let pieces = doc.get_node(root).unwrap().ifc_inline_boxes().unwrap();
        let letter_id = raikiri_dom::generated_content::generated_node_id(
            root,
            raikiri_style::PseudoElem::FirstLetter,
        );
        let pseudo: Vec<_> = pieces
            .iter()
            .filter(|piece| piece.node == letter_id)
            .collect();
        assert_eq!(pseudo.len(), 2);
        for (piece, font, source) in [
            (pseudo[0], quote_size, quote),
            (pseudo[1], letter_size, letter),
        ] {
            let (style, owner) = doc
                .get_node(root)
                .unwrap()
                .ifc_typographic_fragment(piece.node, piece.source_container, piece.source_owner)
                .unwrap();
            assert_eq!(style.font_size.0, font);
            assert_eq!(owner, source);
        }
        let mut scene = Scene::new();
        raikiri_paint::paint_single_page(&mut scene, &doc, &computed, page).unwrap();
    }
}

#[test]
fn generated_first_letter_slices_match_independent_inline_paint_and_geometry() {
    use kurbo::Affine;
    use raikiri_dom::generated_content::generated_node_id;
    use raikiri_style::PseudoElem;
    let sheet = "div::before{content:'(';color:red;font-size:20px} span::before{content:'A';color:blue;font-size:30px} span::after{content:')Y';color:green;font-size:40px} div::first-letter{font-size:50%;background:currentcolor;border-bottom:1px solid currentcolor;text-decoration:underline}";
    let (mut actual, _, empty) = fixture(sheet, None, "");
    let root = actual.parent_of(empty).unwrap();
    let span = actual.append_element(Some(root), "span", Style::default(), Some("display:inline"));
    actual.append_text(root, "Z");
    actual.mark_in_document_flags();
    let computed = cascade(&actual, &build_rule_tree(&actual)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut actual, &computed, page).unwrap();
    let id = generated_node_id(root, PseudoElem::FirstLetter);
    let pieces = actual.get_node(root).unwrap().ifc_inline_boxes().unwrap();
    let pieces: Vec<_> = pieces.iter().filter(|piece| piece.node == id).collect();
    assert_eq!(pieces.len(), 3);
    for (piece, owner, size, x, width) in [
        (
            pieces[0],
            generated_node_id(root, PseudoElem::Before),
            10.0,
            0.0,
            10.0,
        ),
        (
            pieces[1],
            generated_node_id(span, PseudoElem::Before),
            15.0,
            10.0,
            15.0,
        ),
        (
            pieces[2],
            generated_node_id(span, PseudoElem::After),
            20.0,
            25.0,
            20.0,
        ),
    ] {
        assert_eq!(piece.source_owner, Some(owner));
        let (style, source) = actual
            .get_node(root)
            .unwrap()
            .ifc_typographic_fragment(id, piece.source_container, piece.source_owner)
            .unwrap();
        assert_eq!(source, owner);
        assert_eq!(style.font_size.0, size);
        assert_eq!(piece.content_box.x, x);
        assert_eq!(piece.content_box.width, width);
    }
    let (mut reference, _, empty) = fixture("", None, "");
    let root = reference.parent_of(empty).unwrap();
    for (text, size, color, decorated) in [
        ("(", 10, "red", true),
        ("A", 15, "blue", true),
        (")", 20, "green", true),
        ("Y", 40, "green", false),
        ("Z", 10, "black", false),
    ] {
        let css = format!(
            "display:inline;font-size:{size}px;color:{color};{}",
            if decorated {
                "background:currentcolor;border-bottom:1px solid currentcolor;text-decoration:underline"
            } else {
                ""
            }
        );
        let span =
            reference.append_element(Some(root), "span", Style::default(), Some(css.as_str()));
        reference.append_text(span, text);
    }
    reference.mark_in_document_flags();
    let reference_computed = cascade(&reference, &build_rule_tree(&reference)).unwrap();
    layout_single_page(&mut reference, &reference_computed, page).unwrap();
    let render = |doc: &Document, computed: &CascadeResult| {
        let mut scene = Scene::new();
        raikiri_paint::paint_single_page(&mut scene, doc, computed, page).unwrap();
        anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| {
                use anyrender::PaintScene;
                out.append_scene(scene, Affine::IDENTITY);
            },
            100,
            80,
        )
    };
    assert!(render(&actual, &computed) == render(&reference, &reference_computed));
}
