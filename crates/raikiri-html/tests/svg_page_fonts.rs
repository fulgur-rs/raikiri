//! SVG text of a page is set in the fonts the page was laid out with,
//! `@font-face` faces included.

use raikiri_html::{ImageContent, LayoutOptions, LayoutStatus, RenderResources};
use raikiri_svg::{
    SvgDocument, SvgFontFamily, SvgFontRequest, SvgFontSource, SvgResources, SvgRootStyle,
    SvgViewport,
};
use raikiri_traits::vector::{Group, Node, TextRun, VectorImage};
use raikiri_traits::{
    FetchOutcome, FetchedResource, LayoutConfig, NetworkError, NetworkProvider, PageDefaults,
    Request,
};

const AHEM: &[u8] = include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf");

/// Serves the font of the page at one URL.
struct Fonts;

impl NetworkProvider for Fonts {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        if request.url.as_str() != "https://fonts.test/webby.ttf" {
            return Err(NetworkError::Http(404));
        }
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: AHEM.to_vec().into(),
            content_type: Some("font/ttf".into()),
            final_url: request.url,
            encoding: None,
        }))
    }
}

const PAGE: &str = r##"<style>
    @font-face { font-family: Webby; src: url(https://fonts.test/webby.ttf); }
    svg { display: block }
</style>
<svg width="40" height="20">
    <text x="2" y="12" font-family="Webby" font-size="8">AB</text>
</svg>"##;

/// Lays out the page and runs `check` on the SVG fonts of its first page.
fn with_page_fonts(check: impl FnOnce(&raikiri_html::Page<'_>)) {
    let provider = Fonts;
    let resources = RenderResources::new().network_provider(&provider);
    let document = raikiri_html::parse_html_with_resources(PAGE.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(layout) = raikiri_html::layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("expected completed layout")
    };
    check(&layout.page(0).unwrap());
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

/// The scene of the inline SVG `fragment` of `page`.
fn scene_of(page: &raikiri_html::Page<'_>, fragment: &raikiri_html::Fragment<'_>) -> VectorImage {
    let ImageContent::Vector(scene) = page.inline_svg(fragment).unwrap().unwrap().content else {
        panic!("the SVG is drawn as a scene");
    };
    (*scene).clone()
}

#[test]
fn svg_text_uses_the_font_face_rules_of_the_page() {
    with_page_fonts(|page| {
        let fragment = page
            .fragments()
            .find(|fragment| page.dom().local_name(fragment.node()) == Some("svg"))
            .unwrap();
        let first = scene_of(page, &fragment);
        let runs_of_first = runs(&first.root);
        assert_eq!(runs_of_first.len(), 1);
        assert_eq!(runs_of_first[0].glyphs.len(), 2);
        for glyph in &runs_of_first[0].glyphs {
            assert_eq!(glyph.font.bytes(), AHEM);
        }

        // Scenes of one document share the faces they were set in, whichever
        // handle of the page's fonts built them.
        let second = SvgDocument::parse(SVG_TEXT.as_bytes())
            .unwrap()
            .scene_with_resources(
                SvgViewport {
                    width: 40.0,
                    height: 20.0,
                },
                SvgRootStyle::default(),
                None,
                SvgResources::new().with_fonts(&page.svg_fonts()),
            )
            .unwrap();
        assert!(
            runs_of_first[0].glyphs[0]
                .font
                .is_same_face(&runs(&second.root)[0].glyphs[0].font)
        );
    });
}

#[test]
fn the_fallback_of_a_character_finds_the_face_that_has_it() {
    with_page_fonts(|page| {
        let fonts = page.svg_fonts();
        let request = SvgFontRequest::new(vec![SvgFontFamily::Named("Webby".into())]);
        let selected = fonts.select(&request).expect("the @font-face face");
        assert_eq!(selected.bytes(), AHEM);
        // Ahem has a glyph for `A`, so the face that has it is the same one.
        let fallback = fonts.fallback(&request, 'A').expect("a face with `A`");
        assert!(selected.is_same_face(&fallback));
        assert_eq!(format!("{fonts:?}"), "PageSvgFonts { .. }");
    });
}

/// The faces of the text runs of the inline SVG on `page`.
fn svg_text_faces(page: &raikiri_html::Page<'_>) -> Vec<raikiri_traits::vector::FontFace> {
    let mut faces = Vec::new();
    for fragment in page
        .fragments()
        .filter(|fragment| page.dom().local_name(fragment.node()) == Some("svg"))
    {
        let scene = scene_of(page, &fragment);
        for run in runs(&scene.root) {
            faces.extend(run.glyphs.iter().map(|glyph| glyph.font.clone()));
        }
    }
    faces
}

fn layout_of(html: &str) -> raikiri_html::DocumentLayout {
    let provider = Fonts;
    let resources = RenderResources::new().network_provider(&provider);
    let document = raikiri_html::parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(layout) = raikiri_html::layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("expected completed layout")
    };
    layout
}

const SVG_TEXT: &str = r#"<svg width="40" height="20"><text x="2" y="12" font-family="Webby" font-size="8">AB</text></svg>"#;

#[test]
fn pages_laid_out_at_another_width_share_the_faces() {
    // The first page has a wide left margin, so the pages after it are wider
    // and are laid out again in a copy of the document.
    let words = (0..60)
        .map(|index| format!("w{index:03}"))
        .collect::<Vec<_>>()
        .join(" ");
    let layout = layout_of(&format!(
        r#"<style>
            @font-face {{ font-family: Webby; src: url(https://fonts.test/webby.ttf); }}
            @page {{ size: 400px 200px; margin: 20px }}
            @page :first {{ margin-left: 220px }}
            body {{ margin: 0 }}
            svg {{ display: block }}
        </style>{SVG_TEXT}<p>{words}</p>{SVG_TEXT}"#
    ));
    assert!(layout.page_count() >= 2);
    let first = svg_text_faces(&layout.page(0).unwrap());
    let later = svg_text_faces(&layout.page(layout.page_count() - 1).unwrap());
    assert!(!first.is_empty() && !later.is_empty());
    assert!(first[0].is_same_face(&later[0]));
}

#[test]
fn a_running_element_shares_the_faces_of_the_document() {
    let layout = layout_of(&format!(
        r#"<style>
            @font-face {{ font-family: Webby; src: url(https://fonts.test/webby.ttf); }}
            @page {{ size: 200px 100px; margin: 30px 10px; @top-left {{ content: element(hdr) }} }}
            body {{ margin: 0 }}
            svg {{ display: block }}
            .hdr {{ position: running(hdr) }}
        </style><div class="hdr">{SVG_TEXT}</div>{SVG_TEXT}"#
    ));
    let page = layout.page(0).unwrap();
    let body = svg_text_faces(&page);
    let margin_box = page.margin_boxes().into_iter().next().unwrap();

    let placed = page
        .margin_box_running_element(&margin_box)
        .unwrap()
        .unwrap();
    let header = svg_text_faces(&placed.layout.page());
    assert!(!body.is_empty() && !header.is_empty());
    assert!(body[0].is_same_face(&header[0]));
}
