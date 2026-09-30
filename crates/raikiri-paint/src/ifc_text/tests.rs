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
fn a_fixed_position_root_is_painted_like_fixed_text() {
    // A fixed box repeats on every page, so its text is drawn even where its
    // laid-out box does not meet the page. The A4 page is about 1122px tall;
    // the root sits below it. verify in code: choose a `top` where the parley
    // path still draws the text; the assertion below requires that.
    let css = "position:fixed;top:1200px;left:0;width:100px;word-break:break-all";
    let (off, on) = off_and_on(css, |doc, root| {
        doc.append_text(root, "abcde");
    });
    assert!(!ink(&off).is_empty(), "the parley path draws fixed text");
    assert_eq!(ink(&on), ink(&off));
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
