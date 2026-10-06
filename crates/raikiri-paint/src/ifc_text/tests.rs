use super::*;
use anyrender::Scene;
use anyrender::recording::RenderCommand;
use raikiri_dom::{Document, layout_single_page};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::PageBox;
use taffy::Style;

const FONT_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace"
);

/// `html > body > div` of Ahem text at 10px with a 10px line height. `css` is
/// appended to the div; `build` fills the div.
fn paragraph(
    css: &str,
    build: impl FnOnce(&mut Document, usize),
) -> (Document, raikiri_style::CascadeResult, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let style = format!("display:block;font-family:Ahem;font-size:10px;line-height:10px;{css}");
    let root = doc.append_element(Some(body), "div", Style::default(), Some(style.as_str()));
    build(&mut doc, root);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    (doc, cascade, root)
}

fn lay_out(doc: &mut Document, cascade: &raikiri_style::CascadeResult) {
    let dir = std::path::Path::new(FONT_DIR);
    doc.set_font_collection(raikiri_dom::build_wpt_font_collection(dir).expect("collection"));
    layout_single_page(doc, cascade, PageBox::A4).expect("layout");
}

#[test]
fn many_inline_pieces_on_separate_lines_paint_within_three_seconds() {
    use std::time::{Duration, Instant};

    let (mut doc, cascade, root) = paragraph("width:20px", |doc, root| {
        for _ in 0..12_000 {
            let span = doc.append_element(
                Some(root),
                "span",
                Style::default(),
                Some("display:inline;background-color:red"),
            );
            doc.append_text(span, "a");
            doc.append_element(Some(root), "br", Style::default(), None::<&str>);
        }
    });
    lay_out(&mut doc, &cascade);
    let node = doc.get_node(root).expect("root");
    assert!(node.ifc_lines().expect("lines").len() >= 12_000);
    assert!(node.ifc_inline_boxes().expect("pieces").len() >= 12_000);

    let mut scene = Scene::new();
    let started = Instant::now();
    draw_ifc_lines(
        &mut scene,
        &doc,
        &cascade,
        root,
        IfcPosition {
            x: 0.0,
            y: 0.0,
            shift_y: 0.0,
        },
        &crate::text::DecorationContext::default(),
        None,
        &[],
    );
    let elapsed = started.elapsed();
    assert!(!scene.commands.is_empty());
    assert!(
        elapsed < Duration::from_secs(3),
        "painting 12,000 separate inline pieces took {elapsed:?}"
    );
}

/// `(glyph id, absolute x, absolute y, brush)` of every recorded glyph.
fn glyphs(scene: &Scene) -> Vec<(u32, f64, f64, anyrender::Paint)> {
    let mut out = Vec::new();
    for command in &scene.commands {
        if let RenderCommand::GlyphRun(run) = command {
            let origin = run.transform.translation();
            for glyph in &run.glyphs {
                out.push((
                    glyph.id,
                    origin.x + f64::from(glyph.x),
                    origin.y + f64::from(glyph.y),
                    run.brush.clone(),
                ));
            }
        }
    }
    out
}

#[test]
fn glyph_runs_land_on_the_baseline_of_each_line() {
    let (mut doc, cascade, root) = paragraph("width:50px", |doc, root| {
        doc.append_text(root, "aaaa bbbb");
    });
    lay_out(&mut doc, &cascade);
    let mut scene = Scene::new();
    draw_ifc_lines(
        &mut scene,
        &doc,
        &cascade,
        root,
        IfcPosition {
            x: 100.0,
            y: 200.0,
            shift_y: 0.0,
        },
        &crate::text::DecorationContext::default(),
        None,
        &[],
    );
    let placed = glyphs(&scene);
    assert!(!placed.is_empty(), "the ifc painter emitted no glyphs");
    // Ahem at 10px: 0.8em ascent puts the first baseline 8px below the line
    // top, and the second line starts 10px lower. The first word is "aaaa".
    let first_line: Vec<_> = placed.iter().filter(|g| g.2 == 208.0).collect();
    assert!(first_line.len() >= 4);
    assert_eq!(
        first_line.iter().take(4).map(|g| g.1).collect::<Vec<_>>(),
        [100.0, 110.0, 120.0, 130.0]
    );
    assert!(placed.iter().any(|g| g.2 == 218.0), "second line baseline");
}

#[test]
fn vertical_ifc_glyph_origins_and_outline_axes_are_converted_to_physical_space() {
    use shodo::geometry::{PhysicalConverter, WritingMode};

    let (mut doc, cascade, root) = paragraph(
        "box-sizing:border-box;width:40px;height:30px;writing-mode:vertical-rl",
        |doc, root| {
            doc.append_text(root, "bb");
        },
    );
    lay_out(&mut doc, &cascade);
    let node = doc.get_node(root).expect("root");
    let lines = node.ifc_lines().expect("lines");
    let content_size = node.ifc_physical_content_size().expect("content size");
    assert_eq!(node.ifc_writing_mode(), Some(WritingMode::VerticalRl));

    let mut expected_positions = Vec::new();
    let mut expected_transforms = Vec::new();
    for line in lines {
        let converter =
            PhysicalConverter::new(WritingMode::VerticalRl, line.used_direction(), content_size);
        for fragment in line.fragments() {
            let Fragment::GlyphRun(run) = fragment else {
                continue;
            };
            for (index, glyph) in run.glyphs().enumerate() {
                let (inline, block) = run.glyph_origin(index).expect("glyph origin");
                let (x, y) = converter.point(inline, block + line.block_offset());
                expected_positions.push((glyph.id, 100.0 + f64::from(x), 200.0 + f64::from(y)));
            }
            let axes = run.glyph_transform();
            let (xx, yx) = converter.vector(axes.inline_x, axes.block_x);
            let (xy, yy) = converter.vector(axes.inline_y, axes.block_y);
            let orientation = Affine::new([
                f64::from(xx),
                f64::from(yx),
                f64::from(xy),
                f64::from(yy),
                0.0,
                0.0,
            ]);
            let skew = run.skew().map_or(Affine::IDENTITY, |degrees| {
                Affine::skew(f64::from(degrees).to_radians().tan(), 0.0)
            });
            let transform = orientation * skew;
            expected_transforms.push((transform != Affine::IDENTITY).then_some(transform));
        }
    }

    let mut scene = Scene::new();
    draw_ifc_lines(
        &mut scene,
        &doc,
        &cascade,
        root,
        IfcPosition {
            x: 100.0,
            y: 200.0,
            shift_y: 0.0,
        },
        &crate::text::DecorationContext::default(),
        None,
        &[],
    );
    let actual = glyphs(&scene)
        .into_iter()
        .map(|(id, x, y, _)| (id, x, y))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected_positions);
    let actual_transforms = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(run) => Some(run.glyph_transform),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(actual_transforms, expected_transforms);
}

#[test]
fn each_run_takes_the_color_of_its_text_node() {
    let (mut doc, cascade, root) = paragraph("color:blue", |doc, root| {
        doc.append_text(root, "aa ");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;color:red"),
        );
        doc.append_text(inner, "bb");
    });
    lay_out(&mut doc, &cascade);
    let mut scene = Scene::new();
    draw_ifc_lines(
        &mut scene,
        &doc,
        &cascade,
        root,
        IfcPosition {
            x: 0.0,
            y: 0.0,
            shift_y: 0.0,
        },
        &crate::text::DecorationContext::default(),
        None,
        &[],
    );
    let blue = anyrender::Paint::Solid(peniko::Color::from_rgba8(0, 0, 255, 255));
    let red = anyrender::Paint::Solid(peniko::Color::from_rgba8(255, 0, 0, 255));
    let brushes: Vec<_> = glyphs(&scene).into_iter().map(|g| g.3).collect();
    assert!(
        brushes.contains(&blue) && brushes.contains(&red),
        "{brushes:?}"
    );
}

fn painted(doc: &Document, cascade: &raikiri_style::CascadeResult) -> Scene {
    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, doc, cascade, PageBox::A4).expect("paint succeeds");
    scene
}

/// Ink glyphs only, in a canonical order, with positions rounded to 1/64px.
fn ink(scene: &Scene) -> Vec<(u32, i64, i64)> {
    let mut out: Vec<_> = glyphs(scene)
        .into_iter()
        .map(|g| {
            (
                g.0,
                (g.1 * 64.0).round() as i64,
                (g.2 * 64.0).round() as i64,
            )
        })
        .collect();
    out.sort_unstable();
    out
}

/// Paint the paragraph laid out by the inline engine.
fn painted_root(css: &str, build: impl Fn(&mut Document, usize)) -> Scene {
    let (mut doc, cascade, root) = paragraph(css, &build);
    lay_out(&mut doc, &cascade);
    assert!(
        doc.get_node(root).is_some_and(|n| n.is_ifc_root()),
        "{css}: the paragraph did not become an ifc root"
    );
    painted(&doc, &cascade)
}

#[test]
fn ifc_glyph_positions_are_pinned() {
    // `word-break: break-all` wraps a run of distinct letters without spaces,
    // so both engines emit the same ink glyphs and no trailing-space glyphs.
    // The alignment and indent cases pin that shodo's glyph positions already
    // include the line's offset.
    let cases = [
        ("width:50px;word-break:break-all", "abcdefghijklmno"),
        (
            "width:50px;word-break:break-all;text-align:center",
            "abcdefghijklmn",
        ),
        (
            "width:50px;word-break:break-all;text-align:right",
            "abcdefghijklmn",
        ),
        (
            "width:50px;word-break:break-all;text-indent:20px",
            "abcdefghijklmno",
        ),
    ];
    for (css, text) in cases {
        let on = painted_root(css, |doc, root| {
            doc.append_text(root, text);
        });
        assert!(!ink(&on).is_empty(), "{css}: nothing painted");
        let expected = match css {
            "width:50px;word-break:break-all" => &[
                (67, 0, 512),
                (68, 640, 512),
                (69, 1280, 512),
                (70, 1920, 512),
                (71, 2560, 512),
                (72, 0, 1152),
                (73, 640, 1152),
                (74, 1280, 1152),
                (75, 1920, 1152),
                (76, 2560, 1152),
                (77, 0, 1792),
                (78, 640, 1792),
                (79, 1280, 1792),
                (80, 1920, 1792),
                (81, 2560, 1792),
            ][..],
            "width:50px;word-break:break-all;text-align:center" => &[
                (67, 0, 512),
                (68, 640, 512),
                (69, 1280, 512),
                (70, 1920, 512),
                (71, 2560, 512),
                (72, 0, 1152),
                (73, 640, 1152),
                (74, 1280, 1152),
                (75, 1920, 1152),
                (76, 2560, 1152),
                (77, 320, 1792),
                (78, 960, 1792),
                (79, 1600, 1792),
                (80, 2240, 1792),
            ][..],
            "width:50px;word-break:break-all;text-align:right" => &[
                (67, 0, 512),
                (68, 640, 512),
                (69, 1280, 512),
                (70, 1920, 512),
                (71, 2560, 512),
                (72, 0, 1152),
                (73, 640, 1152),
                (74, 1280, 1152),
                (75, 1920, 1152),
                (76, 2560, 1152),
                (77, 640, 1792),
                (78, 1280, 1792),
                (79, 1920, 1792),
                (80, 2560, 1792),
            ][..],
            "width:50px;word-break:break-all;text-indent:20px" => &[
                (67, 1280, 512),
                (68, 1920, 512),
                (69, 2560, 512),
                (70, 0, 1152),
                (71, 640, 1152),
                (72, 1280, 1152),
                (73, 1920, 1152),
                (74, 2560, 1152),
                (75, 0, 1792),
                (76, 640, 1792),
                (77, 1280, 1792),
                (78, 1920, 1792),
                (79, 2560, 1792),
                (80, 0, 2432),
                (81, 640, 2432),
            ][..],
            _ => unreachable!("{css}"),
        };
        assert_eq!(ink(&on), expected, "{css}");
    }
}

/// Like [`ink`], with y rounded to whole pixels: the renderer rounds the y of
/// a hinted glyph itself.
fn ink_whole_pixel_y(scene: &Scene) -> Vec<(u32, i64, i64)> {
    let mut out: Vec<_> = glyphs(scene)
        .into_iter()
        .map(|g| (g.0, (g.1 * 64.0).round() as i64, g.2.round() as i64))
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn ifc_glyph_positions_at_the_default_size_match_to_the_pixel() {
    // At 16px with `line-height: normal` shodo keeps 1/64px metrics
    // (baseline 12.796875), not whole-pixel ones (baseline 13.0), so the
    // recorded y is 0.2px off a whole pixel. The renderer rounds a hinted glyph's y
    // to whole pixels, which hides it; compare y the same way and do not round
    // in the painter.
    let css = "width:80px;word-break:break-all;font-size:16px;line-height:normal";
    let on = painted_root(css, |doc, root| {
        doc.append_text(root, "abcdefghijklmno");
    });
    assert!(!ink(&on).is_empty());
    assert_eq!(
        ink_whole_pixel_y(&on),
        [
            (67, 0, 13),
            (68, 1024, 13),
            (69, 2048, 13),
            (70, 3072, 13),
            (71, 4096, 13),
            (72, 0, 29),
            (73, 1024, 29),
            (74, 2048, 29),
            (75, 3072, 29),
            (76, 4096, 29),
            (77, 0, 45),
            (78, 1024, 45),
            (79, 2048, 45),
            (80, 3072, 45),
            (81, 4096, 45)
        ]
    );
}

#[test]
fn a_body_root_takes_the_body_left_margin_like_its_text() {
    // `<body>` itself is the paragraph root: its lines start inside the
    // body's left margin, which layout carries as inline padding of the
    // synthetic body root.
    let build = || {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;margin-left:30px;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        doc.append_text(body, "abcde");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        assert!(doc.get_node(body).is_some_and(|n| n.is_ifc_root()));
        painted(&doc, &cascade)
    };
    let on = build();
    assert!(!ink(&on).is_empty());
    assert_eq!(
        ink(&on),
        [
            (67, 1920, 512),
            (68, 2560, 512),
            (69, 3200, 512),
            (70, 3840, 512),
            (71, 4480, 512)
        ]
    );
}

#[test]
fn a_paragraph_inside_a_fixed_box_is_painted_like_fixed_text() {
    // A fixed box repeats on every page, so text inside it is drawn even where
    // its laid-out box does not meet the page. The A4 page is about 1122px
    // tall; the box sits below it. A fixed block is not itself an ifc root
    // (see the eligibility rules), so the paragraph is one level down.
    let build = |doc: &mut Document, root: usize| {
        let inner = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;word-break:break-all"),
        );
        doc.append_text(inner, "abcde");
    };
    let css = "position:fixed;top:1200px;left:0;width:100px";

    let (mut on_doc, cascade, root) = paragraph(css, build);
    lay_out(&mut on_doc, &cascade);
    let inner = on_doc.get_node(root).map(|n| n.children[0]).expect("inner");
    assert!(on_doc.get_node(inner).is_some_and(|n| n.is_ifc_root()));
    let on = ink(&painted(&on_doc, &cascade));

    assert!(!on.is_empty(), "fixed text is drawn");
    assert_eq!(
        on,
        [
            (67, 0, 77312),
            (68, 640, 77312),
            (69, 1280, 77312),
            (70, 1920, 77312),
            (71, 2560, 77312)
        ]
    );
}

#[test]
fn ifc_text_is_painted_once() {
    let (mut doc, cascade, _) = paragraph("width:50px", |doc, root| {
        doc.append_text(root, "abcde");
    });
    lay_out(&mut doc, &cascade);
    let scene = painted(&doc, &cascade);
    let runs = scene
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .count();
    assert_eq!(runs, 1);
    assert_eq!(glyphs(&scene).len(), 5);
}

/// Decoration rectangles, rounded to 1/64px.
///
/// `paint_single_page` always fills the paper, so only thin rectangles
/// (decoration lines are at most a couple of pixels tall) are counted; the
/// paper fill would otherwise make an emptiness check vacuous.
fn decoration_fills(scene: &Scene) -> Vec<(i64, i64, i64, i64)> {
    use kurbo::Shape;
    let mut out = Vec::new();
    for command in &scene.commands {
        if let RenderCommand::Fill(fill) = command {
            let bounds = fill.shape.bounding_box();
            if bounds.height() > 3.0 {
                continue;
            }
            let r = |v: f64| (v * 64.0).round() as i64;
            out.push((r(bounds.x0), r(bounds.y0), r(bounds.x1), r(bounds.y1)));
        }
    }
    out.sort_unstable();
    out
}

#[test]
fn underline_and_line_through_are_pinned() {
    for css in ["text-decoration:underline", "text-decoration:line-through"] {
        let build = |doc: &mut Document, root: usize| {
            doc.append_text(root, "abcde");
        };

        let (mut on_doc, cascade, _) = paragraph(css, build);
        lay_out(&mut on_doc, &cascade);
        let on = decoration_fills(&painted(&on_doc, &cascade));

        assert!(!on.is_empty(), "{css}: no line drawn");
        let expected = match css {
            "text-decoration:underline" => &[(0, 544, 3200, 608)][..],
            "text-decoration:line-through" => &[(0, 301, 3200, 365)][..],
            _ => unreachable!("{css}"),
        };
        assert_eq!(on, expected, "{css}");
    }
}

#[test]
fn an_undecorated_paragraph_draws_no_decoration_rectangle() {
    let (mut doc, cascade, _) = paragraph("", |doc, root| {
        doc.append_text(root, "abcde");
    });
    lay_out(&mut doc, &cascade);
    assert!(decoration_fills(&painted(&doc, &cascade)).is_empty());
}

#[test]
fn a_decoration_reaches_text_inside_an_inline_element() {
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa ");
        let inner =
            doc.append_element(Some(root), "span", Style::default(), Some("display:inline"));
        doc.append_text(inner, "bb");
    };
    let css = "text-decoration:underline";
    let (mut on_doc, cascade, _) = paragraph(css, build);
    lay_out(&mut on_doc, &cascade);
    // One segment for "aa " and one for "bb": the line reaches the inline
    // element's text through the ancestor decoration context.
    assert_eq!(decoration_fills(&painted(&on_doc, &cascade)).len(), 2);
}

#[test]
fn a_decoration_originating_on_an_inline_element_covers_only_its_text() {
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa ");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;text-decoration:underline"),
        );
        doc.append_text(inner, "bb");
    };
    let (mut on_doc, cascade, _) = paragraph("", build);
    lay_out(&mut on_doc, &cascade);
    let on = decoration_fills(&painted(&on_doc, &cascade));

    assert_eq!(on.len(), 1, "only the span is underlined");
    assert_eq!(on, [(1920, 544, 3200, 608)]);
}

#[test]
fn a_trailing_empty_line_paints_no_glyphs() {
    let (mut doc, cascade, root) = paragraph("width:200px", |doc, root| {
        doc.append_text(root, "abcd");
        doc.append_element(Some(root), "br", Style::default(), Some("display:inline"));
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let scene = painted(&doc, &cascade);
    // The empty second line draws nothing and does not panic.
    assert_eq!(glyphs(&scene).len(), 4);
}

#[test]
fn a_float_inside_the_paragraph_is_painted_as_a_box() {
    let css = "width:100px";
    let (mut doc, cascade, root) = paragraph(css, |doc, root| {
        doc.append_text(root, "aa");
        doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;float:left;width:30px;height:20px;background-color:red"),
        );
        doc.append_text(root, " bbbb cccc");
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let scene = painted(&doc, &cascade);
    // The float's background is a 30x20 fill at the top left of the paragraph.
    let fills: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(kurbo::Shape::bounding_box(&fill.shape)),
            _ => None,
        })
        .collect();
    assert!(
        fills
            .iter()
            .any(|b| (b.x0, b.y0, b.x1, b.y1) == (0.0, 0.0, 30.0, 20.0)),
        "{fills:?}"
    );
    // The text starts after the float on the first line.
    assert!(glyphs(&scene).iter().any(|g| g.1 == 30.0));
}

#[test]
fn a_float_inside_the_paragraph_is_painted_once() {
    let (mut doc, cascade, _root) = paragraph("width:100px", |doc, root| {
        doc.append_text(root, "aa");
        doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;float:left;width:30px;height:20px;background-color:red"),
        );
        doc.append_text(root, " bb");
    });
    lay_out(&mut doc, &cascade);
    let scene = painted(&doc, &cascade);
    let red_boxes = scene
        .commands
        .iter()
        .filter(|command| {
            matches!(command, RenderCommand::Fill(fill)
            if {
                let b = kurbo::Shape::bounding_box(&fill.shape);
                (b.x1 - b.x0, b.y1 - b.y0) == (30.0, 20.0)
            })
        })
        .count();
    assert_eq!(red_boxes, 1);
}

#[test]
fn an_atomic_inline_is_painted_at_its_position_once() {
    let (mut doc, cascade, root) = paragraph("width:100px", |doc, root| {
        doc.append_text(root, "aa ");
        doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline-block;width:30px;height:10px;background-color:red"),
        );
        doc.append_text(root, " bb");
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let scene = painted(&doc, &cascade);
    let boxes: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(kurbo::Shape::bounding_box(&fill.shape)),
            _ => None,
        })
        .filter(|b| (b.x1 - b.x0, b.y1 - b.y0) == (30.0, 10.0))
        .collect();
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    assert_eq!((boxes[0].x0, boxes[0].y0), (30.0, 0.0));
}

#[test]
fn ch_lengths_place_glyphs() {
    // Ahem at 10px: one ch is 10px, so the lengths below are whole pixels.
    let cases = [
        ("width:200px;letter-spacing:1ch", "abc"),
        ("width:200px;word-spacing:2ch", "a b c"),
        // An indent in `ch`, as in the px indent case above.
        (
            "width:50px;word-break:break-all;text-indent:2ch",
            "abcdefghijklmno",
        ),
        (
            "width:50px;word-break:break-all;text-indent:calc(1ch + 20%)",
            "abcdefghijklmno",
        ),
        ("width:200px;letter-spacing:calc(1ch - 2px)", "abc"),
    ];
    // Ahem's space glyph has no outline. The two engines put the spacing on
    // different sides of it, so only the glyphs that draw are compared.
    let drawn = |scene: &Scene| {
        ink(scene)
            .into_iter()
            .filter(|glyph| glyph.0 != AHEM_SPACE_GLYPH)
            .collect::<Vec<_>>()
    };
    for (css, text) in cases {
        let on = painted_root(css, |doc, root| {
            doc.append_text(root, text);
        });
        assert!(!drawn(&on).is_empty(), "{css}");
        let expected = match css {
            "width:200px;letter-spacing:1ch" => {
                &[(67, 0, 512), (68, 1280, 512), (69, 2560, 512)][..]
            }
            "width:200px;word-spacing:2ch" => &[(67, 0, 512), (68, 2560, 512), (69, 5120, 512)][..],
            "width:50px;word-break:break-all;text-indent:2ch" => &[
                (67, 1280, 512),
                (68, 1920, 512),
                (69, 2560, 512),
                (70, 0, 1152),
                (71, 640, 1152),
                (72, 1280, 1152),
                (73, 1920, 1152),
                (74, 2560, 1152),
                (75, 0, 1792),
                (76, 640, 1792),
                (77, 1280, 1792),
                (78, 1920, 1792),
                (79, 2560, 1792),
                (80, 0, 2432),
                (81, 640, 2432),
            ][..],
            "width:50px;word-break:break-all;text-indent:calc(1ch + 20%)" => &[
                (67, 1280, 512),
                (68, 1920, 512),
                (69, 2560, 512),
                (70, 0, 1152),
                (71, 640, 1152),
                (72, 1280, 1152),
                (73, 1920, 1152),
                (74, 2560, 1152),
                (75, 0, 1792),
                (76, 640, 1792),
                (77, 1280, 1792),
                (78, 1920, 1792),
                (79, 2560, 1792),
                (80, 0, 2432),
                (81, 640, 2432),
            ][..],
            "width:200px;letter-spacing:calc(1ch - 2px)" => {
                &[(67, 0, 512), (68, 1152, 512), (69, 2304, 512)][..]
            }
            _ => unreachable!("{css}"),
        };
        assert_eq!(drawn(&on), expected, "{css}");
    }
}

/// The glyph id of U+0020 in Ahem.
const AHEM_SPACE_GLYPH: u32 = 3;

#[test]
fn an_inherited_ch_spacing_places_glyphs() {
    // The span inherits `1ch` measured with the paragraph's 10px font, not
    // with its own 20px font.
    let on = painted_root("width:300px;letter-spacing:1ch", |doc, root| {
        doc.append_text(root, "ab");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;font-size:20px"),
        );
        doc.append_text(inner, "cd");
    });
    // Only the inline positions are compared: the baseline is unrelated to
    // `ch`.
    let x_of = |scene: &Scene| {
        ink(scene)
            .into_iter()
            .map(|glyph| (glyph.0, glyph.1))
            .collect::<Vec<_>>()
    };
    assert!(!x_of(&on).is_empty());
    assert_eq!(x_of(&on), [(67, 0), (68, 1280), (69, 2560), (70, 4480)]);
    // `cd` sits after `ab` and two 10px spacings: 40px, not 60px.
    assert_eq!(x_of(&on)[2].1, 40 * 64);
}

#[test]
fn break_word_breaks_a_long_word() {
    let on = painted_root("width:50px;word-break:break-word", |doc, root| {
        doc.append_text(root, "abcdefghij");
    });
    assert!(!ink(&on).is_empty());
    // Two lines of five letters each.
    assert!(ink(&on).iter().any(|glyph| glyph.2 > 10 * 64));
    assert_eq!(
        ink(&on),
        [
            (67, 0, 512),
            (68, 640, 512),
            (69, 1280, 512),
            (70, 1920, 512),
            (71, 2560, 512),
            (72, 0, 1152),
            (73, 640, 1152),
            (74, 1280, 1152),
            (75, 1920, 1152),
            (76, 2560, 1152)
        ]
    );
}

#[test]
fn text_emphasis_paints_nothing_extra() {
    let on = painted_root("width:100px;text-emphasis-style:dot", |doc, root| {
        doc.append_text(root, "abc");
    });
    assert!(!ink(&on).is_empty());
    assert_eq!(ink(&on), [(67, 0, 512), (68, 640, 512), (69, 1280, 512)]);
    assert_eq!(on.commands.len(), 2);
}

#[test]
fn tabs_and_spaces_place_glyphs_on_the_same_positions() {
    // tab-size 8 and Ahem's 10px space: a tab after `a` reaches the stop at
    // 80px, which seven spaces reach too. The positions must be identical.
    let place = |text: &'static str| {
        let (mut doc, cascade, _) = paragraph("white-space:pre;width:400px", |doc, root| {
            doc.append_text(root, text);
        });
        lay_out(&mut doc, &cascade);
        ink(&painted(&doc, &cascade))
    };
    let b_of = |ink: &Vec<(u32, i64, i64)>| ink.iter().map(|g| g.1).max().unwrap_or(0);
    assert_eq!(b_of(&place("a\tb")), 80 * 64);
    assert_eq!(b_of(&place("a       b")), 80 * 64);
}

#[test]
fn a_hanging_opening_bracket_sits_before_the_line_start() {
    // `(` is an opening mark at the start of the first line, so it hangs: one
    // em (10px) to the left of the content start, and `a` starts the line.
    let (mut doc, cascade, root) =
        paragraph("width:100px;hanging-punctuation:first", |doc, root| {
            doc.append_text(root, "(ab");
        });
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    // The paragraph's content box starts at x 0 (no UA margins here).
    let mut xs: Vec<i64> = ink(&painted(&doc, &cascade))
        .iter()
        .map(|g| g.1 / 64)
        .collect();
    xs.sort_unstable();
    assert_eq!(xs, vec![-10, 0, 10]);
}

#[test]
fn a_relative_inline_with_no_offset_paints() {
    // No space at the text boundary, so the glyphs are those of the letters
    // only.
    let on = painted_root("width:100px", |doc, root| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;position:relative;color:blue"),
        );
        doc.append_text(inner, "bb");
    });
    assert!(!glyphs(&on).is_empty());
    assert_eq!(
        ink(&on),
        [
            (67, 0, 512),
            (67, 640, 512),
            (68, 1280, 512),
            (68, 1920, 512)
        ]
    );
    let brushes = |scene: &Scene| {
        let mut out: Vec<String> = glyphs(scene)
            .into_iter()
            .map(|glyph| format!("{:?}", glyph.3))
            .collect();
        out.sort();
        out
    };
    assert_eq!(
        format!("{:?}", brushes(&on)),
        "[\"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", \"Solid(AlphaColor { components: [0.0, 0.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", \"Solid(AlphaColor { components: [0.0, 0.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"]"
    );
}

#[test]
fn a_relative_block_child_with_no_offset_is_painted_once() {
    let on = painted_root("width:100px", |doc, root| {
        doc.append_text(root, "aa");
        let block = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;position:relative;background-color:green"),
        );
        doc.append_text(block, "bb");
        doc.append_text(root, "cc");
    });
    assert_eq!(ink(&on).len(), 6);
    assert_eq!(
        ink(&on),
        [
            (67, 0, 512),
            (67, 640, 512),
            (68, 0, 1152),
            (68, 640, 1152),
            (69, 0, 1792),
            (69, 640, 1792)
        ]
    );
    let fills = |scene: &Scene| {
        scene
            .commands
            .iter()
            .filter(|command| matches!(command, RenderCommand::Fill(_)))
            .count()
    };
    assert_eq!(fills(&on), 2);
}

/// Every glyph run as (glyph id, x·64, y·64, brush), in command order.
fn runs_in_order(scene: &Scene) -> Vec<(u32, i64, i64, String)> {
    glyphs(scene)
        .into_iter()
        .map(|g| {
            (
                g.0,
                (g.1 * 64.0).round() as i64,
                (g.2 * 64.0).round() as i64,
                format!("{:?}", g.3),
            )
        })
        .collect()
}

#[test]
fn text_shadows_are_painted_before_the_glyphs_in_reverse_order() {
    let on = painted_root(
        "width:100px;text-shadow:2px 3px red, 4px 1px blue;color:black",
        |doc, root| {
            doc.append_text(root, "ab");
        },
    );
    assert!(runs_in_order(&on).len() >= 6, "two shadows and the text");
    assert_eq!(
        format!("{:?}", runs_in_order(&on)),
        "[(67, 256, 576, \"Solid(AlphaColor { components: [0.0, 0.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 896, 576, \"Solid(AlphaColor { components: [0.0, 0.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (67, 128, 704, \"Solid(AlphaColor { components: [1.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 768, 704, \"Solid(AlphaColor { components: [1.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (67, 0, 512, \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 640, 512, \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\")]"
    );
}

#[test]
fn a_blurred_shadow_is_drawn_inside_a_filter_layer() {
    let layers = |scene: &Scene| {
        scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::PushLayer(_)))
            .count()
    };
    let on = painted_root("width:100px;text-shadow:1px 1px 3px red", |doc, root| {
        doc.append_text(root, "ab");
    });
    assert!(layers(&on) >= 1);
    assert_eq!(layers(&on), 1);
    assert_eq!(
        format!("{:?}", runs_in_order(&on)),
        "[(67, 64, 576, \"Solid(AlphaColor { components: [1.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 704, 576, \"Solid(AlphaColor { components: [1.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (67, 0, 512, \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 640, 512, \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\")]"
    );
}

#[test]
fn a_shadow_of_an_inline_element_uses_that_elements_color() {
    let on = painted_root("width:100px;text-shadow:1px 1px", |doc, root| {
        doc.append_text(root, "a");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;color:blue"),
        );
        doc.append_text(inner, "b");
    });
    // `currentcolor` shadows take the color of the text they belong to.
    assert_eq!(runs_in_order(&on).len(), 4, "a shadow and a glyph each");
    assert_eq!(
        format!("{:?}", runs_in_order(&on)),
        "[(67, 64, 576, \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (67, 0, 512, \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 704, 576, \"Solid(AlphaColor { components: [0.0, 0.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 640, 512, \"Solid(AlphaColor { components: [0.0, 0.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\")]"
    );
}

/// Whole-pixel x of every drawn glyph, sorted. Ahem's space glyph has no
/// outline and is left out: at the end of a wrapped right-to-left line the
/// inline engine hangs it at the line's left end, past the content.
fn sorted_xs(scene: &Scene) -> Vec<i64> {
    let mut xs: Vec<i64> = ink(scene)
        .iter()
        .filter(|g| g.0 != AHEM_SPACE_GLYPH)
        .map(|g| g.1 / 64)
        .collect();
    xs.sort_unstable();
    xs
}

#[test]
fn rtl_lines_place_glyphs() {
    let cases: [(&str, &str, &[i64]); 7] = [
        ("direction:rtl;width:100px", "abc", &[70, 80, 90]),
        (
            "direction:rtl;width:50px",
            "abc def",
            &[20, 20, 30, 30, 40, 40],
        ),
        (
            "direction:rtl;text-align:left;width:100px",
            "abc",
            &[0, 10, 20],
        ),
        (
            "direction:rtl;width:100px;text-align:center",
            "abc",
            &[35, 45, 55],
        ),
        // Hebrew letters have no glyph in Ahem, so every one draws the same
        // notdef box: the positions of the boxes are compared, not the order
        // of the letters inside the right-to-left run.
        (
            "width:100px",
            "abc \u{05d0}\u{05d1}\u{05d2}",
            &[0, 10, 20, 40, 50, 60],
        ),
        (
            "direction:rtl;width:100px",
            "\u{05d0}\u{05d1} abc",
            &[40, 50, 60, 80, 90],
        ),
        // The content box does not start at the page's left edge, and its
        // width is not authored.
        (
            "direction:rtl;margin:0 30px",
            "abc def",
            &[693, 703, 713, 733, 743, 753],
        ),
    ];
    for (css, text, expected) in cases {
        let on = painted_root(css, |doc, root| {
            doc.append_text(root, text);
        });
        assert!(!ink(&on).is_empty(), "{css}");
        assert_eq!(sorted_xs(&on), expected, "{css}");
    }
}

#[test]
fn an_rtl_underline_spans_its_text() {
    let on = painted_root(
        "direction:rtl;width:100px;text-decoration:underline",
        |doc, root| {
            doc.append_text(root, "abc");
        },
    );
    assert!(!decoration_fills(&on).is_empty());
    assert_eq!(decoration_fills(&on), [(4480, 544, 6400, 608)]);
}

#[test]
fn an_rtl_text_indent_is_taken_from_the_right_edge() {
    // CSS Text 3 §7.1: the indent is at the start side, the right in a
    // right-to-left line. The start is at 100 - 20 = 80, so the three 10px
    // letters end there: a 50, b 60, c 70 (hand-computed).
    let (mut doc, cascade, root) =
        paragraph("direction:rtl;width:100px;text-indent:20px", |doc, root| {
            doc.append_text(root, "abc");
        });
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    assert_eq!(sorted_xs(&painted(&doc, &cascade)), vec![50, 60, 70]);
}

/// `html > body > div(width:100px) > [empty float div, root div]`, Ahem 10px.
fn beside_float(
    float_css: &str,
    root_css: &str,
    text: &'static str,
) -> (Document, raikiri_style::CascadeResult, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let wrapper = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:100px;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some(format!("display:block;{float_css}").as_str()),
    );
    let root = doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some(format!("display:block;{root_css}").as_str()),
    );
    doc.append_text(root, text);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    (doc, cascade, root)
}

#[test]
fn an_rtl_paragraph_beside_a_left_float_ends_at_the_right_edge() {
    let (mut doc, cascade, root) =
        beside_float("float:left;width:30px;height:10px", "direction:rtl", "ab");
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    // The line spans 30..100 and the two glyphs end at the right edge.
    assert_eq!(sorted_xs(&painted(&doc, &cascade)), vec![80, 90]);
}

#[test]
fn an_rtl_paragraph_beside_a_right_float_starts_before_it() {
    let (mut doc, cascade, root) =
        beside_float("float:right;width:30px;height:10px", "direction:rtl", "ab");
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    // The line spans 0..70 and the glyphs end at the float's left edge.
    assert_eq!(sorted_xs(&painted(&doc, &cascade)), vec![50, 60]);
}

#[test]
fn an_ltr_paragraph_beside_a_left_float_is_unchanged() {
    let (mut doc, cascade, root) =
        beside_float("float:left;width:30px;height:10px", "direction:ltr", "ab");
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    assert_eq!(sorted_xs(&painted(&doc, &cascade)), vec![30, 40]);
}

#[test]
fn an_rtl_line_ends_at_the_right_edge_of_the_content_box() {
    // Padding 5px left and 15px right around a 100px content box: the content
    // box spans 5..105, so `abc` ends at 105 (a 75, b 85, c 95)
    // (hand-computed).
    let (mut doc, cascade, root) = paragraph(
        "direction:rtl;padding:0 15px 0 5px;width:100px",
        |doc, root| {
            doc.append_text(root, "abc");
        },
    );
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    assert_eq!(sorted_xs(&painted(&doc, &cascade)), vec![75, 85, 95]);
}

#[test]
fn a_wrapped_underline_leaves_out_the_break_space() {
    // "aaaa bbbb" at 50px wraps after the space. The underline of the first
    // line ends at the last letter, not after the space. The indent is not
    // part of the line's content width either.
    for css in [
        "width:50px;text-decoration:underline",
        "width:60px;text-indent:10px;text-decoration:underline",
    ] {
        let on = painted_root(css, |doc, root| {
            doc.append_text(root, "aaaa bbbb");
        });
        assert!(decoration_fills(&on).len() >= 2, "{css}: one line each");
        let expected = match css {
            "width:50px;text-decoration:underline" => {
                &[(0, 544, 2560, 608), (0, 1184, 2560, 1248)][..]
            }
            "width:60px;text-indent:10px;text-decoration:underline" => {
                &[(0, 1184, 2560, 1248), (640, 544, 3200, 608)][..]
            }
            _ => unreachable!("{css}"),
        };
        assert_eq!(decoration_fills(&on), expected, "{css}");
    }
}

#[test]
fn an_unwrapped_underline_keeps_its_full_extent() {
    let on = painted_root("width:200px;text-decoration:underline", |doc, root| {
        doc.append_text(root, "aaaa bbbb");
    });
    assert!(!decoration_fills(&on).is_empty());
    assert_eq!(decoration_fills(&on), [(0, 544, 5760, 608)]);
}

#[test]
fn a_rtl_wrapped_underline_leaves_out_the_break_space_too() {
    let on = painted_root(
        "direction:rtl;width:50px;text-decoration:underline",
        |doc, root| {
            doc.append_text(root, "aaaa bbbb");
        },
    );
    assert!(decoration_fills(&on).len() >= 2, "one line each");
    assert_eq!(
        decoration_fills(&on),
        [(640, 544, 3200, 608), (640, 1184, 3200, 1248)]
    );
}

#[test]
fn an_underline_under_a_hanging_bracket_covers_the_bracket_and_the_content() {
    // `(` hangs one em (10px) before the line start, so the glyphs are at
    // -10, 0, 10. The line's content width leaves the hung bracket out (it is
    // 20px), so the extent has to add the hang back: -10 .. 20
    // (hand-computed).
    let (mut doc, cascade, _) = paragraph(
        "width:100px;hanging-punctuation:first;text-decoration:underline",
        |doc, root| {
            doc.append_text(root, "(ab");
        },
    );
    lay_out(&mut doc, &cascade);
    let fills = decoration_fills(&painted(&doc, &cascade));
    assert_eq!(fills.len(), 1);
    assert_eq!((fills[0].0, fills[0].2), (-10 * 64, 20 * 64));
}

/// `aa` then a span `bb` with `vertical-align: {align}`. No space at the text
/// boundary, so a decoration covers the letters only.
fn raised(doc: &mut Document, root: usize, align: &str, css: &str) {
    doc.append_text(root, "aa");
    let inner = doc.append_element(
        Some(root),
        "span",
        Style::default(),
        Some(format!("display:inline;vertical-align:{align};{css}").as_str()),
    );
    doc.append_text(inner, "bb");
}

#[test]
fn an_outer_underline_stays_at_the_parents_baseline_under_a_raised_inline() {
    // Lengths and percentages raise by the same amount on both paths.
    for align in ["4px", "-3px", "50%"] {
        let on = painted_root("width:100px;text-decoration:underline", |doc, root| {
            raised(doc, root, align, "");
        });
        assert!(!decoration_fills(&on).is_empty(), "{align}");
        let expected = match align {
            "4px" => &[(0, 800, 1280, 864), (1280, 800, 2560, 864)][..],
            "-3px" => &[(0, 544, 1280, 608), (1280, 544, 2560, 608)][..],
            "50%" => &[(0, 864, 1280, 928), (1280, 864, 2560, 928)][..],
            _ => unreachable!("{align}"),
        };
        assert_eq!(decoration_fills(&on), expected, "{align}");
    }
}

#[test]
fn an_inner_underline_follows_the_raised_inline() {
    for align in ["4px", "-3px"] {
        let on = painted_root("width:100px", |doc, root| {
            raised(doc, root, align, "text-decoration:underline");
        });
        assert!(!decoration_fills(&on).is_empty(), "{align}");
        let expected = match align {
            "4px" => &[(1280, 544, 2560, 608)][..],
            "-3px" => &[(1280, 736, 2560, 800)][..],
            _ => unreachable!("{align}"),
        };
        assert_eq!(decoration_fills(&on), expected, "{align}");
    }
}

#[test]
fn a_line_through_follows_the_same_rule() {
    let on = painted_root("width:100px;text-decoration:line-through", |doc, root| {
        raised(doc, root, "4px", "");
    });
    assert!(!decoration_fills(&on).is_empty());
    assert_eq!(
        decoration_fills(&on),
        [(0, 557, 1280, 621), (1280, 557, 2560, 621)]
    );
}

/// y0 (in 1/64 px) of the decoration rectangles that start at `x` (in px),
/// lowest first.
fn fills_starting_at(scene: &Scene, x: i64) -> Vec<i64> {
    let mut ys: Vec<i64> = decoration_fills(scene)
        .into_iter()
        .filter(|fill| fill.0 == x * 64)
        .map(|fill| fill.1)
        .collect();
    ys.sort_unstable();
    ys
}

#[test]
fn super_and_sub_raise_by_the_fonts_offsets() {
    // Both the root and the span carry an underline, so the two rectangles that
    // start at the span's text (x = 20: `aa` is 2 glyphs of 10px) differ only by
    // how far the span is raised: the root's underline sits at the parent's
    // baseline and the span's at the span's own. shodo raises by the font's
    // OS/2 offsets: Ahem has 453/1000 em for `super` and 143/1000 em for
    // `sub`, i.e. 4.53px and 1.43px at 10px. Each value is rounded to 1/64 px
    // once, so the difference can be 1/64 px off the exact product.
    for (align, expected) in [("super", -4.53_f64), ("sub", 1.43)] {
        let (mut doc, cascade, root) =
            paragraph("width:100px;text-decoration:underline", |doc, root| {
                raised(doc, root, align, "text-decoration:underline")
            });
        lay_out(&mut doc, &cascade);
        assert!(
            doc.get_node(root).is_some_and(|n| n.is_ifc_root()),
            "{align}"
        );
        let ys = fills_starting_at(&painted(&doc, &cascade), 20);
        assert_eq!(ys.len(), 2, "{align}: the outer and the inner underline");
        let delta = (ys[1] - ys[0]) as f64 / 64.0;
        assert!(
            (delta - expected.abs()).abs() <= 1.0 / 64.0 + 1e-9,
            "{align}: {delta} vs {}",
            expected.abs()
        );
    }
}

// ── boxes of inline elements ─────────────────────────────────

fn solid(r: u8, g: u8, b: u8) -> anyrender::Paint {
    anyrender::Paint::Solid(peniko::Color::from_rgba8(r, g, b, 255))
}

/// Bounding boxes of the fills painted with `brush`, rounded to 1/64px, in
/// paint order.
fn fills_with(scene: &Scene, brush: &anyrender::Paint) -> Vec<[i64; 4]> {
    use kurbo::Shape;
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) if &fill.brush == brush => {
                let b = fill.shape.bounding_box();
                let r = |v: f64| (v * 64.0).round() as i64;
                Some([r(b.x0), r(b.y0), r(b.x1), r(b.y1)])
            }
            _ => None,
        })
        .collect()
}

/// Every fill as (brush, rounded bounding box), in paint order.
fn all_fills(scene: &Scene) -> Vec<(String, [i64; 4])> {
    use kurbo::Shape;
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => {
                let b = fill.shape.bounding_box();
                let r = |v: f64| (v * 64.0).round() as i64;
                Some((
                    format!("{:?}", fill.brush),
                    [r(b.x0), r(b.y0), r(b.x1), r(b.y1)],
                ))
            }
            _ => None,
        })
        .collect()
}

/// In 1/64px, so expectations read like pixels.
fn px(v: i64) -> i64 {
    v * 64
}

#[test]
fn inline_background_pieces_keep_source_order_within_and_across_lines() {
    let colors = ["red", "blue", "green"];
    let (mut doc, cascade, root) = paragraph("width:80px", |doc, root| {
        for (index, color) in colors.into_iter().enumerate() {
            if index == 2 {
                doc.append_element(Some(root), "br", Style::default(), None::<&str>);
            }
            let style = format!("display:inline;background-color:{color}");
            let span = doc.append_element(Some(root), "span", Style::default(), Some(&style));
            doc.append_text(span, "aa");
        }
    });
    lay_out(&mut doc, &cascade);
    assert!(
        doc.get_node(root)
            .and_then(|node| node.ifc_lines())
            .expect("lines")
            .len()
            >= 2
    );

    let scene = painted(&doc, &cascade);
    let expected = [
        format!("{:?}", solid(255, 0, 0)),
        format!("{:?}", solid(0, 0, 255)),
        format!("{:?}", solid(0, 128, 0)),
    ];
    let mut drawn: Vec<_> = all_fills(&scene)
        .into_iter()
        .map(|(brush, _)| brush)
        .filter(|brush| expected.contains(brush))
        .collect();
    drawn.dedup();
    assert_eq!(drawn, expected);
}

const BOX_EDGES: &str = "background-color:rgb(255,0,0);padding:0 3px;\
border-width:0 2px;border-style:solid;border-color:rgb(0,0,255);margin:0 4px";

#[test]
fn an_inline_background_and_border_are_pinned() {
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(format!("display:inline;{BOX_EDGES}").as_str()),
        );
        doc.append_text(inner, "bb");
        doc.append_text(root, "cc");
    };
    let on = painted_root("width:200px", build);
    // After "aa" and the 4px margin: border box x 24..54, y 0..10; borders
    // 2px wide.
    assert_eq!(
        fills_with(&on, &solid(255, 0, 0)),
        [[px(24), 0, px(54), px(10)]]
    );
    assert_eq!(
        fills_with(&on, &solid(0, 0, 255)),
        [[px(24), 0, px(26), px(10)], [px(52), 0, px(54), px(10)]]
    );
    assert_eq!(
        format!("{:?}", all_fills(&on)),
        "[(\"Solid(AlphaColor { components: [1.0, 1.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [0, 0, 50797, 71841]), (\"Solid(AlphaColor { components: [1.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [1536, 0, 3456, 640]), (\"Solid(AlphaColor { components: [0.0, 0.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [1536, 0, 1664, 640]), (\"Solid(AlphaColor { components: [0.0, 0.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [3328, 0, 3456, 640])]"
    );
}

#[test]
fn a_wrapping_inline_keeps_the_start_border_on_its_first_piece_only() {
    let (mut doc, cascade, _root) = paragraph("width:60px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(format!("display:inline;{BOX_EDGES}").as_str()),
        );
        doc.append_text(inner, "aaaa bbbb");
    });
    lay_out(&mut doc, &cascade);
    let scene = painted(&doc, &cascade);
    // The span wraps after "aaaa ": line 1 piece x 4..49 (y 0..10), line 2
    // piece x 0..45 (y 10..20).
    assert_eq!(
        fills_with(&scene, &solid(255, 0, 0)),
        [[px(4), 0, px(49), px(10)], [0, px(10), px(45), px(20)]]
    );
    // Line 1 keeps the left border, line 2 the right one.
    assert_eq!(
        fills_with(&scene, &solid(0, 0, 255)),
        [[px(4), 0, px(6), px(10)], [px(43), px(10), px(45), px(20)]]
    );
}

#[test]
fn a_wrapping_inline_has_square_corners_where_it_continues() {
    // With a radius, line 1's piece is rounded on the left only: its right
    // edge runs straight from the top to the bottom.
    let (mut doc, cascade, _root) = paragraph("width:60px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(format!("display:inline;{BOX_EDGES};border-radius:4px").as_str()),
        );
        doc.append_text(inner, "aaaa bbbb");
    });
    lay_out(&mut doc, &cascade);
    let scene = painted(&doc, &cascade);
    let backgrounds: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) if fill.brush == solid(255, 0, 0) => Some(&fill.shape),
            _ => None,
        })
        .collect();
    assert_eq!(backgrounds.len(), 2);
    let corner_inside = |shape: &kurbo::BezPath, x: f64, y: f64| {
        use kurbo::Shape;
        shape.contains(kurbo::Point::new(x, y))
    };
    // Line 1 (x 4..49, y 0..10): the top-right corner point is inside the
    // background, the top-left one is cut by the radius.
    assert!(corner_inside(backgrounds[0], 48.9, 0.1));
    assert!(!corner_inside(backgrounds[0], 4.1, 0.1));
    // Line 2 (x 0..45, y 10..20): the other way round.
    assert!(corner_inside(backgrounds[1], 0.1, 10.1));
    assert!(!corner_inside(backgrounds[1], 44.9, 10.1));
}

#[test]
fn vertical_edges_grow_the_painted_box_but_not_the_line() {
    let (mut doc, cascade, _root) = paragraph("width:200px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(
                "display:inline;background-color:rgb(255,0,0);padding:1px 0;\
                 border-width:2px 0;border-style:solid;border-color:rgb(0,0,255)",
            ),
        );
        doc.append_text(inner, "bb");
    });
    lay_out(&mut doc, &cascade);
    let scene = painted(&doc, &cascade);
    // 1px padding and 2px border above and below: border box x 0..20,
    // y -3..13; the line stays 10px tall.
    assert_eq!(
        fills_with(&scene, &solid(255, 0, 0)),
        [[0, px(-3), px(20), px(13)]]
    );
    assert_eq!(
        fills_with(&scene, &solid(0, 0, 255)),
        [[0, px(-3), px(20), px(-1)], [0, px(11), px(20), px(13)]]
    );
}

#[test]
fn an_inline_box_shadow_is_pinned() {
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;padding:0 3px;box-shadow:2px 2px rgb(0,255,0)"),
        );
        doc.append_text(inner, "bb");
    };
    let on = painted_root("width:200px", build);
    // The span: x 20..46 (padding 3 + "bb" 20 + padding 3), y 0..10.
    assert_eq!(
        box_shadows(&on),
        [([px(22), px(2), px(48), px(12)], solid(0, 255, 0))],
        "a shadow is painted"
    );
    assert_eq!(
        format!("{:?}", box_shadows(&on)),
        "[([1408, 128, 3072, 768], Solid(AlphaColor { components: [0.0, 1.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> }))]"
    );
    assert_eq!(
        format!("{:?}", all_fills(&on)),
        "[(\"Solid(AlphaColor { components: [1.0, 1.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [0, 0, 50797, 71841])]"
    );
}

/// Every box shadow as (rounded rectangle, colour), in paint order.
fn box_shadows(scene: &Scene) -> Vec<([i64; 4], anyrender::Paint)> {
    box_shadows_with_radius(scene)
        .into_iter()
        .map(|(rect, brush, _)| (rect, brush))
        .collect()
}

/// Every box shadow as (rounded rectangle, colour, corner radius).
fn box_shadows_with_radius(scene: &Scene) -> Vec<([i64; 4], anyrender::Paint, i64)> {
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::BoxShadow(shadow) => {
                let b = shadow.transform.transform_rect_bbox(shadow.rect);
                let r = |v: f64| (v * 64.0).round() as i64;
                Some((
                    [r(b.x0), r(b.y0), r(b.x1), r(b.y1)],
                    anyrender::Paint::Solid(shadow.brush),
                    r(shadow.radius),
                ))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn an_inline_outline_is_pinned() {
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;padding:0 3px;outline:2px solid rgb(0,255,0)"),
        );
        doc.append_text(inner, "bb");
    };
    let on = painted_root("width:200px", build);
    assert!(
        !fills_with(&on, &solid(0, 255, 0)).is_empty(),
        "an outline is painted"
    );
    assert_eq!(
        format!("{:?}", all_fills(&on)),
        "[(\"Solid(AlphaColor { components: [1.0, 1.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [0, 0, 50797, 71841]), (\"Solid(AlphaColor { components: [0.0, 1.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [1152, -128, 3072, 0]), (\"Solid(AlphaColor { components: [0.0, 1.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [1152, 640, 3072, 768]), (\"Solid(AlphaColor { components: [0.0, 1.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [1152, 0, 1280, 640]), (\"Solid(AlphaColor { components: [0.0, 1.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [2944, 0, 3072, 640])]"
    );
}

#[test]
fn the_start_border_follows_the_elements_own_direction() {
    // An ltr span in an rtl paragraph still has its start (left) border on
    // the physical left of the piece that carries the start edge.
    let (mut doc, cascade, root) = paragraph("width:60px;direction:rtl", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(
                "display:inline;direction:ltr;padding:0 3px;border-width:0 2px;\
                 border-style:solid;border-color:rgb(0,0,255)",
            ),
        );
        doc.append_text(inner, "aaaa bbbb");
    });
    lay_out(&mut doc, &cascade);
    let node = doc.get_node(root).expect("root");
    assert!(node.is_ifc_root());
    let pieces = node.ifc_inline_boxes().expect("pieces");
    assert_eq!(pieces.len(), 2);
    let first = pieces
        .iter()
        .find(|p| p.has_start_edge)
        .expect("start piece");
    assert!(!first.has_end_edge);
    let scene = painted(&doc, &cascade);
    let blue = fills_with(&scene, &solid(0, 0, 255));
    // The piece with the start edge carries a left border only: a 2px strip
    // whose left edge is the piece's left edge.
    let left = (first.border_box.x * 64.0).round() as i64;
    let top = (first.border_box.y * 64.0).round() as i64;
    let on_first_line: Vec<_> = blue.iter().filter(|b| b[1] == top).collect();
    assert_eq!(on_first_line.len(), 1, "{blue:?}");
    assert_eq!(on_first_line[0][0], left, "{blue:?}");
    assert_eq!(on_first_line[0][2] - on_first_line[0][0], px(2), "{blue:?}");
}

#[test]
fn an_inline_shadow_without_a_border_or_background_has_square_corners() {
    // The element visit drops the radius of a box that draws neither a border
    // nor a background; the shadow then has square corners on both paths.
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;border-radius:4px;box-shadow:2px 2px rgb(0,255,0)"),
        );
        doc.append_text(inner, "bb");
    };
    let on = painted_root("width:200px", build);
    let shadows = box_shadows_with_radius(&on);
    assert_eq!(shadows.len(), 1);
    assert_eq!(shadows[0].2, 0, "the corners are square");
    assert_eq!(
        format!("{:?}", box_shadows_with_radius(&on)),
        "[([1408, 128, 2688, 768], Solid(AlphaColor { components: [0.0, 1.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> }), 0)]"
    );
}

#[test]
fn an_inline_background_clipped_to_its_content_box_is_pinned() {
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(format!("display:inline;{BOX_EDGES};background-clip:content-box").as_str()),
        );
        doc.append_text(inner, "bb");
    };
    let on = painted_root("width:200px", build);
    // The span's content box: x 29..49.
    assert_eq!(
        fills_with(&on, &solid(255, 0, 0)),
        [[px(29), 0, px(49), px(10)]]
    );
    assert_eq!(
        format!("{:?}", all_fills(&on)),
        "[(\"Solid(AlphaColor { components: [1.0, 1.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [0, 0, 50797, 71841]), (\"Solid(AlphaColor { components: [1.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [1856, 0, 3136, 640]), (\"Solid(AlphaColor { components: [0.0, 0.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [1536, 0, 1664, 640]), (\"Solid(AlphaColor { components: [0.0, 0.0, 1.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\", [3328, 0, 3456, 640])]"
    );
}

#[test]
fn a_wrapped_piece_has_no_padding_where_it_continues() {
    // The wrapping span with the background clipped to the content box: line 1's piece has
    // its padding on the left only (content x 9..49), line 2's on the right
    // only (content x 0..40).
    let (mut doc, cascade, _root) = paragraph("width:60px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(format!("display:inline;{BOX_EDGES};background-clip:content-box").as_str()),
        );
        doc.append_text(inner, "aaaa bbbb");
    });
    lay_out(&mut doc, &cascade);
    let scene = painted(&doc, &cascade);
    assert_eq!(
        fills_with(&scene, &solid(255, 0, 0)),
        [[px(9), 0, px(49), px(10)], [0, px(10), px(40), px(20)]]
    );
}

#[test]
fn a_right_to_left_element_has_its_start_border_on_the_right() {
    // An rtl span in an ltr paragraph starts on its right side: the piece
    // that carries the start edge has its border on the physical right.
    let (mut doc, cascade, root) = paragraph("width:60px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(
                "display:inline;direction:rtl;padding:0 3px;border-width:0 2px;\
                 border-style:solid;border-color:rgb(0,0,255)",
            ),
        );
        doc.append_text(inner, "aaaa bbbb");
    });
    lay_out(&mut doc, &cascade);
    let node = doc.get_node(root).expect("root");
    assert!(node.is_ifc_root());
    let pieces = node.ifc_inline_boxes().expect("pieces");
    assert_eq!(pieces.len(), 2);
    let first = pieces
        .iter()
        .find(|p| p.has_start_edge)
        .expect("start piece");
    assert!(!first.has_end_edge);
    let scene = painted(&doc, &cascade);
    let blue = fills_with(&scene, &solid(0, 0, 255));
    let right = ((first.border_box.x + first.border_box.width) * 64.0).round() as i64;
    let top = (first.border_box.y * 64.0).round() as i64;
    let on_first_line: Vec<_> = blue.iter().filter(|b| b[1] == top).collect();
    assert_eq!(on_first_line.len(), 1, "{blue:?}");
    assert_eq!(on_first_line[0][2], right, "{blue:?}");
    assert_eq!(on_first_line[0][2] - on_first_line[0][0], px(2), "{blue:?}");
}

#[test]
fn a_content_box_background_leaves_out_the_vertical_padding() {
    // The vertically padded span with the background clipped to the content box: y 0..10, inside
    // the 1px padding and 2px border above and below.
    let (mut doc, cascade, _root) = paragraph("width:200px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(
                "display:inline;background-color:rgb(255,0,0);background-clip:content-box;\
                 padding:1px 0;border-width:2px 0;border-style:solid;border-color:rgb(0,0,255)",
            ),
        );
        doc.append_text(inner, "bb");
    });
    lay_out(&mut doc, &cascade);
    let scene = painted(&doc, &cascade);
    assert_eq!(
        fills_with(&scene, &solid(255, 0, 0)),
        [[0, 0, px(20), px(10)]]
    );
}

#[test]
fn a_relative_inline_is_painted() {
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(
                "display:inline;position:relative;left:5px;top:2px;\
                 background-color:rgb(255,0,0);text-decoration:underline",
            ),
        );
        doc.append_text(inner, "bb");
    };
    let on = painted_root("width:200px", build);
    assert_eq!(
        ink(&on),
        [
            (67, 0, 512),
            (67, 640, 512),
            (68, 1600, 640),
            (68, 2240, 640)
        ]
    );
    assert!(!decoration_fills(&on).is_empty());
    assert_eq!(decoration_fills(&on), [(1600, 672, 2880, 736)]);
    assert_eq!(
        fills_with(&on, &solid(255, 0, 0)),
        [[px(25), px(2), px(45), px(12)]]
    );
}

#[test]
fn a_text_shadow_moves_with_a_relative_inline() {
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;position:relative;left:5px;top:2px;text-shadow:1px 1px rgb(0,255,0)"),
        );
        doc.append_text(inner, "bb");
    };
    let on = painted_root("width:200px", build);
    assert_eq!(
        format!("{:?}", runs_in_order(&on)),
        "[(67, 0, 512, \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (67, 640, 512, \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 1664, 704, \"Solid(AlphaColor { components: [0.0, 1.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 2304, 704, \"Solid(AlphaColor { components: [0.0, 1.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 1600, 640, \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\"), (68, 2240, 640, \"Solid(AlphaColor { components: [0.0, 0.0, 0.0, 1.0], cs: PhantomData<color::colorspace::Srgb> })\")]"
    );
}

#[test]
fn the_dom_and_the_paint_crate_agree_on_the_relative_offset() {
    // `relative_offset` (layout) and the walk's `position_offset_px` (paint)
    // are two implementations of one rule; pin that they agree.
    for css in [
        "left:5px;top:2px",
        "right:4px;bottom:3px",
        "left:1px;right:9px;top:-2px;bottom:7px",
        "",
    ] {
        let (_doc, cascade, root) = paragraph(&format!("position:relative;{css}"), |doc, root| {
            doc.append_text(root, "x");
        });
        let cv = &cascade.computed[root];
        assert_eq!(
            raikiri_dom::relative_offset(cv),
            Some(crate::walk::position_offset_px(cv)),
            "{css}"
        );
    }
}

#[test]
fn a_contents_element_is_transparent_in_the_paragraph() {
    // Hand-computed: "aabbcc" on one line, 10px per glyph, on
    // the 8px baseline; "bb" takes the contents element's colour.
    let (mut doc, cascade, root) = paragraph("width:200px", |doc, root| {
        doc.append_text(root, "aa");
        let wrapper = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:contents;color:rgb(0,0,255)"),
        );
        doc.append_text(wrapper, "bb");
        doc.append_text(root, "cc");
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let scene = painted(&doc, &cascade);
    let placed: Vec<(i64, i64)> = ink(&scene).iter().map(|g| (g.1, g.2)).collect();
    assert_eq!(
        placed,
        (0..6).map(|i| (px(10 * i), px(8))).collect::<Vec<_>>()
    );
    let blue: Vec<i64> = glyphs(&scene)
        .into_iter()
        .filter(|g| g.3 == solid(0, 0, 255))
        .map(|g| (g.1 * 64.0).round() as i64)
        .collect();
    assert_eq!(blue, [px(20), px(30)]);
}

#[test]
fn a_block_child_of_a_paragraph_paints_its_text() {
    // The block is a box of the paragraph and the root of its own text: its
    // text is laid out and painted by the inline engine, once.
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa");
        let block = doc.append_element(Some(root), "div", Style::default(), Some("display:block"));
        doc.append_text(block, "bbbb");
    };
    let on = painted_root("", build);
    let (mut doc, cascade, root) = paragraph("", build);
    lay_out(&mut doc, &cascade);
    let block = doc.get_node(root).expect("root").children[1];
    assert!(doc.get_node(block).is_some_and(|n| n.is_ifc_root()));
    assert_eq!(
        ink(&on),
        [
            (67, 0, 512),
            (67, 640, 512),
            (68, 0, 1152),
            (68, 640, 1152),
            (68, 1280, 1152),
            (68, 1920, 1152)
        ]
    );
    // Hand-computed: "aa" on line 1 and "bbbb" in the block below it.
    assert_eq!(ink(&on).len(), 6);
}

#[test]
fn bare_text_in_a_flex_container_is_painted() {
    // "aaaa bbbb" directly in a 50px flex row: an anonymous item, laid out and
    // painted as a paragraph of its own.
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aaaa bbbb");
    };
    let (mut doc, cascade, root) = paragraph("display:flex;width:50px", build);
    lay_out(&mut doc, &cascade);
    let text = doc.get_node(root).expect("root").children[0];
    assert!(doc.get_node(text).is_some_and(|n| n.is_ifc_root()));
    let on = ink(&painted(&doc, &cascade));
    assert_eq!(
        on,
        [
            (3, 2560, 512),
            (67, 0, 512),
            (67, 640, 512),
            (67, 1280, 512),
            (67, 1920, 512),
            (68, 0, 1152),
            (68, 640, 1152),
            (68, 1280, 1152),
            (68, 1920, 1152)
        ]
    );
    // Hand-computed: "aaaa " and "bbbb" on two 10px lines (the space that
    // ends line 1 keeps its glyph on both paths), with the second line's
    // glyphs 10px below the first's.
    assert_eq!(on.len(), 9);
    let ys: std::collections::BTreeSet<i64> = on.iter().map(|g| g.2).collect();
    assert_eq!(ys.len(), 2);
    assert_eq!(
        ys.iter().last().unwrap() - ys.iter().next().unwrap(),
        10 * 64
    );
}

#[test]
fn a_raised_inline_block_root_is_painted_where_the_engine_placed_it() {
    // "aa" then an inline-block "bb" raised by `vertical-align:10px`. The
    // inline engine places the raised box itself (CSS 2.1 10.8.1): the box's
    // baseline (8px into it) sits 10px above the line's baseline, so the line
    // box grows to put the outer baseline at 18 and the box at the top. The
    // painter must not raise the box's text a second time.
    let (mut doc, cascade, root) = paragraph("width:200px", |doc, root| {
        doc.append_text(root, "aa");
        let block = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline-block;vertical-align:10px"),
        );
        doc.append_text(block, "bb");
    });
    lay_out(&mut doc, &cascade);
    let block = doc.get_node(root).expect("root").children[1];
    assert!(doc.get_node(block).is_some_and(|n| n.is_ifc_root()));
    let mut ys: Vec<i64> = ink(&painted(&doc, &cascade)).iter().map(|g| g.2).collect();
    ys.sort_unstable();
    assert_eq!(ys, [px(8), px(8), px(18), px(18)]);
}

#[test]
fn lines_overflowing_a_short_root_are_painted_on_a_later_page() {
    // A 10px-tall, 20px-wide root of 200 "aa" words: its lines overflow the
    // box (overflow is visible) down to y = 2000. Painted for the page that
    // starts at y = 1000, the line at y = 1000 has its baseline 8px below the
    // page top, although the root's box ends on the first page.
    let (mut doc, cascade, root) = paragraph("width:20px;height:10px", |doc, root| {
        doc.append_text(root, "aa ".repeat(200));
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let mut scene = Scene::new();
    let mut budget = raikiri_dom::CounterSnapshotBudget::default();
    crate::paint_single_page_with_origin(
        &mut scene,
        &doc,
        &cascade,
        PageBox::A4,
        1000.0,
        &mut budget,
    )
    .expect("paint succeeds");
    let ys: std::collections::BTreeSet<i64> = ink(&scene).iter().map(|g| g.2).collect();
    assert!(ys.contains(&px(8)), "{ys:?}");
}

#[test]
fn a_table_cell_root_paints_its_text_inside_its_border_and_padding() {
    // A cell with a 3px border and 5px of padding: its content box starts
    // 8px in, so "aa" starts at x = 8 with its baseline at 8 + 8.
    let build = |doc: &mut Document, table: usize| {
        let row = doc.append_element(
            Some(table),
            "div",
            Style::default(),
            Some("display:table-row"),
        );
        let cell = doc.append_element(
            Some(row),
            "div",
            Style::default(),
            Some("display:table-cell;padding:5px;border:3px solid"),
        );
        doc.append_text(cell, "aa");
    };
    let (mut doc, cascade, table) = paragraph("display:table", build);
    lay_out(&mut doc, &cascade);
    let row = doc.get_node(table).expect("table").children[0];
    let cell = doc.get_node(row).expect("row").children[0];
    assert!(doc.get_node(cell).is_some_and(|n| n.is_ifc_root()));
    let on = ink(&painted(&doc, &cascade));
    assert_eq!(
        on.iter().map(|g| (g.1, g.2)).collect::<Vec<_>>(),
        [(px(8), px(16)), (px(18), px(16))]
    );
    assert_eq!(on, [(67, 512, 1024), (67, 1152, 1024)]);
}

#[test]
fn a_block_child_under_a_decoration_is_underlined() {
    // The root's underline reaches the text of its block child, which is a
    // root of its own: one 20px underline under each line, 0.5px below the
    // baselines at 8 and 18.
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa");
        let block = doc.append_element(Some(root), "div", Style::default(), Some("display:block"));
        doc.append_text(block, "bb");
    };
    let on = painted_root("width:200px;text-decoration:underline", build);
    let px = |v: f64| (v * 64.0) as i64;
    assert_eq!(
        decoration_fills(&on),
        [
            (0, px(8.5), px(20.0), px(9.5)),
            (0, px(18.5), px(20.0), px(19.5))
        ]
    );
    assert_eq!(
        decoration_fills(&on),
        [(0, 544, 1280, 608), (0, 1184, 1280, 1248)]
    );
}

#[test]
fn a_propagated_underline_stays_on_its_box_across_a_line_relative_inline() {
    // CSS Text Decoration 3, 2.1: a decoration propagated from the root is
    // drawn at the root's position across all its text, including a
    // descendant aligned to the line's top.
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aa");
        let span = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;vertical-align:top;font-size:20px"),
        );
        doc.append_text(span, "bb");
    };
    let on = painted_root("width:200px;text-decoration:underline", build);
    let px = |v: f64| (v * 64.0) as i64;
    assert_eq!(
        decoration_fills(&on),
        [
            (0, px(8.5), px(20.0), px(9.5)),
            (px(20.0), px(8.5), px(60.0), px(9.5))
        ]
    );
    assert_eq!(
        ink(&on),
        [
            (67, 0, 512),
            (67, 640, 512),
            (68, 1280, 704),
            (68, 2560, 704)
        ]
    );
}

/// `aa <span>bb</span>` in an Ahem root 200px wide, styled by `sheet`.
fn generated_paragraph(sheet: &str) -> (Document, raikiri_style::CascadeResult, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = doc.append_element(Some(html), "head", Style::default(), Some("display:none"));
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(style, sheet);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let root = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;width:200px"),
    );
    doc.append_text(root, "aa ");
    let span = doc.append_element(Some(root), "span", Style::default(), None::<&str>);
    doc.append_text(span, "bb");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    (doc, cascade, root)
}

/// The x of every glyph painted in `color`.
fn glyph_xs_in(scene: &Scene, color: peniko::Color) -> Vec<f64> {
    glyphs(scene)
        .into_iter()
        .filter(|g| g.3 == anyrender::Paint::Solid(color))
        .map(|g| g.1)
        .collect()
}

#[test]
fn generated_text_of_an_inline_is_painted_once_in_its_place_and_style() {
    let (mut doc, cascade, root) =
        generated_paragraph(r#"span::before { content: "x"; color: rgb(255, 0, 0) }"#);
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let scene = painted(&doc, &cascade);
    // Hand-computed: "aa " is 30px wide, so the span's "x" starts at 30 and
    // its "bb" at 40.
    assert_eq!(
        glyph_xs_in(&scene, peniko::Color::from_rgba8(255, 0, 0, 255)),
        [30.0]
    );
    let black: Vec<f64> = glyph_xs_in(&scene, peniko::Color::from_rgba8(0, 0, 0, 255));
    assert!(black.contains(&40.0) && black.contains(&50.0), "{black:?}");
}

#[test]
fn generated_text_of_the_root_is_painted_once() {
    let (mut doc, cascade, root) = generated_paragraph(
        r#"div::before { content: "x"; color: rgb(255, 0, 0) } div::after { content: "y"; color: rgb(0, 0, 255) }"#,
    );
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let scene = painted(&doc, &cascade);
    // Hand-computed: "x" at 0, "aa bb" from 10 to 60, "y" at 60; the
    // generated text is drawn once.
    assert_eq!(
        glyph_xs_in(&scene, peniko::Color::from_rgba8(255, 0, 0, 255)),
        [0.0]
    );
    assert_eq!(
        glyph_xs_in(&scene, peniko::Color::from_rgba8(0, 0, 255, 255)),
        [60.0]
    );
}

#[test]
fn multicol_paint_places_lines_in_columns() {
    // A multicol container whose own content is the paragraph, and one whose
    // child paragraph is split in its columns.
    let own = |doc: &mut Document, root: usize| {
        doc.append_text(root, "aaaa bbbb cccc dddd");
    };
    let on = painted_root("width:100px;column-count:2;column-gap:10px", own);
    assert_eq!(
        ink(&on),
        [
            (3, 2560, 512),
            (3, 2560, 1152),
            (3, 6080, 512),
            (67, 0, 512),
            (67, 640, 512),
            (67, 1280, 512),
            (67, 1920, 512),
            (68, 0, 1152),
            (68, 640, 1152),
            (68, 1280, 1152),
            (68, 1920, 1152),
            (69, 3520, 512),
            (69, 4160, 512),
            (69, 4800, 512),
            (69, 5440, 512),
            (70, 3520, 1152),
            (70, 4160, 1152),
            (70, 4800, 1152),
            (70, 5440, 1152)
        ]
    );
    // Hand-computed: one 40px word per line in two 45px columns; "cccc" starts
    // the second column at x = 55, on the first baseline (y = 8).
    let placed = glyphs(&on);
    assert!(
        placed.iter().any(|g| g.1 == 55.0 && g.2 == 8.0),
        "{:?}",
        placed.iter().map(|g| (g.1, g.2)).collect::<Vec<_>>()
    );
}

#[test]
fn a_dfs_offset_accumulator_does_not_add_an_inline_elements_location() {
    // An inline-block inside a relatively positioned span: it is painted
    // once, at its place on the line moved by the span's offset.
    let (mut doc, cascade, root) = paragraph("width:100px", |doc, root| {
        let span = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("position:relative;left:5px"),
        );
        doc.append_text(span, "aa ");
        doc.append_element(
            Some(span),
            "span",
            Style::default(),
            Some("display:inline-block;width:30px;height:10px;background-color:red"),
        );
        doc.append_text(root, " bb");
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let scene = painted(&doc, &cascade);
    let boxes: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(kurbo::Shape::bounding_box(&fill.shape)),
            _ => None,
        })
        .filter(|b| (b.x1 - b.x0, b.y1 - b.y0) == (30.0, 10.0))
        .collect();
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    // Hand-computed: "aa " is 30px, and the span moves it 5px right.
    assert_eq!((boxes[0].x0, boxes[0].y0), (35.0, 0.0));
}

#[test]
fn a_raised_inline_block_inside_a_span_is_painted_where_the_engine_placed_it() {
    // As a_raised_inline_block_root_is_painted_where_the_engine_placed_it,
    // with the inline-block inside a span: it is still a box the inline
    // engine placed, so its text is not raised a second time.
    let (mut doc, cascade, root) = paragraph("width:200px", |doc, root| {
        let span = doc.append_element(Some(root), "span", Style::default(), None::<&str>);
        doc.append_text(span, "aa");
        let block = doc.append_element(
            Some(span),
            "span",
            Style::default(),
            Some("display:inline-block;vertical-align:10px"),
        );
        doc.append_text(block, "bb");
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let mut ys: Vec<i64> = ink(&painted(&doc, &cascade)).iter().map(|g| g.2).collect();
    ys.sort_unstable();
    assert_eq!(ys, [px(8), px(8), px(18), px(18)]);
}

#[test]
fn lines_after_a_block_moved_to_the_next_page_are_painted_there() {
    // "aa", a block with `break-before: page` holding "bb", then " cc": the
    // block starts the second 50px page and the line after it follows.
    let (mut doc, cascade, root) = paragraph("width:100px", |doc, root| {
        doc.append_text(root, "aa");
        let block = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;break-before:page"),
        );
        doc.append_text(block, "bb");
        doc.append_text(root, " cc");
    });
    let dir = std::path::Path::new(FONT_DIR);
    let collection = raikiri_dom::build_wpt_font_collection(dir).expect("collection");
    doc.set_font_collection(collection);
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;
    raikiri_dom::layout_pages(&mut doc, &cascade, page).expect("pages");
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let mut scene = Scene::new();
    let mut budget = raikiri_dom::CounterSnapshotBudget::default();
    crate::paint_single_page_with_origin(&mut scene, &doc, &cascade, page, 50.0, &mut budget)
        .expect("paint succeeds");
    // Glyphs above the page (the first page's "aa") are outside it.
    let ys: std::collections::BTreeSet<i64> = ink(&scene)
        .iter()
        .map(|g| g.2)
        .filter(|y| (0..px(50)).contains(y))
        .collect();
    // Hand-computed: "bb" on the second page's first line (baseline 8) and
    // "cc" on its second (baseline 18).
    assert_eq!(ys.into_iter().collect::<Vec<_>>(), [px(8), px(18)]);
}

#[test]
fn a_positioned_box_inside_the_paragraph_is_painted_once_where_it_was_placed() {
    // "aaaa " + an absolutely positioned 10x10 span inside a red-bordered
    // inline + " bbbb", in a relatively positioned root: the span is painted
    // at its insets, the text keeps one 90px line and the inline's piece is
    // still painted.
    let (mut doc, cascade, root) = paragraph("width:200px;position:relative", |doc, root| {
        doc.append_text(root, "aaaa ");
        let inline = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("background-color:blue"),
        );
        doc.append_text(inline, "cc");
        doc.append_element(
            Some(inline),
            "span",
            Style::default(),
            Some("position:absolute;left:5px;top:20px;width:10px;height:10px;background-color:red"),
        );
        doc.append_text(root, " bbbb");
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let scene = painted(&doc, &cascade);
    let fills: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(kurbo::Shape::bounding_box(&fill.shape)),
            _ => None,
        })
        .collect();
    let squares: Vec<_> = fills
        .iter()
        .filter(|b| (b.x1 - b.x0, b.y1 - b.y0) == (10.0, 10.0))
        .map(|b| (b.x0, b.y0))
        .collect();
    assert_eq!(squares, [(5.0, 20.0)]);
    // The inline's piece: "cc" from 50 to 70 on the first line.
    assert!(
        fills
            .iter()
            .any(|b| (b.x0, b.y0, b.x1, b.y1) == (50.0, 0.0, 70.0, 10.0)),
        "{fills:?}"
    );
    // Hand-computed: one line, "aaaa ccbbbb" with the glyphs of "bbbb" ending
    // at 110 (the positioned span takes no room).
    let max_x = glyphs(&scene).iter().map(|g| g.1).fold(0.0_f64, f64::max);
    assert_eq!(max_x, 110.0);
}

#[test]
fn a_fixed_root_is_painted_like_fixed_text() {
    // As a_paragraph_inside_a_fixed_box_is_painted_like_fixed_text, with the
    // fixed box itself the paragraph's root: its lines repeat on every page,
    // even below the laid-out box.
    let build = |doc: &mut Document, root: usize| {
        doc.append_text(root, "abcde");
    };
    let css = "position:fixed;top:1200px;left:0;width:100px;word-break:break-all";

    let (mut on_doc, cascade, root) = paragraph(css, build);
    lay_out(&mut on_doc, &cascade);
    assert!(on_doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let on = ink(&painted(&on_doc, &cascade));

    assert!(!on.is_empty(), "fixed text is drawn");
    assert_eq!(
        on,
        [
            (67, 0, 77312),
            (68, 640, 77312),
            (69, 1280, 77312),
            (70, 1920, 77312),
            (71, 2560, 77312)
        ]
    );
}

#[test]
fn an_inline_box_after_a_block_moved_to_the_next_page_is_painted_with_its_line() {
    // "aa", a block with `break-before: page` holding "bb", then a span with
    // a background holding "cc": the span's box follows its line to the
    // second page, once.
    let (mut doc, cascade, root) = paragraph("width:100px", |doc, root| {
        doc.append_text(root, "aa");
        let block = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;break-before:page"),
        );
        doc.append_text(block, "bb");
        let span = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("background-color:red"),
        );
        doc.append_text(span, "cc");
    });
    let dir = std::path::Path::new(FONT_DIR);
    let collection = raikiri_dom::build_wpt_font_collection(dir).expect("collection");
    doc.set_font_collection(collection);
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;
    raikiri_dom::layout_pages(&mut doc, &cascade, page).expect("pages");
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let mut scene = Scene::new();
    let mut budget = raikiri_dom::CounterSnapshotBudget::default();
    crate::paint_single_page_with_origin(&mut scene, &doc, &cascade, page, 50.0, &mut budget)
        .expect("paint succeeds");
    let boxes: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(kurbo::Shape::bounding_box(&fill.shape)),
            _ => None,
        })
        .filter(|b| (b.x1 - b.x0, b.y1 - b.y0) == (20.0, 10.0))
        .map(|b| (b.x0, b.y0))
        .collect();
    // Hand-computed: "cc" is the second line of the second page, so its box
    // spans y 10..20 from the page top.
    assert_eq!(boxes, [(0.0, 10.0)]);
}

#[test]
fn draw_ifc_lines_selects_only_the_requested_fragmentainer() {
    let (mut doc, cascade, root) = paragraph(
        "width:10px;height:10px;column-count:2;column-gap:0;word-break:break-all",
        |doc, root| {
            doc.append_text(root, "ab");
        },
    );
    lay_out(&mut doc, &cascade);
    let paint = |fragmentainer: Option<usize>| {
        let mut scene = Scene::new();
        draw_ifc_lines(
            &mut scene,
            &doc,
            &cascade,
            root,
            IfcPosition {
                x: 100.0 + fragmentainer.unwrap_or(0) as f32 * 5.0,
                y: 200.0,
                shift_y: 0.0,
            },
            &crate::text::DecorationContext::default(),
            fragmentainer,
            &[],
        );
        glyphs(&scene)
    };

    let first = paint(Some(0));
    let second = paint(Some(1));
    let all = paint(None);
    assert_eq!(
        first.iter().map(|glyph| glyph.1).collect::<Vec<_>>(),
        [100.0]
    );
    assert_eq!(
        second.iter().map(|glyph| glyph.1).collect::<Vec<_>>(),
        [105.0]
    );
    assert_eq!(
        all.iter().map(|glyph| glyph.1).collect::<Vec<_>>(),
        [100.0, 105.0]
    );
}

fn draw_root(doc: &Document, cascade: &raikiri_style::CascadeResult, root: usize) -> Scene {
    let mut scene = Scene::new();
    draw_ifc_lines(
        &mut scene,
        doc,
        cascade,
        root,
        IfcPosition {
            x: 0.0,
            y: 0.0,
            shift_y: 0.0,
        },
        &crate::text::DecorationContext::default(),
        None,
        &[],
    );
    scene
}

#[test]
fn emphasis_marks_sit_over_each_cluster_but_not_punctuation() {
    let (mut doc, cascade, root) = paragraph(
        "line-height:40px;text-emphasis:dot;text-emphasis-color:rgb(255,0,0)",
        |doc, root| {
            doc.append_text(root, "aa,a");
        },
    );
    lay_out(&mut doc, &cascade);
    let placed = glyphs(&draw_root(&doc, &cascade, root));
    let red = solid(255, 0, 0);
    let (marks, text): (Vec<_>, Vec<_>) = placed.iter().partition(|g| g.3 == red);
    assert_eq!(text.len(), 4, "every character is drawn");
    // One mark for each letter; the comma takes none.
    assert_eq!(marks.len(), 3);
    let baseline = text[0].2;
    assert!(
        marks.iter().all(|mark| mark.2 < baseline),
        "marks go over the text"
    );
    // Each mark is centered on its 10px cluster: the three marks keep the
    // letters' pitch, skipping the comma's.
    let xs: Vec<f64> = marks.iter().map(|mark| mark.1).collect();
    assert!((xs[1] - xs[0] - 10.0).abs() < 0.01, "{xs:?}");
    assert!((xs[2] - xs[1] - 20.0).abs() < 0.01, "{xs:?}");
}

#[test]
fn emphasis_marks_follow_the_text_color_by_default_and_can_go_under() {
    let (mut doc, cascade, root) = paragraph(
        "line-height:40px;color:rgb(0,0,255);text-emphasis:dot;text-emphasis-position:under right",
        |doc, root| {
            doc.append_text(root, "a");
        },
    );
    lay_out(&mut doc, &cascade);
    let placed = glyphs(&draw_root(&doc, &cascade, root));
    assert_eq!(placed.len(), 2);
    let blue = solid(0, 0, 255);
    assert!(placed.iter().all(|g| g.3 == blue));
    let (text, mark) = (placed[0].2.min(placed[1].2), placed[0].2.max(placed[1].2));
    assert!(mark > text, "the mark is below the baseline");
}

#[test]
fn ellipsis_is_painted_in_the_root_style() {
    let css =
        "width:50px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;color:rgb(0,0,255)";
    let (mut doc, cascade, root) = paragraph(css, |doc, root| {
        let span = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;color:rgb(255,0,0)"),
        );
        doc.append_text(span, "aaaaaaaaaa");
    });
    lay_out(&mut doc, &cascade);
    let placed = glyphs(&draw_root(&doc, &cascade, root));
    let blue = solid(0, 0, 255);
    let ellipsis: Vec<_> = placed.iter().filter(|g| g.3 == blue).collect();
    let kept = placed.len() - ellipsis.len();
    assert!(
        !ellipsis.is_empty(),
        "the ellipsis is drawn in the root color"
    );
    assert!(kept < 10, "content past the ellipsis is dropped");
    let last_kept = placed
        .iter()
        .filter(|g| g.3 != blue)
        .map(|g| g.1)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        ellipsis.iter().all(|g| g.1 > last_kept),
        "the ellipsis follows the content"
    );
    assert!(
        ellipsis.iter().all(|g| g.1 < 50.0),
        "the ellipsis fits the box"
    );

    // A hidden root hides its ellipsis while visible content stays.
    let (mut doc, cascade, root) = paragraph(&format!("{css};visibility:hidden"), |doc, root| {
        let span = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;visibility:visible;color:rgb(255,0,0)"),
        );
        doc.append_text(span, "aaaaaaaaaa");
    });
    lay_out(&mut doc, &cascade);
    let placed = glyphs(&draw_root(&doc, &cascade, root));
    assert!(!placed.is_empty());
    assert!(placed.iter().all(|g| g.3 != blue));
}
