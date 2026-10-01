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

fn lay_out(doc: &mut Document, cascade: &raikiri_style::CascadeResult, inline_formatting: bool) {
    let dir = std::path::Path::new(FONT_DIR);
    if inline_formatting {
        let collection = raikiri_dom::build_wpt_font_collection(dir).expect("collection");
        doc.enable_inline_formatting(collection, shodo::limits::Limits::default());
    }
    let fonts = raikiri_dom::build_wpt_font_ctx(dir).expect("font ctx");
    layout_single_page(doc, cascade, PageBox::A4, fonts).expect("layout");
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
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
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
    crate::paint_single_page(&mut scene, doc, cascade, PageBox::A4);
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

/// Paint the same paragraph with the switch off and on.
fn off_and_on(css: &str, build: impl Fn(&mut Document, usize)) -> (Scene, Scene) {
    let (mut off_doc, cascade, _) = paragraph(css, &build);
    lay_out(&mut off_doc, &cascade, false);
    let off = painted(&off_doc, &cascade);

    let (mut on_doc, cascade, root) = paragraph(css, &build);
    lay_out(&mut on_doc, &cascade, true);
    assert!(
        on_doc.get_node(root).is_some_and(|n| n.is_ifc_root()),
        "{css}: the paragraph did not become an ifc root"
    );
    let on = painted(&on_doc, &cascade);
    (off, on)
}

#[test]
fn ifc_glyph_positions_match_the_parley_path() {
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
        let (off, on) = off_and_on(css, |doc, root| {
            doc.append_text(root, text);
        });
        assert!(
            !ink(&off).is_empty(),
            "{css}: the parley path painted nothing"
        );
        assert_eq!(ink(&on), ink(&off), "{css}");
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
    // At 16px with `line-height: normal` parley rounds the ascent and descent
    // (baseline 13.0) while shodo keeps 1/64px metrics (baseline 12.796875), so
    // the recorded y differs by 0.2px. The renderer rounds a hinted glyph's y
    // to whole pixels, which hides it; compare y the same way and do not round
    // in the painter.
    let css = "width:80px;word-break:break-all;font-size:16px;line-height:normal";
    let (off, on) = off_and_on(css, |doc, root| {
        doc.append_text(root, "abcdefghijklmno");
    });
    assert!(!ink(&off).is_empty());
    assert_eq!(ink_whole_pixel_y(&on), ink_whole_pixel_y(&off));
}

#[test]
fn a_body_root_takes_the_body_left_margin_like_its_text() {
    // `<body>` itself is the paragraph root: the walk shifts a body's direct
    // text by the body's left margin, so the lines must be shifted too.
    let build = |inline_formatting: bool| {
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
        lay_out(&mut doc, &cascade, inline_formatting);
        if inline_formatting {
            assert!(doc.get_node(body).is_some_and(|n| n.is_ifc_root()));
        }
        painted(&doc, &cascade)
    };
    let (off, on) = (build(false), build(true));
    assert!(!ink(&off).is_empty());
    assert_eq!(ink(&on), ink(&off));
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
    let (mut off_doc, cascade, _) = paragraph(css, build);
    lay_out(&mut off_doc, &cascade, false);
    let off = ink(&painted(&off_doc, &cascade));

    let (mut on_doc, cascade, root) = paragraph(css, build);
    lay_out(&mut on_doc, &cascade, true);
    let inner = on_doc.get_node(root).map(|n| n.children[0]).expect("inner");
    assert!(on_doc.get_node(inner).is_some_and(|n| n.is_ifc_root()));
    let on = ink(&painted(&on_doc, &cascade));

    assert!(!off.is_empty(), "the parley path draws fixed text");
    assert_eq!(on, off);
}

#[test]
fn ifc_text_is_painted_once() {
    let (mut doc, cascade, _) = paragraph("width:50px", |doc, root| {
        doc.append_text(root, "abcde");
    });
    lay_out(&mut doc, &cascade, true);
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
fn underline_and_line_through_match_the_parley_path() {
    for css in ["text-decoration:underline", "text-decoration:line-through"] {
        let build = |doc: &mut Document, root: usize| {
            doc.append_text(root, "abcde");
        };
        let (mut off_doc, cascade, _) = paragraph(css, build);
        lay_out(&mut off_doc, &cascade, false);
        let off = decoration_fills(&painted(&off_doc, &cascade));

        let (mut on_doc, cascade, _) = paragraph(css, build);
        lay_out(&mut on_doc, &cascade, true);
        let on = decoration_fills(&painted(&on_doc, &cascade));

        assert!(!off.is_empty(), "{css}: the parley path drew no line");
        assert_eq!(on, off, "{css}");
    }
}

#[test]
fn an_undecorated_paragraph_draws_no_decoration_rectangle() {
    let (mut doc, cascade, _) = paragraph("", |doc, root| {
        doc.append_text(root, "abcde");
    });
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut on_doc, &cascade, true);
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
    lay_out(&mut on_doc, &cascade, true);
    let on = decoration_fills(&painted(&on_doc, &cascade));

    let (mut off_doc, cascade, _) = paragraph("", build);
    lay_out(&mut off_doc, &cascade, false);
    let off = decoration_fills(&painted(&off_doc, &cascade));

    assert_eq!(off.len(), 1, "the parley path underlines only the span");
    assert_eq!(on, off);
}

#[test]
fn a_trailing_empty_line_paints_no_glyphs() {
    let (mut doc, cascade, root) = paragraph("width:200px", |doc, root| {
        doc.append_text(root, "abcd");
        doc.append_element(Some(root), "br", Style::default(), Some("display:inline"));
    });
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
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
fn ch_lengths_place_glyphs_like_the_parley_path() {
    // Ahem at 10px: one ch is 10px, so the lengths below are whole pixels.
    let cases = [
        ("width:200px;letter-spacing:1ch", "abc"),
        ("width:200px;word-spacing:2ch", "a b c"),
        // The parley path indents a run only when it re-breaks it to a
        // narrower width, as in the px indent case above.
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
        let (off, on) = off_and_on(css, |doc, root| {
            doc.append_text(root, text);
        });
        assert!(!drawn(&off).is_empty(), "{css}");
        assert_eq!(drawn(&on), drawn(&off), "{css}");
    }
}

/// The glyph id of U+0020 in Ahem.
const AHEM_SPACE_GLYPH: u32 = 3;

#[test]
fn an_inherited_ch_spacing_places_glyphs_like_the_parley_path() {
    // The span inherits `1ch` measured with the paragraph's 10px font, not
    // with its own 20px font.
    let (off, on) = off_and_on("width:300px;letter-spacing:1ch", |doc, root| {
        doc.append_text(root, "ab");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;font-size:20px"),
        );
        doc.append_text(inner, "cd");
    });
    // Only the inline positions are compared: the parley path lays the 10px
    // text on its own baseline instead of the line's shared one, which is
    // unrelated to `ch`.
    let x_of = |scene: &Scene| {
        ink(scene)
            .into_iter()
            .map(|glyph| (glyph.0, glyph.1))
            .collect::<Vec<_>>()
    };
    assert!(!x_of(&off).is_empty());
    assert_eq!(x_of(&on), x_of(&off));
    // `cd` sits after `ab` and two 10px spacings: 40px, not 60px.
    assert_eq!(x_of(&on)[2].1, 40 * 64);
}

#[test]
fn break_word_breaks_a_long_word_like_the_parley_path() {
    let (off, on) = off_and_on("width:50px;word-break:break-word", |doc, root| {
        doc.append_text(root, "abcdefghij");
    });
    assert!(!ink(&off).is_empty());
    // Two lines of five letters each.
    assert!(ink(&off).iter().any(|glyph| glyph.2 > 10 * 64));
    assert_eq!(ink(&on), ink(&off));
}

#[test]
fn text_emphasis_paints_nothing_extra() {
    let (off, on) = off_and_on("width:100px;text-emphasis-style:dot", |doc, root| {
        doc.append_text(root, "abc");
    });
    assert!(!ink(&off).is_empty());
    assert_eq!(ink(&on), ink(&off));
    assert_eq!(on.commands.len(), off.commands.len());
}

#[test]
fn tabs_and_spaces_place_glyphs_on_the_same_positions() {
    // tab-size 8 and Ahem's 10px space: a tab after `a` reaches the stop at
    // 80px, which seven spaces reach too. The positions must be identical,
    // without the snapping the parley path needs for this case.
    let place = |text: &'static str| {
        let (mut doc, cascade, _) = paragraph("white-space:pre;width:400px", |doc, root| {
            doc.append_text(root, text);
        });
        lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
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
fn a_relative_inline_with_no_offset_paints_like_the_parley_path() {
    // No space at the text boundary: the parley path shapes a space that ends
    // a text node with another glyph, which is unrelated to the position.
    let (off, on) = off_and_on("width:100px", |doc, root| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;position:relative;color:blue"),
        );
        doc.append_text(inner, "bb");
    });
    assert!(!glyphs(&off).is_empty());
    assert_eq!(ink(&on), ink(&off));
    let brushes = |scene: &Scene| {
        let mut out: Vec<String> = glyphs(scene)
            .into_iter()
            .map(|glyph| format!("{:?}", glyph.3))
            .collect();
        out.sort();
        out
    };
    assert_eq!(brushes(&on), brushes(&off));
}

#[test]
fn a_relative_block_child_with_no_offset_is_painted_once_like_the_parley_path() {
    let (off, on) = off_and_on("width:100px", |doc, root| {
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
    assert_eq!(ink(&off).len(), 6);
    assert_eq!(ink(&on), ink(&off));
    let fills = |scene: &Scene| {
        scene
            .commands
            .iter()
            .filter(|command| matches!(command, RenderCommand::Fill(_)))
            .count()
    };
    assert_eq!(fills(&on), fills(&off));
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
    let (off, on) = off_and_on(
        "width:100px;text-shadow:2px 3px red, 4px 1px blue;color:black",
        |doc, root| {
            doc.append_text(root, "ab");
        },
    );
    assert!(runs_in_order(&off).len() >= 6, "two shadows and the text");
    assert_eq!(runs_in_order(&on), runs_in_order(&off));
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
    let (off, on) = off_and_on("width:100px;text-shadow:1px 1px 3px red", |doc, root| {
        doc.append_text(root, "ab");
    });
    assert!(layers(&off) >= 1);
    assert_eq!(layers(&on), layers(&off));
    assert_eq!(runs_in_order(&on), runs_in_order(&off));
}

#[test]
fn a_shadow_of_an_inline_element_uses_that_elements_color() {
    let (off, on) = off_and_on("width:100px;text-shadow:1px 1px", |doc, root| {
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
    assert_eq!(runs_in_order(&off).len(), 4, "a shadow and a glyph each");
    assert_eq!(runs_in_order(&on), runs_in_order(&off));
}

/// Whole-pixel x of every drawn glyph, sorted. Ahem's space glyph has no
/// outline and is left out: at the end of a wrapped right-to-left line the
/// inline engine hangs it at the line's left end, past the content, while the
/// parley path puts it at the right edge.
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
fn rtl_lines_place_glyphs_like_the_parley_path() {
    let cases = [
        ("direction:rtl;width:100px", "abc"),
        ("direction:rtl;width:50px", "abc def"),
        ("direction:rtl;text-align:left;width:100px", "abc"),
        ("direction:rtl;width:100px;text-align:center", "abc"),
        // Hebrew letters have no glyph in Ahem, so every one draws the same
        // notdef box: the positions of the boxes are still compared, but not
        // the order of the letters inside the right-to-left run.
        ("width:100px", "abc \u{05d0}\u{05d1}\u{05d2}"),
        ("direction:rtl;width:100px", "\u{05d0}\u{05d1} abc"),
        // The content box does not start at the page's left edge, and its
        // width is not authored.
        ("direction:rtl;margin:0 30px", "abc def"),
    ];
    for (css, text) in cases {
        let (off, on) = off_and_on(css, |doc, root| {
            doc.append_text(root, text);
        });
        assert!(!ink(&off).is_empty(), "{css}");
        assert_eq!(sorted_xs(&on), sorted_xs(&off), "{css}");
    }
}

#[test]
fn an_rtl_underline_spans_the_same_extent_as_the_parley_path() {
    let (off, on) = off_and_on(
        "direction:rtl;width:100px;text-decoration:underline",
        |doc, root| {
            doc.append_text(root, "abc");
        },
    );
    assert!(!decoration_fills(&off).is_empty());
    assert_eq!(decoration_fills(&on), decoration_fills(&off));
}

#[test]
fn an_rtl_text_indent_is_taken_from_the_right_edge() {
    // CSS Text 3 §7.1: the indent is at the start side, the right in a
    // right-to-left line. The start is at 100 - 20 = 80, so the three 10px
    // letters end there: a 50, b 60, c 70. The parley path does not indent a
    // run it does not re-break, so this is hand-computed, not an oracle.
    let (mut doc, cascade, root) =
        paragraph("direction:rtl;width:100px;text-indent:20px", |doc, root| {
            doc.append_text(root, "abc");
        });
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    // The line spans 30..100 and the two glyphs end at the right edge.
    assert_eq!(sorted_xs(&painted(&doc, &cascade)), vec![80, 90]);
}

#[test]
fn an_rtl_paragraph_beside_a_right_float_starts_before_it() {
    let (mut doc, cascade, root) =
        beside_float("float:right;width:30px;height:10px", "direction:rtl", "ab");
    lay_out(&mut doc, &cascade, true);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    // The line spans 0..70 and the glyphs end at the float's left edge.
    assert_eq!(sorted_xs(&painted(&doc, &cascade)), vec![50, 60]);
}

#[test]
fn an_ltr_paragraph_beside_a_left_float_is_unchanged() {
    let (mut doc, cascade, root) =
        beside_float("float:left;width:30px;height:10px", "direction:ltr", "ab");
    lay_out(&mut doc, &cascade, true);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    assert_eq!(sorted_xs(&painted(&doc, &cascade)), vec![30, 40]);
}

#[test]
fn an_rtl_line_ends_at_the_right_edge_of_the_content_box() {
    // Padding 5px left and 15px right around a 100px content box: the content
    // box spans 5..105, so `abc` ends at 105 (a 75, b 85, c 95). The parley
    // path aligns the run 20px further right, past the content box, for
    // `text-align: right` in a left-to-right paragraph as well, so this is
    // hand-computed, not an oracle.
    let (mut doc, cascade, root) = paragraph(
        "direction:rtl;padding:0 15px 0 5px;width:100px",
        |doc, root| {
            doc.append_text(root, "abc");
        },
    );
    lay_out(&mut doc, &cascade, true);
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
        let (off, on) = off_and_on(css, |doc, root| {
            doc.append_text(root, "aaaa bbbb");
        });
        assert!(decoration_fills(&off).len() >= 2, "{css}: one line each");
        assert_eq!(decoration_fills(&on), decoration_fills(&off), "{css}");
    }
}

#[test]
fn an_unwrapped_underline_keeps_its_full_extent() {
    let (off, on) = off_and_on("width:200px;text-decoration:underline", |doc, root| {
        doc.append_text(root, "aaaa bbbb");
    });
    assert!(!decoration_fills(&off).is_empty());
    assert_eq!(decoration_fills(&on), decoration_fills(&off));
}

#[test]
fn a_rtl_wrapped_underline_leaves_out_the_break_space_too() {
    let (off, on) = off_and_on(
        "direction:rtl;width:50px;text-decoration:underline",
        |doc, root| {
            doc.append_text(root, "aaaa bbbb");
        },
    );
    assert!(decoration_fills(&off).len() >= 2, "one line each");
    assert_eq!(decoration_fills(&on), decoration_fills(&off));
}

#[test]
fn an_underline_under_a_hanging_bracket_covers_the_bracket_and_the_content() {
    // `(` hangs one em (10px) before the line start, so the glyphs are at
    // -10, 0, 10. The line's content width leaves the hung bracket out (it is
    // 20px), so the extent has to add the hang back: -10 .. 20. The parley
    // path only hangs a leading U+3000, so this is a hand-computed value, not
    // an oracle.
    let (mut doc, cascade, _) = paragraph(
        "width:100px;hanging-punctuation:first;text-decoration:underline",
        |doc, root| {
            doc.append_text(root, "(ab");
        },
    );
    lay_out(&mut doc, &cascade, true);
    let fills = decoration_fills(&painted(&doc, &cascade));
    assert_eq!(fills.len(), 1);
    assert_eq!((fills[0].0, fills[0].2), (-10 * 64, 20 * 64));
}

/// `aa` then a span `bb` with `vertical-align: {align}`. No space at the text
/// boundary: the parley path ends each text node's decoration before a
/// trailing space, which is unrelated to the baseline.
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
        let (off, on) = off_and_on("width:100px;text-decoration:underline", |doc, root| {
            raised(doc, root, align, "");
        });
        assert!(!decoration_fills(&off).is_empty(), "{align}");
        assert_eq!(decoration_fills(&on), decoration_fills(&off), "{align}");
    }
}

#[test]
fn an_inner_underline_follows_the_raised_inline() {
    for align in ["4px", "-3px"] {
        let (off, on) = off_and_on("width:100px", |doc, root| {
            raised(doc, root, align, "text-decoration:underline");
        });
        assert!(!decoration_fills(&off).is_empty(), "{align}");
        assert_eq!(decoration_fills(&on), decoration_fills(&off), "{align}");
    }
}

#[test]
fn a_line_through_follows_the_same_rule() {
    let (off, on) = off_and_on("width:100px;text-decoration:line-through", |doc, root| {
        raised(doc, root, "4px", "");
    });
    assert!(!decoration_fills(&off).is_empty());
    assert_eq!(decoration_fills(&on), decoration_fills(&off));
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
        lay_out(&mut doc, &cascade, true);
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

const BOX_EDGES: &str = "background-color:rgb(255,0,0);padding:0 3px;\
border-width:0 2px;border-style:solid;border-color:rgb(0,0,255);margin:0 4px";

#[test]
fn an_inline_background_and_border_match_the_parley_path() {
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
    let (off, on) = off_and_on("width:200px", build);
    // After "aa" and the 4px margin: border box x 24..54, y 0..10; borders
    // 2px wide.
    assert_eq!(
        fills_with(&off, &solid(255, 0, 0)),
        [[px(24), 0, px(54), px(10)]]
    );
    assert_eq!(
        fills_with(&off, &solid(0, 0, 255)),
        [[px(24), 0, px(26), px(10)], [px(52), 0, px(54), px(10)]]
    );
    assert_eq!(all_fills(&on), all_fills(&off));
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
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
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
fn an_inline_box_shadow_matches_the_parley_path() {
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
    let (off, on) = off_and_on("width:200px", build);
    // The span: x 20..46 (padding 3 + "bb" 20 + padding 3), y 0..10.
    assert_eq!(
        box_shadows(&off),
        [([px(22), px(2), px(48), px(12)], solid(0, 255, 0))],
        "the parley path paints a shadow"
    );
    assert_eq!(box_shadows(&on), box_shadows(&off));
    assert_eq!(all_fills(&on), all_fills(&off));
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
fn an_inline_outline_matches_the_parley_path() {
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
    let (off, on) = off_and_on("width:200px", build);
    assert!(
        !fills_with(&off, &solid(0, 255, 0)).is_empty(),
        "the parley path paints an outline"
    );
    assert_eq!(all_fills(&on), all_fills(&off));
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
    lay_out(&mut doc, &cascade, true);
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
    let (off, on) = off_and_on("width:200px", build);
    let off_shadows = box_shadows_with_radius(&off);
    assert_eq!(off_shadows.len(), 1);
    assert_eq!(off_shadows[0].2, 0, "the parley path squares the corners");
    assert_eq!(box_shadows_with_radius(&on), off_shadows);
}

#[test]
fn an_inline_background_clipped_to_its_content_box_matches_the_parley_path() {
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
    let (off, on) = off_and_on("width:200px", build);
    // The span's content box: x 29..49.
    assert_eq!(
        fills_with(&off, &solid(255, 0, 0)),
        [[px(29), 0, px(49), px(10)]]
    );
    assert_eq!(all_fills(&on), all_fills(&off));
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
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
    let scene = painted(&doc, &cascade);
    assert_eq!(
        fills_with(&scene, &solid(255, 0, 0)),
        [[0, 0, px(20), px(10)]]
    );
}

#[test]
fn a_relative_inline_is_painted_like_the_parley_path() {
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
    let (off, on) = off_and_on("width:200px", build);
    assert_eq!(ink(&on), ink(&off));
    assert!(!decoration_fills(&off).is_empty());
    assert_eq!(decoration_fills(&on), decoration_fills(&off));
    assert_eq!(
        fills_with(&off, &solid(255, 0, 0)),
        [[px(25), px(2), px(45), px(12)]]
    );
    assert_eq!(
        fills_with(&on, &solid(255, 0, 0)),
        fills_with(&off, &solid(255, 0, 0))
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
    let (off, on) = off_and_on("width:200px", build);
    assert_eq!(runs_in_order(&on), runs_in_order(&off));
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
    // The parley path stacks the three texts on three lines here, so the
    // positions are hand-computed: "aabbcc" on one line, 10px per glyph, on
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
    lay_out(&mut doc, &cascade, true);
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
    let (off, on) = off_and_on("", build);
    let (mut doc, cascade, root) = paragraph("", build);
    lay_out(&mut doc, &cascade, true);
    let block = doc.get_node(root).expect("root").children[1];
    assert!(doc.get_node(block).is_some_and(|n| n.is_ifc_root()));
    assert_eq!(ink(&on), ink(&off));
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
    lay_out(&mut doc, &cascade, true);
    let text = doc.get_node(root).expect("root").children[0];
    assert!(doc.get_node(text).is_some_and(|n| n.is_ifc_root()));
    let on = ink(&painted(&doc, &cascade));
    let (mut off_doc, cascade, _) = paragraph("display:flex;width:50px", build);
    lay_out(&mut off_doc, &cascade, false);
    assert_eq!(on, ink(&painted(&off_doc, &cascade)));
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
    lay_out(&mut doc, &cascade, true);
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
    lay_out(&mut doc, &cascade, true);
    assert!(doc.get_node(root).is_some_and(|n| n.is_ifc_root()));
    let mut scene = Scene::new();
    crate::paint_single_page_with_origin(&mut scene, &doc, &cascade, PageBox::A4, 1000.0);
    let ys: std::collections::BTreeSet<i64> = ink(&scene).iter().map(|g| g.2).collect();
    assert!(ys.contains(&px(8)), "{ys:?}");
}
