//! SVG text of a page is set in the fonts the page was laid out with,
//! `@font-face` faces included.

use raikiri_html::{LayoutOptions, LayoutStatus, RenderResources};
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

#[test]
fn svg_text_uses_the_font_face_rules_of_the_page() {
    with_page_fonts(|page| {
        let fragment = page
            .fragments()
            .find(|fragment| page.dom().local_name(fragment.node()) == Some("svg"))
            .unwrap();
        let source = page.inline_svg(&fragment).unwrap().unwrap().source;
        let scene = |fonts: &raikiri_html::PageSvgFonts<'_>| -> VectorImage {
            SvgDocument::parse(source.as_bytes())
                .unwrap()
                .scene_with_resources(
                    SvgViewport {
                        width: 40.0,
                        height: 20.0,
                    },
                    SvgRootStyle::default(),
                    None,
                    SvgResources::new().with_fonts(fonts),
                )
                .unwrap()
        };

        let fonts = page.svg_fonts();
        let first = scene(&fonts);
        let runs_of_first = runs(&first.root);
        assert_eq!(runs_of_first.len(), 1);
        assert_eq!(runs_of_first[0].glyphs.len(), 2);
        for glyph in &runs_of_first[0].glyphs {
            assert_eq!(glyph.font.bytes(), AHEM);
        }

        // Scenes of one document share the faces they were set in, whichever
        // handle of the page's fonts built them.
        let second = scene(&page.svg_fonts());
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
