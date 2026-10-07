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
        let previous = if prefix.starts_with("text:") {
            doc.append_text(body, &prefix[5..])
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
