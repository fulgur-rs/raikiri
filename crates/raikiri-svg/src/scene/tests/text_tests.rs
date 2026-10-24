//! Text in scenes: set in the caller's faces, compared with the raster the
//! parser draws from a database that holds the same faces.

use super::*;
use crate::SvgFontFamily;
use crate::SvgFontRequest;
use crate::SvgFontStyle;
use raikiri_traits::vector::{FontFace, FontVariation, TextRun};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use std::sync::{Arc, Mutex};

const AHEM: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/Ahem.ttf"));
/// Has only the letters A to E.
const CANVAS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/CanvasTest-nospace.ttf"
));

fn face(bytes: &'static [u8]) -> FontFace {
    let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(bytes);
    FontFace::new(data, 0)
}

/// Answers every request with one face and every fallback with another, and
/// writes down what it was asked.
struct Fonts {
    primary: Option<FontFace>,
    fallback: Option<FontFace>,
    selected: Mutex<Vec<SvgFontRequest>>,
    fell_back: Mutex<Vec<char>>,
}

impl Fonts {
    fn ahem() -> Self {
        Self {
            primary: Some(face(AHEM)),
            fallback: None,
            selected: Mutex::default(),
            fell_back: Mutex::default(),
        }
    }

    fn none() -> Self {
        Self {
            primary: None,
            ..Self::ahem()
        }
    }

    /// Letters A to E in the primary face, everything else in Ahem.
    fn with_fallback() -> Self {
        Self {
            primary: Some(face(CANVAS)),
            fallback: Some(face(AHEM)),
            ..Self::ahem()
        }
    }
}

impl SvgFontSource for Fonts {
    fn select(&self, request: &SvgFontRequest) -> Option<FontFace> {
        self.selected.lock().unwrap().push(request.clone());
        self.primary.clone()
    }

    fn fallback(&self, _: &SvgFontRequest, ch: char) -> Option<FontFace> {
        self.fell_back.lock().unwrap().push(ch);
        self.fallback.clone()
    }
}

fn scene_with(document: &SvgDocument, fonts: &Fonts) -> VectorImage {
    document
        .scene_with_fonts(VIEWPORT, style(1.0), None, fonts)
        .unwrap()
}

/// The raster the parser itself draws when it knows both test faces.
fn raster_with_faces(document: &SvgDocument) -> Vec<u8> {
    let mut options = usvg::Options::default();
    let database = Arc::make_mut(&mut options.fontdb);
    database.load_font_data(AHEM.to_vec());
    database.load_font_data(CANVAS.to_vec());
    let tree = usvg::Tree::from_str(&document.source, &options).unwrap();
    let mut pixmap = tiny_skia::Pixmap::new(40, 20).unwrap();
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap.take_demultiplied()
}

fn runs(group: &Group) -> Vec<&TextRun> {
    let mut found = Vec::new();
    for child in &group.children {
        match child {
            Node::Text(run) => found.push(run),
            Node::Group(group) => found.extend(runs(group)),
            _ => {}
        }
    }
    found
}

struct Pen(tiny_skia::PathBuilder);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.quad_to(cx, cy, x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, cy0, cx1, cy1, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// The outline of a glyph in design units, `y` down.
fn glyph_outline(face: &FontFace, id: u32) -> Option<tiny_skia::Path> {
    let font = FontRef::from_index(face.bytes(), face.index()).unwrap();
    let glyph = font.outline_glyphs().get(GlyphId::new(id))?;
    let mut pen = Pen(tiny_skia::PathBuilder::new());
    glyph
        .draw(
            DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
            &mut pen,
        )
        .unwrap();
    pen.0
        .finish()?
        .transform(tiny_skia::Transform::from_scale(1.0, -1.0))
}

/// The glyphs of `run` as one path in the space of the run, painted like the
/// run: its fill and stroke each cover all the glyphs.
fn run_path(run: &TextRun) -> Path {
    let point = |point: tiny_skia::Point| (point.x, point.y);
    let mut segments = Vec::new();
    for glyph in &run.glyphs {
        let Some(outline) = glyph_outline(&glyph.font, glyph.id)
            .and_then(|outline| outline.transform(tiny_transform(glyph.transform)))
        else {
            continue;
        };
        segments.extend(outline.segments().map(|segment| match segment {
            tiny_skia::PathSegment::MoveTo(to) => PathSegment::MoveTo(point(to)),
            tiny_skia::PathSegment::LineTo(to) => PathSegment::LineTo(point(to)),
            tiny_skia::PathSegment::QuadTo(control, to) => PathSegment::QuadTo {
                control: point(control),
                to: point(to),
            },
            tiny_skia::PathSegment::CubicTo(control1, control2, to) => PathSegment::CubicTo {
                control1: point(control1),
                control2: point(control2),
                to: point(to),
            },
            tiny_skia::PathSegment::Close => PathSegment::Close,
        }));
    }
    let mut path = Path::new(segments);
    path.fill = run.fill.clone();
    path.stroke = run.stroke.clone();
    path.paint_order = run.paint_order;
    path.anti_alias = run.anti_alias;
    path
}

/// Draws `run` the way a consumer with only outlines would.
pub(super) fn draw_text(
    pixmap: &mut tiny_skia::Pixmap,
    run: &TextRun,
    parent: tiny_skia::Transform,
) {
    if !run.glyphs.is_empty() {
        draw_path(pixmap, &run_path(run), parent);
    }
}

fn assert_matches_raster(body: &str, fonts: &Fonts, tolerance: u8) -> VectorImage {
    let document = document(body);
    let scene = scene_with(&document, fonts);
    assert_close(&render(&scene), &raster_with_faces(&document), tolerance);
    scene
}

#[test]
fn text_is_set_in_the_faces_the_source_gives() {
    let fonts = Fonts::ahem();
    let scene = assert_matches_raster(
        r##"<text x="3" y="12" font-family="Ahem" font-size="8" fill="#336699">AB</text>"##,
        &fonts,
        2,
    );
    let runs = runs(&scene.root);
    assert_eq!(runs.len(), 1);
    let run = runs[0];
    assert_eq!(run.font_size, 8.0);
    assert_eq!(run.glyphs.len(), 2);
    assert_eq!(
        run.glyphs
            .iter()
            .map(|glyph| glyph.text.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    // Both glyphs share the one face the source gave.
    let ahem = fonts.primary.as_ref().unwrap();
    assert!(run.glyphs.iter().all(|glyph| glyph.font.is_same_face(ahem)));
    // Ahem advances one em per glyph.
    let [first, second] = &run.glyphs[..] else {
        panic!("two glyphs");
    };
    assert!((second.transform.e - first.transform.e - 8.0).abs() < 0.01);
    assert_eq!(first.transform.f, 12.0);
    // The run is painted like the text.
    let fill = run.fill.as_ref().unwrap();
    assert_eq!(
        fill.paint,
        Paint::Color(PaintColor::new(0.2, 0.4, 0.6, 1.0))
    );
    assert!(run.stroke.is_none());
}

#[test]
fn styled_spans_become_runs_with_their_own_paint() {
    let scene = assert_matches_raster(
        r##"<text x="2" y="12" font-family="Ahem" font-size="6" fill="#c00">A<tspan fill="#00c" dx="3">B</tspan><tspan font-size="10" fill="#0c0">C</tspan></text>"##,
        &Fonts::ahem(),
        2,
    );
    let runs = runs(&scene.root);
    assert_eq!(runs.len(), 3);
    assert_eq!(
        runs.iter().map(|run| run.font_size).collect::<Vec<_>>(),
        [6.0, 6.0, 10.0]
    );
    let colors: Vec<_> = runs
        .iter()
        .map(|run| run.fill.as_ref().unwrap().paint.clone())
        .collect();
    assert_ne!(colors[0], colors[1]);
    assert_ne!(colors[1], colors[2]);
}

#[test]
fn anchors_rotation_and_positions_match_the_raster() {
    for body in [
        r##"<text x="20" y="12" text-anchor="middle" font-family="Ahem" font-size="8" fill="#000">ABC</text>"##,
        r##"<text x="36" y="12" text-anchor="end" font-family="Ahem" font-size="8" fill="#000">AB</text>"##,
        r##"<text x="2 12 22" y="10 12 14" font-family="Ahem" font-size="6" fill="#000">ABC</text>"##,
        r##"<text x="4" y="10" rotate="15 -15" font-family="Ahem" font-size="6" fill="#000">ABC</text>"##,
        r##"<text x="2" y="12" letter-spacing="2" word-spacing="4" font-family="Ahem" font-size="5" fill="#000">AB C</text>"##,
        r##"<g transform="translate(3 2) scale(1.5)"><text x="1" y="8" font-family="Ahem" font-size="6" fill="#000">AB</text></g>"##,
    ] {
        assert_matches_raster(body, &Fonts::ahem(), 3);
    }
}

#[test]
fn text_strokes_gradients_and_paint_order_match_the_raster() {
    for body in [
        r##"<text x="3" y="14" font-family="Ahem" font-size="10" fill="none" stroke="#c00" stroke-width="2">AB</text>"##,
        r##"<text x="3" y="14" font-family="Ahem" font-size="10" fill="#fc0" stroke="#006" stroke-width="2" paint-order="stroke">AB</text>"##,
        r##"<linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" x2="40"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient>
            <text x="2" y="14" font-family="Ahem" font-size="10" fill="url(#g)">ABC</text>"##,
        r##"<linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient>
            <text x="2" y="14" font-family="Ahem" font-size="10" fill="url(#g)" fill-opacity="0.5">ABC</text>"##,
        r##"<text x="3" y="14" font-family="Ahem" font-size="10" fill="#000" fill-opacity="0.5" stroke="#00c" stroke-opacity="0.5" stroke-dasharray="3 2">A</text>"##,
    ] {
        assert_matches_raster(body, &Fonts::ahem(), 3);
    }
}

#[test]
fn text_rendering_optimize_speed_asks_for_hard_edges() {
    let body = |rendering: &str| {
        format!(
            r##"<text x="3.5" y="12.5" text-rendering="{rendering}" font-family="Ahem" font-size="7.5" fill="#000">AB</text>"##
        )
    };
    let fonts = Fonts::ahem();
    for (rendering, smooth) in [
        ("optimizeSpeed", false),
        ("optimizeLegibility", true),
        ("geometricPrecision", true),
    ] {
        let scene = scene_with(&document(&body(rendering)), &fonts);
        assert_eq!(runs(&scene.root)[0].anti_alias, smooth, "{rendering}");
    }
}

#[test]
fn decorations_are_paths_around_the_run() {
    let scene = assert_matches_raster(
        r##"<text x="3" y="12" font-family="Ahem" font-size="8" fill="#336699" text-decoration="underline overline line-through">AB</text>"##,
        &Fonts::ahem(),
        2,
    );
    let kinds: Vec<_> = scene
        .root
        .children
        .iter()
        .map(|node| match node {
            Node::Path(_) => "path",
            Node::Text(_) => "text",
            _ => "group",
        })
        .collect();
    // The overline and underline come before the glyphs, the strike-through
    // after them.
    assert_eq!(kinds, ["path", "path", "text", "path"]);
}

#[test]
fn the_requests_carry_the_text_font() {
    let fonts = Fonts::ahem();
    let document = document(
        r##"<text x="2" y="12" font-family="Foo, 'Bar Baz', monospace" font-weight="700" font-style="italic" font-stretch="condensed" font-size="8">A</text>"##,
    );
    scene_with(&document, &fonts);
    let selected = fonts.selected.lock().unwrap();
    assert_eq!(
        *selected,
        [SvgFontRequest {
            families: vec![
                SvgFontFamily::Named("Foo".into()),
                SvgFontFamily::Named("Bar Baz".into()),
                SvgFontFamily::Monospace,
                SvgFontFamily::Serif,
            ],
            weight: 700,
            stretch: 75.0,
            style: SvgFontStyle::Italic,
        }]
    );
}

#[test]
fn every_stretch_and_slant_keyword_reaches_the_request() {
    let fonts = Fonts::ahem();
    let stretches = [
        ("ultra-condensed", 50.0),
        ("extra-condensed", 62.5),
        ("condensed", 75.0),
        ("semi-condensed", 87.5),
        ("normal", 100.0),
        ("semi-expanded", 112.5),
        ("expanded", 125.0),
        ("extra-expanded", 150.0),
        ("ultra-expanded", 200.0),
    ];
    for (keyword, percent) in stretches {
        scene_with(
            &document(&format!(
                r##"<text x="2" y="12" font-stretch="{keyword}" font-size="8">A</text>"##
            )),
            &fonts,
        );
        assert_eq!(
            fonts.selected.lock().unwrap().last().unwrap().stretch,
            percent
        );
    }
    for (keyword, style) in [
        ("normal", SvgFontStyle::Normal),
        ("italic", SvgFontStyle::Italic),
        ("oblique", SvgFontStyle::Oblique),
    ] {
        scene_with(
            &document(&format!(
                r##"<text x="2" y="12" font-style="{keyword}" font-size="8">A</text>"##
            )),
            &fonts,
        );
        assert_eq!(fonts.selected.lock().unwrap().last().unwrap().style, style);
    }
}

#[test]
fn a_family_list_that_names_serif_does_not_get_it_twice() {
    let fonts = Fonts::ahem();
    scene_with(
        &document(r##"<text x="2" y="12" font-family="serif" font-size="8">A</text>"##),
        &fonts,
    );
    assert_eq!(
        fonts.selected.lock().unwrap()[0].families,
        [SvgFontFamily::Serif]
    );
}

#[test]
fn a_character_the_face_lacks_comes_from_the_fallback() {
    let fonts = Fonts::with_fallback();
    let scene = assert_matches_raster(
        r##"<text x="2" y="12" font-family="CanvasTestNoSpace" font-size="8" fill="#000">AXB</text>"##,
        &fonts,
        2,
    );
    assert!(!fonts.fell_back.lock().unwrap().is_empty());
    assert!(fonts.fell_back.lock().unwrap().iter().all(|&ch| ch == 'X'));
    let run = &runs(&scene.root)[0];
    // The parser sets the whole text in the fallback face when that face has
    // every character, so the letters of the first face come along.
    let faces: Vec<_> = run
        .glyphs
        .iter()
        .map(|glyph| glyph.font.is_same_face(fonts.fallback.as_ref().unwrap()))
        .collect();
    assert_eq!(faces, [true, true, true]);
}

#[test]
fn a_fallback_to_a_face_already_used_is_refused() {
    // The fallback answers with the face that lacks the character again.
    let fonts = Fonts {
        fallback: Some(face(CANVAS)),
        ..Fonts::with_fallback()
    };
    let document = document(
        r##"<text x="2" y="12" font-family="CanvasTestNoSpace" font-size="8" fill="#000">AX</text>"##,
    );
    let scene = scene_with(&document, &fonts);
    // The characters it has are still set; the other draws the missing glyph.
    assert_eq!(runs(&scene.root)[0].glyphs.len(), 2);
    assert!(fonts.fell_back.lock().unwrap().iter().all(|&ch| ch == 'X'));
    assert!(!fonts.fell_back.lock().unwrap().is_empty());
}

#[test]
fn text_that_no_face_can_set_is_left_out() {
    let document = document(
        r##"<rect width="10" height="10"/><text x="2" y="12" font-family="Ahem" font-size="8">A</text>"##,
    );
    let scene = scene_with(&document, &Fonts::none());
    assert!(runs(&scene.root).is_empty());
    assert_eq!(paths(&scene.root).len(), 1);
    // Without a source the text is left out the same way.
    let plain = document.scene(VIEWPORT, style(1.0)).unwrap();
    assert_eq!(scene, plain);
}

#[test]
fn a_document_without_text_does_not_ask_for_fonts() {
    let fonts = Fonts::ahem();
    let document = document(r##"<rect width="10" height="10" fill="#14c"/>"##);
    let scene = scene_with(&document, &fonts);
    assert_eq!(scene, document.scene(VIEWPORT, style(1.0)).unwrap());
    assert!(fonts.selected.lock().unwrap().is_empty());
}

#[test]
fn hidden_and_unpainted_text_makes_no_run() {
    for body in [
        r##"<text x="2" y="12" font-family="Ahem" font-size="8" fill="#000" visibility="hidden">AB</text>"##,
        r##"<text x="2" y="12" font-family="Ahem" font-size="8" fill="none">AB</text>"##,
        r##"<text x="2" y="12" font-family="Ahem" font-size="8" fill="#000" display="none">AB</text>"##,
        r##"<text x="2" y="12" font-family="Ahem" font-size="8" fill="#000"> </text>"##,
    ] {
        let scene = scene_with(&document(body), &Fonts::ahem());
        assert!(runs(&scene.root).is_empty(), "{body}");
    }
    // A span can hide itself without hiding its siblings.
    let scene = assert_matches_raster(
        r##"<text x="2" y="12" font-family="Ahem" font-size="6" fill="#000">A<tspan visibility="hidden">B</tspan>C</text>"##,
        &Fonts::ahem(),
        2,
    );
    assert_eq!(
        runs(&scene.root)
            .iter()
            .map(|run| run.glyphs.len())
            .sum::<usize>(),
        2
    );
}

#[test]
fn a_pattern_fill_on_text_keeps_the_document_on_the_raster() {
    let document = document(
        r##"<pattern id="p" width="4" height="4" patternUnits="userSpaceOnUse"><rect width="2" height="2"/></pattern>
            <text x="2" y="12" font-family="Ahem" font-size="8" fill="url(#p)">A</text>"##,
    );
    let error = document
        .scene_with_fonts(VIEWPORT, style(1.0), None, &Fonts::ahem())
        .unwrap_err();
    assert!(matches!(
        error,
        SvgSceneError::Unsupported(SvgSceneFeature::Pattern)
    ));
}

#[test]
fn text_in_a_clip_path_becomes_outlines() {
    let body = r##"<clipPath id="c"><text x="2" y="14" font-family="Ahem" font-size="12">AB</text></clipPath>
        <rect width="40" height="20" fill="#14c" clip-path="url(#c)"/>"##;
    let scene = assert_matches_raster(body, &Fonts::ahem(), 3);
    // No glyph run anywhere: the clip holds geometry.
    assert!(runs(&scene.root).is_empty());
    let clipped = scene
        .root
        .children
        .iter()
        .find_map(|node| match node {
            Node::Group(group) => group.clip.as_ref(),
            _ => None,
        })
        .expect("the rect is clipped");
    assert!(!paths(&clipped.root).is_empty());
}

#[test]
fn root_font_styles_reach_text_like_they_reach_the_raster() {
    let body = r##"<text x="2" y="12" fill="#000">AB</text>"##;
    let document = document(body);
    let resolved = SvgResolvedStyles {
        root_color: [0, 0, 0, 255],
        font: SvgRootFont {
            size: 8.0,
            family: "Ahem",
            weight: 400.0,
            style: "normal",
        },
        elements: &[],
    };
    let fonts = Fonts::ahem();
    let scene = document
        .scene_with_fonts(VIEWPORT, style(1.0), Some(resolved), &fonts)
        .unwrap();
    let run = &runs(&scene.root)[0];
    assert_eq!(run.font_size, 8.0);
    assert_eq!(
        fonts.selected.lock().unwrap()[0].families[0],
        SvgFontFamily::Named("Ahem".into())
    );
    // The same document with the font written on the root.
    let written = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" font-family="Ahem" font-size="8"><text x="2" y="12" fill="#000">AB</text></svg>"##,
    )
    .unwrap();
    assert_close(&render(&scene), &raster_with_faces(&written), 2);
}

#[test]
fn variation_settings_ride_on_the_run() {
    let scene = scene_with(
        &document(
            r##"<text x="2" y="12" font-family="Ahem" font-size="8" font-variation-settings="'wght' 700, 'wdth' 80">A</text>"##,
        ),
        &Fonts::ahem(),
    );
    assert_eq!(
        runs(&scene.root)[0].variations,
        [
            FontVariation::new(*b"wght", 700.0),
            FontVariation::new(*b"wdth", 80.0)
        ]
    );
}

#[test]
fn glyphs_are_a_budget_of_the_scene() {
    let document = document(
        r##"<text x="1" y="12" font-family="Ahem" font-size="2" fill="#000">ABCDEFGHIJ</text>"##,
    );
    let fonts = Fonts::ahem();
    let at = |glyphs| {
        let limits = SceneLimits {
            glyphs,
            ..SceneLimits::default()
        };
        document.build_scene(VIEWPORT, style(1.0), None, limits, Some(&fonts))
    };
    assert!(at(10).is_ok());
    let error = at(9).unwrap_err();
    assert!(matches!(
        error,
        SvgSceneError::LimitExceeded {
            limit: SvgSceneLimit::Glyphs,
            max: 9
        }
    ));
    assert_eq!(error.to_string(), "SVG scene exceeds the limit of 9 glyphs");
}

#[test]
fn each_run_and_decoration_counts_as_a_node() {
    let document = document(
        r##"<text x="2" y="12" font-family="Ahem" font-size="6" fill="#000" text-decoration="underline">A<tspan fill="#c00">B</tspan></text>"##,
    );
    let fonts = Fonts::ahem();
    let scene = scene_with(&document, &fonts);
    // The root, two runs and two underlines.
    assert_eq!(scene.node_count(), 5);
    let limits = SceneLimits {
        nodes: 4,
        ..SceneLimits::default()
    };
    assert!(matches!(
        document.build_scene(VIEWPORT, style(1.0), None, limits, Some(&fonts)),
        Err(SvgSceneError::LimitExceeded {
            limit: SvgSceneLimit::Nodes,
            ..
        })
    ));
}

#[test]
fn text_under_host_opacity_leaves_the_opacity_to_the_host() {
    let document =
        document(r##"<text x="2" y="12" font-family="Ahem" font-size="8" fill="#000">AB</text>"##);
    let fonts = Fonts::ahem();
    let half = document
        .scene_with_fonts(VIEWPORT, style(0.5), None, &fonts)
        .unwrap();
    assert_eq!(half, scene_with(&document, &fonts));
}
